//! The server on a session in memory, through pipes: the protocol (`docs/mcp.md`, 1 to 3), the bounds of section 5, the
//! queue, progress and cancellation (section 4), and the end of the input. Expected values are written by hand from
//! mcp.md, commands.md and the programs below.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::io::{BufReader, Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use labcraft_engine::workspace::Workspace;
use labcraft_engine::{Config, Memory, Runs, Session, Shipped, Still};
use labcraft_types::Limits;
use serde_json::{Value as Json, json};

use crate::{Options, serve};

/// Counts until it is aborted: `0.0 >= m` never holds once `m` is 1.
const FOREVER: &str = "labcraft 1\n\nuse core\n\n/// Counts for ever.\nfn forever() -> (n: f64) {\n    n = loop carry i = 0.0 {\n        m = core.add(i, 1.0)\n        done = core.ge(0.0, m)\n        next i = m\n        until done\n    }\n}\n";
const DOUBLE: &str = "labcraft 1\n\nuse core\n\n/// Doubles a number.\nfn double(x: f64) -> (y: f64) {\n    y = core.mul(x, 2)\n}\n";

/// The input of the server: what a test sends, line by line; dropping the sender closes it.
struct Pipe {
    lines: Receiver<Vec<u8>>,
    held: Vec<u8>,
    at: usize,
}

impl Read for Pipe {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.at >= self.held.len() {
            match self.lines.recv() {
                Ok(bytes) => {
                    self.held = bytes;
                    self.at = 0;
                }
                Err(_) => return Ok(0),
            }
        }
        let rest = &self.held[self.at..];
        let n = rest.len().min(out.len());
        out[..n].copy_from_slice(&rest[..n]);
        self.at += n;
        Ok(n)
    }
}

/// The output of the server, which a test waits on.
#[derive(Clone, Default)]
struct Out(Arc<(Mutex<Vec<u8>>, Condvar)>);

impl Write for Out {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.0.lock().unwrap().extend_from_slice(bytes);
        self.0.1.notify_all();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Out {
    fn messages(&self) -> Vec<Json> {
        let bytes = self.0.0.lock().unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(text.is_empty() || text.ends_with('\n'), "a message is a whole line: {text}");
        text.lines().map(|line| serde_json::from_str(line).unwrap()).collect()
    }

    /// Waits until a message passes `wanted`, up to 20 s.
    #[expect(clippy::disallowed_methods, reason = "the server is never built for WASM; its tests run natively")]
    fn wait_for(&self, wanted: impl Fn(&Json) -> bool) -> Json {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let mut bytes = self.0.0.lock().unwrap();
        loop {
            let text = String::from_utf8_lossy(&bytes).into_owned();
            if let Some(found) = text.lines().filter_map(|line| serde_json::from_str::<Json>(line).ok()).find(|message| wanted(message)) {
                return found;
            }
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            assert!(!left.is_zero(), "no such message came: {text}");
            bytes = self.0.1.wait_timeout(bytes, left).unwrap().0;
        }
    }

    fn answer(&self, id: i64) -> Json {
        self.wait_for(|message| message["id"] == id && (message.get("result").is_some() || message.get("error").is_some()))
    }
}

struct Server {
    send: Option<Sender<Vec<u8>>>,
    out: Out,
    runs: Runs,
    ws: Arc<dyn Workspace>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    fn start(max_request: usize) -> Server {
        let memory = Memory::new();
        memory.put("/p/forever.lcf", FOREVER);
        memory.put("/p/double.lcf", DOUBLE);
        let ws: Arc<dyn Workspace> = Arc::new(memory);
        let config = Config {
            root: PathBuf::from("/p"),
            lib_paths: Vec::new(),
            limits: Limits::default(),
            shipped: Shipped::default(),
            natives: Vec::new(),
            clock: Arc::new(Still),
            process: 9,
        };
        let session = Session::new(Arc::clone(&ws), config, false).unwrap();
        let runs = session.runs();
        let (send, lines) = channel();
        let out = Out::default();
        let written = out.clone();
        let thread = std::thread::Builder::new()
            .stack_size(labcraft_format::STACK_SIZE)
            .spawn(move || {
                let mut input = BufReader::new(Pipe { lines, held: Vec::new(), at: 0 });
                let options = Options { viewer: false, version: "0.0.0-test".to_owned(), max_request };
                serve(session, &mut input, Box::new(written), &options);
            })
            .unwrap();
        Server { send: Some(send), out, runs, ws, thread: Some(thread) }
    }

