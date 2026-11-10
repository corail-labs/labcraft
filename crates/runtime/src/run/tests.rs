//! Runs of small programs through `check`, `ir` and the scheduler. Every expected value is worked out by hand from the program
//! and the rules it cites (spec 8, 8.2; cli.md 6.2); the times are those of the waits written in it.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use labcraft_check::{Checked, Content, Found, Libraries, LibrarySource, Origin, SourceFile, check_for_run};
use labcraft_ir::{ElementId, Node, Program, lower_for_run};
use labcraft_types::{Interner, Limits, Severity};

use super::{AbortBy, Control, Native, Observer, PanelSetting, Run, RunEnd, SetBy, Settings, Status, run};
use crate::fault::FaultCode;
use crate::native::{CallContext, CallId, HandleInfo, Key, LibraryCode, LibraryRun, LogBudget, NativeLibrary, NativeOutcome, Simulation, Step};
use crate::value::{Handle, Value};

const MS: i64 = 1_000_000;

/// A library shipped for these tests: `bench.open` gives a handle, `bench.slow` holds the key of its handle for one second,
/// `bench.close` closes it and is `always`.
const BENCH: &[(&str, &str)] = &[
    ("port.lct", "labcraft 1\n\ntype port = opaque\n"),
    ("open.lcf", "labcraft 1\n\nextern rust(crate: \"test\", function: \"open\")\nfn open(err: error = none) -> (h: port, err: error)\n"),
    ("slow.lcf", "labcraft 1\n\nextern rust(crate: \"test\", function: \"slow\")\nfn slow(h: port, err: error = none) -> (h: port, err: error)\n"),
    ("close.lcf", "labcraft 1\n\nextern rust(crate: \"test\", function: \"close\")\nfn close(h: port, err: error = none) -> (err: error)\n"),
];

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

/// What the test library saw: the calls cancelled and the handles released.
#[derive(Default)]
struct Seen {
    cancelled: Vec<u64>,
    released: Vec<u64>,
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
    fn ran(&mut self, at: i64, _: u64, node: &Node, iter: &[u64], _: &[Value], skipped: bool) {
        let what = if skipped { "skipped" } else { "ran" };
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
        let writer = crate::text::Writer { env: crate::convert::Env { program: &self.program, names: &self.names }, viewer: function.library };
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
    let end = run(Run { program: &program, names: &names, files: &checked.files, natives: &natives, inputs, settings }, &mut Still, &mut heard);
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
