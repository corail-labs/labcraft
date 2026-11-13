//! `labcraft-cli run` (`docs/cli.md`, section 6), through `engine` as `check` and `fmt` are: `run.start` with what the
//! command line gives beyond its parameters (`engine::runs::CommandLine`), then `run.wait` once a second until the run
//! ends. Here are the disk, the real clock, the id of the process, Ctrl-C and the answer.

// `report["key"]` of `serde_json::Value` never panics: a key that is missing, or a value that is no object, gives `Null`.
#![allow(clippy::indexing_slicing, reason = "indexing a `serde_json::Value` by a key gives `Null`, never a panic")]

use std::fmt::Write as _;
use std::io::{IsTerminal as _, Write as _};
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use labcraft_engine::runs::{CommandLine, Outside};
use labcraft_engine::workspace::Workspace;
use labcraft_engine::{Config, Runs, Session};
use serde_json::{Value as Json, json};

use crate::args::{Limit, RunOptions, TraceTo};
use crate::disk::Disk;
use crate::output::{counted, write_json, write_text};
use labcraft_types::printed::push_line;

/// How often the run is asked where it is, and the line of stderr written again.
const EVERY: &str = "1 s";

/// The exit codes of `run` (`docs/cli.md`, 6.1).
fn exit_of(status: &str) -> ExitCode {
    ExitCode::from(match status {
        "ok" => 0,
        "aborted" => 3,
        "fault" => 4,
        _ => 1,
    })
}

/// A path of an option, as `check` shows it: from the current folder.
fn shown(path: &Path, current: Option<&Path>) -> String {
    labcraft_engine::walk::shown_of(&Disk, path, std::fs::canonicalize(path).ok().as_deref(), current)
}

fn outside(path: &Path, current: Option<&Path>) -> Outside {
    Outside { path: std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()), shown: shown(path, current) }
}

/// `<labcraft>/visa/sim/dmm-230V.toml`: a file shipped with LabCraft (`docs/commands.md`, section 2).
fn shipped_sim(path: &Path) -> Option<&str> {
    path.to_str().filter(|text| text.starts_with("<labcraft>/"))
}

/// The parameters of `run.start` that the options give (`docs/commands.md`, 6.4).
fn params_of(options: &RunOptions) -> Json {
    let mut params = json!({
        "function": options.function,
        "deny_warnings": options.deny_warnings,
        "trace": if options.trace == TraceTo::None { "none" } else { "file" },
        "mode": if options.real_time { "real_time" } else { "deterministic" },
        "wait": EVERY,
    });
    let Some(object) = params.as_object_mut() else { return params };
    // A simulation file shipped with LabCraft is named as `run.start` names it.
    if let Some(sim) = options.sim.as_deref().and_then(shipped_sim) {
        object.insert("sim".to_owned(), json!(sim));
    }
    if let Some(seed) = options.schedule_seed {
        // Beyond 2^53 - 1, a string of digits (commands.md 6.4).
        object.insert("schedule_seed".to_owned(), if seed < (1 << 53) { json!(seed) } else { json!(seed.to_string()) });
    }
    for (name, value) in [("timeout", &options.timeout), ("max_real_time", &options.max_real_time)] {
        if let Some(value) = value {
            object.insert(name.to_owned(), json!(value));
        }
    }
    for (name, limit) in [("max_iterations", options.max_iterations), ("max_nodes", options.max_nodes), ("max_made", options.max_made)] {
        match limit {
            Some(Limit::Count(count)) => object.insert(name.to_owned(), json!(count)),
            Some(Limit::None) => object.insert(name.to_owned(), Json::Null),
            None => None,
        };
    }
    for (name, value) in [("max_call_depth", options.max_call_depth), ("max_memory", options.max_memory)] {
        if let Some(value) = value {
            object.insert(name.to_owned(), json!(value));
        }
    }
    params
}

/// The Ctrl-C of the terminal (`docs/cli.md`, 6.5): the first aborts the run, the second ends the grace period. Counted
/// here too, so that one that comes before the run is going still reaches it.
fn on_interrupt(runs: &Runs) -> Arc<AtomicU32> {
    let presses = Arc::new(AtomicU32::new(0));
    let (counted, runs) = (Arc::clone(&presses), runs.clone());
    let _ = ctrlc::set_handler(move || {
        if counted.fetch_add(1, Ordering::SeqCst) == 0 {
            runs.abort_going();
        } else {
            runs.end_grace();
        }
    });
    presses
}

