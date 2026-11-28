//! What one run of `visa` holds: its sessions and the instruments of its simulation, and each call in progress as a state
//! machine (`docs/architecture.md`, 10.6). What `visa` guarantees (`libs/visa/README.md`) is in its keys: a call takes the
//! key of its session for all of its length, so it is atomic on its session; a command takes the key of its instrument for
//! its latency, so an instrument handles one command at a time, whichever session sends it, and is free while a session
//! waits for its answer (simulation.md 4).
//!
//! One run serves the native functions of `visa` (`visa.open` and the others, a `visa.session`) and those of its
//! sub-library `visa.sim` (README, "Simulation: `visa.sim`", a `visa.sim.instrument`), over the same instruments: a handle
//! of either is a session here, which knows its type.

use std::collections::HashMap;
use std::sync::Arc;

use labcraft_runtime::io::{CLOSED, INVALID_ARGUMENT, UNSUPPORTED};
use labcraft_runtime::native::{CallContext, CallId, HandleInfo, InstrumentLog, Key, LibraryRun, LogBudget, NativeOutcome, Step};
use labcraft_runtime::stream::{Failure, Got, Need, Op, Scan, bytes_at, op_of, shown};
use labcraft_runtime::{Handle, Value};

use crate::file::{Bench, InstrumentSpec, key_of};
use crate::instrument::{Instrument, QuantityState, execute, hung, ignore};

const VI_ERROR_TMO: i64 = -1_073_807_339;
const VI_ERROR_RSRC_NFOUND: i64 = -1_073_807_343;
const VI_ERROR_INV_OBJECT: i64 = -1_073_807_346;
const VI_ERROR_CONN_LOST: i64 = -1_073_807_194;

/// The type of a handle: a `visa.session` of the native `visa`, or a `visa.sim.instrument`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flavor {
    Session,
    Instrument,
}

impl Flavor {
    /// As [`HandleInfo::kind`] names it, from the library `visa`: the report writes `visa.session`, `visa.sim.instrument`.
    fn kind(self) -> &'static str {
        match self {
            Flavor::Session => "session",
            Flavor::Instrument => "sim.instrument",
        }
    }

    /// What a message calls it.
    fn noun(self) -> &'static str {
        match self {
            Flavor::Session => "session",
            Flavor::Instrument => "instrument",
        }
    }
}

#[derive(Debug)]
struct Session {
    flavor: Flavor,
    /// Its instrument, by its index in the bench, which names its resource: a session costs the same whatever it names.
    instrument: usize,
    /// Nanoseconds: what a read with no answer waits.
    timeout: i64,
    open: bool,
    /// What is left of the answer that waits, as a real instrument sends it: its text, then `\n` (simulation.md 4, "Bytes").
    answer: Option<Vec<u8>>,
}

impl Session {
    /// Closed by `close` or by `release`: the answer that waits goes with it.
    fn close(&mut self) {
        self.open = false;
        self.answer = None;
    }
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

#[derive(Debug)]
enum Kind {
    Open {
        resource: String,
        timeout: i64,
    },
    /// `visa.write` of a text, `visa.sim.write` of bytes: one command.
    Write(Vec<u8>),
    /// `visa.read`: the whole answer, as text.
    Read,
    Query(Vec<u8>),
    /// `visa.sim.read` and `visa.sim.read_until`: bytes of the answer.
    Take(Need),
    /// Why its settings are out of range, when they are.
    SetSerial(Option<String>),
    Close,
    /// An argument out of its range: the call fails at once.
    Refused(Failure),
}

#[derive(Debug)]
struct VisaCall {
    flavor: Flavor,
    kind: Kind,
    phase: Phase,
    session: Handle,
}

/// One run of `visa`.
pub struct VisaRun {
    library: u32,
    bench: Option<Arc<Bench>>,
    instruments: Vec<Instrument>,
    quantities: Vec<QuantityState>,
    /// Each instrument by what its names reduce to: its `SIM::` name, or the endpoint of its real name.
    by_key: HashMap<String, usize>,
    sessions: Vec<Session>,
    calls: HashMap<CallId, VisaCall>,
    next_call: u64,
    /// What the log of all its instruments may still hold: the budget of the run, shared with its other libraries.
    log_budget: LogBudget,
}

impl VisaRun {
    pub fn new(library: u32, bench: Option<Arc<Bench>>, budget: LogBudget) -> Self {
        let (instruments, quantities, by_key) = match &bench {
            Some(bench) => (
                bench.instruments.iter().map(Instrument::new).collect(),
                QuantityState::of(bench),
                bench.instruments.iter().enumerate().map(|(index, spec)| (spec.key.clone(), index)).collect(),
            ),
            None => (Vec::new(), Vec::new(), HashMap::new()),
        };
        VisaRun { library, bench, instruments, quantities, by_key, sessions: Vec::new(), calls: HashMap::new(), next_call: 0, log_budget: budget }
    }

