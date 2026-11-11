//! What `run.start` judges before anything runs (`docs/cli.md`, 6.1; `docs/commands.md`, 6.4): the parameters, the folder
//! and its check, the function, the arguments, the panel settings and the simulation file. What the command line calls
//! wrong usage is a command error; what needs the program to be judged is a diagnostic of a `refused` run, and every one
//! is found, so that one report gives every problem.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use labcraft_check::Checked;
use labcraft_ir::{ElementId, Program, Refused, lower_for_run};
use labcraft_runtime::native::NativeLibrary;
use labcraft_runtime::native::Simulation;
use labcraft_runtime::run::{PanelSetting, SetBy, Settings};
use labcraft_runtime::simulation;
use labcraft_runtime::{Env, Value, constant};
use labcraft_types::{Interner, LibraryId, Span, TyId};
use serde_json::{Value as Json, json};

use super::convert::{Converter, Problem, Types, duration_text};
use super::json_text::{self, Bad};
use super::parts::CheckPart;
use crate::error::{CommandError, did_you_mean};
use crate::files::bytes_of;
use crate::folders::check_folder_for_run;
use crate::params::Params;
use crate::paths::{self, Place, Wanted};
use crate::session::Session;
use crate::walk::{shown_of, slashed};
use labcraft_runtime::report::Part as _;

/// `wait` of a run: 30 s by default, at most 300 s *(proposed)*.
pub(crate) const WAIT_MAX_NS: i64 = 300_000_000_000;
/// A duration given to `max_real_time`: 10 min by default *(proposed)*.
const MAX_REAL_TIME_NS: u64 = 600_000_000_000;

/// Where the trace of a run goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TraceMode {
    File,
    Inline,
    None,
}

/// A native library of the run: its id in the table of types, the library, and its part of the simulation file.
pub(crate) struct NativeOf {
    pub library: LibraryId,
    pub native: Arc<dyn NativeLibrary>,
    pub simulation: Option<Simulation>,
}

/// A run ready to go: what it runs, with what, and what its report and its trace need.
pub(crate) struct Prepared {
    pub id: String,
    /// As `run.start` named it, from the folder: `check_voltage`, `dmm_driver.measure`.
    pub function: String,
    pub checked: Checked,
    pub names: Interner,
    pub program: Program,
    pub inputs: Vec<Value>,
    pub settings: Settings,
    pub natives: Vec<NativeOf>,
    pub check: CheckPart,
    /// The files the run reaches, with the SHA-256 of their bytes, for the first line of the trace.
    pub files: Vec<(String, String)>,
    pub trace: TraceMode,
    /// The file of the last trace of the function, from the root (`task::trace_path`).
    pub trace_file: String,
    /// How the trace file is shown in the report: from the root, or from the current folder of the command line.
    pub trace_shown: String,
    /// `--trace FILE`: the trace goes there instead (decision of the owner, 2026-11-11).
    pub trace_to: Option<Outside>,
    /// Whether to keep the text of each output (`CommandLine::texts`).
    pub texts: bool,
    /// `mode: "real_time"`.
    pub real_time: bool,
    /// `--max-nesting`, for the values of `run.set_element`.
    pub max_nesting: u32,
}

/// A file of the command line, outside the rules of paths of commands (commands.md 2): named from the current folder,
/// anywhere (cli.md 6).
#[derive(Clone, Debug)]
pub struct Outside {
    /// Absolute, or from the current folder of the process.
    pub path: PathBuf,
    /// As the command line shows it.
    pub shown: String,
}

/// What `labcraft-cli run` gives a run beyond the parameters of `run.start` (cli.md 6; decisions of the owner,
/// 2026-11-11): the folder it shows paths from, its files, and its values as text, which `engine` reads against their
/// types with the freedoms of cli.md 6.3, so that MCP keeps `args` in their exact JSON form.
#[derive(Clone, Debug, Default)]
pub struct CommandLine {
    /// The current folder, without links, which paths are shown from, as `check` shows them; the root otherwise.
    pub current: Option<PathBuf>,
    /// The folder as the command line names it, from the current folder: the root, walked and shown under that name, so
    /// that a folder that is not under the current one is shown as the user wrote it (`../bench/x.lcf`).
    pub folder: Option<PathBuf>,
    /// Whether the run keeps the text of each output, for the human form (cli.md 6.7).
    pub texts: bool,
    /// `--arg NAME=VALUE`, in the order given, each name once.
    pub args: Vec<(String, String)>,
    /// `--args FILE`.
    pub args_file: Option<Outside>,
    /// `--sim FILE`, in place of the parameter `sim`.
    pub sim: Option<Outside>,
    /// `--set ELEMENT=VALUE@TIME`: the element, the value as text, the time as text.
    pub set: Vec<(String, String, String)>,
    /// `--trace FILE`.
    pub trace: Option<Outside>,
}

/// A value given to a run: in its JSON form (`args`, `set`, a key of `--args`), or as the text of the command line.
#[derive(Clone, Debug)]
pub(crate) enum Supplied {
    Json(Json),
    Text(String),
}

/// An argument, and where a problem with its value is shown: the file of `--args` at its key, or else the signature.
struct Arg {
    name: String,
    value: Supplied,
    at: Option<(String, Span)>,
}

