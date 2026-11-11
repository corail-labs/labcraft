//! The corpus of runs (`tests/acceptance/runs/`, `docs/architecture.md`, 10.11), through `engine` as it is: each `run.json`
//! is given to `run.start` unchanged, on a session whose root is the repository, held in memory (decision of the owner,
//! 2026-11-10). Natively through `Session::execute`, which runs it on a thread of its own; in WASM, which has no threads,
//! through `engine::run_to_end`, the same judging, run and report on the caller's thread. The report is compared with the
//! parts of it that `expected.json` gives, and the trace that the run wrote to `.labcraft/runs/` is read back.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use labcraft_engine::{Config, Memory, Session};
use labcraft_trace::{Expected, ExpectedNode, ExpectedWire, ReadLimits, Reader};
use labcraft_types::Limits;
use serde_json::Value as Json;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

use crate::{PROGRAMS, RUNS, RunCase};

/// The repository in memory: the programs of `examples/`, `libs/` and `docs/rfc/`, and the files of every case, at their
/// paths from the root.
fn repository() -> Arc<Memory> {
    repository_as(|bytes| bytes.to_vec())
}

/// [`repository`], each file as `written` gives it.
fn repository_as(written: impl Fn(&[u8]) -> Vec<u8>) -> Arc<Memory> {
    let memory = Memory::new();
    for file in PROGRAMS.iter().chain(RUNS.iter().flat_map(|case| case.files.iter())) {
        memory.put(format!("/{}", file.path), written(file.bytes));
    }
    Arc::new(memory)
}

/// A function file with the lines of each body in the reverse of their order, but `collect`, whose written order means
/// something, as `crates/format/tests/round_trip.rs` writes them, where `fmt` brings them back to the canonical text. Any
/// other file as it is.
fn reversed(bytes: &[u8]) -> Vec<u8> {
    let mut names = labcraft_types::Interner::new();
    let Some(mut file) = labcraft_format::read(bytes, &mut names, &Limits::default()).file else { return bytes.to_vec() };
    let labcraft_graph::Decl::Function(function) = &mut file.decl else { return bytes.to_vec() };
    reverse_body(&mut function.body);
    labcraft_format::write(&file, &names, &Limits::default()).map_or_else(|_| bytes.to_vec(), String::into_bytes)
}

fn reverse_body(body: &mut labcraft_graph::Body) {
    use labcraft_graph::{Rhs, StatementKind};
    let collects = body.statements.iter().filter(|statement| matches!(statement.kind, StatementKind::Collect(_))).count();
    if collects < 2 {
        body.statements.reverse();
    }
    for statement in &mut body.statements {
        if let StatementKind::Binding { value, .. } = &mut statement.kind {
            match value {
                Rhs::Match(branching) | Rhs::Handle(branching) => {
                    for arm in &mut branching.arms {
                        reverse_body(&mut arm.body);
                    }
                }
                Rhs::For(structure) => reverse_body(&mut structure.body),
                Rhs::Loop(structure) => reverse_body(&mut structure.body),
                Rhs::Call(_) | Rhs::Struct(_) | Rhs::Operand(_) => {}
            }
        }
    }
}

/// What a run gave, without where a line is: an error and a fault name the line of their node, which the order moves.
fn results(report: &Json) -> Json {
    fn without_lines(value: &mut Json) {
        match value {
            Json::Object(object) => {
                object.retain(|key, _| key != "line" && key != "span");
                object.values_mut().for_each(without_lines);
            }
            Json::Array(items) => items.iter_mut().for_each(without_lines),
            _ => {}
        }
    }
    let mut kept = serde_json::Map::new();
    for key in ["status", "outputs", "error", "fault", "abort", "resources", "instruments", "logs"] {
        if let Some(value) = report.get(key) {
            kept.insert(key.to_owned(), value.clone());
        }
    }
    kept.insert("simulated_ms".to_owned(), report["clock"]["simulated_ms"].clone());
    let mut kept = Json::Object(kept);
    without_lines(&mut kept);
    kept
}

fn session(memory: &Arc<Memory>) -> Session {
    session_with(memory, Arc::new(labcraft_engine::Still))
}

