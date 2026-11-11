//! The files of a project, as the app gives them (`docs/architecture.md`, 10.8): only an app touches the file system
//! (principle 1). The command line gives the disk, with atomic writes; tests and the web viewer give [`Memory`].
//!
//! The operations are those a walk needs, with the meaning of `std::fs`: a link is followed by [`Workspace::metadata`]
//! and [`Workspace::canonicalize`], never by [`Workspace::link_metadata`] nor by [`Workspace::read_dir`].

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

/// What a path names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    File,
    Folder,
    Link,
    /// A device, a socket, a pipe: nothing to read as a program.
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Metadata {
    pub kind: Kind,
    pub len: u64,
    /// When the file last changed, in nanoseconds since 1970 as the system says it, if it says it.
    pub modified: Option<u128>,
    pub read_only: bool,
}

/// An entry of a folder, its kind as it is, a link not followed.
#[derive(Debug)]
pub struct Entry {
    pub name: OsString,
    pub kind: io::Result<Kind>,
}

/// What a bounded read gives.
#[derive(Debug, PartialEq, Eq)]
pub enum Read {
    /// At most the limit and one byte, even if the file grows meanwhile: whoever reads it then refuses it.
    Bytes(Vec<u8>),
    /// The size the file has, over the limit: nothing was read.
    TooLarge(u64),
}

pub trait Workspace: Send + Sync {
    /// The path without links, `.` or `..`, of something that exists.
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf>;
    /// The path made absolute against the current folder, without touching what it names.
    fn absolute(&self, path: &Path) -> io::Result<PathBuf>;
    /// The current folder, if there is one.
    fn current(&self) -> Option<PathBuf>;
    /// What the path names, a link followed.
    fn metadata(&self, path: &Path) -> io::Result<Metadata>;
    /// What the path names, a link not followed.
    fn link_metadata(&self, path: &Path) -> io::Result<Metadata>;
    /// Each entry of a folder, in the order the system gives them, one at a time: a folder can hold millions.
    fn read_dir(&self, path: &Path, each: &mut dyn FnMut(io::Result<Entry>)) -> io::Result<()>;
    /// Reads a file, refusing one whose size passes `max` before reading it.
    fn read(&self, path: &Path, max: u64) -> io::Result<Read>;
    /// Writes a file atomically: a crash leaves the old file or the new one. Its folder exists.
    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()>;
    /// Makes the folder and those above it.
    fn create_folders(&self, path: &Path) -> io::Result<()>;
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
    fn remove_file(&self, path: &Path) -> io::Result<()>;
    /// Creates a file, or empties it, to write as it goes: the `.partial` of a trace, which can be followed while a run
    /// writes it (`docs/commands.md`, section 8). Not atomic. Its folder exists.
    fn create(&self, _path: &Path) -> io::Result<Box<dyn io::Write + Send>> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
    /// Opens a file to read from `offset` as it goes, a trace of 256 MiB being read in pieces; on Windows, so that it can be
    /// replaced while it is open (commands.md 8).
    fn open(&self, _path: &Path, _offset: u64) -> io::Result<Box<dyn io::Read + Send>> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
    /// Takes the exclusive lock of a file, made empty if it is missing, held until the guard is dropped; `None` when someone
    /// else holds it. A process that writes a `.partial` holds the lock of `.<pid>.lock` beside it, which tells another
    /// process that it still runs (`docs/commands.md`, section 8; decision of the owner, 2026-11-11). Its folder exists.
    fn try_lock(&self, _path: &Path) -> io::Result<Option<Held>> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

/// A lock that [`Workspace::try_lock`] took: dropping it lets it go.
pub type Held = Box<dyn std::any::Any + Send + Sync>;

/// A file system in memory, for tests and for the web viewer, with the meaning of the disk: files, folders and links,
/// whose names are compared exactly. Paths are absolute from `/`; the current folder is `/`, or the one given.
#[derive(Debug, Default)]
pub struct Memory {
    nodes: Arc<Mutex<Nodes>>,
}

/// A file of [`Memory`] written as it goes: each write lands at once, so that a reader sees it.
struct MemoryFile {
    nodes: Arc<Mutex<Nodes>>,
    path: PathBuf,
}

impl io::Write for MemoryFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut nodes = self.nodes.lock().unwrap_or_else(PoisonError::into_inner);
        nodes.clock += 1;
        let clock = nodes.clock;
        match nodes.by_path.get_mut(&self.path) {
            Some(Node::File { bytes, modified, .. }) => {
                bytes.extend_from_slice(buf);
                *modified = clock;
                Ok(buf.len())
            }
            _ => Err(not_found()),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Debug, Default)]
