//! `labcraft-cli run` (`docs/cli.md`, section 6), the program itself: every case of the corpus of runs gives through the
//! command line the report that `run.start` gives, the exit codes of 6.1, the human form of 6.7, wrong usage, `--trace`,
//! the `.partial` of a process that no longer runs, and Ctrl-C. Expected values are worked out by hand from the
//! documents, `examples/` and `libs/visa/sim/`, or are the report of `engine` itself for the same parameters, the reference
//! that the corpus checks by hand.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;

use labcraft_cli::disk::Disk;
use labcraft_engine::{Config, Session};
use labcraft_types::Limits;
use serde_json::{Value, json};

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").canonicalize().unwrap()
}

fn cli_in(folder: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_labcraft-cli")).current_dir(folder).args(args).output().expect("the program starts")
}

fn cli(args: &[&str]) -> Output {
    cli_in(&repository(), args)
}

fn report_of(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output.stdout)))
}

/// The value of the output `name` of a report.
fn output(report: &Value, name: &str) -> Value {
    report["outputs"].as_array().unwrap().iter().find(|output| output["name"] == name).unwrap()["value"].clone()
}

/// A folder of its own for a test, empty.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Scratch {
        let path = std::env::temp_dir().join(format!("labcraft-run-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Scratch(path.canonicalize().unwrap())
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The report of `run.start` for the same parameters, from a session of `engine` on the repository.
/// The real clock, for a case in real time.
struct Real(std::time::Instant);

impl labcraft_engine::Clock for Real {
    fn real_ns(&self) -> u64 {
        u64::try_from(self.0.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }
    /// From 1970: no test here asserts the time of day.
    fn utc_ns(&self) -> i64 {
        0
    }
}

fn engine_report(params: &Value) -> Value {
    let real_time = params["mode"] == "real_time";
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(labcraft_format::STACK_SIZE)
            .spawn_scoped(scope, || {
                let config = Config {
                    root: repository(),
                    lib_paths: Vec::new(),
                    limits: Limits::default(),
                    shipped: labcraft_cli::shipped(),
                    natives: labcraft_libs::natives(),
                    clock: if real_time {
                        #[expect(clippy::disallowed_methods, reason = "a test of real time, natively")]
                        let now = std::time::Instant::now();
                        Arc::new(Real(now))
                    } else {
                        Arc::new(labcraft_engine::Still)
                    },
                    process: 0,
                };
                let mut session = Session::new(Arc::new(Disk), config, false).unwrap();
                // A run in real time needs threads: through the command, which waits for its end.
                if real_time { session.execute("run.start", params).unwrap() } else { labcraft_engine::run_to_end(&session, params).unwrap() }
            })
            .unwrap()
            .join()
            .unwrap()
    })
}

/// The text of a value of `set` on the command line: a string without its quotes, anything else as JSON.
fn text_of(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// The options of `labcraft-cli run` that give the parameters of a `run.json` (commands.md 6.4); the arguments go in a
/// file of `--args`, which takes the JSON form exactly, as `args` do.
fn options_of(params: &Value, scratch: &Scratch) -> Vec<String> {
    let mut options = vec!["run".to_owned(), "--json".to_owned(), "--no-trace".to_owned()];
    let object = params.as_object().unwrap();
    for (key, value) in object {
        match key.as_str() {
            "function" | "folder" | "trace" => {}
            "sim" => options.extend(["--sim".to_owned(), text_of(value)]),
            "args" => {
                let file = scratch.0.join("args.json");
                fs::write(&file, value.to_string()).unwrap();
                options.extend(["--args".to_owned(), file.to_string_lossy().into_owned()]);
            }
            "set" => {
                for setting in value.as_array().unwrap() {
                    let text = format!("{}={}@{}", setting["element"].as_str().unwrap(), text_of(&setting["value"]), setting["at"].as_str().unwrap());
                    options.extend(["--set".to_owned(), text]);
                }
            }
            "deny_warnings" if value == &json!(true) => options.push("--deny-warnings".to_owned()),
            "mode" if value == "real_time" => options.push("--real-time".to_owned()),
            "mode" => {}
            _ => options.extend([format!("--{}", key.replace('_', "-")), if value.is_null() { "none".to_owned() } else { text_of(value) }]),
        }
    }
    options.push(object["folder"].as_str().unwrap_or(".").to_owned());
    options.push(object["function"].as_str().unwrap().to_owned());
    options
}

/// What differs between two runs of one program: the real clock, and in real time the times on it.
fn without_real_time(mut report: Value) -> Value {
    if let Some(clock) = report.get_mut("clock").and_then(Value::as_object_mut) {
        clock.remove("real_ms");
    }
    if report["mode"] == "real_time" {
        for instrument in report["instruments"].as_array_mut().into_iter().flatten() {
            for command in instrument["commands"].as_array_mut().into_iter().flatten() {
                command.as_object_mut().unwrap().remove("t_ms");
            }
        }
        if let Some(abort) = report.get_mut("abort").and_then(Value::as_object_mut) {
            abort.remove("at_ms");
        }
    }
    report
}

fn exit_of(status: &str) -> i32 {
    match status {
        "ok" => 0,
        "aborted" => 3,
        "fault" => 4,
        _ => 1,
    }
}

#[test]
fn every_case_of_the_corpus_gives_through_the_command_line_the_report_of_run_start() {
    let corpus = repository().join("tests/acceptance/runs");
    let mut cases: Vec<PathBuf> =
        fs::read_dir(&corpus).unwrap().map(|entry| entry.unwrap().path()).filter(|path| path.join("run.json").is_file()).collect();
    cases.sort();
    assert!(cases.len() >= 15, "the corpus is there: {cases:?}");
    for case in cases {
        let name = case.file_name().unwrap().to_string_lossy().into_owned();
        let mut params: Value = serde_json::from_slice(&fs::read(case.join("run.json")).unwrap()).unwrap();
        params.as_object_mut().unwrap().insert("trace".to_owned(), json!("none"));
        let scratch = Scratch::new(&name);
        let options = options_of(&params, &scratch);
        let options: Vec<&str> = options.iter().map(String::as_str).collect();
        let output = cli(&options);
        let report = report_of(&output);
        let status = report["status"].as_str().unwrap().to_owned();
        assert_eq!(output.status.code(), Some(exit_of(&status)), "{name}: {report}");
        let expected = engine_report(&params);
        let (mut got, mut want) = (without_real_time(report), without_real_time(expected));
        // A problem with an argument is at its key in the file of `--args`, not at the signature (cli.md 6.3).
        for report in [&mut got, &mut want] {
            for diagnostic in report["check"]["diagnostics"].as_array_mut().unwrap() {
                if diagnostic["code"].as_str().is_some_and(|code| code.starts_with("LC810")) {
                    let object = diagnostic.as_object_mut().unwrap();
                    object.remove("file");
                    object.remove("span");
                }
            }
        }
        assert_eq!(got, want, "{name}: {options:?}");
    }
}

#[test]
fn a_run_reads_as_cli_6_7_shows_it() {
    let output = cli(&["run", "--no-trace", "--sim", "libs/visa/sim/dmm-230V.toml", "--arg", "resource=SIM::DMM0", "examples", "check_voltage"]);
    assert_eq!(output.status.code(), Some(0), "{}", String::from_utf8_lossy(&output.stderr));
    let text = String::from_utf8(output.stdout).unwrap();
    let mut lines = text.lines();
    // By hand: the real time varies, the rest is that of cli.md 6.7 (`dmm-230V.toml` answers 230.071217).
    let first = lines.next().unwrap();
    assert!(first.starts_with("check_voltage: ok, 0 ms simulated, ") && first.ends_with(" ms real"), "{first}");
    assert_eq!(
        lines.collect::<Vec<_>>(),
        ["  verdict = Verdict.Pass", "  voltage = 230.071217 V", "  err     = none", "resources: 1 opened, 0 open"]
    );
}

/// #138: `visa` found by `--lib-path` (rule 3 of cli.md 3) closes its session after an error as the shipped one does,
/// since its manifest marks `close` as `always` (spec 7.2). By hand: `dmm-timeout.toml` never answers, so the run ends in
/// `io.timeout` (exit 1, cli.md 6.7), and the session opened is closed.
#[test]
fn always_holds_for_a_library_found_by_a_lib_path() {
    for lib_path in [&[][..], &["--lib-path", "libs"][..]] {
        let mut args = vec!["run", "--no-trace"];
        args.extend_from_slice(lib_path);
        args.extend_from_slice(&["--sim", "libs/visa/sim/dmm-timeout.toml", "--arg", "resource=SIM::DMM0", "examples", "check_voltage"]);
        let output = cli(&args);
        assert_eq!(output.status.code(), Some(1), "{lib_path:?}: {}", String::from_utf8_lossy(&output.stderr));
        let text = String::from_utf8(output.stdout).unwrap();
        assert_eq!(text.lines().last(), Some("resources: 1 opened, 0 open"), "{lib_path:?}: {text}");
    }
}

#[test]
fn a_refused_run_gives_its_diagnostics_then_refused_and_exit_1() {
    let output = cli(&["run", "--no-trace", "--arg", "resourc=SIM::DMM0", "examples", "check_voltage"]);
    assert_eq!(output.status.code(), Some(1));
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.starts_with("examples/check_voltage.lcf:"), "files shown from the current folder, as `check` shows them: {text}");
    assert!(text.contains("error[LC8102]") && text.contains("error[LC8103]"), "{text}");
    assert!(text.ends_with("check_voltage: refused\n"), "{text}");
    // A folder that does not exist is data: refused, with `LC0001`, not wrong usage.
    let output = cli(&["run", "--json", "no-such-folder", "f"]);
    assert_eq!(output.status.code(), Some(1));
    let report = report_of(&output);
    assert_eq!((report["status"].as_str(), report["check"]["diagnostics"][0]["code"].as_str()), (Some("refused"), Some("LC0001")), "{report}");
    assert_eq!(report["check"]["diagnostics"][0]["file"], "no-such-folder");
}

#[test]
fn wrong_usage_is_exit_2_and_runs_nothing() {
    for args in [
        &["run", "examples"][..],
        &["run", "examples", "check_voltage", "--timeout", "30"],
        &["run", "examples", "check_voltage", "--real-time", "--max-real-time", "1s"],
        &["run", "examples", "check_voltage", "--real-time", "--schedule-seed", "3"],
        &["run", "examples", "check_voltage", "--trace", "examples/check_voltage.lcf"],
        &["run", "examples", "check_voltage", "--arg", "x=1", "--arg", "x=2"],
        &["run", "examples/check_voltage.lcf", "check_voltage"],
    ] {
        let output = cli(args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        assert!(String::from_utf8_lossy(&output.stderr).starts_with("error: "), "{args:?}");
    }
}

/// A project of one function that counts for ever, and one that doubles.
fn project(scratch: &Scratch) -> PathBuf {
    let folder = scratch.0.join("p");
    fs::create_dir_all(&folder).unwrap();
    fs::write(
        folder.join("double.lcf"),
        "labcraft 1\n\nuse core\n\n/// Doubles a number.\nfn double(x: f64) -> (y: f64) {\n    y = core.mul(x, 2)\n}\n",
    )
    .unwrap();
    fs::write(
        folder.join("forever.lcf"),
        "labcraft 1\n\nuse core\n\n/// Counts for ever.\nfn forever() -> (n: f64) {\n    n = loop carry i = 0.0 {\n        m = core.add(i, 1.0)\n        done = core.ge(0.0, m)\n        next i = m\n        until done\n    }\n}\n",
    )
    .unwrap();
    folder
}

#[test]
fn a_trace_goes_under_the_folder_or_to_the_file_given() {
    let scratch = Scratch::new("trace");
    let folder = project(&scratch);
    let output = cli_in(&scratch.0, &["run", "--json", "--arg", "x=1.5", "p", "double"]);
    let report = report_of(&output);
    // By hand: 1.5 × 2; the trace under `FOLDER/.labcraft/runs/`, shown from the current folder.
    assert_eq!(report["outputs"][0]["value"], 3.0, "{report}");
    assert_eq!(report["trace"]["file"], "p/.labcraft/runs/double.trace.jsonl");
    let runs = folder.join(".labcraft/runs");
    let names: Vec<String> = fs::read_dir(&runs).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(names, ["double.trace.jsonl"], "no `.partial` and no lock left once the process ends");
    let output = cli_in(&scratch.0, &["run", "--json", "--arg", "x=2", "--trace", "t.jsonl", "p", "double"]);
    assert_eq!(report_of(&output)["trace"]["file"], "t.jsonl");
    assert!(fs::read_to_string(scratch.0.join("t.jsonl")).unwrap().starts_with("{\"schema\":\"labcraft-trace/1\""));
    // The last trace of the function stays that of the first run, x = 1.5.
    assert!(fs::read_to_string(runs.join("double.trace.jsonl")).unwrap().contains("1.5"));
    // A trace replaces a trace, never another file.
    let output = cli_in(&scratch.0, &["run", "--arg", "x=2", "--trace", "t.jsonl", "p", "double"]);
    assert_eq!(output.status.code(), Some(0));
    fs::write(scratch.0.join("notes.txt"), "mine").unwrap();
    let output = cli_in(&scratch.0, &["run", "--arg", "x=2", "--trace", "notes.txt", "p", "double"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(fs::read_to_string(scratch.0.join("notes.txt")).unwrap(), "mine");
}

#[test]
fn the_partial_trace_of_a_process_that_no_longer_runs_is_removed() {
    let scratch = Scratch::new("partial");
    let folder = project(&scratch);
    let runs = folder.join(".labcraft/runs");
    fs::create_dir_all(&runs).unwrap();
    // Process 4000000001 left a `.partial` and no lock; the test holds the lock of 4000000002, which still runs.
    let dead = runs.join("double.trace.jsonl.4000000001-1-r1.partial");
    let alive = runs.join("double.trace.jsonl.4000000002-1-r1.partial");
    fs::write(&dead, "{}\n").unwrap();
    fs::write(&alive, "{}\n").unwrap();
    let lock = fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(runs.join(".4000000002.lock")).unwrap();
    lock.try_lock().unwrap();
    let output = cli_in(&folder, &["run", "--arg", "x=1", ".", "double"]);
    assert_eq!(output.status.code(), Some(0), "{}", String::from_utf8_lossy(&output.stdout));
    assert!(!dead.exists(), "left by a process that no longer runs");
    assert!(alive.exists(), "the process holds its lock");
    drop(lock);
}

#[cfg(unix)]
#[test]
fn ctrl_c_aborts_the_run_and_the_report_is_still_written() {
    let scratch = Scratch::new("interrupt");
    project(&scratch);
    let child = Command::new(env!("CARGO_BIN_EXE_labcraft-cli"))
        .current_dir(&scratch.0)
        .args(["run", "--json", "--no-trace", "--max-iterations", "none", "--max-nodes", "none", "p", "forever"])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1500));
    let sent = Command::new("kill").args(["-INT", &child.id().to_string()]).status().unwrap();
    assert!(sent.success());
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(3), "{}", String::from_utf8_lossy(&output.stdout));
    let report = report_of(&output);
    assert_eq!((report["status"].as_str(), report["abort"]["by"].as_str()), (Some("aborted"), Some("interrupt")), "{report}");
}

/// In real time, Ctrl-C wakes a run that waits: the wait of 60 s ends at once (spec 8.2).
#[cfg(unix)]
#[test]
#[expect(clippy::disallowed_methods, reason = "a test of real time, natively")]
fn ctrl_c_wakes_a_run_in_real_time_that_waits() {
    let scratch = Scratch::new("interrupt-real-time");
    let folder = scratch.0.join("q");
    fs::create_dir_all(&folder).unwrap();
    fs::write(
        folder.join("nap.lcf"),
        "labcraft 1

use time

/// Waits a minute.
fn nap() -> (err: error) {
    err = time.wait(60 s)
}
",
    )
    .unwrap();
    let started = std::time::Instant::now();
    let child = Command::new(env!("CARGO_BIN_EXE_labcraft-cli"))
        .current_dir(&scratch.0)
        .args(["run", "--json", "--no-trace", "--real-time", "q", "nap"])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1500));
    let sent = Command::new("kill").args(["-INT", &child.id().to_string()]).status().unwrap();
    assert!(sent.success());
    let output = child.wait_with_output().unwrap();
    // Bounded, not timed: far below the minute of the wait.
    assert!(started.elapsed() < std::time::Duration::from_secs(30), "{:?}", started.elapsed());
    assert_eq!(output.status.code(), Some(3), "{}", String::from_utf8_lossy(&output.stdout));
    let report = report_of(&output);
    assert_eq!((report["mode"].as_str(), report["abort"]["by"].as_str()), (Some("real_time"), Some("interrupt")), "{report}");
}

