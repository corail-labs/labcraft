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
            // No window of the user is found (architecture 12.8).
            .env("LABCRAFT_ATTACH_DIR", "")
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
    // The nineteen of M0b, the eight of editing built in M1 step 0, and `value_rename` and `input_set_constant` of step 4.
    assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 29, "mcp.md 2");
    assert!(init["result"]["instructions"].as_str().unwrap().contains("`guide_read` once"));

    // mcp.md 8, call 1: the skills, and the one that writes a program, built into the binary.
    let guide = client.call("guide_read", json!({}));
    assert_eq!(guide["skill"], "labcraft-writing", "{guide}");
    assert!(guide["text"].as_str().unwrap().contains("`functions_list`"));

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
    // Point 5: at most 20 calls to the tools; this loop takes 8, and the comparison with the command line one more.
    assert_eq!(client.calls, 9);
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

/// The program of the tests of consent: it opens a TCP connection, then closes it.
const REACH: &str = "labcraft 1\n\nuse net\n\n/// Opens a connection and closes it.\nfn reach(host: string, port: u16) -> (err: error) {\n    c, e1 = net.connect(host, port)\n    err = net.close(c, err: e1)\n}\n";

/// A run of the agent reaches a real endpoint only with `--allow-hardware` (io.md 4; issue #232), given as an endpoint or
/// as a VISA name that reduces to it; a name is never resolved, so `localhost` does not allow `127.0.0.1`. Without the right
/// the open gets `io.not_allowed`, code 0, on its wire, whose message names the option; `labcraft-cli run` of the same
/// program, a run of the human, reaches it. The endpoint is a TCP server on `127.0.0.1` that the test starts (io.md 7).
#[test]
fn a_run_of_the_agent_reaches_a_real_endpoint_only_with_allow_hardware() {
    let scratch = Scratch::new("allow");
    std::fs::write(scratch.0.join("reach.lcf"), REACH).unwrap();
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            drop(stream);
        }
    });
    let endpoint = format!("tcp:127.0.0.1:{port}");
    let reach = |args: &[&str]| {
        let mut client = Client::start(&scratch.0, args);
        client.request("initialize", json!({ "protocolVersion": "2025-11-25" }));
        let report = client.call("run_start", json!({ "function": "reach", "mode": "real_time", "args": { "host": "127.0.0.1", "port": port } }));
        assert!(client.end().success());
        // An error on a wire, not a refused run: the run ends, its status that of its `err` (spec 8.2).
        let err = output(&report, "err");
        assert_eq!(report["status"], if err.is_null() { "ok" } else { "error" }, "{args:?}: {report}");
        err
    };
    let refused = reach(&[]);
    assert_eq!((refused["category"].as_str(), refused["code"].as_i64()), (Some("io.not_allowed"), Some(0)), "{refused}");
    assert!(refused["message"].as_str().unwrap().contains(&format!("--allow-hardware {endpoint}")), "{refused}");
    let by_name = reach(&["--allow-hardware", &format!("tcp:LOCALHOST:{port}")]);
    assert_eq!(by_name["category"], "io.not_allowed", "a name is never resolved: {by_name}");
    assert_eq!(reach(&["--allow-hardware", &endpoint]), Value::Null);
    assert_eq!(reach(&["--allow-hardware", &format!("TCPIP0::127.0.0.1::0{port}::SOCKET")]), Value::Null, "the same right");
    assert_eq!(reach(&["--allow-hardware", "serial:COM3", "--allow-hardware", &endpoint]), Value::Null, "repeated");

    let host = Command::new(env!("CARGO_BIN_EXE_labcraft-cli"))
        .current_dir(&scratch.0)
        .args(["run", "--json", "--no-trace", "--real-time", "--arg", "host=127.0.0.1", "--arg", &format!("port={port}"), ".", "reach"])
        .output()
        .unwrap();
    let report: Value = serde_json::from_slice(&host.stdout).unwrap();
    assert_eq!(output(&report, "err"), Value::Null, "the human has every right: {report}");
}