/// The exit code of the run, or the message of a wrong usage that `run.start` found.
pub fn run(options: &RunOptions) -> Result<ExitCode, String> {
    let current = Disk.current().and_then(|current| std::fs::canonicalize(current).ok());
    let current = current.as_deref();
    let config = Config {
        root: options.folder.clone(),
        lib_paths: options.lib_paths.clone(),
        limits: options.limits.clone(),
        shipped: crate::shipped(),
        natives: vec![Arc::new(labcraft_visa::Visa), Arc::new(labcraft_results_log::ResultsLog)],
        clock: Arc::new(crate::clock::Real::new()),
        process: std::process::id(),
    };
    // A path is data: a `FOLDER` that does not read is a refused run, not wrong usage (cli.md 6.1).
    let mut session = match Session::new(Arc::new(Disk), config, false) {
        Ok(session) => session,
        Err(error) => {
            let folder = shown(&options.folder, current);
            let diagnostic =
                json!({ "code": "LC0001", "severity": "error", "message": format!("cannot read this path: {}", error.message), "file": folder });
            return Ok(answer(options, &labcraft_engine::runs::refused_report(&options.function, options.real_time, vec![diagnostic]), &[]));
        }
    };
    let runs = session.runs();
    let presses = on_interrupt(&runs);
    let line = CommandLine {
        current: current.map(Path::to_path_buf),
        folder: Some(options.folder.clone()),
        texts: !options.json,
        args: options.args.clone(),
        args_file: options.args_file.as_deref().map(|path| outside(path, current)),
        sim: options.sim.as_deref().filter(|path| shipped_sim(path).is_none()).map(|path| outside(path, current)),
        set: options.set.clone(),
        trace: match &options.trace {
            TraceTo::File(path) => Some(outside(path, current)),
            _ => None,
        },
    };
    let mut report = match session.start_run(&params_of(options), line) {
        Ok(report) => report,
        Err(error) if error.code == "internal" => return Ok(internal_failure(options, &error.message)),
        // What the command line did not see as wrong usage, `run.start` does (commands.md 6.4).
        Err(error) => return Err(error.message),
    };
    let progress = !options.json && std::io::stderr().is_terminal();
    let mut shown_width = 0;
    while report.get("status").and_then(Json::as_str) == Some("running") {
        match presses.load(Ordering::SeqCst) {
            0 => {}
            1 => runs.abort_going(),
            _ => {
                runs.abort_going();
                runs.end_grace();
            }
        }
        if progress {
            shown_width = show_progress(&report, shown_width);
        }
        report = match runs.execute(session.workspace(), session.root(), "run.wait", &json!({ "wait": EVERY })) {
            Ok(report) => report,
            Err(error) => return Ok(internal_failure(options, &error.message)),
        };
    }
    if shown_width > 0 {
        let _ = write!(std::io::stderr(), "\r{}\r", " ".repeat(shown_width));
    }
    let texts = report.get("run").and_then(Json::as_str).and_then(|id| runs.output_texts(id)).unwrap_or_default();
    labcraft_engine::runs::let_go();
    drop(session);
    Ok(answer(options, &report, &texts))
}

/// Where the run is, on one line of stderr written again (cli.md 6.7); its width, to clear it.
fn show_progress(report: &Json, before: usize) -> usize {
    let mut text = format!("{}: running, {}", report["function"].as_str().unwrap_or_default(), clock_text(report));
    for at in report["loops"].as_array().map(Vec::as_slice).unwrap_or_default() {
        let _ = write!(text, "; loop `{}` at iteration {}", at["node"].as_str().unwrap_or_default(), at["iteration"].as_u64().unwrap_or(0) + 1);
    }
    let text = labcraft_types::printed::printable_line(&text);
    let width = text.chars().count();
    let _ = write!(std::io::stderr(), "\r{text}{}", " ".repeat(before.saturating_sub(width)));
    let _ = std::io::stderr().flush();
    width
}

/// When the work could not finish, the answer keeps its form: a refused run, with `LC0099`.
pub fn internal_failure(options: &RunOptions, what: &str) -> ExitCode {
    let diagnostic =
        json!({ "code": "LC0099", "severity": "error", "message": format!("the tool could not finish: {what}"), "file": "labcraft-cli" });
    answer(options, &labcraft_engine::runs::refused_report(&options.function, options.real_time, vec![diagnostic]), &[])
}

fn answer(options: &RunOptions, report: &Json, texts: &[Option<String>]) -> ExitCode {
    if options.json {
        write_json(report);
    } else {
        write_text(&human(report, texts));
    }
    exit_of(report.get("status").and_then(Json::as_str).unwrap_or("refused"))
}

