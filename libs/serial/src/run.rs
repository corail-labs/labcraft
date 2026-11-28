//! What one run of `serial` holds: its ports, and each call in progress as a state machine (`docs/architecture.md`, 10.6).
//! A call takes the key of its port for all of its length, so it is atomic on it (`libs/serial/README.md`, "What `serial`
//! guarantees"): a read that waits for its timeout holds it. An open follows the order of io.md 3: the name, then its
//! arguments, then a port already open in this run (`serial.port_busy`), then `io.unsupported`, then the table of what a
//! run reaches, then the right of the run, then the open of the hardware; then it waits `settle` on the clock of the run,
//! and empties the input.

use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use labcraft_runtime::io::{self, CLOSED, INVALID_ARGUMENT, Open, open_real};
use labcraft_runtime::native::{
    CallContext, CallId, Context, EndpointLogs, HandleInfo, InstrumentLog, Key, LibraryRun, LogBudget, Mode, NativeOutcome, Step,
};
use labcraft_runtime::stream::{self, Clock, Device, Failure, Names, Next, Op, Scan, Taken, read_done, read_step, write_step, wrote};
use labcraft_runtime::{Handle, Items, Value};

use crate::endpoint::{self, Endpoint, PORT_BUSY, PORT_NOT_FOUND};
use crate::file::{DeviceSpec, Devices};
#[cfg(not(target_arch = "wasm32"))]
use crate::real;
use crate::settings::{DATA_BITS, Flow, Parity, Settings, StopBits};

fn failed(category: &'static str, message: String) -> NativeOutcome {
    Failure::new(category, message).outcome()
}

/// A simulated device seen from one port.
struct Simulated {
    spec: Arc<DeviceSpec>,
    stream: Device,
    /// The port has settings the device does not expect: it takes the writes and never answers (io.md 6). Set by the open and
    /// by each `configure`, so the writes after a `configure` to the settings it expects are answered.
    deaf: bool,
}

/// Whether a device that expects `spec` is deaf to a port with `settings`; when it is, the report says which settings differ,
/// at `now` (io.md 6; decision of the owner, 2026-11-20). An open and a `configure` say them alike.
fn judge(log: &mut EndpointLogs, endpoint: &str, now: i64, spec: &DeviceSpec, settings: &Settings) -> bool {
    let differ = spec.expected.differ(settings);
    let deaf = !differ.is_empty();
    if deaf {
        log.settings(endpoint, now, differ);
    }
    deaf
}

/// What a port talks to.
enum Backend {
    Simulated(Simulated),
    #[cfg(not(target_arch = "wasm32"))]
    Real(real::Port),
    Closed,
}

struct Port {
    /// As the report names it, whole: `serial:COM3`.
    endpoint: String,
    /// What its messages show of it: its start (`endpoint::shown`).
    label: String,
    /// Nanoseconds: the timeout of its open, which each of its reads and writes keeps.
    timeout: i64,
    open: bool,
    backend: Backend,
}

impl Port {
    fn shut(&mut self) {
        self.open = false;
        self.backend = Backend::Closed;
    }

    /// Whether a real call found the port gone, after which it is closed.
    #[cfg(not(target_arch = "wasm32"))]
    fn broken(&self) -> bool {
        match &self.backend {
            Backend::Real(port) => port.broken(),
            Backend::Simulated(_) | Backend::Closed => false,
        }
    }

    fn closed(&self) -> NativeOutcome {
        closed(&self.label)
    }
}

/// `io.closed` of a port closed.
fn closed(label: &str) -> NativeOutcome {
    failed(CLOSED, format!("{label}: the port is closed"))
}

/// A call in progress, and what its `Blocking` work returned.
struct SerialCall {
    body: Body,
    done: Option<Box<dyn Any + Send>>,
}

