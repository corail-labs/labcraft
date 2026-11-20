//! What one run of `net` holds: its connections and sockets, and each call in progress as a state machine
//! (`docs/architecture.md`, 10.6). A call takes the key of its connection or socket for all of its length, so it is atomic on
//! it (`libs/net/README.md`, "What `net` guarantees"): a read that waits for its timeout holds it. An open follows the order
//! of io.md 3: the name, then `io.unsupported`, then the table of what a run reaches, then the right of the run, then the
//! open of the hardware.

use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

use labcraft_runtime::io::{self, CLOSED, Open, open_real};
use labcraft_runtime::native::{
    CallContext, CallId, Context, EndpointLogs, HandleInfo, InstrumentLog, Job, Key, LibraryRun, LogBudget, Mode, NativeOutcome, Step,
};
use labcraft_runtime::stream::{self, Clock, Failure, Names, Next, Scan, Taken, read_step, write_step};
use labcraft_runtime::{Handle, Value};

use crate::device::Simulated;
use crate::endpoint::{self, Endpoint, HOST_NOT_FOUND};
use crate::file::{DeviceSpec, Devices};
#[cfg(not(target_arch = "wasm32"))]
use crate::real;

pub(crate) const CONNECTION_REFUSED: &str = "net.connection_refused";
pub(crate) const DATAGRAM_TOO_LONG: &str = "net.datagram_too_long";
/// The largest datagram UDP carries over IPv4.
const MAX_DATAGRAM: usize = 65_507;

fn failed(category: &'static str, message: String) -> NativeOutcome {
    NativeOutcome::Fail { category, code: 0, message }
}

/// A TCP connection or a UDP socket: two opaque types, so that a wire from one into a function of the other is refused when
/// it is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Transport {
    Tcp,
    Udp,
}

impl Transport {
    fn udp(self) -> bool {
        self == Transport::Udp
    }

    /// Its opaque type, as the manifest names it.
    fn kind(self) -> &'static str {
        match self {
            Transport::Tcp => "connection",
            Transport::Udp => "udp_socket",
        }
    }

    /// What a message calls one.
    fn noun(self) -> &'static str {
        match self {
            Transport::Tcp => "connection",
            Transport::Udp => "socket",
        }
    }
}

/// What a connection or a socket talks to.
enum Backend {
    Simulated(Simulated),
    #[cfg(not(target_arch = "wasm32"))]
    Tcp(real::Tcp),
    #[cfg(not(target_arch = "wasm32"))]
    Udp(real::Udp),
    Closed,
}

struct Connection {
    /// As the report names it, whole.
    endpoint: String,
    /// What its messages show of it: its start (`endpoint::shown`).
    label: String,
    transport: Transport,
    /// Nanoseconds: the timeout of its open, which each of its reads and writes keeps.
    timeout: i64,
    open: bool,
    backend: Backend,
}

impl Connection {
    /// Closes it; a real socket is shut down.
    fn shut(&mut self) {
        self.open = false;
        self.backend = Backend::Closed;
    }

    /// Whether a real call met a reset or an abort, after which it is closed.
    fn broken(&self) -> bool {
        match &self.backend {
            #[cfg(not(target_arch = "wasm32"))]
            Backend::Tcp(tcp) => tcp.broken(),
            #[cfg(not(target_arch = "wasm32"))]
            Backend::Udp(udp) => udp.broken(),
            Backend::Simulated(_) | Backend::Closed => false,
        }
    }

    fn closed(&self) -> NativeOutcome {
        failed(CLOSED, format!("{}: the {} is closed", self.label, self.transport.noun()))
    }
}

/// A call in progress, and what its `Blocking` work returned.
struct NetCall {
    body: Body,
    done: Option<Box<dyn Any + Send>>,
}

enum Body {
    Open(OpenCall),
    Close(CloseCall),
    Transfer(TransferCall),
}

/// `connect` or `open_udp`.
struct OpenCall {
    transport: Transport,
    host: String,
    port: u16,
    #[cfg_attr(target_arch = "wasm32", expect(dead_code, reason = "read by the real open only, which WASM has not"))]
    local_port: u16,
    /// Nanoseconds.
    timeout: i64,
    phase: OpenPhase,
}

