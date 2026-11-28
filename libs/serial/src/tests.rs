//! `serial` against `libs/serial/README.md`, `libs/net/README.md` ("Writing", "Reading") and `docs/io.md` (sections 2, 3,
//! 6, 7 and 8), and the decisions of the owner of 2026-11-20 for issue #234. Every expected value is worked out by hand from
//! those rules, and each test says which; none comes from the output of `serial`. Only simulated devices run here, on the
//! clock of a hand driver, as the scheduler steps them (`docs/architecture.md`, 10.6): a real port is tried by hand (io.md 7).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::HashMap;
use std::io::ErrorKind;
use std::sync::Arc;

use labcraft_runtime::native::{
    CallContext, CallId, Context, Key, LOG_BUDGET, LibraryDiagnostic, LibraryRun, LogBudget, Mode, NativeLibrary, NativeOutcome, Setting,
    SettingDiff, Start, Step,
};
use labcraft_runtime::simulation::read;
use labcraft_runtime::{Handle, Value, VariantValue};

use crate::Serial;
use crate::endpoint::{of_call, of_file};

const MS: i64 = 1_000_000;
const S: i64 = 1_000 * MS;

/// The diagnostics of a file, or nothing when it is accepted.
fn judged(text: &str) -> Result<Option<labcraft_runtime::native::Simulation>, Vec<LibraryDiagnostic>> {
    let file = read(text, 64).map_err(|diagnostic| vec![diagnostic])?;
    let mut simulations = file.judge(&[&Serial])?;
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
    /// The `Blocking` works it ran: the real ones, the list of the system's ports among them.
    jobs: usize,
}

type Failed = (String, i64, String);

/// The settings of an open: baud, data bits, and the indices of parity, stop bits and flow in their enumerations.
#[derive(Clone, Copy)]
struct Opened {
    baud: u32,
    data_bits: u8,
    parity: u32,
    stop_bits: u32,
    flow: u32,
    timeout: i64,
    settle: i64,
}

/// The defaults of `open` (README): 9600, 8, none, one, none, 2 s, 0 s.
const DEFAULTS: Opened = Opened { baud: 9600, data_bits: 8, parity: 0, stop_bits: 0, flow: 0, timeout: 2 * S, settle: 0 };

fn variant(index: u32) -> Value {
    Value::Variant(Arc::new(VariantValue { index, values: Box::new([]) }))
}

impl Driver {
    fn with(text: Option<&str>, mode: Mode, context: Context) -> Self {
        let simulation = text.map(|text| judged(text).unwrap());
        let run = Serial.start_run(Start {
            library: 0,
            simulation: simulation.as_ref().and_then(Option::as_ref),
            budget: LogBudget::new(LOG_BUDGET),
            mode,
            context: &context,
        });
        Driver { run, now: 0, tasks: Vec::new(), holders: HashMap::new(), jobs: 0 }
    }

