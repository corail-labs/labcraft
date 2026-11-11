//! Runs as tasks (`docs/commands.md`, sections 4 and 6.4; `docs/architecture.md`, 10.8). `run.start` judges the run on
//! the caller's thread, then runs it on a thread of its own and waits for it up to `wait`; one run at a time per session,
//! the reports of the last 16 kept. **A caller waits on the run, not in the session**: [`Runs`] is shared, and `run.wait`,
//! `run.status` and `run.abort` reach it through [`Runs::execute`] without the session, so an MCP server can answer them
//! while another request is waiting.
//!
//! Without threads (WASM), the commands of runs are not enabled, and [`run_to_end`] runs one on the caller's thread: the
//! same judging, the same run and the same report (decision of the owner, 2026-11-10).

// Without threads, what keeps runs as tasks is not used.
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

pub(crate) mod convert;
mod json_text;
mod parts;
mod prepare;
mod read;
mod task;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use labcraft_runtime::run::{AbortBy, Pool, Wake};
use serde_json::{Value as Json, json};

pub use prepare::{CommandLine, Outside};
pub use task::{Clock, Still, let_go};

use crate::error::CommandError;
use crate::params::{self, Params};
use crate::registry;
use crate::session::Session;
use prepare::{Ready, given, prepare, wait_param, with_line};
use task::{Alone, Finished, Request, Steer, execute};

/// The reports a session keeps *(proposed)*: an older id is `run.unknown`.
pub const KEPT_RUNS: usize = 16;

/// How long `run.status` waits for a run to say where it is, in milliseconds, before it answers with what it knows.
#[cfg(not(target_arch = "wasm32"))]
const PROGRESS_WAIT_MS: u64 = 250;

/// Numbers the sessions of a process, so that two of them never name a `.partial` alike (commands.md 8).
static SESSIONS: AtomicU64 = AtomicU64::new(1);

/// The runs of a session, shared with whoever waits for them.
#[derive(Clone, Default)]
pub struct Runs {
    shared: Arc<Shared>,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    /// The number of the session in its process.
    session: u64,
    /// The threads for the work of native calls that blocks, in real time (decision of the owner, 2026-11-11).
    pool: Pool,
}

impl Default for Shared {
    fn default() -> Self {
        Shared { state: Mutex::default(), changed: Condvar::new(), session: SESSIONS.fetch_add(1, Ordering::Relaxed), pool: Pool::default() }
    }
}

#[derive(Default)]
struct State {
    /// The number of the last run given: `r1`, `r2`…
    count: u64,
    going: Option<Arc<Going>>,
    /// The last runs that ended, the newest last.
    ended: VecDeque<Ended>,
    /// The run whose trace is the last of its file, by the file from the root.
    traces: HashMap<String, String>,
}

/// What names the `.partial` of a run: its process and its session (commands.md 8).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Tag {
    pub process: u32,
    pub session: u64,
}

/// A run going, from the moment its id is given: `run.status` and `run.abort` reach it while it is judged.
struct Going {
    id: String,
    function: String,
    /// `file`, `inline` or `none`.
    trace: &'static str,
    /// Its `.partial` from the root, with `trace: file`, once `prepare` has found its folder.
    partial: Mutex<Option<String>>,
    abort: Mutex<Option<AbortBy>>,
    /// The grace period ended from outside: a second Ctrl-C.
    grace_over: AtomicBool,
    wanted: AtomicBool,
    /// The last `running` report it wrote, and how many it wrote.
    progress: Mutex<(Json, u64)>,
    /// `mode: "real_time"`: only such a run takes `run.set_element`.
    real_time: bool,
    /// Wakes its scheduler when something reaches it from outside.
    wake: Wake,
    /// The settings of `run.set_element` on their way, and their answers.
    live: Mutex<LiveQueue>,
    /// Settings wait in `live`: the run looks there only then.
    live_waiting: AtomicBool,
    shared: Arc<Shared>,
}

/// The settings of `run.set_element` given to a run, which it takes between two nodes, and what it answered for each.
#[derive(Default)]
struct LiveQueue {
    requests: Vec<Request>,
    /// The run ended: no more settings.
    closed: bool,
    replies: HashMap<u64, Result<Json, CommandError>>,
    next: u64,
}

impl Steer for Going {
    fn abort(&self) -> Option<AbortBy> {
        self.abort.lock().unwrap_or_else(PoisonError::into_inner).take()
    }

