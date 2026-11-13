//! A check of folders as `labcraft-cli check` makes it (`docs/cli.md`, section 3): every folder given, walked within
//! the bounds of a library, its libraries found by the rules of section 3, among them those of `--lib-path`. The logic
//! stays in `labcraft-check`; here are the folders, the files and the libraries, through the [`Workspace`].

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet};
use std::ffi::OsString;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use labcraft_check::{Checked, Content, DependencyReport, Found as FoundLibrary, Libraries, LibrarySource, Origin, SourceFile, check, check_for_run};
use labcraft_types::{CheckCounts, Code, Diagnostic, Interner, Limits, Report, Severity, normalize};

use crate::kept::Seen;
use crate::panics::last_panic;
use crate::shipped::Shipped;
use crate::walk::{self, Found, Refused, Walked, io_text, read_program, read_program_stamped, shown_of, slashed};
use crate::workspace::{Kind, Workspace};

/// What to check, and where to look for libraries.
pub struct Folders<'a> {
    pub folders: &'a [PathBuf],
    pub lib_paths: &'a [PathBuf],
    /// What paths are shown from: the current folder of the command line, the root of a project.
    pub current: Option<PathBuf>,
    pub limits: &'a Limits,
}

/// A check of folders, and what a session keeps of it.
pub struct FoldersChecked {
    pub report: Report,
    pub counts: CheckCounts,
    /// What `check` found in each folder that read, with the names it interned.
    pub checked: Vec<(Checked, Interner)>,
    /// What the check read and found, as a session compares it later (`kept`).
    pub seen: Seen,
}

/// Checks the folders: the report of every one, and what they hold, counted.
pub fn check_folders(ws: &dyn Workspace, shipped: &Shipped, given: &Folders<'_>) -> (Report, CheckCounts) {
    let checked = check_folders_keeping(ws, shipped, given, false, false);
    (checked.report, checked.counts)
}

/// As [`check_folders`], keeping what `check` found when `keep` is true, and noting what it saw when `stamp` is.
pub fn check_folders_keeping(ws: &dyn Workspace, shipped: &Shipped, given: &Folders<'_>, keep: bool, stamp: bool) -> FoldersChecked {
    let current = given.current.clone();
    let limits = given.limits;
    // Every folder given is walked, so that naming one never loses what naming it alone would keep; each file then
    // belongs to the outermost of them.
    let mut walked = walk::walk(ws, given.folders, current.as_deref(), limits);
    let seen = RefCell::new(Seen::of_walk(&walked));
    // What a walk could not read, a dependency in the same place cannot either, and it is said once.
    let mut reported: HashSet<String> =
        walked.report.diagnostics().iter().filter(|diagnostic| diagnostic.code() == Code::Lc0001).map(|diagnostic| diagnostic.file.clone()).collect();
    let in_the_walk: HashSet<PathBuf> = if given.lib_paths.is_empty() {
        HashSet::new()
    } else {
        walked.files.iter().map(|found| found.path.clone()).chain(walked.refused.iter().map(|refused| refused.path.clone())).collect()
    };
    let mut report = std::mem::replace(&mut walked.report, Report::for_limits(limits));
    let roots = roots(ws, given.folders, current.as_deref());
    let lib_paths = readable_lib_paths(ws, given.folders, given.lib_paths, current.as_deref(), &mut report);
    seen.borrow_mut().lib_paths.clone_from(&lib_paths);
    let host =
        Host { ws, shipped, lib_paths: &lib_paths, in_the_walk: &in_the_walk, current: current.as_deref(), limits, seen: stamp.then_some(&seen) };
    let mut counts = CheckCounts::default();
    let mut kept = Vec::new();
    for root in &roots {
        let Some(source) = library_of(ws, &root.real, &root.shown, &mut walked, limits, stamp.then_some(&seen)) else { continue };
        let (folder_report, folder_counts, dependencies, checked) = check_folder(source, &host, limits, &root.shown);
        if let (true, Some(checked)) = (keep, checked) {
            kept.push(checked);
        }
        report.merge(folder_report);
        counts.libraries += folder_counts.libraries;
        counts.functions += folder_counts.functions;
        counts.types += folder_counts.types;
        for DependencyReport { shown, report: dependency } in dependencies {
            if reported.insert(shown) {
                report.merge(dependency);
            }
        }
    }
    FoldersChecked { report, counts, checked: kept, seen: seen.into_inner() }
}