    fn line(&self, text: &str) {
        self.send.as_ref().unwrap().send(format!("{text}\n").into_bytes()).unwrap();
    }

    fn request(&self, id: i64, method: &str, params: Json) {
        self.line(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }).to_string());
    }

    fn call(&self, id: i64, tool: &str, arguments: Json) {
        self.request(id, "tools/call", json!({ "name": tool, "arguments": arguments }));
    }

    /// Closes the input and waits for the server to end.
    fn end(&mut self) {
        self.send = None;
        self.thread.take().unwrap().join().unwrap();
    }

    fn status(&self) -> Json {
        self.runs.execute(&*self.ws, std::path::Path::new("/p"), "run.status", &json!({})).unwrap()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if self.thread.is_some() {
            self.end();
        }
    }
}

#[test]
fn initialize_gives_the_revision_asked_for_or_the_latest() {
    let server = Server::start(crate::MAX_REQUEST);
    server.request(1, "initialize", json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "t", "version": "1" } }));
    let first = server.out.answer(1)["result"].clone();
    assert_eq!(first["protocolVersion"], "2025-06-18");
    assert_eq!(first["capabilities"], json!({ "tools": { "listChanged": false } }));
    assert_eq!(first["serverInfo"]["name"], "labcraft");
    assert_eq!(first["serverInfo"]["version"], "0.0.0-test");
    let instructions = first["instructions"].as_str().unwrap();
    assert!(instructions.contains("`functions_list`") && instructions.contains("`run_start`"), "{instructions}");
    assert!(instructions.contains("Loop: `guide_read` once, `functions_list`"), "{instructions}");
    assert!(!instructions.contains("examples"), "scenario 1: the agent is not pointed to the answer");
    server.request(2, "initialize", json!({ "protocolVersion": "2025-03-26" }));
    assert_eq!(server.out.answer(2)["result"]["protocolVersion"], "2025-11-25");
    server.request(3, "initialize", json!({}));
    assert_eq!(server.out.answer(3)["result"]["protocolVersion"], "2025-11-25");
}

#[test]
fn the_tools_are_the_enabled_commands_without_the_viewer() {
    let server = Server::start(crate::MAX_REQUEST);
    server.request(1, "tools/list", json!({}));
    let tools = server.out.answer(1)["result"]["tools"].as_array().unwrap().clone();
    let names: Vec<&str> = tools.iter().map(|tool| tool["name"].as_str().unwrap()).collect();
    // commands.md 6 in its order, less `view.*` (decision of the owner, 2026-11-11).
    assert_eq!(
        names,
        [
            "guide_read",
            "functions_list",
            "project_describe",
            "file_read",
            "file_write",
            "file_rename",
            "file_delete",
            "library_check",
            "file_format",
            "simulation_describe",
            "run_start",
            "run_wait",
            "run_status",
            "run_set_element",
            "run_abort",
            "run_trace",
            "diagram_svg",
            "commands_list",
            "commands_describe",
        ]
    );
    let tool = |name: &str| tools.iter().find(|tool| tool["name"] == name).unwrap().clone();
    let hints = |name: &str| {
        let annotations = tool(name)["annotations"].clone();
        ["readOnlyHint", "destructiveHint", "idempotentHint", "openWorldHint"].map(|hint| annotations[hint].as_bool().unwrap())
    };
    assert_eq!(hints("guide_read"), [true, false, false, false]);
    assert_eq!(hints("functions_list"), [true, false, false, false]);
    assert_eq!(hints("file_write"), [false, true, false, false]);
    assert_eq!(hints("file_format"), [false, true, false, false]);
    assert_eq!(hints("run_start"), [false, false, false, true]);
    assert_eq!(hints("run_abort"), [false, false, false, false]);
    for name in names.iter().filter(|name| **name != "run_start") {
        assert!(!hints(name)[3], "{name}: only `run_start` reaches instruments");
    }
    let write = tool("file_write");
    assert_eq!(write["inputSchema"]["type"], "object");
    assert_eq!(write["inputSchema"]["required"], json!(["path", "text"]));
    assert_eq!(write["inputSchema"]["additionalProperties"], false);
    assert!(write["description"].as_str().unwrap().len() > 40);
}