    fn wants_progress(&self) -> bool {
        self.wanted.swap(false, Ordering::SeqCst)
    }

    fn grace_over(&self) -> bool {
        self.grace_over.load(Ordering::SeqCst)
    }

    fn progress(&self, report: Json) {
        let mut progress = self.progress.lock().unwrap_or_else(PoisonError::into_inner);
        progress.0 = report;
        progress.1 += 1;
        drop(progress);
        self.shared.changed.notify_all();
    }

    fn wake(&self) -> Wake {
        self.wake.clone()
    }

    fn live(&self) -> Vec<Request> {
        if !self.live_waiting.swap(false, Ordering::SeqCst) {
            return Vec::new();
        }
        std::mem::take(&mut self.live.lock().unwrap_or_else(PoisonError::into_inner).requests)
    }

    fn close_live(&self) -> Vec<Request> {
        let mut live = self.live.lock().unwrap_or_else(PoisonError::into_inner);
        live.closed = true;
        std::mem::take(&mut live.requests)
    }

    fn reply(&self, tag: u64, reply: Result<Json, CommandError>) {
        self.live.lock().unwrap_or_else(PoisonError::into_inner).replies.insert(tag, reply);
        // Through the lock of the state, so that a caller that has just found no answer is already waiting.
        drop(self.shared.lock());
        self.shared.changed.notify_all();
    }
}

impl Going {
    fn new(id: &str, function: &str, shared: &Arc<Shared>, trace: &'static str, real_time: bool) -> Going {
        // What `run.status` says while the run is judged.
        let judged = json!({
            "schema": "labcraft-run/1", "status": "running", "run": id, "function": function, "mode": mode_name(real_time), "outputs": [],
            "clock": clock_zero(real_time), "resources": { "opened": 0, "open": [], "leaked": [] }, "loops": []
        });
        Going {
            id: id.to_owned(),
            function: function.to_owned(),
            trace,
            partial: Mutex::new(None),
            abort: Mutex::new(None),
            grace_over: AtomicBool::new(false),
            wanted: AtomicBool::new(false),
            progress: Mutex::new((judged, 0)),
            real_time,
            wake: Wake::default(),
            live: Mutex::default(),
            live_waiting: AtomicBool::new(false),
            shared: Arc::clone(shared),
        }
    }

    /// As the run ends: the settings it never took are answered, and no more are taken.
    fn close(&self) {
        let mut live = self.live.lock().unwrap_or_else(PoisonError::into_inner);
        live.closed = true;
        for request in std::mem::take(&mut live.requests) {
            live.replies.insert(request.tag, Err(unknown(format!("`{}` ended before it took the setting", self.id))));
        }
    }
}

fn mode_name(real_time: bool) -> &'static str {
    if real_time { "real_time" } else { "deterministic" }
}

/// `clock` of a report before its run starts (commands.md 7): `simulated_ms` in deterministic mode only.
fn clock_zero(real_time: bool) -> Json {
    if real_time { json!({ "real_ms": 0 }) } else { json!({ "simulated_ms": 0.0, "real_ms": 0 }) }
}

/// A run that ended.
struct Ended {
    id: String,
    function: String,
    /// As text: a report held as a tree of JSON takes many times its text.
    report: Arc<str>,
    /// The text of each output, for the human form of the command line.
    texts: Vec<Option<String>>,
    /// Its trace file, from the root, if it wrote one.
    trace_file: Option<String>,
    /// `file`, `inline` or `none`.
    trace: &'static str,
}

/// Which run a command acts on.
enum Found {
    Going(Arc<Going>),
    Ended(Arc<str>),
}