/// What a run checks before it is lowered (`docs/cli.md`, 6.1): the folder, as `library.check` checks it, and the bodies of
/// every library it uses (`check::check_for_run`).
pub struct RunChecked {
    /// The report of the folder, what its walk and its dependencies said included; the bodies of the dependencies stay in
    /// `Checked::bodies`, for the run that reaches them.
    pub report: Report,
    pub counts: CheckCounts,
    /// `None` when the folder holds nothing to check, or `check` met a bug, which the report says.
    pub checked: Option<(Checked, Interner)>,
    /// The SHA-256 of the bytes of each file of the folder that read, by how reports show it, for the first line of a
    /// trace (commands.md 8).
    pub hashes: Vec<(String, String)>,
}

/// Checks one folder for a run, its paths shown from `current`.
pub fn check_folder_for_run(
    ws: &dyn Workspace,
    shipped: &Shipped,
    folder: &Path,
    lib_paths: &[PathBuf],
    current: &Path,
    limits: &Limits,
) -> RunChecked {
    let folders = [folder.to_path_buf()];
    let mut walked = walk::walk(ws, &folders, Some(current), limits);
    let in_the_walk: HashSet<PathBuf> = if lib_paths.is_empty() {
        HashSet::new()
    } else {
        walked.files.iter().map(|found| found.path.clone()).chain(walked.refused.iter().map(|refused| refused.path.clone())).collect()
    };
    let mut report = std::mem::replace(&mut walked.report, Report::for_limits(limits));
    let lib_paths = readable_lib_paths(ws, &folders, lib_paths, Some(current), &mut report);
    let host = Host { ws, shipped, lib_paths: &lib_paths, in_the_walk: &in_the_walk, current: Some(current), limits, seen: None };
    let mut counts = CheckCounts::default();
    let mut checked = None;
    let mut hashes = Vec::new();
    for root in roots(ws, &folders, Some(current)) {
        let Some(source) = library_of(ws, &root.real, &root.shown, &mut walked, limits, None) else { continue };
        hash_files(&source, &mut hashes);
        let work = || {
            let mut names = Interner::new();
            let mut found = check_for_run(source, &host, &mut names, limits);
            let report = std::mem::replace(&mut found.report, Report::for_limits(limits));
            let dependencies = std::mem::take(&mut found.dependencies);
            (report, found.counts, dependencies, (found, names))
        };
        match panic::catch_unwind(AssertUnwindSafe(work)) {
            Ok((folder_report, folder_counts, dependencies, found)) => {
                report.merge(folder_report);
                counts.libraries += folder_counts.libraries;
                counts.functions += folder_counts.functions;
                counts.types += folder_counts.types;
                for DependencyReport { report: dependency, .. } in dependencies {
                    report.merge(dependency);
                }
                checked = Some(found);
            }
            Err(_) => {
                let message = format!("internal error, which is a bug of the tool: {}", last_panic());
                report.push(Diagnostic::new(Code::Lc0099, message, root.shown.as_str()));
            }
        }
    }
    RunChecked { report, counts, checked, hashes }
}

pub(crate) fn hash_files(source: &LibrarySource, hashes: &mut Vec<(String, String)>) {
    let mut pending = vec![source];
    while let Some(library) = pending.pop() {
        for file in &library.files {
            if let Content::Bytes(bytes) = &file.content {
                hashes.push((file.shown.clone(), labcraft_trace::sha256_hex(bytes)));
            }
        }
        pending.extend(&library.sub_libraries);
    }
}