/// The human form of the other statuses (cli.md 6.7), on cases of the corpus whose reports are worked out by hand.
#[test]
fn an_error_a_fault_and_an_abort_read_as_cli_6_7_says() {
    let text = |args: &[&str]| -> (Option<i32>, String) {
        let output = cli(args);
        (output.status.code(), String::from_utf8(output.stdout).unwrap())
    };
    // No simulation file: `visa.open` returns an error (cli.md 6.2) after the timeout of the open, 2 s (libs/visa/
    // simulation.md 6, as `tests/acceptance/runs/s1-no-instrument` works out); the run ends with `error`, exit 1.
    let (code, error) = text(&["run", "--no-trace", "--arg", "resource=SIM::DMM0", "examples", "check_voltage"]);
    assert_eq!(code, Some(1), "{error}");
    assert!(error.starts_with("check_voltage: error, 2000 ms simulated, "), "{error}");
    assert!(error.lines().any(|line| line.starts_with("error: ") && line.contains(": ")), "its category and message: {error}");
    assert!(error.lines().any(|line| line.starts_with("  from `")), "where it came from: {error}");
    // `LC8002` after 10 nodes: the first line of the diagnostic, exit 4.
    let (code, fault) = text(&["run", "--no-trace", "--max-nodes", "10", "tests/acceptance/runs/fault-LC8002-nodes", "spin"]);
    assert_eq!(code, Some(4), "{fault}");
    assert!(
        fault.lines().any(|line| line.starts_with("tests/acceptance/runs/fault-LC8002-nodes/spin.lcf:") && line.contains(": error[LC8002]: fault: "))
    );
    // Scenario 3, point 5: aborted by the timeout at 350 ms, both loops at their fourth iteration, exit 3.
    let (code, aborted) = text(&["run", "--no-trace", "--sim", "<labcraft>/visa/sim/two-dmm.toml", "--timeout", "350ms", "examples", "two_benches"]);
    assert_eq!(code, Some(3), "{aborted}");
    let lines: Vec<&str> = aborted.lines().collect();
    assert_eq!(
        lines.get(1..5),
        Some(
            &[
                "aborted by --timeout at 350 ms",
                "  in loop `n_a` of `two_benches`, iteration 4",
                "  in loop `n_b` of `two_benches`, iteration 4",
                "warning: the instruments are in an unknown state",
            ][..]
        ),
        "{aborted}"
    );
}

