//! Runs of small programs through `check`, `ir` and the scheduler. Every expected value is worked out by hand from the program
//! and the rules it cites (spec 8, 8.2; cli.md 6.2); the times are those of the waits written in it.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::any::Any;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use labcraft_check::{Checked, Content, Found, Libraries, LibrarySource, Origin, SourceFile, check_for_run};
use labcraft_ir::{ElementId, Node, Program, lower_for_run};
use labcraft_types::{Interner, Limits, Severity};

use super::{AbortBy, Control, Live, Native, Observer, PanelSetting, Pool, RealTime, Run, RunEnd, SetBy, Settings, Status, Wake, run};
use crate::fault::FaultCode;
use crate::native::{CallContext, CallId, HandleInfo, Key, LibraryCode, LibraryRun, LogBudget, NativeLibrary, NativeOutcome, Simulation, Step};
use crate::value::{Handle, Value};

const MS: i64 = 1_000_000;

/// A library shipped for these tests: `bench.open` gives a handle, `bench.slow` holds the key of its handle for one second,
/// `bench.block` holds it while work that blocks waits for the gate of the test, `bench.close` closes it and is `always`.
const BENCH: &[(&str, &str)] = &[
    ("port.lct", "labcraft 1\n\ntype port = opaque\n"),
    ("open.lcf", "labcraft 1\n\nextern rust(crate: \"test\", function: \"open\")\nfn open(err: error = none) -> (h: port, err: error)\n"),
    ("slow.lcf", "labcraft 1\n\nextern rust(crate: \"test\", function: \"slow\")\nfn slow(h: port, err: error = none) -> (h: port, err: error)\n"),
    ("block.lcf", "labcraft 1\n\nextern rust(crate: \"test\", function: \"block\")\nfn block(h: port, err: error = none) -> (h: port, err: error)\n"),
    ("close.lcf", "labcraft 1\n\nextern rust(crate: \"test\", function: \"close\")\nfn close(h: port, err: error = none) -> (err: error)\n"),
];

/// What the work of `bench.block` waits for: it never waits more than 30 s, so that a test cannot hang.
#[derive(Default)]
struct Gate {
    open: Mutex<bool>,
    changed: Condvar,
}

impl Gate {
    fn open(&self) {
        *self.open.lock().unwrap() = true;
        self.changed.notify_all();
    }

    fn pass(&self) {
        let open = self.open.lock().unwrap();
        let _held = self.changed.wait_timeout_while(open, Duration::from_secs(30), |open| !*open).unwrap();
    }
}

const MANIFEST: &str = "name = \"bench\"\nversion = \"0.1.0\"\ndescription = \"test\"\ndependencies = []\nalways = [\"close\"]\nresources = [\"port\"]\ncategories = []\n";

struct Shipped;

impl Libraries for Shipped {
    fn find(&self, name: &str, _: Origin) -> Found {
        if name != "bench" {
            return Found::NotFound;
        }
        let files = BENCH
            .iter()
            .map(|(file, text)| SourceFile { shown: format!("<labcraft>/bench/{file}"), content: Content::Bytes(Cow::Borrowed(text.as_bytes())) })
            .collect();
        Found::Library(LibrarySource {
            files,
            manifest: Some(MANIFEST.to_owned()),
            ..LibrarySource::new("bench".to_owned(), "<labcraft>/bench".to_owned(), Origin::Shipped)
        })
    }
}

/// What the test library saw: the calls cancelled and the handles released; what the work of `bench.block` waits for, whether
/// it panics, and what it gave back.
#[derive(Default)]
struct Seen {
    cancelled: Vec<u64>,
    released: Vec<u64>,
    gate: Option<Arc<Gate>>,
    panics: bool,
    resumed: Vec<u64>,
}

struct Bench(Arc<Mutex<Seen>>);

struct BenchRun {
    library: u32,
    open: Vec<bool>,
    calls: HashMap<CallId, (String, Handle, u8)>,
    next: u64,
    seen: Arc<Mutex<Seen>>,
}

impl NativeLibrary for Bench {
    fn name(&self) -> &str {
        "bench"
    }
    fn codes(&self) -> &'static [LibraryCode] {
        &[]
    }
    fn start_run(&self, library: u32, _: Option<&Simulation>, _: LogBudget) -> Box<dyn LibraryRun> {
        Box::new(BenchRun { library, open: Vec::new(), calls: HashMap::new(), next: 0, seen: Arc::clone(&self.0) })
    }
}

impl LibraryRun for BenchRun {
    fn call(&mut self, function: &str, inputs: Vec<Value>) -> Option<CallId> {
        let handle = match inputs.first() {
            Some(Value::Handle(handle)) => *handle,
            _ => Handle::INVALID,
        };
        self.next += 1;
        let id = CallId(self.next);
        self.calls.insert(id, (function.to_owned(), handle, 0));
        Some(id)
    }

    fn step(&mut self, call: CallId, _: CallContext) -> Step {
        let Some((function, handle, phase)) = self.calls.get_mut(&call) else { return Step::Done(NativeOutcome::Outputs(vec![])) };
        let key = Key { library: self.library, id: handle.number };
        match (function.as_str(), *phase) {
            ("open", _) => {
                self.open.push(true);
                let handle = Handle { library: self.library, number: self.open.len() as u64 };
                self.calls.remove(&call);
                Step::Done(NativeOutcome::Outputs(vec![Value::Handle(handle), Value::NONE]))
            }
            ("slow", 0) => {
                *phase = 1;
                Step::Exclusive(key)
            }
            ("slow", 1) => {
                *phase = 2;
                Step::Sleep(1000 * MS)
            }
            ("slow", _) => {
                let handle = *handle;
                self.calls.remove(&call);
                Step::Done(NativeOutcome::Outputs(vec![Value::Handle(handle), Value::NONE]))
            }
            ("block", 0) => {
                *phase = 1;
                Step::Exclusive(key)
            }
            ("block", 1) => {
                *phase = 2;
                let (gate, panics) = {
                    let seen = self.seen.lock().unwrap();
                    (seen.gate.clone(), seen.panics)
                };
                Step::Blocking(Box::new(move || {
                    assert!(!panics, "the work of `bench.block` panics, as the test asks");
                    if let Some(gate) = gate {
                        gate.pass();
                    }
                    Box::new(42_u64) as Box<dyn Any + Send>
                }))
            }
            ("block", _) => {
                let handle = *handle;
                self.calls.remove(&call);
                Step::Done(NativeOutcome::Outputs(vec![Value::Handle(handle), Value::NONE]))
            }
            _ => {
                let handle = *handle;
                self.calls.remove(&call);
                if let Some(open) = self.open.get_mut((handle.number as usize).wrapping_sub(1)) {
                    *open = false;
                }
                Step::Done(NativeOutcome::Outputs(vec![Value::NONE]))
            }
        }
    }

