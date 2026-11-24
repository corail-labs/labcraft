//! `net` against `libs/net/README.md` and `docs/io.md` (sections 2, 3, 6, 7 and 8), and the decisions of the owner of
//! 2026-11-19 that the README states. Every expected value is worked out by hand from those rules, and each test says which;
//! none comes from the output of `net`. Real calls go to TCP and UDP servers on 127.0.0.1 that the tests start (io.md 7),
//! with timeouts generous enough for a loaded machine; simulated ones run on the clock of a hand driver, as the scheduler
//! steps them (`docs/architecture.md`, 10.6).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]
#![expect(clippy::disallowed_methods, reason = "tests of real sockets, never built for WASM, where the clock traps")]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, UdpSocket};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use labcraft_runtime::io::{Allowed, Consent, Question, Rights, StartedBy};
use labcraft_runtime::native::{
    CallContext, CallId, CommandLog, Context, Key, LOG_BUDGET, LibraryDiagnostic, LibraryRun, LogBudget, Mode, NativeLibrary, NativeOutcome, Start,
    Step,
};
use labcraft_runtime::simulation::read;
use labcraft_runtime::{Handle, Value};

use crate::Net;
use crate::endpoint::{of_call, of_file};
use labcraft_runtime::stream::{Got, Need, Scan, Terminator};

const MS: i64 = 1_000_000;

/// The diagnostics of a file, or nothing when it is accepted.
fn judged(text: &str) -> Result<Option<labcraft_runtime::native::Simulation>, Vec<LibraryDiagnostic>> {
    let file = read(text, 64).map_err(|diagnostic| vec![diagnostic])?;
    let mut simulations = file.judge(&[&Net])?;
    Ok(simulations.pop().flatten())
}

fn codes(text: &str) -> Vec<(&'static str, u32, u32)> {
    match judged(text) {
        Ok(_) => Vec::new(),
        Err(found) => found.iter().map(|d| (d.code, d.span.map_or(0, |s| s.start.line), d.span.map_or(0, |s| s.start.column))).collect(),
    }
}

struct Task {
    id: CallId,
    wake: i64,
    blocked: Option<Key>,
    held: Vec<Key>,
    done: Option<(NativeOutcome, i64)>,
}

/// What the scheduler does for these calls: the clock, the keys given in the order they were asked, and the work that blocks
/// run at once, its result given back before the next step, as in deterministic mode.
struct Driver {
    run: Box<dyn LibraryRun>,
    now: i64,
    tasks: Vec<Task>,
    holders: HashMap<Key, usize>,
}

/// An error, as the tests compare it.
type Failed = (String, i64, String);

impl Driver {
    fn with(text: Option<&str>, mode: Mode, context: Context) -> Self {
        let simulation = text.map(|text| judged(text).unwrap());
        let run = Net.start_run(Start {
            library: 0,
            simulation: simulation.as_ref().and_then(Option::as_ref),
            budget: LogBudget::new(LOG_BUDGET),
            mode,
            context: &context,
        });
        Driver { run, now: 0, tasks: Vec::new(), holders: HashMap::new() }
    }

    /// A run with `--sim` and this file, in deterministic mode.
    fn simulated(text: &str) -> Self {
        Driver::with(Some(text), Mode::Deterministic, Context { simulation_file: true, ..Context::default() })
    }

    /// A run of the human without `--sim`, in real time: it reaches the hardware.
    fn real() -> Self {
        Driver::with(None, Mode::RealTime, Context::default())
    }

    fn start(&mut self, function: &str, inputs: Vec<Value>) -> usize {
        let id = self.run.call(function, inputs).unwrap();
        self.tasks.push(Task { id, wake: self.now, blocked: None, held: Vec::new(), done: None });
        self.tasks.len() - 1
    }

    fn give(&mut self, key: Key) {
        self.holders.remove(&key);
        if let Some(next) = self.tasks.iter().position(|task| task.blocked == Some(key)) {
            let task = &mut self.tasks[next];
            task.blocked = None;
            task.held.push(key);
            task.wake = task.wake.max(self.now);
            self.holders.insert(key, next);
        }
    }

    fn go(&mut self) {
        loop {
            let ready = (0..self.tasks.len())
                .filter(|index| self.tasks[*index].done.is_none() && self.tasks[*index].blocked.is_none())
                .min_by_key(|index| (self.tasks[*index].wake, *index));
            let Some(index) = ready else { break };
            self.now = self.now.max(self.tasks[index].wake);
            let id = self.tasks[index].id;
            match self.run.step(id, CallContext { now: self.now }) {
                Step::Sleep(ns) => {
                    assert!(ns >= 0, "a sleep is not negative");
                    self.tasks[index].wake = self.now + ns;
                }
                Step::Exclusive(key) => match self.holders.entry(key) {
                    std::collections::hash_map::Entry::Occupied(held) if *held.get() != index => self.tasks[index].blocked = Some(key),
                    std::collections::hash_map::Entry::Occupied(_) => {}
                    std::collections::hash_map::Entry::Vacant(free) => {
                        free.insert(index);
                        self.tasks[index].held.push(key);
                    }
                },
                Step::Release(key) => {
                    self.tasks[index].held.retain(|held| *held != key);
                    self.give(key);
                }
                Step::Blocking(job) => {
                    let out = job();
                    self.run.resume(id, out);
                }
                Step::Done(outcome) => {
                    self.tasks[index].done = Some((outcome, self.now));
                    for key in std::mem::take(&mut self.tasks[index].held) {
                        self.give(key);
                    }
                }
            }
        }
        assert!(self.tasks.iter().all(|task| task.done.is_some()), "a call left waiting for a key");
    }

    /// One call, alone, to its end: its outcome and when it ended.
    fn call(&mut self, function: &str, inputs: Vec<Value>) -> (NativeOutcome, i64) {
        let index = self.start(function, inputs);
        self.go();
        self.tasks[index].done.take().unwrap()
    }

    fn open(&mut self, function: &str, inputs: Vec<Value>) -> Result<Handle, Failed> {
        match self.call(function, inputs).0 {
            NativeOutcome::Outputs(outputs) => match outputs.first() {
                Some(Value::Handle(handle)) => Ok(*handle),
                other => panic!("{other:?}"),
            },
            NativeOutcome::Fail { category, code, message } => Err((category.to_owned(), code, message)),
        }
    }

    fn connect(&mut self, host: &str, port: u16, timeout: i64) -> Result<Handle, Failed> {
        self.open("connect", vec![Value::string(host), Value::U16(port), Value::Duration(timeout), Value::NONE])
    }

    fn open_udp(&mut self, host: &str, port: u16, timeout: i64) -> Result<Handle, Failed> {
        self.open("open_udp", vec![Value::string(host), Value::U16(port), Value::U16(0), Value::Duration(timeout), Value::NONE])
    }

    fn write(&mut self, connection: Handle, data: &[u8]) -> Result<(), Failed> {
        done(self.call("write", vec![Value::Handle(connection), bytes(data), Value::NONE]).0).map(|_| ())
    }

    fn write_line(&mut self, connection: Handle, text: &str, terminator: &str) -> Result<(), Failed> {
        done(self.call("write_line", vec![Value::Handle(connection), Value::string(text), Value::string(terminator), Value::NONE]).0).map(|_| ())
    }

    fn read(&mut self, connection: Handle, count: i32) -> Result<Vec<u8>, Failed> {
        bytes_of(self.call("read", vec![Value::Handle(connection), Value::I32(count), Value::NONE]).0)
    }

    fn read_until(&mut self, connection: Handle, terminator: &[u8], max: i32) -> Result<Vec<u8>, Failed> {
        bytes_of(self.call("read_until", vec![Value::Handle(connection), bytes(terminator), Value::I32(max), Value::NONE]).0)
    }

    fn read_line(&mut self, connection: Handle) -> Result<String, Failed> {
        let outcome = self.call("read_line", vec![Value::Handle(connection), Value::string("\n"), Value::I32(65536), Value::NONE]).0;
        done(outcome).map(|outputs| match outputs.first() {
            Some(Value::String(text)) => text.to_string(),
            other => panic!("{other:?}"),
        })
    }

    fn read_available(&mut self, connection: Handle, max: i32) -> Result<Vec<u8>, Failed> {
        bytes_of(self.call("read_available", vec![Value::Handle(connection), Value::I32(max), Value::NONE]).0)
    }

    fn close(&mut self, connection: Handle) -> Result<(), Failed> {
        done(self.call("close", vec![Value::Handle(connection), Value::NONE]).0).map(|_| ())
    }

