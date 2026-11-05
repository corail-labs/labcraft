//! From the paths of the command line to the files to work on (`docs/cli.md`, section 1): recursive, in code point
//! order of the paths, without following a symbolic link, within the bounds of a library.
//!
//! The result does not depend on the order of the arguments, nor on the order in which a folder gives its entries.
//! A file is known by its path without links, on which a path given twice, or also reached through a folder, is one.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{MAIN_SEPARATOR_STR, Path, PathBuf};

use labcraft_types::{Code, Diagnostic, Limits, Report};

/// The bounds of a folder given, apart from its bytes, which `Limits` holds.
#[derive(Clone, Copy, Debug)]
pub struct Bounds {
    pub levels: usize,
    /// Of files, and of folders.
    pub files: usize,
}

const BOUNDS: Bounds = Bounds { levels: 32, files: 100_000 };

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
    pub refused: Vec<String>,
    pub report: Report,
}

/// An error of the operating system, in English and without a path: the text of the system follows its language,
/// and `craft-atomic` puts a whole path before the cause, which the limit on a message would then cut off.
pub fn io_text(error: &io::Error) -> String {
    error.kind().to_string()
}

fn slashed(path: &Path) -> String {
    path.to_string_lossy().replace(MAIN_SEPARATOR_STR, "/")
}

fn relative_to(path: &Path, current: Option<&Path>) -> Option<String> {
    let relative = path.strip_prefix(current?).ok()?;
    Some(if relative.as_os_str().is_empty() { ".".to_owned() } else { slashed(relative) })
}

fn is_a_link(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink())
}

/// With `/` on every platform, and relative to the current folder when the file is under it. A path is shown by its
/// real name, whatever its spelling; a link stays what the user called it, in the real name of its folder.
fn shown_path(path: &Path, real: Option<&Path>, current: Option<&Path>) -> String {
    let located = match real {
        Some(real) if !is_a_link(path) => Some(real.to_path_buf()),
        _ => std::path::absolute(path).ok().map(|absolute| match (absolute.parent(), absolute.file_name()) {
            (Some(parent), Some(name)) => fs::canonicalize(parent).map_or_else(|_| absolute.clone(), |parent| parent.join(name)),
            _ => absolute.clone(),
        }),
    };
    let shown = located.and_then(|located| relative_to(&located, current)).unwrap_or_else(|| slashed(path));
    if shown.is_empty() { "\"\"".to_owned() } else { shown }
}