    fn resume(&mut self, _: CallId, done: Box<dyn Any + Send>) {
        if let Ok(value) = done.downcast::<u64>() {
            self.seen.lock().unwrap().resumed.push(*value);
        }
    }

    fn cancel(&mut self, call: CallId) {
        self.calls.remove(&call);
        self.seen.lock().unwrap().cancelled.push(call.0);
    }

    fn describe(&self, handle: Handle) -> Option<HandleInfo> {
        let open = *self.open.get((handle.number as usize).checked_sub(1)?)?;
        Some(HandleInfo { kind: "handle", designates: format!("H{}", handle.number), open })
    }

    fn release(&mut self, handle: Handle) {
        if let Some(open) = self.open.get_mut((handle.number as usize).wrapping_sub(1)) {
            *open = false;
        }
        self.seen.lock().unwrap().released.push(handle.number);
    }
}

/// The files of a project; the run starts at the first.
fn checked(files: &[(&str, &str)], names: &mut Interner) -> Checked {
    let mut root = LibrarySource::new(String::new(), String::new(), Origin::Project);
    for (shown, text) in files {
        root.files.push(SourceFile { shown: (*shown).to_owned(), content: Content::Bytes(Cow::Owned(text.as_bytes().to_vec())) });
    }
    let checked = check_for_run(root, &Shipped, names, &Limits::default());
    let mut errors: Vec<_> = checked.report.diagnostics().iter().filter(|diagnostic| diagnostic.severity() == Severity::Error).collect();
    for dependency in &checked.dependencies {
        errors.extend(dependency.report.diagnostics().iter().filter(|diagnostic| diagnostic.severity() == Severity::Error));
    }
    assert!(errors.is_empty(), "{errors:?}");
    checked
}

/// The clock of these runs: real time never passes.
struct Still;

impl Control for Still {
    fn real_ns(&mut self) -> u64 {
        0
    }
}

/// What the run said, as text: `ran name at ms`.
struct Heard<'a> {
    events: Vec<String>,
    names: &'a Interner,
}

impl Observer for Heard<'_> {
    fn ran(&mut self, at: i64, _: u64, node: &Node, iter: &[u64], outputs: super::Outputs<'_>, _: &dyn crate::json::Describe) {
        let what = if outputs.skipped { "skipped" } else { "ran" };
        self.events.push(format!("{what} {}{iter:?} at {}", self.names.resolve(node.name), at / MS));
    }
}

struct Ran {
    end: RunEnd,
    events: Vec<String>,
    program: Program,
    names: Interner,
}

impl Ran {
    fn value(&self, index: usize) -> String {
        let function = &self.program.functions[0];
        let mut out = crate::text::Text::new(1 << 16);
        let writer =
            crate::text::Writer { env: crate::convert::Env { program: &self.program, names: &self.names }, viewer: function.library, cut: false };
        writer.write(&mut out, &self.end.outputs[index], function.outputs[index].ty, None).unwrap();
        out.finish()
    }

    fn outputs(&self) -> Vec<String> {
        (0..self.end.outputs.len()).map(|index| self.value(index)).collect()
    }

    fn fault(&self) -> FaultCode {
        self.end.fault.as_ref().map(|fault| fault.fault.code).unwrap_or_else(|| panic!("no fault: {:?}", self.end.status))
    }
}

fn run_with(source: &str, inputs: Vec<Value>, settings: Settings, seen: &Arc<Mutex<Seen>>) -> Ran {
    run_files(&[("f.lcf", source)], inputs, settings, seen)
}

fn run_files(files: &[(&str, &str)], inputs: Vec<Value>, settings: Settings, seen: &Arc<Mutex<Seen>>) -> Ran {
    run_controlled(files, inputs, settings, seen, &mut Still)
}

fn run_controlled(files: &[(&str, &str)], inputs: Vec<Value>, settings: Settings, seen: &Arc<Mutex<Seen>>, control: &mut dyn Control) -> Ran {
    let mut names = Interner::new();
    let checked = checked(files, &mut names);
    let start = checked.files.iter().position(|file| file.shown == files[0].0).unwrap();
    let program = lower_for_run(&checked, start, &mut names).unwrap();
    let bench = Bench(Arc::clone(seen));
    let natives: Vec<Native<'_>> = checked
        .libraries
        .iter()
        .filter(|library| library.name == "bench")
        .map(|library| Native { library: library.id, native: &bench, simulation: None })
        .collect();
    let mut heard = Heard { events: Vec::new(), names: &names };
    let end = run(Run { program: &program, names: &names, files: &checked.files, natives: &natives, inputs, settings }, control, &mut heard);
    let events = heard.events;
    drop(natives);
    Ran { end, events, program, names }
}

fn ran(source: &str, inputs: Vec<Value>) -> Ran {
    run_with(source, inputs, Settings::default(), &Arc::default())
}

#[test]
fn a_function_gives_its_outputs() {
    let ran = ran(
        "labcraft 1\n\nuse core\n\nfn f(a: f64, b: f64) -> (s: f64, p: f64) {\n    s = core.add(a, b)\n    p = core.mul(s, b)\n}\n",
        vec![Value::F64(1.5), Value::F64(2.0)],
    );
    assert_eq!(ran.end.status, Status::Ok);
    // By hand: 1.5 + 2.0, then 3.5 × 2.0.
    assert_eq!(ran.outputs(), ["3.5", "7.0"]);
    assert_eq!(ran.end.nodes, 2);
    assert_eq!(ran.end.held, 0, "what the run counted is given back when it ends");
}

#[test]
fn the_waits_of_two_branches_overlap() {
    let source = "labcraft 1\n\nuse errors\nuse time\n\nfn f() -> (err: error) {\n    a = time.wait(100 ms)\n    b = time.wait(50 ms)\n    err = errors.merge(a, b)\n}\n";
    let ran = ran(source, vec![]);
    assert_eq!(ran.end.status, Status::Ok);
    // Spec 8: both waits start at 0; `b` ends at 50 ms, `a` at 100 ms, and the clock ends at 100 ms, not 150 ms.
    assert_eq!(ran.events, ["ran b[] at 50", "ran a[] at 100", "ran err[] at 100"]);
    assert_eq!(ran.end.clock, 100 * MS);
}

#[test]
fn ready_nodes_run_in_the_global_order_and_a_seed_changes_only_the_interleaving() {
    let source = "labcraft 1\n\nuse core\n\nfn f(x: f64) -> (a: f64, b: f64, c: f64) {\n    b = core.add(x, 1.0)\n    a = core.add(x, 2.0)\n    c = core.add(a, b)\n}\n";
    let ordered = ran(source, vec![Value::F64(1.0)]);
    // Spec 5.2: among the lines whose inputs are there, by name in code point order, whatever the order of the file: `a`,
    // then `b`, then `c`, which waits for both.
    assert_eq!(ordered.events, ["ran a[] at 0", "ran b[] at 0", "ran c[] at 0"]);
    let mut orders = std::collections::BTreeSet::new();
    for seed in 0..20 {
        let settings = Settings { schedule_seed: Some(seed), ..Settings::default() };
        let seeded = run_with(source, vec![Value::F64(1.0)], settings, &Arc::default());
        assert_eq!(seeded.outputs(), ["3.0", "2.0", "5.0"], "seed {seed}");
        assert_eq!(seeded.events.last().map(String::as_str), Some("ran c[] at 0"));
        orders.insert(seeded.events.clone());
    }
    assert_eq!(orders.len(), 2, "over 20 seeds, both orders of `a` and `b`");
}