    fn send(&mut self, socket: Handle, data: &[u8]) -> Result<(), Failed> {
        done(self.call("send", vec![Value::Handle(socket), bytes(data), Value::NONE]).0).map(|_| ())
    }

    fn receive(&mut self, socket: Handle, max: i32) -> Result<Vec<u8>, Failed> {
        bytes_of(self.call("receive", vec![Value::Handle(socket), Value::I32(max), Value::NONE]).0)
    }

    /// The entries of the report for one endpoint: `(sent, text, hex)`.
    fn entries(&self, endpoint: &str) -> Vec<(bool, String, bool)> {
        self.run
            .instruments()
            .into_iter()
            .find(|log| log.resource == endpoint)
            .map(|log| log.commands.iter().map(entry).collect())
            .unwrap_or_default()
    }
}

fn entry(command: &CommandLog) -> (bool, String, bool) {
    match (&command.sent, &command.answer) {
        (Some(sent), None) => (true, sent.kept.clone(), sent.hex),
        (None, Some(answer)) => (false, answer.kept.clone(), answer.hex),
        other => panic!("an entry of a stream is a write or a read: {other:?}"),
    }
}

fn bytes(data: &[u8]) -> Value {
    Value::Bytes(data.to_vec().into())
}

fn done(outcome: NativeOutcome) -> Result<Vec<Value>, Failed> {
    match outcome {
        NativeOutcome::Outputs(outputs) => Ok(outputs),
        NativeOutcome::Fail { category, code, message } => Err((category.to_owned(), code, message)),
    }
}

fn bytes_of(outcome: NativeOutcome) -> Result<Vec<u8>, Failed> {
    done(outcome).map(|outputs| match outputs.first() {
        Some(Value::Bytes(bytes)) => bytes.to_vec(),
        other => panic!("{other:?}"),
    })
}

fn category<T: std::fmt::Debug>(result: Result<T, Failed>) -> (String, i64) {
    match result {
        Err((category, code, _)) => (category, code),
        Ok(value) => panic!("an error was expected, not {value:?}"),
    }
}

fn hex(text: &str) -> Vec<u8> {
    let digits: Vec<u8> = text.bytes().filter(|byte| *byte != b' ').collect();
    digits.chunks(2).map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap()).collect()
}

// ---- Real TCP and UDP, on 127.0.0.1 ----

/// A TCP server that accepts one connection, sends `send`, then keeps the connection until the test says so (or closes it at
/// once when `close`), and gives what it received.
struct Server {
    port: u16,
    stop: mpsc::Sender<()>,
    received: mpsc::Receiver<Vec<u8>>,
}

fn server(send: &'static [u8], close: bool) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (stop, stopped) = mpsc::channel();
    let (got, received) = mpsc::channel();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream.write_all(send).unwrap();
        if close {
            return;
        }
        stream.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
        let mut buffer = [0_u8; 1024];
        loop {
            match stream.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    let _ = got.send(buffer[..n].to_vec());
                }
                Err(_) => {}
            }
            if stopped.try_recv().is_ok() {
                break;
            }
        }
    });
    Server { port, stop, received }
}

impl Server {
    /// What it received, until `count` bytes.
    fn got(&self, count: usize) -> Vec<u8> {
        let mut all = Vec::new();
        while all.len() < count {
            all.extend(self.received.recv_timeout(Duration::from_secs(10)).unwrap());
        }
        all
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.stop.send(());
    }
}

const GENEROUS: i64 = 10_000 * MS;

#[test]
fn a_real_connection_reads_lines_and_exact_counts_and_a_timeout_takes_nothing() {
    // The issue (How to check): `"OK\r\n"` reads `OK` (README, `read_line`); `read(2)` on `4142` gives `4142`; `read(3)` with
    // two bytes there fails `io.timeout` after the timeout, and the two bytes are still there for `read(2)` (README, `read`).
    let server = server(b"OK\r\nABCD", false);
    let mut driver = Driver::real();
    let timeout = 500 * MS;
    let connection = driver.connect("127.0.0.1", server.port, timeout).unwrap();
    assert_eq!(driver.read_line(connection), Ok("OK".to_owned()));
    assert_eq!(driver.read(connection, 2), Ok(hex("4142")));
    let started = Instant::now();
    assert_eq!(category(driver.read(connection, 3)), ("io.timeout".to_owned(), 0));
    assert!(started.elapsed() >= Duration::from_millis(500), "it waits for the timeout of the connection");
    assert_eq!(driver.read(connection, 2), Ok(b"CD".to_vec()), "the timeout took nothing");
    assert_eq!(driver.read(connection, 0), Ok(Vec::new()), "count 0 gives empty bytes at once");
    // Each read that took bytes has its entry, in order; the one that timed out has none (decision of the owner).
    let endpoint = format!("tcp:127.0.0.1:{}", server.port);
    assert_eq!(driver.entries(&endpoint), [(false, "OK\r\n".to_owned(), false), (false, "AB".to_owned(), false), (false, "CD".to_owned(), false)]);
    assert_eq!(driver.close(connection), Ok(()));
}

#[test]
fn a_real_read_until_without_its_terminator_takes_max_bytes() {
    // The issue: `read_until` with `max` 3 on `"abcdef\n"` fails `io.too_long` and takes `abc`; the next read starts at `d`.
    let server = server(b"abcdef\n", false);
    let mut driver = Driver::real();
    let connection = driver.connect("127.0.0.1", server.port, GENEROUS).unwrap();
    assert_eq!(category(driver.read_until(connection, b"\n", 3)), ("io.too_long".to_owned(), 0));
    assert_eq!(driver.read(connection, 1), Ok(b"d".to_vec()));
    assert_eq!(driver.read_until(connection, b"\n", 65536), Ok(b"ef\n".to_vec()));
    // A read that fails and takes bytes has its entry (decision of the owner).
    let endpoint = format!("tcp:127.0.0.1:{}", server.port);
    assert_eq!(driver.entries(&endpoint).first(), Some(&(false, "abc".to_owned(), false)));
}

#[test]
fn a_real_connection_closed_by_the_other_end_gives_what_arrived_then_io_closed() {
    // The issue: the server closes after two bytes; `read(2)` gives them, then `read(1)` fails `io.closed` (README,
    // "Reading"); `read_available` with nothing left fails `io.closed` too (decision of the owner).
    let server = server(b"XY", true);
    let mut driver = Driver::real();
    let connection = driver.connect("127.0.0.1", server.port, GENEROUS).unwrap();
    assert_eq!(driver.read(connection, 2), Ok(b"XY".to_vec()));
    assert_eq!(category(driver.read(connection, 1)).0, "io.closed");
    assert_eq!(category(driver.read_available(connection, 10)).0, "io.closed");
    // Closing twice succeeds: `close` is `always` (decision of the owner); a call on it then fails `io.closed`.
    assert_eq!(driver.close(connection), Ok(()));
    assert_eq!(driver.close(connection), Ok(()));
    let (category, code, message) = driver.read(connection, 1).unwrap_err();
    assert_eq!((category.as_str(), code), ("io.closed", 0));
    assert!(message.contains("the connection is closed"), "{message}");
}

#[test]
fn a_real_write_sends_every_byte_and_write_line_its_terminator() {
    // README, "Writing": `write_line` writes the UTF-8 bytes of `text`, then those of `terminator`.
    let server = server(b"", false);
    let mut driver = Driver::real();
    let connection = driver.connect("127.0.0.1", server.port, GENEROUS).unwrap();
    assert_eq!(driver.write(connection, &hex("0001 ff")), Ok(()));
    assert_eq!(driver.write_line(connection, "é", "\r\n"), Ok(()));
    assert_eq!(server.got(7), [0x00, 0x01, 0xff, 0xc3, 0xa9, b'\r', b'\n']);
    // io.md 6: one entry per write, `sent`; text when UTF-8 without control characters other than `\r` `\n` `\t`, else hex.
    let endpoint = format!("tcp:127.0.0.1:{}", server.port);
    assert_eq!(driver.entries(&endpoint), [(true, "0001ff".to_owned(), true), (true, "é\r\n".to_owned(), false)]);
    // `read_available` with nothing there gives empty bytes at once, and no entry (decision of the owner).
    assert_eq!(driver.read_available(connection, 10), Ok(Vec::new()));
    assert_eq!(driver.entries(&endpoint).len(), 2);
}