    /// A run with `--sim` and this file, in deterministic mode; the engine gives the endpoints of the file.
    fn simulated(text: &str) -> Self {
        let mut endpoints = judged(text).unwrap().map(|simulation| Serial.endpoints(&simulation)).unwrap_or_default();
        endpoints.sort_unstable();
        endpoints.dedup();
        let endpoints = endpoints.into();
        Driver::with(Some(text), Mode::Deterministic, Context { simulation_file: true, endpoints, ..Context::default() })
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
                    self.jobs += 1;
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

    fn call(&mut self, function: &str, inputs: Vec<Value>) -> (NativeOutcome, i64) {
        let index = self.start(function, inputs);
        self.go();
        self.tasks[index].done.take().unwrap()
    }

    fn open_with(&mut self, port: &str, opened: Opened) -> Result<Handle, Failed> {
        let inputs = vec![
            Value::string(port),
            Value::U32(opened.baud),
            Value::U8(opened.data_bits),
            variant(opened.parity),
            variant(opened.stop_bits),
            variant(opened.flow),
            Value::Duration(opened.timeout),
            Value::Duration(opened.settle),
            Value::NONE,
        ];
        match self.call("open", inputs).0 {
            NativeOutcome::Outputs(outputs) => match outputs.first() {
                Some(Value::Handle(handle)) => Ok(*handle),
                other => panic!("{other:?}"),
            },
            NativeOutcome::Fail { category, code, message } => Err((category.to_owned(), code, message)),
        }
    }

    fn open(&mut self, port: &str) -> Result<Handle, Failed> {
        self.open_with(port, Opened { timeout: 100 * MS, ..DEFAULTS })
    }

    fn write_line(&mut self, port: Handle, text: &str) -> Result<(), Failed> {
        done(self.call("write_line", vec![Value::Handle(port), Value::string(text), Value::string("\n"), Value::NONE]).0).map(|_| ())
    }

    fn write(&mut self, port: Handle, data: &[u8]) -> Result<(), Failed> {
        done(self.call("write", vec![Value::Handle(port), Value::Bytes(data.to_vec().into()), Value::NONE]).0).map(|_| ())
    }

    fn read(&mut self, port: Handle, count: i32) -> Result<Vec<u8>, Failed> {
        done(self.call("read", vec![Value::Handle(port), Value::I32(count), Value::NONE]).0).map(|outputs| match outputs.first() {
            Some(Value::Bytes(bytes)) => bytes.to_vec(),
            other => panic!("{other:?}"),
        })
    }

    fn read_line(&mut self, port: Handle) -> Result<String, Failed> {
        let outcome = self.call("read_line", vec![Value::Handle(port), Value::string("\n"), Value::I32(65536), Value::NONE]).0;
        done(outcome).map(|outputs| match outputs.first() {
            Some(Value::String(text)) => text.to_string(),
            other => panic!("{other:?}"),
        })
    }

    fn close(&mut self, port: Handle) -> Result<(), Failed> {
        done(self.call("close", vec![Value::Handle(port), Value::NONE]).0).map(|_| ())
    }

    fn list(&mut self) -> Result<Vec<String>, Failed> {
        done(self.call("list", vec![Value::NONE]).0).map(|outputs| match outputs.first() {
            Some(Value::Array(items)) => items
                .items()
                .iter()
                .map(|item| match item {
                    Value::String(text) => text.to_string(),
                    other => panic!("{other:?}"),
                })
                .collect(),
            other => panic!("{other:?}"),
        })
    }
}

fn done(outcome: NativeOutcome) -> Result<Vec<Value>, Failed> {
    match outcome {
        NativeOutcome::Outputs(outputs) => Ok(outputs),
        NativeOutcome::Fail { category, code, message } => Err((category.to_owned(), code, message)),
    }
}

fn category<T: std::fmt::Debug>(result: Result<T, Failed>) -> (String, i64) {
    match result {
        Err((category, code, _)) => (category, code),
        Ok(value) => panic!("an error was expected, not {value:?}"),
    }
}

/// The Arduino board of io.md 6.
const ARDUINO: &str = r#"
[[serial.device]]
endpoint = "serial:COM3"
latency_ms = 2

[[serial.device.reply]]
request = "LED ON\n"
response = "OK\r\n"

[[serial.device.reply]]
request = "TEMP?\n"
response = "23.5\r\n"
"#;

#[test]
fn the_arduino_of_io_md_6_answers_after_its_latency() {
    // The issue: `write_line("TEMP?")` then `read_line` gives `23.5`, ready 2 ms after the write on the clock; `LED ON\n`
    // gives `OK`; `read_line` removes `\r\n` (libs/net, `read_line`).
    let mut driver = Driver::simulated(ARDUINO);
    let port = driver.open("COM3").unwrap();
    driver.write_line(port, "TEMP?").unwrap();
    assert_eq!(driver.now, 0);
    assert_eq!(driver.read_line(port), Ok("23.5".to_owned()));
    assert_eq!(driver.now, 2 * MS);
    driver.write(port, b"LED ON\n").unwrap();
    assert_eq!(driver.read_line(port), Ok("OK".to_owned()));
    assert_eq!(driver.now, 4 * MS);
    // The report: one entry per write, its `sent`, and one per read, its `answer`, by the endpoint (commands.md 7).
    let logs = driver.run.instruments();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].resource, "serial:COM3");
    let entries: Vec<(i64, Option<&str>, Option<&str>)> =
        logs[0].commands.iter().map(|c| (c.at, c.sent.as_ref().map(|t| t.kept.as_str()), c.answer.as_ref().map(|t| t.kept.as_str()))).collect();
    assert_eq!(
        entries,
        [(0, Some("TEMP?\n"), None), (2 * MS, None, Some("23.5\r\n")), (2 * MS, Some("LED ON\n"), None), (4 * MS, None, Some("OK\r\n"))]
    );
    assert!(logs[0].settings.is_empty(), "opened with the settings it expects: any");
}

#[test]
fn an_unknown_request_without_unknown_gets_no_answer() {
    // io.md 6: what matches nothing is forgotten; without `unknown`, nothing answers, and the read fails `io.timeout` after the
    // timeout of the port (100 ms), code 0.
    let mut driver = Driver::simulated(ARDUINO);
    let port = driver.open("COM3").unwrap();
    driver.write_line(port, "HELLO").unwrap();
    assert_eq!(category(driver.read_line(port)), ("io.timeout".to_owned(), 0));
    assert_eq!(driver.now, 100 * MS);
}