struct Nodes {
    by_path: BTreeMap<PathBuf, Node>,
    /// Counts the writes, so that each one changes the time of what it writes.
    clock: u128,
    current: Option<PathBuf>,
    /// The files whose lock someone holds.
    locked: BTreeSet<PathBuf>,
}

/// A lock of [`Memory`], let go when dropped.
struct MemoryLock {
    nodes: Arc<Mutex<Nodes>>,
    path: PathBuf,
}

impl Drop for MemoryLock {
    fn drop(&mut self) {
        self.nodes.lock().unwrap_or_else(PoisonError::into_inner).locked.remove(&self.path);
    }
}

#[derive(Clone, Debug)]
enum Node {
    File { bytes: Vec<u8>, modified: u128, read_only: bool },
    Folder,
    Link(PathBuf),
}

/// A link followed more times than this is a loop, as the systems say.
const MAX_LINKS: usize = 40;

fn not_found() -> io::Error {
    io::Error::from(io::ErrorKind::NotFound)
}

/// The path from `/`, its `.` and `..` resolved by their names alone.
fn lexical(path: &Path) -> PathBuf {
    let mut parts: Vec<OsString> = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(name) => parts.push(name.to_os_string()),
            Component::ParentDir => {
                parts.pop();
            }
            Component::CurDir | Component::RootDir | Component::Prefix(_) => {}
        }
    }
    let mut resolved = PathBuf::from("/");
    resolved.extend(parts);
    resolved
}

