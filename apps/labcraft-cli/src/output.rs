//! Turns a report into text or JSON (`docs/cli.md`, section 4). A closed stdout, as in `labcraft-cli fmt . | head -1`,
//! changes nothing: the exit code still says what was found.

use std::borrow::Cow;
use std::fmt::Write as _;
use std::io::{BufWriter, Write as _};

use labcraft_types::{CheckCounts, Diagnostic, FileEntry, FileResult, Report};

/// What a path or a text from a file could hide or forge on a terminal: a line feed in a file name would make a
/// second diagnostic, an escape sequence would move the cursor. JSON has its own escapes.
fn printable(text: &str) -> Cow<'_, str> {
    let hidden = |c: char| c.is_control() || matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{2028}'..='\u{202E}' | '\u{2066}'..='\u{2069}');
    if !text.contains(hidden) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if hidden(c) {
            out.extend(c.escape_unicode());
        } else {
            out.push(c);
        }
    }
    Cow::Owned(out)
}

fn line(out: &mut String, text: &str) {
    out.push_str(&printable(text));
    out.push('\n');
}

/// `file:line:column: severity[code]: message`, then the details, indented (`docs/cli.md`, section 4.1).
fn diagnostic_text(out: &mut String, diagnostic: &Diagnostic) {
    let place = match diagnostic.span {
        Some(span) => format!("{}:{}:{}", diagnostic.file, span.start.line, span.start.column),
        None => diagnostic.file.clone(),
    };
    let code = diagnostic.code();
    line(out, &format!("{place}: {}[{}]: {}", diagnostic.severity().as_str(), code.as_str(), diagnostic.message));
    if let Some(text) = where_it_is(diagnostic) {
        line(out, &format!("  {text}"));
    }
    for wire in &diagnostic.wires {
        let name = wire.value.as_ref().map_or_else(|| "terminal".to_owned(), |value| format!("wire `{value}`"));
        let route = match (&wire.from, &wire.to) {
            (Some(from), Some(to)) => format!("{from} -> {to}"),
            (Some(terminal), None) | (None, Some(terminal)) => terminal.clone(),
            (None, None) => String::new(),
        };
        line(out, &format!("  {name}: {route}"));
    }
    match (&diagnostic.expected, &diagnostic.found) {
        (Some(expected), Some(found)) => line(out, &format!("  expected `{expected}`, found `{found}`")),
        (Some(expected), None) => line(out, &format!("  expected `{expected}`")),
        (None, Some(found)) => line(out, &format!("  found `{found}`")),
        (None, None) => {}
    }
    for fix in &diagnostic.fixes {
        line(out, &format!("  fix: {}", fix.title));
    }
    for related in &diagnostic.related {
        let place = match related.span {
            Some(span) => format!("{}:{}:{}", related.file, span.start.line, span.start.column),
            None => related.file.clone(),
        };
        line(out, &format!("  {place}: {}", related.message));
    }
}

fn where_it_is(diagnostic: &Diagnostic) -> Option<String> {
    let node = diagnostic.node.as_ref().map(|node| match &node.name {
        Some(name) => format!("node `{name}`: {}", node.text),
        None => format!("node: {}", node.text),
    });
    match (&diagnostic.function, node) {
        (Some(function), Some(node)) => Some(format!("in function `{function}`, {node}")),
        (Some(function), None) => Some(format!("in function `{function}`")),
        (None, Some(node)) => Some(format!("in {node}")),
        (None, None) => None,
    }
}

fn counted(count: u64, one: &str, many: &str) -> String {
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
    use labcraft_types::{Code, Fix, Node, Position, Related, Span, Wire};

    use super::*;

    fn at(line: u32, column: u32) -> Span {
        Span { start: Position { line, column }, end: Position { line, column: column + 1 } }
    }

    fn text_of(diagnostic: &Diagnostic) -> String {
        let mut out = String::new();
        diagnostic_text(&mut out, diagnostic);
        out
    }

    /// The example of `docs/cli.md`, section 4.1.
    #[test]
    fn a_diagnostic_reads_as_the_documentation_shows() {
        let mut diagnostic =
            Diagnostic::new(Code::Lc3002, "cannot add `f64[V]` and `f64[A]`: different dimensions", "bench/power.lcf").with_span(at(9, 22));
        diagnostic.function = Some("power".to_owned());
        diagnostic.node = Some(Node { name: Some("p".to_owned()), text: "p = add(voltage, current)".to_owned() });
        diagnostic.wires = vec![
            Wire { value: Some("voltage".to_owned()), from: Some("fn.voltage".to_owned()), to: Some("p.a".to_owned()) },
            Wire { value: Some("current".to_owned()), from: Some("current.result".to_owned()), to: Some("p.b".to_owned()) },
        ];
        diagnostic.expected = Some("f64[V]".to_owned());
        diagnostic.found = Some("f64[A]".to_owned());
        assert_eq!(
            text_of(&diagnostic),
            "bench/power.lcf:9:22: error[LC3002]: cannot add `f64[V]` and `f64[A]`: different dimensions\n\
             \x20 in function `power`, node `p`: p = add(voltage, current)\n\
             \x20 wire `voltage`: fn.voltage -> p.a\n\
             \x20 wire `current`: current.result -> p.b\n\
             \x20 expected `f64[V]`, found `f64[A]`\n"
        );
    }

    #[test]
    fn a_problem_with_no_position_has_the_file_alone() {
        assert_eq!(text_of(&Diagnostic::new(Code::Lc0003, "not valid UTF-8", "a.lcf")), "a.lcf: error[LC0003]: not valid UTF-8\n");
    }

    #[test]
    fn the_rarer_details_have_a_line_each() {
        let mut diagnostic = Diagnostic::new(Code::Lc3005, "m", "a.lcf");
        diagnostic.wires = vec![Wire { to: Some("n.unit".to_owned()), ..Wire::default() }];
        diagnostic.fixes = vec![Fix { title: "Convert".to_owned(), edits: vec![] }];
        diagnostic.related = vec![Related { message: "first here".to_owned(), file: "b.lcf".to_owned(), span: Some(at(5, 5)) }];
        assert_eq!(text_of(&diagnostic), "a.lcf: error[LC3005]: m\n  terminal: n.unit\n  fix: Convert\n  b.lcf:5:5: first here\n");
    }

    #[test]
    fn a_file_name_cannot_forge_a_line_or_drive_a_terminal() {
        let diagnostic = Diagnostic::new(Code::Lc0003, "m", "a\nb.lcf:1:1: error[LC0001]: \u{1B}[2J\u{202E}.lcf");
        let text = text_of(&diagnostic);
        assert_eq!(text.lines().count(), 1);
        assert!(!text.contains('\u{1B}') && !text.contains('\u{202E}'), "{text}");
        assert!(text.contains("\\u{a}") && text.contains("\\u{1b}") && text.contains("\\u{202e}"), "{text}");
    }

    #[test]
    fn line_separators_and_direction_marks_are_escaped_and_other_text_is_not() {
        for hidden in ['\u{0085}', '\u{061C}', '\u{200E}', '\u{200F}', '\u{2028}', '\u{2029}', '\u{202A}', '\u{2066}', '\u{2069}'] {
            assert!(!printable(&format!("a{hidden}b")).contains(hidden), "U+{:04X}", u32::from(hidden));
        }
        for shown in ["\u{00E9}", "\u{03BC}", "\u{2027}", "\u{202F}", "\u{2065}", "\u{206A}"] {
            assert_eq!(printable(shown), shown);
        }
    }

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