#[test]
fn a_folder_that_is_not_under_the_current_one_is_shown_as_written() {
    let output = cli_in(&repository().join("docs"), &["run", "--json", "--no-trace", "../tests/acceptance/runs/refused-a-type-error", "power"]);
    let report = report_of(&output);
    let file = report["check"]["diagnostics"][0]["file"].as_str().unwrap();
    assert!(file.starts_with("../tests/acceptance/runs/refused-a-type-error/"), "as `check` shows it: {report}");
}

#[test]
fn the_problems_of_the_file_of_args_are_at_their_keys_on_the_disk() {
    let scratch = Scratch::new("args");
    project(&scratch);
    fs::write(scratch.0.join("args.json"), "{\n  \"x\": \"two\",\n  \"y\": 1\n}\n").unwrap();
    let output = cli_in(&scratch.0, &["run", "--json", "--no-trace", "--args", "args.json", "p", "double"]);
    let report = report_of(&output);
    let found: Vec<(String, u64)> = report["check"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| (format!("{} {}", d["code"].as_str().unwrap(), d["file"].as_str().unwrap()), d["span"]["start"]["line"].as_u64().unwrap()))
        .collect();
    // By hand: `"x"` on line 2 does not read as an `f64`; `"y"` on line 3 names no input.
    assert_eq!(found, [("LC8104 args.json".to_owned(), 2), ("LC8102 args.json".to_owned(), 3)], "{report}");
}

