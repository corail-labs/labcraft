//! Diagnostics and reports: the schema of `docs/cli.md`, section 4. A field that does not apply is
//! absent, not `null`.

use std::borrow::Cow;

use serde::Serialize;

use crate::code::{Code, Severity};

/// Long texts are cut at this many code points and end with `…`, so that a hostile file cannot make
/// the report huge.
pub const MAX_TEXT: usize = 200;

/// The wires, fixes, edits or related places of one diagnostic: `LC2013` can relate 100,000 files.
pub const MAX_LIST: usize = 32;

pub fn cut_text(text: &str) -> Cow<'_, str> {
    let mut starts = text.char_indices().map(|(start, _)| start);
    let Some(last_kept) = starts.nth(MAX_TEXT - 1) else {
        return Cow::Borrowed(text);
    };
    if starts.next().is_none() {
        return Cow::Borrowed(text);
    }
    Cow::Owned(format!("{}\u{2026}", text.get(..last_kept).unwrap_or_default()))
}

fn cut_in_place(text: &mut String) {
    if let Cow::Owned(cut) = cut_text(text) {
        *text = cut;
    }
}

/// Lines and columns count from 1; columns count Unicode code points of the file as read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Position {
    pub line: u32,
    pub column: u32,
}

/// A start and an exclusive end.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Span {
    pub start: Position,
    pub end: Position,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Node {
    /// The name of the line (spec 5.2), absent if empty.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The text of the line, in canonical form.
    pub text: String,
}

/// A wire or terminal at fault; each part is present when it exists (`docs/cli.md`, section 4.3).
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Wire {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Edit {
    pub span: Span,
    pub text: String,
}

/// A suggestion: nothing applies it in M0a.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Fix {
    pub title: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub edits: Vec<Edit>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Related {
    pub message: String,
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

/// The severity is set by the code, which is why neither can be changed once made.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Diagnostic {
    code: Code,
    severity: Severity,
    pub message: String,
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<Node>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub wires: Vec<Wire>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub found: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fixes: Vec<Fix>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<Related>,
}

impl Diagnostic {
    pub fn new(code: Code, message: impl Into<String>, file: impl Into<String>) -> Self {
        Diagnostic {
            code,
            severity: code.severity(),
            message: message.into(),
            file: file.into(),
            span: None,
            function: None,
            node: None,
            wires: Vec::new(),
            expected: None,
            found: None,
            fixes: Vec::new(),
            related: Vec::new(),
        }
    }

    /// Other crates cannot use struct update syntax here, since `code` and `severity` are private.
    #[must_use]
    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }

    pub fn code(&self) -> Code {
        self.code
    }

    pub fn severity(&self) -> Severity {
        self.severity
    }

    /// What a hostile name could make huge. Names and terminals are cut like any text, since they
    /// only say where to look. The text of an edit is not: cut, it would change the program when
    /// applied, so a fix with an edit too long keeps its title and loses its edits. Paths stay
    /// whole; the walk of a folder bounds them.
    fn bound_texts_and_lists(&mut self) {
        cut_in_place(&mut self.message);
        if let Some(function) = &mut self.function {
            cut_in_place(function);
        }
        if let Some(node) = &mut self.node {
            cut_in_place(&mut node.text);
            if let Some(name) = &mut node.name {
                cut_in_place(name);
            }
        }
        for text in [&mut self.expected, &mut self.found].into_iter().flatten() {
            cut_in_place(text);
        }
        self.wires.truncate(MAX_LIST);
        for wire in &mut self.wires {
            for text in [&mut wire.value, &mut wire.from, &mut wire.to].into_iter().flatten() {
                cut_in_place(text);
            }
        }
        self.fixes.truncate(MAX_LIST);
        for fix in &mut self.fixes {
            cut_in_place(&mut fix.title);
            if fix.edits.len() > MAX_LIST || fix.edits.iter().any(|edit| matches!(cut_text(&edit.text), Cow::Owned(_))) {
                fix.edits.clear();
            }
        }
        self.related.truncate(MAX_LIST);
        for related in &mut self.related {
            cut_in_place(&mut related.message);
        }
    }

    /// `docs/cli.md`, section 4.2: file in code point order, line, column (no span first), code, message.
    fn sort_key(&self) -> (&str, Option<Position>, &str, &str) {
        (&self.file, self.span.map(|span| span.start), self.code.as_str(), &self.message)
    }

    /// Which diagnostics a cut report keeps: errors before warnings, then the order of a report.
    fn keep_key(&self) -> (Severity, (&str, Option<Position>, &str, &str)) {
        (self.severity, self.sort_key())
    }
}

