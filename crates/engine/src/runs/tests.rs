//! Runs through the session, on projects in memory: what `run.start` refuses and with which code (cli.md 6.6), a run as a
//! task that `run.wait`, `run.status` and `run.abort` follow, the reports kept, the trace under `.labcraft/runs/` and
//! `run.trace`. The expected values are worked out by hand from `docs/commands.md`, `docs/cli.md` and the programs below.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::path::PathBuf;
use std::sync::Arc;

use labcraft_types::Limits;
use serde_json::{Value, json};

use crate::{Config, Memory, Session, Shipped, Still, Workspace};

const DOUBLE: &str = "labcraft 1\n\nuse core\n\n/// Doubles a number.\nfn double(x: f64) -> (y: f64) {\n    y = core.mul(x, 2)\n}\n";

/// Counts until it is aborted: `0.0 >= m` never holds once `m` is 1.
const FOREVER: &str = "labcraft 1\n\nuse core\n\n/// Counts for ever.\nfn forever() -> (n: f64) {\n    n = loop carry i = 0.0 {\n        m = core.add(i, 1.0)\n        done = core.ge(0.0, m)\n        next i = m\n        until done\n    }\n}\n";

/// Counts to 100, 100 iterations of 3 nodes.
const SPIN: &str = "labcraft 1\n\nuse core\n\n/// Counts to 100.\nfn spin() -> (n: f64) {\n    n = loop carry i = 0.0 {\n        m = core.add(i, 1.0)\n        done = core.ge(m, 100.0)\n        next i = m\n        until done\n    }\n}\n";

const TAKES: &str = "labcraft 1\n\nuse core\n\n/// One input of each kind.\nfn takes(v: f64[mV], n: u8, d: duration, k: Verdict, xs: [f64], s: Step, label: string = \"a\") -> (y: f64[mV], m: u8) {\n    y = core.add(v, 0 mV)\n    m = core.add(n, 1)\n}\n";
const VERDICT: &str = "labcraft 1\n\ntype Verdict = enum { Pass, Fail }\n";
const STEP: &str = "labcraft 1\n\ntype Step = struct { name: string, voltage: f64[V] }\n";

const WATCH: &str = "labcraft 1\n\nuse ui\n\n/// Reads a switch.\nfn watch() -> (on: bool) {\n    on = ui.get(stop_switch)\n}\n\npanel {\n    control stop_switch: switch\n}\n";

fn project() -> Arc<Memory> {
    let memory = Memory::new();
    for (name, text) in [
        ("double.lcf", DOUBLE),
        ("forever.lcf", FOREVER),
        ("spin.lcf", SPIN),
        ("takes.lcf", TAKES),
        ("Verdict.lct", VERDICT),
        ("Step.lct", STEP),
        ("watch.lcf", WATCH),
    ] {
        memory.put(format!("/p/{name}"), text);
    }
    Arc::new(memory)
}

fn session(memory: &Arc<Memory>) -> Session {
    let config = Config {
        root: PathBuf::from("/p"),
        lib_paths: Vec::new(),
        limits: Limits::default(),
        shipped: Shipped::default(),
        natives: Vec::new(),
        clock: Arc::new(Still),
        process: 7,
    };
    Session::new(Arc::clone(memory) as Arc<dyn Workspace>, config, false).unwrap()
}

fn ok(session: &mut Session, id: &str, params: Value) -> Value {
    session.execute(id, &params).unwrap_or_else(|error| panic!("{id}: {}", error.to_json()))
}

/// The codes of the diagnostics of a refused run, in order.
fn codes(report: &Value) -> Vec<String> {
    assert_eq!(report["status"], "refused", "{report}");
    report["check"]["diagnostics"].as_array().unwrap().iter().map(|diagnostic| diagnostic["code"].as_str().unwrap().to_owned()).collect()
}

fn takes(args: Value) -> Value {
    let memory = project();
    let mut session = session(&memory);
    ok(&mut session, "run.start", json!({ "function": "takes", "args": args, "trace": "none" }))
}

fn good_args() -> Value {
    json!({ "v": "0.22 V", "n": 7, "d": "100ms", "k": "Verdict.Fail", "xs": [1, 2.5], "s": { "name": "a", "voltage": "230 V" } })
}

#[test]
fn a_run_gives_its_report_and_its_trace_in_the_folder_of_runs() {
    let memory = project();
    let mut session = session(&memory);
    let report = ok(&mut session, "run.start", json!({ "function": "double", "args": { "x": 1.5 } }));
    assert_eq!(report["status"], "ok", "{report}");
    assert_eq!((report["run"].as_str(), report["function"].as_str()), (Some("r1"), Some("double")));
    // By hand: 1.5 × 2.
    assert_eq!(report["outputs"], json!([{ "name": "y", "type": "f64", "value": 3.0 }]));
    assert_eq!(report["trace"]["file"], ".labcraft/runs/double.trace.jsonl");
    let trace = String::from_utf8(memory.get("/p/.labcraft/runs/double.trace.jsonl").unwrap()).unwrap();
    assert!(trace.starts_with("{\"schema\":\"labcraft-trace/1\",\"run\":\"r1\",\"function\":\"double\""), "{trace}");
    assert!(trace.contains("\"file\":\"double.lcf\""), "the first line names the file the run read: {trace}");
    assert_eq!(trace.lines().count() as u64, report["trace"]["events"].as_u64().unwrap() + 1);
    assert_eq!(memory.get("/p/.labcraft/.gitignore").as_deref(), Some(&b"*\n"[..]), "made with `.labcraft`");
    let mut names = Vec::new();
    memory.read_dir(std::path::Path::new("/p/.labcraft/runs"), &mut |entry| names.push(entry.unwrap().name)).unwrap();
    names.sort();
    assert_eq!(names, [".7.lock", "double.trace.jsonl"], "no `.partial` left once the trace is renamed; the lock of the process");
    let lock = std::path::Path::new("/p/.labcraft/runs/.7.lock");
    assert!(memory.try_lock(lock).unwrap().is_none(), "held while the process runs");
    // The report is kept: `run.wait` and `run.status` give it again.
    assert_eq!(ok(&mut session, "run.wait", json!({})), report);
    assert_eq!(ok(&mut session, "run.status", json!({ "run": "r1" })), report);
}