/// A folder given with nothing around it among those given. One inside another is one of its sub-libraries.
pub struct Root {
    pub real: PathBuf,
    pub shown: String,
}

pub fn roots(ws: &dyn Workspace, folders: &[PathBuf], current: Option<&Path>) -> Vec<Root> {
    let current = current.and_then(|current| ws.canonicalize(current).ok());
    let mut known: Vec<Root> = folders
        .iter()
        .filter_map(|folder| {
            let real = ws.canonicalize(folder).ok()?;
            let shown = shown_of(ws, folder, Some(&real), current.as_deref());
            Some(Root { real, shown })
        })
        .collect();
    known.sort_by(|a, b| (&a.real, &a.shown).cmp(&(&b.real, &b.shown)));
    let mut kept: Vec<Root> = Vec::new();
    for root in known {
        if !kept.iter().any(|outer| root.real.starts_with(&outer.real)) {
            kept.push(root);
        }
    }
    kept
}

/// A `--lib-path` that cannot be listed is reported once, and is no place to look for a library: it must not hide the
/// next one, nor those shipped. One that also names a folder given has been reported by the walk.
pub fn readable_lib_paths(
    ws: &dyn Workspace,
    folders: &[PathBuf],
    lib_paths: &[PathBuf],
    current: Option<&Path>,
    report: &mut Report,
) -> Vec<PathBuf> {
    let canonical_current = current.and_then(|current| ws.canonicalize(current).ok());
    let shown = |path: &Path| shown_of(ws, path, ws.canonicalize(path).ok().as_deref(), canonical_current.as_deref());
    let mut said: HashSet<String> = folders.iter().map(|folder| shown(folder)).collect();
    let mut places = HashSet::new();
    let mut readable = Vec::new();
    for path in lib_paths {
        match ws.read_dir(path, &mut |_| {}) {
            Ok(()) => {
                if places.insert(ws.canonicalize(path).unwrap_or_else(|_| path.clone())) {
                    readable.push(path.clone());
                }
            }
            Err(error) => {
                if said.insert(shown(path)) {
                    report.push(Diagnostic::new(Code::Lc0001, format!("cannot read this path: {}", io_text(&error)), shown(path)));
                }
            }
        }
    }
    readable
}

/// The files that the walk found under this folder, in the shape of its folders.
/// The files read are noted in `seen`, with what each read saw.
pub fn library_of(
    ws: &dyn Workspace,
    real: &Path,
    shown: &str,
    walked: &mut Walked,
    limits: &Limits,
    seen: Option<&RefCell<Seen>>,
) -> Option<LibrarySource> {
    let (mine, others): (Vec<Found>, Vec<Found>) = std::mem::take(&mut walked.files).into_iter().partition(|found| found.path.starts_with(real));
    walked.files = others;
    let (refused, others): (Vec<Refused>, Vec<Refused>) =
        std::mem::take(&mut walked.refused).into_iter().partition(|refused| refused.path.starts_with(real));
    walked.refused = others;
    if mine.is_empty() && refused.is_empty() {
        return None;
    }
    let mut tree = Tree::default();
    let mut report = Report::for_limits(limits);
    for found in mine {
        // Only a session that keeps the check needs what each read saw; the command line reads alone.
        let read = match seen {
            Some(seen) => {
                let (read, stamp) = read_program_stamped(ws, &found.path, limits);
                seen.borrow_mut().contents.insert(found.path.clone(), stamp);
                read
            }
            None => read_program(ws, &found.path, limits),
        };
        let content = match read {
            Ok(bytes) => Content::Bytes(Cow::Owned(bytes)),
            Err((code, message)) => {
                report.push(Diagnostic::new(code, message, found.shown.as_str()));
                Content::Unread
            }
        };
        tree.insert(found.path.strip_prefix(real).ok(), SourceFile { shown: found.shown, content });
    }
    for refused in refused {
        tree.insert(refused.path.strip_prefix(real).ok(), SourceFile { shown: refused.shown, content: Content::Unread });
    }
    let mut source = tree.into_source(String::new(), shown);
    source.report = report;
    Some(source)
}