pub fn walk(paths: &[PathBuf], current: Option<&Path>, limits: &Limits) -> Walked {
    walk_within(paths, current, limits, BOUNDS)
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
fn walk_within(paths: &[PathBuf], current: Option<&Path>, limits: &Limits, bounds: Bounds) -> Walked {
    let current = current.and_then(|current| fs::canonicalize(current).ok());
    let mut named: Vec<Named<'_>> = paths
        .iter()
        .map(|given| {
            let real = fs::canonicalize(given);
            Named { given, shown: shown_path(given, real.as_deref().ok(), current.as_deref()), real }
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
    let mut walked = Walked { files: Vec::new(), refused: Vec::new(), report: Report::new(limits.max_report) };
    for Named { given, shown, real } in named {
        let (real, metadata) = match real.and_then(|real| fs::metadata(&real).map(|metadata| (real, metadata))) {
            Ok(found) => found,
            Err(error) => {
                if said.first_time(Path::new(&shown)) {
                    error_at(&mut walked.report, Code::Lc0001, &shown, format!("cannot read this path: {}", io_text(&error)));
                }
                continue;
            }
        };
        if metadata.is_dir() {
            if folders.insert(real.clone()) {
                let root = Given { path: &real, shown: &shown, current: current.as_deref(), through_a_link: is_a_link(given) };
                walk_root(&root, limits, bounds, &mut said, &mut walked);
            }
        } else if !metadata.is_file() {
            if said.first_time(&real) {
                error_at(&mut walked.report, Code::Lc0001, &shown, "cannot read this path: not a regular file".to_owned());
                walked.refused.push(shown);
            }
        } else if !has_program_extension(given) || !has_program_extension(&real) {
            if said.first_time(&real) {
                let message = "a file named explicitly, and what a link points to, must have the extension `.lcf` or `.lct`".to_owned();
                error_at(&mut walked.report, Code::Lc0008, &shown, message);
                walked.refused.push(shown);
            }
        } else {
            walked.files.push(Found { shown, path: real, through_a_link: is_a_link(given) });
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

fn walk_root(given: &Given<'_>, limits: &Limits, bounds: Bounds, said: &mut Said, walked: &mut Walked) {
    let mut root = Root {
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
        let entries = match fs::read_dir(path) {
            Ok(entries) => entries,
            Err(error) => return self.fail(path, shown, &error),
        };
        let mut sub_folders: Vec<(String, PathBuf)> = Vec::new();
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    self.fail(path, shown, &error);
                    continue;
                }
            };
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(error) => {
                    let entry_path = entry.path();
                    self.fail(&entry_path, &child_shown(shown, &name), &error);
                    continue;
                }
            };
            // A folder of a million other files costs no path and no string.
            if file_type.is_symlink() {
                let entry_path = entry.path();
                if self.said.first_time_in_a_walk(&entry_path) {
                    let entry_shown = self.shown_in(shown, &name, &entry_path);
                    self.report.push(Diagnostic::new(Code::Lc0009, "symbolic link: not followed", entry_shown));
                }
            } else if file_type.is_dir() {
                if !name.starts_with('.') {
                    let entry_path = entry.path();
                    sub_folders.push((self.shown_in(shown, &name, &entry_path), entry_path));
                    if sub_folders.len() > self.bounds.files.saturating_mul(2) {
                        self.too_many_folders |= keep_first(&mut sub_folders, self.bounds.files);
                    }
                }
            } else if has_program_extension(Path::new(&file_name)) {
                let entry_path = entry.path();
                let entry_shown = self.shown_in(shown, &name, &entry_path);
                self.file(&entry, entry_path, entry_shown, file_type.is_file());
            }
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

    fn file(&mut self, entry: &fs::DirEntry, path: PathBuf, shown: String, regular: bool) {
        let (size, refused) = if regular {
            match entry.metadata() {
                // A file over its own limit counts only for what would be read of it.
                Ok(metadata) => (metadata.len().min(self.limits.max_file_size.saturating_add(1)), None),
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
    fn finish(mut self, shown: &str, files: &mut Vec<Found>, refused: &mut Vec<String>) {
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
        if self.entries.is_empty() && !self.failed {
            error_at(self.report, Code::Lc0002, shown, "nothing to check: no `.lcf` or `.lct` file under this path".to_owned());
        }
        for entry in self.entries.drain(..) {
            match entry.refused {
                None => files.push(Found { shown: entry.shown, path: entry.path, through_a_link: self.through_a_link }),
                Some(why) => {
                    if self.said.first_time_in_a_walk(&entry.path) {
                        error_at(self.report, Code::Lc0001, &entry.shown, format!("cannot read this path: {why}"));
                        refused.push(entry.shown);
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

    /// A folder of its own under the temporary folder of the system, removed at the end of the test.
    struct Folder(PathBuf);

    impl Folder {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("labcraft-walk-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Folder(fs::canonicalize(path).unwrap())
        }

        fn file(&self, relative: &str, bytes: &[u8]) -> &Self {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
            self
        }

        fn walk(&self) -> Walked {
            walk(std::slice::from_ref(&self.0), Some(&self.0), &Limits::default())
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn shown(path: &Path, current: Option<&Path>) -> String {
        shown_path(path, fs::canonicalize(path).ok().as_deref(), current)
    }

    fn shown_files(walked: &Walked) -> Vec<&str> {
        walked.files.iter().map(|file| file.shown.as_str()).collect()
    }

    fn codes(walked: &mut Walked) -> Vec<&'static str> {
        walked.report.finish();
        walked.report.diagnostics().iter().map(|diagnostic| diagnostic.code().as_str()).collect()
    }

    #[test]
    fn a_path_is_shown_relative_to_the_current_folder_when_it_is_under_it() {
        let folder = Folder::new("shown");
        folder.file("a/b.lcf", b"");
        let current = Some(folder.0.as_path());
        assert_eq!(shown(&folder.0.join("a").join("b.lcf"), current), "a/b.lcf");
        assert_eq!(shown(&folder.0.join("a").join("..").join("a").join("b.lcf"), current), "a/b.lcf");
        assert_eq!(shown(&folder.0, current), ".");
        assert_eq!(shown(&folder.0.join("a").join(".."), current), ".");
        assert_eq!(shown(&folder.0.join("missing"), current), "missing");
        assert_eq!(shown(&folder.0.join("missing").join("x.lcf"), current), "missing/x.lcf");
        assert_eq!(shown(Path::new(""), current), "\"\"");
        let elsewhere = shown(&folder.0, Some(&folder.0.join("a")));
        assert!(elsewhere.contains("labcraft-walk-shown") && !elsewhere.contains('\\'), "{elsewhere}");
    }

    #[test]
    fn what_a_parent_folder_reaches_under_the_current_folder_is_relative_to_it() {
        let folder = Folder::new("parent");
        folder.file("a.lcf", b"").file("sub/b.lcf", b"");
        let sub = folder.0.join("sub");
        let walked = walk(std::slice::from_ref(&folder.0), Some(&sub), &Limits::default());
        let names = shown_files(&walked);
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"b.lcf"), "{names:?}");
        assert!(names.iter().any(|name| name.ends_with("/a.lcf")), "{names:?}");
    }

    #[test]
    fn files_come_in_the_order_of_their_paths_not_of_their_folders() {
        let folder = Folder::new("order");
        folder.file("a.lcf", b"").file("a/x.lct", b"").file("a-b.lcf", b"").file("a0.lcf", b"").file("Z.lcf", b"").file("note.txt", b"");
        let mut walked = folder.walk();
        assert_eq!(shown_files(&walked), ["Z.lcf", "a-b.lcf", "a.lcf", "a/x.lct", "a0.lcf"]);
        assert!(codes(&mut walked).is_empty());
    }

    #[test]
    fn a_folder_that_starts_with_a_dot_is_skipped_unless_it_is_named() {
        let folder = Folder::new("hidden");
        folder.file(".hidden/a.lcf", b"").file("b.lcf", b"").file(".c.lcf", b"");
        assert_eq!(shown_files(&folder.walk()), [".c.lcf", "b.lcf"]);
        let walked = walk(&[folder.0.join(".hidden")], Some(&folder.0), &Limits::default());
        assert_eq!(shown_files(&walked), [".hidden/a.lcf"]);
    }

    #[test]
    fn a_file_named_twice_or_through_its_folder_counts_once() {
        let folder = Folder::new("twice");
        folder.file("sub/a.lcf", b"");
        let file = folder.0.join("sub").join("a.lcf");
        let sub = folder.0.join("sub");
        let dots = folder.0.join("sub").join("..").join("sub").join("a.lcf");
        let mut walked = walk(&[file.clone(), file, dots, sub.clone(), folder.0.clone(), sub], Some(&folder.0), &Limits::default());
        assert_eq!(shown_files(&walked), ["sub/a.lcf"]);
        assert!(codes(&mut walked).is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn a_folder_spelled_in_another_case_is_the_same_folder() {
        let folder = Folder::new("case");
        folder.file("sub/a.lcf", b"");
        let walked = walk(&[folder.0.join("SUB").join("a.lcf"), folder.0.join("sub").join("a.lcf")], Some(&folder.0), &Limits::default());
        assert_eq!(shown_files(&walked), ["sub/a.lcf"]);
        let walked = walk(&[folder.0.join("SUB"), folder.0.clone()], Some(&folder.0), &Limits::default());
        assert_eq!(shown_files(&walked), ["sub/a.lcf"]);
    }

    #[cfg(windows)]
    #[test]
    fn a_stream_of_a_file_is_not_a_program() {
        let folder = Folder::new("stream");
        folder.file("notes.txt", b"");
        let stream = folder.0.join("notes.txt:a.lcf");
        fs::write(&stream, b"").unwrap();
        let mut walked = walk(&[stream], Some(&folder.0), &Limits::default());
        assert!(walked.files.is_empty());
        assert_eq!(codes(&mut walked), ["LC0008"]);
    }

    #[test]
    fn naming_a_folder_inside_another_loses_nothing_and_it_has_its_own_bounds() {
        let folder = Folder::new("inside");
        // Too deep for the folder given first, not for the one given inside it.
        folder.file(&format!("{}/deep.lcf", nested(BOUNDS.levels)), b"").file("a.lcf", b"");
        let inner = folder.0.join("d");
        let mut walked = walk(&[folder.0.clone(), inner.clone()], Some(&folder.0), &Limits::default());
        assert_eq!(walked.files.len(), 2);
        assert_eq!(codes(&mut walked), ["LC0010"]);
        let mut reversed = walk(&[inner, folder.0.clone()], Some(&folder.0), &Limits::default());
        assert_eq!(reversed.files, walked.files);
        assert_eq!(codes(&mut reversed), ["LC0010"]);
    }

    #[test]
    fn a_folder_inside_another_with_nothing_to_check_says_so_whatever_the_order() {
        let folder = Folder::new("inside-empty");
        folder.file("a.lcf", b"").file("empty/note.txt", b"");
        let empty = folder.0.join("empty");
        let mut one = walk(&[folder.0.clone(), empty.clone()], Some(&folder.0), &Limits::default());
        let mut other = walk(&[empty, folder.0.clone()], Some(&folder.0), &Limits::default());
        assert_eq!(codes(&mut one), ["LC0002"]);
        assert_eq!(codes(&mut other), ["LC0002"]);
    }

    #[test]
    fn the_order_of_the_arguments_changes_nothing() {
        let folder = Folder::new("arguments");
        folder.file("a.lcf", b"").file("empty/note.txt", b"").file("full/b.lcf", b"");
        let paths = [folder.0.clone(), folder.0.join("empty"), folder.0.join("full"), folder.0.join("missing")];
        let mut one = walk(&paths, Some(&folder.0), &Limits::default());
        let reversed: Vec<PathBuf> = paths.iter().rev().cloned().collect();
        let mut other = walk(&reversed, Some(&folder.0), &Limits::default());
        assert_eq!(one.files, other.files);
        assert_eq!(codes(&mut one), ["LC0002", "LC0001"]);
        assert_eq!(one.report.diagnostics(), {
            other.report.finish();
            other.report.diagnostics()
        });
    }

    #[test]
    fn a_name_that_only_ends_like_an_extension_has_none() {
        let folder = Folder::new("extension");
        folder.file(".lcf", b"").file("a.LCF", b"").file("b.lcf.txt", b"").file("c.lct", b"");
        assert_eq!(shown_files(&folder.walk()), ["c.lct"]);
        let mut named = walk(&[folder.0.join("a.LCF")], Some(&folder.0), &Limits::default());
        assert_eq!(named.refused, ["a.LCF"]);
        assert_eq!(codes(&mut named), ["LC0008"]);
    }

    #[test]
    fn nothing_found_is_an_error_for_each_path() {
        let folder = Folder::new("nothing");
        folder.file("empty/note.txt", b"").file("full/a.lcf", b"");
        let paths = [folder.0.join("empty"), folder.0.join("full"), folder.0.join("missing")];
        let mut walked = walk(&paths, Some(&folder.0), &Limits::default());
        assert_eq!(shown_files(&walked), ["full/a.lcf"]);
        assert!(walked.refused.is_empty(), "a folder or a missing path is not a file");
        assert_eq!(codes(&mut walked), ["LC0002", "LC0001"]);
    }

    fn nested(levels: usize) -> String {
        vec!["d"; levels].join("/")
    }

    #[test]
    fn folders_stop_at_32_levels() {
        let folder = Folder::new("levels");
        // The folder given is the first level.
        folder.file(&format!("{}/kept.lcf", nested(BOUNDS.levels - 1)), b"");
        let mut walked = folder.walk();
        assert_eq!(walked.files.len(), 1);
        assert!(codes(&mut walked).is_empty());
        folder.file(&format!("{}/lost.lcf", nested(BOUNDS.levels)), b"");
        let mut walked = folder.walk();
        assert_eq!(walked.files.len(), 1);
        assert_eq!(codes(&mut walked), ["LC0010"]);
    }

    #[test]
    fn a_folder_with_only_what_is_left_out_is_not_said_to_be_empty() {
        let folder = Folder::new("only-deep");
        folder.file(&format!("{}/lost.lcf", nested(BOUNDS.levels)), b"");
        assert_eq!(codes(&mut folder.walk()), ["LC0010"]);
    }

    fn small(folder: &Folder, files: usize) -> Walked {
        walk_within(std::slice::from_ref(&folder.0), Some(&folder.0), &Limits::default(), Bounds { levels: 32, files })
    }

    #[test]
    fn the_files_that_stay_are_the_first_in_the_order_of_the_paths() {
        let folder = Folder::new("files");
        for name in ["d", "b", "e", "a", "c", "g", "f"] {
            folder.file(&format!("{name}.lcf"), b"");
        }
        let mut walked = small(&folder, 3);
        assert_eq!(shown_files(&walked), ["a.lcf", "b.lcf", "c.lcf"]);
        assert_eq!(codes(&mut walked), ["LC0010"]);
    }

    #[test]
    fn a_refused_file_counts_as_a_file() {
        let folder = Folder::new("exactly");
        folder.file("a.lcf", b"").file("b.lcf", b"");
        let mut walked = small(&folder, 2);
        assert_eq!(shown_files(&walked).len(), 2);
        assert!(codes(&mut walked).is_empty());
    }

    #[test]
    fn folders_are_bounded_too() {
        let folder = Folder::new("folders");
        for name in ["a", "b", "c", "d"] {
            folder.file(&format!("{name}/x.lcf"), b"");
        }
        // The folder given and two of those in it.
        let mut walked = small(&folder, 3);
        assert_eq!(shown_files(&walked), ["a/x.lcf", "b/x.lcf"]);
        assert_eq!(codes(&mut walked), ["LC0010"]);
    }

    #[test]
    fn the_bytes_of_a_folder_are_bounded_in_the_order_of_the_paths() {
        let folder = Folder::new("bytes");
        folder.file("a.lcf", &[0; 6]).file("b.lcf", &[0; 4]).file("c.lcf", &[0; 1]).file("d.lcf", &[0; 1]);
        let mut limits = Limits::default();
        limits.max_library_size = 10;
        let mut walked = walk(std::slice::from_ref(&folder.0), Some(&folder.0), &limits);
        assert_eq!(shown_files(&walked), ["a.lcf", "b.lcf"]);
        assert_eq!(codes(&mut walked), ["LC0010"]);
    }

    #[test]
    fn a_file_over_its_own_limit_counts_only_for_what_would_be_read() {
        let folder = Folder::new("large");
        folder.file("a.lcf", &[0; 100]).file("b.lcf", &[0; 1]);
        let mut limits = Limits::default();
        limits.max_library_size = 10;
        limits.max_file_size = 5;
        let mut walked = walk(std::slice::from_ref(&folder.0), Some(&folder.0), &limits);
        assert_eq!(shown_files(&walked), ["a.lcf", "b.lcf"]);
        assert!(codes(&mut walked).is_empty());
    }

    #[test]
    fn what_stays_does_not_depend_on_when_the_list_was_cut() {
        let mut items: Vec<u32> = (0..100).rev().collect();
        assert!(keep_first(&mut items, 3));
        items.extend([7, 1]);
        assert!(keep_first(&mut items, 3));
        assert_eq!(items, [0, 1, 1]);
        assert!(!keep_first(&mut items, 3));
    }

    #[test]
    fn an_error_of_the_system_says_its_cause_alone() {
        let wrapped = io::Error::new(io::ErrorKind::PermissionDenied, "cannot save C:\\a\\long\\path: Accès refusé");
        assert_eq!(io_text(&wrapped), "permission denied");
        assert_eq!(io_text(&io::Error::from(io::ErrorKind::NotFound)), "entity not found");
    }

    #[cfg(unix)]
    #[test]
    fn what_two_paths_reach_is_said_once() {
        let folder = Folder::new("said-once");
        let fifo = folder.0.join("pipe.lcf");
        assert!(std::process::Command::new("mkfifo").arg(&fifo).status().unwrap().success());
        std::os::unix::fs::symlink(&folder.0, folder.0.join("loop")).unwrap();
        fs::create_dir(folder.0.join("sub")).unwrap();
        std::os::unix::fs::symlink(&folder.0, folder.0.join("sub").join("loop")).unwrap();
        let paths = [folder.0.clone(), fifo.clone(), folder.0.join("sub"), fifo];
        let walked = walk(&paths, Some(&folder.0), &Limits::default());
        assert_eq!(walked.refused, ["pipe.lcf"]);
        assert_eq!(walked.report.errors(), 2, "the file, and `sub`, which holds nothing");
        assert_eq!(walked.report.warnings(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn a_refused_file_is_known_by_its_real_path() {
        let folder = Folder::new("refused-once");
        fs::create_dir(folder.0.join("sub")).unwrap();
        let sub = folder.0.join("sub");
        let fifo = sub.join("pipe.lcf");
        assert!(std::process::Command::new("mkfifo").arg(&fifo).status().unwrap().success());
        let spelled = sub.join(".").join("pipe.lcf");
        let mut walked = walk(&[folder.0.clone(), fifo, spelled], Some(&sub), &Limits::default());
        assert_eq!(walked.refused, ["pipe.lcf"]);
        assert_eq!(codes(&mut walked), ["LC0001"]);
    }

    #[cfg(unix)]
    #[test]
    fn two_names_that_show_alike_are_two_files() {
        use std::os::unix::ffi::OsStrExt as _;
        let folder = Folder::new("not-unicode");
        let names = [folder.0.join(std::ffi::OsStr::from_bytes(b"a\xFF.txt")), folder.0.join(std::ffi::OsStr::from_bytes(b"a\xFE.txt"))];
        for name in &names {
            fs::write(name, b"").unwrap();
        }
        let mut walked = walk(&names, Some(&folder.0), &Limits::default());
        assert_eq!(walked.refused.len(), 2);
        assert_eq!(codes(&mut walked), ["LC0008", "LC0008"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_special_file_with_the_extension_is_refused_never_read_and_counted() {
        let folder = Folder::new("fifo");
        let fifo = folder.0.join("pipe.lcf");
        assert!(std::process::Command::new("mkfifo").arg(&fifo).status().unwrap().success());
        let mut walked = folder.walk();
        assert_eq!(walked.refused, ["pipe.lcf"]);
        assert_eq!(codes(&mut walked), ["LC0001"]);
        let mut walked = walk(std::slice::from_ref(&fifo), Some(&folder.0), &Limits::default());
        assert_eq!(walked.refused, ["pipe.lcf"]);
        assert_eq!(codes(&mut walked), ["LC0001"]);
        assert!(std::process::Command::new("mkfifo").arg(folder.0.join("other.lcf")).status().unwrap().success());
        let mut walked = small(&folder, 1);
        assert_eq!(walked.refused, ["other.lcf"]);
        assert_eq!(codes(&mut walked), ["LC0010", "LC0001"]);
    }
}