const COUNT: &str = "labcraft 1\n\nuse core\n\nfn f(stop_at: f64) -> (n: f64, total: f64, xs: [f64]) {\n    n, total, xs = loop carry n = 0.0, carry s = 0.0 {\n        m = core.add(n, 1.0)\n        done = core.ge(m, stop_at)\n        next n = m\n        next s = core.add(s, m)\n        collect m\n        until done\n    }\n}\n";

#[test]
fn a_loop_carries_collects_and_ends_on_its_condition() {
    let ran = ran(COUNT, vec![Value::F64(5.0)]);
    assert_eq!(ran.end.status, Status::Ok);
    // By hand: m = 1, 2, 3, 4, 5; the sum 15; the loop ends when m reaches 5.
    assert_eq!(ran.outputs(), ["5.0", "15.0", "[1.0, 2.0, 3.0, 4.0, 5.0]"]);
    assert_eq!(ran.end.held, 0);
}

#[test]
fn a_loop_past_its_iterations_is_a_fault_at_the_loop() {
    let settings = Settings { max_iterations: Some(3), ..Settings::default() };
    let ran = run_with(COUNT, vec![Value::F64(5.0)], settings, &Arc::default());
    assert_eq!(ran.end.status, Status::Fault);
    assert_eq!(ran.fault(), FaultCode::Iterations);
    let fault = ran.end.fault.as_ref().unwrap();
    // The fourth iteration would pass the limit of 3; the message counts from 1 for a human.
    assert!(fault.fault.message.contains("at iteration 4"), "{}", fault.fault.message);
    assert_eq!(fault.iteration, Some(3), "from 0 in the report");
    assert_eq!(ran.names.resolve(fault.node.unwrap()), "n");
    assert!(ran.end.outputs.is_empty());
}

#[test]
fn the_limit_of_nodes_counts_every_node() {
    let settings = Settings { max_nodes: Some(1), ..Settings::default() };
    let source = "labcraft 1\n\nuse core\n\nfn f(a: f64) -> (s: f64) {\n    t = core.add(a, a)\n    s = core.add(t, a)\n}\n";
    let ran = run_with(source, vec![Value::F64(1.0)], settings, &Arc::default());
    assert_eq!(ran.fault(), FaultCode::Nodes);
    assert_eq!(ran.names.resolve(ran.end.fault.as_ref().unwrap().node.unwrap()), "s");
}

#[test]
fn a_call_too_deep_is_a_fault_and_never_the_stack() {
    let source = "labcraft 1\n\nuse core\n\nfn f(x: f64) -> (y: f64) {\n    y = f(x)\n}\n";
    let settings = Settings { max_call_depth: 10, ..Settings::default() };
    let ran = run_with(source, vec![Value::F64(1.0)], settings, &Arc::default());
    assert_eq!(ran.fault(), FaultCode::Depth);
    assert!(ran.end.fault.as_ref().unwrap().fault.message.contains("11 calls deep"));
    // The default limit, 1,024 frames on the heap.
    let ran = ran_deep(source);
    assert_eq!(ran.fault(), FaultCode::Depth);
}

fn ran_deep(source: &str) -> Ran {
    ran(source, vec![Value::F64(1.0)])
}

#[test]
fn a_timeout_aborts_and_waits_end_at_once() {
    let source = "labcraft 1\n\nuse time\n\nfn f() -> (err: error) {\n    err = time.wait(10 s)\n}\n";
    let settings = Settings { timeout: Some(1000 * MS), ..Settings::default() };
    let ran = run_with(source, vec![], settings, &Arc::default());
    assert_eq!(ran.end.status, Status::Aborted);
    let abort = ran.end.abort.as_ref().unwrap();
    assert_eq!((abort.by, abort.at), (AbortBy::Timeout, 1000 * MS));
    // Spec 8.2: a wait ends at once, and nothing runs after the abort.
    assert_eq!(ran.end.clock, 1000 * MS);
    assert!(ran.events.is_empty());
    assert!(ran.end.outputs.is_empty());
}

const STOPPED: &str = "labcraft 1\n\nuse core\nuse time\nuse ui\n\nfn f() -> (n: f64, err: error) {\n    n, err = loop carry n = 0.0, carry e = none {\n        e1 = time.wait(100 ms, err: e)\n        stop = ui.get(stop_switch, after: e1)\n        next n = core.add(n, 1.0)\n        next e = e1\n        until stop\n    }\n}\n\npanel {\n    control stop_switch: switch\n}\n";

#[test]
fn a_switch_set_during_the_run_stops_the_loop_through_the_diagram() {
    let settings = Settings {
        panel: vec![PanelSetting { element: ElementId(0), value: Value::Bool(true), at: 250 * MS, by: SetBy::Set }],
        ..Settings::default()
    };
    let ran = run_with(STOPPED, vec![], settings, &Arc::default());
    assert_eq!(ran.end.status, Status::Ok);
    // Iterations end at 100, 200 and 300 ms; the setting at 250 ms is seen by the read at 300 ms, so three iterations.
    assert_eq!(ran.outputs()[0], "3.0");
    assert_eq!(ran.end.clock, 300 * MS);
    assert!(ran.end.panel[0].applied);
}

#[test]
fn a_setting_after_the_end_never_applies() {
    let settings = Settings {
        panel: vec![
            PanelSetting { element: ElementId(0), value: Value::Bool(true), at: 0, by: SetBy::Set },
            PanelSetting { element: ElementId(0), value: Value::Bool(false), at: 5000 * MS, by: SetBy::Set },
        ],
        ..Settings::default()
    };
    let ran = run_with(STOPPED, vec![], settings, &Arc::default());
    // Set at 0 ms: the first read, at 100 ms, sees it.
    assert_eq!(ran.outputs()[0], "1.0");
    assert_eq!(ran.end.panel.iter().map(|applied| applied.applied).collect::<Vec<_>>(), [true, false]);
}

#[test]
fn a_node_after_an_error_skips_and_the_run_ends_in_error() {
    let source = "labcraft 1\n\nuse errors\nuse time\n\nfn f() -> (err: error) {\n    e1 = errors.make(code: 7, message: \"no\", category: io.timeout)\n    err = time.wait(1 s, err: e1)\n}\n";
    let ran = ran(source, vec![]);
    assert_eq!(ran.end.status, Status::Error);
    // Spec 7.2: the wait skips, passes on the error, and takes no time.
    assert_eq!(ran.events, ["ran e1[] at 0", "skipped err[] at 0"]);
    assert_eq!(ran.end.clock, 0);
    assert!(ran.outputs()[0].starts_with("io.timeout"), "{:?}", ran.outputs());
}

