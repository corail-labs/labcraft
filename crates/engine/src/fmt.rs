//! Formatting files as `labcraft-cli fmt` does (`docs/cli.md`, section 2): one file at a time, reading no other file.
//! The logic stays in `labcraft-format`; here are the files, the atomic write and the last resort.

use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use labcraft_format::format;
use labcraft_types::{Code, Diagnostic, FileEntry, FileResult, Interner, Limits, Position, Report, Span};

use crate::panics::last_panic;
use crate::walk::{self, Found, io_text, read_program};
use crate::workspace::Workspace;

/// What a formatting gives: its report, the result of each file in the order of their names, and how many changed.
pub struct Formatted {
    pub report: Report,
    pub files: Vec<FileEntry>,
    pub changed: u64,
}

/// Puts each file found under the paths in canonical form, or, with `check_only`, writes nothing and finds those that
/// are not. Paths are shown from `current`.
pub fn format_paths(ws: &dyn Workspace, paths: &[PathBuf], current: Option<&Path>, limits: &Limits, check_only: bool) -> Formatted {
    let walked = walk::walk(ws, paths, current, limits);
    let mut report = walked.report;
    let mut files: Vec<FileEntry> = walked.refused.into_iter().map(|refused| FileEntry { file: refused.shown, result: FileResult::Error }).collect();
    let mut changed = 0;
    for found in &walked.files {
        let (result, file_report) = last_resort(found, limits, || format_file(ws, &found.path, limits, check_only));
        changed += u64::from(result == FileResult::Formatted);
        report.merge(named(file_report, &found.shown));
        files.push(FileEntry { file: found.shown.clone(), result });
    }
    files.sort_by(|a, b| a.file.cmp(&b.file));
    Formatted { report, files, changed }
}

fn named(mut report: Report, file: &str) -> Report {
    report.set_file(file);
    report
}

/// A panic in one file is a bug, reported on that file while the others go on (`docs/architecture.md`, section 5).
fn last_resort(found: &Found, limits: &Limits, work: impl FnOnce() -> (FileResult, Report)) -> (FileResult, Report) {
    panic::catch_unwind(AssertUnwindSafe(work)).unwrap_or_else(|_| {
        let mut report = Report::new(limits.max_report);
        report.push(Diagnostic::new(Code::Lc0099, format!("internal error, which is a bug of the tool: {}", last_panic()), found.shown.as_str()));
        (FileResult::Error, report)
    })
}

fn format_file(ws: &dyn Workspace, path: &Path, limits: &Limits, check_only: bool) -> (FileResult, Report) {
    let bytes = match read_program(ws, path, limits) {
        Ok(bytes) => bytes,
        Err((code, message)) => {
            let mut report = Report::new(limits.max_report);
            report.push(Diagnostic::new(code, message, ""));
            return (FileResult::Error, report);
        }
    };
    let mut names = Interner::new();
    let formatted = format(&bytes, &mut names, limits);
    let mut report = formatted.report;
    let Some(text) = formatted.text else {
        return (FileResult::Error, report);
    };
    if text.as_bytes() == bytes {
        return (FileResult::Unchanged, report);
    }
    if check_only {
        let message = "not in canonical form: this is the first line that differs, and `labcraft-cli fmt` rewrites the file";
        let mut diagnostic = Diagnostic::new(Code::Lc6001, message, "");
        diagnostic.span = first_difference(&bytes, &text);
        report.push(diagnostic);
        return (FileResult::NotCanonical, report);
    }
    // On Windows, the rename would refuse it too, but only after retrying for most of a second, once per file.
    if ws.metadata(path).is_ok_and(|metadata| metadata.read_only) {
        report.push(Diagnostic::new(Code::Lc6003, "the file is read-only, so it stays as it is", ""));
        return (FileResult::Error, report);
    }
    match ws.write(path, text.as_bytes()) {
        Ok(()) => (FileResult::Formatted, report),
        Err(error) => {
            let message = format!("the file system refused the canonical form, so the file stays as it is: {}", io_text(&error));
            report.push(Diagnostic::new(Code::Lc6003, message, ""));
            (FileResult::Error, report)
        }
    }
}

/// The first line that differs, with its line ending, in the terms of a span (`docs/cli.md`, section 4.2): columns
/// count code points of the file as read, a byte order mark excluded. `None` if the bytes are not text, which
/// `format` has already refused.
fn first_difference(bytes: &[u8], text: &str) -> Option<Span> {
    let original = std::str::from_utf8(bytes).ok()?;
    let mut lines = original.split_inclusive('\n');
    let mut canonical = text.split_inclusive('\n');
    let mut number: u32 = 1;
    loop {
        match (lines.next(), canonical.next()) {
            (Some(a), Some(b)) if a == b => number = number.saturating_add(1),
            (None, None) => return None,
            (line, _) => {
                let line = line.unwrap_or_default();
                let line = line.strip_prefix('\u{FEFF}').unwrap_or(line);
                let width = line.trim_end_matches(['\n', '\r']).chars().count();
                let end = u32::try_from(width).unwrap_or(u32::MAX).saturating_add(1);
                return Some(Span { start: Position { line: number, column: 1 }, end: Position { line: number, column: end } });
            }
        }
    }
}