fn session_with(memory: &Arc<Memory>, clock: Arc<dyn labcraft_engine::Clock>) -> Session {
    let config = Config {
        root: "/".into(),
        lib_paths: Vec::new(),
        limits: Limits::default(),
        shipped: crate::engine::shipped(),
        natives: vec![Arc::new(labcraft_visa::Visa)],
        clock,
        process: 0,
    };
    Session::new(Arc::clone(memory) as Arc<dyn labcraft_engine::Workspace>, config, false).unwrap()
}

/// What a case gave: its report, and its trace, when it wrote one.
struct Ran {
    report: Json,
    trace: Option<Vec<u8>>,
}

/// The real clock, for a case in real time, which the scheduler waits on.
#[cfg(not(target_arch = "wasm32"))]
struct Real(std::time::Instant);

#[cfg(not(target_arch = "wasm32"))]
impl labcraft_engine::Clock for Real {
    fn real_ns(&self) -> u64 {
        u64::try_from(self.0.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }
    /// From 1970: no test here asserts the time of day.
    fn utc_ns(&self) -> i64 {
        0
    }
}

fn real_time(params: &Json) -> bool {
    params["mode"] == "real_time"
}

fn run_case(case: &RunCase) -> Ran {
    run_case_on(case, &repository())
}

fn run_case_on(case: &RunCase, memory: &Arc<Memory>) -> Ran {
    let params: Json = serde_json::from_str(case.run).unwrap_or_else(|error| panic!("{}: run.json: {error}", case.name));
    let memory = Arc::clone(memory);
    #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
    let mut session = session(&memory);
    #[cfg(not(target_arch = "wasm32"))]
    if real_time(&params) {
        #[expect(clippy::disallowed_methods, reason = "a test of real time, natively only")]
        let clock = Real(std::time::Instant::now());
        session = session_with(&memory, Arc::new(clock));
    }
    #[cfg(not(target_arch = "wasm32"))]
    let report = session.execute("run.start", &params);
    #[cfg(target_arch = "wasm32")]
    let report = labcraft_engine::run_to_end(&session, &params);
    let report = report.unwrap_or_else(|error| panic!("{}: a command error: {}", case.name, error.to_json()));
    let trace = report["trace"]["file"].as_str().and_then(|file| memory.get(format!("/{file}")));
    Ran { report, trace }
}

/// What `expected` gives and `actual` does not hold: an object compares the keys it gives (`how` says how the values were
/// found, and is not compared), an array every element, in order; numbers compare as numbers, and a range
/// `{"at_least": a, "below": b}` takes a number from `a` to below `b`.
fn differences(expected: &Json, actual: &Json, at: &str, found: &mut Vec<String>) {
    if let (Some(range), Some(number)) = (range_of(expected), actual.as_f64()) {
        if number < range.0 {
            found.push(format!("{at}: {number}, expected at least {}", range.0));
        } else if number >= range.1 {
            found.push(format!("{at}: {number}, {PAST_THE_UPPER_BOUND} {}", range.1));
        }
        return;
    }
    match (expected, actual) {
        (Json::Object(wanted), Json::Object(held)) => {
            for (key, value) in wanted {
                if key == "how" {
                    continue;
                }
                match held.get(key) {
                    Some(actual) => differences(value, actual, &format!("{at}.{key}"), found),
                    None => found.push(format!("{at}.{key}: missing")),
                }
            }
        }
        (Json::Array(wanted), Json::Array(held)) => {
            if wanted.len() != held.len() {
                found.push(format!("{at}: {} elements, expected {}", held.len(), wanted.len()));
                return;
            }
            for (index, (value, actual)) in wanted.iter().zip(held).enumerate() {
                differences(value, actual, &format!("{at}[{index}]"), found);
            }
        }
        (Json::Number(wanted), Json::Number(held)) if wanted.as_f64() == held.as_f64() => {}
        (wanted, held) if wanted == held => {}
        (wanted, held) => found.push(format!("{at}: {held}, expected {wanted}")),
    }
}

/// What a difference past the upper bound of a range says: the only one a run in real time is given more tries for.
const PAST_THE_UPPER_BOUND: &str = "expected below";

/// A range of a number that depends on the machine: from `at_least` to below `below`.
fn range_of(expected: &Json) -> Option<(f64, f64)> {
    let object = expected.as_object().filter(|object| object.len() == 2)?;
    Some((object.get("at_least")?.as_f64()?, object.get("below")?.as_f64()?))
}

/// Each line of a trace reads as JSON, its events are numbered in order, and the last says the status of the report.
fn check_trace(name: &str, trace: &[u8], report: &Json) {
    let text = std::str::from_utf8(trace).unwrap();
    let lines: Vec<Json> = text.lines().map(|line| serde_json::from_str(line).unwrap_or_else(|error| panic!("{name}: {error}: {line}"))).collect();
    assert_eq!(lines[0]["schema"], "labcraft-trace/1", "{name}");
    let mut seq = 0;
    for line in &lines[1..] {
        let next = line["seq"].as_u64().unwrap();
        assert!(next > seq, "{name}: seq {next} after {seq}");
        seq = next;
        // The loop of each level of `iter`, named.
        if let Some(iter) = line["iter"].as_array() {
            assert_eq!(line["loops"].as_array().map(Vec::len), Some(iter.len()), "{name}: {line}");
        }
    }
    let end = lines.last().unwrap();
    assert_eq!((end["event"].as_str(), end["status"].as_str()), (Some("end"), report["status"].as_str()), "{name}");
    assert_eq!(report["trace"]["events"].as_u64(), Some(lines.len() as u64 - 1), "{name}: events of the report and lines of the trace");
}

#[test]
fn the_corpus_of_runs_gives_its_expected_reports() {
    assert!(RUNS.len() >= 18, "the corpus of runs is embedded");
    let mut failed = Vec::new();
    for case in RUNS {
        let params: Json = serde_json::from_str(case.run).unwrap_or_else(|error| panic!("{}: run.json: {error}", case.name));
        // Without threads, no run in real time (D-22): the scheduler in real time is tested apart, without the real clock.
        if cfg!(target_arch = "wasm32") && real_time(&params) {
            continue;
        }
        let expected: Json = serde_json::from_str(case.expected).unwrap_or_else(|error| panic!("{}: expected.json: {error}", case.name));
        assert!(expected.get("how").is_some(), "{}: expected.json says how its values were found", case.name);
        // A run in real time is given three tries for what depends on the speed of the machine, the bound of `clock`, and only
        // for that: the rest must hold on each try (decision of the owner, 2026-11-11).
        let tries = if real_time(&params) { 3 } else { 1 };
        let mut ran = run_case(case);
        let mut found = Vec::new();
        for attempt in 1..=tries {
            found.clear();
            differences(&expected, &ran.report, "", &mut found);
            let only_too_slow = !found.is_empty() && found.iter().all(|found| found.contains(PAST_THE_UPPER_BOUND));
            if attempt == tries || !only_too_slow {
                break;
            }
            ran = run_case(case);
        }
        // A field that says how the run ended, or that it was cut, is asserted absent when the case does not give it.
        for key in ["error", "fault", "abort", "truncated"] {
            if expected.get(key).is_none() && ran.report.get(key).is_some() {
                found.push(format!(".{key}: {}, expected none", ran.report[key]));
            }
        }
        assert_eq!(ran.report["schema"], "labcraft-run/1");
        if let Some(trace) = &ran.trace {
            check_trace(case.name, trace, &ran.report);
        }
        if !found.is_empty() {
            failed.push(format!("{}:\n  {}\n  report: {}", case.name, found.join("\n  "), ran.report));
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

/// Scenario 5, point 3, *with M0b*: the order of lines means nothing (spec 5.2), so the programs of every case, each body
/// written the other way round, run to the same results as their canonical text: status, outputs, error, fault, abort,
/// resources, the commands of each instrument, the logs and the simulated clock, where a line is aside. The runs in real
/// time are left out: their clock is a range.
#[test]
fn the_programs_written_in_another_order_run_the_same() {
    let canonical = repository();
    let other = repository_as(reversed);
    let mut moved = 0;
    for file in PROGRAMS.iter().chain(RUNS.iter().flat_map(|case| case.files.iter())) {
        moved += usize::from(reversed(file.bytes) != file.bytes);
    }
    // The functions of more than one line: 20 when this was written.
    assert!(moved >= 10, "the lines of {moved} files moved");
    let mut differ = Vec::new();
    for case in RUNS {
        let params: Json = serde_json::from_str(case.run).unwrap();
        if real_time(&params) {
            continue;
        }
        let (first, second) = (results(&run_case_on(case, &canonical).report), results(&run_case_on(case, &other).report));
        if first != second {
            differ.push(format!("{}:\n  canonical {first}\n  reversed  {second}", case.name));
        }
    }
    assert!(differ.is_empty(), "{}", differ.join("\n"));
}

/// The trace that the runtime writes is the one the reader of the viewer reads (M0v, `docs/architecture.md`, 11.2): the
/// values of `check_voltage` against `dmm-230V`, at the wires that the function draws, as the trace example of commands.md 8
/// gives them, and the arm that each structure took.
#[test]
fn the_reader_of_the_viewer_reads_the_trace_of_a_run() {
    let case = RUNS.iter().find(|case| case.name == "s1-pass-230V").unwrap();
    let ran = run_case(case);
    let wire = |name: &str, ty: &str| ExpectedWire { name: name.to_owned(), ty: ty.to_owned() };
    let node = |name: &str, wires: Vec<ExpectedWire>| ExpectedNode { name: name.to_owned(), wires, ..ExpectedNode::default() };
    // The nodes of `examples/check_voltage.lcf`, in the order of its lines; `measured` is node 7, `verdict` node 8.
    let expected = Expected {
        function: "check_voltage".to_owned(),
        inputs: vec![wire("resource", "string"), wire("low", "f64[V]"), wire("high", "f64[V]"), wire("err", "error")],
        nodes: vec![
            node("dmm", vec![wire("dmm", "visa.session"), wire("e1", "error")]),
            node("raw", vec![wire("raw", "f64"), wire("dmm2", "visa.session"), wire("e2", "error")]),
            node("e3", vec![wire("e3", "error")]),
            node("voltage", vec![wire("voltage", "f64[V]")]),
            node("above", vec![wire("above", "bool")]),
            node("below", vec![wire("below", "bool")]),
            node("in_range", vec![wire("in_range", "bool")]),
            node("measured", vec![wire("measured", "Verdict")]),
            node("verdict", vec![wire("verdict", "Verdict")]),
        ],
    };
    let mut reader = Reader::new(expected, ReadLimits::default());
    reader.feed(ran.trace.as_deref().unwrap());
    let trace = reader.finish();
    assert!(trace.header.is_ok(), "{:?}", trace.header);
    assert_eq!(trace.end.as_deref(), Some("ok"));
    assert_eq!(trace.input(0), Some(r#""SIM::DMM0""#));
    assert_eq!(trace.input(1), Some(r#""220.0 V""#));
    assert_eq!(trace.value(1, 0, &[]), Some("230.071217"));
    assert_eq!(trace.value(1, 1, &[]), Some(r#"{"opaque":"visa.session","resource":"SIM::DMM0","open":true}"#));
    assert_eq!(trace.value(3, 0, &[]), Some(r#""230.071217 V""#));
    assert_eq!(trace.value(6, 0, &[]), Some("true"));
    assert_eq!(trace.value(7, 0, &[]), Some(r#""Verdict.Pass""#));
    assert_eq!(trace.value(8, 0, &[]), Some(r#""Verdict.Pass""#));
    assert_eq!(trace.arm(7, &[]), Some("true"));
    assert_eq!(trace.arm(8, &[]), Some("ok"));
    assert_eq!(trace.value(2, 0, &[]), Some("null"));
    assert_eq!(trace.ignored, 0, "every event fits the function drawn");
}

/// commands.md 8: the two loops of `two_benches` bind the same names; each event names its loop, so the `m` of each
/// branch is told apart. By hand: the first iteration of each loop adds 1.0 to its carry `n`, 0.0, so `m` is 1.0 in both.
#[test]
fn the_nodes_of_two_loops_of_one_name_are_told_apart() {
    let case = RUNS.iter().find(|case| case.name == "s3-2-the-waits-overlap").unwrap();
    let trace = String::from_utf8(run_case(case).trace.unwrap()).unwrap();
    for (branch, count) in [("n_a", 1), ("n_b", 1)] {
        let event = format!(r#""event":"ran","frame":0,"node":"m","loops":["{branch}"],"iter":[0],"out":{{"m":1.0}}}}"#);
        assert_eq!(
            trace.lines().filter(|line| line.ends_with(&event)).count(),
            count,
            "{event}
{trace}"
        );
    }
}