impl Memory {
    pub fn new() -> Memory {
        Memory::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Nodes> {
        self.nodes.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Puts a file, making its folders.
    pub fn put(&self, path: impl AsRef<Path>, bytes: impl Into<Vec<u8>>) {
        let path = lexical(path.as_ref());
        let mut nodes = self.lock();
        nodes.make_folders(path.parent());
        nodes.clock += 1;
        let modified = nodes.clock;
        nodes.by_path.insert(path, Node::File { bytes: bytes.into(), modified, read_only: false });
    }

    /// Replaces the bytes of a file and keeps its time of change: what a file system whose clock ticks slowly does with
    /// two writes within one tick.
    pub fn put_keeping_time(&self, path: impl AsRef<Path>, bytes: impl Into<Vec<u8>>) {
        let path = lexical(path.as_ref());
        let mut nodes = self.lock();
        let modified = match nodes.by_path.get(&path) {
            Some(Node::File { modified, .. }) => *modified,
            _ => 0,
        };
        nodes.make_folders(path.parent());
        nodes.by_path.insert(path, Node::File { bytes: bytes.into(), modified, read_only: false });
    }

    pub fn put_folder(&self, path: impl AsRef<Path>) {
        self.lock().make_folders(Some(&lexical(path.as_ref())));
    }

    /// A link at `path` to `target`, which need not exist.
    pub fn put_link(&self, path: impl AsRef<Path>, target: impl AsRef<Path>) {
        let path = lexical(path.as_ref());
        let mut nodes = self.lock();
        nodes.make_folders(path.parent());
        nodes.by_path.insert(path, Node::Link(lexical(target.as_ref())));
    }

    pub fn set_read_only(&self, path: impl AsRef<Path>, read_only: bool) {
        if let Some(Node::File { read_only: flag, .. }) = self.lock().by_path.get_mut(&lexical(path.as_ref())) {
            *flag = read_only;
        }
    }

    pub fn set_current(&self, path: impl AsRef<Path>) {
        self.lock().current = Some(lexical(path.as_ref()));
    }

    /// The bytes of a file, a link followed.
    pub fn get(&self, path: impl AsRef<Path>) -> Option<Vec<u8>> {
        let nodes = self.lock();
        let real = nodes.resolve(path.as_ref()).ok()?;
        match nodes.by_path.get(&real) {
            Some(Node::File { bytes, .. }) => Some(bytes.clone()),
            _ => None,
        }
    }
}

impl Nodes {
    fn make_folders(&mut self, path: Option<&Path>) {
        let Some(path) = path else { return };
        for ancestor in path.ancestors() {
            self.by_path.entry(ancestor.to_path_buf()).or_insert(Node::Folder);
        }
    }

    fn absolute(&self, path: &Path) -> PathBuf {
        if path.has_root() { lexical(path) } else { lexical(&self.current.clone().unwrap_or_else(|| PathBuf::from("/")).join(path)) }
    }

    /// The path with every link followed, at every level, as the disk resolves it: `..` after a link goes up from its
    /// target.
    fn resolve(&self, path: &Path) -> io::Result<PathBuf> {
        let mut pending: Vec<OsString> = Vec::new();
        let start = if path.has_root() { path.to_path_buf() } else { self.current.clone().unwrap_or_else(|| PathBuf::from("/")).join(path) };
        for component in start.components().rev() {
            match component {
                Component::Normal(name) => pending.push(name.to_os_string()),
                Component::ParentDir => pending.push(OsString::from("..")),
                Component::CurDir | Component::RootDir | Component::Prefix(_) => {}
            }
        }
        let mut resolved = PathBuf::from("/");
        let mut links = 0;
        while let Some(part) = pending.pop() {
            if part == ".." {
                resolved.pop();
                continue;
            }
            let next = resolved.join(&part);
            match self.by_path.get(&next) {
                None => return Err(not_found()),
                Some(Node::Link(target)) => {
                    links += 1;
                    if links > MAX_LINKS {
                        return Err(io::Error::other("too many levels of symbolic links"));
                    }
                    for component in target.components().rev() {
                        if let Component::Normal(name) = component {
                            pending.push(name.to_os_string());
                        }
                    }
                    resolved = PathBuf::from("/");
                }
                Some(Node::File { .. }) if !pending.is_empty() => return Err(io::Error::other("not a directory")),
                Some(_) => resolved = next,
            }
        }
        Ok(resolved)
    }

    /// The node at the path, its folders resolved, the last name not followed.
    fn at(&self, path: &Path) -> io::Result<(PathBuf, &Node)> {
        let absolute = self.absolute(path);
        let real = match (absolute.parent(), absolute.file_name()) {
            (Some(parent), Some(name)) => self.resolve(parent)?.join(name),
            _ => absolute,
        };
        let node = self.by_path.get(&real).ok_or_else(not_found)?;
        Ok((real, node))
    }

    fn metadata_of(node: &Node) -> Metadata {
        match node {
            Node::File { bytes, modified, read_only } => {
                Metadata { kind: Kind::File, len: bytes.len() as u64, modified: Some(*modified), read_only: *read_only }
            }
            Node::Folder => Metadata { kind: Kind::Folder, len: 0, modified: None, read_only: false },
            Node::Link(_) => Metadata { kind: Kind::Link, len: 0, modified: None, read_only: false },
        }
    }
}

impl Workspace for Memory {
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        self.lock().resolve(path)
    }

    fn absolute(&self, path: &Path) -> io::Result<PathBuf> {
        Ok(self.lock().absolute(path))
    }

    fn current(&self) -> Option<PathBuf> {
        Some(self.lock().current.clone().unwrap_or_else(|| PathBuf::from("/")))
    }

    fn metadata(&self, path: &Path) -> io::Result<Metadata> {
        let nodes = self.lock();
        let real = nodes.resolve(path)?;
        nodes.by_path.get(&real).map(Nodes::metadata_of).ok_or_else(not_found)
    }

    fn link_metadata(&self, path: &Path) -> io::Result<Metadata> {
        let nodes = self.lock();
        nodes.at(path).map(|(_, node)| Nodes::metadata_of(node))
    }

    fn read_dir(&self, path: &Path, each: &mut dyn FnMut(io::Result<Entry>)) -> io::Result<()> {
        let entries: Vec<Entry> = {
            let nodes = self.lock();
            let real = nodes.resolve(path)?;
            if !matches!(nodes.by_path.get(&real), Some(Node::Folder)) {
                return Err(io::Error::other("not a directory"));
            }
            nodes
                .by_path
                .iter()
                .filter(|(child, _)| child.parent() == Some(real.as_path()) && **child != real)
                .filter_map(|(child, node)| {
                    let name = child.file_name()?.to_os_string();
                    let kind = match node {
                        Node::File { .. } => Kind::File,
                        Node::Folder => Kind::Folder,
                        Node::Link(_) => Kind::Link,
                    };
                    Some(Entry { name, kind: Ok(kind) })
                })
                .collect()
        };
        for entry in entries {
            each(Ok(entry));
        }
        Ok(())
    }

    fn read(&self, path: &Path, max: u64) -> io::Result<Read> {
        let nodes = self.lock();
        let real = nodes.resolve(path)?;
        match nodes.by_path.get(&real) {
            Some(Node::File { bytes, .. }) if bytes.len() as u64 > max => Ok(Read::TooLarge(bytes.len() as u64)),
            Some(Node::File { bytes, .. }) => Ok(Read::Bytes(bytes.clone())),
            Some(_) => Err(io::Error::other("is a directory")),
            None => Err(not_found()),
        }
    }

    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        let mut nodes = self.lock();
        let absolute = nodes.absolute(path);
        let (Some(parent), Some(name)) = (absolute.parent(), absolute.file_name()) else { return Err(io::Error::other("no file name")) };
        let folder = nodes.resolve(parent)?;
        if !matches!(nodes.by_path.get(&folder), Some(Node::Folder)) {
            return Err(not_found());
        }
        // A link stays a link: what it points to is written, as `craft-atomic` does.
        let mut target = folder.join(name);
        if let Some(Node::Link(_)) = nodes.by_path.get(&target) {
            target = nodes.resolve(&target)?;
        }
        match nodes.by_path.get(&target) {
            Some(Node::File { read_only: true, .. }) => return Err(io::Error::from(io::ErrorKind::PermissionDenied)),
            Some(Node::Folder) => return Err(io::Error::other("is a directory")),
            _ => {}
        }
        nodes.clock += 1;
        let modified = nodes.clock;
        nodes.by_path.insert(target, Node::File { bytes: bytes.to_vec(), modified, read_only: false });
        Ok(())
    }

