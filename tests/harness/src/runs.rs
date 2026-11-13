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

/// The folder of a case read and checked, its function and its drawing, as `labcraft-cli diagram` and `diagram.svg` draw
/// it (`engine::drawings`), in the repository in memory.
fn drawing_of_case(
    memory: &Arc<Memory>,
    params: &Json,
) -> (labcraft_engine::drawings::Loaded, labcraft_engine::drawings::Function, labcraft_diagram::Drawing) {
    use labcraft_engine::drawings;
    let folder = params["folder"].as_str().unwrap_or(".");
    let function = params["function"].as_str().unwrap().to_owned();
    let shipped = crate::engine::shipped();
    let mut loaded =
        drawings::load(&**memory, &shipped, std::path::Path::new(&format!("/{folder}")), &[], &Limits::default(), Some(std::path::Path::new("/")));
    let mut report = std::mem::replace(&mut loaded.report, labcraft_types::Report::new(0));
    let root = loaded.root.clone().unwrap();
    let (asked, _) = drawings::asked(std::slice::from_ref(&function), &loaded.functions, &root, &mut report);
    let mut drawn = drawings::drawings(&asked, &loaded, &labcraft_diagram::DiagramLimits::default(), &mut report);
    let found = asked[0].clone();
    let drawing = drawn.remove(0).0.unwrap_or_else(|| panic!("{function}: not drawn"));
    (loaded, found, drawing)
}

/// Every trace of the corpus fits the drawing of its function (`docs/cli.md`, 7.4: a trace that matches its files never
/// holds an event that fits nothing): the reader keeps every event of frame 0, so that each node of the drawing is named
/// as the runtime names it, each wire with the type the runtime gives it, each loop and each arm told apart.
#[test]
fn every_trace_of_the_corpus_fits_the_drawing_of_its_function() {
    let memory = repository();
    let mut read = 0;
    for case in RUNS {
        let params: Json = serde_json::from_str(case.run).unwrap();
        if real_time(&params) {
            continue;
        }
        let Some(trace) = run_case_on(case, &memory).trace else { continue };
        let (_, _, drawing) = drawing_of_case(&memory, &params);
        let mut reader = Reader::new(labcraft_diagram::expected(&drawing), ReadLimits::default());
        reader.feed(&trace);
        let found = reader.finish();
        let text = String::from_utf8_lossy(&trace);
        let first = found.first_ignored.and_then(|line| text.lines().nth(usize::try_from(line).unwrap() - 1)).unwrap_or_default();
        assert!(found.header.is_ok(), "{}: {:?}", case.name, found.header);
        assert_eq!((found.ignored, found.stop), (0, None), "{}: the first ignored: {first}", case.name);
        read += 1;
    }
    // The cases that trace: every one that is not refused, 20 when this was written.
    assert!(read >= 10, "{read} traces read");
}

