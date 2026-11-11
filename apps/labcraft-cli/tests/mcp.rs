//! `labcraft-cli mcp` (`docs/mcp.md`) through a real stdio client: the scripted loop of scenario 1 (acceptance-m0.md,
//! verification), which writes a draft with one error, reads the diagnostic, fixes it, and runs it against both
//! multimeters and against `no-instrument.toml`; the end of stdin, and wrong usage. The program is
//! `examples/check_voltage.lcf`, and the expected values of its runs those of the corpus of runs, `s1-pass-230V`,
//! `s1-fail-250V` and `s1-no-instrument`, worked out by hand there; the draft is it with one name misspelt, `LC2001`
//! (cli.md 5, unknown value).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use serde_json::{Value, json};

const PROGRAM: &str = include_str!("../../../examples/check_voltage.lcf");
const VERDICT: &str = include_str!("../../../examples/Verdict.lct");

/// A folder of its own for a test, empty, outside the repository.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Scratch {
        let path = std::env::temp_dir().join(format!("labcraft-mcp-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Scratch(path.canonicalize().unwrap())
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The client: the server as a child process, its stdout read line by line on a thread.
struct Client {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
    next: i64,
    calls: usize,
}

impl Client {
    fn start(folder: &Path, args: &[&str]) -> Client {
        let mut child = Command::new(env!("CARGO_BIN_EXE_labcraft-cli"))
            .current_dir(folder)
            .arg("mcp")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("the program starts");
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().unwrap();
        let (send, lines) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { return };
                if send.send(line).is_err() {
                    return;
                }
            }
        });
        Client { child, stdin, lines, next: 0, calls: 0 }
    }

    /// Sends a request and waits for its answer, every line before it being a notification.
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let id = self.next;
        let line = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }).to_string();
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{line}").unwrap();
        stdin.flush().unwrap();
        loop {
            let line = self.lines.recv_timeout(Duration::from_secs(60)).expect("an answer within a minute");
            let message: Value = serde_json::from_str(&line).unwrap_or_else(|error| panic!("{error}: stdout holds only MCP messages: {line}"));
            assert_eq!(message["jsonrpc"], "2.0");
            if message.get("id") == Some(&json!(id)) {
                return message;
            }
            assert!(message.get("method").is_some(), "only notifications come before the answer: {message}");
        }
    }

    /// Calls a tool, and gives its object; a failed call fails the test.
    fn call(&mut self, tool: &str, arguments: Value) -> Value {
        self.calls += 1;
        let answer = self.request("tools/call", json!({ "name": tool, "arguments": arguments }));
        let result = &answer["result"];
        assert_eq!(result["isError"], false, "{tool}: {answer}");
        let text: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(text, result["structuredContent"], "the same JSON as text");
        text
    }

    /// Closes stdin, and waits for the server to end.
    fn end(mut self) -> std::process::ExitStatus {
        self.stdin = None;
        self.child.wait().unwrap()
    }
}

fn output(report: &Value, name: &str) -> Value {
    report["outputs"].as_array().unwrap().iter().find(|output| output["name"] == name).unwrap_or_else(|| panic!("{report}"))["value"].clone()
}

