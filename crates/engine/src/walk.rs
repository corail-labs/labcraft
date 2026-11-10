//! From the paths of the command line to the files to work on (`docs/cli.md`, section 1): recursive, in code point
//! order of the paths, without following a symbolic link, within the bounds of a library.
//!
//! The result does not depend on the order of the arguments, nor on the order in which a folder gives its entries.
//! A file is known by its path without links, on which a path given twice, or also reached through a folder, is one.

use std::collections::HashSet;
use std::io;
use std::path::{MAIN_SEPARATOR_STR, Path, PathBuf};

use labcraft_types::{Code, Diagnostic, Limits, Report};
use sha2::{Digest, Sha256};

use crate::workspace::{Kind, Read, Workspace};

/// The bounds of a folder given, apart from its bytes, which `Limits` holds.
#[derive(Clone, Copy, Debug)]
pub struct Bounds {
    pub levels: usize,
    /// Of files, and of folders.
    pub files: usize,
    /// Only the files of the folder itself, for a library that a `use` asked for: its sub-folders are other libraries,
    /// and a folder with nothing to read says nothing, since it is simply not that library.
    pub alone: bool,
}

pub const BOUNDS: Bounds = Bounds { levels: 32, files: 100_000, alone: false };

/// A file to work on, and its path in reports.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Found {
    pub shown: String,
    pub path: PathBuf,
    /// Named through a link: its real name wins if the file is also reached under it.
    pub through_a_link: bool,
}

#[derive(Debug)]
pub struct Walked {
    /// In code point order of the paths, each file once.
    pub files: Vec<Found>,
    /// Files that will not be read, whose diagnostics are in `report`.
    pub refused: Vec<Refused>,
    pub report: Report,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused {
    pub shown: String,
    pub path: PathBuf,
}

/// Why a file cannot be read: its code and its message.
pub type ReadError = (Code, String);

/// What a read of a file saw, so that a session can tell later whether the same read would see the same: the SHA-256 of
/// its bytes, the size that refused it, or the error of the system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContentStamp {
    Hash([u8; 32]),
    TooLarge(u64),
    Unreadable(String),
}

/// Reads no more than the limit and one byte, even if the file grows meanwhile: reading it then refuses it.
pub fn read_program(ws: &dyn Workspace, path: &Path, limits: &Limits) -> Result<Vec<u8>, ReadError> {
    read_program_stamped(ws, path, limits).0
}

/// As [`read_program`], with what the read saw.
pub fn read_program_stamped(ws: &dyn Workspace, path: &Path, limits: &Limits) -> (Result<Vec<u8>, ReadError>, ContentStamp) {
    let read = ws.read(path, limits.max_file_size);
    let stamp = match &read {
        Ok(Read::Bytes(bytes)) => ContentStamp::Hash(Sha256::digest(bytes).into()),
        Ok(Read::TooLarge(size)) => ContentStamp::TooLarge(*size),
        Err(error) => ContentStamp::Unreadable(io_text(error)),
    };
    (read_result(read, limits), stamp)
}

fn read_result(read: io::Result<Read>, limits: &Limits) -> Result<Vec<u8>, ReadError> {
    match read {
        Ok(Read::Bytes(bytes)) => Ok(bytes),
        Ok(Read::TooLarge(size)) => Err((Code::Lc0006, format!("file of {size} bytes, over the limit of {} bytes", limits.max_file_size))),
        Err(error) => Err((Code::Lc0001, format!("cannot read this file: {}", io_text(&error)))),
    }
}

/// The files of one folder, without its sub-folders, shown as usual.
pub fn walk_alone(ws: &dyn Workspace, folder: &Path, current: Option<&Path>, limits: &Limits) -> Walked {
    walk_within(ws, std::slice::from_ref(&folder.to_path_buf()), current, limits, Bounds { alone: true, ..BOUNDS })
}

/// An error of the operating system, in English and without a path: the text of the system follows its language,
/// and `craft-atomic` puts a whole path before the cause, which the limit on a message would then cut off.
pub fn io_text(error: &io::Error) -> String {
    error.kind().to_string()
}

pub fn slashed(path: &Path) -> String {
    path.to_string_lossy().replace(MAIN_SEPARATOR_STR, "/")
}

fn relative_to(path: &Path, current: Option<&Path>) -> Option<String> {
    let relative = path.strip_prefix(current?).ok()?;
    Some(if relative.as_os_str().is_empty() { ".".to_owned() } else { slashed(relative) })
}