#[test]
fn a_device_opened_with_other_settings_never_answers_and_the_report_says_why() {
    // The issue and decision 33 of the owner: a device with `baud = 115200` opened at 9600 takes the write (its entry is in the
    // report) and gives no response, so a read fails `io.timeout`; the entry of its endpoint has `settings`, with the time of
    // the open and only the settings that differ. At 115200 it answers.
    let file = "[[serial.device]]\nendpoint = \"serial:COM3\"\nbaud = 115200\nparity = \"none\"\n\
                [[serial.device.reply]]\nrequest = \"TEMP?\\n\"\nresponse = \"23.5\\r\\n\"\n";
    let mut driver = Driver::simulated(file);
    driver.now = 5 * MS;
    let port = driver.open("COM3").unwrap();
    driver.write_line(port, "TEMP?").unwrap();
    assert_eq!(category(driver.read_line(port)), ("io.timeout".to_owned(), 0));
    driver.close(port).unwrap();
    let logs = driver.run.instruments();
    assert_eq!(logs[0].commands.len(), 1, "the write only");
    assert_eq!(logs[0].settings.len(), 1);
    assert_eq!(logs[0].settings[0].at, 5 * MS);
    assert_eq!(logs[0].settings[0].differ, [SettingDiff { name: "baud", expected: Setting::Integer(115_200), opened: Setting::Integer(9600) }]);
    let fast = driver.open_with("COM3", Opened { baud: 115_200, timeout: 100 * MS, ..DEFAULTS }).unwrap();
    driver.write_line(fast, "TEMP?").unwrap();
    assert_eq!(driver.read_line(fast), Ok("23.5".to_owned()));
    assert_eq!(driver.run.instruments()[0].settings.len(), 1, "an open with the right settings adds nothing");
    // Several settings that differ, in the order of `open`; an endpoint with no write or read is listed for them.
    let file = "[[serial.device]]\nendpoint = \"/dev/ttyUSB0\"\n";
    assert_eq!(codes(file), [("serial:S003", 2, 12)], "`serial:` is required");
    let file = "[[serial.device]]\nendpoint = \"serial:/dev/ttyUSB0\"\ndata_bits = 7\nparity = \"even\"\nstop_bits = 2\nflow = \"hardware\"\n";
    let mut driver = Driver::simulated(file);
    driver.open("/dev/ttyUSB0").unwrap();
    let logs = driver.run.instruments();
    assert!(logs[0].commands.is_empty());
    let differ: Vec<&str> = logs[0].settings[0].differ.iter().map(|differ| differ.name).collect();
    assert_eq!(differ, ["data_bits", "parity", "stop_bits", "flow"]);
    assert_eq!(logs[0].settings[0].differ[1], SettingDiff { name: "parity", expected: Setting::Text("even"), opened: Setting::Text("none") });
}

#[test]
fn list_under_sim_gives_the_serial_endpoints_of_the_whole_file() {
    // The issue: under `--sim`, a file with `serial:COM3` and `serial:/dev/ttyUSB0` lists `["/dev/ttyUSB0", "COM3"]`, code point
    // order (`/` is 0x2F, below `C`, 0x43), without the prefix, each once. The engine gives the endpoints of every library of
    // the file, sorted and each once (`Context::endpoints`): those that are not serial are left out.
    let endpoints = ["serial:/dev/ttyUSB0", "serial:COM3", "tcp:10.0.0.7:502"].map(str::to_owned).into();
    let context = Context { simulation_file: true, endpoints, ..Context::default() };
    let mut driver = Driver::with(None, Mode::Deterministic, context);
    assert_eq!(driver.list(), Ok(vec!["/dev/ttyUSB0".to_owned(), "COM3".to_owned()]));
    assert_eq!(driver.jobs, 0, "the system is not asked for its ports");
    // A file with no serial endpoint: an empty list, never the ports of the system.
    let mut driver = Driver::with(None, Mode::RealTime, Context { simulation_file: true, ..Context::default() });
    assert_eq!(driver.list(), Ok(Vec::new()));
    assert_eq!(driver.jobs, 0, "in real time either");
    // Through the file itself.
    let mut driver = Driver::simulated(&format!("{ARDUINO}[[serial.device]]\nendpoint = \"serial:/dev/ttyUSB0\"\n"));
    assert_eq!(driver.list(), Ok(vec!["/dev/ttyUSB0".to_owned(), "COM3".to_owned()]));
}