impl Converter<'_> {
    fn supplied(&self, value: &Supplied, ty: TyId) -> Result<Value, Problem> {
        match value {
            Supplied::Json(json) => self.value(json, ty),
            Supplied::Text(text) => self.text_value(text, ty),
        }
    }
}

/// What `run.start` finds before it runs: a run, or the report of a refused one.
pub(crate) enum Ready {
    Run(Box<Prepared>),
    Refused(Json),
}

/// The parameters of `run.start` that need no program: wrong usage is a command error (commands.md 6.4).
pub(crate) struct Given {
    pub function: String,
    pub folder: String,
    pub sim: Option<String>,
    pub args: serde_json::Map<String, Json>,
    pub set: Vec<(String, Supplied, i64)>,
    pub settings: Settings,
    /// `mode: "real_time"` (spec 8): the limits of iterations and nodes have no default there.
    pub real_time: bool,
    pub deny_warnings: bool,
    pub trace: TraceMode,
    /// How long `run.start` waits, in nanoseconds.
    pub wait: i64,
    pub line: CommandLine,
}

fn invalid(parameter: &str, message: impl Into<String>) -> CommandError {
    CommandError::of("params.invalid", parameter, message)
}

/// A duration of a parameter: `"30 s"`, `"500 ms"`, finite and at least 0.
pub(crate) fn duration_param(params: &Params, name: &str, max_nesting: u32) -> Result<Option<i64>, CommandError> {
    let Some(text) = params.str(name) else { return Ok(None) };
    match duration_text(text, max_nesting) {
        Ok(ns) if ns >= 0 => Ok(Some(ns)),
        Ok(_) => Err(invalid(name, format!("`{text}` is below 0: a duration of `{name}` is 0 or more"))),
        Err(problem) => Err(invalid(name, format!("`{text}` is no duration, `\"30 s\"` or `\"500 ms\"`: {}", problem.message))),
    }
}

/// The wait of `run.start` and `run.wait`, at most 300 s.
pub(crate) fn wait_param(params: &Params, max_nesting: u32) -> Result<i64, CommandError> {
    let wait = duration_param(params, "wait", max_nesting)?.unwrap_or(30_000_000_000);
    if wait > WAIT_MAX_NS {
        return Err(invalid("wait", "`wait` is at most `300 s`: follow a longer run with `run.wait`"));
    }
    Ok(wait)
}

/// Names joined by `.`, each a name of the language (spec 14.1): `check_voltage`, `dmm_driver.measure`.
fn is_function_path(text: &str) -> bool {
    !text.is_empty() && text.split('.').all(labcraft_format::is_name)
}

/// Counts the bytes that JSON writes, and stops past a bound.
struct Counted {
    bytes: u64,
    max: u64,
}