fn is_a_link(ws: &dyn Workspace, path: &Path) -> bool {
    ws.link_metadata(path).is_ok_and(|metadata| metadata.kind == Kind::Link)
}

/// `current` is without links, as `walk` makes it.
pub fn shown_of(ws: &dyn Workspace, path: &Path, real: Option<&Path>, current: Option<&Path>) -> String {
    shown_path(ws, path, real, current)
}

/// With `/` on every platform, and relative to the current folder when the file is under it. A path is shown by its
/// real name, whatever its spelling; a link stays what the user called it, in the real name of its folder.
fn shown_path(ws: &dyn Workspace, path: &Path, real: Option<&Path>, current: Option<&Path>) -> String {
    let located = match real {
        Some(real) if !is_a_link(ws, path) => Some(real.to_path_buf()),
        _ => ws.absolute(path).ok().map(|absolute| match (absolute.parent(), absolute.file_name()) {
            (Some(parent), Some(name)) => ws.canonicalize(parent).map_or_else(|_| absolute.clone(), |parent| parent.join(name)),
            _ => absolute.clone(),
        }),
    };
    let shown = located.and_then(|located| relative_to(&located, current)).unwrap_or_else(|| slashed(path));
    if shown.is_empty() { "\"\"".to_owned() } else { shown }
}

pub fn walk(ws: &dyn Workspace, paths: &[PathBuf], current: Option<&Path>, limits: &Limits) -> Walked {
    walk_within(ws, paths, current, limits, BOUNDS)
}

/// A path of the command line.
struct Named<'a> {
    given: &'a Path,
    shown: String,
    /// Without links, or why it cannot be read.
    real: io::Result<PathBuf>,
}

/// What has been said about a file, known by its real path, so that a file reached twice gets its diagnostic once.
struct Said {
    paths: HashSet<PathBuf>,
    /// A walk can say millions of things, so it remembers them only if another path can reach the same ones.
    in_walks: bool,
}

impl Said {
    fn first_time(&mut self, path: &Path) -> bool {
        self.paths.insert(path.to_path_buf())
    }

    fn first_time_in_a_walk(&mut self, path: &Path) -> bool {
        !self.in_walks || self.first_time(path)
    }
}

/// Each folder given is walked on its own, with its own bounds, so that naming a path never loses what naming it
/// alone would keep. What two of them reach is put together at the end.
/// As [`walk`], within the bounds given.
pub fn walk_within(ws: &dyn Workspace, paths: &[PathBuf], current: Option<&Path>, limits: &Limits, bounds: Bounds) -> Walked {
    let current = current.and_then(|current| ws.canonicalize(current).ok());
    let mut named: Vec<Named<'_>> = paths
        .iter()
        .map(|given| {
            let real = ws.canonicalize(given);
            Named { given, shown: shown_path(ws, given, real.as_deref().ok(), current.as_deref()), real }
        })
        .collect();
    // Parents first, and what a folder holds right after it.
    named.sort_by(|a, b| a.real.as_ref().ok().cmp(&b.real.as_ref().ok()).then_with(|| a.shown.cmp(&b.shown)));
    let in_walks = named.iter().zip(named.iter().skip(1)).any(|(a, b)| match (&a.real, &b.real) {
        (Ok(a), Ok(b)) => b.starts_with(a),
        _ => false,
    });
    let mut said = Said { paths: HashSet::new(), in_walks };
    let mut folders = HashSet::new();
    let mut walked = Walked { files: Vec::new(), refused: Vec::new(), report: Report::for_limits(limits) };
    for Named { given, shown, real } in named {
        let (real, metadata) = match real.and_then(|real| ws.metadata(&real).map(|metadata| (real, metadata))) {
            Ok(found) => found,
            Err(error) => {
                if said.first_time(Path::new(&shown)) {
                    error_at(&mut walked.report, Code::Lc0001, &shown, format!("cannot read this path: {}", io_text(&error)));
                }
                continue;
            }
        };
        if metadata.kind == Kind::Folder {
            if folders.insert(real.clone()) {
                let root = Given { path: &real, shown: &shown, current: current.as_deref(), through_a_link: is_a_link(ws, given) };
                walk_root(ws, &root, limits, bounds, &mut said, &mut walked);
            }
        } else if metadata.kind != Kind::File {
            if said.first_time(&real) {
                error_at(&mut walked.report, Code::Lc0001, &shown, "cannot read this path: not a regular file".to_owned());
                walked.refused.push(Refused { shown, path: real });
            }
        } else if !has_program_extension(given) || !has_program_extension(&real) {
            if said.first_time(&real) {
                let message = "a file named explicitly, and what a link points to, must have the extension `.lcf` or `.lct`".to_owned();
                error_at(&mut walked.report, Code::Lc0008, &shown, message);
                walked.refused.push(Refused { shown, path: real });
            }
        } else {
            walked.files.push(Found { shown, path: real, through_a_link: is_a_link(ws, given) });
        }
    }
    walked.files.sort_by(|a, b| (&a.path, a.through_a_link, &a.shown).cmp(&(&b.path, b.through_a_link, &b.shown)));
    walked.files.dedup_by(|later, first| later.path == first.path);
    walked.files.sort();
    walked
}