#[test]
fn a_free_lock_on_the_disk_goes_with_the_partial_beside_it() {
    let scratch = Scratch::new("free-lock");
    let folder = project(&scratch);
    let runs = folder.join(".labcraft/runs");
    fs::create_dir_all(&runs).unwrap();
    let partial = runs.join("double.trace.jsonl.4000000003-1-r1.partial");
    fs::write(&partial, "{}\n").unwrap();
    fs::write(runs.join(".4000000003.lock"), "").unwrap();
    fs::write(runs.join(".4000000004.lock"), "").unwrap();
    let output = cli_in(&folder, &["run", "--arg", "x=1", ".", "double"]);
    assert_eq!(output.status.code(), Some(0));
    assert!(!partial.exists() && !runs.join(".4000000003.lock").exists(), "the lock is free: that process ended");
    assert!(!runs.join(".4000000004.lock").exists(), "a free lock with no `.partial`, left by a process that ended");
}

/// `bytes` runs the same in deterministic mode and in real time: a request of Modbus TCP (`docs/io.md`, 10) built field by
/// field, then read back.
#[test]
fn a_frame_of_bytes_is_the_same_in_both_modes() {
    let scratch = Scratch::new("bytes");
    let folder = scratch.0.join("p");
    fs::create_dir_all(&folder).unwrap();
    fs::write(
        folder.join("frame.lcf"),
        "labcraft 1\n\nuse bytes\n\n/// A request to read two holding registers.\nfn frame(address: i32) -> (request: bytes, length: i32, read_back: u16, err: error) {\n    transaction, e1 = bytes.from_u16(1)\n    protocol, e2 = bytes.from_u16(0, err: e1)\n    size, e3 = bytes.from_u16(6, err: e2)\n    unit, e4 = bytes.from_u8(17, err: e3)\n    function, e5 = bytes.from_u8(3, err: e4)\n    first, e6 = bytes.from_u16(address, err: e5)\n    count, e7 = bytes.from_u16(2, err: e6)\n    request = bytes.concat(transaction, protocol, size, unit, function, first, count)\n    length = bytes.len(request)\n    read_back, err = bytes.read_u16(request, 8, err: e7)\n}\n",
    )
    .unwrap();
    for mode in [&[][..], &["--real-time"][..]] {
        let mut args = vec!["run", "--json", "--no-trace"];
        args.extend_from_slice(mode);
        args.extend_from_slice(&["--arg", "address=107", "p", "frame"]);
        let report = report_of(&cli_in(&scratch.0, &args));
        let value = |name: &str| output(&report, name);
        // By hand: 0001 0000 0006 11 03 006b 0002, 12 bytes (107 is 0x6b), whose base64 Python's `base64.b64encode` gives;
        // the two bytes at 8 are 006b again; `bytes` and `none` as commands.md, section 5, writes them.
        assert_eq!(value("request"), json!({ "base64": "AAEAAAAGEQMAawAC" }), "{mode:?}: {report}");
        assert_eq!(value("length"), 12, "{mode:?}");
        assert_eq!(value("read_back"), 107, "{mode:?}");
        assert_eq!(value("err"), Value::Null, "{mode:?}: {report}");
    }
}