#[test]
fn a_call_gives_its_object_twice_and_a_failed_one_is_an_error_the_model_reads() {
    let server = Server::start(crate::MAX_REQUEST);
    server.call(1, "file_read", json!({ "path": "double.lcf" }));
    let result = server.out.answer(1)["result"].clone();
    assert_eq!(result["isError"], false);
    assert_eq!(result["structuredContent"]["text"], DOUBLE, "{result}");
    let text: Json = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text, result["structuredContent"]);
    assert_eq!(result["content"][0]["type"], "text");
    // A wrong parameter is a failed call, not a protocol error (SEP-1303).
    server.call(2, "file_read", json!({ "path": "double.lcf", "nope": 1 }));
    let failed = server.out.answer(2)["result"].clone();
    assert_eq!(failed["isError"], true, "{failed}");
    // commands.md 3: a parameter the command does not have.
    assert_eq!(failed["structuredContent"]["error"]["code"], "params.unknown");
    assert_eq!(failed["structuredContent"]["error"]["parameter"], "nope");
    server.call(3, "file_read", json!([]));
    assert_eq!(server.out.answer(3)["result"]["isError"], true, "arguments that are no object");
    server.call(4, "file_read", json!({ "path": "missing.lcf" }));
    assert_eq!(server.out.answer(4)["result"]["structuredContent"]["error"]["code"], "path.not_found");
}

#[test]
fn guide_read_starts_an_agent_with_the_skills_built_in() {
    let server = Server::start(crate::MAX_REQUEST);
    server.call(1, "guide_read", json!({}));
    let first = server.out.answer(1)["result"]["structuredContent"].clone();
    assert_eq!(first["skill"], "labcraft-writing", "{first}");
    assert!(first["text"].as_str().unwrap().contains("## The loop"));
    assert!(first["skills"].as_array().unwrap().len() >= 3);
    server.call(2, "guide_read", json!({ "skill": "labcraft-testing" }));
    let testing = server.out.answer(2)["result"]["structuredContent"].clone();
    let text = testing["text"].as_str().unwrap();
    assert!(text.starts_with("---\nname: labcraft-testing\n"), "{testing}");
    server.call(3, "guide_read", json!({ "skill": "examples" }));
    let refused = server.out.answer(3)["result"].clone();
    assert_eq!((refused["isError"].as_bool(), &refused["structuredContent"]["error"]["code"]), (Some(true), &json!("params.invalid")));
}