enum Body {
    List(ListPhase),
    Open(Box<OpenCall>),
    Close(CloseCall),
    Configure(ConfigureCall),
    Transfer(TransferCall),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ListPhase {
    Start,
    /// The system lists its ports.
    #[cfg_attr(target_arch = "wasm32", expect(dead_code, reason = "the system lists its ports outside WASM only"))]
    Listing,
}

struct OpenCall {
    name: String,
    /// Its settings, or why they are wrong, said after its name.
    settings: Result<Settings, String>,
    /// Nanoseconds.
    timeout: i64,
    /// Nanoseconds.
    settle: i64,
    phase: OpenPhase,
}

enum OpenPhase {
    Start,
    /// A file is the whole bench and does not describe the endpoint: the open waits for its timeout.
    NotFound(Endpoint),
    /// The question of io.md 4 waits for its answer.
    #[cfg_attr(target_arch = "wasm32", expect(dead_code, reason = "a real open only, which WASM has not"))]
    Asking(Endpoint, Settings),
    /// The real open runs.
    #[cfg_attr(target_arch = "wasm32", expect(dead_code, reason = "a real open only, which WASM has not"))]
    Opening(Endpoint),
    /// The port is open, and waits `settle`.
    Settling(Handle),
    /// The input of a real port is emptied.
    #[cfg_attr(target_arch = "wasm32", expect(dead_code, reason = "a real port only, which WASM has not"))]
    Emptying(Handle),
}

struct CloseCall {
    handle: Handle,
    /// It holds the key of what it closes.
    held: bool,
}

/// `configure`: new settings for an open port.
struct ConfigureCall {
    handle: Handle,
    /// Its settings, or why they are wrong, said after the name of the port.
    settings: Result<Settings, String>,
    phase: ConfigurePhase,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ConfigurePhase {
    Start,
    /// It holds the key of its port.
    Held,
    /// The real work runs.
    #[cfg_attr(target_arch = "wasm32", expect(dead_code, reason = "a real port only, which WASM has not"))]
    Working,
}

/// A write or a read.
struct TransferCall {
    handle: Handle,
    /// What it does, or why its arguments are wrong, said once it holds its port.
    op: Result<Op, Failure>,
    phase: TransferPhase,
    /// When it started, on the clock of the run: a read waits up to the timeout from there.
    started: i64,
    /// Where the search of `read_until` and `read_line` stopped, between the wakes of a simulated read.
    scan: Scan,
    #[cfg(not(target_arch = "wasm32"))]
    since: Option<std::time::Instant>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TransferPhase {
    Start,
    /// It holds the key of its port.
    Held,
    /// It waits on the clock of the run for a simulated device.
    Simulated,
    /// Its real work runs.
    #[cfg_attr(target_arch = "wasm32", expect(dead_code, reason = "a real port only, which WASM has not"))]
    Working,
}

/// What the real open returned.
#[cfg(not(target_arch = "wasm32"))]
struct Opened(Result<Backend, Failure>);

/// What the real list returned.
#[cfg(not(target_arch = "wasm32"))]
struct Listed(Result<Vec<String>, Failure>);

/// What the real work of `configure` returned.
#[cfg(not(target_arch = "wasm32"))]
struct Configured(Result<(), Failure>);

/// What the real work of a read or a write returned.
#[cfg(not(target_arch = "wasm32"))]
enum Worked {
    Wrote(Result<(), Failure>),
    Read(Result<Taken, Failure>),
}

/// One run of `serial`.
pub struct SerialRun {
    library: u32,
    devices: Option<Arc<Devices>>,
    mode: Mode,
    context: Context,
    ports: Vec<Port>,
    calls: HashMap<CallId, SerialCall>,
    next_call: u64,
    log: EndpointLogs,
    /// The endpoints of the ports open in this run, and of the real opens in progress: a second open is `serial.port_busy`.
    taken: HashSet<String>,
}

impl SerialRun {
    pub fn new(library: u32, devices: Option<Arc<Devices>>, budget: LogBudget, mode: Mode, context: Context) -> Self {
        SerialRun {
            library,
            devices,
            mode,
            context,
            ports: Vec::new(),
            calls: HashMap::new(),
            next_call: 0,
            log: EndpointLogs::new(budget),
            taken: HashSet::new(),
        }
    }