/// A program of `net` against a simulated device, through the command line (io.md 6, commands.md 7). By hand: the write of
/// `TEMP?\n` is taken at 0 ms; its response is ready 2 ms later, its latency, and `read_line` takes it then and strips `\r\n`;
/// the report has one entry per write (its `sent`, an `answer` of `null`) and one per read (the reverse). An endpoint the file
/// does not describe is not found, after the timeout of the open, 2 s by default (io.md 3).
#[test]
fn a_program_of_net_runs_against_a_simulated_device() {
    let scratch = Scratch::new("net");
    let folder = scratch.0.join("p");
    fs::create_dir_all(&folder).unwrap();
    fs::write(
        folder.join("ask.lcf"),
        "labcraft 1\n\nuse net\n\n/// Asks a board its temperature.\nfn ask(host: string) -> (text: string, err: error) {\n    c, e1 = net.connect(host, 23)\n    c2, e2 = net.write_line(c, \"TEMP?\", err: e1)\n    text, c3, e3 = net.read_line(c2, err: e2)\n    err = net.close(c3, err: e3)\n}\n",
    )
    .unwrap();
    fs::write(
        scratch.0.join("board.toml"),
        "[[net.device]]\nendpoint = \"tcp:board.local:23\"\nlatency_ms = 2\n\n[[net.device.reply]]\nrequest = \"TEMP?\\n\"\nresponse = \"23.5\\r\\n\"\n",
    )
    .unwrap();
    let run = |host: &str| {
        let host = format!("host={host}");
        report_of(&cli_in(&scratch.0, &["run", "--json", "--no-trace", "--sim", "board.toml", "--arg", &host, "p", "ask"]))
    };
    let report = run("board.local");
    assert_eq!(output(&report, "text"), json!("23.5"), "{report}");
    assert_eq!(output(&report, "err"), Value::Null, "{report}");
    assert_eq!(
        report["instruments"],
        json!([{ "resource": "tcp:board.local:23", "commands": [
            { "t_ms": 0.0, "sent": "TEMP?\n", "answer": null },
            { "t_ms": 2.0, "sent": null, "answer": "23.5\r\n" }
        ] }]),
        "{report}"
    );
    let report = run("other.local");
    assert!(output(&report, "err").to_string().contains("net.host_not_found"), "{report}");
    assert_eq!(report["clock"]["simulated_ms"], json!(2000.0), "{report}");
    assert!(report.get("instruments").is_none(), "nothing reached: {report}");
}

