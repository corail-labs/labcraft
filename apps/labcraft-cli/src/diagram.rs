//! `labcraft-cli diagram --svg` (`docs/cli.md`, section 7). `check` checks the folder, the `diagram` crate draws each
//! function; here are the files of the SVGs, the names of the functions and the report.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use labcraft_check::Checked;
use labcraft_diagram::{DiagramLimits, Theme, ViewState, svg};
use labcraft_types::{CheckCounts, CheckSummary, Code, Diagnostic, Fix, Interner, Report, Status, cut_text};
use serde::Serialize;

use crate::args::{self, DiagramOptions};
use crate::disk::Disk;
use crate::folder::{self, Drawn3, Function, Loaded};
use crate::output;
use crate::walk::{io_text, shown_of, slashed};

/// The schema of the report, which the attributes of the SVG share (`docs/cli.md`, 7.2 and 7.5).
const SCHEMA: &str = "labcraft-diagram/1";

/// What became of one function.
struct Entry {
    function: String,
    file: Option<String>,
    svg: Option<String>,
    drawn: bool,
    nodes: Option<u64>,
    codes: Vec<Code>,
}

pub fn run(options: &DiagramOptions) -> ExitCode {
    let current = std::env::current_dir().ok();
    let Loaded { mut report, counts, checked, names, functions, root } = folder::load(&options.check, current.as_deref());
    let Some(root) = root else {
        // The folder could not be read: the walk said why, and there is nothing to draw.
        return answer(options, report, CheckCounts::default(), Vec::new());
    };
    // The functions asked for, or every one; an unknown name gets `LC7001`, with the nearest names. Two files can give
    // one name (`a.b.lcf` and `a/b.lcf`): both are drawn, each at its path.
    let mut entries: Vec<Entry> = Vec::new();
    let mut asked: Vec<&Function> = Vec::new();
    if options.functions.is_empty() {
        asked.extend(&functions);
    } else {
        for name in &options.functions {
            let before = asked.len();
            asked.extend(functions.iter().filter(|function| function.name == *name));
            if asked.len() == before {
                report.push(unknown(name, &functions, &root));
                entries.push(Entry { function: name.clone(), file: None, svg: None, drawn: false, nodes: None, codes: vec![Code::Lc7001] });
            }
        }
        asked.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.shown.cmp(&b.shown)));
    }
    let drawings = drawings(&asked, checked.as_ref(), &names, options, &mut report);
    let written = write(&asked, drawings, options, current.as_deref(), &report);
    for (entry, failure) in written {
        if let Some(failure) = failure {
            report.push(failure);
        }
        entries.push(entry);
    }
    report.finish();
    entries.sort_by(|a, b| a.function.cmp(&b.function));
    answer(options, report, counts, entries)
}

/// `LC7001`, at the folder, with the nearest names as fixes.
fn unknown(name: &str, functions: &[Function], folder: &str) -> Diagnostic {
    let mut diagnostic = Diagnostic::new(Code::Lc7001, format!("unknown function `{name}`: not a function of this folder"), folder);
    let mut near: Vec<(usize, &str)> =
        functions.iter().filter_map(|other| near(name, &other.name).map(|edits| (edits, other.name.as_str()))).collect();
    near.sort();
    near.dedup();
    diagnostic.fixes = near.into_iter().take(3).map(|(_, other)| Fix { title: format!("did you mean `{other}`?"), edits: Vec::new() }).collect();
    diagnostic
}

/// The edits from one name to another, when they are near: at most a third of the longer one.
fn near(a: &str, b: &str) -> Option<usize> {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a.len() > 200 || b.len() > 200 {
        return None;
    }
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, x) in a.iter().enumerate() {
        let mut row = vec![i + 1];
        for (j, y) in b.iter().enumerate() {
            let replace = previous.get(j).copied().unwrap_or(0) + usize::from(x != y);
            let insert = row.get(j).copied().unwrap_or(0) + 1;
            let delete = previous.get(j + 1).copied().unwrap_or(0) + 1;
            row.push(replace.min(insert).min(delete));
        }
        previous = row;
    }
    let edits = previous.last().copied().unwrap_or(0);
    (edits * 3 <= a.len().max(b.len())).then_some(edits)
}