    /// The index of the session of a handle of this run: numbers start at 1, so the default of a handle, numbered 0, is none.
    fn index(&self, handle: Handle) -> Option<usize> {
        if handle.library != self.library {
            return None;
        }
        usize::try_from(handle.number).ok()?.checked_sub(1)
    }

    fn session(&self, handle: Handle) -> Option<&Session> {
        self.sessions.get(self.index(handle)?)
    }

    fn session_mut(&mut self, handle: Handle) -> Option<&mut Session> {
        let index = self.index(handle)?;
        self.sessions.get_mut(index)
    }

    fn spec(&self, session: &Session) -> Option<&InstrumentSpec> {
        self.bench.as_ref()?.instruments.get(session.instrument)
    }

    fn session_key(&self, handle: Handle) -> Key {
        Key { library: self.library, id: handle.number.saturating_mul(2) }
    }

    fn instrument_key(&self, instrument: usize) -> Key {
        Key { library: self.library, id: (instrument as u64).saturating_mul(2).saturating_add(1) }
    }

    fn resource(&self, session: &Session) -> &str {
        self.spec(session).map_or("", |spec| spec.resource.as_str())
    }

    /// The open session of a call, or what the call gets on a handle that is not open (README, "Errors and their codes"):
    /// one opened and then closed is `io.closed`, since a value never changes and its transport knows only that it is
    /// closed; one never opened, the default or that of a failed `open`, `visa.invalid_session`.
    fn open_session(&self, call: &VisaCall) -> Result<&Session, NativeOutcome> {
        match self.session(call.session) {
            Some(session) if session.open => Ok(session),
            Some(session) => Err(NativeOutcome::Fail {
                category: CLOSED,
                code: VI_ERROR_CONN_LOST,
                message: format!("{}: the {} is closed", self.resource(session), session.flavor.noun()),
            }),
            None => Err(NativeOutcome::Fail {
                category: "visa.invalid_session",
                code: VI_ERROR_INV_OBJECT,
                message: format!(
                    "no {}: this one was never opened (the default of a `visa.{}`, or that of an `open` that failed)",
                    call.flavor.noun(),
                    call.flavor.kind()
                ),
            }),
        }
    }

    /// A call on a handle that is not open ends with what [`Self::open_session`] says, but `close`, which succeeds and does
    /// nothing, as the `close` of every transport.
    fn not_open(&self, call: &VisaCall, outcome: NativeOutcome) -> Step {
        Step::Done(if matches!(call.kind, Kind::Close) { self.outputs(call, None) } else { outcome })
    }