fn parsed(report: &str) -> Json {
    serde_json::from_str(report).unwrap_or(Json::Null)
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl State {
    /// The run of this id, or without one the run going, or else the last that ended.
    fn find(&self, id: Option<&str>) -> Result<Found, CommandError> {
        match id {
            Some(id) => {
                if let Some(going) = self.going.as_ref().filter(|going| going.id == id) {
                    return Ok(Found::Going(Arc::clone(going)));
                }
                match self.ended.iter().find(|ended| ended.id == id) {
                    Some(ended) => Ok(Found::Ended(Arc::clone(&ended.report))),
                    None => Err(unknown(format!("this session knows no run `{id}`: it keeps its last {KEPT_RUNS} runs"))),
                }
            }
            None => match (&self.going, self.ended.back()) {
                (Some(going), _) => Ok(Found::Going(Arc::clone(going))),
                (None, Some(ended)) => Ok(Found::Ended(Arc::clone(&ended.report))),
                (None, None) => Err(unknown("this session has run nothing yet")),
            },
        }
    }

    fn end(&mut self, going: &Going, finished: Finished, trace: &'static str) {
        going.close();
        if self.going.as_ref().is_some_and(|current| current.id == going.id) {
            self.going = None;
        }
        if let Some(file) = &finished.trace_file {
            self.traces.insert(file.clone(), going.id.clone());
        }
        if self.ended.len() >= KEPT_RUNS {
            self.ended.pop_front();
        }
        self.ended.push_back(Ended {
            id: going.id.clone(),
            function: going.function.clone(),
            report: Arc::from(finished.report.to_string()),
            texts: finished.texts,
            trace_file: finished.trace_file,
            trace,
        });
    }
}

pub(crate) fn unknown(message: impl Into<String>) -> CommandError {
    CommandError::new("run.unknown", message)
}

fn trace_name(mode: prepare::TraceMode) -> &'static str {
    match mode {
        prepare::TraceMode::File => "file",
        prepare::TraceMode::Inline => "inline",
        prepare::TraceMode::None => "none",
    }
}

impl Runs {
    /// The commands of runs that need no session: `run.wait`, `run.status`, `run.abort`, `run.set_element` and `run.trace`, judged against
    /// the registry and bounded as [`Session::execute`] bounds a result. `run.trace` reads through the workspace.
    pub fn execute(&self, ws: &dyn crate::workspace::Workspace, root: &std::path::Path, id: &str, params: &Json) -> Result<Json, CommandError> {
        let Some(command) =
            registry::command(id).filter(|command| matches!(command.id, "run.wait" | "run.status" | "run.abort" | "run.trace" | "run.set_element"))
        else {
            return Err(CommandError::new("command.unknown", format!("`{id}` is not a command of runs that answers without the session")));
        };
        if let Some(planned) = not_enabled_here() {
            return Err(CommandError::new("command.not_enabled", format!("`{id}` is not enabled: {planned}")));
        }
        let params = params::judge(command, params)?;
        let result = self.dispatch(ws, root, command.id, &params, 64)?;
        Ok(crate::bound::fit(result, crate::bound::MAX_RESULT))
    }

    pub(crate) fn dispatch(
        &self,
        ws: &dyn crate::workspace::Workspace,
        root: &std::path::Path,
        id: &str,
        params: &Params,
        max_nesting: u32,
    ) -> Result<Json, CommandError> {
        let run = params.str("run");
        match id {
            "run.wait" => self.wait(run, wait_param(params, max_nesting)?),
            "run.status" => self.status(run),
            "run.abort" => self.abort(run),
            "run.set_element" => self.set_element(run, params),
            "run.trace" => read::trace(self, ws, root, params),
            other => Err(CommandError::internal(format!("`{other}` is no command of runs"))),
        }
    }

    /// The report of the run when it ends within `wait_ns`, or `running`.
    fn wait(&self, id: Option<&str>, wait_ns: i64) -> Result<Json, CommandError> {
        let found = self.shared.lock().find(id)?;
        match found {
            Found::Ended(report) => Ok(parsed(&report)),
            Found::Going(going) => Ok(self.wait_for(&going, wait_ns)),
        }
    }

    /// Waits for a run going, on its own condition: the session goes on answering meanwhile.
    #[cfg(not(target_arch = "wasm32"))]
    fn wait_for(&self, going: &Arc<Going>, wait_ns: i64) -> Json {
        let wait = std::time::Duration::from_nanos(u64::try_from(wait_ns).unwrap_or(0));
        let state = self.shared.lock();
        let (state, _) = self
            .shared
            .changed
            .wait_timeout_while(state, wait, |state| state.going.as_ref().is_some_and(|current| current.id == going.id))
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(ended) = state.ended.iter().find(|ended| ended.id == going.id) {
            return parsed(&ended.report);
        }
        drop(state);
        self.progress_of(going)
    }

    #[cfg(target_arch = "wasm32")]
    fn wait_for(&self, going: &Arc<Going>, _: i64) -> Json {
        self.progress_of(going)
    }