#[test]
fn the_memory_of_a_run_is_bounded() {
    let source = "labcraft 1\n\nuse array\n\nfn f(n: i32) -> (xs: [f64], err: error) {\n    xs, err = array.repeat(1.5, n)\n}\n";
    let settings = Settings { max_memory: 1 << 20, ..Settings::default() };
    // 100,000 items of 24 bytes each (16 for a value, 8 for an `f64`): 2.4 MB, past 1 MiB.
    let ran = run_with(source, vec![Value::I32(100_000)], settings.clone(), &Arc::default());
    assert_eq!(ran.fault(), FaultCode::Memory);
    let ran = run_with(source, vec![Value::I32(1000)], settings, &Arc::default());
    assert_eq!(ran.end.status, Status::Ok);
}

#[test]
fn the_bytes_made_count_copies_and_not_changes_in_place() {
    // An array of 1,000 `f64` is 16 + 1,000 x 24 = 24,016 bytes (cli.md 6.2). Read only by `array.set`, it changes in place
    // and makes nothing: the loop reaches its limit of 50 iterations. Read by `array.len` too, each iteration copies it:
    // the array and four copies make 120,080 bytes, past 100,000, in iteration 3 from 0.
    let in_place = "labcraft 1\n\nuse array\n\nfn f() -> (a: [f64], err: error) {\n    big, e = array.repeat(0.0, 1000)\n    a, err = loop carry xs = big, carry g = e {\n        b, h = array.set(xs, 0, 1.0, err: g)\n        next g = h\n        next xs = b\n        until false\n    }\n}\n";
    let copies = "labcraft 1\n\nuse array\n\nfn f() -> (a: [f64], err: error, k: i32) {\n    big, e = array.repeat(0.0, 1000)\n    a, err, k = loop carry xs = big, carry g = e, carry n = 0 {\n        b, h = array.set(xs, 0, 1.0, err: g)\n        l = array.len(xs)\n        next g = h\n        next n = l\n        next xs = b\n        until false\n    }\n}\n";
    let settings = Settings { max_made: Some(100_000), max_iterations: Some(50), ..Settings::default() };
    let ran = run_with(in_place, vec![], settings.clone(), &Arc::default());
    assert_eq!(ran.fault(), FaultCode::Iterations);
    let ran = run_with(copies, vec![], settings.clone(), &Arc::default());
    assert_eq!(ran.fault(), FaultCode::Made);
    let message = format!("{:?}", ran.end);
    assert!(message.contains("made 120080 bytes of values, past its limit of 100000"), "{message}");
    // No limit, as in real time by default: the loop reaches its limit of iterations.
    let ran = run_with(copies, vec![], Settings { max_made: None, ..settings.clone() }, &Arc::default());
    assert_eq!(ran.fault(), FaultCode::Iterations);
    // A part read out of a value is no copy: the value still holds it. The row and the array of two rows make 24,016 and
    // 16 + 2 x 24,016 = 48,048 bytes, 72,064 in all, under 100,000; each `array.get` makes nothing, so the loop reaches its
    // limit of 50 iterations.
    let parts = "labcraft 1\n\nuse array\n\nfn f() -> (k: i32) {\n    row, e = array.repeat(0.0, 1000)\n    rows, g = array.repeat(row, 2, err: e)\n    k = loop carry n = 0 {\n        x, h = array.get(rows, 0, err: g)\n        l = array.len(x)\n        next n = l\n        until false\n    }\n}\n";
    let ran = run_with(parts, vec![], settings.clone(), &Arc::default());
    assert_eq!(ran.fault(), FaultCode::Iterations);
    // A value built again around parts it shares counts its own level and the place of each part, not the parts: each
    // `array.concat` of two rows that `row` still holds makes 16 + 2 x 16 = 48 bytes, not 16 + 2 x 24,016. With the row,
    // 24,016 + 50 x 48 = 26,416 bytes, under 100,000: the loop reaches its limit of 50 iterations.
    let shared = "labcraft 1\n\nuse array\n\nfn f() -> (k: i32) {\n    row, e = array.repeat(0.0, 1000)\n    k = loop carry n = 0 {\n        m = array.concat([row], [row])\n        l = array.len(m)\n        next n = l\n        until false\n    }\n}\n";
    let ran = run_with(shared, vec![], settings, &Arc::default());
    assert_eq!(ran.fault(), FaultCode::Iterations);
}

#[test]
fn a_for_gives_its_carries_and_collects_in_the_order_of_its_iterations() {
    let source = "labcraft 1\n\nuse core\n\nfn f(xs: [f64]) -> (total: f64, doubled: [f64], indices: [i32]) {\n    total, doubled, indices = for x in xs, i in 0..10, carry acc = 0.0 {\n        d = core.mul(x, 2.0)\n        next acc = core.add(acc, x)\n        collect d\n        collect i\n    }\n}\n";
    let ran = ran(source, vec![Value::array(vec![Value::F64(1.0), Value::F64(2.5), Value::F64(4.0)])]);
    // Spec 6.2: the clauses advance together and stop at the shortest, three elements.
    assert_eq!(ran.outputs(), ["7.5", "[2.0, 5.0, 8.0]", "[0, 1, 2]"]);
}

#[test]
fn the_iterations_of_a_parallel_for_wait_side_by_side() {
    let source = "labcraft 1\n\nuse time\n\nfn f(waits: [duration]) -> (errs: [error]) {\n    errs = for parallel w in waits {\n        e = time.wait(w)\n        collect e\n    }\n}\n";
    let waits = vec![Value::Duration(300 * MS), Value::Duration(100 * MS), Value::Duration(200 * MS)];
    let ran = ran(source, vec![Value::array(waits)]);
    assert_eq!(ran.end.status, Status::Ok);
    // Each iteration at its own pace: the second ends first; the loop ends with the longest, at 300 ms, its collect in the
    // order of the iterations.
    assert_eq!(ran.events, ["ran e[1] at 100", "ran e[2] at 200", "ran e[0] at 300", "ran errs[] at 300"]);
    assert_eq!(ran.outputs(), ["[none, none, none]"]);
}

#[test]
fn a_match_runs_the_arm_its_selector_takes() {
    let source = "labcraft 1\n\nuse core\n\nfn f(n: i32) -> (name: string) {\n    name = match n {\n        0 => {\n            yield \"zero\"\n        }\n        1..=9 => {\n            yield \"digit\"\n        }\n        _ => {\n            yield \"more\"\n        }\n    }\n}\n";
    for (n, expected) in [(0, "zero"), (7, "digit"), (10, "more")] {
        assert_eq!(ran(source, vec![Value::I32(n)]).outputs(), [expected], "{n}");
    }
}