#[test]
fn protocol_errors_have_the_codes_of_json_rpc_and_the_server_goes_on() {
    let server = Server::start(crate::MAX_REQUEST);
    server.line("{ not json");
    server.line("[{\"jsonrpc\": \"2.0\", \"id\": 1, \"method\": \"ping\"}]");
    server.line("\"ping\"");
    server.line("{\"jsonrpc\": \"2.0\", \"id\": true, \"method\": \"ping\"}");
    server.line("{\"jsonrpc\": \"1.0\", \"id\": 5, \"method\": \"ping\"}");
    server.line("{\"jsonrpc\": \"2.0\", \"id\": 6}");
    server.line("{\"jsonrpc\": \"2.0\", \"id\": 7, \"method\": \"ping\", \"id\": 8}");
    server.request(9, "resources/list", json!({}));
    server.call(10, "view_open", json!({ "folder": "." }));
    server.call(11, "file_raed", json!({ "path": "double.lcf" }));
    server.request(12, "tools/call", json!({ "arguments": {} }));
    // No answer to a notification, even a wrong one, nor to the answer of a client.
    server.line("{\"jsonrpc\": \"2.0\", \"method\": \"notifications/initialized\"}");
    server.line("{\"jsonrpc\": \"2.0\", \"method\": \"no/such\"}");
    server.line("{\"jsonrpc\": \"2.0\", \"id\": 99, \"result\": {}}");
    server.line("");
    server.request(13, "ping", json!({}));
    assert_eq!(server.out.answer(13)["result"], json!({}));
    let messages = server.out.messages();
    let codes: Vec<(Json, i64)> = messages
        .iter()
        .filter(|message| message.get("error").is_some())
        .map(|message| (message["id"].clone(), message["error"]["code"].as_i64().unwrap()))
        .collect();
    assert_eq!(
        codes,
        [
            (Json::Null, -32700),
            (Json::Null, -32600),
            (Json::Null, -32600),
            (Json::Null, -32600),
            (json!(5), -32600),
            (json!(6), -32600),
            (Json::Null, -32700),
            (json!(9), -32601),
            (json!(10), -32602),
            (json!(11), -32602),
            (json!(12), -32602),
        ]
    );
    assert!(messages.iter().all(|message| message["jsonrpc"] == "2.0"));
    let near = messages.iter().find(|message| message["id"] == 11).unwrap();
    assert!(near["error"]["message"].as_str().unwrap().contains("`file_read`"), "{near}");
    assert_eq!(messages.len(), 12, "nothing else was answered: {messages:?}");
}

#[test]
fn a_request_too_large_or_too_deep_is_refused_unread_and_the_next_is_read() {
    let server = Server::start(64);
    server.line(&format!("{{\"jsonrpc\": \"2.0\", \"id\": 1, \"method\": \"ping\", \"pad\": \"{}\"}}", "x".repeat(100)));
    server.request(2, "ping", json!({}));
    assert_eq!(server.out.answer(2)["result"], json!({}));
    let refused = server.out.messages()[0].clone();
    assert_eq!((refused["id"].clone(), refused["error"]["code"].clone()), (Json::Null, json!(-32600)), "{refused}");
    drop(server);
    let server = Server::start(crate::MAX_REQUEST);
    // `--max-nesting` 256 by default, and 16 more: 272 levels pass, 273 do not. The envelope takes 1.
    let nested = |levels: usize| {
        format!(
            "{{\"jsonrpc\": \"2.0\", \"id\": 3, \"method\": \"ping\", \"params\": {{\"x\": {}{}}}}}",
            "[".repeat(levels - 2),
            "]".repeat(levels - 2)
        )
    };
    server.line(&nested(273));
    server.line(&nested(272));
    assert_eq!(server.out.answer(3)["result"], json!({}));
    let first = server.out.messages()[0].clone();
    assert_eq!((first["id"].clone(), first["error"]["code"].clone()), (Json::Null, json!(-32600)), "{first}");
}