enum OpenPhase {
    Start,
    /// A file is the whole bench and does not describe the endpoint: the open waits for its timeout.
    NotFound(Endpoint),
    /// The question of io.md 4 waits for its answer.
    Asking(Endpoint),
    /// The real open runs.
    Opening(Endpoint),
}

/// `close` or `close_udp`.
struct CloseCall {
    handle: Handle,
    transport: Transport,
    /// It holds the key of what it closes.
    held: bool,
}

/// A write, a send or a read.
struct TransferCall {
    handle: Handle,
    transport: Transport,
    /// What it does, or why its arguments are wrong, said once it holds its connection.
    op: Result<Op, Failure>,
    phase: TransferPhase,
    /// When it started, on the clock of the run: a read waits up to the timeout from there.
    started: i64,
    /// Where the search of `read_until` and `read_line` stopped, between the wakes of a simulated read.
    scan: Scan,
    #[cfg(not(target_arch = "wasm32"))]
    since: Option<std::time::Instant>,
}

enum Op {
    /// A read or a write of a TCP connection, as `runtime::stream` does it.
    Stream(stream::Op),
    Send(Arc<[u8]>),
    /// `receive`, with its `max`.
    Receive(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TransferPhase {
    Start,
    /// It holds the key of its connection or socket.
    Held,
    /// It waits on the clock of the run for a simulated device.
    Simulated,
    /// Its real work runs.
    Working,
}

/// What the real open returned.
struct Opened(Result<Backend, Failure>);

/// What the real work of a read or a write returned.
enum Worked {
    Wrote(Result<(), Failure>),
    Read(Result<Taken, Failure>),
}

/// One run of `net`.
pub struct NetRun {
    library: u32,
    devices: Option<Arc<Devices>>,
    mode: Mode,
    context: Context,
    connections: Vec<Connection>,
    calls: HashMap<CallId, NetCall>,
    next_call: u64,
    log: EndpointLogs,
}

impl NetRun {
    pub fn new(library: u32, devices: Option<Arc<Devices>>, budget: LogBudget, mode: Mode, context: Context) -> Self {
        NetRun { library, devices, mode, context, connections: Vec::new(), calls: HashMap::new(), next_call: 0, log: EndpointLogs::new(budget) }
    }

    /// The connection of a handle of this run, whatever its kind: numbers start at 1, so the default, numbered 0, is none.
    fn slot(&self, handle: Handle) -> Option<usize> {
        if handle.library != self.library {
            return None;
        }
        usize::try_from(handle.number).ok()?.checked_sub(1).filter(|index| *index < self.connections.len())
    }

    /// The connection of a handle, of the kind asked.
    fn index(&self, handle: Handle, transport: Transport) -> Option<usize> {
        self.slot(handle).filter(|index| self.connections.get(*index).is_some_and(|connection| connection.transport == transport))
    }

    fn connection_mut(&mut self, handle: Handle, transport: Transport) -> Option<&mut Connection> {
        let index = self.index(handle, transport)?;
        self.connections.get_mut(index)
    }

    fn key(&self, handle: Handle) -> Key {
        Key { library: self.library, id: handle.number }
    }

    /// The connection, open; else `io.closed` for one closed or never opened (decision of the owner, 2026-11-19), whose
    /// message says which.
    fn open_index(&self, handle: Handle, transport: Transport) -> Result<usize, NativeOutcome> {
        let index = self.index(handle, transport).ok_or_else(|| never_opened(transport))?;
        match self.connections.get(index) {
            Some(connection) if connection.open => Ok(index),
            Some(connection) => Err(connection.closed()),
            None => Err(never_opened(transport)),
        }
    }

    fn register(&mut self, endpoint: &Endpoint, transport: Transport, timeout: i64, backend: Backend) -> Handle {
        self.connections.push(Connection { endpoint: endpoint.text.clone(), label: endpoint.shown(), transport, timeout, open: true, backend });
        Handle { library: self.library, number: u64::try_from(self.connections.len()).unwrap_or(u64::MAX) }
    }

    fn advance(&mut self, id: CallId, cx: CallContext) -> Step {
        let Some(mut call) = self.calls.remove(&id) else {
            return Step::Done(failed(CLOSED, "a call that is not in progress".to_owned()));
        };
        let done = call.done.take();
        let step = match &mut call.body {
            Body::Open(open) => self.open(open, done),
            Body::Close(close) => self.close(close),
            Body::Transfer(transfer) => self.transfer(transfer, done, cx),
        };
        if !matches!(step, Step::Done(_)) {
            self.calls.insert(id, call);
        }
        step
    }

    fn open(&mut self, call: &mut OpenCall, done: Option<Box<dyn Any + Send>>) -> Step {
        match std::mem::replace(&mut call.phase, OpenPhase::Start) {
            OpenPhase::Start => {
                let endpoint = match endpoint::of_call(call.transport.udp(), &call.host, call.port) {
                    Ok(endpoint) => endpoint,
                    Err(outcome) => return Step::Done(outcome),
                };
                if cfg!(target_arch = "wasm32") {
                    return Step::Done(io::unsupported(&format!("{}: this host has no TCP or UDP", endpoint.shown())));
                }
                let device = self.devices.as_ref().and_then(|devices| devices.find(&endpoint.text));
                match open_real(self.mode, &self.context, device.is_some(), &endpoint.reach()) {
                    Open::Simulated => Step::Done(match device {
                        Some(spec) => self.open_simulated(call, &endpoint, spec),
                        None => self.not_found(&endpoint),
                    }),
                    Open::NotFound => {
                        call.phase = OpenPhase::NotFound(endpoint);
                        Step::Sleep(call.timeout)
                    }
                    Open::Fail(outcome) => Step::Done(outcome),
                    Open::Ask(job) => {
                        call.phase = OpenPhase::Asking(endpoint);
                        Step::Blocking(job)
                    }
                    Open::Real => open_work(call, endpoint),
                }
            }
            OpenPhase::NotFound(endpoint) => Step::Done(self.not_found(&endpoint)),
            OpenPhase::Asking(endpoint) => match self.context.answered(&endpoint.reach(), done.unwrap_or_else(|| Box::new(()))) {
                Ok(()) => open_work(call, endpoint),
                Err(outcome) => Step::Done(outcome),
            },
            OpenPhase::Opening(endpoint) => Step::Done(match done.and_then(|done| done.downcast::<Opened>().ok()).map(|opened| opened.0) {
                Some(Ok(backend)) => {
                    let handle = self.register(&endpoint, call.transport, call.timeout, backend);
                    NativeOutcome::Outputs(vec![Value::Handle(handle), Value::NONE])
                }
                Some(Err(failure)) => failure.outcome(),
                None => failed(CONNECTION_REFUSED, format!("{}: the open did not end", endpoint.shown())),
            }),
        }
    }

    /// A device of the file: a TCP device with `refused` refuses every connect, code 0; one with `close_ms` past, or `hang_ms`
    /// past, still connects (decision of the owner, 2026-11-19), and its connection is closed, or never answers.
    fn open_simulated(&mut self, call: &OpenCall, endpoint: &Endpoint, spec: Arc<DeviceSpec>) -> NativeOutcome {
        if !spec.udp && spec.refused {
            return failed(CONNECTION_REFUSED, format!("{}: the device refuses the connection (`faults.refused`)", endpoint.shown()));
        }
        let handle = self.register(endpoint, call.transport, call.timeout, Backend::Simulated(Simulated::new(spec)));
        NativeOutcome::Outputs(vec![Value::Handle(handle), Value::NONE])
    }

    /// With `--sim`, the file is the whole bench (io.md 3): a name it does not describe is not found, never reached.
    fn not_found(&self, endpoint: &Endpoint) -> NativeOutcome {
        stream::not_found(HOST_NOT_FOUND, &endpoint.shown(), "net", self.devices.is_some())
    }

    /// `close` and `close_udp` succeed on what is closed already, or was never opened; on what is open, they wait for the call
    /// that holds it.
    fn close(&mut self, call: &mut CloseCall) -> Step {
        if call.held {
            if let Some(connection) = self.connection_mut(call.handle, call.transport) {
                connection.shut();
            }
            return Step::Done(NativeOutcome::Outputs(vec![Value::NONE]));
        }
        if self.open_index(call.handle, call.transport).is_err() {
            return Step::Done(NativeOutcome::Outputs(vec![Value::NONE]));
        }
        call.held = true;
        Step::Exclusive(self.key(call.handle))
    }

    /// A write, a send or a read.
    fn transfer(&mut self, call: &mut TransferCall, done: Option<Box<dyn Any + Send>>, cx: CallContext) -> Step {
        if call.phase == TransferPhase::Start {
            call.started = cx.now;
            #[cfg(not(target_arch = "wasm32"))]
            #[expect(clippy::disallowed_methods, reason = "never built for WASM, where the clock traps")]
            {
                call.since = Some(std::time::Instant::now());
            }
            if let Err(outcome) = self.open_index(call.handle, call.transport) {
                return Step::Done(outcome);
            }
            call.phase = TransferPhase::Held;
            return Step::Exclusive(self.key(call.handle));
        }
        // Another call may have closed it while this one waited for its key, or its device.
        let index = match self.open_index(call.handle, call.transport) {
            Ok(index) => index,
            Err(outcome) => return Step::Done(outcome),
        };
        let op = match &call.op {
            Ok(op) => op,
            Err(failure) => return Step::Done(failure.clone().outcome()),
        };
        let Some(connection) = self.connections.get_mut(index) else { return Step::Done(never_opened(call.transport)) };
        let names = Names { endpoint: &connection.endpoint, label: &connection.label };
        if call.phase == TransferPhase::Working {
            let outcome = match done.and_then(|done| done.downcast::<Worked>().ok()).map(|worked| *worked) {
                Some(Worked::Wrote(Ok(()))) => wrote(&mut self.log, names, call.handle, op, cx.now),
                Some(Worked::Read(Ok(taken))) => read_done(&mut self.log, names, call.handle, op, cx.now, taken),
                Some(Worked::Wrote(Err(failure)) | Worked::Read(Err(failure))) => failure.outcome(),
                None => failed(CLOSED, format!("{}: the work of the call did not end", names.label)),
            };
            // A reset or an abort closes it (decision of the owner, 2026-11-19): `describe` says so, and the next call gets
            // `io.closed`.
            if connection.broken() {
                connection.shut();
            }
            return Step::Done(outcome);
        }
        if call.phase == TransferPhase::Held {
            if let Op::Send(data) = op
                && data.len() > MAX_DATAGRAM
            {
                let message = format!("{}: a datagram of {} bytes, more than {MAX_DATAGRAM}: nothing is sent", names.label, data.len());
                return Step::Done(failed(DATAGRAM_TOO_LONG, message));
            }
            if !matches!(connection.backend, Backend::Simulated(_)) {
                call.phase = TransferPhase::Working;
                return match real_job(&connection.backend, op, call) {
                    Some(job) => Step::Blocking(job),
                    None => Step::Done(connection.closed()),
                };
            }
            call.phase = TransferPhase::Simulated;
        }
        let timeout = connection.timeout;
        let Backend::Simulated(device) = &mut connection.backend else {
            return Step::Done(failed(CLOSED, format!("{}: the {} is closed", connection.label, connection.transport.noun())));
        };
        let names = Names { endpoint: &connection.endpoint, label: &connection.label };
        let clock = Clock { now: cx.now, deadline: call.started.saturating_add(timeout), timeout };
        match simulated(device, op, &mut call.scan, names.label, clock) {
            Next::Sleep(ns) => Step::Sleep(ns.max(0)),
            Next::Wrote => Step::Done(wrote(&mut self.log, names, call.handle, op, cx.now)),
            Next::Read(taken) => Step::Done(read_done(&mut self.log, names, call.handle, op, cx.now, taken)),
            Next::Failed(failure) => Step::Done(failure.outcome()),
        }
    }
}

/// `io.closed` for a handle never opened, the default value of its type.
fn never_opened(transport: Transport) -> NativeOutcome {
    stream::never_opened(transport.noun(), &format!("net.{}", transport.kind()))
}

/// One step of a call on a simulated device: over TCP, that of a stream (`runtime::stream`); over UDP, a datagram.
fn simulated(device: &mut Simulated, op: &Op, scan: &mut Scan, endpoint: &str, clock: Clock) -> Next {
    let Clock { now, deadline, timeout } = clock;
    match op {
        Op::Stream(stream::Op::Write(data)) => write_step(&mut device.stream, &device.spec.exchanges, endpoint, now, data),
        Op::Stream(stream::Op::Read { need, .. }) => read_step(&mut device.stream, &device.spec.exchanges, need, scan, endpoint, clock),
        Op::Send(data) => {
            device.send(now, data);
            Next::Wrote
        }
        Op::Receive(max) => match device.datagram(now) {
            Some(datagram) if datagram.len() > *max => Next::Read(Taken::DatagramTooLong(datagram)),
            Some(datagram) => Next::Read(Taken::Bytes(datagram)),
            None if now >= deadline => Next::Failed(Failure::timed_out(endpoint, "no datagram came", timeout)),
            None => Next::Sleep(device.next_datagram(now).map_or(deadline, |ready| ready.min(deadline)).saturating_sub(now)),
        },
    }
}

/// A write or a send that went: its entry in the report, and the connection passes through.
fn wrote(log: &mut EndpointLogs, names: Names<'_>, handle: Handle, op: &Op, now: i64) -> NativeOutcome {
    match op {
        Op::Stream(stream::Op::Write(data)) | Op::Send(data) => stream::wrote(log, names, handle, data, now),
        _ => NativeOutcome::Outputs(vec![Value::Handle(handle), Value::NONE]),
    }
}

/// A read or a receive that ended, as `runtime::stream` says it; a datagram dropped by `max` is `net.datagram_too_long`, and
/// has its entry in the report.
fn read_done(log: &mut EndpointLogs, names: Names<'_>, handle: Handle, op: &Op, now: i64, taken: Taken) -> NativeOutcome {
    match (op, taken) {
        (op, Taken::DatagramTooLong(bytes)) => {
            if !bytes.is_empty() {
                log.entry(names.endpoint, now, false, &bytes);
            }
            let max = if let Op::Receive(max) = op { *max } else { 0 };
            let message = format!("{}: a datagram of {} bytes, longer than `max` ({max}): it is dropped", names.label, bytes.len());
            failed(DATAGRAM_TOO_LONG, message)
        }
        (Op::Stream(op), taken) => stream::read_done(log, names, handle, op.stripped(), now, taken),
        (_, taken) => stream::read_done(log, names, handle, None, now, taken),
    }
}

/// The real open, as `Blocking` work.
fn open_work(call: &mut OpenCall, endpoint: Endpoint) -> Step {
    let job = open_job(call, &endpoint);
    call.phase = OpenPhase::Opening(endpoint);
    Step::Blocking(job)
}

#[cfg(not(target_arch = "wasm32"))]
fn open_job(call: &OpenCall, endpoint: &Endpoint) -> Job {
    let (transport, host, port, local_port, text) = (call.transport, endpoint.host.clone(), endpoint.port, call.local_port, endpoint.shown());
    let timeout = std::time::Duration::from_nanos(u64::try_from(call.timeout).unwrap_or(0));
    Box::new(move || {
        let opened = match transport {
            Transport::Udp => real::Udp::open(&host, port, local_port, timeout, &text).map(Backend::Udp),
            Transport::Tcp => real::Tcp::connect(&host, port, timeout, &text).map(Backend::Tcp),
        };
        Box::new(Opened(opened)) as Box<dyn Any + Send>
    })
}

/// A host without TCP and UDP (io.md 3): `net` builds there, and its opens give `io.unsupported` first.
#[cfg(target_arch = "wasm32")]
fn unsupported(endpoint: &str) -> Failure {
    Failure::new(io::UNSUPPORTED, format!("{endpoint}: this host has no TCP or UDP"))
}

#[cfg(target_arch = "wasm32")]
fn open_job(_: &OpenCall, endpoint: &Endpoint) -> Job {
    let failure = unsupported(&endpoint.shown());
    Box::new(move || Box::new(Opened(Err(failure))) as Box<dyn Any + Send>)
}

/// The real work of a read or a write; `None` when the backend is not real.
#[cfg(not(target_arch = "wasm32"))]
#[expect(clippy::disallowed_methods, reason = "never built for WASM, where the clock traps")]
fn real_job(backend: &Backend, op: &Op, call: &TransferCall) -> Option<Job> {
    let since = call.since.unwrap_or_else(std::time::Instant::now);
    let work: Box<dyn FnOnce() -> Worked + Send> = match (backend, op) {
        (Backend::Tcp(tcp), Op::Stream(stream::Op::Write(data))) => {
            let (shared, data) = (tcp.handle(), Arc::clone(data));
            Box::new(move || Worked::Wrote(real::write(&shared, &data, since)))
        }
        (Backend::Tcp(tcp), Op::Stream(stream::Op::Read { need, .. })) => {
            let (shared, need) = (tcp.handle(), need.clone());
            Box::new(move || Worked::Read(real::read(&shared, &need, since)))
        }
        (Backend::Udp(udp), Op::Send(data)) => {
            let (shared, data) = (udp.handle(), Arc::clone(data));
            Box::new(move || Worked::Wrote(real::send(&shared, &data, since)))
        }
        (Backend::Udp(udp), Op::Receive(max)) => {
            let (shared, max) = (udp.handle(), *max);
            Box::new(move || Worked::Read(real::receive(&shared, max, since)))
        }
        _ => return None,
    };
    Some(Box::new(move || Box::new(work()) as Box<dyn Any + Send>))
}

#[cfg(target_arch = "wasm32")]
fn real_job(_: &Backend, op: &Op, _: &TransferCall) -> Option<Job> {
    let failure = unsupported("net");
    let worked = if matches!(op, Op::Stream(stream::Op::Write(_)) | Op::Send(_)) { Worked::Wrote(Err(failure)) } else { Worked::Read(Err(failure)) };
    Some(Box::new(move || Box::new(worked) as Box<dyn Any + Send>))
}

/// What a read, a write, a send or a receive does, from its inputs; its arguments judged as libs/net says ("Reading",
/// "Writing").
fn op_of(function: &str, inputs: &[Value]) -> Result<Op, Failure> {
    Ok(match function {
        "send" => Op::Send(stream::bytes_at(inputs, 1)),
        "receive" => Op::Receive(stream::count_at(inputs, 1, "max")?),
        _ => Op::Stream(stream::op_of(function, inputs)?),
    })
}

impl LibraryRun for NetRun {
    fn call(&mut self, function: &str, inputs: Vec<Value>) -> Option<CallId> {
        let transport = match function {
            "connect" | "write" | "write_line" | "read" | "read_until" | "read_line" | "read_available" | "close" => Transport::Tcp,
            "open_udp" | "send" | "receive" | "close_udp" => Transport::Udp,
            _ => return None,
        };
        let text = |at: usize| match inputs.get(at) {
            Some(Value::String(text)) => text.to_string(),
            _ => String::new(),
        };
        let port = |at: usize| match inputs.get(at) {
            Some(Value::U16(port)) => *port,
            _ => 0,
        };
        let duration = |at: usize| match inputs.get(at) {
            Some(Value::Duration(ns)) => (*ns).max(0),
            _ => 0,
        };
        let handle = match inputs.first() {
            Some(Value::Handle(handle)) => *handle,
            _ => Handle::INVALID,
        };
        let body = match function {
            "connect" => {
                Body::Open(OpenCall { transport, host: text(0), port: port(1), local_port: 0, timeout: duration(2), phase: OpenPhase::Start })
            }
            "open_udp" => {
                Body::Open(OpenCall { transport, host: text(0), port: port(1), local_port: port(2), timeout: duration(3), phase: OpenPhase::Start })
            }
            "close" | "close_udp" => Body::Close(CloseCall { handle, transport, held: false }),
            _ => Body::Transfer(TransferCall {
                handle,
                transport,
                op: op_of(function, &inputs),
                phase: TransferPhase::Start,
                started: 0,
                scan: Scan::default(),
                #[cfg(not(target_arch = "wasm32"))]
                since: None,
            }),
        };
        let id = CallId(self.next_call);
        self.next_call = self.next_call.saturating_add(1);
        self.calls.insert(id, NetCall { body, done: None });
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

    fn cancel(&mut self, call: CallId) {
        self.calls.remove(&call);
    }

    fn describe(&self, handle: Handle) -> Option<HandleInfo> {
        let connection = self.connections.get(self.slot(handle)?)?;
        Some(HandleInfo { kind: connection.transport.kind(), designates: connection.endpoint.clone(), open: connection.open })
    }

    /// Closes what the run leaves open after an abort or a fault (README, "What `net` guarantees").
    fn release(&mut self, handle: Handle) {
        if let Some(index) = self.slot(handle)
            && let Some(connection) = self.connections.get_mut(index)
        {
            connection.shut();
        }
    }

    fn instruments(&self) -> Vec<InstrumentLog> {
        self.log.instruments()
    }
}