    /// Asks the run where it is, and waits a little for its answer; the last one it gave otherwise.
    fn progress_of(&self, going: &Arc<Going>) -> Json {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let seen = going.progress.lock().unwrap_or_else(PoisonError::into_inner).1;
            going.wanted.store(true, Ordering::SeqCst);
            going.wake.wake();
            let state = self.shared.lock();
            let wait = std::time::Duration::from_millis(PROGRESS_WAIT_MS);
            let (state, _) = self
                .shared
                .changed
                .wait_timeout_while(state, wait, |state| {
                    let going_on = state.going.as_ref().is_some_and(|current| current.id == going.id);
                    going_on && going.progress.lock().unwrap_or_else(PoisonError::into_inner).1 == seen
                })
                .unwrap_or_else(PoisonError::into_inner);
            if let Some(ended) = state.ended.iter().find(|ended| ended.id == going.id) {
                return parsed(&ended.report);
            }
        }
        going.progress.lock().unwrap_or_else(PoisonError::into_inner).0.clone()
    }

    fn status(&self, id: Option<&str>) -> Result<Json, CommandError> {
        let found = self.shared.lock().find(id)?;
        match found {
            Found::Ended(report) => Ok(parsed(&report)),
            Found::Going(going) => Ok(self.progress_of(&going)),
        }
    }

    /// Aborts the run going (spec 8.2) and waits for its final report; a run that ended is left as it is.
    fn abort(&self, id: Option<&str>) -> Result<Json, CommandError> {
        let found = self.shared.lock().find(id)?;
        match found {
            Found::Ended(report) => Ok(parsed(&report)),
            Found::Going(going) => {
                *going.abort.lock().unwrap_or_else(PoisonError::into_inner) = Some(AbortBy::Request);
                going.wake.wake();
                loop {
                    let report = self.wait_for(&going, prepare::WAIT_MAX_NS);
                    if report.get("status").and_then(Json::as_str) != Some("running") {
                        return Ok(report);
                    }
                }
            }
        }
    }

    /// Sets a panel element of the run going, in real time (commands.md 6.4): the run judges it as `set` is judged, applies it
    /// between two nodes, and the answer is its entry of `panel`, with `applied: false` when the run ended first.
    fn set_element(&self, id: Option<&str>, params: &Params) -> Result<Json, CommandError> {
        let going = match self.shared.lock().find(id)? {
            Found::Going(going) => going,
            Found::Ended(_) => return Err(unknown("no run is going: `run.set_element` sets an element of a run while it goes")),
        };
        if !going.real_time {
            let message = format!(
                "`{}` runs in deterministic mode, where the time a call arrives would date the change: give it to `run.start` in `set`, at its time on the simulated clock",
                going.id
            );
            return Err(CommandError::new("command.not_enabled", message));
        }
        let element = params.str("element").unwrap_or_default().to_owned();
        let value = params.get("value").cloned().unwrap_or(Json::Null);
        let tag = {
            let mut live = going.live.lock().unwrap_or_else(PoisonError::into_inner);
            if live.closed {
                // The run takes no more settings and is writing its report: answered once it has ended, as if the setting
                // came just after, so that `run.status` never says `running` after this answer.
                drop(live);
                let mut state = self.shared.lock();
                while state.going.as_ref().is_some_and(|current| current.id == going.id) {
                    state = self.shared.changed.wait(state).unwrap_or_else(PoisonError::into_inner);
                }
                return Err(unknown("no run is going: `run.set_element` sets an element of a run while it goes"));
            }
            live.next += 1;
            let tag = live.next;
            live.requests.push(Request { tag, element, value });
            tag
        };
        going.live_waiting.store(true, Ordering::SeqCst);
        going.wake.wake();
        let answered = |going: &Going| going.live.lock().unwrap_or_else(PoisonError::into_inner).replies.remove(&tag);
        let mut state = self.shared.lock();
        loop {
            if let Some(reply) = answered(&going) {
                return reply;
            }
            // A run that ended without answering (a bug of the runtime, caught) leaves the setting unapplied.
            if !state.going.as_ref().is_some_and(|current| current.id == going.id) {
                drop(state);
                return answered(&going).unwrap_or_else(|| Err(unknown(format!("`{}` ended before it took the setting", going.id))));
            }
            state = self.shared.changed.wait(state).unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// The text of each output of a run that ended, as the language writes a constant (cli.md 6.7), in the order of the
    /// signature; `None` for one that has no text, which holds a session or a function.
    pub fn output_texts(&self, id: &str) -> Option<Vec<Option<String>>> {
        self.shared.lock().ended.iter().find(|ended| ended.id == id).map(|ended| ended.texts.clone())
    }

    /// Aborts the run going, if one is, as when the client goes away (commands.md 4): its report is kept.
    pub fn abort_going(&self) {
        if let Some(going) = self.shared.lock().going.clone() {
            *going.abort.lock().unwrap_or_else(PoisonError::into_inner) = Some(AbortBy::Interrupt);
            going.wake.wake();
        }
    }

    /// Ends the grace period of the run going, aborted or not yet (a second Ctrl-C, cli.md 6.5): the native calls still
    /// going are abandoned at once, and their resources reported as leaked. The report is still written.
    pub fn end_grace(&self) {
        if let Some(going) = self.shared.lock().going.clone() {
            going.grace_over.store(true, Ordering::SeqCst);
            going.wake.wake();
        }
    }

    /// The trace of a run, to read, from the root: the file of the run while it is the last of its file, the `.partial`
    /// of the run going, or why there is none. `last` is the file of a function that `run.trace` names.
    pub(crate) fn trace_of(&self, id: Option<&str>, last: Option<String>) -> Result<String, CommandError> {
        if let Some(last) = last {
            return Ok(last);
        }
        let state = self.shared.lock();
        if let Some(going) = state.going.as_ref().filter(|going| id.is_none_or(|id| going.id == id)) {
            let partial = going.partial.lock().unwrap_or_else(PoisonError::into_inner).clone();
            return match (going.trace, partial) {
                ("file", Some(partial)) => Ok(partial),
                ("file", None) => Err(unknown(format!("`{}` is being checked, and has written no trace yet", going.id))),
                ("inline", _) => Err(unknown(format!("`{}` gives its trace inline, in its report", going.id))),
                _ => Err(unknown(format!("`{}` runs with `trace: none`", going.id))),
            };
        }
        let ended = match id {
            Some(id) => state.ended.iter().find(|ended| ended.id == id),
            None => state.ended.back(),
        };
        let Some(ended) = ended else {
            return Err(unknown(match id {
                Some(id) => format!("this session knows no run `{id}`"),
                None => "this session has run nothing yet".to_owned(),
            }));
        };
        match (ended.trace, &ended.trace_file) {
            ("file", Some(file)) if state.traces.get(file) == Some(&ended.id) => Ok(file.clone()),
            ("file", Some(_)) => {
                Err(unknown(format!("the trace of `{}` is no longer the last of `{}`: a later run replaced it", ended.id, ended.function)))
            }
            ("file", None) => Err(unknown(format!("`{}` wrote no trace: its report says why", ended.id))),
            ("inline", _) => Err(unknown(format!("`{}` gave its trace inline, in its report", ended.id))),
            _ => Err(unknown(format!("`{}` ran with `trace: none`", ended.id))),
        }
    }

    /// Starts a run on a thread of its own, and waits for it up to `wait`.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn start(&self, session: &Session, params: &Params) -> Result<Json, CommandError> {
        self.start_with(session, params, CommandLine::default())
    }

    /// As [`Runs::start`], with what the command line gives beyond the parameters.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn start_with(&self, session: &Session, params: &Params, line: CommandLine) -> Result<Json, CommandError> {
        let max_nesting = session.config.limits.max_nesting();
        let given = with_line(given(params, max_nesting)?, line, max_nesting)?;
        if given.real_time && !session.config.clock.moves() {
            return Err(CommandError::of("command.not_enabled", "mode", "a run in real time needs a clock that moves, which this session has not"));
        }
        let trace = trace_name(given.trace);
        let going = {
            let mut state = self.shared.lock();
            if let Some(going) = &state.going {
                let mut busy = CommandError::new(
                    "run.busy",
                    format!("`{}` is going: one run at a time in a session; follow it with `run.wait`, or stop it with `run.abort`", going.id),
                );
                busy.run = Some(going.id.clone());
                return Err(busy);
            }
            state.count += 1;
            let going = Arc::new(Going::new(&format!("r{}", state.count), &given.function, &self.shared, trace, given.real_time));
            state.going = Some(Arc::clone(&going));
            going
        };
        let ready = match prepare(session, &given, &going.id) {
            Ok(ready) => ready,
            Err(error) => {
                let mut state = self.shared.lock();
                state.going = None;
                state.count -= 1;
                drop(state);
                self.shared.changed.notify_all();
                return Err(error);
            }
        };
        let prepared = match ready {
            Ready::Refused(report) => {
                self.shared.lock().end(&going, Finished { report: report.clone(), texts: Vec::new(), trace_file: None }, trace);
                self.shared.changed.notify_all();
                return Ok(report);
            }
            Ready::Run(prepared) => prepared,
        };
        let tag = Tag { process: session.config.process, session: self.shared.session };
        *going.partial.lock().unwrap_or_else(PoisonError::into_inner) = Some(task::partial_of(&prepared.trace_file, tag, &going.id));
        let ws = Arc::clone(&session.ws);
        let root = session.root.clone();
        let clock = Arc::clone(&session.config.clock);
        let shared = Arc::clone(&self.shared);
        let running = Arc::clone(&going);
        let work = move || {
            let pool = &shared.pool;
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| execute(*prepared, &ws, &root, &*clock, &*running, tag, pool)));
            let finished = outcome.unwrap_or_else(|_| Finished {
                texts: Vec::new(),
                report: json!({
                    "schema": "labcraft-run/1", "status": "fault", "run": running.id, "function": running.function, "mode": mode_name(running.real_time),
                    "outputs": [], "clock": clock_zero(running.real_time), "resources": { "opened": 0, "open": [], "leaked": [] },
                    "fault": { "code": "LC8004", "message": format!("a bug of the runtime, caught: {}", crate::panics::last_panic()), "function": running.function, "node": null },
                    "check": { "summary": {}, "diagnostics": [] }
                }),
                trace_file: None,
            });
            shared.lock().end(&running, finished, trace);
            shared.changed.notify_all();
        };
        let spawned = std::thread::Builder::new().name(format!("run {}", going.id)).stack_size(labcraft_format::STACK_SIZE).spawn(work);
        if let Err(error) = spawned {
            self.shared.lock().going = None;
            self.shared.changed.notify_all();
            return Err(CommandError::internal(format!("the thread of the run could not start: {error}")));
        }
        Ok(self.wait_for(&going, given.wait))
    }

    fn tag(&self, process: u32) -> Tag {
        Tag { process, session: self.shared.session }
    }
}

