//! The report and the trace of small programs, run through `check`, `ir` and the scheduler. Every expected text is worked out
//! by hand from the program and section 5 of `docs/commands.md`, which says how each value is written.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::borrow::Cow;

use labcraft_check::{Checked, Content, Found, Libraries, LibrarySource, Origin, SourceFile, check_for_run};
use labcraft_ir::{Program, lower_for_run};
use labcraft_trace::{Header, WriteLimits};
use labcraft_types::{Interner, Limits};

use super::{RESULT_MAX, ReportOf, refused, report};
use crate::convert::Env;
use crate::run::{Control, Run, RunEnd, Settings, Unobserved, run};
use crate::trace::TraceObserver;
use crate::value::Value;

struct Nothing;

impl Libraries for Nothing {
    fn find(&self, _: &str, _: Origin) -> Found {
        Found::NotFound
    }
}

struct Still;

impl Control for Still {
    fn real_ns(&mut self) -> u64 {
        0
    }
}

fn checked(files: &[(&str, &str)], names: &mut Interner) -> (Checked, Program) {
    let mut root = LibrarySource::new(String::new(), String::new(), Origin::Project);
    for (shown, text) in files {
        root.files.push(SourceFile { shown: (*shown).to_owned(), content: Content::Bytes(Cow::Owned(text.as_bytes().to_vec())) });
    }
    let checked = check_for_run(root, &Nothing, names, &Limits::default());
    assert_eq!(checked.report.errors(), 0, "{:?}", checked.report.diagnostics());
    let start = checked.files.iter().position(|file| file.shown == files[0].0).unwrap();
    let program = lower_for_run(&checked, start, names).unwrap();
    (checked, program)
}

fn report_of(end: &RunEnd, checked: &Checked, program: &Program, names: &Interner) -> String {
    let of = ReportOf { run: "r1", program, names, files: &checked.files, natives: &[], real_ms: 0, check: "{}", trace: None, limit: RESULT_MAX };
    report(end, &of)
}

/// The JSON between `"outputs":` and the field after it.
fn outputs(report: &str) -> &str {
    let (_, rest) = report.split_once("\"outputs\":").unwrap();
    let (outputs, _) = rest.split_once("],\"").unwrap();
    // The `]` that `split_once` took ends the outputs.
    rest.get(..outputs.len() + 1).unwrap()
}