#[test]
fn wrong_usage_is_exit_2_with_nothing_on_stdout() {
    let scratch = Scratch::new("usage");
    let cli = |args: &[&str]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_labcraft-cli"));
        command.env("LABCRAFT_ATTACH_DIR", "").current_dir(&scratch.0).args(args).stdin(Stdio::null()).output().unwrap()
    };
    let home = std::env::home_dir().unwrap();
    let home = home.to_str().unwrap();
    let root = if cfg!(windows) { "C:\\" } else { "/" };
    std::fs::write(scratch.0.join("a-file.txt"), "").unwrap();
    for args in [
        vec!["mcp", "--json"],
        vec!["mcp", "somewhere"],
        vec!["mcp", "--project"],
        vec!["mcp", "--project", "missing-folder"],
        vec!["mcp", "--project", home],
        vec!["mcp", "--project", root],
        vec!["mcp", "--max-nesting", "5000"],
        // io.md 2 and 4 (issue #232): neither an endpoint nor a VISA name that reduces to one, a wildcard, no value; the
        // other forms are in the tests of `labcraft_runtime::io::endpoint`.
        vec!["mcp", "--allow-hardware", "banana"],
        vec!["mcp", "--allow-hardware", "ASRL0"],
        vec!["mcp", "--allow-hardware", "tcp:10.0.0.*:502"],
        vec!["mcp", "--allow-hardware"],
        // A serial name that would allow no port (decision of the owner, 2026-11-24).
        vec!["mcp", "--allow-hardware", "serial:COM*"],
        vec!["mcp", "--allow-hardware", "serial:/dev/ttyUSB?"],
        vec!["mcp", "--allow-hardware", "serial:COM3 "],
        // A folder that does not exist, a file, and the folders too wide for a project (decisions of the owner, 2026-11-24).
        vec!["mcp", "--allow-path", "missing-folder"],
        vec!["mcp", "--allow-path", "a-file.txt"],
        vec!["mcp", "--allow-path", home],
        vec!["mcp", "--allow-path", root],
        vec!["mcp", "--allow-path"],
    ] {
        let output = cli(&args);
        assert_eq!(output.status.code(), Some(2), "{args:?}: {}", String::from_utf8_lossy(&output.stderr));
        assert!(output.stdout.is_empty(), "{args:?}");
        assert!(String::from_utf8_lossy(&output.stderr).starts_with("error: "), "{args:?}");
    }
    let wide = cli(&["mcp", "--project", home]);
    assert!(String::from_utf8_lossy(&wide.stderr).contains("is the home folder"), "{}", String::from_utf8_lossy(&wide.stderr));
    let file = cli(&["mcp", "--allow-path", "a-file.txt"]);
    assert!(String::from_utf8_lossy(&file.stderr).contains("--allow-path takes a folder"), "{}", String::from_utf8_lossy(&file.stderr));
    let wide = cli(&["mcp", "--allow-path", root]);
    assert!(String::from_utf8_lossy(&wide.stderr).contains("is the root of a drive"), "{}", String::from_utf8_lossy(&wide.stderr));
    // An empty stdin: the server ends at once, having said nothing.
    let quiet = cli(&["mcp"]);
    assert_eq!(quiet.status.code(), Some(0), "{}", String::from_utf8_lossy(&quiet.stderr));
    assert!(quiet.stdout.is_empty());
    // Rights that parse, and a folder outside the project that exists: the server starts.
    let outside = Scratch::new("usage-outside");
    let allowed = cli(&["mcp", "--allow-hardware", "ASRL3::INSTR", "--allow-path", outside.0.to_str().unwrap(), "--allow-path", "."]);
    assert_eq!(allowed.status.code(), Some(0), "{}", String::from_utf8_lossy(&allowed.stderr));
    assert!(allowed.stderr.is_empty(), "{}", String::from_utf8_lossy(&allowed.stderr));
}
