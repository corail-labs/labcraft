//! `labcraft-cli check` (`docs/cli.md`, section 3). The logic stays in `labcraft-check`; here are the folders, the files
//! and the libraries found through `--lib-path`.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashSet};
use std::ffi::OsString;
use std::fs;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use labcraft_check::{Content, DependencyReport, Found as FoundLibrary, Libraries, LibrarySource, Origin, SourceFile, check};
use labcraft_types::{CheckCounts, Code, Diagnostic, Interner, Limits, Report, Severity, Status, normalize};

use crate::args::CheckOptions;
use crate::fmt::last_panic;
use crate::output;
use crate::walk::{self, Found, Refused, Walked, io_text, read_program, shown_of, slashed};

pub fn run(options: &CheckOptions) -> ExitCode {
    let current = std::env::current_dir().ok();
    let limits = &options.limits;
    // Every folder given is walked, so that naming one never loses what naming it alone would keep; each file then
    // belongs to the outermost of them.
    let mut walked = walk::walk(&options.folders, current.as_deref(), limits);
    // What a walk could not read, a dependency in the same place cannot either, and it is said once.
    let mut reported: HashSet<String> =
        walked.report.diagnostics().iter().filter(|diagnostic| diagnostic.code() == Code::Lc0001).map(|diagnostic| diagnostic.file.clone()).collect();
    let in_the_walk: HashSet<PathBuf> = if options.lib_paths.is_empty() {
        HashSet::new()
    } else {
        walked.files.iter().map(|found| found.path.clone()).chain(walked.refused.iter().map(|refused| refused.path.clone())).collect()
    };
    let mut report = std::mem::replace(&mut walked.report, Report::new(limits.max_report));
    let roots = roots(&options.folders, current.as_deref());
    let lib_paths = readable_lib_paths(options, current.as_deref(), &mut report);
    let host = Host { lib_paths: &lib_paths, in_the_walk: &in_the_walk, current: current.as_deref(), limits };
    let mut counts = CheckCounts::default();
    for root in &roots {
        let Some(source) = library_of(&root.real, &root.shown, &mut walked, limits) else { continue };
        let (folder_report, folder_counts, dependencies) = check_folder(source, &host, limits, &root.shown);
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
    answer(options, report, counts)
}

/// When the work could not start or ended outside any folder, the answer keeps its form: a report, with `LC0099`.
pub fn internal_failure(options: &CheckOptions, what: &str) -> ExitCode {
    let mut report = Report::new(options.limits.max_report);
    report.push(Diagnostic::new(Code::Lc0099, format!("the tool could not finish: {what}"), "labcraft-cli"));
    answer(options, report, CheckCounts::default())
}

fn answer(options: &CheckOptions, mut report: Report, counts: CheckCounts) -> ExitCode {
    report.finish();
    if options.json {
        output::write_json(&report.check_document(counts, options.deny_warnings));
    } else {
        output::write_text(&output::check_text(&report, counts));
    }
    match report.status(options.deny_warnings) {
        Status::Ok => ExitCode::SUCCESS,
        Status::Error => ExitCode::FAILURE,
    }
}

/// A folder given with nothing around it among those given. One inside another is one of its sub-libraries.
struct Root {
    real: PathBuf,
    shown: String,
}

fn roots(folders: &[PathBuf], current: Option<&Path>) -> Vec<Root> {
    let current = current.and_then(|current| fs::canonicalize(current).ok());
    let mut known: Vec<Root> = folders
        .iter()
        .filter_map(|folder| {
            let real = fs::canonicalize(folder).ok()?;
            let shown = shown_of(folder, Some(&real), current.as_deref());
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
fn readable_lib_paths(options: &CheckOptions, current: Option<&Path>, report: &mut Report) -> Vec<PathBuf> {
    let canonical_current = current.and_then(|current| fs::canonicalize(current).ok());
    let shown = |path: &Path| shown_of(path, fs::canonicalize(path).ok().as_deref(), canonical_current.as_deref());
    let mut said: HashSet<String> = options.folders.iter().map(|folder| shown(folder)).collect();
    let mut places = HashSet::new();
    let mut readable = Vec::new();
    for path in &options.lib_paths {
        match fs::read_dir(path) {
            Ok(_) => {
                if places.insert(fs::canonicalize(path).unwrap_or_else(|_| path.clone())) {
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
fn library_of(real: &Path, shown: &str, walked: &mut Walked, limits: &Limits) -> Option<LibrarySource> {
    let (mine, others): (Vec<Found>, Vec<Found>) = std::mem::take(&mut walked.files).into_iter().partition(|found| found.path.starts_with(real));
    walked.files = others;
    let (refused, others): (Vec<Refused>, Vec<Refused>) =
        std::mem::take(&mut walked.refused).into_iter().partition(|refused| refused.path.starts_with(real));
    walked.refused = others;
    if mine.is_empty() && refused.is_empty() {
        return None;
    }
    let mut tree = Tree::default();
    let mut report = Report::new(limits.max_report);
    for found in mine {
        let content = match read_program(&found.path, limits) {
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

/// A panic that the crate did not catch is a bug, reported on this folder while the others go on.
fn check_folder(source: LibrarySource, host: &Host<'_>, limits: &Limits, shown: &str) -> (Report, CheckCounts, Vec<DependencyReport>) {
    let work = || {
        let mut names = Interner::new();
        let checked = check(source, host, &mut names, limits);
        (checked.report, checked.counts, checked.dependencies)
    };
    panic::catch_unwind(AssertUnwindSafe(work)).unwrap_or_else(|_| {
        let mut report = Report::new(limits.max_report);
        report.push(Diagnostic::new(Code::Lc0099, format!("internal error, which is a bug of the tool: {}", last_panic()), shown));
        (report, CheckCounts::default(), Vec::new())
    })
}

/// Rules 3 and 4 of `docs/cli.md`, section 3.
struct Host<'a> {
    lib_paths: &'a [PathBuf],
    /// The files that a folder given has reached: they say what they have to say there.
    in_the_walk: &'a HashSet<PathBuf>,
    current: Option<&'a Path>,
    limits: &'a Limits,
}

impl Libraries for Host<'_> {
    fn find(&self, name: &str, by: Origin) -> FoundLibrary {
        if by == Origin::Project {
            for lib_path in self.lib_paths {
                match self.in_a_lib_path(lib_path, name) {
                    FoundLibrary::NotFound => {}
                    found => return found,
                }
            }
        }
        labcraft_libs::source(name).map_or(FoundLibrary::NotFound, FoundLibrary::Library)
    }

    fn last_panic(&self) -> Option<String> {
        Some(last_panic())
    }
}

impl Host<'_> {
    fn in_a_lib_path(&self, lib_path: &Path, name: &str) -> FoundLibrary {
        let folder = match locate(lib_path, name) {
            Ok(Some(folder)) => folder,
            Ok(None) => return FoundLibrary::NotFound,
            Err(diagnostic) => return FoundLibrary::Unreadable(diagnostic),
        };
        let current = self.current.and_then(|current| fs::canonicalize(current).ok());
        let real = match fs::canonicalize(&folder) {
            Ok(real) => real,
            Err(error) => {
                let message = format!("cannot read this path: {}", io_text(&error));
                return FoundLibrary::Unreadable(Box::new(Diagnostic::new(Code::Lc0001, message, shown_of(&folder, None, current.as_deref()))));
            }
        };
        let shown = shown_of(&folder, Some(&real), current.as_deref());
        let mut walked = walk::walk_alone(&folder, self.current, self.limits);
        let checked_elsewhere = !walked.files.is_empty() && walked.files.iter().all(|found| self.in_the_walk.contains(&found.path));
        let Some(mut source) = library_of(&real, &shown, &mut walked, self.limits) else {
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
fn locate(lib_path: &Path, name: &str) -> Result<Option<PathBuf>, Box<Diagnostic>> {
    let mut folder = lib_path.to_path_buf();
    for segment in name.split('.') {
        let entries = match fs::read_dir(&folder) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(unreadable(&folder, &error)),
        };
        let mut found = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| unreadable(&folder, &error))?;
            let is_a_folder = entry.file_type().is_ok_and(|file_type| file_type.is_dir());
            if is_a_folder && normalize(&entry.file_name().to_string_lossy()) == segment {
                found.push(entry.path());
            }
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
