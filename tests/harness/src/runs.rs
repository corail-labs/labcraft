//! The corpus of runs (`tests/acceptance/runs/`, `docs/architecture.md`, 10.11), through `check`, `ir`, `runtime` and `visa`,
//! natively and in WASM: each case is run as `run.start` would run it, its report written as `labcraft-run/1` and its trace
//! as `labcraft-trace/1`, and the report compared with the parts of it that `expected.json` gives.
//!
//! What `engine` will do around a run (step 6) is done here only as far as the cases need: the arguments and the settings
//! are converted for the types they use, and a `refused` report is made from what refused it. The converter of cli.md 6.3,
//! with its codes `LC8101` to `LC8108`, comes with `engine`, which this runner will then call.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::borrow::Cow;
use std::collections::BTreeMap;

use labcraft_check::{Checked, Content, Found, Libraries, LibrarySource, Origin, SourceFile, check_for_run};
use labcraft_ir::{ElementId, lower_for_run};
use labcraft_runtime::native::{LibraryDiagnostic, NativeLibrary};
use labcraft_runtime::report::{RESULT_MAX, ReportOf, refused, report};
use labcraft_runtime::run::{Control, Native, PanelSetting, Run, RunEnd, SetBy, Settings, run};
use labcraft_runtime::trace::TraceObserver;
use labcraft_runtime::{Env, Value, constant, simulation};
use labcraft_trace::{Expected, ExpectedNode, ExpectedWire, Header, ReadLimits, Reader, WriteLimits, sha256_hex};
use labcraft_types::{Interner, Limits, NumberKind, Ty, TyId};
use labcraft_visa::Visa;
use serde_json::{Value as Json, json};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

use crate::{Embedded, PROGRAMS, RUNS, RunCase, SIMS};

const CORPUS: &str = "tests/acceptance/runs";

struct Shipped;

impl Libraries for Shipped {
    fn find(&self, name: &str, _: Origin) -> Found {
        labcraft_libs::source(name).map_or(Found::NotFound, Found::Library)
    }
}

struct Still;

impl Control for Still {
    fn real_ns(&mut self) -> u64 {
        0
    }
}

/// A path of `run.json`, from the folder of the case, as a path from the root of the repository.
fn resolve(case: &str, path: &str) -> String {
    let mut parts: Vec<String> = format!("{CORPUS}/{case}").split('/').map(str::to_owned).collect();
    for part in path.split('/') {
        match part {
            "." | "" => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other.to_owned()),
        }
    }
    parts.join("/")
}

/// The embedded programs below a folder, by their paths from it: the library and its sub-libraries.
fn folder_files(case: &RunCase, folder: &str) -> Vec<(String, &'static Embedded)> {
    PROGRAMS
        .iter()
        .chain(case.files.iter())
        .filter(|file| file.path.ends_with(".lcf") || file.path.ends_with(".lct"))
        .filter_map(|file| Some((file.path.strip_prefix(folder)?.strip_prefix('/')?.to_owned(), file)))
        .collect()
}