/// The example Modbus TCP (io.md 10, issue #240) against its simulated servers, `examples/modbus_tcp/server.toml` and
/// `exception.toml`. By hand, from the exchange of io.md 6: the request is 0001 0000 0006 01 03 0000 0002 (transaction 1,
/// protocol 0, length 6, unit 1, function 3, first register 0, count 2); the header of the response is 7 bytes, whose
/// length 7 counts the unit, so 6 bytes follow: 03 04 00e6 0019, the registers 0x00e6 = 230 and 0x0019 = 25. The exception
/// response 0001 0000 0003 01 83 02 has 0x83 & 0x80 set, length 3, so 2 bytes follow, 83 02: exception code 2. A first
/// register of 1 matches no exchange of the file, so nothing answers and the read of the header fails `io.timeout`.
#[test]
fn the_example_modbus_tcp_reads_its_simulated_server() {
    let run = |file: &str, first: &str| {
        let sim = format!("examples/modbus_tcp/{file}.toml");
        let first = format!("first={first}");
        let args = ["run", "--json", "--no-trace", "--sim", &sim, "--arg", "host=10.0.0.7", "--arg", "count=2", "--arg", &first];
        report_of(&cli(&[&args[..], &["examples", "modbus_tcp.read_from"]].concat()))
    };
    let report = run("server", "0");
    assert_eq!(output(&report, "registers"), json!([230, 25]), "{report}");
    assert_eq!(output(&report, "err"), Value::Null, "{report}");
    let exchange = |header: &str, rest: &str| {
        json!([{ "resource": "tcp:10.0.0.7:502", "commands": [
            { "t_ms": 0.0, "sent": "000100000006010300000002", "answer": null, "hex": true },
            { "t_ms": 0.0, "sent": null, "answer": header, "hex": true },
            { "t_ms": 0.0, "sent": null, "answer": rest, "hex": true }
        ] }])
    };
    assert_eq!(report["instruments"], exchange("00010000000701", "030400e60019"), "{report}");
    let report = run("exception", "0");
    let err = output(&report, "err");
    assert_eq!((&err["category"], &err["code"]), (&json!("modbus_tcp.exception"), &json!(2)), "{report}");
    assert_eq!(output(&report, "registers"), json!([]), "{report}");
    assert_eq!(report["instruments"], exchange("00010000000301", "8302"), "{report}");
    // The request for register 1 is written, and the read of the header waits for the 2 s of the connection.
    let report = run("server", "1");
    assert_eq!(output(&report, "err")["category"], json!("io.timeout"), "{report}");
    assert_eq!(
        report["instruments"],
        json!([{ "resource": "tcp:10.0.0.7:502", "commands": [
            { "t_ms": 0.0, "sent": "000100000006010300010002", "answer": null, "hex": true }
        ] }]),
        "{report}"
    );
    assert_eq!(report["clock"]["simulated_ms"], json!(2000.0), "{report}");
}