/// A duration of the command line, `30s`, `"500 ms"`, `@0s` (cli.md 6.2), in nanoseconds; why it is none otherwise.
pub fn duration_ns(text: &str) -> Result<i64, String> {
    convert::duration_text(text, 64).map_err(|problem| problem.message)
}

/// The report of a run refused before any session could start, with its diagnostics: a `FOLDER` that does not read.
pub fn refused_report(function: &str, real_time: bool, diagnostics: Vec<Json>) -> Json {
    prepare::refused_in("r1", function, real_time, parts::CheckPart::of_problems(diagnostics))
}

/// Why the commands of runs cannot run here, if they cannot.
pub(crate) fn not_enabled_here() -> Option<&'static str> {
    if cfg!(target_arch = "wasm32") { Some("runs need threads, which the WASM build of M0 does not have (D-22)") } else { None }
}

/// Runs `run.start` to its end on the caller's thread, whatever `wait`: the commands of runs without threads, and the
/// corpus of runs in WASM. The same judging, run and report as [`Runs`] gives; nothing is kept.
pub fn run_to_end(session: &Session, params: &Json) -> Result<Json, CommandError> {
    let Some(command) = registry::command("run.start") else { return Err(CommandError::internal("`run.start` is not registered")) };
    let params = params::judge(command, params)?;
    let given = given(&params, session.config.limits.max_nesting())?;
    if given.real_time {
        return Err(CommandError::of(
            "command.not_enabled",
            "mode",
            "a run in real time needs threads, which a run to its end on this thread has not",
        ));
    }
    let tag = session.runs().tag(session.config.process);
    let report = match prepare(session, &given, "r1")? {
        Ready::Refused(report) => report,
        Ready::Run(prepared) => {
            execute(*prepared, &session.ws, &session.root, &*session.config.clock, &Alone, tag, &session.runs().shared.pool).report
        }
    };
    Ok(crate::bound::fit(report, crate::bound::MAX_RESULT))
}
