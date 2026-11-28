//! `visa` against `libs/visa/simulation.md`. The expected values are worked out by hand, or with a Python script written from
//! section 3 of that document alone (SplitMix64, the twelve draws, the ramp), `tools/visa-expected/expected.py`, which gives
//! the first factor of seed 1 that the document states, `1.4243480905156507`; each test says which.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::HashMap;

use labcraft_runtime::native::{
    CallContext, CallId, Context, Key, LOG_BUDGET, LibraryDiagnostic, LibraryRun, LogBudget, Mode, NativeLibrary, NativeOutcome, Start, Step,
};
use labcraft_runtime::simulation::read;
use labcraft_runtime::{Handle, Value, hex};

use crate::Visa;
use crate::instrument::SplitMix64;

const MS: i64 = 1_000_000;

fn shipped(name: &str) -> String {
    std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("sim").join(name)).unwrap()
}

/// The diagnostics of a file, or the run it gives.
fn judged(text: &str) -> Result<Box<dyn LibraryRun>, Vec<LibraryDiagnostic>> {
    let file = read(text, 64).map_err(|diagnostic| vec![diagnostic])?;
    let simulations = file.judge(&[&Visa])?;
    Ok(Visa.start_run(Start {
        library: 0,
        simulation: simulations.first().and_then(Option::as_ref),
        budget: LogBudget::new(LOG_BUDGET),
        mode: Mode::Deterministic,
        context: &Context { simulation_file: true, ..Context::default() },
    }))
}

fn codes(text: &str) -> Vec<(&'static str, u32, u32)> {
    match judged(text) {
        Ok(_) => Vec::new(),
        Err(found) => found.iter().map(|d| (d.code, d.span.map_or(0, |s| s.start.line), d.span.map_or(0, |s| s.start.column))).collect(),
    }
}

/// A call waiting in the driver.
struct Task {
    id: CallId,
    wake: i64,
    blocked: Option<Key>,
    held: Vec<Key>,
    done: Option<(NativeOutcome, i64)>,
}

/// What the scheduler of step 5 does for these calls: the clock, and the keys given in the order they were asked.
struct Driver {
    run: Box<dyn LibraryRun>,
    now: i64,
    tasks: Vec<Task>,
    holders: HashMap<Key, usize>,
}

impl Driver {
    fn new(text: &str) -> Self {
        Driver { run: judged(text).unwrap(), now: 0, tasks: Vec::new(), holders: HashMap::new() }
    }

    fn with_budget(text: &str, bytes: u64) -> Self {
        let file = read(text, 64).unwrap();
        let simulations = file.judge(&[&Visa]).unwrap();
        let run = Visa.start_run(Start {
            library: 0,
            simulation: simulations.first().and_then(Option::as_ref),
            budget: LogBudget::new(bytes),
            mode: Mode::Deterministic,
            context: &Context { simulation_file: true, ..Context::default() },
        });
        Driver { run, now: 0, tasks: Vec::new(), holders: HashMap::new() }
    }