/// The drawing of each function, or none when it is not drawable (`docs/cli.md`, 7.1); `LC7002` joins the report.
fn drawings(asked: &[&Function], checked: Option<&Checked>, names: &Interner, options: &DiagramOptions, report: &mut Report) -> Vec<Drawn3> {
    let files = folder::files_of(checked);
    let limits = DiagramLimits { max_nodes: options.max_nodes };
    asked.iter().map(|function| folder::drawing_of(function, checked, &files, names, &ViewState::default(), &limits, report)).collect()
}

/// Each SVG, with the marks or the lines of its first diagnostics, written to its path under `DIR` through a temporary
/// file. On a file system that ignores case, a path that names a file written before it, in another case, is not
/// written (`LC7004`).
fn write(
    asked: &[&Function],
    drawings: Vec<Drawn3>,
    options: &DiagramOptions,
    current: Option<&Path>,
    report: &Report,
) -> Vec<(Entry, Option<Diagnostic>)> {
    let theme = match options.theme {
        args::Theme::Both => Theme::Both,
        args::Theme::Light => Theme::Light,
        args::Theme::Dark => Theme::Dark,
    };
    let current = current.and_then(|current| fs::canonicalize(current).ok());
    // The paths written, by their spelling in lower case: how the report shows each, and its real path.
    let mut written_before: HashMap<String, (String, PathBuf)> = HashMap::new();
    asked
        .iter()
        .zip(drawings)
        .map(|(function, (drawing, nodes, codes))| {
            let drawn = drawing.is_some();
            let codes = if drawn { Vec::new() } else { codes };
            let picture = folder::picture(function, drawing, report);
            let path = options.out.join(&function.relative);
            let mut entry = Entry { function: function.name.clone(), file: Some(function.shown.clone()), svg: None, drawn, nodes, codes };
            let lower = slashed(&path).to_lowercase();
            if let Some((before, real)) = written_before.get(&lower)
                && fs::canonicalize(&path).is_ok_and(|again| again == *real)
            {
                let message =
                    format!("cannot write this SVG: on this file system, it is `{before}`, written before it, whose path differs only in case");
                return (entry, Some(Diagnostic::new(Code::Lc7004, message, function.shown.as_str())));
            }
            let written =
                path.parent().map_or(Ok(()), fs::create_dir_all).and_then(|()| craft_atomic::atomic_write(&path, svg(&picture, theme).as_bytes()));
            let shown = shown_of(&Disk, &path, fs::canonicalize(&path).ok().as_deref(), current.as_deref());
            match written {
                Ok(()) => {
                    if let Ok(real) = fs::canonicalize(&path) {
                        written_before.entry(lower).or_insert((shown.clone(), real));
                    }
                    entry.svg = Some(shown);
                    (entry, None)
                }
                Err(error) => {
                    let message = format!("cannot write `{shown}`: {}", io_text(&error));
                    (entry, Some(Diagnostic::new(Code::Lc7004, message, function.shown.as_str())))
                }
            }
        })
        .collect()
}

#[derive(Serialize)]
struct Document<'a> {
    schema: &'static str,
    status: Status,
    summary: Summary,
    diagrams: Vec<JsonEntry<'a>>,
    check: CheckPart<'a>,
}

#[derive(Serialize)]
struct Summary {
    functions: u64,
    drawn: u64,
    not_drawn: u64,
}

#[derive(Serialize)]
struct JsonEntry<'a> {
    function: std::borrow::Cow<'a, str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    svg: Option<&'a str>,
    drawn: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    nodes: Option<u64>,
    #[serde(skip_serializing_if = "<[Code]>::is_empty")]
    codes: &'a [Code],
}