#[test]
fn a_port_open_in_the_run_is_busy() {
    // The issue: `open("com3")` and `open("COM3")` are one port (io.md 2): the second, while the first is open, fails
    // `serial.port_busy`, code 0, at once; once closed, it opens again.
    let mut driver = Driver::simulated(ARDUINO);
    let first = driver.open("com3").unwrap();
    assert_eq!(driver.run.describe(first).unwrap().designates, "serial:COM3");
    assert_eq!(category(driver.open("COM3")), ("serial.port_busy".to_owned(), 0));
    assert_eq!(driver.now, 0);
    driver.close(first).unwrap();
    assert!(driver.open("COM3").is_ok());
}

#[test]
fn settle_waits_on_the_clock_of_the_run() {
    // README, `settle`: `open` waits `settle` on the clock of the run, then empties the input, then gives the port. By hand:
    // opened at 1 ms with `settle` 2 s, the port is given at 2.001 s; a request then is answered 2 ms later.
    let mut driver = Driver::simulated(ARDUINO);
    driver.now = MS;
    let port = driver.open_with("COM3", Opened { settle: 2 * S, timeout: 100 * MS, ..DEFAULTS }).unwrap();
    assert_eq!(driver.now, 2 * S + MS);
    driver.write_line(port, "TEMP?").unwrap();
    assert_eq!(driver.read_line(port), Ok("23.5".to_owned()));
    assert_eq!(driver.now, 2 * S + 3 * MS);
}

#[test]
fn a_real_name_in_deterministic_mode_without_sim_needs_real_time() {
    // io.md 3: without `--sim`, in deterministic mode, `io.needs_real_time` at once, code 0, before any right is judged.
    let mut driver = Driver::with(None, Mode::Deterministic, Context::default());
    let (category, code, message) = driver.open("COM3").unwrap_err();
    assert_eq!((category.as_str(), code), ("io.needs_real_time", 0));
    assert!(message.contains("serial:COM3"), "{message}");
    assert_eq!(driver.now, 0);
}

#[test]
fn with_sim_a_port_the_file_does_not_describe_is_not_found_after_the_timeout() {
    // io.md 3: with `--sim`, the file is the whole bench: `serial.port_not_found` after the timeout of the open.
    let mut driver = Driver::simulated(ARDUINO);
    assert_eq!(category(driver.open("COM4")), ("serial.port_not_found".to_owned(), 0));
    assert_eq!(driver.now, 100 * MS);
}

#[test]
fn a_port_closed_or_never_opened_is_io_closed() {
    // As `net` (decision of the owner, 2026-11-19): a call on a port closed, or never opened, is `io.closed`; `close` on
    // either succeeds.
    let mut driver = Driver::simulated(ARDUINO);
    let (_, _, message) = driver.read(Handle::INVALID, 1).unwrap_err();
    assert!(message.contains("never opened") && message.contains("serial.port"), "{message}");
    assert_eq!(driver.close(Handle::INVALID), Ok(()));
    let port = driver.open("COM3").unwrap();
    assert_eq!(driver.close(port), Ok(()));
    assert_eq!(driver.close(port), Ok(()));
    let (category, code, message) = driver.write(port, b"x").unwrap_err();
    assert_eq!((category.as_str(), code, message.as_str()), ("io.closed", 0, "serial:COM3: the port is closed"));
    let info = driver.run.describe(port).unwrap();
    assert_eq!((info.kind, info.open), ("port", false));
}

#[test]
fn a_device_that_hangs_at_hang_ms() {
    // io.md 6: from `hang_ms` the device takes no request and gives no response; a write still succeeds.
    let file = "[[serial.device]]\nendpoint = \"serial:COM3\"\nlatency_ms = 20\nfaults = { hang_ms = 30 }\n\
                [[serial.device.reply]]\nrequest = \"A\"\nresponse = \"a\"\n";
    let mut driver = Driver::simulated(file);
    let port = driver.open("COM3").unwrap();
    driver.write(port, b"A").unwrap();
    assert_eq!(driver.read(port, 1), Ok(b"a".to_vec()));
    assert_eq!(driver.now, 20 * MS);
    driver.write(port, b"A").unwrap();
    assert_eq!(category(driver.read(port, 1)), ("io.timeout".to_owned(), 0), "ready at 40 ms, after the hang");
    assert_eq!(driver.now, 120 * MS);
}