#[test]
fn a_real_connect_to_a_closed_port_is_refused_with_the_code_of_the_system() {
    // The issue: `connect` to a closed port on 127.0.0.1 is `net.connection_refused`; io.md 7: its code is that of the
    // system, which is not 0. Windows retries a refused connect for about two seconds: the timeout leaves room for it.
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut driver = Driver::real();
    let (category, code) = category(driver.connect("127.0.0.1", port, GENEROUS));
    assert_eq!(category, "net.connection_refused");
    assert_ne!(code, 0);
}

#[test]
fn a_released_real_connection_is_shut_down() {
    // README, "What `net` guarantees": after an abort or a fault, the connections the run holds are closed; the server sees
    // the end of the stream.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (ended, end) = mpsc::channel();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let mut buffer = [0_u8; 8];
        let _ = ended.send(matches!(stream.read(&mut buffer), Ok(0)));
    });
    let mut driver = Driver::real();
    let connection = driver.connect("127.0.0.1", port, GENEROUS).unwrap();
    driver.run.release(connection);
    assert!(end.recv_timeout(Duration::from_secs(10)).unwrap(), "the server reads the end of the stream");
    assert!(!driver.run.describe(connection).unwrap().open);
    assert_eq!(category(driver.read(connection, 1)).0, "io.closed");
}

