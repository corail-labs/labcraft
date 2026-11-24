//! `labcraft-cli mcp` attached to a window (ide.md 8; architecture 12.8): it finds the file of the window of its
//! project in the folder of `LABCRAFT_ATTACH_DIR`, gives the token, and relays MCP both ways; a file left over from a
//! window that is gone starts it headless; a window that refuses the token stops it. The window here is a stand-in:
//! the channel of `labcraft_cli::attach`, and the MCP server of `mcp` on a session of its own, which counts the calls
//! it runs, so that a call is known to have gone through it.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use labcraft_cli::attach;
use labcraft_engine::{Author, CommandError, Config, Session, Waiter};
use labcraft_mcp::{Executor, Open, Served};
use serde_json::{Value, json};

/// Held by each test as long as its folder: the tests of this file run one at a time. In parallel, on Windows, the
/// connection of the first one is reset as it closes (os error 10054), for a reason not found yet (#295).
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// A folder of its own for a test, empty, outside the repository; and the turn of the test.
struct Scratch(PathBuf, #[expect(dead_code, reason = "held for its drop")] MutexGuard<'static, ()>);

impl Scratch {
    fn new(name: &str) -> Scratch {
        let turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
        let path = std::env::temp_dir().join(format!("labcraft-attach-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(path.join("project")).unwrap();
        std::fs::create_dir_all(path.join("channel")).unwrap();
        Scratch(path.canonicalize().unwrap(), turn)
    }

    fn project(&self) -> PathBuf {
        self.0.join("project").canonicalize().unwrap()
    }

    fn channel(&self) -> PathBuf {
        self.0.join("channel")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The session of the stand-in window, which counts the calls it runs.
struct Counted {
    session: Session,
    calls: Arc<AtomicUsize>,
}

impl Executor for Counted {
    fn execute(&mut self, author: &Author, id: &str, params: &Value, waiter: &dyn Waiter) -> Result<Value, CommandError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Executor::execute(&mut self.session, author, id, params, waiter)
    }

    fn closed(&mut self, served: &Served) {
        self.session.closed(served);
    }
}

fn session(root: &Path) -> Session {
    let config = Config {
        root: root.to_path_buf(),
        lib_paths: Vec::new(),
        limits: labcraft_types::Limits::default(),
        shipped: labcraft_cli::shipped(),
        natives: Vec::new(),
        clock: Arc::new(labcraft_cli::clock::Real::new()),
        process: std::process::id(),
    };
    Session::new(Arc::new(labcraft_cli::disk::Disk), config, false).unwrap()
}

/// A stand-in window of the project at `root`, its file in `channel`: serves one connection, and counts its calls.
fn window(channel: &Path, root: &Path) -> (attach::Published, Arc<AtomicUsize>, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let published = attach::publish(channel, root, listener.local_addr().unwrap().port()).unwrap();
    let token = published.token().to_owned();
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let root = root.to_path_buf();
    let thread = std::thread::Builder::new()
        .stack_size(labcraft_format::STACK_SIZE)
        .spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            if attach::greet(&mut stream, &token, || Ok(())).is_err() {
                return;
            }
            let session = session(&root);
            let served = Served::of(&session, true);
            let mut executor = Counted { session, calls: counted };
            let output = stream.try_clone().unwrap();
            let options = labcraft_mcp::Options { viewer: true, version: "window".to_owned(), max_request: labcraft_mcp::MAX_REQUEST, note: None };
            labcraft_mcp::serve_with(&mut executor, &served, &Open, &mut BufReader::new(stream), Box::new(output), &options);
        })
        .unwrap();
    (published, calls, thread)
}

/// `labcraft-cli mcp` in `project`, its channel in `channel`, given `lines` on stdin: its exit code, what it wrote on
/// stdout, as messages, and on stderr.
fn mcp(project: &Path, channel: &Path, lines: &[Value]) -> (Option<i32>, Vec<Value>, String) {
    mcp_with(project, channel, &[], lines)
}

/// As [`mcp`], with the options `args`.
fn mcp_with(project: &Path, channel: &Path, args: &[&str], lines: &[Value]) -> (Option<i32>, Vec<Value>, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_labcraft-cli"))
        .env(attach::FOLDER_VARIABLE, channel)
        .current_dir(project)
        .arg("mcp")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut messages = Vec::new();
    // Each answer before the next request: the server drops what waits when stdin closes.
    for line in lines {
        // A server that stopped reads nothing.
        if writeln!(stdin, "{line}").and_then(|()| stdin.flush()).is_err() {
            break;
        }
        let mut answer = String::new();
        if stdout.read_line(&mut answer).unwrap() == 0 {
            break;
        }
        messages.push(serde_json::from_str::<Value>(&answer).unwrap());
    }
    drop(stdin);
    let mut rest = String::new();
    std::io::Read::read_to_string(&mut stdout, &mut rest).unwrap();
    assert!(rest.is_empty(), "nothing after the last answer: {rest}");
    let output = child.wait_with_output().unwrap();
    (output.status.code(), messages, String::from_utf8_lossy(&output.stderr).into_owned())
}

fn initialize() -> Value {
    json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": { "name": "tester", "version": "1" } } })
}

fn call(id: i64, tool: &str, arguments: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call", "params": { "name": tool, "arguments": arguments } })
}

#[test]
fn mcp_attaches_to_the_window_of_its_project_and_relays_both_ways() {
    let scratch = Scratch::new("relay");
    let (published, calls, window) = window(&scratch.channel(), &scratch.project());
    let lines = [
        initialize(),
        call(2, "file_write", json!({ "path": "f.lcf", "text": "labcraft 1\n\nfn f() -> () {}\n" })),
        call(3, "file_read", json!({ "path": "f.lcf" })),
    ];
    let (code, messages, stderr) = mcp(&scratch.project(), &scratch.channel(), &lines);
    assert_eq!(code, Some(0), "{stderr}");
    window.join().unwrap();
    // The server that answered is the window's: its version, and the calls it ran.
    assert_eq!(messages[0]["result"]["serverInfo"]["version"], "window", "{messages:?}");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(messages[2]["result"]["structuredContent"]["text"], "labcraft 1\n\nfn f() -> () {}\n", "{messages:?}");
    drop(published);
}

/// Attached, the window asks for real hardware (io.md 4): the rights given on the command line are not used, and the
/// server says so on stderr (decision of the owner, 2026-11-24).
#[test]
fn rights_given_to_a_server_that_attaches_are_not_used_and_it_says_so() {
    let scratch = Scratch::new("rights");
    let (published, _, window) = window(&scratch.channel(), &scratch.project());
    let (code, messages, stderr) = mcp_with(&scratch.project(), &scratch.channel(), &["--allow-hardware", "tcp:10.0.0.5:502"], &[initialize()]);
    assert_eq!(code, Some(0), "{stderr}");
    window.join().unwrap();
    assert_eq!(messages[0]["result"]["serverInfo"]["version"], "window", "{messages:?}");
    assert!(stderr.contains("--allow-hardware and --allow-path are not used"), "{stderr}");
    drop(published);
}

#[test]
fn a_file_left_over_from_a_window_gone_starts_the_server_headless() {
    let scratch = Scratch::new("left-over");
    // A port where nothing listens any more.
    let port = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap().local_addr().unwrap().port();
    let published = attach::publish(&scratch.channel(), &scratch.project(), port).unwrap();
    let (code, messages, stderr) = mcp(&scratch.project(), &scratch.channel(), &[initialize()]);
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(messages[0]["result"]["serverInfo"]["version"], env!("CARGO_PKG_VERSION"), "{messages:?}");
    drop(published);
}

#[test]
fn the_window_of_another_project_is_not_attached_to() {
    let scratch = Scratch::new("other");
    let other = scratch.0.join("other");
    std::fs::create_dir_all(&other).unwrap();
    let (published, calls, _window) = window(&scratch.channel(), &other.canonicalize().unwrap());
    let (code, messages, stderr) = mcp(&scratch.project(), &scratch.channel(), &[initialize()]);
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(messages[0]["result"]["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    drop(published);
}

#[test]
fn a_window_that_refuses_the_token_stops_the_server() {
    let scratch = Scratch::new("token");
    let (published, calls, window) = window(&scratch.channel(), &scratch.project());
    // The file says another token than the window's.
    let path = published.path().to_path_buf();
    let mut entry = attach::read_entry(&path).unwrap();
    entry.token = "0".repeat(64);
    std::fs::write(&path, serde_json::to_string(&entry).unwrap()).unwrap();
    let (code, messages, stderr) = mcp(&scratch.project(), &scratch.channel(), &[initialize()]);
    window.join().unwrap();
    assert_eq!(code, Some(2), "{stderr}");
    assert!(messages.is_empty());
    assert!(stderr.contains("does not hold the token of the window"), "{stderr}");
    assert!(stderr.contains("remove `"), "says which file to remove: {stderr}");
    assert_eq!(stderr.lines().count(), 1, "the reason alone, not the usage: {stderr}");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

/// What takes the connection and answers as a window, without the token: the server stops, and the token never goes
/// on the connection.
#[test]
fn a_process_that_does_not_hold_the_token_gets_nothing() {
    let scratch = Scratch::new("impostor");
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let published = attach::publish(&scratch.channel(), &scratch.project(), listener.local_addr().unwrap().port()).unwrap();
    let token = published.token().to_owned();
    let impostor = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut writer = stream.try_clone().unwrap();
        writeln!(writer, "{}", json!({ "ok": true, "proof": "0".repeat(64), "nonce": "1".repeat(64) })).unwrap();
        let mut heard = String::new();
        let _ = std::io::Read::read_to_string(&mut BufReader::new(stream), &mut heard);
        heard
    });
    let (code, messages, stderr) = mcp(&scratch.project(), &scratch.channel(), &[initialize()]);
    let heard = impostor.join().unwrap();
    assert_eq!(code, Some(2), "{stderr}");
    assert!(messages.is_empty());
    assert!(stderr.contains("does not hold the token of the window"), "{stderr}");
    assert!(!heard.contains(&token), "the token never goes on the connection: {heard}");
    drop(published);
}

/// The window closes while the agent is attached: `mcp` ends at once, exit 1, without waiting for stdin.
#[test]
fn mcp_ends_when_the_window_closes() {
    let scratch = Scratch::new("closed");
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let published = attach::publish(&scratch.channel(), &scratch.project(), listener.local_addr().unwrap().port()).unwrap();
    let token = published.token().to_owned();
    let window = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        attach::greet(&mut stream, &token, || Ok(())).unwrap();
        stream.shutdown(std::net::Shutdown::Both).unwrap();
    });
    let mut child = Command::new(env!("CARGO_BIN_EXE_labcraft-cli"))
        .env(attach::FOLDER_VARIABLE, scratch.channel())
        .current_dir(scratch.project())
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    window.join().unwrap();
    // Stdin stays open: the process ends by itself.
    let _stdin = child.stdin.take();
    let mut ended = None;
    for _ in 0..200 {
        if let Some(status) = child.try_wait().unwrap() {
            ended = Some(status);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    let status = ended.unwrap_or_else(|| {
        let _ = child.kill();
        panic!("mcp still runs after the window closed")
    });
    assert_eq!(status.code(), Some(1));
    drop(published);
}
