//! What `run.start` judges before anything runs (`docs/cli.md`, 6.1; `docs/commands.md`, 6.4): the parameters, the folder
//! and its check, the function, the arguments, the panel settings and the simulation file. What the command line calls
//! wrong usage is a command error; what needs the program to be judged is a diagnostic of a `refused` run, and every one
//! is found, so that one report gives every problem.

use std::collections::HashMap;
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
use super::parts::CheckPart;
use crate::error::{CommandError, did_you_mean};
use crate::files::bytes_of;
use crate::folders::check_folder_for_run;
use crate::params::Params;
use crate::paths::{self, Place, Wanted};
use crate::session::Session;
use crate::walk::slashed;
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
    pub set: Vec<(String, Json, i64)>,
    pub settings: Settings,
    pub deny_warnings: bool,
    pub trace: TraceMode,
    /// How long `run.start` waits, in nanoseconds.
    pub wait: i64,
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

/// Judges what can be judged without the program.
pub(crate) fn given(params: &Params, max_nesting: u32) -> Result<Given, CommandError> {
    let function = params.text("function")?.to_owned();
    if !is_function_path(&function) {
        return Err(invalid("function", format!("`{function}` is not names joined by `.`: `check_voltage`, `dmm_driver.measure`")));
    }
    let real_time = params.str("mode") == Some("real_time");
    if real_time {
        return Err(CommandError::of(
            "command.not_enabled",
            "mode",
            "a run in real time is planned, and comes with M0b, step 8 (real time): give `mode: \"deterministic\"`",
        ));
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
        set.push((element.to_owned(), setting.get("value").cloned().unwrap_or(Json::Null), at));
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
    settings.timeout = match duration_param(params, "timeout", max_nesting)? {
        Some(0) => return Err(invalid("timeout", "a timeout is above 0")),
        other => other,
    };
    for (name, slot) in [("max_iterations", &mut settings.max_iterations), ("max_nodes", &mut settings.max_nodes)] {
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
        deny_warnings: params.bool("deny_warnings"),
        trace,
        wait: wait_param(params, max_nesting)?,
    })
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

    // The folder, a path of the project.
    let folder = match paths::judge(&*session.ws, &session.root, &given.folder, "folder", Wanted::Folder, false)? {
        Place::Project { path, shown, exists } => {
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
    let sim = match &given.sim {
        Some(path) => match read_sim(session, path, max_nesting)? {
            Read::Text(text) => Some((text, sim_shown(session, path))),
            Read::Problem(problem) => {
                problems.push(problem);
                None
            }
        },
        None => None,
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
    let Some((folder, folder_shown)) = folder else {
        return Ok(Ready::Refused(refused(id, &given.function, CheckPart::of_problems(problems))));
    };
    let run_checked = check_folder_for_run(&*session.ws, &session.config.shipped, &folder, &session.config.lib_paths, &session.root, limits);
    let mut report = run_checked.report;
    let Some((checked, mut names)) = run_checked.checked else {
        report.finish();
        let check = CheckPart::of_report(&report, run_checked.counts, given.deny_warnings, problems);
        return Ok(Ready::Refused(refused(id, &given.function, check)));
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
        let check = CheckPart::of_report(&report, run_checked.counts, given.deny_warnings, problems);
        return Ok(Ready::Refused(refused(id, &given.function, check)));
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
        problems.extend(judge_signature(&checked, &names, start, given, file_shown, max_nesting));
        let check = CheckPart::of_report(&report, run_checked.counts, given.deny_warnings, problems);
        return Ok(Ready::Refused(refused(id, &given.function, check)));
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
            problems.extend(judge_signature(&checked, &names, start, given, file_shown, max_nesting));
            let check = CheckPart::of_report(&report, run_checked.counts, given.deny_warnings, problems);
            return Ok(Ready::Refused(refused(id, &given.function, check)));
        }
    };
    let env = Env { program: &program, names: &names };
    let Some(top) = program.functions.first() else { return Err(CommandError::internal("a program lowered with no function")) };
    let converter =
        Converter { types: Types { table: &program.types, definitions: &program.definitions, names: &names, viewer: top.library }, max_nesting };

    // The inputs, in the order of the signature, each problem at the name of its input; an unknown one at the first.
    let names_of: Vec<&str> = top.inputs.iter().map(|port| names.resolve(port.name)).collect();
    let signature = top.inputs.first().and_then(|port| span_of(port.origin));
    for key in given.args.keys() {
        if !names_of.contains(&key.as_str()) {
            let near = labcraft_runtime::simulation::nearest(key, &names_of).map(|near| format!("did you mean `{near}`?"));
            let message = format!("`{key}` names no input of `{}`{}", given.function, did_you_mean(key, &names_of));
            problems.push(diagnostic("LC8102", &message, file_shown, signature, near));
        }
    }
    let mut inputs = Vec::with_capacity(top.inputs.len());
    for (port, name) in top.inputs.iter().zip(&names_of) {
        let span = span_of(port.origin);
        let value = if converter.holds_opaque(port.ty) {
            Err(Problem { code: "LC8107", message: OPAQUE.to_owned() })
        } else {
            match given.args.get(*name) {
                Some(value) => converter.value(value, port.ty),
                None => match port.default {
                    Some(default) => constant(env, default).map_err(|fault| Problem { code: "LC8104", message: fault.message.clone() }),
                    None => Err(Problem { code: "LC8103", message: "it has no default, and no argument gives it".to_owned() }),
                },
            }
        };
        match value {
            Ok(value) => inputs.push(value),
            Err(Problem { code, message }) => problems.push(diagnostic(code, &format!("the input `{name}`: {message}"), file_shown, span, None)),
        }
    }

    // The settings of panel elements (cli.md 6.4).
    let mut settings = given.settings.clone();
    for (element, value, at) in &given.set {
        let found = top.elements.iter().enumerate().find(|(_, declared)| names.resolve(declared.name) == element);
        let Some((index, declared)) = found else {
            let known: Vec<&str> =
                top.elements.iter().filter(|declared| declared.port.is_none()).map(|declared| names.resolve(declared.name)).collect();
            let message = format!("`{element}` names no panel element of `{}`{}", given.function, did_you_mean(element, &known));
            problems.push(diagnostic("LC8108", &message, file_shown, None, None));
            continue;
        };
        let span = span_of(declared.origin);
        if declared.port.is_some() {
            let message =
                format!("`{element}` is the element of an input or an output: an input is given by `args`, an output is written by the function");
            problems.push(diagnostic("LC8108", &message, file_shown, span, None));
            continue;
        }
        let Some(ty) = declared.ty else {
            problems.push(diagnostic("LC8108", &format!("the type of `{element}` is not settled, so it cannot be set"), file_shown, span, None));
            continue;
        };
        if names.resolve(declared.kind) == "button" && *value == Json::Bool(false) {
            let message = format!("`{element}` is a button: setting it to `false` is not taken in M0, the modes of a button being open (spec, Q-30)");
            problems.push(diagnostic("LC8108", &message, file_shown, span, None));
            continue;
        }
        match converter.value(value, ty) {
            Ok(value) => {
                let element = ElementId(u32::try_from(index).map_err(|_| CommandError::internal("an element beyond u32"))?);
                settings.panel.push(PanelSetting { element, value, at: *at, by: SetBy::Set });
            }
            Err(Problem { code, message }) => problems.push(diagnostic(code, &format!("the element `{element}`: {message}"), file_shown, span, None)),
        }
    }

    if !problems.is_empty() {
        let check = CheckPart::of_report(&report, run_checked.counts, given.deny_warnings, problems);
        return Ok(Ready::Refused(refused(id, &given.function, check)));
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
    let trace_file = super::task::trace_path(&folder_shown, &given.function);
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
    })))
}