/// The SVG of the function of each case that traces, with the values of its run, as `labcraft-cli diagram --trace` draws
/// it, through `engine::drawings::read_trace`, compared byte for byte with its snapshot in
/// `tests/harness/snapshots/trace/`, natively and in WASM (scenario 6, point 7; architecture 11.6). With
/// `LABCRAFT_BLESS=1`, natively, the snapshots are written again, to be read by hand and opened in a browser before a
/// commit.
#[test]
fn the_values_of_each_run_draw_their_snapshot() {
    let memory = repository();
    let mut written = std::collections::BTreeMap::new();
    for case in RUNS {
        let params: Json = serde_json::from_str(case.run).unwrap();
        if real_time(&params) {
            continue;
        }
        let ran = run_case_on(case, &memory);
        let Some(file) = ran.report["trace"]["file"].as_str() else { continue };
        let (loaded, function, drawing) = drawing_of_case(&memory, &params);
        let mut report = labcraft_types::Report::new(0);
        let path = std::path::PathBuf::from(format!("/{file}"));
        let (read, part) = labcraft_engine::drawings::read_trace(
            &*memory,
            &path,
            file,
            Some(&drawing),
            &function,
            &loaded,
            labcraft_engine::drawings::DEFAULT_TRACE_SIZE,
            None,
            &mut report,
        );
        report.finish();
        let overlay = read.map(|(trace, judged)| labcraft_diagram::values(&drawing, &trace, &judged, &labcraft_diagram::Choice::default()));
        assert_eq!(
            (part.ignored, part.stale.len(), report.diagnostics().len()),
            (0, 0, 0),
            "{}: {:?} {:?}",
            case.name,
            report.diagnostics(),
            part.stale
        );
        let svg = labcraft_diagram::svg_with(&drawing, overlay.as_ref(), labcraft_diagram::Theme::Both);
        written.insert(format!("tests/harness/snapshots/trace/{}.svg", case.name), svg);
    }
    #[cfg(not(target_arch = "wasm32"))]
    if std::env::var_os("LABCRAFT_BLESS").is_some() {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for (path, text) in &written {
            let path = base.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        return;
    }
    let stored: std::collections::BTreeMap<&str, &str> =
        crate::TRACED.iter().map(|snapshot| (snapshot.path, std::str::from_utf8(snapshot.bytes).unwrap())).collect();
    assert_eq!(
        stored.keys().copied().collect::<Vec<_>>(),
        written.keys().map(String::as_str).collect::<Vec<_>>(),
        "snapshots without a case, or cases without a snapshot"
    );
    for (path, text) in &written {
        assert_eq!(stored.get(path.as_str()).copied(), Some(text.as_str()), "{path}: write the snapshots again with LABCRAFT_BLESS=1");
    }
}

/// What a node does not bind is worked out from the trace (notation 6.1), at the last iteration of the loop: by hand, for
/// `values` = [1.0, 2.0, 4.0] and `start` = `Reading.Value(10.0)`, at iteration 2 the element `x` is 4.0 and the index `i`
/// is 2; the carry `t` is its `next` of iteration 1, 1.0 + 2.0 = 3.0, and the `next t` of iteration 2 is 3.0 + 4.0 =
/// 7.0; the carry `k`, passed on unchanged, is its first value 5.0 at every iteration; `v`, bound by the arm
/// `Reading.Value(v)`, is 10.0. The loop gives 7.0, 5.0 and 2 (the `next j` of iteration 2, which `last` converts from
/// `i32` to `i64`); `j` has no wire, nothing reading it, so no value.
#[test]
fn the_values_that_no_node_binds_are_worked_out() {
    let memory = repository();
    memory.put(
        "/walk/Reading.lct",
        b"labcraft 1

type Reading = enum { Value(f64), Overload }
"
        .to_vec(),
    );
    memory.put(
        "/walk/walk.lcf",
        b"labcraft 1

use core

fn walk(
    values: [f64] = [1.0, 2.0, 4.0],
    start: Reading = Reading.Value(10.0),
) -> (total: f64, same: f64, last: i64, got: f64) {
    got = match start {
        Reading.Value(v) => yield v
        Reading.Overload => yield 0.0
    }
    total, same, last = for x in values, i in 0..3, carry t = 0.0, carry k = 5.0, carry j = 0 {
        next j = i
        next k = k
        next t = core.add(t, x)
    }
}
"
        .to_vec(),
    );
    let params = serde_json::json!({ "function": "walk", "folder": "walk" });
    #[cfg(not(target_arch = "wasm32"))]
    let report = session(&memory).execute("run.start", &params).unwrap();
    #[cfg(target_arch = "wasm32")]
    let report = labcraft_engine::run_to_end(&session(&memory), &params).unwrap();
    assert_eq!(report["status"], "ok", "{report}");
    let file = report["trace"]["file"].as_str().unwrap();
    let (loaded, function, drawing) = drawing_of_case(&memory, &params);
    let mut diagnostics = labcraft_types::Report::new(0);
    let path = std::path::PathBuf::from(format!("/{file}"));
    let (read, part) = labcraft_engine::drawings::read_trace(
        &*memory,
        &path,
        file,
        Some(&drawing),
        &function,
        &loaded,
        labcraft_engine::drawings::DEFAULT_TRACE_SIZE,
        None,
        &mut diagnostics,
    );
    assert_eq!((part.ignored, part.stale.len()), (0, 0), "{part:?}");
    let overlay = read.map(|(trace, judged)| labcraft_diagram::values(&drawing, &trace, &judged, &labcraft_diagram::Choice::default()));
    let values: Vec<(String, String)> = overlay.unwrap().bubbles.into_iter().map(|bubble| (bubble.wire, bubble.title)).collect();
    let expected = [
        ("values", "[1.0,2.0,4.0]"),
        ("start", r#"{"Reading.Value":[10.0]}"#),
        ("got", "10.0"),
        ("v", "10.0"),
        ("total", "7.0"),
        ("same", "5.0"),
        ("last", "2"),
        ("i", "2"),
        ("k", "5.0"),
        ("t", "3.0"),
        ("x", "4.0"),
        ("t", "7.0"),
    ];
    assert_eq!(values, expected.map(|(wire, title)| (wire.to_owned(), title.to_owned())));
}

/// What reading `bytes` for `drawing` says: its diagnostics, by code and line (`docs/cli.md`, 7.4 and 7.6).
fn read_codes(drawing: &labcraft_diagram::Drawing, bytes: &[u8], limits: ReadLimits) -> Vec<(String, u32)> {
    let mut reader = Reader::new(labcraft_diagram::expected(drawing), limits);
    reader.feed(bytes);
    let trace = reader.finish();
    // The values over the drawing are worked out whatever the trace holds, and the SVG written.
    let overlay = labcraft_diagram::values(drawing, &trace, &labcraft_diagram::Judged::default(), &labcraft_diagram::Choice::default());
    assert!(labcraft_diagram::svg_with(drawing, Some(&overlay), labcraft_diagram::Theme::Both).ends_with("</svg>\n"));
    labcraft_diagram::values::diagnostics(&trace, "t.jsonl")
        .into_iter()
        .map(|diagnostic| (diagnostic.code().as_str().to_owned(), diagnostic.span.map_or(0, |span| span.start.line)))
        .collect()
}

/// The family Traces of scenario 4 (`docs/acceptance-m0.md`), through the reader of the viewer, on the trace of
/// `check_voltage` against `dmm-230V` (case `s1-pass-230V`), each case built from it by hand and its diagnostic worked out
/// from `docs/cli.md`, 7.4: what reading stops at gives `LC7103` at its line, what fits nothing `LC7104` at the line of
/// the first, a first line that gives no values `LC7101`, a trace of another function `LC7102`. And every prefix of every
/// trace of the corpus, byte by byte, reads without a fault.
#[test]
fn the_family_traces_reads_without_a_fault() {
    let memory = repository();
    let case = RUNS.iter().find(|case| case.name == "s1-pass-230V").unwrap();
    let params: Json = serde_json::from_str(case.run).unwrap();
    let base = String::from_utf8(run_case_on(case, &memory).trace.unwrap()).unwrap();
    let (_, _, drawing) = drawing_of_case(&memory, &params);
    let lines: Vec<&str> = base.lines().collect();
    let joined = |lines: &[String]| lines.join("\n") + "\n";
    let owned: Vec<String> = lines.iter().map(|line| (*line).to_owned()).collect();
    let with = |line: usize, text: &str| {
        let mut changed = owned.clone();
        changed[line] = text.to_owned();
        joined(&changed)
    };
    let inserted = |line: usize, text: &str| {
        let mut changed = owned.clone();
        changed.insert(line, text.to_owned());
        joined(&changed)
    };
    let code = |code: &str, line: usize| vec![(code.to_owned(), u32::try_from(line).unwrap())];
    let defaults = ReadLimits::default();
    assert_eq!(read_codes(&drawing, base.as_bytes(), defaults), vec![], "the trace as the run wrote it");
    // The first line.
    assert_eq!(read_codes(&drawing, with(0, "{").as_bytes(), defaults), code("LC7101", 1));
    assert_eq!(read_codes(&drawing, with(0, &lines[0].replace("labcraft-trace/1", "other/1")).as_bytes(), defaults), code("LC7101", 1));
    assert_eq!(read_codes(&drawing, with(0, &lines[0].replace("labcraft-trace/1", "labcraft-trace/2")).as_bytes(), defaults), code("LC7101", 1));
    assert_eq!(read_codes(&drawing, with(0, &lines[0].replace("\"check_voltage\"", "\"other\"")).as_bytes(), defaults), code("LC7102", 1));
    // A line that does not read, in the middle: line 4 of the file.
    assert_eq!(read_codes(&drawing, with(3, "not json").as_bytes(), defaults), code("LC7103", 4));
    // Each limit, just passed: the file by a byte, the longest line by a byte, the lines by one, the values kept.
    let length = base.len() as u64;
    let file = read_codes(&drawing, base.as_bytes(), ReadLimits { file: length - 1, ..defaults });
    assert_eq!(file.first().map(|found| found.0.as_str()), Some("LC7103"), "{file:?}");
    // The first line is the longest of this trace: over the limit of a line, it gives no values.
    let header = lines[0].len();
    assert_eq!(read_codes(&drawing, base.as_bytes(), ReadLimits { line: header - 1, ..defaults }), code("LC7101", 1));
    let events = lines.len() as u64 - 1;
    assert_eq!(read_codes(&drawing, base.as_bytes(), ReadLimits { lines: events - 1, ..defaults }), code("LC7103", lines.len()));
    assert_eq!(read_codes(&drawing, base.as_bytes(), ReadLimits { kept: 1, ..defaults }), code("LC7103", 1));
    // The `ran` of the node `raw`: a value over the 64 KiB a run writes, a value nested deeper than a line reads.
    let raw = lines.iter().position(|line| line.contains("\"node\":\"raw\"") && line.contains("\"ran\"") && line.contains("\"frame\":0")).unwrap();
    let long =
        format!("{{\"seq\":99,\"t_ms\":0.0,\"event\":\"ran\",\"frame\":0,\"node\":\"raw\",\"out\":{{\"raw\":\"{}\"}}}}", "a".repeat(65 * 1024));
    assert_eq!(read_codes(&drawing, with(raw, &long).as_bytes(), defaults), code("LC7104", raw + 1));
    // A line of an event, just over the limit of a line.
    assert_eq!(read_codes(&drawing, with(raw, &long).as_bytes(), ReadLimits { line: long.len() - 1, ..defaults }), code("LC7103", raw + 1));
    let deep = format!(
        "{{\"seq\":99,\"t_ms\":0.0,\"event\":\"ran\",\"frame\":0,\"node\":\"raw\",\"out\":{{\"raw\":{}1{}}}}}",
        "[".repeat(300),
        "]".repeat(300)
    );
    assert_eq!(read_codes(&drawing, with(raw, &deep).as_bytes(), defaults), code("LC7103", raw + 1));
    // Numbers: negative ones do not read, huge ones fit nothing; so do an unknown node, and iterations whose levels are
    // not the loops around the node. Each event goes before the last line.
    let last = lines.len() - 1;
    for (fields, expected) in [
        (r#""frame":0,"node":"raw","iter":[-1]"#, "LC7103"),
        (r#""frame":-1,"node":"raw""#, "LC7103"),
        (r#""frame":0,"node":"raw","loops":["x"],"iter":[9223372036854775808]"#, "LC7104"),
        (r#""frame":18446744073709551615,"node":"raw""#, "LC7104"),
        (r#""frame":0,"node":"nowhere""#, "LC7104"),
        (r#""frame":0,"node":"raw","loops":["x"],"iter":[0]"#, "LC7104"),
    ] {
        let event = format!("{{\"seq\":99,\"t_ms\":0.0,\"event\":\"ran\",{fields},\"out\":{{\"raw\":1.0}}}}");
        assert_eq!(read_codes(&drawing, inserted(last, &event).as_bytes(), defaults), code(expected, last + 1), "{fields}");
    }
    let seq = lines[raw].split(",\"t_ms\"").nth(1).map(|rest| format!("{{\"seq\":-1,\"t_ms\"{rest}")).unwrap();
    assert_eq!(read_codes(&drawing, with(raw, &seq).as_bytes(), defaults), code("LC7103", raw + 1));
    // A value of another type than its wire, and the same value twice.
    let typed = lines[raw].replacen("\"out\":", "\"types\":{\"raw\":\"string\"},\"out\":", 1);
    assert_eq!(read_codes(&drawing, with(raw, &typed).as_bytes(), defaults), code("LC7104", raw + 1));
    assert_eq!(read_codes(&drawing, inserted(raw + 1, lines[raw]).as_bytes(), defaults), code("LC7104", raw + 2));
    // Every prefix of every trace of the runs of `examples/`, byte by byte; natively, where it takes a few seconds.
    #[cfg(target_arch = "wasm32")]
    return;
    let mut prefixes = 0;
    for case in RUNS {
        let params: Json = serde_json::from_str(case.run).unwrap();
        if real_time(&params) || params["folder"] != "examples" {
            continue;
        }
        let Some(trace) = run_case_on(case, &memory).trace else { continue };
        let (_, _, drawing) = drawing_of_case(&memory, &params);
        // One reader fed a byte at a time; each prefix read to its end on a clone of it.
        let mut reader = Reader::new(labcraft_diagram::expected(&drawing), defaults);
        let mut going = true;
        for end in 0..=trace.len() {
            if end > 0 && going {
                going = reader.feed(&trace[end - 1..end]);
            }
            let found = reader.clone().finish();
            assert!(found.events <= trace[..end].iter().filter(|byte| **byte == b'\n').count() as u64 + 1);
            // The values and the SVG worked out at the end of each line, where what is read changes.
            if end == trace.len() || trace.get(end) == Some(&b'\n') {
                read_codes(&drawing, &trace[..end], defaults);
            }
            prefixes += 1;
        }
    }
    assert!(prefixes > 1000, "{prefixes}");
}
