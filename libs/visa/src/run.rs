//! What one run of `visa` holds: its sessions and the instruments of its simulation, and each call in progress as a state
//! machine (`docs/architecture.md`, 10.6). What `visa` guarantees (`libs/visa/README.md`) is in its keys: a call takes the
//! key of its session for all of its length, so it is atomic on its session; a command takes the key of its instrument for
//! its latency, so an instrument handles one command at a time, whichever session sends it, and is free while a session
//! waits for its answer (simulation.md 4).

use std::collections::HashMap;
use std::sync::Arc;

use labcraft_runtime::native::{CallContext, CallId, HandleInfo, InstrumentLog, Key, LibraryRun, NativeOutcome, Step};
use labcraft_runtime::{Handle, Value};
use labcraft_trace::number::duration_text;

use crate::file::Bench;
use crate::instrument::{Instrument, LOG_BUDGET, QuantityState, execute};

const VI_ERROR_TMO: i64 = -1_073_807_339;
const VI_ERROR_RSRC_NFOUND: i64 = -1_073_807_343;
const VI_ERROR_INV_OBJECT: i64 = -1_073_807_346;

#[derive(Debug)]
struct Session {
    /// Its instrument, by its index in the bench, which names its resource: a session costs the same whatever it names.
    instrument: usize,
    /// Nanoseconds: what a read with no answer waits.
    timeout: i64,
    open: bool,
    /// The answer that waits for `visa.read`.
    answer: Option<String>,
}

/// Where a call is.
#[derive(Debug)]
enum Phase {
    Start,
    /// It holds the key of its session.
    Session,
    /// It holds the key of its instrument too, and waits for its latency.
    Instrument,
    /// The latency has passed: the command is handled now.
    Handle,
    /// The instrument is free again; a query reads its answer.
    Read,
    /// A read with no answer waited for its timeout.
    TimedOut,
    /// `open` of a resource that does not exist waited for its timeout.
    NotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Open,
    Write,
    Read,
    Query,
    Close,
}

#[derive(Debug)]
struct VisaCall {
    kind: Kind,
    phase: Phase,
    session: Handle,
    resource: String,
    command: String,
    timeout: i64,
}

/// One run of `visa`.
pub struct VisaRun {
    library: u32,
    bench: Option<Arc<Bench>>,
    instruments: Vec<Instrument>,
    quantities: Vec<QuantityState>,
    by_resource: HashMap<String, usize>,
    sessions: Vec<Session>,
    calls: HashMap<CallId, VisaCall>,
    next_call: u64,
    /// What the log of all its instruments may still hold, in bytes.
    log_budget: u64,
}

impl VisaRun {
    pub fn new(library: u32, bench: Option<Arc<Bench>>) -> Self {
        let (instruments, quantities, by_resource) = match &bench {
            Some(bench) => (
                bench.instruments.iter().map(Instrument::new).collect(),
                QuantityState::of(bench),
                bench.instruments.iter().enumerate().map(|(index, spec)| (spec.resource.clone(), index)).collect(),
            ),
            None => (Vec::new(), Vec::new(), HashMap::new()),
        };
        VisaRun {
            library,
            bench,
            instruments,
            quantities,
            by_resource,
            sessions: Vec::new(),
            calls: HashMap::new(),
            next_call: 0,
            log_budget: LOG_BUDGET,
        }
    }

    /// The session of a handle of this run: numbers start at 1, so the default of a `session`, numbered 0, is none.
    fn session(&self, handle: Handle) -> Option<&Session> {
        if handle.library != self.library {
            return None;
        }
        let index = usize::try_from(handle.number).ok()?.checked_sub(1)?;
        self.sessions.get(index)
    }

    fn session_mut(&mut self, handle: Handle) -> Option<&mut Session> {
        if handle.library != self.library {
            return None;
        }
        let index = usize::try_from(handle.number).ok()?.checked_sub(1)?;
        self.sessions.get_mut(index)
    }

    fn session_key(&self, handle: Handle) -> Key {
        Key { library: self.library, id: handle.number.saturating_mul(2) }
    }

    fn instrument_key(&self, instrument: usize) -> Key {
        Key { library: self.library, id: (instrument as u64).saturating_mul(2).saturating_add(1) }
    }

    fn resource(&self, session: &Session) -> &str {
        self.bench.as_ref().and_then(|bench| bench.instruments.get(session.instrument)).map_or("", |spec| spec.resource.as_str())
    }