#[test]
fn a_partial_trace_is_removed_when_this_session_or_a_process_that_no_longer_runs_left_it() {
    let memory = project();
    let mut session = session(&memory);
    let mine = session.runs().tag(7).session;
    let partial = |process: u32, session: u64| format!("/p/.labcraft/runs/double.trace.jsonl.{process}-{session}-r9.partial");
    for (process, number) in [(7, mine), (7, mine + 1000), (8, 1), (9, 1), (10, 1)] {
        memory.put(partial(process, number), "{}\n");
    }
    // 9 runs: it holds its lock. 10 ended: its lock file is there, and free. 8 left no lock file.
    let running = memory.try_lock(std::path::Path::new("/p/.labcraft/runs/.9.lock")).unwrap();
    memory.put("/p/.labcraft/runs/.10.lock", "");
    // 11 ended too, with a `.partial` of another function, which stays until that function runs: so does its lock.
    memory.put("/p/.labcraft/runs/other.trace.jsonl.11-1-r1.partial", "{}\n");
    memory.put("/p/.labcraft/runs/.11.lock", "");
    ok(&mut session, "run.start", json!({ "function": "double", "args": { "x": 1 } }));
    assert!(memory.get(partial(7, mine)).is_none(), "left by this session");
    assert!(memory.get(partial(7, mine + 1000)).is_some(), "another session of the process, which may be running");
    assert!(memory.get(partial(8, 1)).is_none(), "a process without a lock no longer runs");
    assert!(memory.get(partial(9, 1)).is_some(), "a process that holds its lock runs");
    assert!(memory.get(partial(10, 1)).is_none() && memory.get("/p/.labcraft/runs/.10.lock").is_none(), "a free lock: gone, with it");
    assert!(memory.get("/p/.labcraft/runs/.11.lock").is_some(), "kept while a `.partial` of 11 is left");
    assert!(memory.get("/p/.labcraft/runs/.9.lock").is_some());
    drop(running);
    assert!(memory.get("/p/.labcraft/.gitignore").is_none(), "`.labcraft` was there: nothing is added to it");
}

#[test]
fn arguments_convert_as_literals_of_the_language() {
    let report = takes(good_args());
    assert_eq!(report["status"], "ok", "{report}");
    // By hand: 0.22 V is 220 mV; 7 + 1.
    assert_eq!(report["outputs"][0]["value"], "220.0 mV");
    assert_eq!(report["outputs"][1]["value"], 8);
}

#[test]
fn each_argument_that_does_not_convert_has_its_code() {
    let with = |key: &str, value: Value| {
        let mut args = good_args();
        args[key] = value;
        codes(&takes(args))
    };
    assert_eq!(with("n", json!(10.0)), ["LC8104"], "an integer written with a fraction");
    assert_eq!(with("n", json!("x")), ["LC8104"]);
    assert_eq!(with("k", json!("Maybe")), ["LC8104"], "no such variant");
    assert_eq!(with("v", json!(220)), ["LC8105"], "no unit for a type with one");
    assert_eq!(with("v", json!("2 A")), ["LC8105"], "another dimension");
    assert_eq!(with("d", json!("100")), ["LC8105"], "a duration without a unit");
    assert_eq!(with("n", json!(300)), ["LC8106"], "beyond a u8");
    assert_eq!(with("v", json!("1e999 mV")), ["LC8106"], "beyond an f64");
    assert_eq!(with("n", json!("1000000000000000000000000000000000000000000")), ["LC8104"], "a u8 is a JSON number");
    assert_eq!(with("n", json!(1e42)), ["LC8104"], "a fraction, as JSON writes it");
    assert_eq!(with("xs", json!(["10"])), ["LC8104"], "a number without a unit is a JSON number (decision of the owner, 2026-11-10)");
    assert_eq!(with("xs", json!(["-nan"])), ["LC8104"], "no number of the language");
    let mut args = good_args();
    args["v"] = json!("1000000000000000000000000000000000000000000 mV");
    assert_eq!(takes(args)["outputs"][0]["value"], "1e42 mV", "digits past 128 bits are still a real");
    assert_eq!(with("s", json!({ "name": "a" })), ["LC8104"], "a field missing");
    let mut args = good_args();
    args["nope"] = json!(1);
    assert_eq!(codes(&takes(args)), ["LC8102"]);
    let mut args = good_args();
    args.as_object_mut().unwrap().remove("n");
    assert_eq!(codes(&takes(args)), ["LC8103"]);
    // Every problem is found, in one report.
    let mut args = good_args();
    args["n"] = json!(300);
    args["v"] = json!(1);
    assert_eq!(codes(&takes(args)), ["LC8105", "LC8106"]);
}

#[test]
fn an_unknown_function_and_an_unknown_element_are_diagnostics_not_command_errors() {
    let memory = project();
    let mut session = session(&memory);
    let report = ok(&mut session, "run.start", json!({ "function": "doubel" }));
    assert_eq!(codes(&report), ["LC8101"]);
    assert_eq!(report["check"]["diagnostics"][0]["fixes"][0]["title"], "did you mean `double`?");
    assert_eq!(report["check"]["diagnostics"][0]["file"], ".", "at the folder, with no position");
    let report = ok(&mut session, "run.start", json!({ "function": "watch", "set": [{ "element": "stop", "value": true, "at": "0 s" }] }));
    assert_eq!(codes(&report), ["LC8108"]);
    let report = ok(&mut session, "run.start", json!({ "function": "watch", "set": [{ "element": "stop_switch", "value": true, "at": "0 s" }] }));
    assert_eq!(report["outputs"][0]["value"], true, "set before the first node: {report}");
    assert_eq!(report["panel"][0]["by"], "set");
}

#[test]
fn what_the_command_line_calls_wrong_usage_is_a_command_error() {
    let memory = project();
    let mut session = session(&memory);
    let code = |session: &mut Session, params: Value| session.execute("run.start", &params).unwrap_err().code;
    assert_eq!(code(&mut session, json!({ "function": "a..b" })), "params.invalid");
    assert_eq!(code(&mut session, json!({ "function": "double", "set": [{ "element": "e", "value": 1, "at": "-1 s" }] })), "params.invalid");
    assert_eq!(code(&mut session, json!({ "function": "double", "args": { "next": 1 } })), "params.invalid", "a reserved word is no name");
    assert_eq!(code(&mut session, json!({ "function": "double", "timeout": "30" })), "params.invalid", "seconds are not assumed");
    assert_eq!(code(&mut session, json!({ "function": "double", "wait": "301 s" })), "params.invalid");
    assert_eq!(code(&mut session, json!({ "function": "double", "folder": "../x" })), "path.outside_project");
    assert_eq!(code(&mut session, json!({ "function": "double", "mode": "real_time", "schedule_seed": 1 })), "params.invalid");
    assert_eq!(code(&mut session, json!({ "function": "double", "mode": "real_time", "max_real_time": "1 s" })), "params.invalid");
}