/// The diagnostics of a run and their counts, which stay complete when the list is cut. After each
/// push or merge, it holds at most twice `max_report` diagnostics: a file of 16 MiB can hold millions
/// of errors, and keeping them all until the end would break the memory bound of scenario 4.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    max_report: usize,
    diagnostics: Vec<Diagnostic>,
    errors: u64,
    warnings: u64,
}

impl Report {
    pub fn new(max_report: usize) -> Self {
        Report { max_report, diagnostics: Vec::new(), errors: 0, warnings: 0 }
    }

    /// Every diagnostic enters here, so its long texts and lists are always bounded.
    pub fn push(&mut self, mut diagnostic: Diagnostic) {
        diagnostic.bound_texts_and_lists();
        match diagnostic.severity {
            Severity::Error => self.errors += 1,
            Severity::Warning => self.warnings += 1,
        }
        self.diagnostics.push(diagnostic);
        self.prune_when_full();
    }

    /// Names the file of the diagnostics that have none yet: a crate that reads bytes does not know
    /// where they came from. So the report of one file is made first, and merged once named.
    pub fn set_file(&mut self, file: &str) {
        for diagnostic in self.diagnostics.iter_mut().filter(|diagnostic| diagnostic.file.is_empty()) {
            diagnostic.file = file.to_owned();
        }
    }

    pub fn merge(&mut self, other: Report) {
        self.errors += other.errors;
        self.warnings += other.warnings;
        self.diagnostics.extend(other.diagnostics);
        self.prune_when_full();
    }

    /// The errors of `other` with their complete count, and none of its warnings: what a dependency has to say.
    pub fn merge_errors(&mut self, other: Report) {
        self.errors += other.errors;
        self.diagnostics.extend(other.diagnostics.into_iter().filter(|diagnostic| diagnostic.severity == Severity::Error));
        self.prune_when_full();
    }

    fn prune_when_full(&mut self) {
        if self.diagnostics.len() > self.max_report.saturating_mul(2) {
            self.prune();
        }
    }

    /// Keeps the diagnostics that the final report lists: which ones does not depend on when this
    /// runs, so the cost is amortized over the pushes and the result is that of a cut at the end.
    fn prune(&mut self) {
        self.diagnostics.sort_by(|a, b| a.keep_key().cmp(&b.keep_key()).then_with(|| a.cmp(b)));
        self.diagnostics.truncate(self.max_report);
    }

