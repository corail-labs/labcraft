//! `labcraft-cli diagram --svg` (`docs/cli.md`, section 7). `check` checks the folder, the `diagram` crate draws each
//! function, `engine::drawings` names the functions and writes the report, as for the command `diagram.svg`; here are
//! the files of the SVGs.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use labcraft_diagram::{DiagramLimits, Theme, svg};
use labcraft_engine::drawings::{self, Drawn3, Entry, Function, Loaded};
use labcraft_types::{CheckCounts, Code, Diagnostic, Report, cut_text};

use crate::args::{self, DiagramOptions};
use crate::disk::Disk;
use crate::output;
use crate::walk::{io_text, shown_of, slashed};

pub fn run(options: &DiagramOptions) -> ExitCode {
    let current = std::env::current_dir().ok();
    let shipped = crate::shipped();
    let Some(folder) = options.check.folders.first() else {
        return answer(options, Report::new(options.check.limits.max_report), CheckCounts::default(), Vec::new());
    };
    let mut loaded = drawings::load(&Disk, &shipped, folder, &options.check.lib_paths, &options.check.limits, current.as_deref());
    let mut report = std::mem::replace(&mut loaded.report, Report::new(0));
    let Some(root) = loaded.root.clone() else {
        // The folder could not be read: the walk said why, and there is nothing to draw.
        return answer(options, report, CheckCounts::default(), Vec::new());
    };
    // The functions asked for, or every one; an unknown name gets `LC7001`, with the nearest names. Two files can give
    // one name (`a.b.lcf` and `a/b.lcf`): both are drawn, each at its path.
    let (asked, mut entries) = drawings::asked(&options.functions, &loaded.functions, &root, &mut report);
    let drawn = drawings::drawings(&asked, &loaded, &DiagramLimits { max_nodes: options.max_nodes }, &mut report);
    let written = write(&asked, drawn, options, current.as_deref(), &report);
    for (entry, failure) in written {
        if let Some(failure) = failure {
            report.push(failure);
        }
        entries.push(entry);
    }
    report.finish();
    entries.sort_by(|a, b| a.function.cmp(&b.function));
    let Loaded { counts, .. } = loaded;
    answer(options, report, counts, entries)
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
            let picture = drawings::picture(function, drawing, report);
            let path = options.out.join(&function.relative);
            let mut entry = Entry { function: function.name.clone(), file: Some(function.shown.clone()), drawn, nodes, codes, ..Entry::default() };
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

/// The report, then the exit code: 1 when a function is not drawn, a name is unknown, there is nothing to draw, or an
/// SVG could not be written (`docs/cli.md`, 7.1).
fn answer(options: &DiagramOptions, mut report: Report, counts: CheckCounts, entries: Vec<Entry>) -> ExitCode {
    report.finish();
    let failed = drawings::failed(&entries, &report, true);
    if options.check.json {
        output::write_json(&drawings::document(&entries, &report, counts, failed));
    } else {
        let drawn = entries.iter().filter(|entry| entry.drawn).count();
        let not_drawn = entries.len() - drawn;
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