#[test]
fn a_call_of_the_default_of_a_function_type_fails_with_an_error() {
    let source = "labcraft 1\n\nuse array\nuse core\n\nfn f() -> (y: f64, err: error, e1: error) {\n    g, e1 = array.get([twice], 3)\n    y, err = core.call(g, 1.0, none)\n}\n";
    let twice =
        "labcraft 1\n\nuse core\n\nfn twice(x: f64, err: error = none) -> (y: f64, err: error) {\n    y = core.mul(x, 2.0)\n    err = err\n}\n";
    let ran = run_files(&[("f.lcf", source), ("twice.lcf", twice)], vec![], Settings::default(), &Arc::default());
    // `array.get` past the end gives the default of `fn(f64, error) -> (f64, error)` (Q-21): calling it runs nothing, its
    // `f64` takes its default, its `error` the new error.
    assert_eq!(ran.end.status, Status::Error);
    let outputs = ran.outputs();
    assert_eq!(outputs[0], "0.0");
    assert!(outputs[1].starts_with("core.invalid_function"), "{outputs:?}");
}

const SLOW: &str = "labcraft 1\n\nuse bench\n\nfn f() -> (err: error) {\n    h, e1 = bench.open()\n    h2, e2 = bench.slow(h, err: e1)\n    err = bench.close(h2, err: e2)\n}\n";

#[test]
fn a_native_call_takes_its_time_on_the_clock() {
    let seen = Arc::default();
    let ran = run_with(SLOW, vec![], Settings::default(), &seen);
    assert_eq!(ran.end.status, Status::Ok);
    assert_eq!(ran.end.clock, 1000 * MS);
    assert_eq!(ran.end.resources.len(), 1);
    // The program closed it; the runtime released nothing.
    assert!(!ran.end.resources[0].info.as_ref().unwrap().open);
    assert!(seen.lock().unwrap().released.is_empty());
}

#[test]
fn two_calls_on_one_key_take_turns() {
    let source = "labcraft 1\n\nuse bench\nuse errors\n\nfn f() -> (err: error) {\n    h, e1 = bench.open()\n    a, ea = bench.slow(h, err: e1)\n    b, eb = bench.slow(h, err: e1)\n    ca = bench.close(a, err: ea)\n    err = errors.merge(ca, eb)\n}\n";
    let ran = run_with(source, vec![], Settings::default(), &Arc::default());
    assert_eq!(ran.end.status, Status::Ok);
    // `a` comes first in the global order and holds the key for 1 s; `b` takes it then, and `close` of `a` runs at 1 s while
    // `b` holds it, since `close` takes no key here: 2 s in all.
    assert_eq!(ran.end.clock, 2000 * MS);
    assert_eq!(ran.events, ["ran h[] at 0", "ran a[] at 1000", "ran ca[] at 1000", "ran b[] at 2000", "ran err[] at 2000"]);
}

#[test]
fn an_abort_lets_a_call_end_in_the_grace_period_then_releases_what_is_open() {
    let seen = Arc::default();
    let settings = Settings { timeout: Some(500 * MS), ..Settings::default() };
    let ran = run_with(SLOW, vec![], settings, &seen);
    assert_eq!(ran.end.status, Status::Aborted);
    // Spec 8.2: the call in progress ends at 1 s, within the 5 s of grace; `close` never runs; the runtime releases `h`.
    assert_eq!(ran.end.abort.as_ref().unwrap().at, 500 * MS);
    assert_eq!(ran.end.clock, 1000 * MS);
    assert_eq!(seen.lock().unwrap().released, [1]);
    assert!(ran.end.resources[0].released && !ran.end.resources[0].leaked);
}

#[test]
fn a_call_past_the_grace_period_is_abandoned_and_its_resource_leaked() {
    let seen = Arc::default();
    let settings = Settings { timeout: Some(500 * MS), grace: 200 * MS, ..Settings::default() };
    let ran = run_with(SLOW, vec![], settings, &seen);
    assert_eq!(ran.end.status, Status::Aborted);
    // The grace period ends at 700 ms with the call still going: cancelled, its handle leaked and not released.
    assert_eq!(ran.end.clock, 700 * MS);
    let seen = seen.lock().unwrap();
    assert_eq!(seen.cancelled.len(), 1);
    assert!(seen.released.is_empty());
    assert!(ran.end.resources[0].leaked);
}

/// A second Ctrl-C, before the run is aborted: the grace period is over as soon as it starts.
struct GraceOver;

impl Control for GraceOver {
    fn real_ns(&mut self) -> u64 {
        0
    }
    fn grace_over(&mut self) -> bool {
        true
    }
}

#[test]
fn a_grace_period_ended_from_outside_abandons_the_call_at_once() {
    let seen = Arc::default();
    let settings = Settings { timeout: Some(500 * MS), ..Settings::default() };
    let ran = run_controlled(&[("f.lcf", SLOW)], vec![], settings, &seen, &mut GraceOver);
    assert_eq!(ran.end.status, Status::Aborted);
    // cli.md 6.5: the call that would end at 1 s is abandoned at the abort, 500 ms, not at the end of the 5 s of grace.
    assert_eq!(ran.end.clock, 500 * MS);
    let seen = seen.lock().unwrap();
    assert_eq!(seen.cancelled.len(), 1);
    assert!(seen.released.is_empty());
    assert!(ran.end.resources[0].leaked);
}

#[test]
fn a_fault_releases_what_the_run_opened() {
    let seen = Arc::default();
    let source = "labcraft 1\n\nuse bench\nuse core\n\nfn f() -> (err: error) {\n    h, e1 = bench.open()\n    x = core.to_i32(1e20)\n    err = bench.close(h, err: e1, after: x)\n}\n";
    let ran = run_with(source, vec![], Settings::default(), &seen);
    // `core.to_i32` of 1e20 is beyond the `i32`: `LC8008` (Q-25); `close` does not run, the runtime releases `h`.
    assert_eq!(ran.fault(), FaultCode::OutOfRange);
    assert_eq!(seen.lock().unwrap().released, [1]);
}

#[test]
fn positions_compare_element_by_element_and_a_start_comes_first() {
    use super::position::{child, compare, indices, root};
    use std::cmp::Ordering;
    let top = root();
    let call = child(&top, 3);
    let inside = child(&call, 0);
    let later = child(&top, 4);
    let iteration = child(&child(&top, 3), 1);
    // [3, 0] after [3], the start of it; [3, 0] before [4]; [3, 1] after [3, 0] though made apart from it.
    assert_eq!(indices(&inside), [3, 0]);
    assert_eq!(compare(&call, &inside), Ordering::Less);
    assert_eq!(compare(&inside, &later), Ordering::Less);
    assert_eq!(compare(&later, &inside), Ordering::Greater);
    assert_eq!(compare(&iteration, &later), Ordering::Less);
    assert_eq!(compare(&inside, &inside), Ordering::Equal);
}