#[test]
fn a_run_is_followed_and_stopped_while_run_start_waits() {
    let server = Server::start(crate::MAX_REQUEST);
    let forever = json!({ "function": "forever", "max_iterations": null, "max_nodes": null, "wait": "300 s", "trace": "none" });
    server.call(1, "run_start", forever);
    // Queued behind it: answered only once it is.
    server.call(2, "file_read", json!({ "path": "double.lcf" }));
    let status = loop {
        server.call(3, "run_status", json!({}));
        let answer = server.out.answer(3);
        if answer["result"]["isError"] == false {
            break answer;
        }
        // `run.unknown` until the worker has started the run: asked again, then the answers are dropped.
        server.out.0.0.lock().unwrap().clear();
    };
    assert_eq!(status["result"]["structuredContent"]["status"], "running", "{status}");
    server.request(4, "ping", json!({}));
    assert_eq!(server.out.answer(4)["result"], json!({}));
    assert!(server.out.messages().iter().all(|message| message["id"] != 1 && message["id"] != 2), "run_start still waits");
    server.call(5, "run_abort", json!({}));
    assert_eq!(server.out.answer(5)["result"]["structuredContent"]["status"], "aborted");
    assert_eq!(server.out.answer(1)["result"]["structuredContent"]["status"], "aborted");
    assert_eq!(server.out.answer(2)["result"]["structuredContent"]["text"], DOUBLE);
}

#[test]
fn seventeen_requests_waiting_make_the_server_busy() {
    let server = Server::start(crate::MAX_REQUEST);
    server.call(1, "run_start", json!({ "function": "forever", "max_iterations": null, "max_nodes": null, "wait": "300 s", "trace": "none" }));
    // Once the worker holds `run_start`, the queue is empty: wait for the run.
    while server.runs.execute(&*server.ws, std::path::Path::new("/p"), "run.status", &json!({})).is_err() {
        std::thread::sleep(Duration::from_millis(5));
    }
    for id in 100..=116 {
        server.call(id, "file_read", json!({ "path": "double.lcf" }));
    }
    let busy = server.out.answer(116);
    assert_eq!(busy["error"]["code"], -32000, "{busy}");
    assert!(busy["error"]["message"].as_str().unwrap().contains("busy"));
    server.call(2, "run_abort", json!({}));
    for id in 100..=115 {
        assert_eq!(server.out.answer(id)["result"]["isError"], false, "{id}");
    }
}

#[test]
fn a_waiting_run_start_tells_its_progress_each_second() {
    let server = Server::start(crate::MAX_REQUEST);
    let params = json!({
        "name": "run_start",
        "arguments": { "function": "forever", "max_iterations": null, "max_nodes": null, "wait": "2500 ms", "trace": "none" },
        "_meta": { "progressToken": "p1" },
    });
    server.request(1, "tools/call", params);
    let answer = server.out.answer(1);
    assert_eq!(answer["result"]["structuredContent"]["status"], "running", "after 2.5 s, the run goes on");
    let messages = server.out.messages();
    let progress: Vec<&Json> = messages.iter().filter(|message| message["method"] == "notifications/progress").collect();
    // At 1 s and at 2 s; none after the answer.
    assert_eq!(progress.len(), 2, "{messages:?}");
    assert_eq!(progress.iter().map(|message| message["params"]["progress"].as_u64().unwrap()).collect::<Vec<_>>(), [1, 2]);
    assert!(progress.iter().all(|message| message["params"]["progressToken"] == "p1"));
    let message = progress[0]["params"]["message"].as_str().unwrap();
    assert!(message.contains("simulated") && message.contains("loop `n` of `forever`"), "{message}");
    assert_eq!(messages.last().unwrap()["id"], 1, "the answer comes after its notifications");
    // Without a token, no notification.
    server.call(2, "run_wait", json!({ "wait": "1500 ms" }));
    server.out.answer(2);
    assert_eq!(server.out.messages().iter().filter(|message| message["method"] == "notifications/progress").count(), 2);
    server.call(3, "run_abort", json!({}));
    server.out.answer(3);
}