impl std::io::Write for Counted {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.bytes = self.bytes.saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
        if self.bytes > self.max {
            return Err(std::io::Error::other("past the bound"));
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// `args` and `set` hold at most `max` bytes each, written as JSON: the bound of a file of `--args` (cli.md 6.3,
/// `--max-file-size`), so that what an MCP request gives costs no more to judge than what the command line reads.
fn bounded(params: &Params, name: &str, max: u64) -> Result<(), CommandError> {
    let Some(value) = params.get(name) else { return Ok(()) };
    if serde_json::to_writer(Counted { bytes: 0, max }, value).is_err() {
        return Err(invalid(
            name,
            format!("`{name}` holds more than {max} bytes written as JSON, the bound of a file of `--args` (--max-file-size)"),
        ));
    }
    Ok(())
}

/// Judges what can be judged without the program.
pub(crate) fn given(params: &Params, max_nesting: u32, max_file_size: u64) -> Result<Given, CommandError> {
    bounded(params, "args", max_file_size)?;
    bounded(params, "set", max_file_size)?;
    let function = params.text("function")?.to_owned();
    if !is_function_path(&function) {
        return Err(invalid("function", format!("`{function}` is not names joined by `.`: `check_voltage`, `dmm_driver.measure`")));
    }
    let real_time = params.str("mode") == Some("real_time");
    // What only deterministic mode has (cli.md 6.2): wrong usage on the command line, `params.invalid` here.
    for name in ["schedule_seed", "max_real_time"] {
        if real_time && params.get(name).is_some_and(|value| !value.is_null()) {
            let message = match name {
                "schedule_seed" => "`schedule_seed` orders a deterministic run: a run in real time takes its nodes as they are ready",
                _ => "`max_real_time` bounds a deterministic run: in real time, `timeout` counts real time already",
            };
            return Err(invalid(name, message));
        }
    }
    let args = match params.get("args") {
        Some(Json::Object(args)) => args.clone(),
        _ => serde_json::Map::new(),
    };
    if let Some(name) = args.keys().find(|name| !labcraft_format::is_name(name)) {
        return Err(invalid("args", format!("`{name}` is not a name, so no input has it (spec 14.1)")));
    }
    let mut set = Vec::new();
    for setting in params.get("set").and_then(Json::as_array).map(Vec::as_slice).unwrap_or_default() {
        let element = setting.get("element").and_then(Json::as_str).unwrap_or_default();
        if !labcraft_format::is_name(element) {
            return Err(invalid("set", format!("`{element}` is not a name, so no element has it (spec 14.1)")));
        }
        let at_text = setting.get("at").and_then(Json::as_str).unwrap_or_default();
        let at = match duration_text(at_text, max_nesting) {
            Ok(at) if at >= 0 => at,
            Ok(_) => return Err(invalid("set", format!("`{at_text}` is below 0: a setting is at a time of the run, from its start"))),
            Err(problem) => return Err(invalid("set", format!("`{at_text}` is no time of the run, `\"5 s\"`: {}", problem.message))),
        };
        set.push((element.to_owned(), Supplied::Json(setting.get("value").cloned().unwrap_or(Json::Null)), at));
    }
    let schedule_seed = match params.get("schedule_seed") {
        None | Some(Json::Null) => None,
        Some(Json::Number(number)) => Some(number.as_u64().ok_or_else(|| invalid("schedule_seed", "a seed is an integer from 0 to 2^64-1"))?),
        Some(Json::String(digits)) if !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()) => {
            Some(digits.parse().map_err(|_| invalid("schedule_seed", "a seed is an integer from 0 to 2^64-1"))?)
        }
        Some(_) => return Err(invalid("schedule_seed", "a seed is an integer, or a string of digits beyond 2^53-1")),
    };
    let mut settings = Settings { schedule_seed, ..Settings::default() };
    // In real time a loop runs as long as the program says, unless the run is given a limit (spec 8.2).
    if real_time {
        settings.max_iterations = None;
        settings.max_nodes = None;
        settings.max_made = None;
    }
    settings.timeout = match duration_param(params, "timeout", max_nesting)? {
        Some(0) => return Err(invalid("timeout", "a timeout is above 0")),
        other => other,
    };
    for (name, slot) in
        [("max_iterations", &mut settings.max_iterations), ("max_nodes", &mut settings.max_nodes), ("max_made", &mut settings.max_made)]
    {
        if params.is_null(name) {
            *slot = None;
        } else if let Some(value) = params.u64(name) {
            *slot = Some(value);
        }
    }
    if let Some(depth) = params.u64("max_call_depth") {
        settings.max_call_depth = u32::try_from(depth).map_err(|_| invalid("max_call_depth", "at most 4096"))?;
    }
    if let Some(memory) = params.u64("max_memory") {
        settings.max_memory = memory;
    }
    settings.max_real_time = match duration_param(params, "max_real_time", max_nesting)? {
        Some(0) => return Err(invalid("max_real_time", "`max_real_time` is above 0")),
        Some(ns) => Some(u64::try_from(ns).unwrap_or(MAX_REAL_TIME_NS)),
        None if real_time => None,
        None => Some(MAX_REAL_TIME_NS),
    };
    let trace = match params.str("trace") {
        Some("inline") => TraceMode::Inline,
        Some("none") => TraceMode::None,
        _ => TraceMode::File,
    };
    Ok(Given {
        function,
        folder: params.str("folder").unwrap_or(".").to_owned(),
        sim: params.str("sim").map(str::to_owned),
        args,
        set,
        settings,
        real_time,
        deny_warnings: params.bool("deny_warnings"),
        trace,
        wait: wait_param(params, max_nesting)?,
        line: CommandLine::default(),
    })
}

/// Joins what the command line gives to the parameters judged: its settings come after those of `set`.
pub(crate) fn with_line(mut given: Given, line: CommandLine, max_nesting: u32) -> Result<Given, CommandError> {
    for (element, value, at_text) in &line.set {
        if !labcraft_format::is_name(element) {
            return Err(invalid("set", format!("`{element}` is not a name, so no element has it (spec 14.1)")));
        }
        let at = match duration_text(at_text, max_nesting) {
            Ok(at) if at >= 0 => at,
            Ok(_) => return Err(invalid("set", format!("`{at_text}` is below 0: a setting is at a time of the run, from its start"))),
            Err(problem) => return Err(invalid("set", format!("`{at_text}` is no time of the run, `5s`: {}", problem.message))),
        };
        given.set.push((element.clone(), Supplied::Text(value.clone()), at));
    }
    if let Some(name) = line.args.iter().map(|(name, _)| name).find(|name| !labcraft_format::is_name(name)) {
        return Err(invalid("args", format!("`{name}` is not a name, so no input has it (spec 14.1)")));
    }
    given.line = line;
    Ok(given)
}

/// A diagnostic of a run (`LC81`, `LC0001` for a path that does not read, those of the simulation file), as JSON.
fn diagnostic(code: &str, message: &str, file: &str, span: Option<Span>, fix: Option<String>) -> Json {
    let mut diagnostic = json!({ "code": code, "severity": "error", "message": labcraft_types::cut_text(message), "file": file });
    if let Some(object) = diagnostic.as_object_mut() {
        if let Some(span) = span {
            object.insert(
                "span".to_owned(),
                json!({ "start": { "line": span.start.line, "column": span.start.column }, "end": { "line": span.end.line, "column": span.end.column } }),
            );
        }
        if let Some(fix) = fix {
            object.insert("fixes".to_owned(), json!([{ "title": fix }]));
        }
    }
    diagnostic
}

/// A file the run reads, as a diagnostic shows it, or the error that a command gives for it.
enum Read {
    Text(String),
    Problem(Json),
}

/// The simulation file, read as the command line reads `--sim` (cli.md 6.2): a file that does not read is a diagnostic.
fn read_sim(session: &Session, given: &str, max_nesting: u32) -> Result<Read, CommandError> {
    let place = paths::judge(&*session.ws, &session.root, given, "sim", Wanted::Simulation, true)?;
    let shown = match &place {
        Place::Project { shown, .. } => shown.clone(),
        Place::Shipped(shown) => shown.clone(),
    };
    let _ = max_nesting;
    match bytes_of(session, &place, "sim") {
        Ok((bytes, _)) => match String::from_utf8(bytes.into_owned()) {
            Ok(text) => Ok(Read::Text(text)),
            Err(_) => Ok(Read::Problem(diagnostic(
                simulation::NOT_TOML,
                "the simulation file is not UTF-8, so it does not read as TOML",
                &shown,
                None,
                None,
            ))),
        },
        Err(error) => {
            let code = match error.code {
                "file.too_large" => "LC0006",
                "path.not_found" | "file.unreadable" | "path.invalid" => "LC0001",
                _ => return Err(error),
            };
            Ok(Read::Problem(diagnostic(code, &error.message, &shown, None, None)))
        }
    }
}

/// A file of the command line, read as `--sim` and `--args` are (cli.md 6.2): one that does not read is a diagnostic.
fn read_outside(session: &Session, outside: &Outside, what: &str) -> Read {
    let exists = session.ws.link_metadata(&outside.path).is_ok();
    let place = Place::Project { path: outside.path.clone(), shown: outside.shown.clone(), exists };
    match bytes_of(session, &place, what) {
        Ok((bytes, _)) => match String::from_utf8(bytes.into_owned()) {
            Ok(text) => Read::Text(text),
            Err(_) => {
                let code = if what == "sim" { simulation::NOT_TOML } else { "LC8104" };
                Read::Problem(diagnostic(code, &format!("`{}` is not UTF-8", outside.shown), &outside.shown, None, None))
            }
        },
        Err(error) => {
            let code = if error.code == "file.too_large" { "LC0006" } else { "LC0001" };
            Read::Problem(diagnostic(code, &error.message, &outside.shown, None, None))
        }
    }
}

/// The arguments given to a run, each once, in the order first given, and whether the file of `--args` did not read, in
/// which case no input is said to have no argument: the file may have given it.
struct Arguments {
    list: Vec<Arg>,
    at: HashMap<String, usize>,
    file_failed: bool,
    /// The arguments that name no input, said in full up to `--max-report`, counted after (`LC8102`): a file of `--args`
    /// can hold hundreds of thousands of keys.
    unknown_said: std::cell::Cell<usize>,
    unknown_counted: std::cell::Cell<u64>,
    max_report: usize,
}

impl Arguments {
    /// Whether one more argument that names no input is said in full; counted otherwise.
    fn say_unknown(&self) -> bool {
        if self.unknown_said.get() < self.max_report {
            self.unknown_said.set(self.unknown_said.get() + 1);
            true
        } else {
            self.unknown_counted.set(self.unknown_counted.get() + 1);
            false
        }
    }
}

impl Arguments {
    fn put(&mut self, arg: Arg) {
        match self.at.get(&arg.name) {
            Some(index) => {
                if let Some(known) = self.list.get_mut(*index) {
                    *known = arg;
                }
            }
            None => {
                self.at.insert(arg.name.clone(), self.list.len());
                self.list.push(arg);
            }
        }
    }