#[test]
fn a_long_run_is_followed_and_aborted_while_the_session_answers() {
    let memory = project();
    let mut session = session(&memory);
    let limits = json!({ "function": "forever", "max_iterations": null, "max_nodes": null, "wait": "0 s", "trace": "none" });
    let running = ok(&mut session, "run.start", limits.clone());
    assert_eq!(running["status"], "running", "{running}");
    assert_eq!(running["run"], "r1");
    let busy = session.execute("run.start", &limits).unwrap_err();
    assert_eq!((busy.code, busy.run.as_deref()), ("run.busy", Some("r1")));
    assert_eq!(busy.to_json()["error"]["run"], "r1");
    // The session goes on answering while the run goes.
    assert_eq!(ok(&mut session, "library.check", json!({}))["schema"], "labcraft-check/1");
    // The run may not be in its loop yet when it is first asked (`"loops": []`): asked again, a bounded number of times.
    let mut status = ok(&mut session, "run.status", json!({}));
    for _ in 0..200 {
        if status["loops"].as_array().is_some_and(|loops| !loops.is_empty()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
        status = ok(&mut session, "run.status", json!({}));
    }
    assert_eq!(status["status"], "running");
    assert_eq!(status["loops"][0]["function"], "forever", "{status}");
    assert_eq!(status["loops"][0]["node"], "n");
    // Reached without the session, as an MCP server would.
    let runs = session.runs();
    let waited = runs.execute(session.workspace(), session.root(), "run.wait", &json!({ "wait": "10 ms" })).unwrap();
    assert_eq!(waited["status"], "running");
    let aborted = ok(&mut session, "run.abort", json!({ "run": "r1" }));
    assert_eq!(aborted["status"], "aborted", "{aborted}");
    assert_eq!(aborted["abort"]["by"], "request");
    assert_eq!(aborted["abort"]["loops"][0]["node"], "n");
    assert_eq!(ok(&mut session, "run.wait", json!({})), aborted, "an ended run is left as it is");
    assert_eq!(ok(&mut session, "run.abort", json!({})), aborted);
}

#[test]
fn the_last_16_reports_are_kept() {
    let memory = project();
    let mut session = session(&memory);
    assert_eq!(session.execute("run.status", &json!({})).unwrap_err().code, "run.unknown", "no run yet");
    for _ in 0..17 {
        ok(&mut session, "run.start", json!({ "function": "double", "args": { "x": 1 }, "trace": "none" }));
    }
    assert_eq!(session.execute("run.wait", &json!({ "run": "r1" })).unwrap_err().code, "run.unknown");
    assert_eq!(ok(&mut session, "run.wait", json!({ "run": "r2" }))["run"], "r2");
    assert_eq!(ok(&mut session, "run.status", json!({}))["run"], "r17");
}

#[test]
fn run_trace_reads_a_trace_in_pages_with_its_calls() {
    let memory = project();
    let mut session = session(&memory);
    let report = ok(&mut session, "run.start", json!({ "function": "spin" }));
    assert_eq!(report["status"], "ok");
    // By hand: `m` runs once in each of the 100 iterations.
    let page = ok(&mut session, "run.trace", json!({ "node": "m", "limit": 10 }));
    assert_eq!(page["header"]["function"], "spin");
    let events = page["events"].as_array().unwrap();
    assert_eq!(events.len(), 10);
    assert!(events.iter().all(|event| event["node"] == "m" && event["event"] == "ran"));
    assert_eq!(events[3]["iter"], json!([3]));
    assert_eq!(page["calls"][0]["event"], "call", "the call of frame 0, so that the values decode");
    assert_eq!(page["calls"][0]["frame"], 0);
    let next = page["next"].as_u64().unwrap();
    let second = ok(&mut session, "run.trace", json!({ "node": "m", "limit": 10, "from": next }));
    assert_eq!(second["events"][0]["iter"], json!([10]));
    let all = ok(&mut session, "run.trace", json!({ "function": "spin", "node": "m", "limit": 2000 }));
    assert_eq!(all["events"].as_array().unwrap().len(), 100);
    assert_eq!(all["next"], Value::Null);
    let ends = ok(&mut session, "run.trace", json!({ "event": "end" }));
    assert_eq!(ends["events"][0]["status"], "ok");
    // A run without a trace file.
    ok(&mut session, "run.start", json!({ "function": "spin", "trace": "none" }));
    assert_eq!(session.execute("run.trace", &json!({})).unwrap_err().code, "run.unknown");
    assert_eq!(ok(&mut session, "run.trace", json!({ "run": "r1", "event": "end" }))["events"][0]["status"], "ok", "still the last of `spin`");
    ok(&mut session, "run.start", json!({ "function": "spin" }));
    let replaced = session.execute("run.trace", &json!({ "run": "r1" })).unwrap_err();
    assert_eq!(replaced.code, "run.unknown", "a later run replaced it");
}

#[test]
fn an_inline_trace_is_in_the_report() {
    let memory = project();
    let mut session = session(&memory);
    let report = ok(&mut session, "run.start", json!({ "function": "double", "args": { "x": 2 }, "trace": "inline" }));
    assert_eq!(report["trace"]["header"]["schema"], "labcraft-trace/1");
    let events = report["trace"]["events"].as_array().unwrap();
    assert_eq!(events.first().unwrap()["event"], "call");
    assert_eq!(events.last().unwrap()["event"], "end");
    assert!(memory.get("/p/.labcraft/runs/double.trace.jsonl").is_none(), "nothing written");
}

/// Every code a run gives for its arguments, settings and simulation file is a row of cli.md 6.6, an error (decision of the
/// owner, 2026-11-10: the table there is the reference of these families, which stay out of the enumeration `Code`).
#[test]
fn the_codes_of_a_run_are_those_of_cli_md_6_6() {
    let cli = include_str!("../../../../docs/cli.md");
    let table: Vec<&str> = cli
        .split("### 6.6 Codes of `run`")
        .nth(1)
        .unwrap()
        .split("\n### ")
        .next()
        .unwrap()
        .lines()
        .filter(|line| line.starts_with("| `LC8"))
        .collect();
    let source = concat!(include_str!("prepare.rs"), include_str!("convert.rs"));
    let mut emitted: Vec<&str> = source.match_indices("\"LC8").filter_map(|(at, _)| source.get(at + 1..at + 7)).collect();
    emitted.sort_unstable();
    emitted.dedup();
    assert!(emitted.len() >= 9, "{emitted:?}");
    for code in emitted {
        assert!(table.iter().any(|row| row.starts_with(&format!("| `{code}` | error |"))), "{code} is not an error of cli.md 6.6");
    }
}

#[test]
fn a_huge_integer_is_out_of_range_and_wide_integers_are_strings_of_digits() {
    let memory = project();
    memory.put("/p/wide.lcf", "labcraft 1\n\nuse core\n\n/// An i64.\nfn wide(n: i64) -> (m: i64) {\n    m = core.add(n, 1)\n}\n");
    let mut session = session(&memory);
    let run = |session: &mut Session, n: Value| ok(session, "run.start", json!({ "function": "wide", "args": { "n": n }, "trace": "none" }));
    // By hand: 2^53 + 1 = 9007199254740993, given as digits since JSON numbers stop being exact past 2^53 - 1.
    assert_eq!(run(&mut session, json!("9007199254740993"))["outputs"][0]["value"], "9007199254740994");
    assert_eq!(codes(&run(&mut session, json!("1000000000000000000000000000000000000000000"))), ["LC8106"]);
    assert_eq!(codes(&run(&mut session, json!(9_007_199_254_740_993_u64))), ["LC8106"], "a JSON number past 2^53 - 1 is not exact");
}

#[test]
fn a_bad_argument_and_a_bad_setting_are_both_said() {
    let memory = project();
    memory.put("/p/both.lcf", "labcraft 1\n\nuse ui\n\n/// Reads a switch.\nfn both(x: f64) -> (on: bool) {\n    on = ui.get(stop_switch)\n}\n\npanel {\n    control stop_switch: switch\n}\n");
    let mut session = session(&memory);
    let report = ok(
        &mut session,
        "run.start",
        json!({ "function": "both", "args": { "x": "a" }, "set": [{ "element": "stop", "value": true, "at": "0 s" }] }),
    );
    // Sorted as cli.md 4.2 sorts them: the unknown element has no position, so it comes first in `both.lcf`.
    assert_eq!(codes(&report), ["LC8108", "LC8104"]);
    // By hand: `x` is on line 6 of `both.lcf`.
    assert_eq!(report["check"]["diagnostics"][1]["span"]["start"]["line"], 6, "at the name of the input: {report}");
}

#[test]
fn a_limit_given_as_null_is_removed() {
    let memory = project();
    let mut session = session(&memory);
    // `forever` would reach 10,000 iterations in a moment; without a limit it is still running.
    let running =
        ok(&mut session, "run.start", json!({ "function": "forever", "max_iterations": null, "max_nodes": null, "wait": "200 ms", "trace": "none" }));
    assert_eq!(running["status"], "running", "{running}");
    assert_eq!(ok(&mut session, "run.abort", json!({}))["status"], "aborted");
}

#[test]
fn a_page_of_a_trace_takes_an_event_larger_than_a_page() {
    let memory = project();
    let big = "x".repeat(4 << 20);
    let trace = format!(
        "{{\"schema\":\"labcraft-trace/1\",\"run\":\"r1\",\"function\":\"big\",\"mode\":\"deterministic\",\"files\":[]}}\n\
         {{\"seq\":1,\"t_ms\":0.0,\"event\":\"call\",\"frame\":0,\"function\":\"big\",\"in\":[{{\"name\":\"s\",\"type\":\"string\",\"value\":\"{big}\"}}]}}\n\
         {{\"seq\":2,\"t_ms\":0.0,\"event\":\"end\",\"status\":\"ok\"}}\n"
    );
    memory.put("/p/.labcraft/runs/big.trace.jsonl", trace);
    let mut session = session(&memory);
    let first = ok(&mut session, "run.trace", json!({ "function": "big", "limit": 1 }));
    assert_eq!(first["events"].as_array().map(Vec::len), Some(1), "the event of 4 MiB, its value cut by the bound");
    assert_eq!(first["next"], 2, "paging goes on");
    let second = ok(&mut session, "run.trace", json!({ "function": "big", "from": 2 }));
    assert_eq!(second["events"][0]["event"], "end");
}

#[test]
fn a_trace_is_never_written_or_read_through_a_link_out_of_the_project() {
    let memory = project();
    memory.put_folder("/outside");
    memory.put("/outside/double.trace.jsonl", "{\"schema\":\"labcraft-trace/1\"}\n");
    memory.put_link("/p/.labcraft/runs", "/outside");
    let mut session = session(&memory);
    let report = ok(&mut session, "run.start", json!({ "function": "double", "args": { "x": 1 } }));
    assert_eq!(report["status"], "ok");
    assert_eq!(report["trace"]["error"]["code"], "file.unwritable", "{report}");
    let mut names = Vec::new();
    memory.read_dir(std::path::Path::new("/outside"), &mut |entry| names.push(entry.unwrap().name)).unwrap();
    assert_eq!(names, ["double.trace.jsonl"], "nothing written there");
    assert_eq!(session.execute("run.trace", &json!({ "function": "double" })).unwrap_err().code, "path.outside_project");
    // A folder of its own under the link is not made there either.
    memory.put("/p/a/double.lcf", DOUBLE);
    let report = ok(&mut session, "run.start", json!({ "function": "double", "folder": "a", "args": { "x": 1 } }));
    assert_eq!(report["trace"]["error"]["code"], "file.unwritable");
    assert!(memory.metadata(std::path::Path::new("/outside/a")).is_err(), "no folder made out of the project");
    assert_eq!(session.execute("run.trace", &json!({ "function": "double", "folder": ".labcraft" })).unwrap_err().code, "path.outside_project");
}

#[test]
fn a_run_of_a_folder_has_its_trace_in_a_folder_of_its_own() {
    let memory = project();
    memory.put("/p/sub/double.lcf", DOUBLE);
    let mut session = session(&memory);
    let report = ok(&mut session, "run.start", json!({ "function": "double", "folder": "sub", "args": { "x": 1 } }));
    assert_eq!(report["trace"]["file"], ".labcraft/runs/sub/double.trace.jsonl");
    ok(&mut session, "run.start", json!({ "function": "double", "args": { "x": 1 } }));
    assert_eq!(ok(&mut session, "run.trace", json!({ "run": "r1", "event": "end" }))["events"][0]["status"], "ok", "not replaced by `./double`");
    let read = ok(&mut session, "run.trace", json!({ "function": "double", "folder": "sub", "event": "end" }));
    assert_eq!(read["file"], ".labcraft/runs/sub/double.trace.jsonl");
}

#[test]
fn two_sessions_of_one_process_never_share_a_partial_and_a_run_ends_with_its_session() {
    let memory = project();
    let long = json!({ "function": "forever", "max_iterations": null, "max_nodes": null, "wait": "0 s" });
    let mut first = session(&memory);
    let mut second = session(&memory);
    assert_eq!(ok(&mut first, "run.start", long.clone())["status"], "running");
    assert_eq!(ok(&mut second, "run.start", long)["status"], "running");
    // Each run makes its `.partial` on its own thread, as it starts.
    let mut partials = Vec::new();
    for _ in 0..200 {
        partials.clear();
        memory
            .read_dir(std::path::Path::new("/p/.labcraft/runs"), &mut |entry| {
                let name = entry.unwrap().name;
                if name.to_string_lossy().ends_with(".partial") {
                    partials.push(name);
                }
            })
            .unwrap();
        if partials.len() == 2 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(partials.len(), 2, "one each: {partials:?}");
    let runs = second.runs();
    let (ws, root) = (Arc::clone(&memory), PathBuf::from("/p"));
    drop(second);
    let ended = runs.execute(&*ws, &root, "run.wait", &json!({ "wait": "30 s" })).unwrap();
    assert_eq!((ended["status"].as_str(), ended["abort"]["by"].as_str()), (Some("aborted"), Some("interrupt")), "{ended}");
    assert!(ended["trace"].get("error").is_none(), "{ended}");
    assert_eq!(ok(&mut first, "run.abort", json!({}))["trace"].get("error"), None);
}

// The command line (cli.md 6; decisions of the owner, 2026-11-11) -----------------------------------------------------

fn texts(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs.iter().map(|(name, text)| ((*name).to_owned(), (*text).to_owned())).collect()
}

fn outside(path: &str, shown: &str) -> Option<crate::runs::Outside> {
    Some(crate::runs::Outside { path: PathBuf::from(path), shown: shown.to_owned() })
}

fn from_line(memory: &Arc<Memory>, params: Value, line: crate::runs::CommandLine) -> Value {
    let mut session = session(memory);
    session.start_run(&params, line).unwrap_or_else(|error| panic!("{}", error.to_json()))
}

/// The inputs of `takes` but `n`, as the command line writes them.
const GOOD_TEXTS: [(&str, &str); 5] =
    [("v", "0.22V"), ("d", "100ms"), ("k", "Fail"), ("xs", "[1, 2.5]"), ("s", r#"{"name": "a", "voltage": "230 V"}"#)];

#[test]
fn the_command_line_gives_values_as_text_with_the_freedoms_of_cli_6_3() {
    let memory = project();
    let mut args = texts(&GOOD_TEXTS);
    // A string is its text as it is, even one that JSON would read as a number.
    args.extend(texts(&[("n", "7"), ("label", "10")]));
    let line = crate::runs::CommandLine { args, ..Default::default() };
    let report = from_line(&memory, json!({ "function": "takes", "trace": "none" }), line);
    assert_eq!(report["status"], "ok", "{report}");
    // By hand: 0.22 V, the unit written without its space, is 220 mV; 7 + 1.
    assert_eq!(report["outputs"][0]["value"], "220.0 mV");
    assert_eq!(report["outputs"][1]["value"], 8);
}

#[test]
fn a_text_that_does_not_convert_has_the_code_of_its_problem() {
    for (name, text, code) in [
        ("n", "seven", "LC8104"),
        ("n", "300", "LC8106"),
        ("n", "7.0", "LC8104"),
        ("v", "220", "LC8105"),
        ("s", "{bad", "LC8104"),
        ("xs", "[1e999]", "LC8106"),
    ] {
        let memory = project();
        let mut args: Vec<(String, String)> = texts(&GOOD_TEXTS).into_iter().filter(|(given, _)| given != name).collect();
        if name != "n" {
            args.push(("n".to_owned(), "7".to_owned()));
        }
        args.push((name.to_owned(), text.to_owned()));
        let line = crate::runs::CommandLine { args, ..Default::default() };
        let report = from_line(&memory, json!({ "function": "takes", "trace": "none" }), line);
        assert_eq!(codes(&report), [code], "{name}={text}: {report}");
        assert_eq!(report["check"]["diagnostics"][0]["file"], "takes.lcf", "at the name of the input");
    }
}

#[test]
fn the_file_of_args_has_its_problems_at_its_keys_and_an_arg_replaces_its_value() {
    let memory = project();
    memory.put("/w/args.json", "{\n  \"v\": \"1 V\",\n  \"n\": 300,\n  \"zz\": 1\n}\n");
    let rest = || texts(&GOOD_TEXTS).into_iter().filter(|(name, _)| name != "v").collect::<Vec<_>>();
    let line = crate::runs::CommandLine { args: rest(), args_file: outside("/w/args.json", "w/args.json"), ..Default::default() };
    let report = from_line(&memory, json!({ "function": "takes", "trace": "none" }), line);
    let diagnostics = report["check"]["diagnostics"].as_array().unwrap();
    let found: Vec<(&str, &str, u64, u64)> = diagnostics
        .iter()
        .map(|d| {
            let start = &d["span"]["start"];
            (d["code"].as_str().unwrap(), d["file"].as_str().unwrap(), start["line"].as_u64().unwrap(), start["column"].as_u64().unwrap())
        })
        .collect();
    // By hand: `"n"` is on line 3 and `"zz"` on line 4, both at column 3, after two spaces; in the order of their places.
    assert_eq!(found, [("LC8106", "w/args.json", 3, 3), ("LC8102", "w/args.json", 4, 3)], "{report}");
    // `--arg n=7` replaces the 300 of the file; `zz` still names no input.
    let mut args = rest();
    args.push(("n".to_owned(), "7".to_owned()));
    let line = crate::runs::CommandLine { args, args_file: outside("/w/args.json", "w/args.json"), ..Default::default() };
    assert_eq!(codes(&from_line(&memory, json!({ "function": "takes", "trace": "none" }), line)), ["LC8102"]);
}

#[test]
fn a_file_of_args_that_is_not_an_object_of_inputs_is_refused_in_the_file() {
    for (text, code, line_at) in [("{\"n\": 1, \"n\": 2}", "LC8104", Some(1)), ("[1]", "LC8104", None), ("{\"n\": ", "LC8104", Some(1))] {
        let memory = project();
        memory.put("/w/args.json", text);
        let line = crate::runs::CommandLine { args_file: outside("/w/args.json", "args.json"), ..Default::default() };
        let report = from_line(&memory, json!({ "function": "double", "trace": "none" }), line);
        let first = &report["check"]["diagnostics"][0];
        assert_eq!((first["code"].as_str(), first["file"].as_str()), (Some(code), Some("args.json")), "{text}: {report}");
        assert_eq!(first["span"]["start"]["line"].as_u64(), line_at, "{text}");
    }
    let memory = project();
    let line =
        crate::runs::CommandLine { args_file: outside("/w/none.json", "none.json"), sim: outside("/w/none.toml", "none.toml"), ..Default::default() };
    let report = from_line(&memory, json!({ "function": "double", "trace": "none" }), line);
    let files: Vec<(&str, &str)> =
        report["check"]["diagnostics"].as_array().unwrap().iter().map(|d| (d["code"].as_str().unwrap(), d["file"].as_str().unwrap())).collect();
    // A path is data: files that do not exist give `LC0001`, each at itself, in the order of their names; `x` is not said
    // to have no argument, since the file of `--args` that did not read may have given it.
    assert_eq!(files, [("LC0001", "none.json"), ("LC0001", "none.toml")], "{report}");
}

#[test]
fn paths_are_shown_from_the_current_folder_of_the_command_line() {
    let memory = project();
    let current = || Some(PathBuf::from("/"));
    let line = crate::runs::CommandLine { current: current(), args: texts(&[("x", "1.5")]), ..Default::default() };
    let report = from_line(&memory, json!({ "function": "double" }), line);
    assert_eq!(report["trace"]["file"], "p/.labcraft/runs/double.trace.jsonl", "{report}");
    assert!(memory.get("/p/.labcraft/runs/double.trace.jsonl").is_some(), "under the root, as ever");
    let line = crate::runs::CommandLine { current: current(), ..Default::default() };
    let report = from_line(&memory, json!({ "function": "doubel", "trace": "none" }), line);
    assert_eq!(report["check"]["diagnostics"][0]["file"], "p", "{report}");
    let line = crate::runs::CommandLine { current: current(), ..Default::default() };
    let report = from_line(&memory, json!({ "function": "double", "trace": "none" }), line);
    assert_eq!(report["check"]["diagnostics"][0]["file"], "p/double.lcf", "{report}");
}

#[test]
fn a_trace_to_a_file_goes_there_and_the_folder_of_runs_is_left_alone() {
    let memory = project();
    memory.put_folder("/out");
    memory.put("/out/t.jsonl.3-1-r1.partial", "{}\n");
    let line = crate::runs::CommandLine { args: texts(&[("x", "2")]), trace: outside("/out/t.jsonl", "out/t.jsonl"), ..Default::default() };
    let report = from_line(&memory, json!({ "function": "double" }), line);
    assert_eq!(report["trace"]["file"], "out/t.jsonl", "{report}");
    let trace = String::from_utf8(memory.get("/out/t.jsonl").unwrap()).unwrap();
    assert!(trace.starts_with("{\"schema\":\"labcraft-trace/1\""), "{trace}");
    assert!(memory.get("/p/.labcraft").is_none() && memory.link_metadata(std::path::Path::new("/p/.labcraft")).is_err(), "nothing under the root");
    let mut names = Vec::new();
    memory.read_dir(std::path::Path::new("/out"), &mut |entry| names.push(entry.unwrap().name.to_string_lossy().into_owned())).unwrap();
    names.sort();
    // The `.partial` of process 3, which holds no lock, is gone; this process holds its lock beside the file.
    assert_eq!(names, [".7.lock", "t.jsonl"]);
    // A folder that does not exist: the run goes on, and the report says why there is no trace.
    let line = crate::runs::CommandLine { args: texts(&[("x", "2")]), trace: outside("/nowhere/t.jsonl", "nowhere/t.jsonl"), ..Default::default() };
    let report = from_line(&memory, json!({ "function": "double" }), line);
    assert_eq!((report["status"].as_str(), report["trace"]["error"]["code"].as_str()), (Some("ok"), Some("file.unwritable")), "{report}");
}

#[test]
fn a_setting_of_the_command_line_converts_as_an_argument_does() {
    let memory = project();
    let set = vec![("stop_switch".to_owned(), "true".to_owned(), "0s".to_owned())];
    let line = crate::runs::CommandLine { set, ..Default::default() };
    let report = from_line(&memory, json!({ "function": "watch", "trace": "none" }), line);
    // By hand: set at 0 s, before the first node, so the read sees it.
    assert_eq!(report["outputs"][0]["value"], true, "{report}");
    assert_eq!(report["panel"][0]["by"], "set", "{report}");
    let set = vec![("stop_switch".to_owned(), "maybe".to_owned(), "0s".to_owned())];
    let line = crate::runs::CommandLine { set, ..Default::default() };
    assert_eq!(codes(&from_line(&memory, json!({ "function": "watch", "trace": "none" }), line)), ["LC8104"]);
}

#[test]
fn a_warning_in_a_file_of_the_folder_reached_through_a_lib_path_copy_refuses_under_deny_warnings() {
    // `x` is found through `--lib-path /p/vendor`, so the run reaches `vendor/x/f.lcf` as a dependency; it is a file of
    // the folder all the same (architecture 10.1, cli.md 6.1), and its warning (`LC2020`, `core` imported twice) counts.
    let memory = Arc::new(Memory::new());
    memory.put("/p/main.lcf", "labcraft 1\n\nuse x\n\n/// Calls x.\nfn main() -> (y: f64) {\n    y = x.f()\n}\n");
    memory.put("/p/vendor/x/f.lcf", "labcraft 1\n\nuse core\nuse core\n\n/// One.\nfn f() -> (y: f64) {\n    y = core.add(1.0, 0.0)\n}\n");
    let run = |deny: bool| {
        let config = Config {
            root: PathBuf::from("/p"),
            lib_paths: vec![PathBuf::from("/p/vendor")],
            limits: Limits::default(),
            shipped: Shipped::default(),
            natives: Vec::new(),
            clock: Arc::new(Still),
            process: 7,
        };
        let mut session = Session::new(Arc::clone(&memory) as Arc<dyn Workspace>, config, false).unwrap();
        ok(&mut session, "run.start", json!({ "function": "main", "deny_warnings": deny, "trace": "none" }))
    };
    let plain = run(false);
    assert_eq!(plain["status"], "ok", "{plain}");
    let denied = run(true);
    assert_eq!(denied["status"], "refused", "{denied}");
    let codes: Vec<&str> = denied["check"]["diagnostics"].as_array().unwrap().iter().filter_map(|d| d["code"].as_str()).collect();
    assert!(codes.contains(&"LC2020"), "{denied}");
}

#[test]
fn a_number_that_no_f64_holds_is_out_of_range_in_text_and_in_the_file_of_args() {
    let memory = project();
    // cli.md 6.6 names `1e999` under `LC8106`.
    let line = crate::runs::CommandLine { args: texts(&[("x", "1e999")]), ..Default::default() };
    assert_eq!(codes(&from_line(&memory, json!({ "function": "double", "trace": "none" }), line)), ["LC8106"]);
    memory.put("/w/args.json", "{\"x\": 1e999}");
    let line = crate::runs::CommandLine { args_file: outside("/w/args.json", "args.json"), ..Default::default() };
    let report = from_line(&memory, json!({ "function": "double", "trace": "none" }), line);
    assert_eq!(codes(&report), ["LC8106"], "no `LC8103` after it: {report}");
    // By hand: `1e999` starts at byte 6, column 7.
    assert_eq!(report["check"]["diagnostics"][0]["span"]["start"]["column"], 7, "{report}");
}

#[test]
fn a_lock_with_no_partial_left_of_a_process_that_no_longer_runs_goes() {
    let memory = project();
    memory.put("/p/.labcraft/runs/.40.lock", "");
    let running = memory.try_lock(std::path::Path::new("/p/.labcraft/runs/.41.lock")).unwrap();
    let mut session = session(&memory);
    ok(&mut session, "run.start", json!({ "function": "double", "args": { "x": 1 } }));
    assert!(memory.get("/p/.labcraft/runs/.40.lock").is_none(), "free, with no `.partial`: an ended process left it");
    assert!(memory.get("/p/.labcraft/runs/.41.lock").is_some(), "held");
    drop(running);
}

#[test]
fn a_file_of_args_of_many_unknown_keys_lists_the_first_and_counts_them_all() {
    let memory = project();
    let keys: Vec<String> = (0..1500).map(|index| format!("\"k{index}\": 1")).collect();
    memory.put("/w/args.json", format!("{{{}, \"x\": 2}}", keys.join(", ")));
    let line = crate::runs::CommandLine { args_file: outside("/w/args.json", "args.json"), ..Default::default() };
    let report = from_line(&memory, json!({ "function": "double", "trace": "none" }), line);
    // By hand: 1,500 keys name no input; `--max-report` is 1,000 by default, so 1,000 are said and 500 counted.
    assert_eq!(report["check"]["diagnostics"].as_array().unwrap().len(), 1000, "{}", report["check"]["summary"]);
    assert_eq!(report["check"]["summary"]["errors"], 1500);
    assert_eq!(report["check"]["summary"]["truncated"], true);
}

#[test]
fn a_large_json_number_reads_and_one_that_no_integer_type_holds_is_out_of_range() {
    let memory = project();
    // `serde_json` writes 1e20 back as `1e+20`; by hand, 1e20 × 2.
    let mut session = session(&memory);
    let report = ok(&mut session, "run.start", json!({ "function": "double", "args": { "x": 1e20 }, "trace": "none" }));
    assert_eq!(report["outputs"][0]["value"], 2e20, "{report}");
    let mut args: Vec<(String, String)> = texts(&GOOD_TEXTS);
    args.push(("n".to_owned(), "99999999999999999999".to_owned()));
    let line = crate::runs::CommandLine { args, ..Default::default() };
    assert_eq!(codes(&from_line(&memory, json!({ "function": "takes", "trace": "none" }), line)), ["LC8106"]);
}

// Real time ----------------------------------------------------------------------------------------------------------------

/// Counts 10 ms waits until the switch is on.
const POLL: &str = "labcraft 1\n\nuse core\nuse time\nuse ui\n\n/// Counts until the switch is on.\nfn poll() -> (n: f64, err: error) {\n    n, err = loop carry n = 0.0, carry e = none {\n        e1 = time.wait(10 ms, err: e)\n        stop = ui.get(stop_switch, after: e1)\n        next n = core.add(n, 1.0)\n        next e = e1\n        until stop\n    }\n}\n\npanel {\n    control stop_switch: switch\n    control note: text_box\n}\n";

/// Waits 10 s.
const SLEEP: &str = "labcraft 1\n\nuse time\n\n/// Waits 10 s.\nfn sleep() -> (err: error) {\n    err = time.wait(10 s)\n}\n";

/// The real clock, for runs in real time.
struct Real(std::time::Instant);

impl crate::Clock for Real {
    fn real_ns(&self) -> u64 {
        u64::try_from(self.0.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }
    /// From 1970: no test here asserts the time of day.
    fn utc_ns(&self) -> i64 {
        0
    }
}

#[expect(clippy::disallowed_methods, reason = "a test of real time, natively")]
fn real_session() -> Session {
    let memory = Memory::new();
    memory.put("/p/poll.lcf", POLL);
    memory.put("/p/sleep.lcf", SLEEP);
    let config = Config {
        root: PathBuf::from("/p"),
        lib_paths: Vec::new(),
        limits: Limits::default(),
        shipped: Shipped::default(),
        natives: Vec::new(),
        clock: Arc::new(Real(std::time::Instant::now())),
        process: 7,
    };
    Session::new(Arc::new(memory) as Arc<dyn Workspace>, config, false).unwrap()
}

#[test]
fn run_set_element_sets_an_element_of_a_run_in_real_time() {
    let mut session = real_session();
    let running = ok(&mut session, "run.start", json!({ "function": "poll", "mode": "real_time", "wait": "0 s", "trace": "none" }));
    // commands.md 7: `mode` says it, and `clock` has no simulated time.
    assert_eq!((running["status"].as_str(), running["mode"].as_str()), (Some("running"), Some("real_time")), "{running}");
    assert!(running["clock"].get("simulated_ms").is_none(), "{running}");
    // A setting that `--set` would refuse is `params.invalid`, whose message gives the code (commands.md 6.4), and the run goes on.
    let unknown = session.execute("run.set_element", &json!({ "element": "stop", "value": true })).unwrap_err();
    assert_eq!((unknown.code, unknown.parameter.as_deref()), ("params.invalid", Some("element")));
    assert!(unknown.message.starts_with("LC8108: "), "{}", unknown.message);
    let wrong = session.execute("run.set_element", &json!({ "element": "stop_switch", "value": 1 })).unwrap_err();
    assert_eq!((wrong.code, wrong.parameter.as_deref()), ("params.invalid", Some("value")));
    assert!(wrong.message.starts_with("LC8104: "), "{}", wrong.message);
    // Its entry of `panel`, once the run took it.
    let entry = ok(&mut session, "run.set_element", json!({ "element": "stop_switch", "value": true }));
    assert_eq!(
        (entry["run"].as_str(), entry["element"].as_str(), &entry["value"], entry["by"].as_str(), &entry["applied"]),
        (Some("r1"), Some("stop_switch"), &json!(true), Some("run.set_element"), &json!(true)),
        "{entry}"
    );
    assert!(entry["at_ms"].as_f64().is_some_and(|at| at >= 0.0), "{entry}");
    let report = ok(&mut session, "run.wait", json!({ "wait": "10 s" }));
    assert_eq!((report["status"].as_str(), report["mode"].as_str()), (Some("ok"), Some("real_time")), "{report}");
    assert!(report["outputs"][0]["value"].as_f64().is_some_and(|n| n >= 1.0), "the loop reads the switch after a wait: {report}");
    let mut in_panel = entry.clone();
    in_panel.as_object_mut().unwrap().remove("run");
    assert_eq!(report["panel"], json!([in_panel]));
    // With no run going: `run.unknown`.
    let ended = session.execute("run.set_element", &json!({ "element": "stop_switch", "value": true })).unwrap_err();
    assert_eq!(ended.code, "run.unknown");
}

#[test]
fn run_set_element_is_not_enabled_on_a_deterministic_run() {
    let memory = project();
    let mut session = session(&memory);
    let limits = json!({ "function": "forever", "max_iterations": null, "max_nodes": null, "wait": "0 s", "trace": "none" });
    assert_eq!(ok(&mut session, "run.start", limits)["status"], "running");
    // commands.md 6.4: the time a call arrives would date the change; a deterministic run takes it in `set`.
    let error = session.execute("run.set_element", &json!({ "element": "stop_switch", "value": true })).unwrap_err();
    assert_eq!(error.code, "command.not_enabled");
    assert!(error.message.contains("`set`"), "{}", error.message);
    assert_eq!(ok(&mut session, "run.abort", json!({}))["status"], "aborted");
}

#[test]
#[expect(clippy::disallowed_methods, reason = "a test of real time, natively")]
fn an_abort_wakes_a_run_that_waits_in_real_time() {
    let mut session = real_session();
    let running = ok(&mut session, "run.start", json!({ "function": "sleep", "mode": "real_time", "wait": "0 s", "trace": "none" }));
    assert_eq!(running["status"], "running");
    let asked = std::time::Instant::now();
    let aborted = ok(&mut session, "run.abort", json!({}));
    // The run waits 10 s; the abort wakes it, and waits end at once (spec 8.2). Bounded, not timed: far below 10 s.
    assert!(asked.elapsed() < std::time::Duration::from_secs(5), "{:?}", asked.elapsed());
    assert_eq!((aborted["status"].as_str(), aborted["abort"]["by"].as_str()), (Some("aborted"), Some("request")), "{aborted}");
    assert!(aborted["abort"]["at_ms"].as_f64().is_some_and(|at| at < 5000.0), "{aborted}");
}

#[test]
fn what_only_deterministic_mode_has_is_refused_in_real_time_and_a_run_to_its_end_has_no_real_time() {
    let mut session = real_session();
    // Real time has no limit of iterations or nodes by default (cli.md 6.2): the run goes on, bounded by its timeout.
    let report = ok(&mut session, "run.start", json!({ "function": "poll", "mode": "real_time", "timeout": "50 ms", "trace": "none" }));
    assert_eq!((report["status"].as_str(), report["abort"]["by"].as_str()), (Some("aborted"), Some("timeout")), "{report}");
    assert!(report["clock"]["real_ms"].as_u64().is_some_and(|real| real >= 50), "a timeout of real time: {report}");
    let error = crate::run_to_end(&session, &json!({ "function": "poll", "mode": "real_time" })).unwrap_err();
    assert_eq!(error.code, "command.not_enabled");
}

#[test]
fn a_setting_that_does_not_fit_in_the_memory_of_the_run_is_refused_and_the_run_goes_on() {
    let mut session = real_session();
    let params = json!({ "function": "poll", "mode": "real_time", "max_memory": 1_048_576, "wait": "0 s", "trace": "none" });
    assert_eq!(ok(&mut session, "run.start", params)["status"], "running");
    // Decision of the owner, 2026-11-11: a text of 2 MiB, past the 1 MiB of the run, is `params.invalid` with `LC8005`.
    let big = session.execute("run.set_element", &json!({ "element": "note", "value": "x".repeat(2 << 20) })).unwrap_err();
    assert_eq!((big.code, big.parameter.as_deref()), ("params.invalid", Some("value")));
    assert!(big.message.starts_with("LC8005: "), "{}", big.message);
    // The run goes on, and takes the next setting.
    let entry = ok(&mut session, "run.set_element", json!({ "element": "stop_switch", "value": true }));
    assert_eq!(entry["applied"], json!(true), "{entry}");
    let report = ok(&mut session, "run.wait", json!({ "wait": "10 s" }));
    assert_eq!(report["status"], "ok", "{report}");
    assert_eq!(report["panel"].as_array().map(Vec::len), Some(1), "{report}");
}

#[test]
fn a_run_in_real_time_on_a_clock_that_never_moves_is_not_enabled() {
    let memory = project();
    let mut session = session(&memory);
    // `Still` never moves, so such a run would never end, its timeout included (decision of the owner, 2026-11-11).
    let error = session.execute("run.start", &json!({ "function": "forever", "mode": "real_time", "timeout": "50 ms" })).unwrap_err();
    assert_eq!((error.code, error.parameter.as_deref()), ("command.not_enabled", Some("mode")));
}

/// The memory, slow to name the paths under `/slow`: a network share, where judging a run takes time.
struct Slow(Memory);

impl crate::Workspace for Slow {
    fn canonicalize(&self, path: &std::path::Path) -> std::io::Result<PathBuf> {
        if path.starts_with("/slow") {
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
        self.0.canonicalize(path)
    }
    fn absolute(&self, path: &std::path::Path) -> std::io::Result<PathBuf> {
        self.0.absolute(path)
    }
    fn current(&self) -> Option<PathBuf> {
        self.0.current()
    }
    fn metadata(&self, path: &std::path::Path) -> std::io::Result<crate::workspace::Metadata> {
        self.0.metadata(path)
    }
    fn link_metadata(&self, path: &std::path::Path) -> std::io::Result<crate::workspace::Metadata> {
        self.0.link_metadata(path)
    }
    fn read_dir(&self, path: &std::path::Path, each: &mut dyn FnMut(std::io::Result<crate::workspace::Entry>)) -> std::io::Result<()> {
        self.0.read_dir(path, each)
    }
    fn read(&self, path: &std::path::Path, max: u64) -> std::io::Result<crate::workspace::Read> {
        self.0.read(path, max)
    }
    fn write(&self, path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
        self.0.write(path, bytes)
    }
    fn create_folders(&self, path: &std::path::Path) -> std::io::Result<()> {
        self.0.create_folders(path)
    }
    fn rename(&self, from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
        self.0.rename(from, to)
    }
    fn remove_file(&self, path: &std::path::Path) -> std::io::Result<()> {
        self.0.remove_file(path)
    }
}

#[test]
fn an_abort_of_a_run_refused_as_it_is_judged_ends() {
    let memory = Memory::new();
    memory.put("/p/double.lcf", DOUBLE);
    let ws: Arc<dyn Workspace> = Arc::new(Slow(memory));
    let config = Config {
        root: PathBuf::from("/p"),
        lib_paths: Vec::new(),
        limits: Limits::default(),
        shipped: Shipped::default(),
        natives: Vec::new(),
        clock: Arc::new(Still),
        process: 7,
    };
    let mut session = Session::new(Arc::clone(&ws), config, false).unwrap();
    let runs = session.runs();
    let (done, answered) = std::sync::mpsc::channel();
    let aborting = {
        let ws = Arc::clone(&ws);
        std::thread::spawn(move || {
            // While `run.start` judges its `folder`, the run is going, and this abort finds it.
            std::thread::sleep(std::time::Duration::from_millis(100));
            let _ = done.send(runs.execute(&*ws, std::path::Path::new("/p"), "run.abort", &json!({})));
        })
    };
    let refused = session.execute("run.start", &json!({ "function": "double", "folder": "/slow/elsewhere", "args": { "x": 1 } })).unwrap_err();
    assert!(refused.code.starts_with("path."), "{refused:?}");
    // Before the fix, the abort waited for ever on a run that had gone without a report.
    let abort = answered.recv_timeout(std::time::Duration::from_secs(10)).expect("the abort ends");
    let abort = abort.unwrap_err();
    // Not "has run nothing yet": the abort found the run, so this checks the fix even on a slow machine.
    assert_eq!(abort.code, "run.unknown");
    assert!(abort.message.contains("refused before it ran"), "the abort came before the run existed: {}", abort.message);
    aborting.join().unwrap();
}

#[test]
fn args_and_set_hold_at_most_the_bytes_of_a_file_of_args() {
    let memory = project();
    let config = Config {
        root: PathBuf::from("/p"),
        lib_paths: Vec::new(),
        limits: {
            let mut limits = Limits::default();
            limits.max_file_size = 1024;
            limits
        },
        shipped: Shipped::default(),
        natives: Vec::new(),
        clock: Arc::new(Still),
        process: 7,
    };
    let mut session = Session::new(Arc::clone(&memory) as Arc<dyn Workspace>, config, false).unwrap();
    // `{"x":[0,…,0]}` of n elements: `{"x":[` 6 bytes, n digits, n - 1 commas, `]}` 2, so 2n + 7 bytes. With a bound of
    // 1,024: 508 elements (1,023 bytes) pass to the run, which refuses an array for `x`; 509 (1,025 bytes) do not.
    let args = |count: usize| json!({ "function": "double", "args": { "x": vec![0; count] }, "trace": "none" });
    assert_eq!(serde_json::to_string(&args(508)["args"]).unwrap().len(), 1023);
    let refused = session.execute("run.start", &args(509)).unwrap_err();
    assert_eq!((refused.code, refused.parameter.as_deref()), ("params.invalid", Some("args")), "{}", refused.message);
    assert!(refused.message.contains("more than 1024 bytes"), "{}", refused.message);
    let passed = ok(&mut session, "run.start", args(508));
    assert_eq!(passed["status"], "refused", "judged against the input, not refused for its size: {passed}");
    let set = json!({ "function": "double", "args": { "x": 1 }, "set": [{ "element": "e", "value": "v".repeat(1100), "at": "0 s" }] });
    let refused = session.execute("run.start", &set).unwrap_err();
    assert_eq!((refused.code, refused.parameter.as_deref()), ("params.invalid", Some("set")), "{}", refused.message);
}