/// The folders under the one given. A walk stops at 32 levels, which bounds the recursion here. Folders are told apart by
/// their names on disk: two that are not Unicode can show alike, and are still two.
#[derive(Default)]
struct Tree {
    files: Vec<SourceFile>,
    folders: BTreeMap<OsString, Tree>,
}

impl Tree {
    /// `relative` is the path of the file under this folder: a walk finds nothing else.
    fn insert(&mut self, relative: Option<&Path>, file: SourceFile) {
        let mut folder = self;
        let parts: Vec<OsString> = relative
            .and_then(Path::parent)
            .map(|parent| parent.components().map(|part| part.as_os_str().to_os_string()).collect())
            .unwrap_or_default();
        for part in parts {
            folder = folder.folders.entry(part).or_default();
        }
        folder.files.push(file);
    }

    fn into_source(self, name: String, shown: &str) -> LibrarySource {
        let sub_libraries = self
            .folders
            .into_iter()
            .map(|(sub, tree)| {
                let sub = sub.to_string_lossy().into_owned();
                let sub_shown = format!("{}/{sub}", shown.trim_end_matches('/'));
                tree.into_source(sub, &sub_shown)
            })
            .collect();
        LibrarySource { files: self.files, sub_libraries, ..LibrarySource::new(name, shown.to_owned(), Origin::Project) }
    }
}

/// What one folder gives: its report, its counts, what its dependencies say, and what `check` found.
type OneFolder = (Report, CheckCounts, Vec<DependencyReport>, Option<(Checked, Interner)>);

/// A panic that the crate did not catch is a bug, reported on this folder while the others go on.
fn check_folder(source: LibrarySource, host: &Host<'_>, limits: &Limits, shown: &str) -> OneFolder {
    let work = || {
        let mut names = Interner::new();
        let mut checked = check(source, host, &mut names, limits);
        let report = std::mem::replace(&mut checked.report, Report::for_limits(limits));
        let dependencies = std::mem::take(&mut checked.dependencies);
        (report, checked.counts, dependencies, Some((checked, names)))
    };
    panic::catch_unwind(AssertUnwindSafe(work)).unwrap_or_else(|_| {
        let mut report = Report::for_limits(limits);
        report.push(Diagnostic::new(Code::Lc0099, format!("internal error, which is a bug of the tool: {}", last_panic()), shown));
        (report, CheckCounts::default(), Vec::new(), None)
    })
}

/// Rules 3 and 4 of `docs/cli.md`, section 3.
pub struct Host<'a> {
    pub ws: &'a dyn Workspace,
    pub shipped: &'a Shipped,
    pub lib_paths: &'a [PathBuf],
    /// The files that a folder given has reached: they say what they have to say there.
    pub in_the_walk: &'a HashSet<PathBuf>,
    pub current: Option<&'a Path>,
    pub limits: &'a Limits,
    /// What the check looks for under `lib_paths` and reads, for a session that keeps it.
    pub seen: Option<&'a RefCell<Seen>>,
}

impl Libraries for Host<'_> {
    fn find(&self, name: &str, by: Origin) -> FoundLibrary {
        if by == Origin::Project {
            if let Some(seen) = self.seen {
                let missing = !seen.borrow().libraries.contains_key(name);
                if missing {
                    let (found, stamps) = crate::kept::look_for(self.ws, self.lib_paths, name, self.current, self.limits);
                    let mut seen = seen.borrow_mut();
                    seen.libraries.insert(name.to_owned(), found);
                    seen.contents.extend(stamps);
                }
            }
            for lib_path in self.lib_paths {
                match self.in_a_lib_path(lib_path, name) {
                    FoundLibrary::NotFound => {}
                    found => return found,
                }
            }
        }
        self.shipped.source(name).map_or(FoundLibrary::NotFound, FoundLibrary::Library)
    }

    fn last_panic(&self) -> Option<String> {
        Some(last_panic())
    }
}