#[test]
fn an_always_call_passes_on_the_error_it_received() {
    let source = "labcraft 1

use bench
use errors

fn f() -> (err: error) {
    h, e1 = bench.open()
    e2 = errors.make(code: 3, message: \"late\", category: io.timeout, after: e1)
    err = bench.close(h, err: e2)
}
";
    let ran = run_with(source, vec![], Settings::default(), &Arc::default());
    // Spec 7.2: `close` is `always` in the manifest of `bench`: it runs, closes the handle, and its `err` passes on the error.
    assert_eq!(ran.end.status, Status::Error);
    assert!(ran.outputs()[0].starts_with("io.timeout"), "{:?}", ran.outputs());
    assert!(!ran.end.resources[0].info.as_ref().unwrap().open);
}

#[test]
fn positions_made_apart_compare_by_their_outermost_difference() {
    use super::position::{child, compare, root};
    use std::cmp::Ordering;
    let top = root();
    let one = child(&child(&top, 3), 1);
    let zero = child(&child(&top, 3), 0);
    // [3, 1] after [3, 0], though their steps [3] were made apart; [2, 9] before [3, 0].
    assert_eq!(compare(&one, &zero), Ordering::Greater);
    assert_eq!(compare(&child(&child(&top, 2), 9), &zero), Ordering::Less);
}

#[test]
fn a_function_with_a_variadic_parameter_takes_its_arguments_as_one_array() {
    let total = "labcraft 1\n\nuse array\n\nfn total(xs: f64...) -> (sum: f64) {\n    sum = array.sum(xs)\n}\n";
    let main = "labcraft 1\n\nfn f() -> (s: f64) {\n    s = total(1.0, 2.0, 3.5)\n}\n";
    let ran = run_files(&[("f.lcf", main), ("total.lcf", total)], vec![], Settings::default(), &Arc::default());
    assert_eq!(ran.end.status, Status::Ok, "{:?}", ran.end.fault);
    // By hand: 1.0 + 2.0 + 3.5.
    assert_eq!(ran.outputs(), ["6.5"]);
}

#[test]
fn a_wait_of_nothing_keeps_its_place_in_the_global_order() {
    let source = "labcraft 1\n\nuse core\nuse time\n\nfn f(x: f64) -> (b: f64, c: f64) {\n    a0 = time.wait(0 s)\n    b = core.add(x, 1.0, after: a0)\n    c = core.add(x, 2.0)\n}\n";
    let ran = ran(source, vec![Value::F64(0.0)]);
    // Spec 5.2: `a0` < `b` < `c`, and `b` waits for `a0`. Spec 8: `a0` ends at 0 ms, and `b`, ready then, comes before `c`
    // in the global order, so it runs before `c`, not after every node ready at that time.
    assert_eq!(ran.events, ["ran a0[] at 0", "ran b[] at 0", "ran c[] at 0"].map(String::from));
}

#[test]
fn a_setting_and_a_timeout_at_one_time_apply_in_that_order() {
    let settings = Settings {
        timeout: Some(150 * MS),
        panel: vec![PanelSetting { element: ElementId(0), value: Value::Bool(true), at: 150 * MS, by: SetBy::Set }],
        ..Settings::default()
    };
    let ran = run_with(STOPPED, vec![], settings, &Arc::default());
    // cli.md 6.2: at 150 ms the setting applies first, then the timeout aborts, before any node of that time.
    assert_eq!(ran.end.status, Status::Aborted);
    assert!(ran.end.panel[0].applied);
    assert_eq!(ran.end.clock, 150 * MS);
}

#[test]
fn a_timeout_past_the_end_of_the_clock_is_a_fault() {
    let settings = Settings { timeout: Some(super::CLOCK_END + 1), ..Settings::default() };
    let ran = run_with(STOPPED, vec![], settings, &Arc::default());
    assert_eq!(ran.fault(), FaultCode::Clock);
}

#[test]
fn a_run_deep_in_calls_and_loops_ends_without_a_recursion_of_its_own() {
    let source = "labcraft 1\n\nfn f(x: f64) -> (y: f64) {\n    done = true\n    y = loop carry a = x {\n        b = loop carry c = a {\n            d = f(c)\n            next c = d\n            until done\n        }\n        next a = b\n        until done\n    }\n}\n";
    let settings = Settings { max_call_depth: 4096, ..Settings::default() };
    // 4,096 calls, each in two loops: positions 5 steps deeper at each call, 20,480 in all, freed without recursing on the
    // stack of a test thread.
    let ran = run_with(source, vec![Value::F64(1.0)], settings, &Arc::default());
    assert_eq!(ran.fault(), FaultCode::Depth);
}

// Real time ----------------------------------------------------------------------------------------------------------------

/// Real time on a clock of the test, so that what is asserted does not depend on the speed of the machine: a wait jumps to
/// the time it waits for, or to the next thing the test makes happen; only a wait for the pool with nothing else to come
/// waits for real, after opening the gate of the work.
#[derive(Default)]
struct Virtual {
    now: u64,
    abort_at: Option<u64>,
    grace_over_at: Option<u64>,
    /// `run.set_element`, by the time it is given, in order.
    live: Vec<(u64, Live)>,
    applied: Vec<(u64, Option<i64>)>,
    refused: Vec<(u64, FaultCode)>,
    gate: Option<Arc<Gate>>,
    /// How late each wait wakes, as the tick of a system makes it.
    late: u64,
}

impl Control for Virtual {
    fn real_ns(&mut self) -> u64 {
        self.now
    }

    fn abort(&mut self) -> Option<AbortBy> {
        let due = self.abort_at.is_some_and(|at| self.now >= at);
        due.then(|| {
            self.abort_at = None;
            AbortBy::Interrupt
        })
    }

    fn grace_over(&mut self) -> bool {
        self.grace_over_at.is_some_and(|at| self.now >= at)
    }

    fn wait(&mut self, wake: &Wake, seen: u64, until: Option<u64>) {
        if wake.seen() != seen || until.is_some_and(|until| until <= self.now) {
            return;
        }
        let next =
            [until, self.abort_at, self.grace_over_at, self.live.first().map(|(at, _)| *at)].into_iter().flatten().filter(|at| *at > self.now).min();
        match next {
            Some(at) => self.now = at + self.late,
            None => {
                if let Some(gate) = self.gate.take() {
                    gate.open();
                }
                wake.wait(seen, Some(Duration::from_secs(30)));
            }
        }
    }

    fn live(&mut self) -> Vec<Live> {
        let due = self.live.iter().take_while(|(at, _)| *at <= self.now).count();
        self.live.drain(..due).map(|(_, live)| live).collect()
    }

    fn applied(&mut self, tag: u64, at: Option<i64>) {
        self.applied.push((tag, at));
    }

    fn close_live(&mut self) -> Vec<Live> {
        self.live.drain(..).map(|(_, live)| live).collect()
    }

    fn refused(&mut self, tag: u64, fault: &crate::fault::Fault) {
        self.refused.push((tag, fault.code));
    }
}

/// The settings of a run in real time (cli.md 6.2): no limit of iterations, nodes or real time unless one is given.
fn real_time(epoch: i64) -> Settings {
    Settings {
        max_iterations: None,
        max_nodes: None,
        max_real_time: None,
        real_time: Some(RealTime { pool: Pool::default(), wake: Wake::default(), epoch }),
        ..Settings::default()
    }
}

fn run_real(source: &str, settings: Settings, seen: &Arc<Mutex<Seen>>, control: &mut Virtual) -> Ran {
    run_controlled(&[("f.lcf", source)], vec![], settings, seen, control)
}

#[test]
fn in_real_time_the_waits_of_two_branches_overlap_on_the_real_clock() {
    let source = "labcraft 1\n\nuse errors\nuse time\n\nfn f() -> (err: error) {\n    a = time.wait(100 ms)\n    b = time.wait(50 ms)\n    err = errors.merge(a, b)\n}\n";
    let ran = run_real(source, real_time(0), &Arc::default(), &mut Virtual::default());
    assert_eq!(ran.end.status, Status::Ok);
    // As in deterministic mode (spec 8), on the real clock: both waits start at 0, the clock ends at 100 ms, not 150 ms.
    assert_eq!(ran.events, ["ran b[] at 50", "ran a[] at 100", "ran err[] at 100"]);
    assert_eq!(ran.end.clock, 100 * MS);
}

#[test]
fn in_real_time_time_now_counts_from_the_start_of_the_run() {
    let source = "labcraft 1\n\nuse time\n\nfn f() -> (t: timestamp, err: error) {\n    err = time.wait(20 ms)\n    t = time.now(after: err)\n}\n";
    // 2026-11-11T00:00:00Z, in nanoseconds since 1970: 20,768 days of 86,400 s (Python's `datetime`, by subtraction).
    let epoch = 20_768 * 86_400 * 1_000 * MS;
    let ran = run_real(source, real_time(epoch), &Arc::default(), &mut Virtual::default());
    assert_eq!(ran.end.status, Status::Ok, "{:?}", ran.end.fault);
    assert!(matches!(ran.end.outputs[0], Value::Timestamp(ns) if ns == epoch + 20 * MS), "{:?}", ran.end.outputs[0]);
    assert_eq!(ran.outputs()[0], "2026-11-11T00:00:00.020000000Z");
}

const BLOCKED: &str = "labcraft 1\n\nuse bench\nuse errors\nuse time\n\nfn f() -> (err: error) {\n    h, e1 = bench.open()\n    b, eb = bench.block(h, err: e1)\n    w = time.wait(100 ms)\n    c = bench.close(b, err: eb)\n    err = errors.merge(c, w)\n}\n";

#[test]
fn work_that_blocks_runs_on_the_pool_while_the_other_branches_go_on() {
    let gate = Arc::new(Gate::default());
    let seen = Arc::new(Mutex::new(Seen { gate: Some(Arc::clone(&gate)), ..Seen::default() }));
    let mut control = Virtual { gate: Some(gate), ..Virtual::default() };
    let ran = run_real(BLOCKED, real_time(0), &seen, &mut control);
    assert_eq!(ran.end.status, Status::Ok, "{:?}", ran.end.fault);
    // `b` blocks from 0 ms; `w` ends at 100 ms meanwhile (D-12); then the gate opens, the work returns, `b` and what follows
    // end at 100 ms.
    assert_eq!(ran.events, ["ran h[] at 0", "ran w[] at 100", "ran b[] at 100", "ran c[] at 100", "ran err[] at 100"]);
    assert_eq!(seen.lock().unwrap().resumed, [42]);
    assert!(!ran.end.resources[0].info.as_ref().unwrap().open);
}

#[test]
fn in_deterministic_mode_work_that_blocks_runs_at_once_on_the_thread_of_the_run() {
    let seen = Arc::default();
    let ran = run_with(BLOCKED, vec![], Settings::default(), &seen);
    assert_eq!(ran.end.status, Status::Ok, "{:?}", ran.end.fault);
    // In the global order: `h`, then `b`, whose work takes no simulated time, then `w` starts, `c` runs at 0, `w` ends at 100.
    assert_eq!(ran.events, ["ran h[] at 0", "ran b[] at 0", "ran c[] at 0", "ran w[] at 100", "ran err[] at 100"]);
    assert_eq!(seen.lock().unwrap().resumed, [42]);
}

const BLOCKS: &str = "labcraft 1\n\nuse bench\n\nfn f() -> (err: error) {\n    h, e1 = bench.open()\n    b, eb = bench.block(h, err: e1)\n    err = bench.close(b, err: eb)\n}\n";

#[test]
fn an_abort_abandons_work_still_on_the_pool_when_the_grace_period_ends() {
    let gate = Arc::new(Gate::default());
    let seen = Arc::new(Mutex::new(Seen { gate: Some(Arc::clone(&gate)), ..Seen::default() }));
    let mut control = Virtual { abort_at: Some(10_000_000), ..Virtual::default() };
    let ran = run_real(BLOCKS, real_time(0), &seen, &mut control);
    gate.open();
    assert_eq!(ran.end.status, Status::Aborted);
    // Spec 8.2: Ctrl-C at 10 ms; the work never returns, so the call is abandoned when the 5 s of grace end, at 5,010 ms, and
    // its handle leaked, not released.
    assert_eq!(ran.end.abort.as_ref().unwrap().at, 10 * MS);
    assert_eq!(ran.end.clock, 5010 * MS);
    let seen = seen.lock().unwrap();
    assert_eq!((seen.cancelled.len(), seen.released.len()), (1, 0));
    assert!(ran.end.resources[0].leaked);
}

#[test]
fn a_second_interrupt_abandons_work_on_the_pool_at_once() {
    let gate = Arc::new(Gate::default());
    let seen = Arc::new(Mutex::new(Seen { gate: Some(Arc::clone(&gate)), ..Seen::default() }));
    let mut control = Virtual { abort_at: Some(10_000_000), grace_over_at: Some(1_010_000_000), ..Virtual::default() };
    let ran = run_real(BLOCKS, real_time(0), &seen, &mut control);
    gate.open();
    assert_eq!(ran.end.status, Status::Aborted);
    // cli.md 6.5: the second Ctrl-C, at 1,010 ms, ends the grace period there, not at 5,010 ms.
    assert_eq!(ran.end.clock, 1010 * MS);
    assert!(ran.end.resources[0].leaked);
}

#[test]
fn work_that_panics_is_a_fault_at_its_node_in_both_modes() {
    for real in [true, false] {
        let seen = Arc::new(Mutex::new(Seen { panics: true, ..Seen::default() }));
        let settings = if real { real_time(0) } else { Settings::default() };
        let ran = run_controlled(&[("f.lcf", BLOCKS)], vec![], settings, &seen, &mut Virtual::default());
        // 10.4: a panic on a thread of the pool is caught there, and ends the run with `LC8004`; the runtime releases `h`.
        assert_eq!(ran.fault(), FaultCode::Bug, "real time: {real}");
        assert_eq!(ran.end.fault.as_ref().unwrap().node.map(|node| ran.names.resolve(node).to_owned()).as_deref(), Some("b"));
        assert_eq!(seen.lock().unwrap().released, [1], "real time: {real}");
    }
}

#[test]
fn a_setting_of_run_set_element_is_seen_by_the_reads_that_start_after_it() {
    let live = Live { tag: 7, element: ElementId(0), value: Value::Bool(true) };
    let mut control = Virtual { live: vec![(250_000_000, live)], ..Virtual::default() };
    let ran = run_real(STOPPED, real_time(0), &Arc::default(), &mut control);
    assert_eq!(ran.end.status, Status::Ok);
    // Given at 250 ms, between the reads of 200 and 300 ms: the read at 300 ms sees it, so three iterations.
    assert_eq!(ran.outputs()[0], "3.0");
    assert_eq!(ran.end.clock, 300 * MS);
    let applied = &ran.end.panel[0];
    assert!(applied.applied && applied.setting.by == SetBy::RunSetElement && applied.setting.at == 250 * MS);
    assert_eq!(control.applied, [(7, Some(250 * MS))]);
}

#[test]
fn a_setting_of_run_set_element_given_as_the_run_ends_never_applies() {
    let mut settings = real_time(0);
    settings.panel = vec![PanelSetting { element: ElementId(0), value: Value::Bool(true), at: 0, by: SetBy::Set }];
    let live = Live { tag: 9, element: ElementId(0), value: Value::Bool(false) };
    let mut control = Virtual { live: vec![(5000 * 1_000_000, live)], ..Virtual::default() };
    let ran = run_real(STOPPED, settings, &Arc::default(), &mut control);
    // The `set` at 0 ms stops the loop at its first read, 100 ms; the setting due at 5 s is never taken.
    assert_eq!(ran.end.clock, 100 * MS);
    assert_eq!(
        ran.end.panel.iter().map(|applied| (applied.setting.by, applied.applied)).collect::<Vec<_>>(),
        [(SetBy::Set, true), (SetBy::RunSetElement, false)]
    );
    assert_eq!(control.applied, [(9, None)]);
}

#[test]
fn in_real_time_a_timeout_counts_real_time() {
    let mut settings = real_time(0);
    settings.timeout = Some(150 * MS);
    let ran = run_real(STOPPED, settings, &Arc::default(), &mut Virtual::default());
    assert_eq!(ran.end.status, Status::Aborted);
    // cli.md 6.2: the loop waits until 200 ms; the timeout at 150 ms aborts it there, and no native call is in progress.
    let abort = ran.end.abort.as_ref().unwrap();
    assert_eq!((abort.by, abort.at, ran.end.clock), (AbortBy::Timeout, 150 * MS, 150 * MS));
}

/// `STOPPED`, with a `text_box` that nothing reads, `ElementId(1)`.
const STOPPED_WITH_A_NOTE: &str = "labcraft 1

use core
use time
use ui

fn f() -> (n: f64, err: error) {
    n, err = loop carry n = 0.0, carry e = none {
        e1 = time.wait(100 ms, err: e)
        stop = ui.get(stop_switch, after: e1)
        next n = core.add(n, 1.0)
        next e = e1
        until stop
    }
}

panel {
    control stop_switch: switch
    control note: text_box
}
";

#[test]
fn a_setting_of_run_set_element_that_does_not_fit_is_refused_and_the_run_goes_on() {
    let mut settings = real_time(0);
    settings.max_memory = 1 << 20;
    // Two settings in one pass, at 50 ms: a text of 2 MiB, past the 1 MiB of the run, then the switch.
    let big = Live { tag: 1, element: ElementId(1), value: Value::string("x".repeat(2 << 20)) };
    let stop = Live { tag: 2, element: ElementId(0), value: Value::Bool(true) };
    let mut control = Virtual { live: vec![(50 * 1_000_000, big), (50 * 1_000_000, stop)], ..Virtual::default() };
    let ran = run_real(STOPPED_WITH_A_NOTE, settings, &Arc::default(), &mut control);
    // Decision of the owner, 2026-11-11: the first is refused, `LC8005`, and the run goes on; the second applies, and the
    // read at 100 ms stops the loop.
    assert_eq!(ran.end.status, Status::Ok, "{:?}", ran.end.fault);
    assert_eq!(ran.outputs()[0], "1.0");
    assert_eq!(control.refused, [(1, FaultCode::Memory)]);
    assert_eq!(control.applied, [(2, Some(50 * MS))]);
    assert_eq!(ran.end.panel.len(), 1);
    assert_eq!(ran.end.held, 0, "what the panel kept is given back when the run ends");
}

#[test]
fn the_values_a_panel_keeps_count_in_the_memory_of_the_run() {
    let mut settings = real_time(0);
    settings.max_memory = 1 << 20;
    // Three texts of 400,000 bytes, at 10, 20 and 30 ms: `panel` keeps each, so the third would hold 1.2 MB, past 1 MiB,
    // although the element holds only the last.
    let note = |tag: u64, letter: &str| Live { tag, element: ElementId(1), value: Value::string(letter.repeat(400_000)) };
    let stop = Live { tag: 4, element: ElementId(0), value: Value::Bool(true) };
    let live = vec![(10 * 1_000_000, note(1, "a")), (20 * 1_000_000, note(2, "b")), (30 * 1_000_000, note(3, "c")), (40 * 1_000_000, stop)];
    let mut control = Virtual { live, ..Virtual::default() };
    let ran = run_real(STOPPED_WITH_A_NOTE, settings, &Arc::default(), &mut control);
    assert_eq!(ran.end.status, Status::Ok, "{:?}", ran.end.fault);
    assert_eq!(control.refused, [(3, FaultCode::Memory)]);
    assert_eq!(control.applied, [(1, Some(10 * MS)), (2, Some(20 * MS)), (4, Some(40 * MS))]);
    assert_eq!(ran.end.held, 0);
}

#[test]
fn in_real_time_a_setting_of_set_is_dated_when_it_applies() {
    let mut settings = real_time(0);
    settings.panel = vec![PanelSetting { element: ElementId(0), value: Value::Bool(true), at: 150 * MS, by: SetBy::Set }];
    // Each wait wakes 5 ms late: the reads come at 105 and 210 ms, and the setting due at 150 ms applies at 155 ms, the
    // time `panel` gives (decision of the owner, 2026-11-11), not the time it was given for.
    let mut control = Virtual { late: 5 * 1_000_000, ..Virtual::default() };
    let ran = run_real(STOPPED, settings, &Arc::default(), &mut control);
    assert_eq!(ran.end.status, Status::Ok);
    assert_eq!(ran.outputs()[0], "2.0");
    assert!(ran.end.panel[0].applied);
    assert_eq!(ran.end.panel[0].setting.at, 155 * MS);
}