    /// The port of a handle of this run: numbers start at 1, so the default, numbered 0, is none.
    fn slot(&self, handle: Handle) -> Option<usize> {
        if handle.library != self.library {
            return None;
        }
        usize::try_from(handle.number).ok()?.checked_sub(1).filter(|index| *index < self.ports.len())
    }

    fn port_mut(&mut self, handle: Handle) -> Option<&mut Port> {
        let index = self.slot(handle)?;
        self.ports.get_mut(index)
    }

    fn key(&self, handle: Handle) -> Key {
        Key { library: self.library, id: handle.number }
    }

    /// The port, open; else `io.closed` for one closed or never opened, whose message says which.
    fn open_index(&self, handle: Handle) -> Result<usize, NativeOutcome> {
        let index = self.slot(handle).ok_or_else(never_opened)?;
        match self.ports.get(index) {
            Some(port) if port.open => Ok(index),
            Some(port) => Err(port.closed()),
            None => Err(never_opened()),
        }
    }

    fn register(&mut self, endpoint: &Endpoint, timeout: i64, backend: Backend) -> Handle {
        self.taken.insert(endpoint.text.clone());
        self.ports.push(Port { endpoint: endpoint.text.clone(), label: endpoint.shown(), timeout, open: true, backend });
        Handle { library: self.library, number: u64::try_from(self.ports.len()).unwrap_or(u64::MAX) }
    }

    /// Closes the port of a handle, which frees its endpoint.
    fn shut(&mut self, handle: Handle) {
        if let Some(index) = self.slot(handle)
            && let Some(port) = self.ports.get_mut(index)
        {
            if port.open {
                self.taken.remove(&port.endpoint);
            }
            port.shut();
        }
    }

    fn advance(&mut self, id: CallId, cx: CallContext) -> Step {
        let Some(mut call) = self.calls.remove(&id) else {
            return Step::Done(failed(CLOSED, "a call that is not in progress".to_owned()));
        };
        let done = call.done.take();
        let step = match &mut call.body {
            Body::List(phase) => self.list(phase, done),
            Body::Open(open) => self.open(open, done, cx),
            Body::Close(close) => self.close(close),
            Body::Configure(configure) => self.configure(configure, done, cx),
            Body::Transfer(transfer) => self.transfer(transfer, done, cx),
        };
        if !matches!(step, Step::Done(_)) {
            self.calls.insert(id, call);
        }
        step
    }

