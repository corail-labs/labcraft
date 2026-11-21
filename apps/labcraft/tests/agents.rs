//! An agent attached to the window (ide.md 8; scenario 2 of acceptance-m1.md, points 1, 4 and 5): the session of a
//! project on its thread, as the app runs it, its channel open, and clients that speak MCP to it over the loopback as
//! `labcraft-cli mcp` relays them. The answers of the user are given by the test, as the window would give them.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use labcraft::agents::{Agents, Channel};
use labcraft::session::{Handle, Opening};
use labcraft_cli::attach;
use labcraft_ide::Host;
use labcraft_ide::agents::{Answer, Asks, Policy};
use serde_json::{Value, json};

const PROGRAM: &str = include_str!("../../../examples/check_voltage.lcf");
const VERDICT: &str = include_str!("../../../examples/Verdict.lct");

/// A folder of its own for a test, outside the repository: the project, and the folder of the channel.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Scratch {
        let path = std::env::temp_dir().join(format!("labcraft-agents-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(path.join("project")).unwrap();
        std::fs::create_dir_all(path.join("channel")).unwrap();
        std::fs::write(path.join("project/check_voltage.lcf"), PROGRAM).unwrap();
        std::fs::write(path.join("project/Verdict.lct"), VERDICT).unwrap();
        Scratch(path.canonicalize().unwrap())
    }

    fn project(&self) -> PathBuf {
        self.0.join("project")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The window: the session of the project, its agents and their channel, and the user who answers the questions.
struct Window {
    channel: Option<Channel>,
    handle: Option<Handle>,
    answering: Arc<AtomicBool>,
    user: Option<std::thread::JoinHandle<Vec<Asks>>>,
}

impl Window {
    /// Opens the project; `answer` is what the user answers to each question, as it comes.
    fn open(scratch: &Scratch, answer: impl Fn(&Asks, &str) -> Answer + Send + 'static) -> Window {
        let root = scratch.project();
        let opening = Opening { folder: root.clone(), lib_paths: Vec::new(), user_settings: serde_json::Map::new(), style: None, new: false };
        let (handle, reply) = Handle::spawn(opening, Box::new(|| {})).unwrap();
        reply.wait(Duration::from_secs(60)).expect("the project opens").unwrap();
        let agents = Arc::new(Agents::new(&root, Box::new(|| {})));
        let channel = Channel::open_in(&scratch.0.join("channel"), &root, handle.sender(), handle.served().unwrap(), agents.clone()).unwrap();
        assert!(std::ptr::eq(channel.agents(), &*agents));
        let answering = Arc::new(AtomicBool::new(true));
        let (asked, going) = (agents.clone(), answering.clone());
        let user = std::thread::spawn(move || {
            let mut seen = Vec::new();
            while going.load(Ordering::SeqCst) {
                for question in asked.questions(true).unwrap_or_default() {
                    asked.answer(question.id, answer(&question.asks, &question.signed));
                    seen.push(question.asks);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            seen
        });
        Window { channel: Some(channel), handle: Some(handle), answering, user: Some(user) }
    }

    fn handle(&self) -> &Handle {
        self.handle.as_ref().unwrap()
    }

    /// A command of the user, from the window.
    fn user(&self, id: &str, params: Value) -> Value {
        self.handle().send(id, params).wait(Duration::from_secs(60)).unwrap().unwrap()
    }

    /// Closes the window: what the user was asked.
    fn close(mut self) -> Vec<Asks> {
        self.answering.store(false, Ordering::SeqCst);
        let asked = self.user.take().unwrap().join().unwrap();
        self.channel = None;
        self.handle = None;
        asked
    }
}

/// A client, as `labcraft-cli mcp` relays it.
struct Client {
    stream: TcpStream,
    lines: BufReader<TcpStream>,
    next: i64,
}

impl Client {
    fn attach(window: &Window, name: &str) -> Client {
        let entry = attach::read_entry(&window.channel.as_ref().unwrap().file()).unwrap();
        let stream = attach::connect(&entry).unwrap();
        let lines = BufReader::new(stream.try_clone().unwrap());
        let mut client = Client { stream, lines, next: 0 };
        let init = client
            .request("initialize", json!({ "protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": { "name": name, "version": "1" } }));
        assert_eq!(init["result"]["serverInfo"]["name"], "labcraft", "{init}");
        client
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let id = self.next;
        writeln!(self.stream, "{}", json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })).unwrap();
        self.stream.flush().unwrap();
        loop {
            let mut line = String::new();
            assert_ne!(self.lines.read_line(&mut line).unwrap(), 0, "the window answers");
            let message: Value = serde_json::from_str(&line).unwrap();
            if message.get("id") == Some(&json!(id)) {
                return message;
            }
        }
    }

    /// Calls a tool: its object, and whether it failed.
    fn call(&mut self, tool: &str, arguments: Value) -> (Value, bool) {
        let answer = self.request("tools/call", json!({ "name": tool, "arguments": arguments }));
        let result = &answer["result"];
        (result["structuredContent"].clone(), result["isError"] == true)
    }

    fn ok(&mut self, tool: &str, arguments: Value) -> Value {
        let (value, failed) = self.call(tool, arguments);
        assert!(!failed, "{tool}: {value}");
        value
    }

    fn code(&mut self, tool: &str, arguments: Value) -> String {
        let (value, failed) = self.call(tool, arguments);
        assert!(failed, "{tool} fails: {value}");
        value["error"]["code"].as_str().unwrap_or_else(|| panic!("{value}")).to_owned()
    }
}

fn edits(asks: &Asks, _: &str) -> Answer {
    match asks {
        Asks::Connect => Answer::Policy { policy: Policy::Edits, keep: false },
        Asks::Command { .. } => Answer::Refuse,
    }
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn the_agent_edits_through_the_session_of_the_window_signed_with_its_name() {
    let scratch = Scratch::new("edits");
    let window = Window::open(&scratch, edits);
    let mut agent = Client::attach(&window, "claude");
    let before = read(&scratch.project().join("check_voltage.lcf"));
    // A direct tool, and a command that is not one through `commands_execute` (mcp.md 2).
    agent.ok("node_add", json!({ "file": "check_voltage.lcf", "call": "core.not" }));
    agent.ok("commands_execute", json!({ "id": "file.write", "params": { "path": "notes.lcf", "text": "labcraft 1\n\nfn notes() -> () {}\n" } }));
    let listed = window.user("history.list", json!({}));
    let authors: Vec<&str> = listed["steps"].as_array().unwrap().iter().map(|step| step["author"].as_str().unwrap()).collect();
    assert_eq!(authors, ["claude", "claude"], "{listed}");
    // Point 2 by command: the user's undo takes back the agent's last step; and the one before, byte for byte.
    window.user("history.undo", json!({}));
    window.user("history.undo", json!({}));
    assert_eq!(read(&scratch.project().join("check_voltage.lcf")), before);
    // A run asks under `Edits`, and the user refused it.
    assert_eq!(agent.code("run_start", json!({ "function": "check_voltage" })), "agent.not_allowed");
    // The window's state is read; what the user sees is not changed (ide.md 8).
    agent.ok("view_get_state", json!({}));
    assert_eq!(agent.code("commands_execute", json!({ "id": "view.select", "params": { "nodes": [] } })), "agent.not_allowed");
    // The settings of AI are the user's.
    assert_eq!(agent.code("commands_execute", json!({ "id": "settings.get", "params": { "key": "ai.policies" } })), "agent.not_allowed");
    let asked = window.close();
    assert_eq!(asked.iter().filter(|asks| **asks == Asks::Connect).count(), 1, "{asked:?}");
    assert!(
        asked.iter().any(|asks| matches!(asks, Asks::Command { effects, .. } if effects.contains(&labcraft_engine::registry::Effect::Run))),
        "{asked:?}"
    );
}

#[test]
fn the_agent_does_not_undo_a_step_of_the_user() {
    let scratch = Scratch::new("not-yours");
    let window = Window::open(&scratch, edits);
    let mut agent = Client::attach(&window, "claude");
    agent.ok("node_add", json!({ "file": "check_voltage.lcf", "call": "core.not" }));
    window.user("node.add", json!({ "file": "check_voltage.lcf", "call": "core.not" }));
    // Point 4: `history.not_yours`.
    assert_eq!(agent.code("history_undo", json!({})), "history.not_yours");
    window.close();
}

#[test]
fn under_ask_an_edit_waits_for_the_user_and_a_refusal_changes_nothing() {
    let scratch = Scratch::new("ask");
    let window = Window::open(&scratch, |asks, _| match asks {
        Asks::Connect => Answer::Policy { policy: Policy::Ask, keep: false },
        Asks::Command { label, .. } if label == "Add Node" => Answer::Refuse,
        Asks::Command { .. } => Answer::Allow,
    });
    let mut agent = Client::attach(&window, "claude");
    let before = read(&scratch.project().join("check_voltage.lcf"));
    // A read runs without asking.
    agent.ok("file_read", json!({ "path": "check_voltage.lcf" }));
    // Point 5: refused, `agent.not_allowed`, and the file is as it was.
    assert_eq!(agent.code("node_add", json!({ "file": "check_voltage.lcf", "call": "core.not" })), "agent.not_allowed");
    assert_eq!(read(&scratch.project().join("check_voltage.lcf")), before);
    // Allowed, it runs.
    agent.ok("constant_add", json!({ "file": "check_voltage.lcf", "value": "1" }));
    assert_ne!(read(&scratch.project().join("check_voltage.lcf")), before);
    window.close();
}

#[test]
fn a_refused_connection_gets_nothing_and_two_clients_of_one_name_are_numbered() {
    let scratch = Scratch::new("refused");
    let window = Window::open(&scratch, |asks, client| match (asks, client) {
        (Asks::Connect, "agent 2") => Answer::Refuse,
        (Asks::Connect, _) => Answer::Policy { policy: Policy::All, keep: false },
        _ => Answer::Refuse,
    });
    let mut first = Client::attach(&window, "agent");
    let mut second = Client::attach(&window, "agent");
    first.ok("file_read", json!({ "path": "check_voltage.lcf" }));
    assert_eq!(second.code("file_read", json!({ "path": "check_voltage.lcf" })), "agent.not_allowed");
    first.ok("node_add", json!({ "file": "check_voltage.lcf", "call": "core.not" }));
    let listed = window.user("history.list", json!({}));
    assert_eq!(listed["steps"][0]["author"], "agent", "{listed}");
    window.close();
    assert!(window_gone(&scratch), "the file of the channel goes with the window");
}

fn window_gone(scratch: &Scratch) -> bool {
    std::fs::read_dir(scratch.0.join("channel")).unwrap().next().is_none()
}

const DOUBLE: &str = "labcraft 1\n\nuse core\n\n/// Doubles a number.\nfn double(x: f64) -> (y: f64) {\n    y = core.mul(x, 2)\n}\n";
const FOREVER: &str = "labcraft 1\n\nuse core\n\n/// Counts for ever.\nfn forever() -> (n: f64) {\n    n = loop carry i = 0.0 {\n        m = core.add(i, 1.0)\n        done = core.ge(0.0, m)\n        next i = m\n        until done\n    }\n}\n";

#[test]
fn under_all_a_run_of_the_agent_is_waited_for_off_the_session_and_aborted_as_it_leaves() {
    let scratch = Scratch::new("runs");
    let window = Window::open(&scratch, |_, _| Answer::Policy { policy: Policy::All, keep: false });
    let mut agent = Client::attach(&window, "claude");
    agent.ok("file_write", json!({ "path": "double.lcf", "text": DOUBLE }));
    agent.ok("file_write", json!({ "path": "forever.lcf", "text": FOREVER }));
    // `run.start` waits, as it does headless, but not on the session: the window still answers meanwhile.
    let report = agent.ok("run_start", json!({ "function": "double", "args": { "x": 21.0 } }));
    assert_eq!((report["status"].as_str(), report["outputs"][0]["value"].as_f64()), (Some("ok"), Some(42.0)), "{report}");
    let long = json!({ "function": "forever", "max_iterations": null, "max_nodes": null, "wait": "0 s", "trace": "none" });
    let going = agent.ok("run_start", long);
    assert_eq!(going["status"], "running", "{going}");
    assert!(window.user("history.list", json!({}))["steps"].is_array(), "the session answers the user while the run goes");
    let run = going["run"].as_str().unwrap().to_owned();
    // The agent leaves: its run is aborted.
    drop(agent);
    let mut status = Value::Null;
    for _ in 0..200 {
        status = window.user("run.status", json!({ "run": run }));
        if status["status"] != "running" {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    assert_eq!(status["status"], "aborted", "{status}");
    window.close();
}

const START: &str = "labcraft 1\n\nfn f(x: f64) -> (y: f64) {\n    y = x\n}\n";

/// Scenario 2, point 1: a node added, wired and given a constant, through the direct tools.
#[test]
fn the_agent_adds_a_node_wires_it_and_sets_a_constant() {
    let scratch = Scratch::new("wire");
    std::fs::write(scratch.project().join("f.lcf"), START).unwrap();
    let window = Window::open(&scratch, edits);
    let mut agent = Client::attach(&window, "claude");
    agent.ok("node_add", json!({ "file": "f.lcf", "call": "core.add" }));
    agent.ok("wire_connect", json!({ "file": "f.lcf", "from": "x", "to": "sum.a" }));
    agent.ok("input_set_constant", json!({ "file": "f.lcf", "to": "sum.b", "value": "1.0" }));
    // Worked out by hand from commands.md 6.7: the call takes its inputs in order, the wire then the constant.
    assert!(read(&scratch.project().join("f.lcf")).contains("    sum = core.add(x, 1.0)\n"), "{}", read(&scratch.project().join("f.lcf")));
    let listed = window.user("history.list", json!({}));
    assert_eq!(listed["steps"].as_array().unwrap().len(), 3, "{listed}");
    window.close();
}

/// A wait on a run through `commands_execute` does not hold the session of the window, and an agent under `Edits`
/// stops its own run at once, without a question; not one of the user.
#[test]
#[expect(clippy::disallowed_methods, reason = "a test of the desktop app, never built for WASM")]
fn a_wait_through_commands_execute_does_not_hold_the_window_and_an_agent_stops_its_own_run() {
    let scratch = Scratch::new("own-run");
    std::fs::write(scratch.project().join("forever.lcf"), FOREVER).unwrap();
    let window = Window::open(&scratch, |asks, _| match asks {
        Asks::Connect => Answer::Policy { policy: Policy::Edits, keep: false },
        Asks::Command { .. } => Answer::Allow,
    });
    let mut agent = Client::attach(&window, "claude");
    let long = json!({ "function": "forever", "max_iterations": null, "max_nodes": null, "wait": "0 s", "trace": "none" });
    let going = agent.ok("run_start", long);
    let run = going["run"].as_str().unwrap().to_owned();
    let waiting = std::thread::spawn(move || {
        let waited = agent.ok("commands_execute", json!({ "id": "run.wait", "params": { "run": run, "wait": "3 s" } }));
        (agent, waited)
    });
    std::thread::sleep(Duration::from_millis(300));
    let asked = std::time::Instant::now();
    window.user("history.list", json!({}));
    assert!(asked.elapsed() < Duration::from_secs(2), "the session answered the user while the agent waited: {:?}", asked.elapsed());
    let (mut agent, waited) = waiting.join().unwrap();
    assert_eq!(waited["status"], "running", "{waited}");
    // `run_abort` is answered at once: its own run, under `Edits`.
    let aborted = agent.ok("run_abort", json!({}));
    assert_eq!(aborted["status"], "aborted", "{aborted}");
    window.close();
}
