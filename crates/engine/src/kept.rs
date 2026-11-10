//! The check that the session keeps (`docs/commands.md`, 6.2; decisions of the owner, 2026-11-10): the last report of
//! each folder, with what that check saw. When nothing changed, the report is given at once; when something did, the
//! whole folder is checked again, so the answer is always the one a full check gives. Keeping work library by library
//! is for later, when a measure asks for it.
//!
//! What a check saw is noted while it runs, so that a file changed while the check reads it is seen next time: the files
//! the walk of the folder found or refused, and what it said; the folders of `--lib-path` that could be listed; for each
//! library looked for under them, the folder found under each, and the files there; and for each file read, the SHA-256
//! of its bytes, the size that refused it, or the error of the system. A command that reports a check looks again the
//! same way, reading every file, and compares. Size and time of change would be cheaper, but a file rewritten with the
//! same size within one tick of the clock of the file system keeps both, and the report would be wrong.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use labcraft_types::{Code, Limits, Report};

use crate::folders::{FoldersChecked, locate, readable_lib_paths};
use crate::walk::{self, ContentStamp, Walked, read_program_stamped};
use crate::workspace::Workspace;

/// A folder of a library under one `--lib-path`, the files a walk of it found or refused and what it said; or why the
/// search could not tell.
type Located = Result<Option<(PathBuf, Vec<PathBuf>, Vec<(Code, String)>)>, String>;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Seen {
    /// The files that the walk found or refused, in order.
    pub listing: Vec<PathBuf>,
    /// What the walk said, by code and file.
    pub said: Vec<(Code, String)>,
    /// The folders of `--lib-path` that could be listed.
    pub lib_paths: Vec<PathBuf>,
    /// Each library looked for, and what is under each folder of `lib_paths` by its name.
    pub libraries: BTreeMap<String, Vec<Located>>,
    /// What each read saw.
    pub contents: BTreeMap<PathBuf, ContentStamp>,
}

impl Seen {
    pub fn of_walk(walked: &Walked) -> Seen {
        let mut listing: Vec<PathBuf> =
            walked.files.iter().map(|found| found.path.clone()).chain(walked.refused.iter().map(|refused| refused.path.clone())).collect();
        listing.sort();
        let said = walked.report.diagnostics().iter().map(|diagnostic| (diagnostic.code(), diagnostic.file.clone())).collect();
        Seen { listing, said, ..Seen::default() }
    }
}

/// What is under each folder of `lib_paths` by this name, and what a read of each file found there sees: the check reads
/// those of one of them, and a session reads them all, so that each side notes the same.
pub fn look_for(
    ws: &dyn Workspace,
    lib_paths: &[PathBuf],
    name: &str,
    current: Option<&Path>,
    limits: &Limits,
) -> (Vec<Located>, Vec<(PathBuf, ContentStamp)>) {
    let mut read = Vec::new();
    let located = lib_paths
        .iter()
        .map(|lib_path| {
            let folder = match locate(ws, lib_path, name) {
                Ok(Some(folder)) => folder,
                Ok(None) => return Ok(None),
                Err(diagnostic) => return Err(diagnostic.message),
            };
            let walked = walk::walk_alone(ws, &folder, current, limits);
            read.extend(walked.files.iter().map(|found| (found.path.clone(), read_program_stamped(ws, &found.path, limits).1)));
            let said = walked.report.diagnostics().iter().map(|diagnostic| (diagnostic.code(), diagnostic.file.clone())).collect();
            let mut listing: Vec<PathBuf> =
                walked.files.into_iter().map(|found| found.path).chain(walked.refused.into_iter().map(|refused| refused.path)).collect();
            listing.sort();
            Ok(Some((folder, listing, said)))
        })
        .collect();
    (located, read)
}

/// Looks again as the check of `folder` did, reading every file it would read.
fn look_again(
    ws: &dyn Workspace,
    folder: &Path,
    lib_paths: &[PathBuf],
    names: impl Iterator<Item = String>,
    current: Option<&Path>,
    limits: &Limits,
) -> Seen {
    let walked = walk::walk(ws, std::slice::from_ref(&folder.to_path_buf()), current, limits);
    let mut seen = Seen::of_walk(&walked);
    let read: Vec<PathBuf> = walked.files.into_iter().map(|found| found.path).collect();
    let mut scratch = Report::for_limits(limits);
    seen.lib_paths = readable_lib_paths(ws, std::slice::from_ref(&folder.to_path_buf()), lib_paths, current, &mut scratch);
    for name in names {
        let (found, stamps) = look_for(ws, &seen.lib_paths, &name, current, limits);
        seen.contents.extend(stamps);
        seen.libraries.insert(name, found);
    }
    for path in read {
        let stamp = read_program_stamped(ws, &path, limits).1;
        seen.contents.insert(path, stamp);
    }
    seen
}

/// A check kept, and what it saw.
pub struct Kept {
    pub checked: FoldersChecked,
}

impl Kept {
    pub fn take(checked: FoldersChecked) -> Kept {
        Kept { checked }
    }

    /// Whether it still says what a check of the files would say now: what it saw is what one would see.
    pub fn holds(&self, ws: &dyn Workspace, folder: &Path, lib_paths: &[PathBuf], current: Option<&Path>, limits: &Limits) -> bool {
        let seen = &self.checked.seen;
        look_again(ws, folder, lib_paths, seen.libraries.keys().cloned(), current, limits) == *seen
    }
}