/// What `examples/modbus_tcp` refuses (decisions of the owner on #240): a count outside 1..125, which Modbus cannot read
/// at once, before anything is sent; and a response that is not function 3 with two bytes per register, here function 4
/// (0x04, high bit clear) answering the request of io.md 6, or one cut short. None gives a register.
#[test]
fn the_example_modbus_tcp_refuses_a_wrong_count_and_a_wrong_response() {
    let scratch = Scratch::new("modbus");
    fs::write(
        scratch.0.join("function4.toml"),
        "[[net.device]]
endpoint = \"tcp:10.0.0.7:502\"

[[net.device.reply]]
request_hex = \"0001 0000 0006 01 03 0000 0002\"
response_hex = \"0001 0000 0007 01 04 04 00e6 0019\"
",
    )
    .unwrap();
    let examples = repository().join("examples");
    let run = |sim: &Path, count: &str| {
        let (sim, count) = (sim.to_str().unwrap(), format!("count={count}"));
        let args = [
            "run",
            "--json",
            "--no-trace",
            "--sim",
            sim,
            "--arg",
            "host=10.0.0.7",
            "--arg",
            &count,
            examples.to_str().unwrap(),
            "modbus_tcp.read_from",
        ];
        report_of(&cli(&args))
    };
    for count in ["0", "126", "40000"] {
        let report = run(&repository().join("examples/modbus_tcp/server.toml"), count);
        assert_eq!(output(&report, "err")["category"], json!("io.invalid_argument"), "{count}: {report}");
        assert_eq!(output(&report, "registers"), json!([]), "{count}: {report}");
        assert!(report.get("instruments").is_none(), "{count}: nothing sent: {report}");
    }
    // Cut short: the byte count says 4, but the length of 5 leaves 2 bytes after it (unit, function, count, then 00e6).
    fs::write(
        scratch.0.join("cut.toml"),
        "[[net.device]]
endpoint = \"tcp:10.0.0.7:502\"

[[net.device.reply]]
request_hex = \"0001 0000 0006 01 03 0000 0002\"
response_hex = \"0001 0000 0005 01 03 04 00e6\"
",
    )
    .unwrap();
    for sim in ["function4.toml", "cut.toml"] {
        let report = run(&scratch.0.join(sim), "2");
        assert_eq!(output(&report, "err")["category"], json!("modbus_tcp.invalid_response"), "{sim}: {report}");
        assert_eq!(output(&report, "registers"), json!([]), "{sim}: {report}");
    }
}