fn error_at(report: &mut Report, code: Code, file: &str, message: String) {
    report.push(Diagnostic::new(code, message, file));
}

fn has_program_extension(path: &Path) -> bool {
    // On Windows, `notes.txt:a.lcf` names a stream of `notes.txt`, which is no program and cannot be replaced.
    let a_stream = cfg!(windows) && path.file_name().is_some_and(|name| name.to_string_lossy().contains(':'));
    !a_stream && path.extension().is_some_and(|extension| extension == "lcf" || extension == "lct")
}

/// A file met in a folder.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Entry {
    shown: String,
    path: PathBuf,
    /// What would be read of it, unless it is refused for the reason given.
    size: u64,
    refused: Option<String>,
}

/// A folder given on the command line.
struct Given<'a> {
    path: &'a Path,
    shown: &'a str,
    current: Option<&'a Path>,
    through_a_link: bool,
}

/// One folder given on the command line, and what is met in it.
struct Root<'a> {
    ws: &'a dyn Workspace,
    current: Option<&'a Path>,
    through_a_link: bool,
    limits: &'a Limits,
    bounds: Bounds,
    said: &'a mut Said,
    report: &'a mut Report,
    entries: Vec<Entry>,
    folders: usize,
    too_many_files: bool,
    too_many_folders: bool,
    too_deep: bool,
    failed: bool,
}

fn walk_root(ws: &dyn Workspace, given: &Given<'_>, limits: &Limits, bounds: Bounds, said: &mut Said, walked: &mut Walked) {
    let mut root = Root {
        ws,
        current: given.current,
        through_a_link: given.through_a_link,
        limits,
        bounds,
        said,
        report: &mut walked.report,
        entries: Vec::new(),
        folders: 0,
        too_many_files: false,
        too_many_folders: false,
        too_deep: false,
        failed: false,
    };
    root.folder(given.path, given.shown, 1);
    root.finish(given.shown, &mut walked.files, &mut walked.refused);
}

/// Keeps what comes first, however the list was made: a folder can hold millions, so a caller calls this as soon as
/// it holds twice the limit. True if something was left out.
fn keep_first<T: Ord>(items: &mut Vec<T>, limit: usize) -> bool {
    items.sort();
    let over = items.len() > limit;
    items.truncate(limit);
    over
}

fn child_shown(shown: &str, name: &str) -> String {
    match shown {
        "." => name.to_owned(),
        _ if shown.ends_with('/') => format!("{shown}{name}"),
        _ => format!("{shown}/{name}"),
    }
}