    /// `list` (decisions of the owner, 2026-11-20): on a host without serial ports `io.unsupported`, with `--sim` too; with
    /// `--sim`, the serial endpoints of the whole file, never the system; else the ports of the system, with no consent.
    #[cfg_attr(target_arch = "wasm32", expect(unused_variables, reason = "the system lists its ports outside WASM only"))]
    fn list(&mut self, phase: &mut ListPhase, done: Option<Box<dyn Any + Send>>) -> Step {
        if cfg!(target_arch = "wasm32") {
            return Step::Done(io::unsupported("this host has no serial ports"));
        }
        if self.context.simulation_file {
            // The endpoints of the file are sorted and each once: without their prefix, the ports are too.
            let ports = self.context.endpoints.iter().filter_map(|endpoint| endpoint.strip_prefix("serial:")).map(str::to_owned).collect();
            return Step::Done(listed(ports));
        }
        #[cfg(not(target_arch = "wasm32"))]
        if *phase == ListPhase::Start {
            *phase = ListPhase::Listing;
            return Step::Blocking(Box::new(|| Box::new(Listed(real::list())) as Box<dyn Any + Send>));
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(listing) = done.and_then(|done| done.downcast::<Listed>().ok()) {
            return Step::Done(match listing.0 {
                Ok(ports) => listed(ports),
                Err(failure) => failure.outcome(),
            });
        }
        Step::Done(io::unsupported("the list of the ports did not end"))
    }

    #[cfg_attr(target_arch = "wasm32", expect(unused_variables, reason = "a real open only, which WASM has not, does work"))]
    fn open(&mut self, call: &mut OpenCall, done: Option<Box<dyn Any + Send>>, cx: CallContext) -> Step {
        match std::mem::replace(&mut call.phase, OpenPhase::Start) {
            OpenPhase::Start => {
                let endpoint = match endpoint::of_call(&call.name) {
                    Ok(endpoint) => endpoint,
                    Err(outcome) => return Step::Done(outcome),
                };
                let busy = self.taken.contains(&endpoint.text);
                let settings = match first_checks(&endpoint, &call.settings, cfg!(target_arch = "wasm32"), busy) {
                    Ok(settings) => settings,
                    Err(outcome) => return Step::Done(outcome),
                };
                let device = self.devices.as_ref().and_then(|devices| devices.find(&endpoint.text));
                match open_real(self.mode, &self.context, device.is_some(), &endpoint.reach()) {
                    Open::Simulated => match device {
                        Some(spec) => {
                            let handle = self.open_simulated(&endpoint, call.timeout, settings, spec, cx.now);
                            self.settle(call, handle, cx.now)
                        }
                        None => Step::Done(self.not_found(&endpoint)),
                    },
                    Open::NotFound => {
                        call.phase = OpenPhase::NotFound(endpoint);
                        Step::Sleep(call.timeout)
                    }
                    Open::Fail(outcome) => Step::Done(outcome),
                    #[cfg(not(target_arch = "wasm32"))]
                    Open::Ask(job) => {
                        self.taken.insert(endpoint.text.clone());
                        call.phase = OpenPhase::Asking(endpoint, settings);
                        Step::Blocking(job)
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    Open::Real => {
                        self.taken.insert(endpoint.text.clone());
                        open_work(call, endpoint, settings)
                    }
                    // `io.unsupported` came first.
                    #[cfg(target_arch = "wasm32")]
                    Open::Ask(_) | Open::Real => Step::Done(io::unsupported(&endpoint.shown())),
                }
            }
            OpenPhase::NotFound(endpoint) => Step::Done(self.not_found(&endpoint)),
            #[cfg(not(target_arch = "wasm32"))]
            OpenPhase::Asking(endpoint, settings) => match self.context.answered(&endpoint.reach(), done.unwrap_or_else(|| Box::new(()))) {
                Ok(()) => open_work(call, endpoint, settings),
                Err(outcome) => {
                    self.taken.remove(&endpoint.text);
                    Step::Done(outcome)
                }
            },
            #[cfg(not(target_arch = "wasm32"))]
            OpenPhase::Opening(endpoint) => match done.and_then(|done| done.downcast::<Opened>().ok()).map(|opened| opened.0) {
                Some(Ok(backend)) => {
                    let handle = self.register(&endpoint, call.timeout, backend);
                    self.settle(call, handle, cx.now)
                }
                Some(Err(failure)) => {
                    self.taken.remove(&endpoint.text);
                    Step::Done(failure.outcome())
                }
                None => {
                    self.taken.remove(&endpoint.text);
                    Step::Done(failed(PORT_NOT_FOUND, format!("{}: the open did not end", endpoint.shown())))
                }
            },
            OpenPhase::Settling(handle) => self.empty(call, handle, cx.now),
            #[cfg(not(target_arch = "wasm32"))]
            OpenPhase::Emptying(handle) => Step::Done(opened(handle)),
            #[cfg(target_arch = "wasm32")]
            OpenPhase::Asking(..) | OpenPhase::Opening(_) | OpenPhase::Emptying(_) => Step::Done(io::unsupported("serial")),
        }
    }

    /// A device of the file. Opened with settings it does not expect, it gets no response from it, and the report says which
    /// differ (io.md 6; decision of the owner, 2026-11-20).
    fn open_simulated(&mut self, endpoint: &Endpoint, timeout: i64, settings: Settings, spec: Arc<DeviceSpec>, now: i64) -> Handle {
        let deaf = judge(&mut self.log, &endpoint.text, now, &spec, &settings);
        self.register(endpoint, timeout, Backend::Simulated(Simulated { spec, stream: Device::default(), deaf }))
    }

    /// After the open: waits `settle` on the clock of the run, then empties the input.
    fn settle(&mut self, call: &mut OpenCall, handle: Handle, now: i64) -> Step {
        if call.settle > 0 {
            call.phase = OpenPhase::Settling(handle);
            return Step::Sleep(call.settle);
        }
        self.empty(call, handle, now)
    }

    /// Empties the input, then gives the port: a simulated device drops what is ready; a real port discards what the system
    /// holds, as `Blocking` work.
    #[cfg_attr(target_arch = "wasm32", expect(unused_variables, reason = "a real port only, which WASM has not, sets the phase"))]
    fn empty(&mut self, call: &mut OpenCall, handle: Handle, now: i64) -> Step {
        let Some(port) = self.port_mut(handle) else { return Step::Done(never_opened()) };
        match &mut port.backend {
            Backend::Simulated(device) => {
                let ready = device.stream.ready_len(now);
                device.stream.take(ready);
                Step::Done(opened(handle))
            }
            #[cfg(not(target_arch = "wasm32"))]
            Backend::Real(real) => {
                let shared = real.handle();
                call.phase = OpenPhase::Emptying(handle);
                Step::Blocking(Box::new(move || {
                    real::discard(&shared);
                    Box::new(()) as Box<dyn Any + Send>
                }))
            }
            // Released during `settle`, by an abort.
            Backend::Closed => Step::Done(closed(&port.label)),
        }
    }

    /// With `--sim`, the file is the whole bench (io.md 3): a name it does not describe is not found, never reached.
    fn not_found(&self, endpoint: &Endpoint) -> NativeOutcome {
        stream::not_found(PORT_NOT_FOUND, &endpoint.shown(), "serial", self.devices.is_some())
    }

    /// `close` succeeds on a port closed already, or never opened; on one that is open, it waits for the call that holds it.
    fn close(&mut self, call: &mut CloseCall) -> Step {
        if call.held {
            self.shut(call.handle);
            return Step::Done(NativeOutcome::Outputs(vec![Value::NONE]));
        }
        if self.open_index(call.handle).is_err() {
            return Step::Done(NativeOutcome::Outputs(vec![Value::NONE]));
        }
        call.held = true;
        Step::Exclusive(self.key(call.handle))
    }

    /// `configure`: closed or never opened gives `io.closed`; then, holding the key of the port, a setting out of range gives
    /// `io.invalid_argument`, before anything is changed, and a simulated device takes the settings at once (it judges them at
    /// each write; one it does not expect is said in the report, as an open's are, and takes no time on the clock), and a real
    /// port has them set by the system, as `Blocking` work, without closing it.
    #[cfg_attr(target_arch = "wasm32", expect(unused_variables, reason = "a real port only, which WASM has not, does work"))]
    fn configure(&mut self, call: &mut ConfigureCall, done: Option<Box<dyn Any + Send>>, cx: CallContext) -> Step {
        let index = match self.open_index(call.handle) {
            Ok(index) => index,
            Err(outcome) => return Step::Done(outcome),
        };
        if call.phase == ConfigurePhase::Start {
            call.phase = ConfigurePhase::Held;
            return Step::Exclusive(self.key(call.handle));
        }
        let Some(port) = self.ports.get_mut(index) else { return Step::Done(never_opened()) };
        #[cfg(not(target_arch = "wasm32"))]
        if call.phase == ConfigurePhase::Working {
            let outcome = match done.and_then(|done| done.downcast::<Configured>().ok()).map(|configured| configured.0) {
                Some(Ok(())) => opened(call.handle),
                Some(Err(failure)) => failure.outcome(),
                None => failed(CLOSED, format!("{}: the work of the call did not end", port.label)),
            };
            // A port gone (a board unplugged) is closed, as after a read or a write.
            if port.broken() {
                self.shut(call.handle);
            }
            return Step::Done(outcome);
        }
        // Judged once the call holds the port, as a write is: a port that the call before closed gives `io.closed`.
        let settings = match &call.settings {
            Ok(settings) => *settings,
            Err(message) => return Step::Done(failed(INVALID_ARGUMENT, format!("{}: {message}", port.label))),
        };
        match &mut port.backend {
            Backend::Simulated(device) => {
                device.deaf = judge(&mut self.log, &port.endpoint, cx.now, &device.spec, &settings);
                Step::Done(opened(call.handle))
            }
            #[cfg(not(target_arch = "wasm32"))]
            Backend::Real(real) => {
                let shared = real.handle();
                call.phase = ConfigurePhase::Working;
                Step::Blocking(Box::new(move || Box::new(Configured(real::reconfigure(&shared, settings))) as Box<dyn Any + Send>))
            }
            Backend::Closed => Step::Done(closed(&port.label)),
        }
    }

    /// A write or a read.
    #[cfg_attr(target_arch = "wasm32", expect(unused_variables, reason = "a real port only, which WASM has not, does work"))]
    fn transfer(&mut self, call: &mut TransferCall, done: Option<Box<dyn Any + Send>>, cx: CallContext) -> Step {
        if call.phase == TransferPhase::Start {
            call.started = cx.now;
            #[cfg(not(target_arch = "wasm32"))]
            #[expect(clippy::disallowed_methods, reason = "never built for WASM, where the clock traps")]
            {
                call.since = Some(std::time::Instant::now());
            }
            if let Err(outcome) = self.open_index(call.handle) {
                return Step::Done(outcome);
            }
            call.phase = TransferPhase::Held;
            return Step::Exclusive(self.key(call.handle));
        }
        // Another call may have closed it while this one waited for its key, or its device.
        let index = match self.open_index(call.handle) {
            Ok(index) => index,
            Err(outcome) => return Step::Done(outcome),
        };
        let op = match &call.op {
            Ok(op) => op,
            Err(failure) => return Step::Done(failure.clone().outcome()),
        };
        let Some(port) = self.ports.get_mut(index) else { return Step::Done(never_opened()) };
        let names = Names { endpoint: &port.endpoint, label: &port.label };
        #[cfg(not(target_arch = "wasm32"))]
        if call.phase == TransferPhase::Working {
            let outcome = match done.and_then(|done| done.downcast::<Worked>().ok()).map(|worked| *worked) {
                Some(Worked::Wrote(Ok(()))) => finished(&mut self.log, names, call.handle, op, cx.now, None),
                Some(Worked::Read(Ok(taken))) => finished(&mut self.log, names, call.handle, op, cx.now, Some(taken)),
                Some(Worked::Wrote(Err(failure)) | Worked::Read(Err(failure))) => failure.outcome(),
                None => failed(CLOSED, format!("{}: the work of the call did not end", names.label)),
            };
            // A port gone (a board unplugged) is closed: `describe` says so, and the next call gets `io.closed`.
            if port.broken() {
                self.shut(call.handle);
            }
            return Step::Done(outcome);
        }
        if call.phase == TransferPhase::Held {
            #[cfg(not(target_arch = "wasm32"))]
            if let Backend::Real(real) = &port.backend {
                call.phase = TransferPhase::Working;
                return Step::Blocking(real_job(real, op, call));
            }
            call.phase = TransferPhase::Simulated;
        }
        let timeout = port.timeout;
        let Backend::Simulated(device) = &mut port.backend else { return Step::Done(closed(&port.label)) };
        let names = Names { endpoint: &port.endpoint, label: &port.label };
        let spec = &device.spec.exchanges;
        let next = match op {
            // A device opened with other settings takes the bytes and makes nothing of them: they are none to it.
            Op::Write(data) => write_step(&mut device.stream, spec, names.label, cx.now, if device.deaf { &[] } else { data }),
            Op::Read { need, .. } => {
                let clock = Clock { now: cx.now, deadline: call.started.saturating_add(timeout), timeout };
                read_step(&mut device.stream, spec, need, &mut call.scan, names.label, clock)
            }
        };
        match next {
            Next::Sleep(ns) => Step::Sleep(ns.max(0)),
            Next::Wrote => Step::Done(finished(&mut self.log, names, call.handle, op, cx.now, None)),
            Next::Read(taken) => Step::Done(finished(&mut self.log, names, call.handle, op, cx.now, Some(taken))),
            Next::Failed(failure) => Step::Done(failure.outcome()),
        }
    }
}

/// A write that went (`taken` none) or a read that ended, as `runtime::stream` says it.
fn finished(log: &mut EndpointLogs, names: Names<'_>, handle: Handle, op: &Op, now: i64, taken: Option<Taken>) -> NativeOutcome {
    match (op, taken) {
        (_, Some(taken)) => read_done(log, names, handle, op.stripped(), now, taken),
        (Op::Write(data), None) => wrote(log, names, handle, data, now),
        (Op::Read { .. }, None) => NativeOutcome::Outputs(vec![Value::Handle(handle), Value::NONE]),
    }
}

/// The checks of an open after its name, in the order of io.md 3: a host without serial ports (`unsupported`) gives
/// `io.unsupported`, then a setting out of range `io.invalid_argument`, then a port open or opening in this run (`busy`)
/// `serial.port_busy`; else the settings, for the table of what a run reaches.
pub(crate) fn first_checks(
    endpoint: &Endpoint,
    settings: &Result<Settings, String>,
    unsupported: bool,
    busy: bool,
) -> Result<Settings, NativeOutcome> {
    if unsupported {
        return Err(io::unsupported(&format!("{}: this host has no serial ports", endpoint.shown())));
    }
    let settings = settings.clone().map_err(|message| failed(INVALID_ARGUMENT, format!("{}: {message}", endpoint.shown())))?;
    if busy {
        return Err(failed(PORT_BUSY, format!("{}: the port is open already in this run", endpoint.shown())));
    }
    Ok(settings)
}

/// The outputs of an open, and of a `configure`: the port, no error.
fn opened(handle: Handle) -> NativeOutcome {
    NativeOutcome::Outputs(vec![Value::Handle(handle), Value::NONE])
}

/// The outputs of `list`.
fn listed(ports: Vec<String>) -> NativeOutcome {
    let items = ports.into_iter().map(Value::string).collect();
    NativeOutcome::Outputs(vec![Value::Array(Arc::new(Items::new(items))), Value::NONE])
}

/// `io.closed` for a handle never opened, the default value of its type.
fn never_opened() -> NativeOutcome {
    stream::never_opened("port", "serial.port")
}

/// The real open, as `Blocking` work.
#[cfg(not(target_arch = "wasm32"))]
fn open_work(call: &mut OpenCall, endpoint: Endpoint, settings: Settings) -> Step {
    let (name, label) = (endpoint.port().to_owned(), endpoint.shown());
    let timeout = std::time::Duration::from_nanos(u64::try_from(call.timeout).unwrap_or(0));
    call.phase = OpenPhase::Opening(endpoint);
    Step::Blocking(Box::new(move || Box::new(Opened(real::Port::open(&name, settings, timeout, &label).map(Backend::Real))) as Box<dyn Any + Send>))
}

/// The real work of a read or a write.
#[cfg(not(target_arch = "wasm32"))]
#[expect(clippy::disallowed_methods, reason = "never built for WASM, where the clock traps")]
fn real_job(port: &real::Port, op: &Op, call: &TransferCall) -> labcraft_runtime::native::Job {
    let since = call.since.unwrap_or_else(std::time::Instant::now);
    let shared = port.handle();
    let work: Box<dyn FnOnce() -> Worked + Send> = match op {
        Op::Write(data) => {
            let data = Arc::clone(data);
            Box::new(move || Worked::Wrote(real::write(&shared, &data, since)))
        }
        Op::Read { need, .. } => {
            let need = need.clone();
            Box::new(move || Worked::Read(real::read(&shared, &need, since)))
        }
    };
    Box::new(move || Box::new(work()) as Box<dyn Any + Send>)
}

/// The settings of `open(port, baud, data_bits, parity, stop_bits, flow, …)`, or of `configure(port, …)` with the same
/// places: `baud` above 0 and `data_bits` 5 to 8 (`libs/serial/README.md`, `open`), else why not.
fn settings_of(inputs: &[Value]) -> Result<Settings, String> {
    let variant = |at: usize| match inputs.get(at) {
        Some(Value::Variant(variant)) => variant.index,
        _ => 0,
    };
    let baud = match inputs.get(1) {
        Some(Value::U32(baud)) => *baud,
        _ => 0,
    };
    let data_bits = match inputs.get(2) {
        Some(Value::U8(bits)) => *bits,
        _ => 0,
    };
    if baud == 0 {
        return Err("a baud rate of 0: `baud` is above 0".to_owned());
    }
    if !DATA_BITS.contains(&data_bits) {
        return Err(format!("`data_bits` is 5 to 8, and this is {data_bits}"));
    }
    let parity = Parity::of_index(variant(3)).ok_or("no such `serial.Parity`")?;
    let stop_bits = StopBits::of_index(variant(4)).ok_or("no such `serial.StopBits`")?;
    let flow = Flow::of_index(variant(5)).ok_or("no such `serial.Flow`")?;
    Ok(Settings { baud, data_bits, parity, stop_bits, flow })
}

impl LibraryRun for SerialRun {
    fn call(&mut self, function: &str, inputs: Vec<Value>) -> Option<CallId> {
        let duration = |at: usize| match inputs.get(at) {
            Some(Value::Duration(ns)) => (*ns).max(0),
            _ => 0,
        };
        let handle = match inputs.first() {
            Some(Value::Handle(handle)) => *handle,
            _ => Handle::INVALID,
        };
        let body = match function {
            "list" => Body::List(ListPhase::Start),
            "open" => Body::Open(Box::new(OpenCall {
                name: match inputs.first() {
                    Some(Value::String(text)) => text.to_string(),
                    _ => String::new(),
                },
                settings: settings_of(&inputs),
                timeout: duration(6),
                settle: duration(7),
                phase: OpenPhase::Start,
            })),
            "close" => Body::Close(CloseCall { handle, held: false }),
            "configure" => Body::Configure(ConfigureCall { handle, settings: settings_of(&inputs), phase: ConfigurePhase::Start }),
            "write" | "write_line" | "read" | "read_until" | "read_line" | "read_available" => Body::Transfer(TransferCall {
                handle,
                op: stream::op_of(function, &inputs),
                phase: TransferPhase::Start,
                started: 0,
                scan: Scan::default(),
                #[cfg(not(target_arch = "wasm32"))]
                since: None,
            }),
            _ => return None,
        };
        let id = CallId(self.next_call);
        self.next_call = self.next_call.saturating_add(1);
        self.calls.insert(id, SerialCall { body, done: None });
        Some(id)
    }

    fn step(&mut self, call: CallId, cx: CallContext) -> Step {
        self.advance(call, cx)
    }

    fn resume(&mut self, call: CallId, done: Box<dyn Any + Send>) {
        if let Some(call) = self.calls.get_mut(&call) {
            call.done = Some(done);
        }
    }

    /// A call the run abandons: an open in progress frees its endpoint, and one that had opened its port, which it had not
    /// given yet (during `settle`), closes it, since nothing else would.
    fn cancel(&mut self, call: CallId) {
        let Some(SerialCall { body: Body::Open(open), .. }) = self.calls.remove(&call) else { return };
        match open.phase {
            OpenPhase::Asking(endpoint, _) | OpenPhase::Opening(endpoint) => {
                self.taken.remove(&endpoint.text);
            }
            OpenPhase::Settling(handle) | OpenPhase::Emptying(handle) => self.shut(handle),
            OpenPhase::Start | OpenPhase::NotFound(_) => {}
        }
    }

    fn describe(&self, handle: Handle) -> Option<HandleInfo> {
        let port = self.ports.get(self.slot(handle)?)?;
        Some(HandleInfo { kind: "port", designates: port.endpoint.clone(), open: port.open })
    }

    /// Closes what the run leaves open after an abort or a fault (README, "What `serial` guarantees").
    fn release(&mut self, handle: Handle) {
        self.shut(handle);
    }

    fn instruments(&self) -> Vec<InstrumentLog> {
        self.log.instruments()
    }
}