impl Host<'_> {
    fn in_a_lib_path(&self, lib_path: &Path, name: &str) -> FoundLibrary {
        let folder = match locate(self.ws, lib_path, name) {
            Ok(Some(folder)) => folder,
            Ok(None) => return FoundLibrary::NotFound,
            Err(diagnostic) => return FoundLibrary::Unreadable(diagnostic),
        };
        let current = self.current.and_then(|current| self.ws.canonicalize(current).ok());
        let real = match self.ws.canonicalize(&folder) {
            Ok(real) => real,
            Err(error) => {
                let message = format!("cannot read this path: {}", io_text(&error));
                return FoundLibrary::Unreadable(Box::new(Diagnostic::new(
                    Code::Lc0001,
                    message,
                    shown_of(self.ws, &folder, None, current.as_deref()),
                )));
            }
        };
        let shown = shown_of(self.ws, &folder, Some(&real), current.as_deref());
        let mut walked = walk::walk_alone(self.ws, &folder, self.current, self.limits);
        let checked_elsewhere = !walked.files.is_empty() && walked.files.iter().all(|found| self.in_the_walk.contains(&found.path));
        let Some(mut source) = library_of(self.ws, &real, &shown, &mut walked, self.limits, self.seen) else {
            // Nothing to read: not this library, unless the folder would not say what it holds. The list can be cut,
            // and this diagnostic is what makes the error count.
            let failure = walked.report.diagnostics().iter().find(|diagnostic| diagnostic.severity() == Severity::Error);
            return match failure {
                Some(diagnostic) => FoundLibrary::Unreadable(Box::new(diagnostic.clone())),
                None if walked.report.errors() > 0 => FoundLibrary::Unreadable(Box::new(unreadable_folder(&shown))),
                None => FoundLibrary::NotFound,
            };
        };
        source.report.merge(walked.report);
        source.checked_elsewhere = checked_elsewhere;
        FoundLibrary::Library(source)
    }
}

fn unreadable_folder(shown: &str) -> Diagnostic {
    Diagnostic::new(Code::Lc0001, "cannot read this path", shown)
}

/// The folder of a library under a `--lib-path`. Names are compared normalized and exactly (R-20), even on a file system
/// that ignores case, so each level is listed rather than asked for. A link is not followed, as in any walk.
pub fn locate(ws: &dyn Workspace, lib_path: &Path, name: &str) -> Result<Option<PathBuf>, Box<Diagnostic>> {
    let mut folder = lib_path.to_path_buf();
    for segment in name.split('.') {
        let mut found = Vec::new();
        let mut failed = None;
        let listed = ws.read_dir(&folder, &mut |entry| match entry {
            Ok(entry) => {
                let is_a_folder = entry.kind.is_ok_and(|kind| kind == Kind::Folder);
                if is_a_folder && normalize(&entry.name.to_string_lossy()) == segment {
                    found.push(folder.join(&entry.name));
                }
            }
            Err(error) => {
                failed.get_or_insert(error);
            }
        });
        match listed {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(unreadable(&folder, &error)),
            Ok(()) => {}
        }
        if let Some(error) = failed {
            return Err(unreadable(&folder, &error));
        }
        // Two spellings of one name on a file system that keeps both: the same one on every machine.
        found.sort();
        let Some(first) = found.into_iter().next() else { return Ok(None) };
        folder = first;
    }
    Ok(Some(folder))
}

fn unreadable(path: &Path, error: &std::io::Error) -> Box<Diagnostic> {
    Box::new(Diagnostic::new(Code::Lc0001, format!("cannot read this path: {}", io_text(error)), slashed(path)))
}
