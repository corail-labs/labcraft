//! Turns a report into text or JSON (`docs/cli.md`, section 4). A closed stdout, as in `labcraft-cli fmt . | head -1`,
//! changes nothing: the exit code still says what was found.

use std::fmt::Write as _;
use std::io::{BufWriter, Write as _};

use labcraft_types::printed::{diagnostic_text, line};
use labcraft_types::{CheckCounts, FileEntry, FileResult, Report};

pub fn counted(count: u64, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// The files rewritten, the diagnostics, then the summary, which is the last line.
pub fn fmt_text(report: &Report, files: &[FileEntry], changed: u64) -> String {
    let mut out = String::new();
    for entry in files.iter().filter(|entry| entry.result == FileResult::Formatted) {
        line(&mut out, &format!("formatted {}", entry.file));
    }
    for diagnostic in report.diagnostics() {
        diagnostic_text(&mut out, diagnostic);
    }
    let _ = write!(
        out,
        "fmt: {}, {changed} changed: {}, {}",
        counted(files.len() as u64, "file", "files"),
        counted(report.errors(), "error", "errors"),
        counted(report.warnings(), "warning", "warnings"),
    );
    if report.truncated() {
        let _ = write!(out, " ({} listed)", report.diagnostics().len());
    }
    out.push('\n');
    out
}

/// The diagnostics, then the summary, which is the last line.
pub fn check_text(report: &Report, counts: CheckCounts) -> String {
    let mut out = String::new();
    for diagnostic in report.diagnostics() {
        diagnostic_text(&mut out, diagnostic);
    }
    let _ = write!(
        out,
        "check: {}, {}, {}: {}, {}",
        counted(counts.libraries, "library", "libraries"),
        counted(counts.functions, "function", "functions"),
        counted(counts.types, "type", "types"),
        counted(report.errors(), "error", "errors"),
        counted(report.warnings(), "warning", "warnings"),
    );
    if report.truncated() {
        let _ = write!(out, " ({} listed)", report.diagnostics().len());
    }
    out.push('\n');
    out
}

pub fn write_text(text: &str) {
    let _ = std::io::stdout().lock().write_all(text.as_bytes());
}

/// One document and nothing else: it goes straight to stdout, so that 100,000 files need no second copy in memory.
pub fn write_json(document: &impl serde::Serialize) {
    let mut out = BufWriter::new(std::io::stdout().lock());
    if serde_json::to_writer_pretty(&mut out, document).is_ok() {
        let _ = out.write_all(b"\n");
    }
    let _ = out.flush();
}

#[cfg(test)]
mod tests {
    use labcraft_types::{Code, Diagnostic};

    use super::*;

    /// The last line of the example of `docs/cli.md`, section 4.1.
    #[test]
    fn the_summary_of_check_reads_as_the_documentation_shows() {
        let mut report = Report::new(10);
        report.push(Diagnostic::new(Code::Lc3002, "m", "a.lcf"));
        report.finish();
        let counts = CheckCounts { libraries: 1, functions: 14, types: 3 };
        assert_eq!(check_text(&report, counts), "a.lcf: error[LC3002]: m\ncheck: 1 library, 14 functions, 3 types: 1 error, 0 warnings\n");
    }

    #[test]
    fn the_last_line_is_the_summary() {
        let mut report = Report::new(1);
        let entry = |file: &str, result| FileEntry { file: file.to_owned(), result };
        let files = [entry("a.lcf", FileResult::Formatted), entry("b.lcf", FileResult::Unchanged)];
        assert_eq!(fmt_text(&report, &files, 1), "formatted a.lcf\nfmt: 2 files, 1 changed: 0 errors, 0 warnings\n");
        report.push(Diagnostic::new(Code::Lc0003, "m", "c.lcf"));
        report.push(Diagnostic::new(Code::Lc0003, "m", "d.lcf"));
        report.push(Diagnostic::new(Code::Lc0009, "m", "e.lcf"));
        report.finish();
        let text = fmt_text(&report, &files[1..], 0);
        assert_eq!(text, "c.lcf: error[LC0003]: m\nfmt: 1 file, 0 changed: 2 errors, 1 warning (1 listed)\n");
    }
}