    fn create_folders(&self, path: &Path) -> io::Result<()> {
        let mut nodes = self.lock();
        let absolute = nodes.absolute(path);
        for ancestor in absolute.ancestors() {
            if let Some(Node::File { .. }) = nodes.by_path.get(ancestor) {
                return Err(io::Error::from(io::ErrorKind::AlreadyExists));
            }
        }
        nodes.make_folders(Some(&absolute));
        Ok(())
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        let mut nodes = self.lock();
        let (from, node) = nodes.at(from).map(|(real, node)| (real, node.clone()))?;
        if matches!(node, Node::Folder) {
            return Err(io::Error::other("renaming a folder is not supported here"));
        }
        let absolute = nodes.absolute(to);
        let (Some(parent), Some(name)) = (absolute.parent(), absolute.file_name()) else { return Err(io::Error::other("no file name")) };
        let to = nodes.resolve(parent)?.join(name);
        if let Some(Node::Folder) = nodes.by_path.get(&to) {
            return Err(io::Error::other("is a directory"));
        }
        nodes.by_path.remove(&from);
        nodes.by_path.insert(to, node);
        Ok(())
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        let mut nodes = self.lock();
        let (real, node) = nodes.at(path)?;
        if matches!(node, Node::Folder) {
            return Err(io::Error::other("is a directory"));
        }
        nodes.by_path.remove(&real);
        Ok(())
    }

    fn create(&self, path: &Path) -> io::Result<Box<dyn io::Write + Send>> {
        self.write(path, b"")?;
        let real = self.lock().resolve(path)?;
        Ok(Box::new(MemoryFile { nodes: Arc::clone(&self.nodes), path: real }))
    }