    fn invalid(&self, handle: Handle) -> NativeOutcome {
        let message = match self.session(handle) {
            Some(session) => format!("{}: the session is closed", self.resource(session)),
            None => "no session: this one was never opened (the default of a `visa.session`)".to_owned(),
        };
        NativeOutcome::Fail { category: "visa.invalid_session", code: VI_ERROR_INV_OBJECT, message }
    }

    fn is_open(&self, handle: Handle) -> bool {
        self.session(handle).is_some_and(|session| session.open)
    }

    /// What a call gives once it is done, by its kind: its session passes through (5.3).
    fn outputs(&self, call: &VisaCall, text: Option<String>) -> NativeOutcome {
        let session = Value::Handle(call.session);
        NativeOutcome::Outputs(match call.kind {
            Kind::Open => vec![session, Value::NONE],
            Kind::Write => vec![session, Value::NONE],
            Kind::Read | Kind::Query => vec![Value::string(text.unwrap_or_default()), session, Value::NONE],
            Kind::Close => vec![Value::NONE],
        })
    }

    fn advance(&mut self, id: CallId, cx: CallContext) -> Step {
        let Some(mut call) = self.calls.remove(&id) else {
            return Step::Done(NativeOutcome::Fail {
                category: "visa.invalid_session",
                code: VI_ERROR_INV_OBJECT,
                message: "a call that is not in progress".to_owned(),
            });
        };
        let step = self.next_step(&mut call, cx);
        if !matches!(step, Step::Done(_)) {
            self.calls.insert(id, call);
        }
        step
    }

    fn next_step(&mut self, call: &mut VisaCall, cx: CallContext) -> Step {
        match (call.kind, &call.phase) {
            (Kind::Open, Phase::Start) => match self.by_resource.get(&call.resource).copied() {
                Some(instrument) => {
                    self.sessions.push(Session { instrument, timeout: call.timeout, open: true, answer: None });
                    call.session = Handle { library: self.library, number: self.sessions.len() as u64 };
                    Step::Done(self.outputs(call, None))
                }
                // VISA looks for the resource for the time it is given (simulation.md 6).
                None => {
                    call.phase = Phase::NotFound;
                    Step::Sleep(call.timeout)
                }
            },
            (Kind::Open, _) => {
                let resource = shown(&call.resource);
                let message = if self.bench.is_none() {
                    format!("{resource}: no resource exists without a simulation file (--sim)")
                } else {
                    format!("{resource}: the simulation file has no such instrument")
                };
                Step::Done(NativeOutcome::Fail { category: "visa.resource_not_found", code: VI_ERROR_RSRC_NFOUND, message })
            }
            (_, Phase::Start) => {
                if !self.is_open(call.session) {
                    return Step::Done(self.invalid(call.session));
                }
                call.phase = Phase::Session;
                Step::Exclusive(self.session_key(call.session))
            }
            (_, Phase::Session) => {
                // Another call may have closed it while this one waited for its key.
                if !self.is_open(call.session) {
                    return Step::Done(self.invalid(call.session));
                }
                match call.kind {
                    Kind::Close => {
                        if let Some(session) = self.session_mut(call.session) {
                            session.open = false;
                            session.answer = None;
                        }
                        Step::Done(self.outputs(call, None))
                    }
                    Kind::Read => {
                        call.phase = Phase::Read;
                        self.next_step(call, cx)
                    }
                    _ => {
                        let instrument = self.session(call.session).map(|session| session.instrument);
                        call.phase = Phase::Instrument;
                        match instrument {
                            Some(instrument) => Step::Exclusive(self.instrument_key(instrument)),
                            None => Step::Done(self.invalid(call.session)),
                        }
                    }
                }
            }
            (_, Phase::Instrument) => {
                let latency = self
                    .session(call.session)
                    .and_then(|session| self.bench.as_ref()?.instruments.get(session.instrument))
                    .map_or(0, |spec| spec.latency);
                call.phase = Phase::Handle;
                if latency > 0 { Step::Sleep(latency) } else { self.next_step(call, cx) }
            }
            (_, Phase::Handle) => {
                // A session that the run released while the command waited is closed: the command goes nowhere.
                let Some(instrument) = self.session(call.session).filter(|session| session.open).map(|session| session.instrument) else {
                    return Step::Done(self.invalid(call.session));
                };
                // A new command discards an answer not read yet, when the instrument handles it (simulation.md 4; decision of
                // the owner, 2026-11-09): the queue follows the order in which the instrument handles its commands.
                let discarded = self.session_mut(call.session).and_then(|session| session.answer.take()).is_some();
                if discarded && let Some(state) = self.instruments.get_mut(instrument) {
                    state.push_error(-410, "Query INTERRUPTED");
                }
                let answer = match (&self.bench, self.instruments.get_mut(instrument)) {
                    (Some(bench), Some(state)) => match bench.instruments.get(instrument) {
                        Some(spec) => execute(spec, state, &mut self.quantities, bench, &mut self.log_budget, &call.command, cx.now),
                        None => None,
                    },
                    _ => None,
                };
                if let Some(session) = self.session_mut(call.session) {
                    session.answer = answer;
                }
                call.phase = Phase::Read;
                Step::Release(self.instrument_key(instrument))
            }
            (Kind::Write, Phase::Read) => Step::Done(self.outputs(call, None)),
            (_, Phase::Read) => match self.session_mut(call.session).and_then(|session| session.answer.take()) {
                Some(answer) => Step::Done(self.outputs(call, Some(answer))),
                None => {
                    let timeout = self.session(call.session).map_or(0, |session| session.timeout);
                    call.phase = Phase::TimedOut;
                    Step::Sleep(timeout)
                }
            },
            (_, Phase::TimedOut) => {
                let (resource, timeout) =
                    self.session(call.session).map_or((String::new(), 0), |session| (self.resource(session).to_owned(), session.timeout));
                let what = if call.kind == Kind::Query { format!("to `{}`", shown(&call.command)) } else { "to read".to_owned() };
                let message = format!("{resource}: no answer {what} within {}", duration_text(timeout));
                Step::Done(NativeOutcome::Fail { category: "io.timeout", code: VI_ERROR_TMO, message })
            }
            (_, Phase::NotFound) => Step::Done(NativeOutcome::Fail {
                category: "visa.resource_not_found",
                code: VI_ERROR_RSRC_NFOUND,
                message: format!("{}: not found", shown(&call.resource)),
            }),
        }
    }
}