fn ms_text(ms: f64) -> String {
    if ms.fract() == 0.0 && ms.abs() < 1e15 { format!("{ms:.0} ms") } else { format!("{ms} ms") }
}

fn clock_text(report: &Json) -> String {
    let clock = &report["clock"];
    let real = clock["real_ms"].as_u64().unwrap_or(0);
    match clock["simulated_ms"].as_f64() {
        Some(simulated) => format!("{} simulated, {real} ms real", ms_text(simulated)),
        None => format!("{real} ms real"),
    }
}

/// `file:line:column` of a place in the report, or the file alone.
fn place(file: &Json, span: &Json) -> String {
    let file = file.as_str().unwrap_or_default();
    match (span["start"]["line"].as_u64(), span["start"]["column"].as_u64()) {
        (Some(line), Some(column)) => format!("{file}:{line}:{column}"),
        _ => file.to_owned(),
    }
}

/// A diagnostic of the report, as `check` writes one (cli.md 4.1).
fn diagnostic_text(out: &mut String, diagnostic: &Json) {
    let text = |field: &str| diagnostic[field].as_str().unwrap_or_default().to_owned();
    push_line(out, &format!("{}: {}[{}]: {}", place(&diagnostic["file"], &diagnostic["span"]), text("severity"), text("code"), text("message")));
    let node = diagnostic.get("node").map(|node| match node["name"].as_str() {
        Some(name) => format!("node `{name}`: {}", node["text"].as_str().unwrap_or_default()),
        None => format!("node: {}", node["text"].as_str().unwrap_or_default()),
    });
    match (diagnostic["function"].as_str(), node) {
        (Some(function), Some(node)) => push_line(out, &format!("  in function `{function}`, {node}")),
        (Some(function), None) => push_line(out, &format!("  in function `{function}`")),
        (None, Some(node)) => push_line(out, &format!("  in {node}")),
        (None, None) => {}
    }
    for wire in diagnostic["wires"].as_array().map(Vec::as_slice).unwrap_or_default() {
        let name = wire["value"].as_str().map_or_else(|| "terminal".to_owned(), |value| format!("wire `{value}`"));
        let route = match (wire["from"].as_str(), wire["to"].as_str()) {
            (Some(from), Some(to)) => format!("{from} -> {to}"),
            (Some(terminal), None) | (None, Some(terminal)) => terminal.to_owned(),
            (None, None) => String::new(),
        };
        push_line(out, &format!("  {name}: {route}"));
    }
    match (diagnostic["expected"].as_str(), diagnostic["found"].as_str()) {
        (Some(expected), Some(found)) => push_line(out, &format!("  expected `{expected}`, found `{found}`")),
        (Some(expected), None) => push_line(out, &format!("  expected `{expected}`")),
        (None, Some(found)) => push_line(out, &format!("  found `{found}`")),
        (None, None) => {}
    }
    for fix in diagnostic["fixes"].as_array().map(Vec::as_slice).unwrap_or_default() {
        push_line(out, &format!("  fix: {}", fix["title"].as_str().unwrap_or_default()));
    }
    for related in diagnostic["related"].as_array().map(Vec::as_slice).unwrap_or_default() {
        push_line(out, &format!("  {}: {}", place(&related["file"], &related["span"]), related["message"].as_str().unwrap_or_default()));
    }
}

/// Where a resource was opened, for the lines of resources.
fn resource_text(resource: &Json) -> String {
    let mut text = format!("{} {}", resource["type"].as_str().unwrap_or_default(), resource["resource"].as_str().unwrap_or_default());
    if let Some(function) = resource["function"].as_str() {
        let _ = write!(text, ", opened in `{function}`");
        if let Some(node) = resource["node"].as_str() {
            let _ = write!(text, " by `{node}`");
        }
        let _ = write!(text, " ({})", place(&resource["file"], &resource["span"]));
    }
    text
}

