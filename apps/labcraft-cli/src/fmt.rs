//! `labcraft-cli fmt` (`docs/cli.md`, section 2): one file at a time, reading no other file. The logic stays in
//! `labcraft-format`; here are the files, the atomic write and the last resort.

use std::panic::{self, AssertUnwindSafe};
use std::path::Path;
use std::process::ExitCode;
use std::sync::Mutex;

use labcraft_format::format;
use labcraft_types::{Code, Diagnostic, FileEntry, FileResult, Interner, Limits, Position, Report, Span, Status};

use crate::args::FmtOptions;
use crate::output;
use crate::walk::{self, Found, io_text, read_program};

/// What the panic hook saw, for `LC0099`.
static LAST_PANIC: Mutex<Option<String>> = Mutex::new(None);

/// A panic is reported as a diagnostic, not printed: stderr is for wrong usage alone.
pub fn keep_panics_for_the_report() {
    panic::set_hook(Box::new(|info| {
        if let Ok(mut last) = LAST_PANIC.lock() {
            *last = Some(info.to_string());
        }
    }));
}

/// As [`keep_panics_for_the_report`], but the hook that was there still runs: the desktop viewer prints a panic of
/// its window, as any program does, and keeps the message of one caught in its session for its `LC0099`.
pub fn keep_panics_and_print_them() {
    let before = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        if let Ok(mut last) = LAST_PANIC.lock() {
            *last = Some(info.to_string());
        }
        before(info);
    }));
}

pub fn last_panic() -> String {
    LAST_PANIC.lock().ok().and_then(|mut last| last.take()).unwrap_or_else(|| "a panic".to_owned())
}

pub fn run(options: &FmtOptions) -> ExitCode {
    let current = std::env::current_dir().ok();
    let walked = walk::walk(&options.paths, current.as_deref(), &options.limits);
    let mut report = walked.report;
    let mut files: Vec<FileEntry> = walked.refused.into_iter().map(|refused| FileEntry { file: refused.shown, result: FileResult::Error }).collect();
    let mut changed = 0;
    for found in &walked.files {
        let (result, file_report) = last_resort(found, &options.limits, || format_file(&found.path, options));
        changed += u64::from(result == FileResult::Formatted);
        report.merge(named(file_report, &found.shown));
        files.push(FileEntry { file: found.shown.clone(), result });
    }
    files.sort_by(|a, b| a.file.cmp(&b.file));
    answer(options, report, &files, changed)
}

/// When the work could not start or ended outside any file, the answer keeps its form: a report, with `LC0099`.
pub fn internal_failure(options: &FmtOptions, what: &str) -> ExitCode {
    answer(options, failure_report(options, what), &[], 0)
}

fn failure_report(options: &FmtOptions, what: &str) -> Report {
    let mut report = Report::new(options.limits.max_report);
    report.push(Diagnostic::new(Code::Lc0099, format!("the tool could not finish: {what}"), "labcraft-cli"));
    report
}

fn answer(options: &FmtOptions, mut report: Report, files: &[FileEntry], changed: u64) -> ExitCode {
    report.finish();
    if options.json {
        output::write_json(&report.fmt_document(files, changed));
    } else {
        output::write_text(&output::fmt_text(&report, files, changed));
    }
    match report.status(false) {
        Status::Ok => ExitCode::SUCCESS,
        Status::Error => ExitCode::FAILURE,
    }
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

fn format_file(path: &Path, options: &FmtOptions) -> (FileResult, Report) {
    let limits = &options.limits;
    let bytes = match read_program(path, limits) {
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
    if options.check {
        let message = "not in canonical form: this is the first line that differs, and `labcraft-cli fmt` rewrites the file";
        let mut diagnostic = Diagnostic::new(Code::Lc6001, message, "");
        diagnostic.span = first_difference(&bytes, &text);
        report.push(diagnostic);
        return (FileResult::NotCanonical, report);
    }
    // On Windows, the rename would refuse it too, but only after retrying for most of a second, once per file.
    if std::fs::metadata(path).is_ok_and(|metadata| metadata.permissions().readonly()) {
        report.push(Diagnostic::new(Code::Lc6003, "the file is read-only, so it stays as it is", ""));
        return (FileResult::Error, report);
    }
    match craft_atomic::atomic_write(path, text.as_bytes()) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn line_and_end(original: &str, canonical: &str) -> Option<(u32, u32)> {
        first_difference(original.as_bytes(), canonical).map(|span| {
            assert_eq!(span.start.column, 1);
            assert_eq!(span.start.line, span.end.line);
            (span.start.line, span.end.column)
        })
    }

    #[test]
    fn the_first_line_that_differs_is_found() {
        assert_eq!(line_and_end("a\nbb\nc\n", "a\nb\nc\n"), Some((2, 3)));
        assert_eq!(line_and_end("a\n", "a\n"), None);
    }

    #[test]
    fn a_line_ending_is_part_of_its_line() {
        assert_eq!(line_and_end("ab\r\nc\r\n", "ab\nc\n"), Some((1, 3)));
        assert_eq!(line_and_end("a\nb", "a\nb\n"), Some((2, 2)));
    }

    #[test]
    fn lines_in_excess_or_missing_are_located() {
        assert_eq!(line_and_end("a\n\n\n", "a\n"), Some((2, 1)));
        assert_eq!(line_and_end("a\n", "a\nb\n"), Some((2, 1)));
    }

    #[test]
    fn a_byte_order_mark_is_not_a_column_and_columns_count_code_points() {
        assert_eq!(line_and_end("\u{FEFF}ab\n", "ab\n"), Some((1, 3)));
        assert_eq!(line_and_end("\u{E9}\u{E9}\n", "x\n"), Some((1, 3)));
    }

    /// The hook is global: while it is in place, a failure of another test loses its message, and only that.
    #[test]
    fn a_panic_on_one_file_is_a_diagnostic_on_that_file_with_its_cause() {
        let found = Found { shown: "a.lcf".to_owned(), path: "a.lcf".into(), through_a_link: false };
        let previous = panic::take_hook();
        keep_panics_for_the_report();
        let (result, report) = last_resort(&found, &Limits::default(), || panic!("a bug"));
        panic::set_hook(previous);
        assert_eq!(result, FileResult::Error);
        let report = named(report, &found.shown);
        let diagnostic = &report.diagnostics()[0];
        assert_eq!(diagnostic.code(), Code::Lc0099);
        assert_eq!(diagnostic.file, "a.lcf");
        assert!(diagnostic.message.contains("a bug") && diagnostic.message.contains("fmt.rs"), "{}", diagnostic.message);
    }

    #[test]
    fn a_failure_outside_any_file_is_a_report_too() {
        let options = FmtOptions { check: false, json: true, limits: Limits::default(), paths: Vec::new() };
        let report = failure_report(&options, "cannot start the work");
        assert_eq!(report.diagnostics()[0].code(), Code::Lc0099);
        assert_eq!(report.status(false), Status::Error);
        assert!(report.diagnostics()[0].message.contains("cannot start the work"));
    }
}
