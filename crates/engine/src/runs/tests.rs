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
    assert_eq!(names, ["double.trace.jsonl"], "no `.partial` left once the trace is renamed");
    // The report is kept: `run.wait` and `run.status` give it again.
    assert_eq!(ok(&mut session, "run.wait", json!({})), report);
    assert_eq!(ok(&mut session, "run.status", json!({ "run": "r1" })), report);
}

#[test]
fn a_partial_trace_that_this_process_left_is_removed_and_those_of_others_stay() {
    let memory = project();
    let mut session = session(&memory);
    let mine = session.runs().tag(7).session;
    let partial = |process: u32, session: u64| format!("/p/.labcraft/runs/double.trace.jsonl.{process}-{session}-r9.partial");
    for (process, number) in [(7, mine), (7, mine + 1000), (8, mine)] {
        memory.put(partial(process, number), "{}\n");
    }
    ok(&mut session, "run.start", json!({ "function": "double", "args": { "x": 1 } }));
    assert!(memory.get(partial(7, mine)).is_none(), "left by this session");
    assert!(memory.get(partial(7, mine + 1000)).is_some(), "another session of the process, which may be running");
    assert!(memory.get(partial(8, mine)).is_some(), "another process: step 7");
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
    assert_eq!(code(&mut session, json!({ "function": "double", "mode": "real_time" })), "command.not_enabled", "step 8");
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
    let status = ok(&mut session, "run.status", json!({}));
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
    assert_eq!(codes(&report), ["LC8104", "LC8108"]);
    // By hand: `x` is on line 6 of `both.lcf`.
    assert_eq!(report["check"]["diagnostics"][0]["span"]["start"]["line"], 6, "at the name of the input: {report}");
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
        memory.read_dir(std::path::Path::new("/p/.labcraft/runs"), &mut |entry| partials.push(entry.unwrap().name)).unwrap();
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