/// Issue #371, points 1, 2 and 10: a program reaches `visa.sim` by `use visa.sim` (a sub-library shipped with `visa`, cli.md 3),
/// opens an instrument by another spelling of its real name, and the report keeps its commands under the `resource` of the
/// file and counts its handle as a `visa.sim.instrument`. By hand: the first read of `dmm-230V.toml` is `230.071217`
/// (simulation.md 3), followed by `\n` (README, "Simulation: `visa.sim`").
#[test]
fn a_program_of_visa_sim_runs_against_an_instrument_with_a_real_name() {
    let scratch = Scratch::new("visa-sim");
    let folder = scratch.0.join("p");
    fs::create_dir_all(&folder).unwrap();
    let body = |close: &str| {
        format!(
            "labcraft 1\n\nuse bytes\nuse visa.sim\n\n/// Measures once.\nfn measure() -> (text: string, err: error) {{\n    \
             i0, e0 = visa.sim.open(\"tcpip0::10.0.0.5::05025::socket\")\n    \
             command = bytes.from_string(\"MEAS:VOLT:DC?\")\n    \
             i1, e1 = visa.sim.write(i0, command, err: e0)\n    \
             newline = bytes.from_string(\"\\n\")\n    \
             data, i2, e2 = visa.sim.read_until(i1, newline, err: e1)\n    \
             text, e3 = bytes.to_string(data, err: e2)\n    \
             {close}\n}}\n"
        )
    };
    fs::write(folder.join("measure.lcf"), body("err = visa.sim.close(i2, err: e3)")).unwrap();
    let dmm = fs::read_to_string(repository().join("libs/visa/sim/dmm-230V.toml")).unwrap();
    fs::write(scratch.0.join("bench.toml"), dmm.replace("SIM::DMM0", "TCPIP0::10.0.0.5::5025::SOCKET")).unwrap();
    let run = || report_of(&cli_in(&scratch.0, &["run", "--json", "--no-trace", "--sim", "bench.toml", "p", "measure"]));
    let report = run();
    assert_eq!(output(&report, "text"), json!("230.071217\n"), "{report}");
    assert_eq!(output(&report, "err"), Value::Null, "{report}");
    assert_eq!(
        report["instruments"],
        json!([{ "resource": "TCPIP0::10.0.0.5::5025::SOCKET", "commands": [
            { "t_ms": 0.0, "sent": "MEAS:VOLT:DC?", "answer": "230.071217" }
        ] }]),
        "{report}"
    );
    assert_eq!(report["resources"]["opened"], json!(1), "{report}");
    // Left open, the handle shows by its type.
    fs::write(folder.join("measure.lcf"), body("err = e3")).unwrap();
    let report = run();
    assert_eq!(report["resources"]["open"][0]["type"], json!("visa.sim.instrument"), "{report}");
    assert_eq!(report["resources"]["open"][0]["resource"], json!("TCPIP0::10.0.0.5::5025::SOCKET"), "{report}");
}