impl Root<'_> {
    /// A file under the current folder is shown relative to it, even if the folder given is not, as `..` is not.
    fn shown_in(&self, folder_shown: &str, name: &str, path: &Path) -> String {
        relative_to(path, self.current).unwrap_or_else(|| child_shown(folder_shown, name))
    }

    fn fail(&mut self, path: &Path, shown: &str, error: &io::Error) {
        self.failed = true;
        if self.said.first_time_in_a_walk(path) {
            error_at(self.report, Code::Lc0001, shown, format!("cannot read this path: {}", io_text(error)));
        }
    }

    fn folder(&mut self, path: &Path, shown: &str, level: usize) {
        self.folders += 1;
        let mut sub_folders: Vec<(String, PathBuf)> = Vec::new();
        let ws = self.ws;
        let listed = ws.read_dir(path, &mut |entry| {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => return self.fail(path, shown, &error),
            };
            let name = entry.name.to_string_lossy();
            let kind = match entry.kind {
                Ok(kind) => kind,
                Err(error) => return self.fail(&path.join(&entry.name), &child_shown(shown, &name), &error),
            };
            // A folder of a million other files costs no path and no string.
            match kind {
                Kind::Link => {
                    let entry_path = path.join(&entry.name);
                    if self.said.first_time_in_a_walk(&entry_path) {
                        let entry_shown = self.shown_in(shown, &name, &entry_path);
                        self.report.push(Diagnostic::new(Code::Lc0009, "symbolic link: not followed", entry_shown));
                    }
                }
                Kind::Folder => {
                    if !name.starts_with('.') && !self.bounds.alone {
                        let entry_path = path.join(&entry.name);
                        sub_folders.push((self.shown_in(shown, &name, &entry_path), entry_path));
                        if sub_folders.len() > self.bounds.files.saturating_mul(2) {
                            self.too_many_folders |= keep_first(&mut sub_folders, self.bounds.files);
                        }
                    }
                }
                Kind::File | Kind::Other => {
                    if has_program_extension(Path::new(&entry.name)) {
                        let entry_path = path.join(&entry.name);
                        let entry_shown = self.shown_in(shown, &name, &entry_path);
                        self.file(entry_path, entry_shown, kind == Kind::File);
                    }
                }
            }
        });
        if let Err(error) = listed {
            return self.fail(path, shown, &error);
        }
        sub_folders.sort();
        for (sub_shown, sub_path) in sub_folders {
            if level >= self.bounds.levels {
                self.too_deep = true;
            } else if self.folders >= self.bounds.files {
                self.too_many_folders = true;
            } else {
                self.folder(&sub_path, &sub_shown, level + 1);
            }
        }
    }

    fn file(&mut self, path: PathBuf, shown: String, regular: bool) {
        let (size, refused) = if regular {
            match self.ws.link_metadata(&path) {
                // A file over its own limit counts only for what would be read of it.
                Ok(metadata) => (metadata.len.min(self.limits.max_file_size.saturating_add(1)), None),
                Err(error) => (0, Some(io_text(&error))),
            }
        } else {
            (0, Some("not a regular file".to_owned()))
        };
        self.entries.push(Entry { shown, path, size, refused });
        if self.entries.len() > self.bounds.files.saturating_mul(2) {
            self.too_many_files |= keep_first(&mut self.entries, self.bounds.files);
        }
    }

    /// What comes first in the order of the paths stays, and is either to read or refused.
    fn finish(mut self, shown: &str, files: &mut Vec<Found>, refused: &mut Vec<Refused>) {
        self.too_many_files |= keep_first(&mut self.entries, self.bounds.files);
        let too_many_bytes = self.keep_within_the_bytes();
        let left_out = [
            (self.too_many_files, format!("files left out: over the limit of {} files in a folder", self.bounds.files)),
            (self.too_many_folders, format!("folders left out: over the limit of {} folders, the one given included", self.bounds.files)),
            (self.too_deep, format!("folders left out: over the limit of {} levels of folders", self.bounds.levels)),
            (too_many_bytes, format!("files left out: over the limit of {} bytes in a folder", self.limits.max_library_size)),
        ];
        for (over, message) in left_out {
            if over {
                self.failed = true;
                error_at(self.report, Code::Lc0010, shown, message);
            }
        }
        if self.entries.is_empty() && !self.failed && !self.bounds.alone {
            error_at(self.report, Code::Lc0002, shown, "nothing to check: no `.lcf` or `.lct` file under this path".to_owned());
        }
        for entry in self.entries.drain(..) {
            match entry.refused {
                None => files.push(Found { shown: entry.shown, path: entry.path, through_a_link: self.through_a_link }),
                Some(why) => {
                    if self.said.first_time_in_a_walk(&entry.path) {
                        error_at(self.report, Code::Lc0001, &entry.shown, format!("cannot read this path: {why}"));
                        refused.push(Refused { shown: entry.shown, path: entry.path });
                    }
                }
            }
        }
    }

    fn keep_within_the_bytes(&mut self) -> bool {
        let mut total: u64 = 0;
        let first_left_out = self.entries.iter().position(|entry| {
            total = total.saturating_add(entry.size);
            total > self.limits.max_library_size
        });
        first_left_out.is_some_and(|first| {
            self.entries.truncate(first);
            true
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_stays_does_not_depend_on_when_the_list_was_cut() {
        let mut items: Vec<u32> = (0..100).rev().collect();
        assert!(keep_first(&mut items, 3));
        items.extend([7, 1]);
        assert!(keep_first(&mut items, 3));
        assert_eq!(items, [0, 1, 1]);
        assert!(!keep_first(&mut items, 3));
    }
}