#[test]
fn the_arguments_of_open_are_judged() {
    // README, `open`: `baud` above 0 and `data_bits` 5 to 8, else `io.invalid_argument`, code 0, at once; a name that is no
    // name (empty, a control character) is `serial.port_not_found` at once.
    let mut driver = Driver::simulated(ARDUINO);
    for opened in [Opened { baud: 0, ..DEFAULTS }, Opened { data_bits: 9, ..DEFAULTS }, Opened { data_bits: 4, ..DEFAULTS }] {
        assert_eq!(category(driver.open_with("COM3", opened)), ("io.invalid_argument".to_owned(), 0));
    }
    for opened in [5, 6, 7, 8] {
        let port = driver.open_with("COM3", Opened { data_bits: opened, ..DEFAULTS }).unwrap();
        driver.close(port).unwrap();
    }
    assert_eq!(category(driver.open("")), ("serial.port_not_found".to_owned(), 0));
    assert_eq!(category(driver.open("COM\u{0}3")), ("serial.port_not_found".to_owned(), 0));
    assert_eq!(driver.now, 0);
}

#[test]
fn a_name_compares_by_its_form() {
    // io.md 2: `COM<n>`, n from 1, without case, written upper case; any other name exactly.
    assert_eq!(of_call("com3").unwrap().text, "serial:COM3");
    assert_eq!(of_call("Com12").unwrap().text, "serial:COM12");
    assert_eq!(of_call("com3").unwrap().given, "com3");
    assert_eq!(of_call("COM0").unwrap().text, "serial:COM0", "n from 1: another name");
    assert_eq!(of_call("com03").unwrap().text, "serial:com03", "no zero in front: another name");
    assert_eq!(of_call("/dev/ttyUSB0").unwrap().text, "serial:/dev/ttyUSB0");
    assert_eq!(of_call("/dev/TTYusb0").unwrap().text, "serial:/dev/TTYusb0");
    assert_eq!(of_file("serial:com3"), Some(("serial:COM3".to_owned(), ())));
    assert_eq!(of_file("serial:"), None);
    assert_eq!(of_file("COM3"), None);
    assert_eq!(codes("[[serial.device]]\nendpoint = \"serial:COM3\"\n[[serial.device]]\nendpoint = \"serial:com3\"\n"), [("serial:S004", 4, 12)]);
}

#[test]
fn the_category_of_a_real_error_of_the_open() {
    // Decision of the coordinator (issue #234, point 7): not found, busy or access denied (Windows gives a second open as
    // access denied, io.md 11), a setting the system refuses, a timeout; any other error is not found.
    use crate::real::open_category;
    assert_eq!(open_category(ErrorKind::NotFound), "serial.port_not_found");
    assert_eq!(open_category(ErrorKind::PermissionDenied), "serial.port_busy");
    assert_eq!(open_category(ErrorKind::ResourceBusy), "serial.port_busy");
    assert_eq!(open_category(ErrorKind::InvalidInput), "io.invalid_argument");
    assert_eq!(open_category(ErrorKind::TimedOut), "io.timeout");
    assert_eq!(open_category(ErrorKind::Other), "serial.port_not_found");
}

#[test]
fn a_file_says_its_devices_and_their_endpoints() {
    let simulation = judged(ARDUINO).unwrap().unwrap();
    assert_eq!(Serial.resources(&simulation), ["serial:COM3"]);
    assert_eq!(Serial.endpoints(&simulation), ["serial:COM3"]);
    let described = format!("{:?}", Serial.describe(&simulation).unwrap());
    assert!(described.contains("Text(\"LED ON\\n\")") && described.contains("\"baud\", Null"), "{described}");
}

// ---- The section of a simulation file (README, "Simulation") ----