/// A library from the paths of its files: each folder below it a sub-library, as a walk of the disk gives them.
fn library(name: &str, files: &[(String, &'static Embedded)]) -> LibrarySource {
    let mut source = LibrarySource::new(name.to_owned(), name.to_owned(), Origin::Project);
    let mut folders: BTreeMap<&str, Vec<(String, &'static Embedded)>> = BTreeMap::new();
    for (path, file) in files {
        match path.split_once('/') {
            Some((folder, rest)) => folders.entry(folder).or_default().push((rest.to_owned(), file)),
            None => source.files.push(SourceFile { shown: path.clone(), content: Content::Bytes(Cow::Borrowed(file.bytes)) }),
        }
    }
    source.sub_libraries = folders.into_iter().map(|(folder, files)| library(folder, &files)).collect();
    source
}

fn sim_text(case: &RunCase, path: &str) -> &'static str {
    let file =
        SIMS.iter().chain(case.files.iter()).find(|file| file.path == path).unwrap_or_else(|| panic!("{}: no simulation file {path}", case.name));
    std::str::from_utf8(file.bytes).unwrap()
}

/// A duration as `run.start` takes it: `"350 ms"`, `"5 s"`.
fn duration(text: &str) -> i64 {
    let text = text.trim();
    let split = text.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(text.len());
    let (number, unit) = (text[..split].trim(), text[split..].trim());
    let scale = match unit {
        "ns" => 1.0,
        "us" => 1e3,
        "ms" => 1e6,
        "s" => 1e9,
        other => panic!("a unit of duration this runner does not read: {other}"),
    };
    (number.parse::<f64>().unwrap() * scale).round() as i64
}

/// An argument or a setting, for the types the cases use.
fn convert(value: &Json, ty: TyId, env: Env<'_>) -> Value {
    match (env.ty(ty), value) {
        (Ty::String, Json::String(text)) => Value::string(text.as_str()),
        (Ty::Bool, Json::Bool(value)) => Value::Bool(*value),
        (Ty::Number { kind: NumberKind::F64, .. }, Json::Number(number)) => Value::F64(number.as_f64().unwrap()),
        (Ty::Duration, Json::String(text)) => Value::Duration(duration(text)),
        (other, value) => panic!("the converter of cli.md 6.3 comes with `engine`: {value} for {other:?}"),
    }
}

/// The `check` of a report: the summary and the diagnostics of `labcraft-check/1`, with those of the simulation file.
fn check_json(checked: &Checked, sim: Option<&str>, problems: &[LibraryDiagnostic]) -> String {
    let mut document = serde_json::to_value(checked.report.check_document(checked.counts, false)).unwrap();
    let diagnostics = document["diagnostics"].as_array_mut().unwrap();
    for problem in problems {
        let span = problem.span.map(|span| {
            json!({ "start": { "line": span.start.line, "column": span.start.column }, "end": { "line": span.end.line, "column": span.end.column } })
        });
        diagnostics.push(json!({ "code": problem.code, "severity": "error", "message": problem.message, "file": sim, "span": span }));
    }
    let errors = document["summary"]["errors"].as_u64().unwrap() + problems.len() as u64;
    document["summary"]["errors"] = json!(errors);
    json!({ "summary": document["summary"], "diagnostics": document["diagnostics"] }).to_string()
}

/// What a case gave: its report as JSON, and its trace, when it ran.
struct Ran {
    report: Json,
    trace: Option<Vec<u8>>,
}

fn run_case(case: &RunCase) -> Ran {
    let params: Json = serde_json::from_str(case.run).unwrap_or_else(|error| panic!("{}: run.json: {error}", case.name));
    let function = params["function"].as_str().unwrap();
    let folder = resolve(case.name, params["folder"].as_str().unwrap_or("."));
    let files = folder_files(case, &folder);
    let source = library("", &files);
    let mut names = Interner::new();
    let checked = check_for_run(source, &Shipped, &mut names, &Limits::default());
    let sim_path = params["sim"].as_str().map(|path| resolve(case.name, path));
    if checked.report.errors() > 0 {
        return Ran { report: serde_json::from_str(&refused("r1", function, 0, &check_json(&checked, None, &[]))).unwrap(), trace: None };
    }
    let start =
        checked.files.iter().position(|file| file.shown == format!("{function}.lcf")).unwrap_or_else(|| panic!("{}: no {function}", case.name));
    let program = lower_for_run(&checked, start, &mut names).unwrap_or_else(|refusal| panic!("{}: {refusal:?}", case.name));
    // The simulation file, judged by `visa` before anything runs.
    let simulations = match &sim_path {
        Some(path) => {
            let shown_sim = path.rsplit('/').next().unwrap();
            let judged = simulation::read(sim_text(case, path), 64)
                .map_err(|problem| vec![problem])
                .and_then(|file| file.judge(&[&Visa as &dyn NativeLibrary]));
            match judged {
                Ok(simulations) => simulations,
                Err(problems) => {
                    let check = check_json(&checked, Some(shown_sim), &problems);
                    return Ran { report: serde_json::from_str(&refused("r1", function, 0, &check)).unwrap(), trace: None };
                }
            }
        }
        None => vec![None],
    };
    let env = Env { program: &program, names: &names };
    let top = &program.functions[0];
    let args = params.get("args").cloned().unwrap_or(json!({}));
    let inputs: Vec<Value> = top
        .inputs
        .iter()
        .map(|port| match args.get(names.resolve(port.name)) {
            Some(value) => convert(value, port.ty, env),
            None => constant(env, port.default.unwrap_or_else(|| panic!("{}: no argument for an input without default", case.name))).unwrap(),
        })
        .collect();
    let mut settings = Settings::default();
    if let Some(seed) = params["schedule_seed"].as_u64() {
        settings.schedule_seed = Some(seed);
    }
    if let Some(timeout) = params["timeout"].as_str() {
        settings.timeout = Some(duration(timeout));
    }
    for (key, slot) in [("max_iterations", &mut settings.max_iterations), ("max_nodes", &mut settings.max_nodes)] {
        if let Some(value) = params[key].as_u64() {
            *slot = Some(value);
        }
    }
    if let Some(depth) = params["max_call_depth"].as_u64() {
        settings.max_call_depth = u32::try_from(depth).unwrap();
    }
    if let Some(memory) = params["max_memory"].as_u64() {
        settings.max_memory = memory;
    }
    for set in params["set"].as_array().map(Vec::as_slice).unwrap_or_default() {
        let element = top.elements.iter().position(|element| names.resolve(element.name) == set["element"].as_str().unwrap()).unwrap();
        let ty = top.elements[element].ty.unwrap();
        let value = convert(&set["value"], ty, env);
        settings.panel.push(PanelSetting { element: ElementId(element as u32), value, at: duration(set["at"].as_str().unwrap()), by: SetBy::Set });
    }
    let visa = checked.libraries.iter().find(|library| library.name == "visa");
    let natives: Vec<Native<'_>> =
        visa.map(|visa| Native { library: visa.id, native: &Visa, simulation: simulations.first().and_then(Option::as_ref) }).into_iter().collect();
    // The trace, in memory; the files of the folder on its first line, with their SHA-256.
    let hashed: Vec<(String, String)> = files.iter().map(|(shown, file)| (shown.clone(), sha256_hex(file.bytes))).collect();
    let header_files: Vec<(&str, &str)> = hashed.iter().map(|(file, sha)| (file.as_str(), sha.as_str())).collect();
    let header = Header { run: "r1", function: &top.shown, mode: "deterministic", files: &header_files };
    let mut observer = TraceObserver::new(Vec::new(), &header, env, &checked.files, WriteLimits::default());
    let given = Run { program: &program, names: &names, files: &checked.files, natives: &natives, inputs, settings };
    let end: RunEnd = run(given, &mut Still, &mut observer);
    let traced = observer.finish(&end);
    assert!(traced.error.is_none(), "{}: {:?}", case.name, traced.error);
    let trace =
        json!({ "file": format!(".labcraft/runs/{}.trace.jsonl", top.shown), "events": traced.events, "truncated": traced.truncated }).to_string();
    let check = check_json(&checked, None, &[]);
    let of = ReportOf {
        run: "r1",
        program: &program,
        names: &names,
        files: &checked.files,
        natives: &natives,
        real_ms: 0,
        check: &check,
        trace: Some(&trace),
        limit: RESULT_MAX,
    };
    let text = report(&end, &of);
    let report = serde_json::from_str(&text).unwrap_or_else(|error| panic!("{}: the report does not read as JSON: {error}\n{text}", case.name));
    Ran { report, trace: Some(traced.sink) }
}

/// What `expected` gives and `actual` does not hold: an object compares the keys it gives (`how` says how the values were
/// found, and is not compared), an array every element, in order; numbers compare as numbers.
fn differences(expected: &Json, actual: &Json, at: &str, found: &mut Vec<String>) {
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
        let ran = run_case(case);
        let expected: Json = serde_json::from_str(case.expected).unwrap_or_else(|error| panic!("{}: expected.json: {error}", case.name));
        assert!(expected.get("how").is_some(), "{}: expected.json says how its values were found", case.name);
        let mut found = Vec::new();
        differences(&expected, &ran.report, "", &mut found);
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