    fn get(&self, name: &str) -> Option<&Arg> {
        self.at.get(name).and_then(|index| self.list.get(*index))
    }
}

/// The arguments of a run: `args`, then the keys of `--args`, then `--arg`, each replacing the one before it for its
/// input; the problems of the file of `--args` go to `problems`, in the file.
fn arguments(session: &Session, given: &Given, max_nesting: u32, problems: &mut Vec<Json>) -> Arguments {
    let mut arguments = Arguments {
        list: Vec::new(),
        at: HashMap::new(),
        file_failed: false,
        unknown_said: std::cell::Cell::new(0),
        unknown_counted: std::cell::Cell::new(0),
        max_report: session.config.limits.max_report,
    };
    for (name, value) in &given.args {
        arguments.put(Arg { name: name.clone(), value: Supplied::Json(value.clone()), at: None });
    }
    if let Some(file) = &given.line.args_file {
        let before = problems.len();
        match read_outside(session, file, "args") {
            Read::Problem(problem) => problems.push(problem),
            Read::Text(text) => match json_text::parse(&text, max_nesting) {
                Ok(parsed) => match parsed.value {
                    Json::Object(mut values) => {
                        // Placed once: a position is then a binary search (`Places`), so a file of many keys is linear.
                        let places = labcraft_runtime::simulation::Places::new(&text);
                        for (name, range) in parsed.keys {
                            let span = places.span(range);
                            if let Some(value) = values.remove(&name) {
                                arguments.put(Arg { name, value: Supplied::Json(value), at: Some((file.shown.clone(), span)) });
                            }
                        }
                    }
                    _ => problems.push(diagnostic(
                        "LC8104",
                        "the file of `--args` is not a JSON object of input names, `{\"resource\": \"SIM::DMM0\"}`",
                        &file.shown,
                        None,
                        None,
                    )),
                },
                Err(bad) => {
                    let place = |range| Some(labcraft_runtime::simulation::span_of(&text, range));
                    let (message, span) = match bad {
                        Bad::Syntax(at) => ("the file of `--args` does not read as JSON".to_owned(), place(at..at)),
                        Bad::TooDeep => (format!("the file of `--args` nests deeper than {max_nesting} levels (--max-nesting)"), None),
                        Bad::Twice(key, range) => (format!("the file of `--args` gives the key `{key}` twice"), place(range)),
                        Bad::OutOfRange(range) => {
                            let number = text.get(range.clone()).unwrap_or_default();
                            let message =
                                format!("the number `{}` of the file of `--args` is out of the range of a number", labcraft_types::cut_text(number));
                            problems.push(diagnostic("LC8106", &message, &file.shown, place(range), None));
                            (String::new(), None)
                        }
                    };
                    if !message.is_empty() {
                        problems.push(diagnostic("LC8104", &message, &file.shown, span, None));
                    }
                }
            },
        }
        arguments.file_failed = problems.len() > before;
    }
    for (name, text) in &given.line.args {
        arguments.put(Arg { name: name.clone(), value: Supplied::Text(text.clone()), at: None });
    }
    arguments
}

/// The diagnostics of a run in the order of their places (cli.md 4.2): by file, then by position, one with no position
/// first in its file, then by code and message.
fn by_place(mut problems: Vec<Json>) -> Vec<Json> {
    let key = |problem: &Json| {
        let at = |field: &str| problem.pointer(&format!("/span/start/{field}")).and_then(Json::as_u64).unwrap_or(0);
        let text = |field: &str| problem.get(field).and_then(Json::as_str).unwrap_or_default().to_owned();
        (text("file"), at("line"), at("column"), text("code"), text("message"))
    };
    problems.sort_by_cached_key(key);
    problems
}

/// What a type declared in a `.lct` file stands for, in a `Checked`: `None` for an opaque one.
fn definitions_of(checked: &Checked) -> HashMap<TyId, Option<TyId>> {
    checked
        .libraries
        .iter()
        .flat_map(|library| library.types.values())
        .map(|declared| (declared.ty, (!declared.opaque).then_some(declared.definition)))
        .collect()
}

/// Judges the folder, the function, the arguments, the settings and the simulation file.
pub(crate) fn prepare(session: &Session, given: &Given, id: &str) -> Result<Ready, CommandError> {
    let limits = &session.config.limits;
    let max_nesting = limits.max_nesting();
    let mut problems: Vec<Json> = Vec::new();

    // Paths are shown from the root, or from the current folder of the command line.
    let current: PathBuf = given.line.current.clone().unwrap_or_else(|| session.root.clone());
    let from_current = |path: &Path| shown_of(&*session.ws, path, session.ws.canonicalize(path).ok().as_deref(), Some(&current));
    // The folder, a path of the project.
    let folder = match paths::judge(&*session.ws, &session.root, &given.folder, "folder", Wanted::Folder, false)? {
        Place::Project { path, shown, exists } => {
            let shown = match (&given.line.folder, given.line.current.is_some()) {
                (Some(typed), _) => from_current(typed),
                (None, true) => from_current(&path),
                (None, false) => shown,
            };
            if !exists {
                problems.push(diagnostic("LC0001", &format!("cannot read this path: `{shown}` does not exist"), &shown, None, None));
                None
            } else {
                Some((path, shown))
            }
        }
        Place::Shipped(shown) => {
            return Err(CommandError::of(
                "path.outside_project",
                "folder",
                format!("`{shown}` is shipped with LabCraft: run a function of the project"),
            ));
        }
    };
    // The simulation file, read now, judged by its libraries once the run is lowered.
    let sim = match (&given.line.sim, &given.sim) {
        (Some(outside), _) => match read_outside(session, outside, "sim") {
            Read::Text(text) => Some((text, outside.shown.clone())),
            Read::Problem(problem) => {
                problems.push(problem);
                None
            }
        },
        (None, Some(path)) => match read_sim(session, path, max_nesting)? {
            Read::Text(text) => Some((text, sim_shown(session, path))),
            Read::Problem(problem) => {
                problems.push(problem);
                None
            }
        },
        (None, None) => None,
    };
    // Each library judges its part of the simulation file now, so that its problems join those of the arguments.
    let mut simulations: Vec<Option<Simulation>> = Vec::new();
    if let Some((text, shown)) = &sim {
        let all: Vec<&dyn NativeLibrary> = session.config.natives.iter().map(|native| &**native as &dyn NativeLibrary).collect();
        let judged = simulation::read(text, max_nesting).map_err(|problem| vec![problem]).and_then(|file| file.judge(&all));
        match judged {
            Ok(judged) => simulations = judged,
            Err(found) => {
                for problem in found {
                    problems.push(diagnostic(
                        problem.code,
                        &problem.message,
                        shown,
                        problem.span,
                        problem.fix.map(|fix| format!("did you mean `{fix}`?")),
                    ));
                }
            }
        }
    }
    let arguments = arguments(session, given, max_nesting, &mut problems);
    let Some((folder, folder_shown)) = folder else {
        return Ok(Ready::Refused(refused_in(
            id,
            &given.function,
            given.real_time,
            CheckPart::of_problems(by_place(problems)).with_unlisted(arguments.unknown_counted.get()),
        )));
    };
    let walked = given.line.folder.clone().unwrap_or_else(|| folder.clone());
    let run_checked = check_folder_for_run(&*session.ws, &session.config.shipped, &walked, &session.config.lib_paths, &current, limits);
    let mut report = run_checked.report;
    let Some((checked, mut names)) = run_checked.checked else {
        report.finish();
        let check =
            CheckPart::of_report(&report, run_checked.counts, given.deny_warnings, by_place(problems)).with_unlisted(arguments.unknown_counted.get());
        return Ok(Ready::Refused(refused_in(id, &given.function, given.real_time, check)));
    };

    // The function, named from the folder.
    let Some(start) = checked.function_named(&given.function, &names) else {
        let known: Vec<String> = checked
            .libraries
            .iter()
            .filter(|library| library.checked)
            .flat_map(|library| {
                library.functions.iter().filter(|(_, function)| function.file.is_some()).map(move |(name, _)| (library.name.clone(), *name))
            })
            .map(|(library, name)| if library.is_empty() { names.resolve(name).to_owned() } else { format!("{library}.{}", names.resolve(name)) })
            .collect();
        let known: Vec<&str> = known.iter().map(String::as_str).collect();
        let near = labcraft_runtime::simulation::nearest(&given.function, &known).map(|near| format!("did you mean `{near}`?"));
        let message = format!("`{}` is not a function of the library{}", given.function, did_you_mean(&given.function, &known));
        problems.insert(0, diagnostic("LC8101", &message, &folder_shown, None, near));
        report.finish();
        let check =
            CheckPart::of_report(&report, run_checked.counts, given.deny_warnings, by_place(problems)).with_unlisted(arguments.unknown_counted.get());
        return Ok(Ready::Refused(refused_in(id, &given.function, given.real_time, check)));
    };

    // What the run reaches: the diagnostics of the bodies of its dependencies join the report.
    let reach = checked.reach(start);
    let reached: Vec<&str> =
        reach.functions.iter().chain(&reach.types).filter_map(|file| checked.files.get(*file as usize)).map(|file| file.shown.as_str()).collect();
    for found in checked.bodies.diagnostics() {
        if reached.contains(&found.file.as_str()) {
            report.push(found.clone());
        }
    }
    report.finish();
    let refuses = reach.refuses(&checked, given.deny_warnings);

    let function_file = checked.files.get(start);
    let file_shown = function_file.map_or(folder_shown.as_str(), |file| file.shown.as_str()).to_owned();
    let file_shown = file_shown.as_str();
    let span_of = |origin| function_file.and_then(|file| file.source_map.span(origin));

    // A run that `check` refuses is not lowered: its arguments are judged against the signature that `check` found, so
    // that one report gives every problem (cli.md 6.1, step 2); its settings need the panel, which lowering settles.
    if refuses {
        problems.extend(judge_signature(&checked, &names, start, &arguments, &given.function, file_shown, max_nesting));
        let check =
            CheckPart::of_report(&report, run_checked.counts, given.deny_warnings, by_place(problems)).with_unlisted(arguments.unknown_counted.get());
        return Ok(Ready::Refused(refused_in(id, &given.function, given.real_time, check)));
    }

    // Lowered for the run: the bounds of the run as a whole (`LC8111`).
    let program = match lower_for_run(&checked, start, &mut names) {
        Ok(program) => program,
        Err(refusal) => {
            let message = match refusal {
                Refused::TooLarge { counted, limit, .. } => format!(
                    "the functions the run reaches hold more than {limit} terminals of calls or values passed into structures ({counted} counted), which a run does not take"
                ),
                Refused::Bug(error) => format!("internal error, which is a bug of the tool: the run could not be lowered: {error:?}"),
            };
            let code = if message.starts_with("internal") { "LC0099" } else { "LC8111" };
            problems.insert(0, diagnostic(code, &message, file_shown, None, None));
            problems.extend(judge_signature(&checked, &names, start, &arguments, &given.function, file_shown, max_nesting));
            let check = CheckPart::of_report(&report, run_checked.counts, given.deny_warnings, by_place(problems))
                .with_unlisted(arguments.unknown_counted.get());
            return Ok(Ready::Refused(refused_in(id, &given.function, given.real_time, check)));
        }
    };
    let env = Env { program: &program, names: &names };
    let Some(top) = program.functions.first() else { return Err(CommandError::internal("a program lowered with no function")) };
    let converter =
        Converter { types: Types { table: &program.types, definitions: &program.definitions, names: &names, viewer: top.library }, max_nesting };

    // The inputs, in the order of the signature, each problem at the name of its input; an unknown one at the first.
    let names_of: Vec<&str> = top.inputs.iter().map(|port| names.resolve(port.name)).collect();
    let signature = top.inputs.first().and_then(|port| span_of(port.origin));
    let known: std::collections::HashSet<&str> = names_of.iter().copied().collect();
    for arg in &arguments.list {
        let key = arg.name.as_str();
        if !known.contains(key) && arguments.say_unknown() {
            let near = labcraft_runtime::simulation::nearest(key, &names_of).map(|near| format!("did you mean `{near}`?"));
            let message = format!("`{key}` names no input of `{}`{}", given.function, did_you_mean(key, &names_of));
            let (file, span) = arg.at.as_ref().map_or((file_shown, signature), |(file, span)| (file.as_str(), Some(*span)));
            problems.push(diagnostic("LC8102", &message, file, span, near));
        }
    }
    let mut inputs = Vec::with_capacity(top.inputs.len());
    for (port, name) in top.inputs.iter().zip(&names_of) {
        let arg = arguments.get(name);
        let (file, span) = match arg.and_then(|arg| arg.at.as_ref()) {
            Some((file, span)) => (file.as_str(), Some(*span)),
            None => (file_shown, span_of(port.origin)),
        };
        let value = if converter.holds_opaque(port.ty) {
            Err(Problem { code: "LC8107", message: OPAQUE.to_owned() })
        } else {
            match arg {
                Some(arg) => converter.supplied(&arg.value, port.ty),
                None => match port.default {
                    Some(default) => constant(env, default).map_err(|fault| Problem { code: "LC8104", message: fault.message.clone() }),
                    // The file of `--args` that did not read may have given it; its problem refuses the run, so this
                    // place holder never runs.
                    None if arguments.file_failed => Ok(Value::Error(None)),
                    None => Err(Problem { code: "LC8103", message: "it has no default, and no argument gives it".to_owned() }),
                },
            }
        };
        match value {
            Ok(value) => inputs.push(value),
            Err(Problem { code, message }) => problems.push(diagnostic(code, &format!("the input `{name}`: {message}"), file, span, None)),
        }
    }

    // The settings of panel elements (cli.md 6.4).
    let mut settings = given.settings.clone();
    for (element, value, at) in &given.set {
        match judge_setting(top, &names, &converter, element, value, &given.function) {
            Ok((element, value)) => settings.panel.push(PanelSetting { element, value, at: *at, by: SetBy::Set }),
            Err(refused) => {
                problems.push(diagnostic(refused.problem.code, &refused.problem.message, file_shown, refused.origin.and_then(span_of), None))
            }
        }
    }

    if !problems.is_empty() {
        let check =
            CheckPart::of_report(&report, run_checked.counts, given.deny_warnings, by_place(problems)).with_unlisted(arguments.unknown_counted.get());
        return Ok(Ready::Refused(refused_in(id, &given.function, given.real_time, check)));
    }
    // The native libraries the program uses, each with its part of the simulation file.
    let used: Vec<(LibraryId, Arc<dyn NativeLibrary>)> = session
        .config
        .natives
        .iter()
        .filter_map(|native| checked.libraries.iter().find(|library| library.name == native.name()).map(|library| (library.id, Arc::clone(native))))
        .collect();
    let natives = used
        .into_iter()
        .map(|(library, native)| {
            let position = session.config.natives.iter().position(|given| Arc::ptr_eq(given, &native));
            let simulation = position.and_then(|position| simulations.get_mut(position)).and_then(Option::take);
            NativeOf { library, native, simulation }
        })
        .collect();

    // The files the run reaches, for the first line of the trace.
    let hashes: HashMap<&str, &str> = run_checked.hashes.iter().map(|(file, sha)| (file.as_str(), sha.as_str())).collect();
    let files = reached
        .iter()
        .filter_map(|shown| match hashes.get(shown) {
            Some(sha) => Some(((*shown).to_owned(), (*sha).to_owned())),
            None => session.config.shipped.file(shown).map(|bytes| ((*shown).to_owned(), labcraft_trace::sha256_hex(bytes))),
        })
        .collect();
    let check = CheckPart::of_report(&report, run_checked.counts, given.deny_warnings, Vec::new());
    let folder_from_root = slashed(folder.strip_prefix(&session.root).unwrap_or(&folder));
    let trace_file = super::task::trace_path(&folder_from_root, &given.function);
    let trace_shown = match (&given.line.trace, given.line.current.is_some()) {
        (Some(outside), _) => outside.shown.clone(),
        (None, true) => {
            let mut path = given.line.folder.clone().unwrap_or_else(|| session.root.clone());
            path.extend(trace_file.split('/'));
            shown_of(&*session.ws, &path, None, Some(&current))
        }
        (None, false) => trace_file.clone(),
    };
    Ok(Ready::Run(Box::new(Prepared {
        id: id.to_owned(),
        function: given.function.clone(),
        checked,
        names,
        program,
        inputs,
        settings,
        natives,
        check,
        files,
        trace: given.trace,
        trace_file,
        trace_shown,
        trace_to: given.line.trace.clone(),
        texts: given.line.texts,
        real_time: given.real_time,
        max_nesting,
    })))
}

/// Why an input whose type holds an opaque type or a function reference has no argument (`LC8107`).
const OPAQUE: &str = "its type holds an opaque type or a function reference, which an argument cannot give: a function that takes one is called by another function, which the run starts";

/// The arguments judged against the signature that `check` found, for a run that is not lowered: no span, since the
/// signature is not lowered either.
fn judge_signature(
    checked: &Checked,
    names: &Interner,
    start: usize,
    arguments: &Arguments,
    function: &str,
    file_shown: &str,
    max_nesting: u32,
) -> Vec<Json> {
    let mut problems = Vec::new();
    let Some(library) = checked.files.get(start).and_then(|file| checked.libraries.get(file.library as usize)) else { return problems };
    let Some(declared) = function.rsplit('.').next().and_then(|name| names.find(name)).and_then(|name| library.functions.get(&name)) else {
        return problems;
    };
    let definitions = definitions_of(checked);
    let converter = Converter { types: Types { table: &checked.types, definitions: &definitions, names, viewer: library.id }, max_nesting };
    let inputs: Vec<&str> = declared.params.iter().map(|param| names.resolve(param.name)).collect();
    let at = |arg: Option<&Arg>| match arg.and_then(|arg| arg.at.as_ref()) {
        Some((file, span)) => (file.clone(), Some(*span)),
        None => (file_shown.to_owned(), None),
    };
    let known: std::collections::HashSet<&str> = inputs.iter().copied().collect();
    for arg in &arguments.list {
        let key = arg.name.as_str();
        if !known.contains(key) && arguments.say_unknown() {
            let near = labcraft_runtime::simulation::nearest(key, &inputs).map(|near| format!("did you mean `{near}`?"));
            let message = format!("`{key}` names no input of `{function}`{}", did_you_mean(key, &inputs));
            let (file, span) = at(Some(arg));
            problems.push(diagnostic("LC8102", &message, &file, span, near));
        }
    }
    for (param, name) in declared.params.iter().zip(&inputs) {
        let arg = arguments.get(name);
        let problem = if converter.holds_opaque(param.ty) {
            Some(Problem { code: "LC8107", message: OPAQUE.to_owned() })
        } else {
            match arg {
                Some(arg) => converter.supplied(&arg.value, param.ty).err(),
                None if !param.has_default && !arguments.file_failed => {
                    Some(Problem { code: "LC8103", message: "it has no default, and no argument gives it".to_owned() })
                }
                None => None,
            }
        };
        if let Some(Problem { code, message }) = problem {
            let (file, span) = at(arg);
            problems.push(diagnostic(code, &format!("the input `{name}`: {message}"), &file, span, None));
        }
    }
    problems
}

/// How a simulation file is shown: from the root, or as a shipped file.
fn sim_shown(session: &Session, given: &str) -> String {
    match paths::judge(&*session.ws, &session.root, given, "sim", Wanted::Simulation, true) {
        Ok(Place::Project { shown, .. } | Place::Shipped(shown)) => shown,
        Err(_) => slashed(std::path::Path::new(given)),
    }
}

/// Why a setting of a panel element is refused: its code and message, and the element when it names one.
pub(crate) struct SettingRefused {
    pub problem: Problem,
    pub origin: Option<labcraft_graph::Id>,
}

/// A setting of a panel element of the top-level function, by `--set`, `set` or `run.set_element` (cli.md 6.4): its element
/// and its value in the type of the element, or why not (`LC8108`, or the code of a value that does not convert).
pub(crate) fn judge_setting(
    top: &labcraft_ir::Function,
    names: &Interner,
    converter: &Converter<'_>,
    element: &str,
    value: &Supplied,
    function: &str,
) -> Result<(ElementId, Value), SettingRefused> {
    let refused = |code: &'static str, message: String, origin| Err(SettingRefused { problem: Problem { code, message }, origin });
    let found = top.elements.iter().enumerate().find(|(_, declared)| names.resolve(declared.name) == element);
    let Some((index, declared)) = found else {
        let known: Vec<&str> = top.elements.iter().filter(|declared| declared.port.is_none()).map(|declared| names.resolve(declared.name)).collect();
        return refused("LC8108", format!("`{element}` names no panel element of `{function}`{}", did_you_mean(element, &known)), None);
    };
    let origin = Some(declared.origin);
    if declared.port.is_some() {
        let message =
            format!("`{element}` is the element of an input or an output: an input is given by `args`, an output is written by the function");
        return refused("LC8108", message, origin);
    }
    let Some(ty) = declared.ty else { return refused("LC8108", format!("the type of `{element}` is not settled, so it cannot be set"), origin) };
    let off = match value {
        Supplied::Json(json) => *json == Json::Bool(false),
        Supplied::Text(text) => text.trim() == "false",
    };
    if names.resolve(declared.kind) == "button" && off {
        let message = format!("`{element}` is a button: setting it to `false` is not taken in M0, the modes of a button being open (spec, Q-30)");
        return refused("LC8108", message, origin);
    }
    let element_id = ElementId(
        u32::try_from(index)
            .map_err(|_| SettingRefused { problem: Problem { code: "LC0099", message: "an element beyond u32".to_owned() }, origin: None })?,
    );
    match converter.supplied(value, ty) {
        Ok(value) => Ok((element_id, value)),
        Err(Problem { code, message }) => refused(code, format!("the element `{element}`: {message}"), origin),
    }
}

/// The report of a refused run (commands.md 7), in its mode.
pub(crate) fn refused_in(id: &str, function: &str, real_time: bool, check: CheckPart) -> Json {
    let text = labcraft_runtime::report::refused(id, function, 0, real_time, &check.within(labcraft_runtime::report::RESULT_MAX / 2).0);
    serde_json::from_str(&text)
        .unwrap_or_else(|error| json!({ "schema": "labcraft-run/1", "status": "refused", "run": id, "error": error.to_string() }))
}