/// A UDP server that sends each datagram back and gives what it received.
fn echo() -> (u16, mpsc::Receiver<Vec<u8>>) {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    let port = socket.local_addr().unwrap().port();
    let (got, received) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buffer = vec![0_u8; 70_000];
        socket.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
        while let Ok((n, from)) = socket.recv_from(&mut buffer) {
            let _ = socket.send_to(&buffer[..n], from);
            if got.send(buffer[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    (port, received)
}

#[test]
fn a_real_udp_socket_sends_and_receives_whole_datagrams() {
    let (port, received) = echo();
    let mut driver = Driver::real();
    let socket = driver.open_udp("127.0.0.1", port, GENEROUS).unwrap();
    assert_eq!(driver.send(socket, b"ping"), Ok(()));
    assert_eq!(driver.receive(socket, 65507), Ok(b"ping".to_vec()));
    // README, "Reading": a datagram longer than `max` fails `net.datagram_too_long`, and is dropped.
    assert_eq!(driver.send(socket, b"0123456789"), Ok(()));
    assert_eq!(category(driver.receive(socket, 4)).0, "net.datagram_too_long");
    // The issue: a datagram of 65,508 bytes is `net.datagram_too_long`, and nothing is sent (README, "Writing").
    assert_eq!(category(driver.send(socket, &vec![7; 65_508])), ("net.datagram_too_long".to_owned(), 0));
    assert_eq!(driver.send(socket, b"after"), Ok(()));
    assert_eq!(driver.receive(socket, 65507), Ok(b"after".to_vec()), "the dropped datagram does not come again");
    let seen: Vec<Vec<u8>> = (0..3).map(|_| received.recv_timeout(Duration::from_secs(10)).unwrap()).collect();
    assert_eq!(seen, [b"ping".to_vec(), b"0123456789".to_vec(), b"after".to_vec()], "the long one never left");
    assert_eq!(driver.run.describe(socket).map(|info| (info.kind, info.designates)), Some(("udp_socket", format!("udp:127.0.0.1:{port}"))));
    assert_eq!(driver.run.instruments().len(), 1);
    assert_eq!(driver.run.instruments()[0].commands.len(), 6, "three sends and three receives, the dropped one included");
    assert_eq!(done(driver.call("close_udp", vec![Value::Handle(socket), Value::NONE]).0).map(|_| ()), Ok(()));
    assert_eq!(done(driver.call("close_udp", vec![Value::Handle(socket), Value::NONE]).0).map(|_| ()), Ok(()), "always");
    assert_eq!(category(driver.send(socket, b"x")).0, "io.closed");
}

#[test]
fn a_real_udp_receive_with_nothing_times_out() {
    let (port, _received) = echo();
    let mut driver = Driver::real();
    let socket = driver.open_udp("127.0.0.1", port, 300 * MS).unwrap();
    let started = Instant::now();
    assert_eq!(category(driver.receive(socket, 100)), ("io.timeout".to_owned(), 0));
    assert!(started.elapsed() >= Duration::from_millis(300));
}

// ---- What a run reaches (io.md 3 and 4) ----

#[test]
fn without_sim_a_deterministic_run_needs_real_time() {
    // io.md 3: without `--sim`, deterministic: `io.needs_real_time`, at once, code 0.
    let mut driver = Driver::with(None, Mode::Deterministic, Context::default());
    let (category, code, message) = driver.connect("10.0.0.5", 502, GENEROUS).unwrap_err();
    assert_eq!((category.as_str(), code, driver.now), ("io.needs_real_time", 0, 0));
    assert!(message.contains("tcp:10.0.0.5:502") && message.contains("--real-time"), "{message}");
    assert_eq!(category_of(driver.open_udp("10.0.0.5", 502, MS)), "io.needs_real_time");
}

fn category_of<T: std::fmt::Debug>(result: Result<T, Failed>) -> String {
    category(result).0
}

#[test]
fn with_sim_an_endpoint_the_file_does_not_describe_is_not_found_after_the_timeout() {
    // io.md 3: with `--sim`, the file is the whole bench: `net.host_not_found` after the timeout of the open, never the
    // hardware. With a file that has no `[net]`, the same.
    let mut driver = Driver::simulated("[[net.device]]\nendpoint = \"tcp:10.0.0.7:502\"\n");
    assert_eq!(category(driver.connect("10.0.0.7", 503, 300 * MS)), ("net.host_not_found".to_owned(), 0));
    assert_eq!(driver.now, 300 * MS);
    assert_eq!(category_of(driver.open_udp("10.0.0.7", 502, 0)), "net.host_not_found", "udp: is another endpoint");
    let mut without = Driver::with(None, Mode::RealTime, Context { simulation_file: true, ..Context::default() });
    let (category, _, message) = without.connect("127.0.0.1", 1, 100 * MS).unwrap_err();
    assert_eq!((category.as_str(), without.now), ("net.host_not_found", 100 * MS));
    assert!(message.contains("no `[net]`"), "{message}");
}

struct Window {
    yes: bool,
    asked: Mutex<Vec<String>>,
}

impl Consent for Window {
    fn ask(&self, question: &Question<'_>) -> bool {
        if let labcraft_runtime::io::Reach::Endpoint { endpoint, given } = question.reach {
            self.asked.lock().unwrap().push(format!("{endpoint} {given}"));
        }
        self.yes
    }
}

fn agent(allowed: Allowed) -> Context {
    Context { started_by: StartedBy::Agent { client: "c".to_owned(), rights: Rights::of_run(allowed, "r1", "c", None) }, ..Context::default() }
}

#[test]
fn an_agent_reaches_what_the_human_allows() {
    // io.md 4: without the right, `io.not_allowed`, code 0, naming the endpoint and `--allow-hardware`.
    let mut driver = Driver::with(None, Mode::RealTime, agent(Allowed { server: true, ..Allowed::default() }));
    let (category, code, message) = driver.connect("10.0.0.5", 502, MS).unwrap_err();
    assert_eq!((category.as_str(), code), ("io.not_allowed", 0));
    assert!(message.contains("--allow-hardware tcp:10.0.0.5:502"), "{message}");
    // The question of the window: no gives `io.not_allowed`; yes reaches the server. It names the endpoint and the name given.
    for yes in [false, true] {
        let server = server(b"hi\n", false);
        let window = Arc::new(Window { yes, asked: Mutex::default() });
        let context = agent(Allowed { consent: Some(Arc::clone(&window) as Arc<dyn Consent>), ..Allowed::default() });
        let mut driver = Driver::with(None, Mode::RealTime, context);
        let opened = driver.connect("127.0.0.1", server.port, GENEROUS);
        assert_eq!(*window.asked.lock().unwrap(), [format!("tcp:127.0.0.1:{0} 127.0.0.1:{0}", server.port)]);
        match opened {
            Ok(connection) => {
                assert!(yes);
                assert_eq!(driver.read_line(connection), Ok("hi".to_owned()));
            }
            Err((category, _, _)) => {
                assert!(!yes);
                assert_eq!(category, "io.not_allowed");
            }
        }
    }
}

#[test]
fn an_open_judges_its_name_first() {
    // io.md 8: a port of 0 is `io.invalid_argument`; an empty host names no device. Both before the table of io.md 3.
    let mut driver = Driver::with(None, Mode::Deterministic, Context::default());
    assert_eq!(category(driver.connect("10.0.0.5", 0, MS)), ("io.invalid_argument".to_owned(), 0));
    assert_eq!(category(driver.connect("", 502, MS)), ("net.host_not_found".to_owned(), 0));
    assert_eq!(category(driver.open_udp("[]", 502, MS)), ("net.host_not_found".to_owned(), 0));
}

#[test]
fn endpoints_are_written_one_way() {
    // io.md 2: the host in lower case for its ASCII letters, IPv6 in brackets whether given or not, the port in decimal
    // without zeros in front.
    let text = |udp: bool, host: &str, port: u16| of_call(udp, host, port).map(|endpoint| endpoint.text).unwrap();
    assert_eq!(text(false, "DMM.Local", 5025), "tcp:dmm.local:5025");
    assert_eq!(text(false, "FE80::1", 502), "tcp:[fe80::1]:502");
    assert_eq!(text(true, "[fe80::1]", 502), "udp:[fe80::1]:502");
    assert_eq!(of_call(false, "Dmm_2-A.LOCAL", 1).unwrap().text, "tcp:dmm_2-a.local:1", "ASCII letters, digits, `-`, `.`, `_`");
    assert_eq!(of_call(false, "h", 7).unwrap().given, "h:7");
    assert_eq!(of_file("tcp:h:0502"), Some(("tcp:h:502".to_owned(), false)));
    assert_eq!(of_file("udp:[FE80::1]:0007"), Some(("udp:[fe80::1]:7".to_owned(), true)));
    assert_eq!(of_file("tcp:fe80::1:502"), Some(("tcp:[fe80::1]:502".to_owned(), false)));
    for wrong in [
        "serial:COM3",
        "tcp:h",
        "tcp::502",
        "tcp:h:0",
        "tcp:h:65536",
        "tcp:h:5x",
        "TCP:h:1",
        "tcp:a b:1",
        "tcp:h:",
        "tcp:[h]502",
        "tcp:[h]:1",
        "tcp:[10.0.0.5]:1",
        "tcp:a:b:1",
        "tcp:Ünï:1",
    ] {
        assert_eq!(of_file(wrong), None, "{wrong}");
    }
}

// ---- Simulated devices (io.md 6) ----

/// The Modbus TCP server of io.md 6, and the board of its example, as a TCP device.
const BENCH: &str = r#"
[[net.device]]
endpoint = "tcp:10.0.0.7:502"

[[net.device.reply]]
request_hex = "0001 0000 0006 01 03 0000 0002"
response_hex = "0001 0000 0007 01 03 04 00e6 0019"

[[net.device]]
endpoint = "tcp:Board.local:0023"
latency_ms = 2

[[net.device.reply]]
request = "LED ON\n"
response = "OK\r\n"

[[net.device.reply]]
request = "TEMP?\n"
response = "23.5\r\n"
"#;

#[test]
fn the_modbus_exchange_of_io_md_6() {
    // io.md 6: `0001 0000 0006 01 03 0000 0002` gives `0001 0000 0007 01 03 04 00e6 0019`: the header of 7 bytes, then the 6
    // its length gives (io.md 10).
    let mut driver = Driver::simulated(BENCH);
    let modbus = driver.connect("10.0.0.7", 502, 1_000 * MS).unwrap();
    driver.write(modbus, &hex("0001 0000 0006 01 03 0000 0002")).unwrap();
    assert_eq!(driver.read(modbus, 7), Ok(hex("0001 0000 0007 01")));
    assert_eq!(driver.read(modbus, 6), Ok(hex("03 04 00e6 0019")));
    assert_eq!(driver.now, 0, "no latency, no time");
    // The report: the bytes are no text, so hexadecimal (io.md 6).
    assert_eq!(
        driver.entries("tcp:10.0.0.7:502"),
        [(true, "000100000006010300000002".to_owned(), true), (false, "00010000000701".to_owned(), true), (false, "030400e60019".to_owned(), true)]
    );
}

#[test]
fn one_write_of_two_requests_gets_both_responses_in_order() {
    // io.md 6: `write("LED ON\nTEMP?\n")` gets both answers, in order; the file writes the host and the port another way,
    // and the endpoint is the same (io.md 2). Latency 2 ms: both are ready 2 ms after the write.
    let mut driver = Driver::simulated(BENCH);
    let board = driver.connect("board.LOCAL", 23, 1_000 * MS).unwrap();
    driver.write(board, b"LED ON\nTEMP?\n").unwrap();
    assert_eq!(driver.read_line(board), Ok("OK".to_owned()));
    assert_eq!(driver.now, 2 * MS);
    assert_eq!(driver.read_line(board), Ok("23.5".to_owned()));
    assert_eq!(driver.now, 2 * MS);
}

#[test]
fn a_request_split_over_two_writes_matches_once_whole() {
    // io.md 6: when what is left is the start of some request, the device waits for more.
    let mut driver = Driver::simulated(BENCH);
    let board = driver.connect("board.local", 23, 1_000 * MS).unwrap();
    driver.write(board, b"LED ").unwrap();
    driver.now = 10 * MS;
    assert_eq!(driver.read_available(board, 100), Ok(Vec::new()), "nothing answered yet");
    driver.write_line(board, "ON", "\n").unwrap();
    assert_eq!(driver.read_line(board), Ok("OK".to_owned()));
    assert_eq!(driver.now, 12 * MS, "2 ms after the write that completed it");
}

#[test]
fn latency_puts_the_read_after_the_write_on_the_clock() {
    // The issue: `latency_ms = 20` makes the read end at the clock 20 ms after the write, in deterministic mode. A timeout
    // shorter than the latency gives `io.timeout` at the timeout, counted from the start of the read, and takes nothing.
    let file = "[[net.device]]\nendpoint = \"tcp:h:1\"\nlatency_ms = 20\n[[net.device.reply]]\nrequest = \"A\"\nresponse = \"B\"\n";
    let mut driver = Driver::simulated(file);
    let connection = driver.connect("h", 1, 1_000 * MS).unwrap();
    driver.now = 5 * MS;
    driver.write(connection, b"A").unwrap();
    assert_eq!(driver.read(connection, 1), Ok(b"B".to_vec()));
    assert_eq!(driver.now, 25 * MS);
    let mut quick = Driver::simulated(file);
    let connection = quick.connect("h", 1, 10 * MS).unwrap();
    quick.write(connection, b"A").unwrap();
    assert_eq!(category(quick.read(connection, 1)), ("io.timeout".to_owned(), 0));
    assert_eq!(quick.now, 10 * MS);
    assert_eq!(quick.read(connection, 1), Ok(b"B".to_vec()), "ready at 20 ms, read from 10 ms");
    assert_eq!(quick.now, 20 * MS);
}

#[test]
fn what_matches_nothing_is_forgotten_and_answered_by_unknown() {
    // io.md 6: what matches nothing is forgotten, and `unknown` answers it if the device has one; without, nothing answers.
    let file = "[[net.device]]\nendpoint = \"tcp:h:1\"\nunknown = \"ERR\\r\\n\"\n[[net.device.reply]]\nrequest = \"A\\n\"\nresponse = \"a\\n\"\n\
                [[net.device]]\nendpoint = \"tcp:h:2\"\n[[net.device.reply]]\nrequest = \"A\\n\"\nresponse = \"a\\n\"\n\
                [[net.device.reply]]\nrequest = \"QUIET\\n\"\n";
    let mut driver = Driver::simulated(file);
    let one = driver.connect("h", 1, 100 * MS).unwrap();
    driver.write(one, b"BOGUS\nA\n").unwrap();
    assert_eq!(driver.read_line(one), Ok("ERR".to_owned()), "all of what is left is forgotten, `A\\n` with it");
    assert_eq!(category(driver.read_line(one)), ("io.timeout".to_owned(), 0));
    let two = driver.connect("h", 2, 100 * MS).unwrap();
    driver.write(two, b"QUIET\nBOGUS\nA\n").unwrap();
    let before = driver.now;
    assert_eq!(category(driver.read(two, 1)), ("io.timeout".to_owned(), 0), "a request without response, then nothing");
    assert_eq!(driver.now - before, 100 * MS);
}

#[test]
fn read_line_strips_its_terminator_and_refuses_bytes_that_are_not_utf8() {
    // README, `read_line`: the terminator is removed and, for `"\n"`, one `"\r"` before it; bytes that are not UTF-8 fail
    // `io.invalid_text`, and are taken. Another terminator keeps the `\r`.
    let file = "[[net.device]]\nendpoint = \"tcp:h:1\"\n[[net.device.reply]]\nrequest = \"?\"\nresponse_hex = \"41 0d 0d 0a ff 0a 42 0d 3b\"\n";
    let mut driver = Driver::simulated(file);
    let connection = driver.connect("h", 1, 100 * MS).unwrap();
    driver.write(connection, b"?").unwrap();
    assert_eq!(driver.read_line(connection), Ok("A\r".to_owned()), "one `\\r` only");
    assert_eq!(category(driver.read_line(connection)), ("io.invalid_text".to_owned(), 0));
    let outcome = driver.call("read_line", vec![Value::Handle(connection), Value::string(";"), Value::I32(10), Value::NONE]).0;
    assert!(
        matches!(&outcome, NativeOutcome::Outputs(outputs) if matches!(outputs.first(), Some(Value::String(text)) if &**text == "B\r")),
        "{outcome:?}"
    );
    let entries = driver.entries("tcp:h:1");
    assert_eq!(entries[2], (false, "ff0a".to_owned(), true), "the bytes taken by the failed read, in hexadecimal");
}

#[test]
fn arguments_out_of_their_range_are_invalid() {
    // README, "Reading": a `count` or a `max` below 0 is `io.invalid_argument`, above 16 MiB too; an empty terminator too.
    let mut driver = Driver::simulated(BENCH);
    let board = driver.connect("board.local", 23, 100 * MS).unwrap();
    for result in [
        driver.read(board, -1),
        driver.read(board, 16 * 1024 * 1024 + 1),
        driver.read_until(board, b"", 10),
        driver.read_until(board, b"\n", -1),
        driver.read_available(board, -5),
    ] {
        assert_eq!(category(result), ("io.invalid_argument".to_owned(), 0));
    }
    assert_eq!(driver.read(board, 0), Ok(Vec::new()));
    let outcome = driver.call("read_line", vec![Value::Handle(board), Value::string(""), Value::I32(10), Value::NONE]).0;
    assert!(matches!(outcome, NativeOutcome::Fail { category: "io.invalid_argument", .. }));
    assert_eq!(driver.now, 0, "at once");
    assert!(driver.entries("tcp:board.local:23").is_empty(), "nothing read, no entry");
}

#[test]
fn a_connection_closed_or_never_opened_is_io_closed() {
    // Decision of the owner: a call on a closed connection, or on one never opened, fails `io.closed`, whose message says which;
    // `close` on either succeeds.
    let mut driver = Driver::simulated(BENCH);
    let (_, _, message) = driver.read(Handle::INVALID, 1).unwrap_err();
    assert!(message.contains("never opened"), "{message}");
    assert_eq!(driver.close(Handle::INVALID), Ok(()));
    let board = driver.connect("board.local", 23, 100 * MS).unwrap();
    assert_eq!(driver.close(board), Ok(()));
    let (category, code, message) = driver.write(board, b"x").unwrap_err();
    assert_eq!((category.as_str(), code, message.as_str()), ("io.closed", 0, "tcp:board.local:23: the connection is closed"));
    let (_, _, message) = driver.send(Handle::INVALID, b"x").unwrap_err();
    assert!(message.contains("never opened") && message.contains("net.udp_socket"), "{message}");
    let other = Handle { library: 1, number: board.number };
    assert_eq!(category_of(driver.read(other, 1)), "io.closed", "a handle of another library is none of its");
    assert!(driver.run.describe(other).is_none());
}

#[test]
fn a_read_that_waits_holds_its_connection() {
    // README, "What `net` guarantees": each call is atomic on its connection; a read that waits for its timeout holds it, so
    // a write from another branch waits for it: by hand, the read times out at 100 ms, then the write goes, at 100 ms.
    let mut driver = Driver::simulated(BENCH);
    let board = driver.connect("board.local", 23, 100 * MS).unwrap();
    let read = driver.start("read_line", vec![Value::Handle(board), Value::string("\n"), Value::I32(100), Value::NONE]);
    let write = driver.start("write", vec![Value::Handle(board), bytes(b"LED ON\n"), Value::NONE]);
    driver.go();
    let (outcome, at) = driver.tasks[read].done.take().unwrap();
    assert!(matches!(outcome, NativeOutcome::Fail { category: "io.timeout", .. }), "{outcome:?}");
    assert_eq!(at, 100 * MS);
    assert_eq!(driver.tasks[write].done.as_ref().unwrap().1, 100 * MS);
    assert_eq!(driver.read_line(board), Ok("OK".to_owned()));
    assert_eq!(driver.now, 102 * MS);
}

#[test]
fn a_device_that_closes_at_close_ms() {
    // The issue: `close_ms = 50` gives `io.closed` to a read after it. Decision of the owner: what was ready before is still
    // read; a write after it is `io.closed`; a read that waits across it fails at 50 ms; a connect after it succeeds, with a
    // connection already closed; a response that would be ready at or after it is never given.
    let file = "[[net.device]]\nendpoint = \"tcp:h:1\"\nlatency_ms = 10\nfaults = { close_ms = 50 }\n\
                [[net.device.reply]]\nrequest = \"A\"\nresponse = \"a\"\n";
    let mut driver = Driver::simulated(file);
    let connection = driver.connect("h", 1, 1_000 * MS).unwrap();
    driver.write(connection, b"A").unwrap();
    driver.now = 45 * MS;
    driver.write(connection, b"A").unwrap();
    driver.now = 60 * MS;
    assert_eq!(driver.read(connection, 1), Ok(b"a".to_vec()), "ready at 10 ms, before the close");
    assert_eq!(category(driver.read(connection, 1)), ("io.closed".to_owned(), 0), "ready at 55 ms: never given");
    assert_eq!(category_of(driver.write(connection, b"A")), "io.closed");
    assert_eq!(category_of(driver.read_available(connection, 10)), "io.closed", "nothing waits, and it is closed");
    let mut waits = Driver::simulated(file);
    let connection = waits.connect("h", 1, 1_000 * MS).unwrap();
    assert_eq!(category_of(waits.read(connection, 1)), "io.closed");
    assert_eq!(waits.now, 50 * MS);
    let late = waits.connect("h", 1, 1_000 * MS).unwrap();
    assert_eq!(category_of(waits.read(late, 1)), "io.closed");
    assert_eq!(waits.now, 50 * MS);
}

#[test]
fn a_device_that_hangs_at_hang_ms() {
    // The issue: `hang_ms` gives `io.timeout`. Decision of the owner: from `hang_ms` the device takes no request and gives no
    // response; a write still succeeds, its bytes ignored; a response that would be ready at or after it is never given.
    let file = "[[net.device]]\nendpoint = \"tcp:h:1\"\nlatency_ms = 20\nfaults = { hang_ms = 30 }\n\
                [[net.device.reply]]\nrequest = \"A\"\nresponse = \"a\"\n";
    let mut driver = Driver::simulated(file);
    let connection = driver.connect("h", 1, 100 * MS).unwrap();
    driver.write(connection, b"A").unwrap();
    assert_eq!(driver.read(connection, 1), Ok(b"a".to_vec()));
    assert_eq!(driver.now, 20 * MS);
    driver.write(connection, b"A").unwrap();
    assert_eq!(category(driver.read(connection, 1)), ("io.timeout".to_owned(), 0), "ready at 40 ms, after the hang");
    assert_eq!(driver.now, 120 * MS);
    assert_eq!(driver.write(connection, b"A"), Ok(()));
    assert_eq!(category_of(driver.read(connection, 1)), "io.timeout");
    let late = driver.connect("h", 1, 100 * MS).unwrap();
    assert_eq!(driver.read_available(late, 1), Ok(Vec::new()), "a connect after the hang succeeds");
    // Both writes are in the report, though the device ignored the second; the reads that took nothing are not.
    assert_eq!(
        driver.entries("tcp:h:1"),
        [(true, "A".to_owned(), false), (false, "a".to_owned(), false), (true, "A".to_owned(), false), (true, "A".to_owned(), false)]
    );
}

#[test]
fn a_device_that_refuses() {
    // The issue: `refused = true` gives `net.connection_refused`, code 0 (io.md 7: a simulated error).
    let mut driver = Driver::simulated("[[net.device]]\nendpoint = \"tcp:h:1\"\nfaults = { refused = true }\n");
    assert_eq!(category(driver.connect("h", 1, 100 * MS)), ("net.connection_refused".to_owned(), 0));
    assert_eq!(driver.now, 0);
}

#[test]
fn a_simulated_udp_device_answers_whole_datagrams() {
    // io.md 6: over UDP each datagram is a whole request; one that equals no request matches nothing, and `unknown` answers.
    let file = "[[net.device]]\nendpoint = \"udp:10.0.0.9:5000\"\nlatency_ms = 1\nunknown = \"?\"\n\
                [[net.device.reply]]\nrequest = \"PING\"\nresponse = \"PONG\"\n\
                [[net.device.reply]]\nrequest = \"EMPTY\"\nresponse = \"\"\n";
    let mut driver = Driver::simulated(file);
    let socket = driver.open_udp("10.0.0.9", 5000, 50 * MS).unwrap();
    driver.send(socket, b"PING").unwrap();
    assert_eq!(driver.receive(socket, 65507), Ok(b"PONG".to_vec()));
    assert_eq!(driver.now, MS);
    driver.send(socket, b"PI").unwrap();
    assert_eq!(driver.receive(socket, 65507), Ok(b"?".to_vec()), "the start of a request is no request over UDP");
    driver.send(socket, b"EMPTY").unwrap();
    assert_eq!(driver.receive(socket, 65507), Ok(Vec::new()), "an empty datagram is a datagram");
    driver.send(socket, b"PING").unwrap();
    assert_eq!(category(driver.receive(socket, 3)), ("net.datagram_too_long".to_owned(), 0));
    assert_eq!(category(driver.receive(socket, 100)), ("io.timeout".to_owned(), 0), "the long one was dropped");
    assert_eq!(category_of(driver.send(socket, &vec![0; 65_508])), "net.datagram_too_long");
    assert_eq!(driver.send(socket, &vec![0; 65_507]), Ok(()), "65,507 bytes is a datagram");
    assert_eq!(category(driver.read(socket, 1)).0, "io.closed", "a socket is no connection");
    assert_eq!(driver.run.describe(socket).unwrap().kind, "udp_socket");
}

#[test]
fn a_simulated_read_available_takes_what_is_ready_up_to_max() {
    // README, "Reading": what has arrived, up to `max` bytes, at once.
    let mut driver = Driver::simulated(BENCH);
    let board = driver.connect("board.local", 23, 100 * MS).unwrap();
    driver.write(board, b"TEMP?\n").unwrap();
    assert_eq!(driver.read_available(board, 10), Ok(Vec::new()), "ready at 2 ms, not at 0");
    driver.now = 2 * MS;
    assert_eq!(driver.read_available(board, 3), Ok(b"23.".to_vec()));
    assert_eq!(driver.read_available(board, 65536), Ok(b"5\r\n".to_vec()));
}

#[test]
fn release_closes_a_simulated_connection_and_cancel_ends_a_call() {
    let mut driver = Driver::simulated(BENCH);
    let board = driver.connect("board.local", 23, 100 * MS).unwrap();
    let info = driver.run.describe(board).unwrap();
    assert_eq!((info.kind, info.designates.as_str(), info.open), ("connection", "tcp:board.local:23", true));
    let id = driver.run.call("read", vec![Value::Handle(board), Value::I32(1), Value::NONE]).unwrap();
    driver.run.cancel(id);
    assert!(matches!(driver.run.step(id, CallContext { now: 0 }), Step::Done(NativeOutcome::Fail { .. })));
    driver.run.release(board);
    assert!(!driver.run.describe(board).unwrap().open);
    assert_eq!(category_of(driver.read(board, 1)), "io.closed");
}

#[test]
fn the_report_lists_each_endpoint_in_code_point_order() {
    // commands.md 7: endpoints in code point order of their names; one that saw nothing is not listed.
    let mut driver = Driver::simulated(BENCH);
    let modbus = driver.connect("10.0.0.7", 502, 100 * MS).unwrap();
    let board = driver.connect("board.local", 23, 100 * MS).unwrap();
    driver.write(board, b"LED ON\n").unwrap();
    driver.write(modbus, &hex("00")).unwrap();
    let listed: Vec<String> = driver.run.instruments().into_iter().map(|log| log.resource).collect();
    assert_eq!(listed, ["tcp:10.0.0.7:502", "tcp:board.local:23"]);
    let first = &driver.run.instruments()[1].commands[0];
    let sent = first.sent.as_ref().unwrap();
    assert_eq!((first.at, sent.kept.as_str(), first.answer.as_ref(), sent.hex), (0, "LED ON\n", None, false));
}

#[test]
fn the_log_keeps_the_start_of_a_long_read() {
    // commands.md 5: a text is cut to its first 1,000 code points, with its whole length; hexadecimal is 2 code points a byte.
    let file = "[[net.device]]\nendpoint = \"tcp:h:1\"\n[[net.device.reply]]\nrequest = \"?\"\nresponse_hex = \"00\"\n";
    let mut driver = Driver::simulated(file);
    let connection = driver.connect("h", 1, 100 * MS).unwrap();
    driver.write(connection, &vec![b'x'; 2500]).unwrap();
    let logs = driver.run.instruments();
    let entry = &logs[0].commands[0];
    let sent = entry.sent.as_ref().unwrap();
    assert_eq!((sent.kept.chars().count(), sent.length), (1000, 2500));
    driver.write(connection, &[0xff; 700]).unwrap();
    let logs = driver.run.instruments();
    let entry = &logs[0].commands[1];
    let sent = entry.sent.as_ref().unwrap();
    assert_eq!((sent.kept.chars().count(), sent.length, sent.hex), (1000, 1400, true));
}

#[test]
fn a_file_says_its_devices_and_their_endpoints() {
    let simulation = judged(BENCH).unwrap().unwrap();
    assert_eq!(Net.resources(&simulation), ["tcp:10.0.0.7:502", "tcp:board.local:23"], "in the order of the file");
    let described = format!("{:?}", Net.describe(&simulation).unwrap());
    assert!(described.contains("request_hex") && described.contains("000100000006010300000002"), "{described}");
    assert!(described.contains("Text(\"LED ON\\n\")"), "{described}");
}

// ---- The section of a simulation file (README, "Simulation") ----

/// One file per row of the table of the README, in `tests/refused/`, each refused with its code and that code only.
#[test]
fn each_refused_file_gives_its_code() {
    let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("refused");
    let mut seen = Vec::new();
    for entry in std::fs::read_dir(folder).unwrap() {
        let path = entry.unwrap().path();
        let code = format!("net:{}", path.file_stem().unwrap().to_str().unwrap());
        let text = std::fs::read_to_string(&path).unwrap();
        let found: Vec<&str> = codes(&text).iter().map(|(code, _, _)| *code).collect();
        assert!(!found.is_empty() && found.iter().all(|found| *found == code), "{}: {found:?}", path.display());
        seen.push(code);
    }
    seen.sort();
    let all: Vec<&str> = crate::CODES.iter().map(|code| code.code).collect();
    assert_eq!(seen, all, "one file for each code");
}

#[test]
fn each_problem_of_a_file_has_its_code_at_its_key() {
    let device = |body: &str| format!("[[net.device]]\nendpoint = \"tcp:h:1\"\n{body}\n");
    // Line and column by hand: line 3 is the first line of `body`.
    assert_eq!(codes(&device("latency = 1")), [("net:S001", 3, 1)]);
    assert_eq!(codes(&device("latency_ms = \"5\"")), [("net:S002", 3, 14)]);
    assert_eq!(codes(&device("latency_ms = -1")), [("net:S005", 3, 14)]);
    assert_eq!(codes(&device("latency_ms = 86400001")), [("net:S005", 3, 14)]);
    assert_eq!(codes(&device("latency_ms = 86400000")), []);
    assert_eq!(codes(&device("latency_ms = nan")), [("net:S005", 3, 14)]);
    assert_eq!(codes("[[net.device]]\nlatency_ms = 1\n"), [("net:S003", 1, 1)], "no endpoint");
    assert_eq!(codes("[[net.device]]\nendpoint = \"serial:COM3\"\n"), [("net:S003", 2, 12)]);
    assert_eq!(codes("[[net.device]]\nendpoint = 3\n"), [("net:S002", 2, 12)]);
    assert_eq!(codes(&format!("{}{}", device(""), "[[net.device]]\nendpoint = \"tcp:H:01\"\n")), [("net:S004", 5, 12)], "the same endpoint");
    assert_eq!(codes(&device("[[net.device.reply]]\nrequest = \"a\"\nrequest_hex = \"61\"")), [("net:S006", 5, 15)]);
    assert_eq!(codes(&device("[[net.device.reply]]\nresponse = \"a\"")), [("net:S006", 3, 1)], "no request");
    assert_eq!(codes(&device("[[net.device.reply]]\nrequest = \"\"")), [("net:S006", 4, 11)]);
    assert_eq!(codes(&device("[[net.device.reply]]\nrequest_hex = \"0 1\"")), [("net:S006", 4, 15)]);
    assert_eq!(codes(&device("[[net.device.reply]]\nrequest_hex = \"0g\"")), [("net:S006", 4, 15)]);
    assert_eq!(codes(&device("[[net.device.reply]]\nrequest_hex = \"00 01  02\"")), [], "spaces between pairs");
    assert_eq!(codes(&device("unknown = \"a\"\nunknown_hex = \"61\"")), [("net:S006", 4, 15)]);
    assert_eq!(codes(&device(&format!("[[net.device.reply]]\nrequest = \"{}\"", "a".repeat(65_537)))), [("net:S006", 4, 11)]);
    assert_eq!(codes(&device(&format!("[[net.device.reply]]\nrequest = \"{}\"", "a".repeat(65_536)))), []);
    // S007 at each request that equals another, both of them, or that is the start of another.
    let two = |a: &str, b: &str| device(&format!("[[net.device.reply]]\nrequest = \"{a}\"\n[[net.device.reply]]\nrequest = \"{b}\""));
    assert_eq!(codes(&two("AB", "AB")), [("net:S007", 4, 11), ("net:S007", 6, 11)]);
    assert_eq!(codes(&two("ABC", "AB")), [("net:S007", 6, 11)]);
    assert_eq!(codes(&two("AB", "ABC")), [("net:S007", 4, 11)]);
    assert_eq!(codes(&two("AB", "AC")), []);
    assert_eq!(
        codes("[[net.device]]\nendpoint = \"udp:h:1\"\nfaults = { close_ms = 5, refused = false }\n"),
        [("net:S008", 3, 23), ("net:S008", 3, 36)]
    );
    assert_eq!(codes(&device("faults = { refused = 1 }")), [("net:S002", 3, 22)]);
    assert_eq!(codes(&device("faults = { hang = 1 }")), [("net:S001", 3, 12)]);
    assert_eq!(codes(&device("faults = { hang_ms = -1 }")), [("net:S005", 3, 22)]);
    assert_eq!(codes(&device("reply = 1")), [("net:S002", 3, 9)]);
    assert_eq!(codes("[net]\ndevice = 1\n"), [("net:S002", 2, 10)]);
    assert_eq!(codes("[net]\ndevices = []\n"), [("net:S001", 2, 1)]);
    let nearest = read("[net]\ndevise = []\n", 64).unwrap().judge(&[&Net]).err().unwrap();
    assert_eq!(nearest[0].fix.as_deref(), Some("device"));
}

#[test]
fn every_problem_is_said_not_the_first_only() {
    let text =
        "[[net.device]]\nendpoint = \"tcp:a:1\"\nlatency = 1\nlatency_ms = -5\n[[net.device]]\nendpoint = \"udp:b:1\"\nfaults = { close_ms = 1 }\n";
    let found: Vec<&str> = codes(text).iter().map(|(code, _, _)| *code).collect();
    assert_eq!(found, ["net:S001", "net:S005", "net:S008"]);
}

// ---- What the reviews found ----

#[test]
fn every_request_that_equals_or_starts_another_is_refused() {
    // README, `net:S007`, by hand: of `A`, `A`, `AB`, the first `A` equals the second and starts `AB`, the second the same:
    // both are said; `AB` starts nothing and equals nothing.
    let text = "[[net.device]]\nendpoint = \"tcp:h:1\"\n[[net.device.reply]]\nrequest = \"A\"\n[[net.device.reply]]\nrequest = \"A\"\n\
                [[net.device.reply]]\nrequest = \"AB\"\n";
    assert_eq!(codes(text), [("net:S007", 4, 11), ("net:S007", 6, 11)]);
}

#[test]
fn a_device_with_a_wrong_endpoint_still_has_its_other_keys_judged() {
    // README, "Simulation": the run is refused if anything is wrong, and every problem is said: an endpoint missing, wrong
    // or given again, and a latency out of its range and a key the format does not have in the same device.
    assert_eq!(codes("[[net.device]]\nlatency_ms = -1\nlatency = 1\n"), [("net:S003", 1, 1), ("net:S001", 3, 1), ("net:S005", 2, 14)]);
    assert_eq!(codes("[[net.device]]\nendpoint = \"serial:COM3\"\nlatency_ms = -1\n"), [("net:S003", 2, 12), ("net:S005", 3, 14)]);
    let again = "[[net.device]]\nendpoint = \"tcp:h:1\"\n[[net.device]]\nendpoint = \"tcp:h:1\"\nfaults = { hang_ms = -1 }\n";
    assert_eq!(codes(again), [("net:S004", 4, 12), ("net:S005", 5, 22)]);
}

#[test]
fn a_message_shows_the_start_of_a_long_endpoint() {
    // commands.md 5 cuts what a message repeats of its input: here the start of the endpoint, 64 code points then `…`, so
    // that 3,000 diagnostics on an endpoint of 253 characters, the longest name, stay small; and an endpoint of 100,000
    // characters, which is no host (`net:S003`), too.
    let name = "a".repeat(253);
    let host = "a".repeat(100_000);
    let replies: String = (0..3000).map(|_| "[[net.device.reply]]\nrequest = \"\"\n").collect();
    let file = format!(
        "[[net.device]]\nendpoint = \"tcp:{name}:1\"\n{replies}[[net.device]]\nendpoint = \"tcp:{name}:1\"\n[[net.device]]\nendpoint = \"tcp:{host}:1\"\n"
    );
    let found = judged(&file).err().unwrap();
    assert_eq!(found.len(), 3002, "3,000 empty requests, the endpoint given again, and one that is no host");
    assert!(found.iter().all(|diagnostic| diagnostic.message.chars().count() < 200), "{:?}", found.first());
    // A host the program gives is cut the same way.
    let mut driver = Driver::simulated("[[net.device]]\nendpoint = \"tcp:h:1\"\n");
    let (_, _, message) = driver.connect(&host, 1, MS).unwrap_err();
    assert!(message.chars().count() < 200, "{message}");
}

#[test]
fn ten_thousand_requests_match_in_any_order_of_the_file() {
    // io.md 6, by hand: 10,000 requests written backwards, `R09999\n` first, each answered by its number; one write of all of
    // them in order gets every answer in order; a write of the start of one waits for the rest.
    let replies: String = (0..10_000).rev().map(|n| format!("[[net.device.reply]]\nrequest = \"R{n:05}\\n\"\nresponse = \"{n}\\n\"\n")).collect();
    let mut driver = Driver::simulated(&format!("[[net.device]]\nendpoint = \"tcp:h:1\"\n{replies}"));
    let connection = driver.connect("h", 1, 100 * MS).unwrap();
    let all: String = (0..10_000).map(|n| format!("R{n:05}\n")).collect();
    driver.write(connection, all.as_bytes()).unwrap();
    let expected: String = (0..10_000).map(|n| format!("{n}\n")).collect();
    assert_eq!(driver.read(connection, expected.len() as i32), Ok(expected.into_bytes()));
    driver.write(connection, b"R0004").unwrap();
    assert_eq!(driver.read_available(connection, 100), Ok(Vec::new()), "the start of `R00042\\n` waits");
    driver.write(connection, b"2\n").unwrap();
    assert_eq!(driver.read_line(connection), Ok("42".to_owned()));
    // A file with 10,000 exchanges in one device is accepted; with 10,001, `net:S009` (README, "Simulation").
    let more = format!("[[net.device]]\nendpoint = \"tcp:h:1\"\n{replies}[[net.device.reply]]\nrequest = \"X\"\n");
    assert_eq!(codes(&more), [("net:S009", 3, 1)]);
}

#[test]
fn the_search_of_a_terminator_goes_on_where_it_stopped() {
    // `read_until` (README): the bytes up to the terminator, included. By hand: `aab` is first whole at bytes 3 to 5 of
    // `aaaab…`, after the bytes come in three parts; and a terminator of 20,000 `a` then `b` at the end of 1 MiB of `a`, which
    // a search that starts again at each byte, or after each part, would take minutes to find.
    let need = Need::Until(Terminator::new(b"aab".to_vec()), 100);
    let mut scan = Scan::default();
    assert_eq!(need.judge(b"aa", &mut scan), Got::More);
    assert_eq!(need.judge(b"aaaa", &mut scan), Got::More);
    assert_eq!(need.judge(b"aaaabxx", &mut scan), Got::Take(5));
    let mut terminator = vec![b'a'; 20_000];
    terminator.push(b'b');
    let mut waiting = vec![b'a'; 1 << 20];
    let need = Need::Until(Terminator::new(terminator), 2 << 20);
    let mut scan = Scan::default();
    assert_eq!(need.judge(&waiting, &mut scan), Got::More);
    waiting.push(b'b');
    assert_eq!(need.judge(&waiting, &mut scan), Got::Take((1 << 20) + 1));
    // A simulated read that waits across two responses finds a terminator split between them.
    let file = "[[net.device]]\nendpoint = \"tcp:h:1\"\nlatency_ms = 5\n[[net.device.reply]]\nrequest = \"1\"\nresponse = \"xx;\"\n\
                [[net.device.reply]]\nrequest = \"2\"\nresponse = \";yy;;\"\n";
    let mut driver = Driver::simulated(file);
    let connection = driver.connect("h", 1, 100 * MS).unwrap();
    driver.write(connection, b"1").unwrap();
    driver.now = 3 * MS;
    driver.write(connection, b"2").unwrap();
    assert_eq!(driver.read_until(connection, b";;", 100), Ok(b"xx;;".to_vec()), "ready at 5 ms, then at 8 ms");
    assert_eq!(driver.now, 8 * MS);
}

#[test]
fn a_reset_closes_a_real_connection() {
    // Decision of the owner: a reset during a call is `io.closed`, and the connection is closed then, as `describe` says.
    // By hand: a server that closes with bytes it did not read sends a reset (RFC 1122, 4.2.2.13).
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (written, wrote) = mpsc::channel::<()>();
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        wrote.recv_timeout(Duration::from_secs(10)).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        drop(stream);
    });
    let mut driver = Driver::real();
    let connection = driver.connect("127.0.0.1", port, GENEROUS).unwrap();
    driver.write(connection, b"not read").unwrap();
    written.send(()).unwrap();
    let (category, code, _) = driver.read(connection, 1).unwrap_err();
    assert_eq!(category, "io.closed");
    assert_ne!(code, 0, "the code of the system");
    assert!(!driver.run.describe(connection).unwrap().open);
    let (_, code, message) = driver.write(connection, b"x").unwrap_err();
    assert_eq!((code, message.contains("the connection is closed")), (0, true), "{message}");
}

#[test]
fn a_real_write_that_cannot_go_times_out() {
    // README, "Writing": `io.timeout` when the system cannot take the bytes within the timeout, counted from the start of
    // the call. A server that never reads; the bound is loose: ten times the timeout.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (stop, stopped) = mpsc::channel::<()>();
    std::thread::spawn(move || {
        let (_stream, _) = listener.accept().unwrap();
        let _ = stopped.recv_timeout(Duration::from_secs(30));
    });
    let mut driver = Driver::real();
    let connection = driver.connect("127.0.0.1", port, 300 * MS).unwrap();
    // The system takes what fits in its buffers first, which on loopback can be tens of MiB: writes of 16 MiB go on until one
    // cannot, at most 1 GiB in all.
    let data = vec![0; 16 << 20];
    let mut outcome = Ok(());
    for _ in 0..64 {
        let started = Instant::now();
        outcome = driver.write(connection, &data);
        assert!(started.elapsed() < Duration::from_secs(3), "{:?}", started.elapsed());
        if outcome.is_err() {
            break;
        }
    }
    assert_eq!(category(outcome).0, "io.timeout");
    let _ = stop.send(());
}

#[test]
fn a_real_connect_with_a_timeout_of_0_still_tries() {
    // As a simulated device connects at once whatever the timeout, a real one is tried once (decision of the coordinator
    // of this change: 1 ms).
    let server = server(b"hi\n", false);
    let mut driver = Driver::real();
    let connection = driver.connect("127.0.0.1", server.port, 0).unwrap();
    assert!(driver.run.describe(connection).unwrap().open);
}

#[test]
fn a_host_is_an_address_or_a_name() {
    // Decision of the owner (README, "Functions"): an IPv4 or IPv6 address, IPv6 with or without brackets, or a name of at
    // most 253 bytes of ASCII letters, digits, `-`, `.` and `_`; anything else is `net.host_not_found` at once, code 0.
    for host in ["10.0.0.5", "fe80::1", "[fe80::1]", "dmm.local", "a_b-c.d", &"a".repeat(253)] {
        assert!(of_call(false, host, 1).is_ok(), "{host}");
    }
    for host in ["", "a b", "a\nb", "h:1", "[h]", "[10.0.0.5]", "h]", "[fe80::1", "Ünï", "a/b", &"a".repeat(254)] {
        assert!(matches!(of_call(true, host, 1), Err(NativeOutcome::Fail { category: "net.host_not_found", code: 0, .. })), "{host:?}");
    }
    // io.md 3: the name first. Under `--sim`, at once, not after the timeout of the open; for an agent, no question.
    let mut driver = Driver::simulated("[[net.device]]\nendpoint = \"tcp:h:1\"\n");
    assert_eq!(category(driver.connect("a b", 1, 1_000 * MS)), ("net.host_not_found".to_owned(), 0));
    assert_eq!(driver.now, 0);
    let window = Arc::new(Window { yes: true, asked: Mutex::default() });
    let context = agent(Allowed { consent: Some(Arc::clone(&window) as Arc<dyn Consent>), ..Allowed::default() });
    let mut driver = Driver::with(None, Mode::RealTime, context);
    assert_eq!(category(driver.open_udp("h:1", 5, MS)), ("net.host_not_found".to_owned(), 0));
    assert!(window.asked.lock().unwrap().is_empty(), "no question");
    // In the file, an endpoint whose host is no host is `net:S003`.
    assert_eq!(codes("[[net.device]]\nendpoint = \"tcp:a b:1\"\n"), [("net:S003", 2, 12)]);
    assert_eq!(codes(&format!("[[net.device]]\nendpoint = \"tcp:{}:1\"\n", "a".repeat(254))), [("net:S003", 2, 12)]);
}

#[test]
fn what_waits_unread_is_bounded() {
    // Decision of the owner (README, "Simulation"), by hand: a response of 65,536 bytes, 300 requests in one write: 256
    // responses fill 16 MiB exactly (256 x 65,536 = 16,777,216), the 44 others are dropped; the write succeeds and has its
    // entry; a read takes the 16 MiB, then nothing more is there.
    let response = "x".repeat(65_536);
    let file = format!("[[net.device]]\nendpoint = \"tcp:h:1\"\n[[net.device.reply]]\nrequest = \"?\"\nresponse = \"{response}\"\n");
    let mut driver = Driver::simulated(&file);
    let connection = driver.connect("h", 1, 100 * MS).unwrap();
    assert_eq!(driver.write(connection, &[b'?'; 300]), Ok(()));
    assert_eq!(driver.read(connection, 16 * 1024 * 1024).map(|bytes| bytes.len()), Ok(16 * 1024 * 1024));
    assert_eq!(driver.read_available(connection, 65536), Ok(Vec::new()));
    let entries = driver.entries("tcp:h:1");
    assert_eq!(entries.first().map(|(sent, text, _)| (*sent, text.len())), Some((true, 300)));
    // Once read, the room is there again: one more request is answered.
    driver.write(connection, b"?").unwrap();
    assert_eq!(driver.read(connection, 65_536).map(|bytes| bytes.len()), Ok(65_536));
    // Over UDP the bytes of the datagrams waiting: 256 kept, the 257th dropped.
    let file = format!("[[net.device]]\nendpoint = \"udp:h:1\"\n[[net.device.reply]]\nrequest = \"?\"\nresponse = \"{response}\"\n");
    let mut driver = Driver::simulated(&file);
    let socket = driver.open_udp("h", 1, 10 * MS).unwrap();
    for _ in 0..257 {
        driver.send(socket, b"?").unwrap();
    }
    for _ in 0..256 {
        assert_eq!(driver.receive(socket, 65_536).map(|bytes| bytes.len()), Ok(65_536));
    }
    assert_eq!(category(driver.receive(socket, 65_536)), ("io.timeout".to_owned(), 0));
}