/// Why an input whose type holds an opaque type or a function reference has no argument (`LC8107`).
const OPAQUE: &str = "its type holds an opaque type or a function reference, which an argument cannot give: a function that takes one is called by another function, which the run starts";

/// The arguments judged against the signature that `check` found, for a run that is not lowered: no span, since the
/// signature is not lowered either.
fn judge_signature(checked: &Checked, names: &Interner, start: usize, given: &Given, file_shown: &str, max_nesting: u32) -> Vec<Json> {
    let mut problems = Vec::new();
    let Some(library) = checked.files.get(start).and_then(|file| checked.libraries.get(file.library as usize)) else { return problems };
    let Some(declared) = given.function.rsplit('.').next().and_then(|name| names.find(name)).and_then(|name| library.functions.get(&name)) else {
        return problems;
    };
    let definitions = definitions_of(checked);
    let converter = Converter { types: Types { table: &checked.types, definitions: &definitions, names, viewer: library.id }, max_nesting };
    let inputs: Vec<&str> = declared.params.iter().map(|param| names.resolve(param.name)).collect();
    for key in given.args.keys() {
        if !inputs.contains(&key.as_str()) {
            let near = labcraft_runtime::simulation::nearest(key, &inputs).map(|near| format!("did you mean `{near}`?"));
            let message = format!("`{key}` names no input of `{}`{}", given.function, did_you_mean(key, &inputs));
            problems.push(diagnostic("LC8102", &message, file_shown, None, near));
        }
    }
    for (param, name) in declared.params.iter().zip(&inputs) {
        let problem = if converter.holds_opaque(param.ty) {
            Some(Problem { code: "LC8107", message: OPAQUE.to_owned() })
        } else {
            match given.args.get(*name) {
                Some(value) => converter.value(value, param.ty).err(),
                None if !param.has_default => Some(Problem { code: "LC8103", message: "it has no default, and no argument gives it".to_owned() }),
                None => None,
            }
        };
        if let Some(Problem { code, message }) = problem {
            problems.push(diagnostic(code, &format!("the input `{name}`: {message}"), file_shown, None, None));
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

/// The report of a refused run (commands.md 7).
pub(crate) fn refused(id: &str, function: &str, check: CheckPart) -> Json {
    let text = labcraft_runtime::report::refused(id, function, 0, &check.within(labcraft_runtime::report::RESULT_MAX / 2).0);
    serde_json::from_str(&text)
        .unwrap_or_else(|error| json!({ "schema": "labcraft-run/1", "status": "refused", "run": id, "error": error.to_string() }))
}