/// The human-readable form (cli.md 6.7): what `check` found first, then the run, its outputs, then what a reader checks
/// first: the resources, the error, the trace. Every line goes through the escapes of cli.md 4.1.
pub fn human(report: &Json, texts: &[Option<String>]) -> String {
    let mut out = String::new();
    for diagnostic in report["check"]["diagnostics"].as_array().map(Vec::as_slice).unwrap_or_default() {
        diagnostic_text(&mut out, diagnostic);
    }
    let function = report["function"].as_str().unwrap_or_default();
    let status = report["status"].as_str().unwrap_or_default();
    if status == "refused" {
        push_line(&mut out, &format!("{function}: refused"));
        return out;
    }
    push_line(&mut out, &format!("{function}: {status}, {}", clock_text(report)));
    let outputs = report["outputs"].as_array().map(Vec::as_slice).unwrap_or_default();
    let width = outputs.iter().map(|output| output["name"].as_str().unwrap_or_default().chars().count()).max().unwrap_or(0);
    for (index, output) in outputs.iter().enumerate() {
        let name = output["name"].as_str().unwrap_or_default();
        // A value that the report cuts whole is cut here too; past the budget of texts, its JSON form.
        let value = match texts.get(index).cloned().flatten() {
            Some(text) if output["value"].get("$cut").is_none() => text,
            _ => output["value"].to_string(),
        };
        push_line(&mut out, &format!("  {name:<width$} = {value}"));
    }
    if let Some(more) = report["outputs_more"].as_u64() {
        push_line(&mut out, &format!("  … {} more", counted(more, "output", "outputs")));
    }
    match status {
        "error" => error_text(&mut out, &report["error"]),
        "fault" => {
            let fault = &report["fault"];
            let message = fault["message"].as_str().unwrap_or_default();
            push_line(
                &mut out,
                &format!("{}: error[{}]: fault: {message}", place(&fault["file"], &fault["span"]), fault["code"].as_str().unwrap_or_default()),
            );
            loops_text(&mut out, &fault["loops"]);
        }
        "aborted" => {
            let abort = &report["abort"];
            let by = match abort["by"].as_str() {
                Some("interrupt") => "Ctrl-C",
                Some("timeout") => "--timeout",
                other => other.unwrap_or("a request"),
            };
            push_line(&mut out, &format!("aborted by {by} at {}", ms_text(abort["at_ms"].as_f64().unwrap_or(0.0))));
            loops_text(&mut out, &abort["loops"]);
            if let Some(warning) = abort["warning"].as_str() {
                push_line(&mut out, &format!("warning: {warning}"));
            }
        }
        _ => {}
    }
    let resources = &report["resources"];
    let open = resources["open"].as_array().map(Vec::as_slice).unwrap_or_default();
    let leaked = resources["leaked"].as_array().map(Vec::as_slice).unwrap_or_default();
    let mut line = format!("resources: {} opened, {} open", resources["opened"].as_u64().unwrap_or(0), open.len());
    if !leaked.is_empty() {
        let _ = write!(line, ", {} leaked", leaked.len());
    }
    push_line(&mut out, &line);
    for resource in open {
        push_line(&mut out, &format!("  open: {}", resource_text(resource)));
    }
    for resource in leaked {
        push_line(&mut out, &format!("  leaked: {}", resource_text(resource)));
    }
    let trace = &report["trace"];
    if let Some(error) = trace.get("error") {
        push_line(&mut out, &format!("trace: not written: {}", error["message"].as_str().unwrap_or_default()));
    } else if let Some(file) = trace["file"].as_str() {
        let events = trace["events"].as_u64().unwrap_or(0);
        let truncated = if trace["truncated"].as_bool() == Some(true) { ", truncated" } else { "" };
        push_line(&mut out, &format!("trace: {file}, {}{truncated}", counted(events, "event", "events")));
    }
    out
}

/// The error of a run, its category, its message and where it came from (spec 7.1).
fn error_text(out: &mut String, error: &Json) {
    push_line(out, &format!("error: {}: {}", error["category"].as_str().unwrap_or_default(), error["message"].as_str().unwrap_or_default()));
    let origin = &error["origin"];
    let mut at = |what: &Json, word: &str| {
        let mut text = format!("  {word} `{}`", what["function"].as_str().unwrap_or_default());
        if let Some(node) = what["node"].as_str() {
            let _ = write!(text, ", node `{node}`");
        }
        if let (Some(file), Some(line)) = (what["file"].as_str(), what["line"].as_u64()) {
            let _ = write!(text, " ({file}:{line})");
        }
        push_line(out, &text);
    };
    if origin.is_object() {
        at(origin, "from");
        for caller in origin["callers"].as_array().map(Vec::as_slice).unwrap_or_default() {
            at(caller, "called by");
        }
    }
}

/// Where each loop was, outermost first; iterations counted from 1, as messages count them.
fn loops_text(out: &mut String, loops: &Json) {
    for at in loops.as_array().map(Vec::as_slice).unwrap_or_default() {
        push_line(
            out,
            &format!(
                "  in loop `{}` of `{}`, iteration {}",
                at["node"].as_str().unwrap_or_default(),
                at["function"].as_str().unwrap_or_default(),
                at["iteration"].as_u64().unwrap_or(0) + 1
            ),
        );
    }
}