    fn open(&self, path: &Path, offset: u64) -> io::Result<Box<dyn io::Read + Send>> {
        let nodes = self.lock();
        let real = nodes.resolve(path)?;
        match nodes.by_path.get(&real) {
            Some(Node::File { bytes, .. }) => {
                let from = usize::try_from(offset).unwrap_or(usize::MAX).min(bytes.len());
                Ok(Box::new(io::Cursor::new(bytes.get(from..).unwrap_or_default().to_vec())))
            }
            Some(_) => Err(io::Error::other("is a directory")),
            None => Err(not_found()),
        }
    }

    fn try_lock(&self, path: &Path) -> io::Result<Option<Held>> {
        if self.link_metadata(path).is_err() {
            self.write(path, b"")?;
        }
        let mut nodes = self.lock();
        let real = nodes.resolve(path)?;
        if !matches!(nodes.by_path.get(&real), Some(Node::File { .. })) {
            return Err(io::Error::other("not a file"));
        }
        if !nodes.locked.insert(real.clone()) {
            return Ok(None);
        }
        Ok(Some(Box::new(MemoryLock { nodes: Arc::clone(&self.nodes), path: real })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lock_is_held_until_its_guard_goes() {
        let memory = Memory::new();
        memory.put_folder("/p");
        let held = memory.try_lock(Path::new("/p/.7.lock")).unwrap();
        assert!(held.is_some());
        assert!(memory.get("/p/.7.lock").is_some(), "the file is made");
        assert!(memory.try_lock(Path::new("/p/.7.lock")).unwrap().is_none());
        drop(held);
        assert!(memory.try_lock(Path::new("/p/.7.lock")).unwrap().is_some());
    }

    #[test]
    fn a_link_is_followed_by_canonicalize_and_by_metadata_but_not_by_link_metadata() {
        let memory = Memory::new();
        memory.put("/p/a/x.lcf", "fn x()\n");
        memory.put_link("/p/b", "/p/a");
        assert_eq!(memory.canonicalize(Path::new("/p/b/x.lcf")).unwrap(), PathBuf::from("/p/a/x.lcf"));
        assert_eq!(memory.metadata(Path::new("/p/b")).unwrap().kind, Kind::Folder);
        assert_eq!(memory.link_metadata(Path::new("/p/b")).unwrap().kind, Kind::Link);
        assert_eq!(memory.canonicalize(Path::new("/p/b/../a")).unwrap(), PathBuf::from("/p/a"), "`..` after a link goes up from its target");
    }

    #[test]
    fn a_loop_of_links_is_an_error_not_a_hang() {
        let memory = Memory::new();
        memory.put_link("/p/a", "/p/b");
        memory.put_link("/p/b", "/p/a");
        assert!(memory.canonicalize(Path::new("/p/a")).is_err());
    }

    #[test]
    fn a_read_over_the_limit_reads_nothing_and_says_the_size() {
        let memory = Memory::new();
        memory.put("/f.lcf", vec![b'x'; 10]);
        assert_eq!(memory.read(Path::new("/f.lcf"), 9).unwrap(), Read::TooLarge(10));
        assert_eq!(memory.read(Path::new("/f.lcf"), 10).unwrap(), Read::Bytes(vec![b'x'; 10]));
    }

    #[test]
    fn each_write_changes_the_time_of_the_file_and_a_read_only_file_is_refused() {
        let memory = Memory::new();
        memory.put("/f.lcf", "a");
        let before = memory.metadata(Path::new("/f.lcf")).unwrap().modified;
        memory.write(Path::new("/f.lcf"), b"b").unwrap();
        assert_ne!(memory.metadata(Path::new("/f.lcf")).unwrap().modified, before);
        memory.set_read_only("/f.lcf", true);
        assert_eq!(memory.write(Path::new("/f.lcf"), b"c").unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert!(memory.write(Path::new("/nowhere/f.lcf"), b"c").is_err(), "the folder must exist");
    }

    #[test]
    fn a_folder_reached_through_a_link_lists_its_entries() {
        let memory = Memory::new();
        memory.put("/p/a/x.lcf", "");
        memory.put_link("/p/l", "/p/a");
        let mut names = Vec::new();
        memory.read_dir(Path::new("/p/l"), &mut |entry| names.push(entry.unwrap().name)).unwrap();
        assert_eq!(names, [OsString::from("x.lcf")]);
    }
}