/// What a message shows of a command: its start, its control characters escaped.
fn shown(text: &str) -> String {
    let start: String = text.chars().take(64).flat_map(char::escape_debug).collect();
    if text.chars().count() > 64 { format!("{start}…") } else { start }
}

impl LibraryRun for VisaRun {
    fn call(&mut self, function: &str, inputs: Vec<Value>) -> Option<CallId> {
        let kind = match function {
            "open" => Kind::Open,
            "write" => Kind::Write,
            "read" => Kind::Read,
            "query" => Kind::Query,
            "close" => Kind::Close,
            _ => return None,
        };
        let mut call = VisaCall { kind, phase: Phase::Start, session: Handle::INVALID, resource: String::new(), command: String::new(), timeout: 0 };
        let text = |value: Option<&Value>| match value {
            Some(Value::String(text)) => text.to_string(),
            _ => String::new(),
        };
        match kind {
            Kind::Open => {
                call.resource = text(inputs.first());
                call.timeout = match inputs.get(1) {
                    Some(Value::Duration(ns)) => (*ns).max(0),
                    _ => 0,
                };
            }
            _ => {
                if let Some(Value::Handle(handle)) = inputs.first() {
                    call.session = *handle;
                }
                if matches!(kind, Kind::Write | Kind::Query) {
                    call.command = text(inputs.get(1));
                }
            }
        }
        let id = CallId(self.next_call);
        self.next_call = self.next_call.saturating_add(1);
        self.calls.insert(id, call);
        Some(id)
    }

    fn step(&mut self, call: CallId, cx: CallContext) -> Step {
        self.advance(call, cx)
    }

    fn cancel(&mut self, call: CallId) {
        self.calls.remove(&call);
    }

    fn describe(&self, handle: Handle) -> Option<HandleInfo> {
        let session = self.session(handle)?;
        Some(HandleInfo { kind: "session", designates: self.resource(session).to_owned(), open: session.open })
    }

    /// Closes a session that the run leaves open after an abort or a fault (README, "What `visa` guarantees"). Closing a session
    /// does not switch a supply off.
    fn release(&mut self, handle: Handle) {
        if let Some(session) = self.session_mut(handle) {
            session.open = false;
            session.answer = None;
        }
    }

    fn instruments(&self) -> Vec<InstrumentLog> {
        let Some(bench) = &self.bench else { return Vec::new() };
        let mut logs: Vec<InstrumentLog> = bench
            .instruments
            .iter()
            .zip(&self.instruments)
            .filter(|(_, state)| !state.log.is_empty() || state.more > 0)
            .map(|(spec, state)| InstrumentLog { resource: spec.resource.clone(), commands: state.log.clone(), more: state.more })
            .collect();
        logs.sort_by(|a, b| a.resource.cmp(&b.resource));
        logs
    }
}