    /// The list in its final form: at most `max_report` diagnostics, errors before warnings, in the
    /// order of `docs/cli.md`, section 4.2. The same input always gives the same report, whatever
    /// order its parts came in. The app calls it once all folders are merged.
    pub fn finish(&mut self) {
        self.prune();
        self.diagnostics.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()).then_with(|| a.cmp(b)));
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn errors(&self) -> u64 {
        self.errors
    }

    pub fn warnings(&self) -> u64 {
        self.warnings
    }

    /// The counts say more than the list can.
    pub fn truncated(&self) -> bool {
        self.errors.saturating_add(self.warnings) > self.max_report.min(self.diagnostics.len()) as u64
    }

    /// `error` exactly when the exit code is 1; `--deny-warnings` makes warnings count.
    pub fn status(&self, deny_warnings: bool) -> Status {
        if self.errors > 0 || (deny_warnings && self.warnings > 0) { Status::Error } else { Status::Ok }
    }

    pub fn check_document(&self, counts: CheckCounts, deny_warnings: bool) -> CheckDocument<'_> {
        CheckDocument {
            schema: "labcraft-check/1",
            status: self.status(deny_warnings),
            summary: CheckSummary {
                libraries: counts.libraries,
                functions: counts.functions,
                types: counts.types,
                errors: self.errors,
                warnings: self.warnings,
                truncated: self.truncated(),
            },
            diagnostics: &self.diagnostics,
        }
    }

    pub fn fmt_document<'a>(&'a self, files: &'a [FileEntry], changed: u64) -> FmtDocument<'a> {
        FmtDocument {
            schema: "labcraft-fmt/1",
            status: self.status(false),
            summary: FmtSummary { files: files.len() as u64, changed, errors: self.errors, warnings: self.warnings, truncated: self.truncated() },
            files,
            diagnostics: &self.diagnostics,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Error,
}

/// What `check` counted, besides the diagnostics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CheckCounts {
    pub libraries: u64,
    pub functions: u64,
    pub types: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct CheckSummary {
    pub libraries: u64,
    pub functions: u64,
    pub types: u64,
    pub errors: u64,
    pub warnings: u64,
    pub truncated: bool,
}

/// `check --json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CheckDocument<'a> {
    pub schema: &'static str,
    pub status: Status,
    pub summary: CheckSummary,
    pub diagnostics: &'a [Diagnostic],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct FmtSummary {
    pub files: u64,
    pub changed: u64,
    pub errors: u64,
    pub warnings: u64,
    pub truncated: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileResult {
    Unchanged,
    Formatted,
    NotCanonical,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FileEntry {
    pub file: String,
    pub result: FileResult,
}

/// `fmt --json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FmtDocument<'a> {
    pub schema: &'static str,
    pub status: Status,
    pub summary: FmtSummary,
    pub files: &'a [FileEntry],
    pub diagnostics: &'a [Diagnostic],
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn at(line: u32, column: u32) -> Option<Span> {
        Some(Span { start: Position { line, column }, end: Position { line, column: column + 1 } })
    }

    fn diagnostic(code: Code, file: &str, span: Option<Span>, message: &str) -> Diagnostic {
        Diagnostic { span, ..Diagnostic::new(code, message, file) }
    }

    /// The example of `docs/cli.md`, section 4.2.
    #[test]
    fn the_json_is_the_one_of_the_documentation() {
        let mut report = Report::new(1000);
        report.push(Diagnostic {
            span: Some(Span { start: Position { line: 9, column: 27 }, end: Position { line: 9, column: 34 } }),
            function: Some("power".to_owned()),
            node: Some(Node { name: Some("p".to_owned()), text: "p = core.add(voltage, current)".to_owned() }),
            wires: vec![
                Wire { value: Some("voltage".to_owned()), from: Some("fn.voltage".to_owned()), to: Some("p.a".to_owned()) },
                Wire { value: Some("current".to_owned()), from: Some("current.result".to_owned()), to: Some("p.b".to_owned()) },
            ],
            expected: Some("f64[V]".to_owned()),
            found: Some("f64[A]".to_owned()),
            ..Diagnostic::new(Code::Lc3002, "cannot add `f64[V]` and `f64[A]`: different dimensions", "bench/power.lcf")
        });
        let document = report.check_document(CheckCounts { libraries: 1, functions: 14, types: 3 }, false);
        assert_eq!(
            serde_json::to_value(&document).unwrap(),
            json!({
                "schema": "labcraft-check/1",
                "status": "error",
                "summary": { "libraries": 1, "functions": 14, "types": 3, "errors": 1, "warnings": 0, "truncated": false },
                "diagnostics": [{
                    "code": "LC3002",
                    "severity": "error",
                    "message": "cannot add `f64[V]` and `f64[A]`: different dimensions",
                    "file": "bench/power.lcf",
                    "span": { "start": { "line": 9, "column": 27 }, "end": { "line": 9, "column": 34 } },
                    "function": "power",
                    "node": { "name": "p", "text": "p = core.add(voltage, current)" },
                    "wires": [
                        { "value": "voltage", "from": "fn.voltage", "to": "p.a" },
                        { "value": "current", "from": "current.result", "to": "p.b" }
                    ],
                    "expected": "f64[V]",
                    "found": "f64[A]"
                }]
            })
        );
    }

    /// The fix and the related place of `docs/cli.md`, section 4.2.
    #[test]
    fn fixes_and_related_places_have_their_form() {
        let span = |line, column, end| Span { start: Position { line, column }, end: Position { line, column: end } };
        let diagnostic = Diagnostic {
            fixes: vec![Fix {
                title: "Convert with `core.round_to_i32`".to_owned(),
                edits: vec![Edit { span: span(4, 5, 5), text: "n2 = core.round_to_i32(x)\n    ".to_owned() }],
            }],
            related: vec![Related {
                message: "`voltage` is already defined here".to_owned(),
                file: "bench/power.lcf".to_owned(),
                span: Some(span(5, 5, 12)),
            }],
            ..Diagnostic::new(Code::Lc3004, "m", "f.lcf")
        };
        let value = serde_json::to_value(&diagnostic).unwrap();
        assert_eq!(
            value["fixes"],
            json!([{
                "title": "Convert with `core.round_to_i32`",
                "edits": [{
                    "span": { "start": { "line": 4, "column": 5 }, "end": { "line": 4, "column": 5 } },
                    "text": "n2 = core.round_to_i32(x)\n    "
                }]
            }])
        );
        assert_eq!(
            value["related"],
            json!([{
                "message": "`voltage` is already defined here",
                "file": "bench/power.lcf",
                "span": { "start": { "line": 5, "column": 5 }, "end": { "line": 5, "column": 12 } }
            }])
        );
    }

    #[test]
    fn a_field_that_does_not_apply_is_absent() {
        let value = serde_json::to_value(Diagnostic::new(Code::Lc0003, "not UTF-8", "f.lcf")).unwrap();
        assert_eq!(value, json!({ "code": "LC0003", "severity": "error", "message": "not UTF-8", "file": "f.lcf" }));
        let unit_argument = Wire { to: Some("n.unit".to_owned()), ..Wire::default() };
        assert_eq!(serde_json::to_value(unit_argument).unwrap(), json!({ "to": "n.unit" }));
        let empty = Report::new(1000);
        assert_eq!(serde_json::to_value(empty.check_document(CheckCounts::default(), false)).unwrap()["diagnostics"], json!([]));
    }

    #[test]
    fn the_fmt_document_has_its_schema_and_files() {
        let mut report = Report::new(1000);
        report.push(Diagnostic::new(Code::Lc6001, "not canonical", "b.lcf"));
        let files = [
            FileEntry { file: "a.lcf".to_owned(), result: FileResult::Unchanged },
            FileEntry { file: "b.lcf".to_owned(), result: FileResult::NotCanonical },
            FileEntry { file: "c.lcf".to_owned(), result: FileResult::Formatted },
            FileEntry { file: "d.lcf".to_owned(), result: FileResult::Error },
        ];
        let value = serde_json::to_value(report.fmt_document(&files, 1)).unwrap();
        assert_eq!(value["schema"], "labcraft-fmt/1");
        assert_eq!(value["status"], "error");
        assert_eq!(value["summary"], json!({ "files": 4, "changed": 1, "errors": 1, "warnings": 0, "truncated": false }));
        let results: Vec<&str> = value["files"].as_array().unwrap().iter().map(|file| file["result"].as_str().unwrap()).collect();
        assert_eq!(results, ["unchanged", "not_canonical", "formatted", "error"]);
    }

    #[test]
    fn the_severity_comes_from_the_code() {
        assert_eq!(Diagnostic::new(Code::Lc3002, "", "").severity(), Severity::Error);
        assert_eq!(Diagnostic::new(Code::Lc4101, "", "").severity(), Severity::Warning);
    }

    #[test]
    fn the_status_follows_the_exit_code() {
        let mut report = Report::new(1000);
        assert_eq!(report.status(false), Status::Ok);
        assert_eq!(report.status(true), Status::Ok);
        report.push(Diagnostic::new(Code::Lc4101, "", ""));
        assert_eq!(report.status(false), Status::Ok);
        assert_eq!(report.status(true), Status::Error);
        report.push(Diagnostic::new(Code::Lc3002, "", ""));
        assert_eq!(report.status(false), Status::Error);
    }

    #[test]
    fn long_texts_are_cut_at_200_code_points() {
        let long = "\u{00E9}".repeat(1000);
        let mut report = Report::new(10);
        report.push(Diagnostic {
            function: Some(long.clone()),
            node: Some(Node { name: Some(long.clone()), text: long.clone() }),
            wires: vec![Wire { value: Some(long.clone()), from: Some(long.clone()), to: Some(long.clone()) }],
            expected: Some(long.clone()),
            found: Some(long.clone()),
            fixes: vec![Fix { title: long.clone(), edits: vec![] }],
            related: vec![Related { message: long.clone(), file: "f".to_owned(), span: None }],
            ..Diagnostic::new(Code::Lc3001, long.clone(), "f")
        });
        let d = &report.diagnostics()[0];
        let (node, wire) = (d.node.as_ref().unwrap(), &d.wires[0]);
        let texts = [
            &d.message,
            d.function.as_ref().unwrap(),
            node.name.as_ref().unwrap(),
            &node.text,
            wire.value.as_ref().unwrap(),
            wire.from.as_ref().unwrap(),
            wire.to.as_ref().unwrap(),
            d.expected.as_ref().unwrap(),
            d.found.as_ref().unwrap(),
            &d.fixes[0].title,
            &d.related[0].message,
        ];
        for text in texts {
            assert_eq!(text.chars().count(), MAX_TEXT);
            assert!(text.ends_with('\u{2026}'));
        }
    }

    #[test]
    fn a_fix_too_long_to_apply_keeps_its_title_only() {
        let position = Position { line: 1, column: 1 };
        let span = Span { start: position, end: position };
        let edit = |text: String| Edit { span, text };
        let mut report = Report::new(10);
        report.push(Diagnostic {
            fixes: vec![
                Fix { title: "short".to_owned(), edits: vec![edit("x".to_owned())] },
                Fix { title: "long".to_owned(), edits: vec![edit("x".to_owned()), edit("x".repeat(MAX_TEXT + 1))] },
                Fix { title: "many".to_owned(), edits: vec![edit("x".to_owned()); MAX_LIST + 1] },
            ],
            ..Diagnostic::new(Code::Lc3001, "", "f")
        });
        let fixes = &report.diagnostics()[0].fixes;
        assert_eq!(fixes[0].edits.len(), 1);
        assert!(fixes[1].edits.is_empty());
        assert!(fixes[2].edits.is_empty());
    }

    #[test]
    fn lists_of_a_diagnostic_are_bounded() {
        let related = Related { message: String::new(), file: "f".to_owned(), span: None };
        let mut report = Report::new(10);
        report.push(Diagnostic {
            wires: vec![Wire::default(); MAX_LIST + 1],
            fixes: vec![Fix { title: String::new(), edits: vec![] }; MAX_LIST + 1],
            related: vec![related; 100_000],
            ..Diagnostic::new(Code::Lc2013, "", "f")
        });
        let d = &report.diagnostics()[0];
        assert_eq!((d.wires.len(), d.fixes.len(), d.related.len()), (MAX_LIST, MAX_LIST, MAX_LIST));
    }

    #[test]
    fn a_text_of_exactly_200_code_points_stays() {
        let exact = "a".repeat(MAX_TEXT);
        assert_eq!(cut_text(&exact), exact);
        let over = "a".repeat(MAX_TEXT + 1);
        assert_eq!(cut_text(&over), format!("{}\u{2026}", "a".repeat(MAX_TEXT - 1)));
        assert_eq!(cut_text(""), "");
    }

    #[test]
    fn diagnostics_sort_by_file_line_column_code_and_message() {
        let unsorted = [
            diagnostic(Code::Lc3002, "b.lcf", at(1, 1), "z"),
            diagnostic(Code::Lc3002, "a.lcf", at(2, 1), "z"),
            diagnostic(Code::Lc3002, "a.lcf", at(1, 9), "z"),
            diagnostic(Code::Lc3002, "a.lcf", at(1, 1), "z"),
            diagnostic(Code::Lc3001, "a.lcf", at(1, 1), "z"),
            diagnostic(Code::Lc3001, "a.lcf", at(1, 1), "a"),
            diagnostic(Code::Lc4101, "a.lcf", None, "z"),
        ];
        let mut report = Report::new(100);
        for item in unsorted.iter().cloned() {
            report.push(item);
        }
        report.finish();
        let order: Vec<(&str, Option<Position>, &str, &str)> = report.diagnostics().iter().map(Diagnostic::sort_key).collect();
        let mut expected = order.clone();
        expected.sort();
        assert_eq!(order, expected);
        assert_eq!(report.diagnostics()[0].code(), Code::Lc4101, "no span first");
        assert_eq!(report.diagnostics()[1].message, "a");
        assert_eq!(report.diagnostics()[6].file, "b.lcf");
    }

    #[test]
    fn the_order_of_arrival_does_not_change_the_report() {
        let end = |column| Position { line: 1, column };
        let start = end(1);
        let (x, y) = (
            Diagnostic { span: Some(Span { start, end: end(2) }), ..Diagnostic::new(Code::Lc3002, "m", "f") },
            Diagnostic { span: Some(Span { start, end: end(3) }), function: Some("g".to_owned()), ..Diagnostic::new(Code::Lc3002, "m", "f") },
        );
        for max_report in [1, 10] {
            let (mut one, mut other) = (Report::new(max_report), Report::new(max_report));
            one.push(x.clone());
            one.push(y.clone());
            other.push(y.clone());
            other.push(x.clone());
            one.finish();
            other.finish();
            assert_eq!(one.diagnostics(), other.diagnostics(), "{max_report}");
        }
    }

    #[test]
    fn file_order_is_code_point_order() {
        let mut report = Report::new(100);
        for file in ["\u{00E9}.lcf", "z.lcf", "Z.lcf"] {
            report.push(Diagnostic::new(Code::Lc0003, "", file));
        }
        report.finish();
        let files: Vec<&str> = report.diagnostics().iter().map(|diagnostic| diagnostic.file.as_str()).collect();
        assert_eq!(files, ["Z.lcf", "z.lcf", "\u{00E9}.lcf"]);
    }

    #[test]
    fn cutting_keeps_errors_before_warnings_and_complete_counts() {
        let mut report = Report::new(2);
        report.push(diagnostic(Code::Lc4101, "a.lcf", at(1, 1), "first warning"));
        report.push(diagnostic(Code::Lc3002, "z.lcf", at(1, 1), "second error"));
        report.push(diagnostic(Code::Lc3002, "m.lcf", at(1, 1), "first error"));
        report.finish();
        assert!(report.truncated());
        assert_eq!((report.errors(), report.warnings()), (2, 1));
        let files: Vec<&str> = report.diagnostics().iter().map(|diagnostic| diagnostic.file.as_str()).collect();
        assert_eq!(files, ["m.lcf", "z.lcf"]);
    }

    #[test]
    fn a_report_within_the_limit_is_not_truncated() {
        let mut report = Report::new(1);
        report.push(Diagnostic::new(Code::Lc3002, "", ""));
        report.finish();
        assert!(!report.truncated());
        assert_eq!(report.diagnostics().len(), 1);
    }

    #[test]
    fn a_limit_of_zero_lists_nothing_and_counts_everything() {
        let mut report = Report::new(0);
        report.push(Diagnostic::new(Code::Lc3002, "", ""));
        assert!(report.truncated());
        report.finish();
        assert!(report.diagnostics().is_empty());
        assert_eq!(report.errors(), 1);
        assert_eq!(report.status(false), Status::Error);
    }

    /// A deterministic mix of errors and warnings, in no useful order.
    fn many(count: u32) -> impl Iterator<Item = Diagnostic> {
        (0..count).map(|i| {
            let mixed = i.wrapping_mul(2_654_435_761);
            let code = if mixed % 3 == 0 { Code::Lc4101 } else { Code::Lc3002 };
            diagnostic(code, &format!("{}.lcf", mixed % 50), at(mixed % 1000 + 1, 1), "m")
        })
    }

    #[test]
    fn memory_stays_bounded_while_the_counts_stay_complete() {
        let mut report = Report::new(10);
        for item in many(100_000) {
            report.push(item);
            assert!(report.diagnostics().len() <= 20);
        }
        assert_eq!(report.errors() + report.warnings(), 100_000);
        assert!(report.truncated());
    }

    #[test]
    fn pruning_gives_the_same_report_as_keeping_everything() {
        let mut kept = Report::new(usize::MAX);
        let mut pruned = Report::new(10);
        for item in many(5_000) {
            kept.push(item.clone());
            pruned.push(item);
        }
        kept.max_report = 10;
        kept.finish();
        pruned.finish();
        assert_eq!(kept.diagnostics(), pruned.diagnostics());
        assert_eq!((kept.errors(), kept.warnings()), (pruned.errors(), pruned.warnings()));
    }

    #[test]
    fn a_file_is_named_once() {
        let mut report = Report::new(10);
        report.push(Diagnostic::new(Code::Lc0005, "", ""));
        report.push(Diagnostic::new(Code::Lc0005, "", "other.lcf"));
        report.set_file("f.lcf");
        report.finish();
        let files: Vec<&str> = report.diagnostics().iter().map(|diagnostic| diagnostic.file.as_str()).collect();
        assert_eq!(files, ["f.lcf", "other.lcf"]);
    }

    #[test]
    fn merging_adds_the_counts_and_stays_bounded() {
        let (mut a, mut b) = (Report::new(1), Report::new(1));
        a.push(Diagnostic::new(Code::Lc3002, "", "a"));
        b.push(Diagnostic::new(Code::Lc4101, "", "b"));
        a.merge(b);
        a.finish();
        assert_eq!((a.errors(), a.warnings()), (1, 1));
        assert!(a.truncated());
        assert_eq!(a.diagnostics().len(), 1);
        assert_eq!(a.diagnostics()[0].code(), Code::Lc3002);
    }

    #[test]
    fn merging_errors_keeps_their_count_even_when_the_list_is_cut_and_leaves_warnings_out() {
        let mut other = Report::new(0);
        other.push(Diagnostic::new(Code::Lc1001, "m", "a.lcf"));
        other.push(Diagnostic::new(Code::Lc0005, "m", "a.lcf"));
        assert!(other.diagnostics().is_empty());
        let mut report = Report::new(10);
        report.merge_errors(other);
        assert_eq!((report.errors(), report.warnings()), (1, 0));
        let mut other = Report::new(10);
        other.push(Diagnostic::new(Code::Lc1001, "m", "a.lcf"));
        other.push(Diagnostic::new(Code::Lc0005, "m", "a.lcf"));
        let mut report = Report::new(10);
        report.merge_errors(other);
        assert_eq!((report.errors(), report.warnings(), report.diagnostics().len()), (1, 0, 1));
    }
}