/// One file per row of the table of the README, in `tests/refused/`, each refused with its code and that code only.
#[test]
fn each_refused_file_gives_its_code() {
    let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("refused");
    let mut seen = Vec::new();
    for entry in std::fs::read_dir(folder).unwrap() {
        let path = entry.unwrap().path();
        let code = format!("serial:{}", path.file_stem().unwrap().to_str().unwrap());
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
    let device = |body: &str| format!("[[serial.device]]\nendpoint = \"serial:COM3\"\n{body}\n");
    // Line and column by hand: line 3 is the first line of `body`.
    assert_eq!(codes(&device("parity = \"mark\"")), [("serial:S005", 3, 10)]);
    assert_eq!(codes(&device("parity = 1")), [("serial:S002", 3, 10)]);
    assert_eq!(codes(&device("data_bits = 9")), [("serial:S005", 3, 13)]);
    assert_eq!(codes(&device("data_bits = 5")), []);
    assert_eq!(codes(&device("baud = 0")), [("serial:S005", 3, 8)]);
    assert_eq!(codes(&device("baud = 4294967296")), [("serial:S005", 3, 8)]);
    assert_eq!(codes(&device("baud = 4294967295")), []);
    assert_eq!(codes(&device("baud = 9600.0")), [("serial:S002", 3, 8)]);
    assert_eq!(codes(&device("stop_bits = 3")), [("serial:S005", 3, 13)]);
    assert_eq!(codes(&device("stop_bits = \"2\"")), [("serial:S002", 3, 13)]);
    assert_eq!(codes(&device("flow = \"rts/cts\"")), [("serial:S005", 3, 8)]);
    assert_eq!(codes(&device("flow = \"software\"")), []);
    // Decision 36 of the owner: `hang_ms` is the only fault; `refused` and `close_ms` are `net`'s.
    assert_eq!(codes(&device("faults = { refused = true }")), [("serial:S001", 3, 12)]);
    assert_eq!(codes(&device("faults = { close_ms = 5 }")), [("serial:S001", 3, 12)]);
    let nearest = judged(&device("faults = { hang_m = 5 }")).err().unwrap();
    assert_eq!((nearest[0].code, nearest[0].fix.as_deref()), ("serial:S001", Some("hang_ms")));
    assert_eq!(codes(&device("faults = { hang_ms = -1 }")), [("serial:S005", 3, 22)]);
    assert_eq!(codes(&device("latency_ms = 86400001")), [("serial:S005", 3, 14)]);
    assert_eq!(codes("[[serial.device]]\nlatency_ms = 1\n"), [("serial:S003", 1, 1)], "no endpoint");
    assert_eq!(codes("[[serial.device]]\nendpoint = \"tcp:h:1\"\n"), [("serial:S003", 2, 12)]);
    assert_eq!(codes("[[serial.device]]\nendpoint = \"serial:a\\u0001\"\n"), [("serial:S003", 2, 12)], "a control character");
    assert_eq!(codes(&device("[[serial.device.reply]]\nrequest = \"\"")), [("serial:S006", 4, 11)]);
    let two = |a: &str, b: &str| device(&format!("[[serial.device.reply]]\nrequest = \"{a}\"\n[[serial.device.reply]]\nrequest = \"{b}\""));
    assert_eq!(codes(&two("AB", "ABC")), [("serial:S007", 4, 11)]);
    assert_eq!(codes("[serial]\ndevices = []\n"), [("serial:S001", 2, 1)]);
}

#[test]
fn an_open_abandoned_during_settle_closes_its_port() {
    // Code review: an open that the run abandons while it waits `settle` had opened its port and not given it; the port is
    // closed, so that a later open of it is not `serial.port_busy`. By hand: `settle` 1 s, abandoned at once.
    let mut driver = Driver::simulated(ARDUINO);
    let inputs = vec![
        Value::string("COM3"),
        Value::U32(9600),
        Value::U8(8),
        variant(0),
        variant(0),
        variant(0),
        Value::Duration(100 * MS),
        Value::Duration(S),
        Value::NONE,
    ];
    let id = driver.run.call("open", inputs).unwrap();
    assert!(matches!(driver.run.step(id, CallContext { now: 0 }), Step::Sleep(ns) if ns == S));
    driver.run.cancel(id);
    let port = driver.open("COM3").unwrap();
    assert_eq!(port.number, 2, "the second port of the run; the first is closed");
    assert!(!driver.run.describe(Handle { library: 0, number: 1 }).unwrap().open);
}

#[test]
fn many_opens_and_closes_of_one_port_stay_free() {
    // Review: a port closed frees its endpoint, however many were opened before; 1,000 opens and closes of `COM3` each
    // open, and the last port is the 1,000th.
    let mut driver = Driver::simulated(ARDUINO);
    let mut last = Handle::INVALID;
    for _ in 0..1_000 {
        last = driver.open("COM3").unwrap();
        driver.close(last).unwrap();
    }
    assert_eq!(last.number, 1_000);
    driver.run.release(last);
    assert!(driver.open("COM3").is_ok(), "release of a port closed already changes nothing");
}

#[test]
fn the_checks_of_an_open_follow_io_md_3() {
    // io.md 3, decision of the owner: the name, then `io.unsupported`, then the arguments, then a port open in this run. By
    // hand: on a host without serial ports (WASM), a baud rate of 0 on a busy port is `io.unsupported`; elsewhere,
    // `io.invalid_argument`; with the right settings, `serial.port_busy`.
    use crate::run::first_checks;
    use crate::settings::{Flow, Parity, Settings, StopBits};
    let endpoint = of_call("COM3").unwrap();
    let wrong: Result<Settings, String> = Err("a baud rate of 0".to_owned());
    let right = Ok(Settings { baud: 9600, data_bits: 8, parity: Parity::None, stop_bits: StopBits::One, flow: Flow::None });
    let category = |result: Result<Settings, NativeOutcome>| match result {
        Err(NativeOutcome::Fail { category, code: 0, .. }) => category,
        other => panic!("{other:?}"),
    };
    assert_eq!(category(first_checks(&endpoint, &wrong, true, true)), "io.unsupported");
    assert_eq!(category(first_checks(&endpoint, &wrong, false, true)), "io.invalid_argument");
    assert_eq!(category(first_checks(&endpoint, &right, false, true)), "serial.port_busy");
    assert_eq!(first_checks(&endpoint, &right, false, false).ok(), right.ok());
}

/// A device that expects 115200 baud and nothing else: the file of the issue #372.
const FAST: &str = "[[serial.device]]\nendpoint = \"serial:COM3\"\nbaud = 115200\n\
                    [[serial.device.reply]]\nrequest = \"TEMP?\\n\"\nresponse = \"23.5\\r\\n\"\n";

impl Driver {
    fn configure_with(&mut self, port: Handle, settings: Opened) -> Result<(), Failed> {
        let inputs = vec![
            Value::Handle(port),
            Value::U32(settings.baud),
            Value::U8(settings.data_bits),
            variant(settings.parity),
            variant(settings.stop_bits),
            variant(settings.flow),
            Value::NONE,
        ];
        done(self.call("configure", inputs).0).map(|outputs| assert!(matches!(outputs.first(), Some(Value::Handle(handle)) if *handle == port)))
    }

    fn configure(&mut self, port: Handle, baud: u32) -> Result<(), Failed> {
        self.configure_with(port, Opened { baud, ..DEFAULTS })
    }

    /// `COM3` opened at the 115200 that `FAST` expects, with a timeout of 100 ms.
    fn open_fast(&mut self) -> Handle {
        self.open_with("COM3", Opened { baud: 115_200, timeout: 100 * MS, ..DEFAULTS }).unwrap()
    }

    /// Writes `TEMP?` and reads the line that comes back.
    fn ask(&mut self, port: Handle) -> Result<String, Failed> {
        self.write_line(port, "TEMP?")?;
        self.read_line(port)
    }
}

#[test]
fn configure_changes_what_a_device_judges_at_each_write() {
    // Issue #372 and io.md 6, worked out by hand. The device expects 115200 and names no other setting (any). `open` at the
    // default 9600 differs in `baud` only: one item of `settings`, at 0 ms, `{expected 115200, opened 9600}`. The write is
    // taken (one entry in the report) and not answered: `read_line` waits the 100 ms of the port and fails `io.timeout`, code 0.
    let mut driver = Driver::simulated(FAST);
    let port = driver.open("COM3").unwrap();
    let pair = SettingDiff { name: "baud", expected: Setting::Integer(115_200), opened: Setting::Integer(9600) };
    assert_eq!(category(driver.ask(port)), ("io.timeout".to_owned(), 0));
    assert_eq!(driver.now, 100 * MS);
    let logs = driver.run.instruments();
    assert_eq!(logs[0].commands.len(), 1, "the write only");
    assert_eq!((logs[0].settings.len(), logs[0].settings[0].at), (1, 0));
    assert_eq!(logs[0].settings[0].differ, std::slice::from_ref(&pair));
    // `configure` to 115200 gives no error, takes no time, and adds nothing to `settings`; the next write is answered, with no
    // latency in this file, so the clock stays at 100 ms.
    driver.configure(port, 115_200).unwrap();
    assert_eq!(driver.now, 100 * MS, "configure takes no time on the clock");
    assert_eq!(driver.ask(port), Ok("23.5".to_owned()));
    assert_eq!(driver.now, 100 * MS);
    assert_eq!(driver.run.instruments()[0].settings.len(), 1, "a configure to what it expects adds nothing");
    // `configure` back to 9600 adds a second item, at 100 ms, with the same pair; the next write is not answered, and its read
    // waits 100 ms more.
    driver.configure(port, 9600).unwrap();
    let logs = driver.run.instruments();
    assert_eq!(logs[0].settings.len(), 2);
    assert_eq!(logs[0].settings[1].at, 100 * MS);
    assert_eq!(logs[0].settings[1].differ, [pair]);
    assert_eq!(category(driver.ask(port)), ("io.timeout".to_owned(), 0));
    assert_eq!(driver.now, 200 * MS);
}

#[test]
fn configure_leaves_the_bytes_that_wait() {
    // Issue #372: the port at 115200 gets its answer after `write_line`; a `configure` to the same settings does not drop it,
    // and `read_line` still gives `23.5`.
    let mut driver = Driver::simulated(FAST);
    let port = driver.open_fast();
    driver.write_line(port, "TEMP?").unwrap();
    driver.configure(port, 115_200).unwrap();
    assert_eq!(driver.read_line(port), Ok("23.5".to_owned()));
    assert!(driver.run.instruments()[0].settings.is_empty(), "opened and configured with what it expects");
}

#[test]
fn configure_judges_its_ranges_before_changing_anything() {
    // Issue #372: the ranges of `open`. `baud` 0, `data_bits` 4 and 9 fail `io.invalid_argument`, code 0, and the port keeps
    // the settings it had: at 115200, the next write is answered. A call that is wrong in `data_bits` and would change `baud`
    // to 9600 changes neither (the device would turn deaf if `baud` had been taken). 5, 6, 7 and 8 are accepted; the device
    // names no `data_bits`, so it answers at each.
    let mut driver = Driver::simulated(FAST);
    let port = driver.open_fast();
    let wrong = [Opened { baud: 0, ..DEFAULTS }, Opened { data_bits: 4, ..DEFAULTS }, Opened { data_bits: 9, ..DEFAULTS }];
    for settings in wrong {
        let (category, code, message) = driver.configure_with(port, settings).unwrap_err();
        assert_eq!((category.as_str(), code), ("io.invalid_argument", 0), "{message}");
        assert!(message.starts_with("serial:COM3: "), "{message}");
        assert_eq!(driver.ask(port), Ok("23.5".to_owned()), "judged at the settings it had: 115200");
    }
    assert_eq!(driver.now, 0);
    for data_bits in 5..=8 {
        driver.configure_with(port, Opened { baud: 115_200, data_bits, ..DEFAULTS }).unwrap();
        assert_eq!(driver.ask(port), Ok("23.5".to_owned()), "{data_bits} data bits");
    }
    assert!(driver.run.instruments()[0].settings.is_empty());
}

#[test]
fn configure_on_a_port_closed_or_never_opened_is_io_closed() {
    // Issue #372, as every other call of `serial`: after `close`, `io.closed`, code 0, "the port is closed"; on the default
    // value of `serial.port`, `io.closed`, code 0, "never opened". `close` of either still gives no error. A range that is
    // wrong on a port closed gives `io.closed` too: the port is judged first.
    let mut driver = Driver::simulated(FAST);
    let port = driver.open("COM3").unwrap();
    driver.close(port).unwrap();
    let (kind, code, message) = driver.configure(port, 115_200).unwrap_err();
    assert_eq!((kind.as_str(), code, message.as_str()), ("io.closed", 0, "serial:COM3: the port is closed"));
    assert_eq!(category(driver.configure(port, 0)).0, "io.closed");
    let (kind, code, message) = driver.configure(Handle::INVALID, 115_200).unwrap_err();
    assert_eq!((kind.as_str(), code), ("io.closed", 0));
    assert!(message.contains("never opened") && message.contains("serial.port"), "{message}");
    assert_eq!(driver.close(port), Ok(()));
    assert_eq!(driver.close(Handle::INVALID), Ok(()));
}

#[test]
fn configure_to_several_settings_that_differ_is_one_item_in_the_order_of_open() {
    // Issue #372: a device that expects 8 data bits and no parity, opened with them (no item), then configured to 7 data bits
    // and even parity (index 2 of `serial.Parity`: None, Odd, Even): one item, at the clock of the call, with `data_bits` first
    // and `parity` second, each `{expected, opened}`; `baud`, `stop_bits` and `flow` are not named by the file, so not listed.
    let file = "[[serial.device]]\nendpoint = \"serial:COM3\"\ndata_bits = 8\nparity = \"none\"\n";
    let mut driver = Driver::simulated(file);
    driver.now = 7 * MS;
    let port = driver.open("COM3").unwrap();
    assert!(driver.run.instruments().is_empty(), "nothing was written or said");
    driver.now = 9 * MS;
    driver.configure_with(port, Opened { data_bits: 7, parity: 2, ..DEFAULTS }).unwrap();
    let logs = driver.run.instruments();
    assert_eq!(logs[0].settings.len(), 1);
    assert_eq!(logs[0].settings[0].at, 9 * MS);
    assert_eq!(
        logs[0].settings[0].differ,
        [
            SettingDiff { name: "data_bits", expected: Setting::Integer(8), opened: Setting::Integer(7) },
            SettingDiff { name: "parity", expected: Setting::Text("none"), opened: Setting::Text("even") },
        ]
    );
}