#[test]
fn the_scripted_loop_of_scenario_1() {
    let scratch = Scratch::new("loop");
    let mut client = Client::start(&scratch.0, &[]);
    let init = client
        .request("initialize", json!({ "protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": { "name": "scripted", "version": "1" } }));
    assert_eq!(init["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(init["result"]["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
    let stdin = client.stdin.as_mut().unwrap();
    writeln!(stdin, "{}", json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).unwrap();
    let tools = client.request("tools/list", json!({}));
    assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 18, "mcp.md 2, less `guide_read` until 9b");

    // The simulation files shipped with `visa`, and their resources.
    let project = client.call("project_describe", json!({}));
    let simulations = project["simulations"].as_array().unwrap();
    let sim = |file: &str| simulations.iter().find(|entry| entry["file"] == file).unwrap_or_else(|| panic!("{file}: {project}")).clone();
    assert_eq!(sim("<labcraft>/visa/sim/dmm-230V.toml")["resources"], json!(["SIM::DMM0"]));
    assert_eq!(sim("<labcraft>/visa/sim/no-instrument.toml")["resources"], json!([]));

    let written = client.call("file_write", json!({ "path": "Verdict.lct", "text": VERDICT }));
    assert_eq!(written["schema"], "labcraft-check/1");
    // A draft with one name misspelt: one error, at its node.
    let draft = PROGRAM.replace("core.le(voltage, high)", "core.le(voltge, high)");
    assert_ne!(draft, PROGRAM);
    let checked = client.call("file_write", json!({ "path": "check_voltage.lcf", "text": draft }));
    assert_eq!(checked["status"], "error", "{checked}");
    let first = &checked["diagnostics"][0];
    assert_eq!((first["code"].as_str(), first["file"].as_str()), (Some("LC2001"), Some("check_voltage.lcf")), "{checked}");
    assert!(first["message"].as_str().unwrap().contains("voltge"), "{first}");
    // Scenario 2, point 4: the report of MCP is that of `labcraft-cli check --json`.
    let cli = Command::new(env!("CARGO_BIN_EXE_labcraft-cli")).current_dir(&scratch.0).args(["check", "--json", "."]).output().unwrap();
    let from_cli: Value = serde_json::from_slice(&cli.stdout).unwrap();
    assert_eq!(client.call("library_check", json!({})), from_cli);
    // Fixed: no error and no warning (scenario 1, point 1).
    let checked = client.call("file_write", json!({ "path": "check_voltage.lcf", "text": PROGRAM }));
    assert_eq!(checked["status"], "ok", "{checked}");
    assert_eq!((checked["summary"]["errors"].as_u64(), checked["summary"]["warnings"].as_u64()), (Some(0), Some(0)), "{checked}");

    // Point 2: PASS at 230 V, FAIL at 250 V; point 3: an error and not PASS without an instrument; point 4: no session left.
    let run = |client: &mut Client, sim: &str| {
        client.call("run_start", json!({ "function": "check_voltage", "sim": sim, "args": { "resource": "SIM::DMM0" } }))
    };
    let pass = run(&mut client, "<labcraft>/visa/sim/dmm-230V.toml");
    assert_eq!(pass["status"], "ok", "{pass}");
    assert_eq!(output(&pass, "verdict"), "Verdict.Pass");
    assert_eq!(output(&pass, "voltage"), "230.071217 V");
    let fail = run(&mut client, "<labcraft>/visa/sim/dmm-250V.toml");
    assert_eq!(fail["status"], "ok", "{fail}");
    assert_eq!(output(&fail, "verdict"), "Verdict.Fail");
    assert_eq!(output(&fail, "voltage"), "250.071217 V");
    let absent = run(&mut client, "<labcraft>/visa/sim/no-instrument.toml");
    assert_eq!(absent["status"], "error", "{absent}");
    assert_eq!(output(&absent, "verdict"), "Verdict.Error");
    assert_eq!(absent["error"]["category"], "visa.resource_not_found");
    for report in [&pass, &fail, &absent] {
        assert_eq!(report["resources"]["open"], json!([]), "{report}");
    }
    assert_eq!(pass["resources"]["opened"], 1);
    assert_eq!(absent["resources"]["opened"], 0);
    // Point 5: at most 20 calls to the tools; this loop takes 7, and the comparison with the command line one more.
    assert_eq!(client.calls, 8);
    let ended = client.end();
    assert!(ended.success(), "{ended:?}");
}

#[test]
#[expect(clippy::disallowed_methods, reason = "the command line is never built for WASM; its tests run natively")]
fn the_end_of_stdin_aborts_the_run_going_and_ends_the_server() {
    let scratch = Scratch::new("end");
    let forever = "labcraft 1\n\nuse core\n\n/// Counts for ever.\nfn forever() -> (n: f64) {\n    n = loop carry i = 0.0 {\n        m = core.add(i, 1.0)\n        done = core.ge(0.0, m)\n        next i = m\n        until done\n    }\n}\n";
    std::fs::write(scratch.0.join("forever.lcf"), forever).unwrap();
    let mut client = Client::start(&scratch.0, &["--project", "."]);
    client.request("initialize", json!({ "protocolVersion": "2025-06-18" }));
    let running = client.call("run_start", json!({ "function": "forever", "max_iterations": null, "max_nodes": null, "wait": "0 s" }));
    assert_eq!(running["status"], "running", "{running}");
    let started = std::time::Instant::now();
    let ended = client.end();
    assert!(ended.success(), "{ended:?}");
    assert!(started.elapsed() < Duration::from_secs(30), "the run is aborted, not waited for");
    // Its trace was written whole, its `.partial` renamed, and the lock of the process let go (commands.md 8).
    let runs = scratch.0.join(".labcraft").join("runs");
    let names: Vec<String> = std::fs::read_dir(&runs).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(names, ["forever.trace.jsonl"], "no `.partial` and no lock left");
    let trace = std::fs::read_to_string(runs.join("forever.trace.jsonl")).unwrap();
    let end: Value = serde_json::from_str(trace.lines().last().unwrap()).unwrap();
    assert_eq!((end["event"].as_str(), end["status"].as_str()), (Some("end"), Some("aborted")), "{end}");
}

#[test]
fn a_request_of_100_mb_is_refused_unread_and_the_server_goes_on() {
    let scratch = Scratch::new("large");
    let mut client = Client::start(&scratch.0, &[]);
    // Scenario 4, family MCP: a line of 100 MB, a request whose `id` cannot be known, sent as a pipe gives it.
    let stdin = client.stdin.as_mut().unwrap();
    stdin.write_all(b"{\"jsonrpc\": \"2.0\", \"id\": 1, \"method\": \"ping\", \"params\": {\"pad\": \"").unwrap();
    let chunk = vec![b'x'; 1024 * 1024];
    for _ in 0..100 {
        stdin.write_all(&chunk).unwrap();
    }
    stdin.write_all(b"\"}}\n").unwrap();
    let refused: Value = serde_json::from_str(&client.lines.recv_timeout(Duration::from_secs(60)).unwrap()).unwrap();
    assert_eq!((refused["id"].clone(), refused["error"]["code"].clone()), (Value::Null, json!(-32600)), "{refused}");
    client.next = 1;
    assert_eq!(client.request("ping", json!({}))["result"], json!({}));
    assert!(client.end().success());
}

#[test]
fn wrong_usage_is_exit_2_with_nothing_on_stdout() {
    let scratch = Scratch::new("usage");
    let cli =
        |args: &[&str]| Command::new(env!("CARGO_BIN_EXE_labcraft-cli")).current_dir(&scratch.0).args(args).stdin(Stdio::null()).output().unwrap();
    let home = std::env::home_dir().unwrap();
    let home = home.to_str().unwrap();
    let root = if cfg!(windows) { "C:\\" } else { "/" };
    for args in [
        vec!["mcp", "--json"],
        vec!["mcp", "somewhere"],
        vec!["mcp", "--project"],
        vec!["mcp", "--project", "missing-folder"],
        vec!["mcp", "--project", home],
        vec!["mcp", "--project", root],
        vec!["mcp", "--max-nesting", "5000"],
    ] {
        let output = cli(&args);
        assert_eq!(output.status.code(), Some(2), "{args:?}: {}", String::from_utf8_lossy(&output.stderr));
        assert!(output.stdout.is_empty(), "{args:?}");
        assert!(String::from_utf8_lossy(&output.stderr).starts_with("error: "), "{args:?}");
    }
    let wide = cli(&["mcp", "--project", home]);
    assert!(String::from_utf8_lossy(&wide.stderr).contains("is the home folder"), "{}", String::from_utf8_lossy(&wide.stderr));
    // An empty stdin: the server ends at once, having said nothing.
    let quiet = cli(&["mcp"]);
    assert_eq!(quiet.status.code(), Some(0), "{}", String::from_utf8_lossy(&quiet.stderr));
    assert!(quiet.stdout.is_empty());
}
