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
fn engine_report(params: &Value) -> Value {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(labcraft_format::STACK_SIZE)
            .spawn_scoped(scope, || {
                let config = Config {
                    root: repository(),
                    lib_paths: Vec::new(),
                    limits: Limits::default(),
                    shipped: labcraft_cli::shipped(),
                    natives: vec![Arc::new(labcraft_visa::Visa), Arc::new(labcraft_results_log::ResultsLog)],
                    clock: Arc::new(labcraft_engine::Still),
                    process: 0,
                };
                let session = Session::new(Arc::new(Disk), config, false).unwrap();
                labcraft_engine::run_to_end(&session, params).unwrap()
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
            _ => options.extend([format!("--{}", key.replace('_', "-")), if value.is_null() { "none".to_owned() } else { text_of(value) }]),
        }
    }
    options.push(object["folder"].as_str().unwrap_or(".").to_owned());
    options.push(object["function"].as_str().unwrap().to_owned());
    options
}

/// What differs between two runs of one program: the real clock.
fn without_real_time(mut report: Value) -> Value {
    if let Some(clock) = report.get_mut("clock").and_then(Value::as_object_mut) {
        clock.remove("real_ms");
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
        &["run", "examples", "check_voltage", "--real-time"],
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