#[derive(Serialize)]
struct CheckPart<'a> {
    summary: CheckSummary,
    diagnostics: &'a [Diagnostic],
}

/// The report, then the exit code: 1 when a function is not drawn, a name is unknown, there is nothing to draw, or an
/// SVG could not be written (`docs/cli.md`, 7.1).
fn answer(options: &DiagramOptions, mut report: Report, counts: CheckCounts, entries: Vec<Entry>) -> ExitCode {
    report.finish();
    let drawn = entries.iter().filter(|entry| entry.drawn).count() as u64;
    let not_drawn = entries.len() as u64 - drawn;
    // An error that no drawing marks: in a file that is not that of a function drawn, or about no file (cli.md 7.1).
    let marked: HashSet<&str> = entries.iter().filter(|entry| entry.drawn).filter_map(|entry| entry.file.as_deref()).collect();
    let unmarked = report.file_counts().any(|(file, counts)| counts.errors > 0 && !marked.contains(file));
    let unwritten = entries.iter().any(|entry| entry.file.is_some() && entry.svg.is_none());
    let failed = entries.is_empty() || not_drawn > 0 || unwritten || unmarked;
    let status = if failed { Status::Error } else { Status::Ok };
    if options.check.json {
        let document = Document {
            schema: SCHEMA,
            status,
            summary: Summary { functions: entries.len() as u64, drawn, not_drawn },
            diagrams: entries
                .iter()
                .map(|entry| JsonEntry {
                    function: cut_text(&entry.function),
                    file: entry.file.as_deref(),
                    svg: entry.svg.as_deref(),
                    drawn: entry.drawn,
                    nodes: entry.nodes,
                    codes: &entry.codes,
                })
                .collect(),
            check: CheckPart { summary: report.check_document(counts, false).summary, diagnostics: report.diagnostics() },
        };
        output::write_json(&document);
    } else {
        let mut out = output::diagnostics_text(&report);
        for entry in &entries {
            let state = if entry.drawn { "drawn" } else { "not drawn" };
            let place = entry.svg.as_deref().unwrap_or("not written");
            let what = match (entry.drawn, entry.file.is_some(), entry.nodes) {
                (_, false, _) => "unknown function".to_owned(),
                (true, true, Some(1)) => "1 node".to_owned(),
                (true, true, nodes) => format!("{} nodes", nodes.unwrap_or(0)),
                (false, true, _) => "its diagnostics".to_owned(),
            };
            let name = cut_text(&entry.function);
            let line = if entry.file.is_some() { format!("{state} {name}: {place}, {what}") } else { format!("{state} {name}: {what}") };
            output::push_line(&mut out, &line);
        }
        let _ = write!(
            out,
            "diagram: {} {}, {drawn} drawn, {not_drawn} not drawn: {}, {}",
            entries.len(),
            if entries.len() == 1 { "function" } else { "functions" },
            output::counted(report.errors(), "error", "errors"),
            output::counted(report.warnings(), "warning", "warnings"),
        );
        if report.truncated() {
            let _ = write!(out, " ({} listed)", report.diagnostics().len());
        }
        out.push('\n');
        output::write_text(&out);
    }
    if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

/// When the work could not start or ended outside any function, the answer keeps its form: a report, with `LC0099`.
pub fn internal_failure(options: &DiagramOptions, what: &str) -> ExitCode {
    let mut report = Report::new(options.check.limits.max_report);
    report.push(Diagnostic::new(Code::Lc0099, format!("the tool could not finish: {what}"), "labcraft-cli"));
    answer(options, report, CheckCounts::default(), Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn near_names_are_a_third_of_their_length_apart_at_most() {
        assert_eq!(near("check_voltag", "check_voltage"), Some(1));
        assert_eq!(near("abc", "xyz"), None);
        assert_eq!(near("measure", "measure"), Some(0));
    }
}