#[test]
fn a_cancelled_wait_gets_no_answer_and_the_run_goes_on() {
    let server = Server::start(crate::MAX_REQUEST);
    server.call(1, "run_start", json!({ "function": "forever", "max_iterations": null, "max_nodes": null, "wait": "300 s", "trace": "none" }));
    server.call(2, "file_read", json!({ "path": "double.lcf" }));
    server.call(3, "file_read", json!({ "path": "forever.lcf" }));
    while server.runs.execute(&*server.ws, std::path::Path::new("/p"), "run.status", &json!({})).is_err() {
        std::thread::sleep(Duration::from_millis(5));
    }
    // One cancelled as it waits in the queue, then the one going.
    server.line(&json!({ "jsonrpc": "2.0", "method": "notifications/cancelled", "params": { "requestId": 2 } }).to_string());
    server.line(&json!({ "jsonrpc": "2.0", "method": "notifications/cancelled", "params": { "requestId": 1, "reason": "test" } }).to_string());
    assert_eq!(server.out.answer(3)["result"]["structuredContent"]["text"], FOREVER, "the worker is free again");
    server.request(4, "ping", json!({}));
    server.out.answer(4);
    let ids: Vec<Json> = server.out.messages().iter().map(|message| message["id"].clone()).collect();
    assert_eq!(ids, [json!(3), json!(4)], "no answer to the cancelled requests");
    assert_eq!(server.status()["status"], "running", "a cancellation stops the wait, not the run");
    server.call(5, "run_abort", json!({}));
    assert_eq!(server.out.answer(5)["result"]["structuredContent"]["status"], "aborted");
}

#[test]
fn the_end_of_the_input_aborts_the_run_and_waits_for_its_report() {
    let mut server = Server::start(crate::MAX_REQUEST);
    server.call(1, "run_start", json!({ "function": "forever", "max_iterations": null, "max_nodes": null, "wait": "0 s", "trace": "none" }));
    assert_eq!(server.out.answer(1)["result"]["structuredContent"]["status"], "running");
    server.call(2, "run_wait", json!({ "wait": "300 s" }));
    server.end();
    let status = server.status();
    assert_eq!(status["status"], "aborted", "{status}");
    assert_eq!(status["abort"]["by"], "request");
    assert!(server.out.messages().iter().all(|message| message["id"] != 2), "the wait going gets no answer");
}

#[test]
fn a_line_of_spaces_and_a_crlf_are_read_as_nothing_and_as_a_feed() {
    let server = Server::start(crate::MAX_REQUEST);
    server.send.as_ref().unwrap().send(b"   \r\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\r\n".to_vec()).unwrap();
    assert_eq!(server.out.answer(1)["result"], json!({}));
    assert_eq!(server.out.messages().len(), 1);
}

#[test]
fn the_requests_waiting_hold_at_most_the_bytes_of_one_request() {
    // A bound of 4,096 bytes: each call below takes about 1,500 bytes as text, so two wait and the third is refused.
    let server = Server::start(4096);
    server.call(1, "run_start", json!({ "function": "forever", "max_iterations": null, "max_nodes": null, "wait": "300 s", "trace": "none" }));
    while server.runs.execute(&*server.ws, std::path::Path::new("/p"), "run.status", &json!({})).is_err() {
        std::thread::sleep(Duration::from_millis(5));
    }
    let pad = "x".repeat(1400);
    for id in 10..=12 {
        server.call(id, "file_read", json!({ "path": "double.lcf", "pad": pad }));
    }
    let busy = server.out.answer(12);
    assert_eq!(busy["error"]["code"], -32000, "{busy}");
    assert!(busy["error"]["message"].as_str().unwrap().contains("4096 bytes in all"), "{busy}");
    server.call(2, "run_abort", json!({}));
    // The two that waited are read again by the worker, and judged: `pad` is no parameter of `file.read`.
    for id in [10, 11] {
        assert_eq!(server.out.answer(id)["result"]["structuredContent"]["error"]["parameter"], "pad", "{id}");
    }
    // Answered, they wait no more: a request as large again waits its turn.
    server.call(13, "file_read", json!({ "path": "double.lcf", "pad": pad }));
    assert_eq!(server.out.answer(13)["result"]["isError"], true);
}