#[test]
fn a_report_writes_each_value_against_its_type() {
    let files = [
        (
            "f.lcf",
            "labcraft 1\n\nuse array\n\nfn f(x: f64[V]) -> (pair: Pair, r: Reading, many: [i32], d: duration, err: error) {\n    \
             pair = Pair { low: x, name: \"t\" }\n    r = Reading.Value(x)\n    many, err = array.repeat(7, 70)\n    d = 100 ms\n}\n",
        ),
        ("Pair.lct", "labcraft 1\n\ntype Pair = struct {\n    low: f64[V],\n    name: string,\n}\n"),
        ("Reading.lct", "labcraft 1\n\ntype Reading = enum { Value(f64[V]), Missing }\n"),
    ];
    let mut names = Interner::new();
    let (checked, program) = checked(&files, &mut names);
    let given =
        Run { program: &program, names: &names, files: &checked.files, natives: &[], inputs: vec![Value::F64(1.5)], settings: Settings::default() };
    let end = run(given, &mut Still, &mut Unobserved);
    let text = report_of(&end, &checked, &program, &names);
    // By hand, from commands.md 5: a struct is an object of its fields in declared order, a number with a unit a string in
    // that unit; a variant with values an object of one key; an array of 70 elements, past 64, its length and its first 16;
    // a duration in seconds; `none` is null.
    let head = ["7"; 16].join(",");
    assert_eq!(
        outputs(&text),
        format!(
            "[{{\"name\":\"pair\",\"type\":\"Pair\",\"value\":{{\"low\":\"1.5 V\",\"name\":\"t\"}}}},\
             {{\"name\":\"r\",\"type\":\"Reading\",\"value\":{{\"Reading.Value\":[\"1.5 V\"]}}}},\
             {{\"name\":\"many\",\"type\":\"[i32]\",\"value\":{{\"$len\":70,\"$head\":[{head}]}}}},\
             {{\"name\":\"d\",\"type\":\"duration\",\"value\":\"0.1 s\"}},\
             {{\"name\":\"err\",\"type\":\"error\",\"value\":null}}]"
        )
    );
    assert!(text.starts_with(r#"{"schema":"labcraft-run/1","status":"ok","run":"r1","function":"f","mode":"deterministic","#), "{text}");
    assert!(text.contains(r#""clock":{"simulated_ms":0.0,"real_ms":0},"resources":{"opened":0,"open":[],"leaked":[]}"#), "{text}");
    assert!(text.ends_with(r#","check":{}}"#), "{text}");
}

/// commands.md 2: a report stays within its bound however many outputs the function has, and says it was cut; the status
/// and the clock, which are never cut, are still there. Each of the 200 outputs holds 64 strings of 1,000 tabs, which JSON
/// writes as 2,000 bytes each: about 128 KiB per value, cut to 64 KiB, so about 13 MB without the bound of 1 MiB given.
#[test]
fn a_report_stays_within_its_bound() {
    let count = 200;
    let outputs: Vec<String> = (0..count).map(|index| format!("o{index}: [string]")).collect();
    let body: Vec<String> = (0..count).map(|index| format!("    o{index} = a\n")).collect();
    let source = format!(
        "labcraft 1\n\nuse array\n\nfn f(s: string) -> ({}, err: error) {{\n    a, err = array.repeat(s, 64)\n{}}}\n",
        outputs.join(", "),
        body.concat()
    );
    let files = [("f.lcf", source.as_str())];
    let mut names = Interner::new();
    let (checked, program) = checked(&files, &mut names);
    let inputs = vec![Value::string("\t".repeat(1000))];
    let given = Run { program: &program, names: &names, files: &checked.files, natives: &[], inputs, settings: Settings::default() };
    let end = run(given, &mut Still, &mut Unobserved);
    let limit = 1 << 20;
    let of =
        ReportOf { run: "r1", program: &program, names: &names, files: &checked.files, natives: &[], real_ms: 0, check: "{}", trace: None, limit };
    let text = report(&end, &of);
    assert!(text.len() <= limit, "{} bytes", text.len());
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!((json["status"].as_str(), json["truncated"].as_bool()), (Some("ok"), Some(true)));
    assert_eq!(json["outputs"].as_array().unwrap().len(), count + 1, "every output is named, its value cut");
    assert!(json["outputs"][count - 1]["value"].to_string().contains("\"$cut\":true"), "{}", json["outputs"][count - 1]);
    assert_eq!(json["clock"]["simulated_ms"], 0.0);
    assert_eq!(json["outputs"][count]["value"], serde_json::Value::Null, "the `none` of `err` is short: never cut");
}

/// commands.md 2: the list of outputs itself stays within the bound: past what fits, the outputs are counted in
/// `outputs_more`. 3,000 outputs `oN: i8` take about 40 bytes each without their value, 120 KB, past a bound of 64 KiB.
#[test]
fn a_report_of_many_outputs_counts_those_that_do_not_fit() {
    let count = 3000;
    let outputs: Vec<String> = (0..count).map(|index| format!("o{index}: i8")).collect();
    let body: Vec<String> = (0..count).map(|index| format!("    o{index} = x\n")).collect();
    let source = format!("labcraft 1\n\nfn f(x: i8) -> ({}) {{\n{}}}\n", outputs.join(", "), body.concat());
    let mut names = Interner::new();
    let (checked, program) = checked(&[("f.lcf", source.as_str())], &mut names);
    let given =
        Run { program: &program, names: &names, files: &checked.files, natives: &[], inputs: vec![Value::I8(0)], settings: Settings::default() };
    let end = run(given, &mut Still, &mut Unobserved);
    let limit = 64 << 10;
    let of =
        ReportOf { run: "r1", program: &program, names: &names, files: &checked.files, natives: &[], real_ms: 0, check: "{}", trace: None, limit };
    let text = report(&end, &of);
    assert!(text.len() <= limit, "{} bytes", text.len());
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    let shown = json["outputs"].as_array().unwrap().len();
    assert!(shown > 0 && shown < count, "{shown}");
    assert_eq!(json["outputs_more"].as_u64(), Some((count - shown) as u64));
    assert_eq!(json["truncated"].as_bool(), Some(true));
    // A short value is written whole, never replaced by the mark of a cut: each shown output is 0.
    assert!(json["outputs"].as_array().unwrap().iter().all(|output| output["value"] == 0), "{text}");
}

/// commands.md 7: `error` is the error of the output `err`, and only when the status is `error`.
#[test]
fn the_error_of_a_report_is_that_of_err() {
    let source = "labcraft 1\n\nuse errors\n\nfn f(first: error, second: error) -> (one: error, err: error) {\n    one = errors.merge(first)\n    \
                  err = errors.merge(second)\n}\n";
    let mut names = Interner::new();
    let (checked, program) = checked(&[("f.lcf", source)], &mut names);
    let error = |category: &str| {
        Value::Error(Some(std::sync::Arc::new(crate::value::ErrorData {
            category: category.into(),
            code: 1,
            message: "m".into(),
            origin: None,
            others: std::sync::Arc::default(),
        })))
    };
    for (second, status, field) in [(error("visa.closed"), "error", Some("visa.closed")), (Value::Error(None), "ok", None)] {
        let inputs = vec![error("io.timeout"), second];
        let given = Run { program: &program, names: &names, files: &checked.files, natives: &[], inputs, settings: Settings::default() };
        let end = run(given, &mut Still, &mut Unobserved);
        let json: serde_json::Value = serde_json::from_str(&report_of(&end, &checked, &program, &names)).unwrap();
        assert_eq!(json["status"].as_str(), Some(status));
        assert_eq!(json.get("error").map(|error| error["category"].as_str().unwrap()), field);
    }
}

#[test]
fn a_refused_report_gives_no_outputs_and_its_check() {
    let text = refused("r1", "f", 3, r#"{"summary":{"errors":1},"diagnostics":[]}"#);
    assert_eq!(
        text,
        r#"{"schema":"labcraft-run/1","status":"refused","run":"r1","function":"f","mode":"deterministic","outputs":[],"clock":{"simulated_ms":0.0,"real_ms":3},"resources":{"opened":0,"open":[],"leaked":[]},"check":{"summary":{"errors":1},"diagnostics":[]}}"#
    );
}

/// A loop of two iterations: its nodes give their iteration, the loop node its outputs once it ends, and the frame the type
/// of each value its body binds, the carry `next.i` included.
#[test]
fn the_trace_gives_the_iteration_of_each_node_in_a_loop() {
    let files = [(
        "g.lcf",
        "labcraft 1\n\nuse core\n\nfn g() -> (n: f64) {\n    n = loop carry i = 0.0 {\n        next i = core.add(i, 1.0)\n        \
         done = core.ge(i, 1.0)\n        until done\n    }\n}\n",
    )];
    let mut names = Interner::new();
    let (checked, program) = checked(&files, &mut names);
    let env = Env { program: &program, names: &names };
    let header = Header { run: "r1", function: "g", mode: "deterministic", files: &[("g.lcf", "0")] };
    let mut observer = TraceObserver::new(Vec::new(), &header, env, &checked.files, WriteLimits::default());
    let given = Run { program: &program, names: &names, files: &checked.files, natives: &[], inputs: vec![], settings: Settings::default() };
    let end = run(given, &mut Still, &mut observer);
    let traced = observer.finish(&end);
    let text = String::from_utf8(traced.sink).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    // By hand: iteration 0 binds next.i = 1.0 and done = false, iteration 1 next.i = 2.0 and done = true; the loop then gives
    // n = 2.0. The line of `next i` has no name of its own: the trace names it by its line, `@7`.
    assert_eq!(lines[0], r#"{"schema":"labcraft-trace/1","run":"r1","function":"g","mode":"deterministic","files":[{"file":"g.lcf","sha256":"0"}]}"#);
    assert!(lines[1].starts_with(r#"{"seq":1,"t_ms":0.0,"event":"call","frame":0,"function":"g","in":[],"types":{"#), "{}", lines[1]);
    for entry in [r#""n":"f64""#, r#""i":"f64""#, r#""next.i":"f64""#, r#""done":"bool""#] {
        assert!(lines[1].contains(entry), "{entry} in {}", lines[1]);
    }
    for event in [
        r#""event":"ran","frame":0,"node":"@7","loops":["n"],"iter":[0],"out":{"next.i":1.0}}"#,
        r#""event":"ran","frame":0,"node":"done","loops":["n"],"iter":[0],"out":{"done":false}}"#,
        r#""event":"ran","frame":0,"node":"@7","loops":["n"],"iter":[1],"out":{"next.i":2.0}}"#,
        r#""event":"ran","frame":0,"node":"done","loops":["n"],"iter":[1],"out":{"done":true}}"#,
        r#""event":"ran","frame":0,"node":"n","out":{"n":2.0}}"#,
    ] {
        assert_eq!(lines.iter().filter(|line| line.ends_with(event)).count(), 1, "{event} in\n{text}");
    }
    assert_eq!(*lines.last().unwrap(), format!(r#"{{"seq":{},"t_ms":0.0,"event":"end","status":"ok"}}"#, lines.len() - 1));
    assert_eq!(traced.events, lines.len() as u64 - 1);
    assert!(!traced.truncated && traced.error.is_none());
}