    /// What a call gives once it is done, by its kind: its handle passes through (5.3).
    fn outputs(&self, call: &VisaCall, read: Option<Value>) -> NativeOutcome {
        let session = Value::Handle(call.session);
        NativeOutcome::Outputs(match call.kind {
            Kind::Open { .. } | Kind::Write(_) | Kind::SetSerial(_) => vec![session, Value::NONE],
            Kind::Read | Kind::Query(_) => vec![read.unwrap_or_else(|| Value::string(String::new())), session, Value::NONE],
            Kind::Take(_) => vec![read.unwrap_or_else(|| Value::Bytes(Arc::from(&[][..]))), session, Value::NONE],
            Kind::Close | Kind::Refused(_) => vec![Value::NONE],
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
        match (&call.kind, &call.phase) {
            (Kind::Refused(failure), _) => Step::Done(failure.clone().outcome()),
            (Kind::Open { resource, timeout }, Phase::Start) => match key_of(resource).and_then(|key| self.by_key.get(&key).copied()) {
                Some(instrument) => {
                    self.sessions.push(Session { flavor: call.flavor, instrument, timeout: *timeout, open: true, answer: None });
                    call.session = Handle { library: self.library, number: self.sessions.len() as u64 };
                    Step::Done(self.outputs(call, None))
                }
                // VISA looks for the resource for the time it is given (simulation.md 6).
                None => {
                    let timeout = *timeout;
                    call.phase = Phase::NotFound;
                    Step::Sleep(timeout)
                }
            },
            // Only an `open` waits there.
            (_, Phase::NotFound) => {
                let resource = match &call.kind {
                    Kind::Open { resource, .. } => shown(resource),
                    _ => String::new(),
                };
                // `visa` sees only its section: a run with no simulation file, and one whose file has no `[visa]`, are one
                // case here.
                let message = if self.bench.is_none() {
                    format!("{resource}: no instrument is simulated: the run has no simulation file (--sim), or its file no `[visa]`")
                } else {
                    format!("{resource}: the simulation file has no such instrument")
                };
                Step::Done(NativeOutcome::Fail { category: "visa.resource_not_found", code: VI_ERROR_RSRC_NFOUND, message })
            }
            (_, Phase::Start) => match self.open_session(call) {
                Err(outcome) => self.not_open(call, outcome),
                Ok(_) => {
                    call.phase = Phase::Session;
                    Step::Exclusive(self.session_key(call.session))
                }
            },
            (_, Phase::Session) => {
                // Another call may have closed it while this one waited for its key.
                let instrument = match self.open_session(call) {
                    Err(outcome) => return self.not_open(call, outcome),
                    Ok(session) => session.instrument,
                };
                match call.kind {
                    Kind::Close => {
                        self.release(call.session);
                        Step::Done(self.outputs(call, None))
                    }
                    Kind::SetSerial(_) => Step::Done(self.set_serial(call)),
                    Kind::Read | Kind::Take(_) => {
                        call.phase = Phase::Read;
                        self.next_step(call, cx)
                    }
                    _ => {
                        call.phase = Phase::Instrument;
                        Step::Exclusive(self.instrument_key(instrument))
                    }
                }
            }
            (_, Phase::Instrument) => {
                let latency = self.session(call.session).and_then(|session| self.spec(session)).map_or(0, |spec| spec.latency);
                call.phase = Phase::Handle;
                if latency > 0 { Step::Sleep(latency) } else { self.next_step(call, cx) }
            }
            (_, Phase::Handle) => {
                // A session that the run released while the command waited is closed: the command goes nowhere.
                let instrument = match self.open_session(call) {
                    Err(outcome) => return Step::Done(outcome),
                    Ok(session) => session.instrument,
                };
                self.handle_command(call, instrument, cx.now);
                call.phase = Phase::Read;
                Step::Release(self.instrument_key(instrument))
            }
            (Kind::Write(_), Phase::Read) => Step::Done(self.outputs(call, None)),
            (Kind::Take(need), Phase::Read) => match self.take(call.session, need) {
                Some(outcome) => Step::Done(self.finish_take(call, outcome)),
                None => self.wait_for_timeout(call),
            },
            (_, Phase::Read) => match self.session_mut(call.session).and_then(|session| session.answer.take()) {
                Some(mut answer) => {
                    // `visa.read` takes all that is left of the answer, without its `\n`.
                    if answer.last() == Some(&b'\n') {
                        answer.pop();
                    }
                    let text = String::from_utf8(answer).unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned());
                    Step::Done(self.outputs(call, Some(Value::string(text))))
                }
                None => self.wait_for_timeout(call),
            },
            (_, Phase::TimedOut) => {
                let (resource, timeout) =
                    self.session(call.session).map_or((String::new(), 0), |session| (self.resource(session).to_owned(), session.timeout));
                let what = match &call.kind {
                    Kind::Query(command) => format!("no answer to `{}`", shown(&String::from_utf8_lossy(command))),
                    Kind::Take(need) => need.wanted(),
                    _ => "no answer to read".to_owned(),
                };
                Step::Done(Failure { code: VI_ERROR_TMO, ..Failure::timed_out(&resource, &what, timeout) }.outcome())
            }
        }
    }

    /// The instrument handles the command of a write or a query, its latency passed (simulation.md 4).
    fn handle_command(&mut self, call: &VisaCall, instrument: usize, now: i64) {
        let (Kind::Write(command) | Kind::Query(command)) = &call.kind else { return };
        let index = self.index(call.session);
        let (Some(bench), Some(state)) = (&self.bench, self.instruments.get_mut(instrument)) else { return };
        let Some(spec) = bench.instruments.get(instrument) else { return };
        let Some(session) = index.and_then(|index| self.sessions.get_mut(index)) else { return };
        // A new command discards an answer not read yet, when the instrument handles it (simulation.md 4; decision of the owner,
        // 2026-11-09): the queue follows the order in which the instrument handles its commands. A hung instrument does nothing
        // else with it, and no error enters its queue: the read that waits ends with its timeout, whatever the query.
        let discarded = session.answer.take().is_some();
        if hung(spec, now) {
            ignore(state, &self.log_budget, command, now);
            return;
        }
        if discarded {
            state.push_error(-410, "Query INTERRUPTED");
        }
        // As a real instrument sends it: its text followed by `\n`.
        session.answer =
            execute(spec, state, &mut self.quantities, bench, &self.log_budget, command, now).map(|answer| format!("{answer}\n").into_bytes());
    }

    /// `visa.sim.read` and `visa.sim.read_until` (simulation.md 4, "Bytes"): what they need of the answer that waits, the rest
    /// left for the next read, and whether it was `max` bytes without the terminator; `None` with less than they need.
    fn take(&mut self, handle: Handle, need: &Need) -> Option<(Vec<u8>, bool)> {
        let session = self.session_mut(handle)?;
        let waiting = session.answer.as_deref().unwrap_or_default();
        let (count, too_long) = match need.judge(waiting, &mut Scan::default()) {
            Got::Take(count) => (count, false),
            Got::TooLong(count) => (count, true),
            Got::More => return None,
        };
        let taken: Vec<u8> = session.answer.as_mut().map(|answer| answer.drain(..count.min(answer.len())).collect()).unwrap_or_default();
        if session.answer.as_ref().is_some_and(Vec::is_empty) {
            session.answer = None;
        }
        Some((taken, too_long))
    }

    fn finish_take(&self, call: &VisaCall, (taken, too_long): (Vec<u8>, bool)) -> NativeOutcome {
        if too_long {
            let resource = self.session(call.session).map_or("", |session| self.resource(session));
            return Failure::too_long(resource, taken.len()).outcome();
        }
        self.outputs(call, Some(Value::Bytes(taken.into())))
    }

    fn wait_for_timeout(&mut self, call: &mut VisaCall) -> Step {
        let timeout = self.session(call.session).map_or(0, |session| session.timeout);
        call.phase = Phase::TimedOut;
        Step::Sleep(timeout)
    }

    /// `visa.sim.set_serial` (simulation.md 4, "Bytes"): an instrument with an `ASRL` name judges the settings as
    /// `serial.open` does, then changes nothing; any other is no serial session.
    fn set_serial(&self, call: &VisaCall) -> NativeOutcome {
        let Some(spec) = self.session(call.session).and_then(|session| self.spec(session)) else { return self.outputs(call, None) };
        if !spec.key.starts_with("serial:") {
            let message = format!("{}: no serial instrument: `set_serial` is for an instrument with an `ASRL` name", shown(&spec.resource));
            return NativeOutcome::Fail { category: UNSUPPORTED, code: 0, message };
        }
        match &call.kind {
            Kind::SetSerial(Some(why)) => Failure::new(INVALID_ARGUMENT, format!("{}: {why}", shown(&spec.resource))).outcome(),
            _ => self.outputs(call, None),
        }
    }
}

/// What is out of range in the settings of `set_serial(instrument, baud, data_bits, parity, stop_bits, flow, err)`, as
/// `serial.open` says it (libs/serial/README.md, `open`): `baud` above 0, `data_bits` 5 to 8. The enumerations are what
/// their types allow.
fn out_of_range(inputs: &[Value]) -> Option<String> {
    let baud = match inputs.get(1) {
        Some(Value::U32(baud)) => *baud,
        _ => 9600,
    };
    let data_bits = match inputs.get(2) {
        Some(Value::U8(bits)) => *bits,
        _ => 8,
    };
    if baud == 0 {
        Some("a baud rate of 0: `baud` is above 0".to_owned())
    } else if !(5..=8).contains(&data_bits) {
        Some(format!("`data_bits` is 5 to 8, and this is {data_bits}"))
    } else {
        None
    }
}

/// The function of a call, by the name its `extern rust` gives (`open` for `visa.open`, `sim_open` for `visa.sim.open`),
/// with its inputs read.
fn kind_of(function: &str, inputs: &[Value]) -> Option<(Flavor, Kind)> {
    let text = || bytes_at(inputs, 1).to_vec();
    let open = || Kind::Open {
        resource: match inputs.first() {
            Some(Value::String(text)) => text.to_string(),
            _ => String::new(),
        },
        timeout: match inputs.get(1) {
            Some(Value::Duration(ns)) => (*ns).max(0),
            _ => 0,
        },
    };
    // A `count` or a `max` out of its range, or an empty terminator, is `io.invalid_argument`, as for `net` and `serial`.
    let take = |stream: &str| match op_of(stream, inputs) {
        Ok(Op::Read { need, .. }) => Kind::Take(need),
        Ok(Op::Write(_)) => Kind::Take(Need::Exactly(0)),
        Err(failure) => Kind::Refused(failure),
    };
    Some(match function {
        "open" => (Flavor::Session, open()),
        "write" => (Flavor::Session, Kind::Write(text())),
        "read" => (Flavor::Session, Kind::Read),
        "query" => (Flavor::Session, Kind::Query(text())),
        "close" => (Flavor::Session, Kind::Close),
        "sim_open" => (Flavor::Instrument, open()),
        "sim_write" => (Flavor::Instrument, Kind::Write(text())),
        "sim_read" => (Flavor::Instrument, take("read")),
        "sim_read_until" => (Flavor::Instrument, take("read_until")),
        "sim_set_serial" => (Flavor::Instrument, Kind::SetSerial(out_of_range(inputs))),
        "sim_close" => (Flavor::Instrument, Kind::Close),
        _ => return None,
    })
}

impl LibraryRun for VisaRun {
    fn call(&mut self, function: &str, inputs: Vec<Value>) -> Option<CallId> {
        let (flavor, kind) = kind_of(function, &inputs)?;
        let session = match (&kind, inputs.first()) {
            (Kind::Open { .. }, _) => Handle::INVALID,
            (_, Some(Value::Handle(handle))) => *handle,
            _ => Handle::INVALID,
        };
        let id = CallId(self.next_call);
        self.next_call = self.next_call.saturating_add(1);
        self.calls.insert(id, VisaCall { flavor, kind, phase: Phase::Start, session });
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
        Some(HandleInfo { kind: session.flavor.kind(), designates: self.resource(session).to_owned(), open: session.open })
    }

    /// Closes a session that the run leaves open after an abort or a fault (README, "What `visa` guarantees"). Closing a session
    /// does not switch a supply off.
    fn release(&mut self, handle: Handle) {
        if let Some(session) = self.session_mut(handle) {
            session.close();
        }
    }

    fn instruments(&self) -> Vec<InstrumentLog> {
        let Some(bench) = &self.bench else { return Vec::new() };
        let mut logs: Vec<InstrumentLog> = bench
            .instruments
            .iter()
            .zip(&self.instruments)
            .filter(|(_, state)| !state.log.is_empty())
            .map(|(spec, state)| state.log.instrument(&spec.resource))
            .collect();
        logs.sort_by(|a, b| a.resource.cmp(&b.resource));
        logs
    }
}