    fn without_simulation() -> Self {
        Driver {
            run: Visa.start_run(Start {
                library: 0,
                simulation: None,
                budget: LogBudget::new(LOG_BUDGET),
                mode: Mode::Deterministic,
                context: &Context::default(),
            }),
            now: 0,
            tasks: Vec::new(),
            holders: HashMap::new(),
        }
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

    /// Runs every task to its end.
    fn go(&mut self) {
        loop {
            let ready = (0..self.tasks.len())
                .filter(|index| self.tasks[*index].done.is_none() && self.tasks[*index].blocked.is_none())
                .min_by_key(|index| (self.tasks[*index].wake, *index));
            let Some(index) = ready else { break };
            self.now = self.now.max(self.tasks[index].wake);
            match self.run.step(self.tasks[index].id, CallContext { now: self.now }) {
                Step::Sleep(ns) => self.tasks[index].wake = self.now + ns,
                Step::Exclusive(key) => match self.holders.entry(key) {
                    std::collections::hash_map::Entry::Occupied(_) => self.tasks[index].blocked = Some(key),
                    std::collections::hash_map::Entry::Vacant(free) => {
                        free.insert(index);
                        self.tasks[index].held.push(key);
                    }
                },
                Step::Release(key) => {
                    self.tasks[index].held.retain(|held| *held != key);
                    self.give(key);
                }
                Step::Blocking(_) => panic!("a simulated call never blocks"),
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

    /// Runs the tasks whose next step comes at `limit` or before, and stops there.
    fn go_until(&mut self, limit: i64) {
        loop {
            let ready = (0..self.tasks.len())
                .filter(|index| self.tasks[*index].done.is_none() && self.tasks[*index].blocked.is_none() && self.tasks[*index].wake <= limit)
                .min_by_key(|index| (self.tasks[*index].wake, *index));
            let Some(index) = ready else { break };
            let before = self.tasks.len();
            self.one(index);
            debug_assert_eq!(before, self.tasks.len());
        }
    }

    /// One step of one task.
    fn one(&mut self, index: usize) {
        self.now = self.now.max(self.tasks[index].wake);
        match self.run.step(self.tasks[index].id, CallContext { now: self.now }) {
            Step::Sleep(ns) => self.tasks[index].wake = self.now + ns,
            Step::Exclusive(key) => match self.holders.entry(key) {
                std::collections::hash_map::Entry::Occupied(_) => self.tasks[index].blocked = Some(key),
                std::collections::hash_map::Entry::Vacant(free) => {
                    free.insert(index);
                    self.tasks[index].held.push(key);
                }
            },
            Step::Release(key) => {
                self.tasks[index].held.retain(|held| *held != key);
                self.give(key);
            }
            Step::Blocking(_) => panic!("a simulated call never blocks"),
            Step::Done(outcome) => {
                self.tasks[index].done = Some((outcome, self.now));
                for key in std::mem::take(&mut self.tasks[index].held) {
                    self.give(key);
                }
            }
        }
    }

    /// One call, alone, to its end: its outcome and when it ended.
    fn call(&mut self, function: &str, inputs: Vec<Value>) -> (NativeOutcome, i64) {
        let index = self.start(function, inputs);
        self.go();
        let task = &mut self.tasks[index];
        task.done.take().unwrap()
    }

    fn open(&mut self, resource: &str, timeout: i64) -> Handle {
        match self.call("open", vec![Value::string(resource), Value::Duration(timeout), Value::NONE]).0 {
            NativeOutcome::Outputs(outputs) => match outputs.first() {
                Some(Value::Handle(handle)) => *handle,
                other => panic!("{other:?}"),
            },
            other => panic!("open {resource}: {other:?}"),
        }
    }

    fn query(&mut self, session: Handle, command: &str) -> Result<String, (String, i64)> {
        text_of(self.call("query", vec![Value::Handle(session), Value::string(command), Value::NONE]).0)
    }

    fn write(&mut self, session: Handle, command: &str) {
        let outcome = self.call("write", vec![Value::Handle(session), Value::string(command), Value::NONE]).0;
        assert!(matches!(outcome, NativeOutcome::Outputs(_)), "{outcome:?}");
    }
}

fn text_of(outcome: NativeOutcome) -> Result<String, (String, i64)> {
    match outcome {
        NativeOutcome::Outputs(outputs) => match outputs.first() {
            Some(Value::String(text)) => Ok(text.to_string()),
            other => panic!("{other:?}"),
        },
        NativeOutcome::Fail { category, code, .. } => Err((category.to_owned(), code)),
    }
}

#[test]
fn the_noise_is_splitmix64_and_twelve_draws() {
    // Python, from simulation.md 3 alone: seed 1 gives 1.4243480905156507 (the document says so), then -0.6189042598785675.
    let mut generator = SplitMix64::new(1);
    assert_eq!(generator.factor(), 1.424_348_090_515_650_7);
    assert_eq!(generator.factor(), -0.618_904_259_878_567_5);
    assert_eq!(SplitMix64::new(2).factor(), 0.580_284_711_620_648_5);
}

#[test]
fn every_shipped_file_is_accepted() {
    let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("sim");
    let mut count = 0;
    for entry in std::fs::read_dir(folder).unwrap() {
        let path = entry.unwrap().path();
        // The folder holds the sub-library `visa.sim` too: its manifest and its declarations are no simulation files.
        if path.extension().is_none_or(|extension| extension != "toml") || path.ends_with("labcraft.toml") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(judged(&text).is_ok(), "{}: {:?}", path.display(), judged(&text).err());
        count += 1;
    }
    assert_eq!(count, 15, "the files of simulation.md 8");
}

#[test]
fn a_multimeter_answers_its_measure_and_its_identity() {
    let mut driver = Driver::new(&shipped("dmm-230V.toml"));
    let dmm = driver.open("SIM::DMM0", 2_000 * MS);
    // Python (section 3): 230 + 0.05 × 1.4243480905156507, then 230 + 0.05 × -0.6189042598785675, to 6 decimals.
    assert_eq!(driver.query(dmm, "MEAS:VOLT:DC?"), Ok("230.071217".to_owned()));
    assert_eq!(driver.query(dmm, "MEAS:VOLT:DC?\n"), Ok("229.969055".to_owned()), "a termination is not part of the command");
    assert_eq!(driver.query(dmm, "*IDN?"), Ok("SIM,DMM,0,1.0".to_owned()));
    assert_eq!(driver.query(dmm, "SYST:ERR?"), Ok("0,\"No error\"".to_owned()));
    assert_eq!(driver.now, 0, "no latency, no time");
}

#[test]
fn a_ramp_and_a_source_follow_the_clock() {
    // Python: 0 + 0.05 g1 at 0; 230 × (150 / 300) + 0.05 g2 at 150 ms; 230 + 0.05 g3 at 300 ms.
    let mut driver = Driver::new(&shipped("dmm-settle-300ms.toml"));
    let dmm = driver.open("SIM::DMM0", 2_000 * MS);
    assert_eq!(driver.query(dmm, "MEAS:VOLT:DC?"), Ok("0.071217".to_owned()));
    driver.now = 150 * MS;
    assert_eq!(driver.query(dmm, "MEAS:VOLT:DC?"), Ok("114.969055".to_owned()));
    driver.now = 300 * MS;
    assert_eq!(driver.query(dmm, "MEAS:VOLT:DC?"), Ok("229.970462".to_owned()));

    // bench.toml: `VOLT 5` reaches the supply at 5 ms, its latency; the multimeter, wired to it, reads at 10 ms after its own.
    // Python: 0 + 5 × ((10 − 5) / 300) ms = 0.08333333333333333, plus 0.05 × the first factor of seed 7: 0.033286.
    let mut driver = Driver::new(&shipped("bench.toml"));
    let (dmm, psu) = (driver.open("SIM::DMM0", 2_000 * MS), driver.open("SIM::PSU0", 2_000 * MS));
    driver.write(psu, "VOLT 5");
    assert_eq!(driver.now, 5 * MS);
    assert_eq!(driver.query(dmm, "MEAS:VOLT:DC?"), Ok("0.033286".to_owned()));
    assert_eq!(driver.now, 10 * MS);
}

#[test]
fn a_query_never_answered_waits_for_the_timeout_of_its_session() {
    let mut driver = Driver::new(&shipped("dmm-timeout.toml"));
    let dmm = driver.open("SIM::DMM0", 2_000 * MS);
    assert_eq!(driver.query(dmm, "MEAS:VOLT:DC?"), Err(("io.timeout".to_owned(), -1_073_807_339)));
    assert_eq!(driver.now, 2_000 * MS);
    let NativeOutcome::Fail { message, .. } = driver.call("read", vec![Value::Handle(dmm), Value::NONE]).0 else { panic!() };
    assert_eq!(message, "SIM::DMM0: no answer to read within 2 s");
    assert_eq!(driver.now, 4_000 * MS);
}

#[test]
fn the_error_queue_answers_in_order_and_overflows() {
    let mut driver = Driver::new(&shipped("dmm-scpi-error.toml"));
    let dmm = driver.open("SIM::DMM0", 2_000 * MS);
    assert_eq!(driver.query(dmm, "SYST:ERR?"), Ok("-221,\"Settings conflict\"".to_owned()));
    assert_eq!(driver.query(dmm, "SYST:ERR?"), Ok("-113,\"Undefined header\"".to_owned()));
    assert_eq!(driver.query(dmm, "SYST:ERR?"), Ok("0,\"No error\"".to_owned()));
    // 33 undefined headers: 32 fit, the 33rd replaces the last by -350.
    for _ in 0..33 {
        driver.write(dmm, "BOGUS");
    }
    for _ in 0..31 {
        assert_eq!(driver.query(dmm, "SYST:ERR?"), Ok("-113,\"Undefined header\"".to_owned()));
    }
    assert_eq!(driver.query(dmm, "SYST:ERR?"), Ok("-350,\"Queue overflow\"".to_owned()));
    driver.write(dmm, "BOGUS");
    driver.write(dmm, "*CLS");
    assert_eq!(driver.query(dmm, "SYST:ERR?"), Ok("0,\"No error\"".to_owned()));
}

#[test]
fn writes_set_accept_or_add_their_error() {
    let text = r#"
[[visa.instrument]]
resource = "SIM::PSU0"
accept = ["ROUT:CLOS"]
[visa.instrument.set]
"VOLT" = "voltage"
[visa.instrument.measure]
"MEAS:VOLT?" = "voltage"
[visa.instrument.commands]
"*OPC?" = "1"
[visa.instrument.faults]
error_on = { "MEAS:VOLT?" = [-221, "Settings "quoted""] }
[visa.instrument.quantity.voltage]
value = 1.0
"#;
    // TOML needs the inner quotes escaped.
    let text = text.replace(r#""Settings "quoted"""#, r#""Settings \"quoted\"""#);
    let mut driver = Driver::new(&text);
    let psu = driver.open("SIM::PSU0", 1_000 * MS);
    driver.write(psu, "VOLT  -0.5");
    assert_eq!(driver.query(psu, "MEAS:VOLT?"), Ok("-0.500000".to_owned()), "the spaces after the header, then the number");
    assert_eq!(driver.query(psu, "SYST:ERR?"), Ok("-221,\"Settings \"\"quoted\"\"\"".to_owned()), "a quote is doubled");
    for (write, error) in [
        ("VOLT abc", "-224,\"Illegal parameter value\""),
        ("VOLT 1e101", "-222,\"Data out of range\""),
        ("VOLT 0x5", "-224,\"Illegal parameter value\""),
    ] {
        driver.write(psu, write);
        assert_eq!(driver.query(psu, "SYST:ERR?"), Ok(error.to_owned()), "{write}");
    }
    driver.write(psu, "VOLT .5");
    driver.write(psu, "ROUT:CLOS (@1)");
    assert_eq!(driver.query(psu, "*OPC?"), Ok("1".to_owned()));
    assert_eq!(driver.query(psu, "SYST:ERR?"), Ok("0,\"No error\"".to_owned()), "both accepted");
    // An answer not read is discarded by the next command, which adds -410.
    // By hand: the second `*OPC?` discards the answer of the first (-410); the `SYST:ERR?` discards that of the second (-410
    // again), then answers the oldest; the next answers the other; the read finds no answer and waits for its timeout.
    driver.write(psu, "*OPC?");
    driver.write(psu, "*OPC?");
    assert_eq!(driver.query(psu, "SYST:ERR?"), Ok("-410,\"Query INTERRUPTED\"".to_owned()));
    assert_eq!(driver.query(psu, "SYST:ERR?"), Ok("-410,\"Query INTERRUPTED\"".to_owned()));
    assert_eq!(driver.query(psu, "SYST:ERR?"), Ok("0,\"No error\"".to_owned()));
    let before = driver.now;
    let read = driver.call("read", vec![Value::Handle(psu), Value::NONE]).0;
    assert!(matches!(read, NativeOutcome::Fail { category: "io.timeout", .. }), "{read:?}");
    assert_eq!(driver.now - before, 1_000 * MS);
}

#[test]
fn an_instrument_handles_one_command_at_a_time_and_is_free_while_a_session_waits() {
    let text = r#"
[[visa.instrument]]
resource = "SIM::DMM0"
latency_ms = 5
[visa.instrument.commands]
"*OPC?" = "1"
[visa.instrument.faults]
timeout = ["HANG?"]
"#;
    let mut driver = Driver::new(text);
    let (a, b) = (driver.open("SIM::DMM0", 1_000 * MS), driver.open("SIM::DMM0", 1_000 * MS));
    // By hand: both start at 0; the second waits for the first command, 5 ms, then takes its own 5 ms.
    let first = driver.start("query", vec![Value::Handle(a), Value::string("*OPC?"), Value::NONE]);
    let second = driver.start("query", vec![Value::Handle(b), Value::string("*OPC?"), Value::NONE]);
    driver.go();
    assert_eq!(driver.tasks[first].done.as_ref().unwrap().1, 5 * MS);
    assert_eq!(driver.tasks[second].done.as_ref().unwrap().1, 10 * MS);
    // A query that hangs holds its session for its timeout, and the instrument only for its latency: at 10 ms both start; the
    // hung one has the instrument from 10 to 15 ms, the other from 15 to 20 ms, while the first waits until 1,015 ms.
    let hung = driver.start("query", vec![Value::Handle(a), Value::string("HANG?"), Value::NONE]);
    let other = driver.start("query", vec![Value::Handle(b), Value::string("*OPC?"), Value::NONE]);
    driver.go();
    assert_eq!(driver.tasks[other].done.as_ref().unwrap().1, 20 * MS);
    assert_eq!(driver.tasks[hung].done.as_ref().unwrap().1, 1_015 * MS);
}

#[test]
fn a_session_closed_is_closed_and_one_never_opened_is_invalid() {
    // simulation.md 6 (0.5.0) and README, "Errors and their codes": `io.closed` with `VI_ERROR_CONN_LOST` once closed,
    // `visa.invalid_session` with `VI_ERROR_INV_OBJECT` never opened; `close` of either succeeds.
    let mut driver = Driver::new(&shipped("dmm-230V.toml"));
    let dmm = driver.open("SIM::DMM0", 2_000 * MS);
    assert!(matches!(driver.call("close", vec![Value::Handle(dmm), Value::NONE]).0, NativeOutcome::Outputs(_)));
    assert_eq!(driver.query(dmm, "*IDN?"), Err(("io.closed".to_owned(), -1_073_807_194)));
    assert_eq!(driver.query(Handle::INVALID, "*IDN?"), Err(("visa.invalid_session".to_owned(), -1_073_807_346)));
    for handle in [dmm, Handle::INVALID] {
        let closed_again = driver.call("close", vec![Value::Handle(handle), Value::NONE]).0;
        assert!(matches!(closed_again, NativeOutcome::Outputs(_)), "{closed_again:?}");
    }
    let info = driver.run.describe(dmm).unwrap();
    assert_eq!((info.kind, info.designates.as_str(), info.open), ("session", "SIM::DMM0", false));
    assert!(driver.run.describe(Handle::INVALID).is_none());
}

#[test]
fn release_closes_what_a_run_leaves_open() {
    let mut driver = Driver::new(&shipped("dmm-230V.toml"));
    let dmm = driver.open("SIM::DMM0", 2_000 * MS);
    assert!(driver.run.describe(dmm).unwrap().open);
    driver.run.release(dmm);
    assert!(!driver.run.describe(dmm).unwrap().open);
    assert_eq!(driver.query(dmm, "*IDN?"), Err(("io.closed".to_owned(), -1_073_807_194)));
}

#[test]
fn a_resource_not_found_waits_for_the_timeout_of_the_open() {
    let mut driver = Driver::new(&shipped("no-instrument.toml"));
    let outcome = driver.call("open", vec![Value::string("SIM::DMM0"), Value::Duration(2_000 * MS), Value::NONE]).0;
    assert!(matches!(outcome, NativeOutcome::Fail { category: "visa.resource_not_found", code: -1_073_807_343, .. }));
    assert_eq!(driver.now, 2_000 * MS);
    let mut alone = Driver::without_simulation();
    let NativeOutcome::Fail { message, .. } = alone.call("open", vec![Value::string("SIM::DMM0"), Value::Duration(MS), Value::NONE]).0 else {
        panic!()
    };
    assert!(message.contains("no instrument is simulated"), "{message}");
    // `no-instrument.toml` is a simulation file without `[visa]`: the message holds for it too (scenario 1, point 3).
    let NativeOutcome::Fail { message, .. } = driver.call("open", vec![Value::string("SIM::DMM0"), Value::Duration(MS), Value::NONE]).0 else {
        panic!()
    };
    assert!(message.contains("or its file no `[visa]`"), "{message}");
}

#[test]
fn the_report_gets_the_commands_of_each_instrument_in_order_of_resources() {
    let mut driver = Driver::new(&shipped("two-dmm.toml"));
    let (one, zero) = (driver.open("SIM::DMM1", MS), driver.open("SIM::DMM0", MS));
    driver.query(one, "MEAS:VOLT:DC?").unwrap();
    driver.query(zero, "*IDN?").unwrap();
    driver.write(zero, "BOGUS");
    let logs = driver.run.instruments();
    assert_eq!(logs.iter().map(|log| log.resource.as_str()).collect::<Vec<_>>(), ["SIM::DMM0", "SIM::DMM1"]);
    let zero_log: Vec<(&str, Option<&str>)> = logs[0]
        .commands
        .iter()
        .map(|command| (command.sent.as_ref().map_or("", |sent| sent.kept.as_str()), command.answer.as_ref().map(|answer| answer.kept.as_str())))
        .collect();
    assert_eq!(zero_log, [("*IDN?", Some("SIM,DMM,0,1.0")), ("BOGUS", None)]);
}

#[test]
fn each_problem_of_a_file_has_its_code_at_its_key() {
    let instrument = |body: &str| format!("[[visa.instrument]]\nresource = \"SIM::DMM0\"\n{body}\n");
    // Line and column by hand: line 3 is the first line of `body`.
    assert_eq!(codes(&instrument("noize = 1")), [("visa:S001", 3, 1)]);
    assert_eq!(codes(&instrument("latency_ms = \"5\"")), [("visa:S002", 3, 14)]);
    assert_eq!(codes(&instrument("latency_ms = -1")), [("visa:S005", 3, 14)]);
    assert_eq!(codes(&instrument("latency_ms = 86400001")), [("visa:S005", 3, 14)]);
    assert_eq!(codes(&instrument("latency_ms = nan")), [("visa:S005", 3, 14)]);
    assert_eq!(codes("[[visa.instrument]]\nidn = \"x\"\n"), [("visa:S003", 1, 1)], "no resource");
    assert_eq!(codes("[[visa.instrument]]\nresource = \"GPIB::1\"\n"), [("visa:S003", 2, 12)]);
    assert_eq!(codes(&format!("{}{}", instrument(""), instrument(""))), [("visa:S004", 5, 12)], "the body is an empty third line");
    assert_eq!(codes(&instrument("[visa.instrument.quantity.v]\nnoise = 0.1")), [("visa:S003", 3, 1)], "a noise needs a seed");
    assert_eq!(codes(&instrument("[visa.instrument.quantity.v]\nseed = 1.5")), [("visa:S002", 4, 8)]);
    assert_eq!(codes(&instrument("[visa.instrument.quantity.v]\nseed = -1")), [("visa:S005", 4, 8)]);
    assert_eq!(codes(&instrument("[visa.instrument.quantity.v]\nvalue = 1e101")), [("visa:S005", 4, 9)]);
    assert_eq!(codes(&instrument("[visa.instrument.measure]\n\"MEAS?\" = \"nothing\"")), [("visa:S006", 4, 11)]);
    assert_eq!(codes(&instrument("[visa.instrument.quantity.v]\nsource = \"SIM::DMM9.v\"")), [("visa:S007", 4, 10)]);
    assert_eq!(codes(&instrument("[visa.instrument.quantity.v]\nsource = \"SIM::DMM0.v\"")), [("visa:S007", 4, 10)], "a loop of one");
    assert_eq!(
        codes(&instrument("[visa.instrument.quantity.v]\nsource = \"SIM::DMM0.w\"\nvalue = 1\n[visa.instrument.quantity.w]")),
        [("visa:S007", 5, 9)]
    );
    assert_eq!(codes(&instrument("idn = \"a\\u0007b\"")), [("visa:S008", 3, 7)]);
    assert_eq!(codes(&instrument("[visa.instrument.quantity.\"2v\"]")), [("visa:S008", 3, 27)]);
    assert_eq!(codes(&instrument("[visa.instrument.faults]\nerror_queue = [[1]]")), [("visa:S009", 4, 16)]);
    assert_eq!(codes(&instrument("[visa.instrument.faults]\nerror_queue = [[3000000000, \"x\"]]")), [("visa:S005", 4, 17)]);
    let many: String = (0..33).map(|_| "[0, \"x\"]").collect::<Vec<_>>().join(", ");
    assert_eq!(codes(&instrument(&format!("[visa.instrument.faults]\nerror_queue = [{many}]"))), [("visa:S010", 4, 15)]);
    assert_eq!(codes(&instrument("[visa.instrument.commands]\n\"OPC\" = \"1\"")), [("visa:S011", 4, 1)], "not a query");
    assert_eq!(codes(&instrument("[visa.instrument.commands]\n\"*IDN?\" = \"1\"")), [("visa:S011", 4, 1)], "the simulator answers it");
    assert_eq!(codes(&instrument("[visa.instrument.commands]\n\" X?\" = \"1\"")), [("visa:S011", 4, 1)], "a space at an end");
    assert_eq!(
        codes(&instrument("[visa.instrument.commands]\n\"M?\" = \"1\"\n[visa.instrument.measure]\n\"M?\" = \"v\"\n[visa.instrument.quantity.v]")),
        [("visa:S011", 6, 1)]
    );
    assert_eq!(codes("[visa]\ninstrument = 5\n"), [("visa:S002", 2, 14)]);
    assert_eq!(codes("[visa]\ninstruments = []\n"), [("visa:S001", 2, 1)]);
}

#[test]
fn every_problem_is_said_not_the_first_only() {
    let text = "[[visa.instrument]]\nresource = \"SIM::A\"\nnoize = 1\nlatency_ms = -5\n[[visa.instrument]]\nresource = \"SIM::B\"\nidn = 3\n";
    let found: Vec<&str> = codes(text).iter().map(|(code, _, _)| *code).collect();
    assert_eq!(found, ["visa:S001", "visa:S005", "visa:S002"]);
}

#[test]
fn the_file_as_a_whole_has_codes_of_the_tool() {
    assert_eq!(codes("visa = ["), [("LC8109", 1, 9)]);
    assert_eq!(codes("[visa]\n[visa]\n"), [("LC8109", 2, 2)], "a table given twice");
    assert_eq!(codes("x = 99999999999999999999\n"), [("LC8109", 1, 5)]);
    assert_eq!(codes("[viza]\n"), [("LC8110", 1, 2)]);
    let deep = format!("x = {}{}", "[".repeat(70), "]".repeat(70));
    assert_eq!(codes(&deep), [("LC0007", 0, 0)]);
    let dotted = format!("{} = 1", vec!["a"; 70].join("."));
    assert_eq!(codes(&dotted), [("LC0007", 0, 0)], "a dotted key nests too");
    // Floats are not keys: many of them nest nothing.
    let floats = format!("[visa]\nx = [{}]", vec!["1.5"; 200].join(", "));
    assert_eq!(codes(&floats), [("visa:S001", 2, 1)]);
    let nearest = read("[viza]\n", 64).unwrap().judge(&[&Visa]).err().unwrap();
    assert_eq!(nearest[0].fix.as_deref(), Some("visa"));
}

#[test]
fn a_file_nests_at_most_80_levels_as_toml_reads_it() {
    // Decision of the owner (2026-11-09): 80 levels at most, or --max-nesting when lower; beyond, LC0007, never LC8109. By hand
    // from the rule of `nesting`: n arrays are n levels, a header of n segments n, a key of n segments n - 1; along a path they
    // add. A key of 81 segments is 80 levels, which `toml` refuses on its own (it reads 80 segments at most): LC0007 too.
    let arrays = |n: usize| format!("x = {}{}\n", "[".repeat(n), "]".repeat(n));
    let key = |n: usize| format!("{} = 1\n", vec!["k"; n].join("."));
    let header = |n: usize| format!("[{}]\n", vec!["k"; n].join("."));
    let top = |text: &str| match read(text, 256) {
        Ok(_) => "read",
        Err(diagnostic) => diagnostic.code,
    };
    assert_eq!(top(&arrays(80)), "read");
    assert_eq!(top(&arrays(81)), "LC0007");
    assert_eq!(top(&arrays(127)), "LC0007");
    assert_eq!(top(&key(80)), "read");
    assert_eq!(top(&key(81)), "LC0007");
    assert_eq!(top(&header(80)), "read");
    assert_eq!(top(&header(81)), "LC0007");
    assert_eq!(top(&format!("[[{}]]\n", vec!["k"; 81].join("."))), "LC0007");
    assert_eq!(top(&format!("{} = {}{}\n", vec!["k"; 40].join("."), "[".repeat(41), "]".repeat(41))), "read", "39 + 41 = 80");
    assert_eq!(top(&format!("{} = {}{}\n", vec!["k"; 40].join("."), "[".repeat(42), "]".repeat(42))), "LC0007", "39 + 42 = 81");
    assert_eq!(top(&format!("x = {}1{}\n", "{a = ".repeat(80), "}".repeat(80))), "read", "80 inline tables");
    let led = format!("{} = ", vec!["a"; 80].join("."));
    assert_eq!(top(&format!("x = {}1{}\n", format!("{{{led}").repeat(80), "}".repeat(80))), "LC0007", "6,400 levels along the path");
    assert_eq!(top(&format!("[{}]\n{} = 1\n", vec!["h"; 41].join("."), vec!["k"; 41].join("."))), "LC0007", "41 + 40 = 81");
    assert_eq!(top(&format!("\"a.b\".{} = 1\n", vec!["k"; 80].join("."))), "LC0007", "a quoted segment counts once");
    assert_eq!(top(&format!("x = {}{}\n", "{a = ".repeat(81), "}".repeat(81))), "LC0007");
    assert!(matches!(read(&arrays(40), 30), Err(diagnostic) if diagnostic.code == "LC0007"), "a lower --max-nesting wins");
}

#[test]
fn an_interrupted_answer_enters_the_queue_when_its_command_is_handled() {
    // Decision of the owner (2026-11-09). By hand, latency 5 ms: `a` leaves an answer waiting (0 to 5 ms). Then `b` asks
    // `SYST:ERR?` and `a` writes: `b` holds the instrument first (5 to 10 ms) and sees no error; `a` is handled at 15 ms, which
    // adds -410; a second `SYST:ERR?` of `b` (15 to 20 ms) answers it.
    let text =
        "[[visa.instrument]]\nresource = \"SIM::DMM0\"\nlatency_ms = 5\n[visa.instrument.commands]\n\"*OPC?\" = \"1\"\n[visa.instrument.accept]\n";
    let text = text.replace("[visa.instrument.accept]\n", "");
    let text = text.replace("latency_ms = 5\n", "latency_ms = 5\naccept = [\"NOP\"]\n");
    let mut driver = Driver::new(&text);
    let (a, b) = (driver.open("SIM::DMM0", 1_000 * MS), driver.open("SIM::DMM0", 1_000 * MS));
    driver.write(a, "*OPC?");
    assert_eq!(driver.now, 5 * MS);
    let first = driver.start("query", vec![Value::Handle(b), Value::string("SYST:ERR?"), Value::NONE]);
    let write = driver.start("write", vec![Value::Handle(a), Value::string("NOP"), Value::NONE]);
    driver.go();
    assert_eq!(driver.tasks[first].done.as_ref().unwrap().1, 10 * MS);
    assert_eq!(driver.tasks[write].done.as_ref().unwrap().1, 15 * MS);
    let NativeOutcome::Outputs(outputs) = &driver.tasks[first].done.as_ref().unwrap().0 else { panic!() };
    assert!(matches!(outputs.first(), Some(Value::String(text)) if &**text == "0,\"No error\""));
    assert_eq!(driver.query(b, "SYST:ERR?"), Ok("-410,\"Query INTERRUPTED\"".to_owned()));
}

#[test]
fn a_session_released_while_its_call_waits_sends_nothing() {
    let text = "[[visa.instrument]]\nresource = \"SIM::DMM0\"\nlatency_ms = 5\n[visa.instrument.commands]\n\"*OPC?\" = \"1\"\n";
    let mut driver = Driver::new(text);
    let dmm = driver.open("SIM::DMM0", 1_000 * MS);
    let call = driver.start("query", vec![Value::Handle(dmm), Value::string("*OPC?"), Value::NONE]);
    driver.go_until(1);
    driver.run.release(dmm);
    driver.go();
    assert!(matches!(driver.tasks[call].done.as_ref().unwrap().0, NativeOutcome::Fail { category: "io.closed", .. }));
    assert!(driver.run.instruments().is_empty(), "the command reached no instrument");
}

#[test]
fn a_cancelled_call_is_no_longer_in_progress() {
    let mut driver = Driver::new(&shipped("dmm-230V.toml"));
    let dmm = driver.open("SIM::DMM0", 1_000 * MS);
    let id = driver.run.call("query", vec![Value::Handle(dmm), Value::string("*IDN?"), Value::NONE]).unwrap();
    driver.run.cancel(id);
    assert!(matches!(driver.run.step(id, CallContext { now: 0 }), Step::Done(NativeOutcome::Fail { .. })));
    assert_eq!(driver.query(dmm, "*IDN?"), Ok("SIM,DMM,0,1.0".to_owned()), "the session is still free");
}

#[test]
fn a_handle_of_another_library_is_no_session() {
    let mut driver = Driver::new(&shipped("dmm-230V.toml"));
    let dmm = driver.open("SIM::DMM0", 1_000 * MS);
    let other = Handle { library: dmm.library + 1, number: dmm.number };
    assert!(driver.run.describe(other).is_none());
    assert_eq!(driver.query(other, "*IDN?"), Err(("visa.invalid_session".to_owned(), -1_073_807_346)));
}

#[test]
fn the_log_keeps_the_start_of_each_text_and_its_length() {
    let mut driver = Driver::new("[[visa.instrument]]\nresource = \"SIM::DMM0\"\n");
    let dmm = driver.open("SIM::DMM0", 1_000 * MS);
    let long = "é".repeat(2500);
    driver.write(dmm, &long);
    let logs = driver.run.instruments();
    let command = &logs[0].commands[0];
    let sent = command.sent.as_ref().unwrap();
    assert_eq!((sent.kept.chars().count(), sent.length), (1000, 2500), "commands.md 5: 1,000 code points");
    assert_eq!(command.answer, None);
}

#[test]
fn the_log_of_all_the_instruments_has_one_budget() {
    // By hand: each write of 1,000 four-byte characters costs 4,000 + 64 bytes; 16 MiB hold 4,128 of them (16,777,216 / 4,064,
    // rounded down), so of 6 instruments × 1,000 writes, 4,128 are kept and 1,872 counted.
    let smile = "\u{1F600}";
    let instruments: String = (0..6).map(|n| format!("[[visa.instrument]]\nresource = \"SIM::I{n}\"\naccept = [\"{smile}\"]\n")).collect();
    let mut driver = Driver::new(&instruments);
    let text = smile.repeat(1000);
    for n in 0..6 {
        let session = driver.open(&format!("SIM::I{n}"), MS);
        for _ in 0..1000 {
            driver.write(session, &text);
        }
    }
    let logs = driver.run.instruments();
    let kept: usize = logs.iter().map(|log| log.commands.len()).sum();
    let more: u64 = logs.iter().map(|log| log.more).sum();
    assert_eq!((kept, more), (4128, 1872));
}

#[test]
fn a_bench_says_what_each_instrument_answers_to() {
    // By hand, from `sim/bench.toml`, a file of two instruments out of code point order, and a file that has two entries in
    // every list but `measure`.
    let bench = |text: &str| {
        let file = read(text, 64).unwrap();
        let simulations = file.judge(&[&Visa]).unwrap();
        simulations[0].as_ref().unwrap().downcast_ref::<std::sync::Arc<crate::Bench>>().unwrap().commands()
    };
    let shipped = bench(&shipped("bench.toml"));
    let resources: Vec<&str> = shipped.iter().map(|commands| commands.resource.as_str()).collect();
    assert_eq!(resources, ["SIM::DMM0", "SIM::PSU0", "SIM::RELAY0"], "in the order of the file");
    let backwards = bench("[[visa.instrument]]\nresource = \"SIM::B\"\n[[visa.instrument]]\nresource = \"SIM::A\"\n");
    let resources: Vec<&str> = backwards.iter().map(|commands| commands.resource.as_str()).collect();
    assert_eq!(resources, ["SIM::B", "SIM::A"], "in the order of the file, not of the resources");
    assert_eq!((shipped[1].set.as_slice(), shipped[2].accept.as_slice()), (&["VOLT".to_owned()][..], &["ROUT:CLOS".to_owned()][..]));
    // Two entries in each list, written out of order: each comes back in code point order.
    let every = bench(
        "[[visa.instrument]]\nresource = \"SIM::X\"\nidn = \"SIM,X\"\nmeasure = { \"MEAS?\" = \"v\" }\n\
         commands = { \"B?\" = \"1\", \"A?\" = \"2\" }\nset = { \"VOLT\" = \"v\", \"CURR\" = \"w\" }\naccept = [\"ZAP\", \"ARM\"]\n\
         quantity.v = { value = 1.0 }\nquantity.w = { value = 2.0 }\n\
         faults = { timeout = [\"HANG?\", \"FREEZE?\"], error_on = { \"BAD\" = [-100, \"Command error\"], \"AWFUL\" = [-200, \"Execution error\"] } }\n",
    );
    let words = |list: &[&str]| list.iter().map(|word| (*word).to_owned()).collect::<Vec<_>>();
    assert_eq!(
        every,
        vec![crate::Commands {
            resource: "SIM::X".to_owned(),
            idn: "SIM,X".to_owned(),
            commands: vec![("A?".to_owned(), "2".to_owned()), ("B?".to_owned(), "1".to_owned())],
            measure: words(&["MEAS?"]),
            set: words(&["CURR", "VOLT"]),
            accept: words(&["ARM", "ZAP"]),
            timeout: words(&["FREEZE?", "HANG?"]),
            error_on: words(&["AWFUL", "BAD"]),
            hang: None,
        }]
    );
}

#[test]
fn the_log_of_an_instrument_is_the_start_of_what_it_received() {
    // By hand: a budget of 100 bytes; a write of 100 characters costs 100 + 64 = 164 bytes and is counted, and `*RST` after it,
    // 4 + 64 = 68 bytes, which would fit, is counted too.
    let mut driver = Driver::with_budget("[[visa.instrument]]\nresource = \"SIM::DMM0\"\n", 100);
    let dmm = driver.open("SIM::DMM0", MS);
    driver.write(dmm, &"X".repeat(100));
    driver.write(dmm, "*RST");
    let logs = driver.run.instruments();
    assert_eq!((logs[0].commands.len(), logs[0].more), (0, 2));
}

// `visa.sim` (0.5.0): README, "Simulation: `visa.sim`" and "Errors and their codes"; simulation.md 2, 4, 5, 6 and 7. The
// values are those of issue #371, worked out from these documents by hand.

const TMO: i64 = -1_073_807_339;
const RSRC_NFOUND: i64 = -1_073_807_343;
const INV_OBJECT: i64 = -1_073_807_346;
const CONN_LOST: i64 = -1_073_807_194;

fn unhex(text: &str) -> Vec<u8> {
    text.as_bytes().chunks(2).map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap()).collect()
}

fn bytes(data: &[u8]) -> Value {
    Value::Bytes(data.into())
}

fn failed(outcome: &NativeOutcome) -> Option<(&'static str, i64)> {
    match outcome {
        NativeOutcome::Fail { category, code, .. } => Some((category, *code)),
        NativeOutcome::Outputs(_) => None,
    }
}

impl Driver {
    fn sim_open(&mut self, resource: &str, timeout: i64) -> Result<Handle, (&'static str, i64)> {
        match self.call("sim_open", vec![Value::string(resource), Value::Duration(timeout), Value::NONE]).0 {
            NativeOutcome::Outputs(outputs) => match outputs.first() {
                Some(Value::Handle(handle)) => Ok(*handle),
                other => panic!("{other:?}"),
            },
            NativeOutcome::Fail { category, code, .. } => Err((category, code)),
        }
    }

    fn sim_write(&mut self, instrument: Handle, data: &[u8]) -> NativeOutcome {
        self.call("sim_write", vec![Value::Handle(instrument), bytes(data), Value::NONE]).0
    }

    /// The bytes a read took, in hexadecimal, or its error.
    fn sim_bytes(outcome: NativeOutcome) -> Result<String, (&'static str, i64)> {
        match outcome {
            NativeOutcome::Outputs(outputs) => match outputs.first() {
                Some(Value::Bytes(data)) => Ok(hex(data)),
                other => panic!("{other:?}"),
            },
            NativeOutcome::Fail { category, code, .. } => Err((category, code)),
        }
    }

    fn sim_read(&mut self, instrument: Handle, count: i32) -> Result<String, (&'static str, i64)> {
        Driver::sim_bytes(self.call("sim_read", vec![Value::Handle(instrument), Value::I32(count), Value::NONE]).0)
    }

    fn sim_read_until(&mut self, instrument: Handle, terminator: &[u8]) -> Result<String, (&'static str, i64)> {
        Driver::sim_bytes(self.call("sim_read_until", vec![Value::Handle(instrument), bytes(terminator), Value::I32(65536), Value::NONE]).0)
    }

    /// A write of the text, then a read up to `\n`, given back as text with its `\n`.
    fn sim_query(&mut self, instrument: Handle, command: &str) -> Result<String, (&'static str, i64)> {
        let wrote = self.sim_write(instrument, command.as_bytes());
        if let Some(failure) = failed(&wrote) {
            return Err(failure);
        }
        self.sim_read_until(instrument, b"\n").map(|data| String::from_utf8(unhex(&data)).unwrap())
    }
}

/// `dmm-230V.toml` with its instrument named `name`.
fn dmm_named(name: &str) -> String {
    shipped("dmm-230V.toml").replace("SIM::DMM0", name)
}

#[test]
fn an_instrument_with_a_real_name_is_found_by_any_name_of_its_endpoint() {
    // simulation.md 2 and README, "Where a name goes": a real name reduces to its endpoint (io.md 2), so these are one.
    let mut driver = Driver::new(&dmm_named("TCPIP0::10.0.0.5::5025::SOCKET"));
    let dmm = driver.sim_open("tcpip0::10.0.0.5::05025::socket", 2_000 * MS).unwrap();
    assert_eq!(driver.sim_query(dmm, "MEAS:VOLT:DC?"), Ok("230.071217\n".to_owned()), "as for SIM::DMM0");
    assert_eq!(driver.sim_open("TCPIP0::10.0.0.6::5025::SOCKET", 2_000 * MS), Err(("visa.resource_not_found", RSRC_NFOUND)));
    assert_eq!(driver.now, 2_000 * MS, "after the default timeout of the open");
    // The native `visa.open` finds it too: one backend.
    let session = driver.open("TCPIP::10.0.0.5::5025::SOCKET", MS);
    assert_eq!(driver.query(session, "*IDN?"), Ok("SIM,DMM,0,1.0".to_owned()));
    // One entry in the report, under the `resource` of the file, whichever spelling opened it.
    let logs = driver.run.instruments();
    assert_eq!(logs.iter().map(|log| log.resource.as_str()).collect::<Vec<_>>(), ["TCPIP0::10.0.0.5::5025::SOCKET"]);
    assert_eq!(logs[0].commands.len(), 2);
    // A `SIM::` name is compared exactly.
    let mut driver = Driver::new(&shipped("dmm-230V.toml"));
    assert_eq!(driver.sim_open("SIM::dmm0", MS), Err(("visa.resource_not_found", RSRC_NFOUND)));
    let mut driver = Driver::new(&dmm_named("ASRL3::INSTR"));
    assert!(driver.sim_open("asrl3::instr", MS).is_ok());
    assert!(driver.sim_open("ASRL03::INSTR", MS).is_err(), "no endpoint");
}

#[test]
fn a_handle_of_visa_sim_shows_as_its_type() {
    let mut driver = Driver::new(&shipped("dmm-230V.toml"));
    let dmm = driver.sim_open("SIM::DMM0", MS).unwrap();
    let info = driver.run.describe(dmm).unwrap();
    assert_eq!((info.kind, info.designates.as_str(), info.open), ("sim.instrument", "SIM::DMM0", true), "visa.sim.instrument");
}

#[test]
fn each_write_is_one_command() {
    // README, "Simulation: `visa.sim`": one final `\n` removed, the other stays in the command, which matches no key
    // (simulation.md 4): no answer, then `-113`.
    let mut driver = Driver::new(&shipped("dmm-230V.toml"));
    let dmm = driver.sim_open("SIM::DMM0", 100 * MS).unwrap();
    assert!(failed(&driver.sim_write(dmm, b"MEAS:VOLT:DC?\nMEAS:VOLT:DC?\n")).is_none());
    assert_eq!(driver.sim_read_until(dmm, b"\n"), Err(("io.timeout", TMO)));
    assert_eq!(driver.sim_query(dmm, "SYST:ERR?"), Ok("-113,\"Undefined header\"\n".to_owned()));
    let logs = driver.run.instruments();
    assert_eq!(logs[0].commands[0].sent.as_ref().unwrap().length, 27, "the command without its final `\\n`");
}

#[test]
fn the_simulator_answers_opc_esr_and_stb() {
    // simulation.md 4, step 3: -221 is an execution error (bit 16), -113 a command error (bit 32); `*ESR?` clears, `*STB?`
    // gives 4 while the queue holds an error.
    let file = "[[visa.instrument]]\nresource = \"SIM::DEV0\"\naccept = [\"ROUT:CLOS\"]\n\
                [visa.instrument.faults]\nerror_on = { \"ROUT:CLOS 1\" = [-221, \"Settings conflict\"] }\n";
    let mut driver = Driver::new(file);
    let dev = driver.open("SIM::DEV0", MS);
    assert_eq!(driver.query(dev, "*OPC?"), Ok("1".to_owned()));
    driver.write(dev, "ROUT:CLOS 1");
    assert_eq!(driver.query(dev, "*ESR?"), Ok("16".to_owned()));
    assert_eq!(driver.query(dev, "*ESR?"), Ok("0".to_owned()));
    driver.write(dev, "FOO");
    for expected in ["4", "4"] {
        assert_eq!(driver.query(dev, "*STB?"), Ok(expected.to_owned()), "it clears nothing");
    }
    assert_eq!(driver.query(dev, "*ESR?"), Ok("32".to_owned()));
    assert_eq!(driver.query(dev, "*ESR?"), Ok("0".to_owned()));
    assert_eq!(driver.query(dev, "*STB?"), Ok("4".to_owned()));
    assert_eq!(driver.query(dev, "SYST:ERR?"), Ok("-221,\"Settings conflict\"".to_owned()));
    assert_eq!(driver.query(dev, "*STB?"), Ok("4".to_owned()));
    assert_eq!(driver.query(dev, "SYST:ERR?"), Ok("-113,\"Undefined header\"".to_owned()));
    assert_eq!(driver.query(dev, "*STB?"), Ok("0".to_owned()));

    let mut driver = Driver::new(file);
    let dev = driver.open("SIM::DEV0", MS);
    driver.write(dev, "FOO");
    driver.write(dev, "*CLS");
    assert_eq!(driver.query(dev, "*ESR?"), Ok("0".to_owned()));
    assert_eq!(driver.query(dev, "SYST:ERR?"), Ok("0,\"No error\"".to_owned()));

    // `commands` wins, so that a file that gave `*OPC?` reads as before.
    let mut driver = Driver::new(&format!("{file}[visa.instrument.commands]\n\"*OPC?\" = \"0\"\n"));
    let dev = driver.open("SIM::DEV0", MS);
    assert_eq!(driver.query(dev, "*OPC?"), Ok("0".to_owned()));

    // The errors of `error_queue` set their bits when the run starts.
    let queued = "[[visa.instrument]]\nresource = \"SIM::DEV0\"\n[visa.instrument.faults]\nerror_queue = [[-221, \"Settings conflict\"]]\n";
    let mut driver = Driver::new(queued);
    let dev = driver.open("SIM::DEV0", MS);
    assert_eq!(driver.query(dev, "*ESR?"), Ok("16".to_owned()));
    let mut driver = Driver::new(queued);
    let dev = driver.open("SIM::DEV0", MS);
    assert_eq!(driver.query(dev, "*STB?"), Ok("4".to_owned()));
}

#[test]
fn a_hung_instrument_handles_no_command() {
    // simulation.md 4: from `hang_ms` on, a command takes its latency and nothing else; the read waits its timeout.
    let file = "[[visa.instrument]]\nresource = \"SIM::DMM0\"\nlatency_ms = 0\n[visa.instrument.faults]\nhang_ms = 100\n";
    let mut driver = Driver::new(file);
    let dmm = driver.sim_open("SIM::DMM0", 500 * MS).unwrap();
    driver.now = 50 * MS;
    assert_eq!(driver.sim_query(dmm, "*IDN?"), Ok("SIM,SIM::DMM0,0,1.0\n".to_owned()));
    let mut driver = Driver::new(file);
    let dmm = driver.sim_open("SIM::DMM0", 500 * MS).unwrap();
    driver.now = 150 * MS;
    assert_eq!(driver.sim_query(dmm, "*IDN?"), Err(("io.timeout", TMO)));
    assert_eq!(driver.now, 650 * MS);
    assert_eq!(codes("[[visa.instrument]]\nresource = \"SIM::DMM0\"\n[visa.instrument.faults]\nhang_ms = -1\n"), [("visa:S005", 4, 11)]);
}

#[test]
fn an_answer_is_read_as_bytes_and_its_rest_waits() {
    // simulation.md 4, "Bytes": the first read of `dmm-230V.toml` is `230.071217`, the 11 bytes `3233302e3037313231370a`.
    let mut driver = Driver::new(&shipped("dmm-230V.toml"));
    let dmm = driver.sim_open("SIM::DMM0", 2_000 * MS).unwrap();
    assert!(failed(&driver.sim_write(dmm, b"MEAS:VOLT:DC?")).is_none());
    assert_eq!(driver.sim_read(dmm, 12), Err(("io.timeout", TMO)), "more than is left");
    assert_eq!(driver.now, 2_000 * MS);
    assert_eq!(driver.sim_read(dmm, 3), Ok("323330".to_owned()), "and the read of 12 took nothing");
    assert_eq!(driver.sim_read_until(dmm, b"\n"), Ok("2e3037313231370a".to_owned()));
    assert_eq!(driver.sim_read(dmm, 1), Err(("io.timeout", TMO)), "the answer is gone");

    let mut driver = Driver::new(&shipped("dmm-230V.toml"));
    let dmm = driver.sim_open("SIM::DMM0", 2_000 * MS).unwrap();
    assert!(failed(&driver.sim_write(dmm, &unhex("4d4541533a564f4c543a44433f0d0a"))).is_none(), "`MEAS:VOLT:DC?` and `\\r\\n`");
    assert_eq!(driver.sim_read_until(dmm, b"\n"), Ok(hex(b"230.071217\n")));
    assert!(failed(&driver.sim_write(dmm, &[0xff])).is_none());
    assert_eq!(driver.sim_query(dmm, "SYST:ERR?"), Ok("-113,\"Undefined header\"\n".to_owned()), "bytes not UTF-8 match nothing");

    // `visa.read` of the native `visa` gives what is left, without its `\n`.
    let session = driver.open("SIM::DMM0", MS);
    driver.write(session, "*IDN?");
    let rest = driver.call("read", vec![Value::Handle(session), Value::NONE]).0;
    assert_eq!(text_of(rest), Ok("SIM,DMM,0,1.0".to_owned()));
}

#[test]
fn a_read_of_bytes_judges_its_arguments() {
    let mut driver = Driver::new(&shipped("dmm-230V.toml"));
    let dmm = driver.sim_open("SIM::DMM0", MS).unwrap();
    assert_eq!(driver.sim_read(dmm, -1), Err(("io.invalid_argument", 0)));
    assert_eq!(driver.sim_read_until(dmm, b""), Err(("io.invalid_argument", 0)));
    assert_eq!(driver.sim_read(dmm, 0), Ok(String::new()), "nothing to take is taken at once");
    assert_eq!(driver.now, 0);
    // No terminator within `max`: `io.too_long`, and the bytes are taken.
    assert!(failed(&driver.sim_write(dmm, b"*IDN?")).is_none());
    let outcome = driver.call("sim_read_until", vec![Value::Handle(dmm), bytes(b"\n"), Value::I32(4), Value::NONE]).0;
    assert_eq!(failed(&outcome), Some(("io.too_long", 0)));
    assert_eq!(driver.sim_read_until(dmm, b"\n"), Ok(hex(b"DMM,0,1.0\n")), "after the 4 bytes `SIM,`");
}

#[test]
fn set_serial_is_for_an_instrument_with_an_asrl_name() {
    let mut driver = Driver::new(&shipped("dmm-230V.toml"));
    let dmm = driver.sim_open("SIM::DMM0", MS).unwrap();
    let outcome = driver.call("sim_set_serial", vec![Value::Handle(dmm)]).0;
    assert_eq!(failed(&outcome), Some(("io.unsupported", 0)));
    let mut driver = Driver::new(&dmm_named("ASRL3::INSTR"));
    let dmm = driver.sim_open("ASRL3::INSTR", MS).unwrap();
    assert_eq!(failed(&driver.call("sim_set_serial", vec![Value::Handle(dmm)]).0), None);
    assert_eq!(driver.sim_query(dmm, "MEAS:VOLT:DC?"), Ok("230.071217\n".to_owned()), "it changed nothing");
    // Its settings are judged as `serial.open` judges them (decision of the owner, 2026-11-28): `baud` above 0, `data_bits` 5
    // to 8; 5 and 8 data bits are taken.
    let set = |baud: u32, bits: u8| vec![Value::Handle(dmm), Value::U32(baud), Value::U8(bits), Value::NONE];
    for (baud, bits, expected) in [
        (0, 8, Some(("io.invalid_argument", 0))),
        (9600, 4, Some(("io.invalid_argument", 0))),
        (9600, 9, Some(("io.invalid_argument", 0))),
        (115_200, 5, None),
        (1, 8, None),
    ] {
        assert_eq!(failed(&driver.call("sim_set_serial", set(baud, bits)).0), expected, "{baud} {bits}");
    }
}

#[test]
fn a_handle_of_visa_sim_that_is_not_open() {
    let mut driver = Driver::new(&shipped("dmm-230V.toml"));
    let dmm = driver.sim_open("SIM::DMM0", MS).unwrap();
    assert_eq!(failed(&driver.call("sim_close", vec![Value::Handle(dmm), Value::NONE]).0), None);
    assert_eq!(failed(&driver.sim_write(dmm, b"*IDN?")), Some(("io.closed", CONN_LOST)));
    assert_eq!(driver.sim_read(dmm, 1), Err(("io.closed", CONN_LOST)));
    assert_eq!(driver.sim_read_until(dmm, b"\n"), Err(("io.closed", CONN_LOST)));
    assert_eq!(failed(&driver.call("sim_close", vec![Value::Handle(dmm), Value::NONE]).0), None, "a second close");
    // The default handle is also what a failed `open` gives: `SIM::NOPE` is not found.
    assert_eq!(driver.sim_open("SIM::NOPE", MS), Err(("visa.resource_not_found", RSRC_NFOUND)));
    let never = Handle::INVALID;
    assert_eq!(failed(&driver.sim_write(never, b"*IDN?")), Some(("visa.invalid_session", INV_OBJECT)));
    assert_eq!(driver.sim_read(never, 1), Err(("visa.invalid_session", INV_OBJECT)));
    assert_eq!(driver.sim_read_until(never, b"\n"), Err(("visa.invalid_session", INV_OBJECT)));
    assert_eq!(failed(&driver.call("sim_close", vec![Value::Handle(never), Value::NONE]).0), None);
}

#[test]
fn the_file_takes_real_names_and_refuses_the_others() {
    // simulation.md 7, `visa:S003` and `visa:S004` (0.5.0), at the `resource` of the second instrument or of the one named.
    let one = |resource: &str| format!("[[visa.instrument]]\nresource = \"{resource}\"\n");
    for resource in ["hello", "ASRL0::INSTR", "GPIB0::5::INSTR"] {
        assert_eq!(codes(&one(resource)), [("visa:S003", 2, 12)], "{resource}");
    }
    for resource in ["TCPIP0::10.0.0.5::5025::SOCKET", "ASRL3::INSTR", "ASRL/dev/ttyUSB0::INSTR", "SIM::DMM0"] {
        assert_eq!(codes(&one(resource)), [], "{resource}");
    }
    for (first, second) in [
        ("TCPIP0::10.0.0.5::5025::SOCKET", "tcpip::10.0.0.5::5025::socket"),
        ("ASRL3::INSTR", "asrl3::instr"),
        ("TCPIP0::h::0502::SOCKET", "TCPIP0::h::502::SOCKET"),
    ] {
        assert_eq!(codes(&format!("{}{}", one(first), one(second))), [("visa:S004", 4, 12)], "{first} and {second}");
    }
    assert_eq!(codes(&format!("{}{}", one("SIM::dmm0"), one("SIM::DMM0"))), []);
    assert_eq!(codes(&format!("{}{}", one("ASRL/dev/ttyUSB0::INSTR"), one("ASRL/dev/ttyusb0::INSTR"))), [], "a path is exact");
    let commands = |key: &str| format!("{}[visa.instrument.commands]\n\"{key}\" = \"1\"\n", one("SIM::DMM0"));
    assert_eq!(codes(&commands("*OPC?")), []);
    assert_eq!(codes(&commands("*IDN?")), [("visa:S011", 4, 1)]);
}

#[test]
fn a_source_names_its_instrument_by_any_of_its_names() {
    // simulation.md 2: the quantity is what follows the last `.`, so `TCPIP0::10.0.0.5::5025::SOCKET.voltage` reads one way.
    // By hand: the multimeter takes the true value of the supply, 5, and has no noise.
    let file = |source: &str| {
        format!(
            "[[visa.instrument]]\nresource = \"TCPIP0::10.0.0.5::5025::SOCKET\"\n[visa.instrument.quantity.voltage]\nvalue = 5.0\n\
             [[visa.instrument]]\nresource = \"SIM::DMM0\"\nmeasure = {{ \"MEAS?\" = \"v\" }}\n\
             [visa.instrument.quantity.v]\nsource = \"{source}\"\n"
        )
    };
    for source in ["TCPIP0::10.0.0.5::5025::SOCKET.voltage", "tcpip::10.0.0.5::05025::socket.voltage"] {
        let mut driver = Driver::new(&file(source));
        let dmm = driver.open("SIM::DMM0", MS);
        assert_eq!(driver.query(dmm, "MEAS?"), Ok("5.000000".to_owned()), "{source}");
    }
    assert_eq!(codes(&file("TCPIP0::10.0.0.6::5025::SOCKET.voltage")), [("visa:S007", 9, 10)]);
}

#[test]
fn a_hung_instrument_drops_the_answer_that_waits_and_says_its_hang() {
    // simulation.md 4: a new command discards an answer that waits; once hung, the read ends with its timeout, whatever the
    // query, and no error enters the queue (no -410). By hand: `*IDN?` at 0 ms is left unread; `MEAS?` at 200 ms times out at
    // 300 ms.
    let file = "[[visa.instrument]]\nresource = \"SIM::DMM0\"\n[visa.instrument.faults]\nhang_ms = 100\n";
    let mut driver = Driver::new(file);
    let dmm = driver.open("SIM::DMM0", 100 * MS);
    driver.write(dmm, "*IDN?");
    driver.now = 200 * MS;
    assert_eq!(driver.query(dmm, "MEAS?"), Err(("io.timeout".to_owned(), TMO)));
    assert_eq!(driver.now, 300 * MS);
    let described = read(file, 64).unwrap().judge(&[&Visa]).unwrap();
    let bench = described[0].as_ref().unwrap().downcast_ref::<std::sync::Arc<crate::Bench>>().unwrap();
    assert_eq!(bench.commands()[0].hang, Some(100 * MS));
    let text = format!("{:?}", bench.describe());
    assert!(text.contains("hang_ms") && text.contains("100.0"), "{text}");
}

#[test]
fn a_measure_of_a_status_query_wins_over_the_simulator() {
    // simulation.md 2 and 4, step 3 (decision of the owner, 2026-11-28): `commands` or `measure` gives `*OPC?`, `*ESR?` and
    // `*STB?`, so that a file of M0 reads as before. By hand: the quantity is 7, with no noise.
    let file = "[[visa.instrument]]\nresource = \"SIM::DEV0\"\nmeasure = { \"*ESR?\" = \"v\", \"*STB?\" = \"v\", \"*OPC?\" = \"v\" }\n\
                [visa.instrument.quantity.v]\nvalue = 7\n";
    let mut driver = Driver::new(file);
    let dev = driver.open("SIM::DEV0", MS);
    for query in ["*ESR?", "*STB?", "*OPC?"] {
        assert_eq!(driver.query(dev, query), Ok("7.000000".to_owned()), "{query}");
    }
}
