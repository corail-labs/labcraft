//! L7: the MCP server (`docs/mcp.md`; `docs/architecture.md`, 10.9). JSON-RPC 2.0 over stdio, one message per line, one
//! tool per enabled command of the registry, written by hand over `serde_json`, without an async runtime.
//!
//! **Two threads.** The caller's thread reads requests, bounded as mcp.md 5 says, and answers at once `initialize`,
//! `ping`, `tools/list`, and the tools that act on a run without the session (`run_status`, `run_set_element`,
//! `run_abort`), so that an agent can always stop what it started. Every other tool call waits its turn in a queue of 16,
//! and a worker that holds the session runs them in order. Progress and cancellation are notifications; every message
//! goes out on the same output, one whole line at a time.
//!
//! **The end.** When the input closes, what waits in the queue is dropped, the request going stops waiting, a run still
//! going is aborted, and [`serve`] returns once its final report is written: its resources are released (spec 8.2).
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable,
    clippy::indexing_slicing,
    clippy::string_slice
)]
#![forbid(unsafe_code)]

mod lines;
mod tools;

use std::collections::VecDeque;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use labcraft_engine::workspace::Workspace;
use labcraft_engine::{JsonProblem, Runs, Session, Waiter};
use serde_json::{Map, Value as Json, json};

use lines::{Line, read_line};
use tools::{AT_ONCE, Tool};

/// The revisions of MCP the server speaks, the latest first (`docs/mcp.md`, 1): not 2025-03-26, which requires batches.
pub const PROTOCOLS: [&str; 2] = ["2025-11-25", "2025-06-18"];

/// A request larger than this is refused before it is parsed *(proposed)* (`docs/mcp.md`, 5): above the 4 MiB of a
/// `file_write`, once quoted in JSON.
pub const MAX_REQUEST: usize = 16 * 1024 * 1024;

/// Levels of nesting a request has beyond `--max-nesting` *(proposed)*: the envelope of JSON-RPC and of the call, so that
/// an argument of any type that `check` accepts can be sent.
pub const NESTING_OF_THE_ENVELOPE: u32 = 16;

/// Requests that wait their turn while one runs *(proposed)*: one more gets "server busy".
pub const MAX_WAITING: usize = 16;

/// The codes of JSON-RPC (`docs/mcp.md`, 3), and the one of a server too busy.
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const BUSY: i64 = -32000;

/// What the app tells the server.
pub struct Options {
    /// A viewer serves the session: the `view.*` commands are tools too. Headless in M0b.
    pub viewer: bool,
    /// The name and version the server gives at `initialize`.
    pub version: String,
    /// The bound of a request, in bytes: [`MAX_REQUEST`].
    pub max_request: usize,
}

impl Default for Options {
    fn default() -> Options {
        Options { viewer: false, version: String::new(), max_request: MAX_REQUEST }
    }
}

/// A tool call that waits its turn, or runs. It keeps the text of its request, read again by the worker: a request of
/// many small values takes far more memory read than as text, so the queue holds text (`docs/mcp.md`, 5).
struct Job {
    id: Json,
    command: &'static str,
    line: Vec<u8>,
    progress: Option<Json>,
    cancelled: Arc<AtomicBool>,
}

#[derive(Default)]
struct Queue {
    waiting: VecDeque<Job>,
    /// The bytes of the requests waiting, at most the bound of one request.
    waiting_bytes: usize,
    /// The id of the call that runs, and its flag.
    current: Option<(Json, Arc<AtomicBool>)>,
    closing: bool,
}

/// What the two threads share.
struct Shared<'a> {
    out: Mutex<Box<dyn Write + Send + 'a>>,
    /// The output cannot be written any more: the server ends, as when the input closes.
    broken: AtomicBool,
    queue: Mutex<Queue>,
    ready: Condvar,
    runs: Runs,
}

impl Shared<'_> {
    fn queue(&self) -> MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Writes one message, as one line.
    fn send(&self, message: &Json) {
        let mut line = serde_json::to_string(message).unwrap_or_default();
        line.push('\n');
        let mut out = self.out.lock().unwrap_or_else(PoisonError::into_inner);
        if out.write_all(line.as_bytes()).and_then(|()| out.flush()).is_err() {
            self.broken.store(true, Ordering::SeqCst);
        }
    }

    fn answer(&self, id: &Json, result: Json) {
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "result": result }));
    }

    fn fail(&self, id: &Json, code: i64, message: impl Into<String>) {
        let message: String = message.into();
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }));
    }
}

/// What waits for a run on behalf of a client: told where it is each second when the call carries a `progressToken`,
/// and stopped by a cancellation or by the end of the input.
struct Client<'s, 'a> {
    shared: &'s Shared<'a>,
    cancelled: &'s AtomicBool,
    token: Option<&'s Json>,
    sent: AtomicU64,
}

impl Waiter for Client<'_, '_> {
    fn stopped(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    fn wants_progress(&self) -> bool {
        self.token.is_some()
    }

    fn progress(&self, report: &Json) {
        let Some(token) = self.token else { return };
        let progress = self.sent.fetch_add(1, Ordering::SeqCst) + 1;
        let message = tools::progress_message(report);
        self.shared.send(&json!({
            "jsonrpc": "2.0",
            "method": "notifications/progress",
            "params": { "progressToken": token, "progress": progress, "message": message },
        }));
    }
}

/// Serves `session` on `input` and `output` until the input closes. The caller runs it on a thread of
/// `format::STACK_SIZE`, the stack that reading a request as deep as `--max-nesting` allows takes.
pub fn serve(mut session: Session, input: &mut dyn BufRead, output: Box<dyn Write + Send + '_>, options: &Options) {
    let tools = tools::tools_of(&session, options.viewer);
    let instructions = tools::instructions(&tools);
    let max_nesting = session.config().limits.max_nesting().saturating_add(NESTING_OF_THE_ENVELOPE);
    let ws = session.shared_workspace();
    let root: PathBuf = session.root().to_path_buf();
    let shared = Shared {
        out: Mutex::new(output),
        broken: AtomicBool::new(false),
        queue: Mutex::new(Queue::default()),
        ready: Condvar::new(),
        runs: session.runs(),
    };
    std::thread::scope(|scope| {
        let worker = std::thread::Builder::new()
            .name("mcp session".to_owned())
            .stack_size(labcraft_format::STACK_SIZE)
            .spawn_scoped(scope, || work(&mut session, &shared, max_nesting));
        let reader = Reader { shared: &shared, tools: &tools, instructions: &instructions, max_nesting, ws: &*ws, root: &root, options };
        // Closed even if reading panics, so that the worker ends and the scope can join it.
        let closer = Closer(&shared);
        if worker.is_ok() {
            reader.read(input);
        }
        drop(closer);
        if let Ok(worker) = worker {
            let _ = worker.join();
        }
        // The call going has ended: a run it started, or one that goes on, is aborted, and its final report waited for.
        let _ = shared.runs.execute(&*ws, &root, "run.abort", &json!({}));
    });
}

/// Closes the server as it goes away.
struct Closer<'s, 'a>(&'s Shared<'a>);

impl Drop for Closer<'_, '_> {
    fn drop(&mut self) {
        close(self.0);
    }
}

/// The input closed: nothing more runs.
fn close(shared: &Shared<'_>) {
    let mut queue = shared.queue();
    queue.closing = true;
    queue.waiting.clear();
    queue.waiting_bytes = 0;
    if let Some((_, cancelled)) = &queue.current {
        cancelled.store(true, Ordering::SeqCst);
    }
    drop(queue);
    shared.ready.notify_all();
    shared.runs.wake_waiters();
}

/// The worker: the calls of the queue, in order, on the session.
fn work(session: &mut Session, shared: &Shared<'_>, max_nesting: u32) {
    loop {
        let job = {
            let mut queue = shared.queue();
            loop {
                if queue.closing {
                    return;
                }
                if let Some(job) = queue.waiting.pop_front() {
                    queue.waiting_bytes = queue.waiting_bytes.saturating_sub(job.line.len());
                    queue.current = Some((job.id.clone(), Arc::clone(&job.cancelled)));
                    break job;
                }
                queue = shared.ready.wait(queue).unwrap_or_else(PoisonError::into_inner);
            }
        };
        let client = Client { shared, cancelled: &job.cancelled, token: job.progress.as_ref(), sent: AtomicU64::new(0) };
        let outcome = match arguments_of(&job.line, max_nesting) {
            Some(arguments) => session.execute_waiting(job.command, &arguments, &client),
            None => Err(labcraft_engine::CommandError::internal("a request read once did not read again")),
        };
        shared.queue().current = None;
        // A cancelled request gets no answer (MCP).
        if !job.cancelled.load(Ordering::SeqCst) {
            shared.answer(&job.id, tools::result_of(outcome));
        }
    }
}

/// The `arguments` of a call, read again from its request; `{}` when it has none.
fn arguments_of(line: &[u8], max_nesting: u32) -> Option<Json> {
    let text = std::str::from_utf8(line).ok()?;
    let Json::Object(mut message) = labcraft_engine::read_json(text, max_nesting).ok()? else { return None };
    let Some(Json::Object(mut params)) = message.remove("params") else { return None };
    Some(params.remove("arguments").unwrap_or_else(|| Json::Object(Map::new())))
}

/// The thread that reads.
struct Reader<'r, 'a> {
    shared: &'r Shared<'a>,
    tools: &'r [Tool],
    instructions: &'r str,
    max_nesting: u32,
    ws: &'r dyn Workspace,
    root: &'r std::path::Path,
    options: &'r Options,
}

impl Reader<'_, '_> {
    fn read(&self, input: &mut dyn BufRead) {
        let mut input = input;
        while !self.shared.broken.load(Ordering::SeqCst) {
            match read_line(&mut input, self.options.max_request) {
                Line::End => return,
                Line::TooLong => self.shared.fail(
                    &Json::Null,
                    INVALID_REQUEST,
                    format!("the request is larger than {} bytes, and was dropped unread", self.options.max_request),
                ),
                Line::Text(bytes) => self.take(&bytes),
            }
        }
    }

    /// One line of the input.
    fn take(&self, bytes: &[u8]) {
        if bytes.iter().all(u8::is_ascii_whitespace) {
            return;
        }
        let Ok(text) = std::str::from_utf8(bytes) else {
            return self.shared.fail(&Json::Null, PARSE_ERROR, "the request is not UTF-8");
        };
        let message = match labcraft_engine::read_json(text, self.max_nesting) {
            Ok(message) => message,
            Err(JsonProblem::TooDeep) => {
                let message = format!("the request nests deeper than {} levels (`--max-nesting` and 16)", self.max_nesting);
                return self.shared.fail(&Json::Null, INVALID_REQUEST, message);
            }
            Err(JsonProblem::Malformed(why)) => return self.shared.fail(&Json::Null, PARSE_ERROR, format!("the request is not JSON: {why}")),
        };
        let Json::Object(mut message) = message else {
            let why = if message.is_array() { "a batch of messages is not accepted (MCP from 2025-06-18)" } else { "a message is an object" };
            return self.shared.fail(&Json::Null, INVALID_REQUEST, why);
        };
        let id = message.get("id");
        if let Some(id) = id
            && !(id.is_string() || id.is_number())
        {
            return self.shared.fail(&Json::Null, INVALID_REQUEST, "the `id` of a request is a string or a number");
        }
        let Some(method) = message.get("method").and_then(Json::as_str) else {
            // An answer of the client, to a request that the server never sends: nothing to do.
            if id.is_some() && (message.contains_key("result") || message.contains_key("error")) {
                return;
            }
            return self.shared.fail(id.unwrap_or(&Json::Null), INVALID_REQUEST, "a request has a `method`");
        };
        if message.get("jsonrpc").and_then(Json::as_str) != Some("2.0") {
            // A notification gets no answer, even when it is wrong.
            if let Some(id) = id {
                self.shared.fail(id, INVALID_REQUEST, "a message of JSON-RPC 2.0 has `\"jsonrpc\": \"2.0\"`");
            }
            return;
        }
        let (id, method) = (id.cloned(), method.to_owned());
        let params = message.remove("params").unwrap_or_else(|| Json::Object(Map::new()));
        drop(message);
        match id {
            None => self.notified(&method, &params),
            Some(id) => self.requested(&id, &method, params, bytes),
        }
    }

    /// A notification gets no answer, even when it is wrong.
    fn notified(&self, method: &str, params: &Json) {
        if method == "notifications/cancelled"
            && let Some(request) = params.get("requestId")
        {
            let mut queue = self.shared.queue();
            let before = queue.waiting.len();
            queue.waiting.retain(|job| job.id != *request);
            queue.waiting_bytes = queue.waiting.iter().map(|job| job.line.len()).sum();
            if queue.waiting.len() == before
                && let Some((current, cancelled)) = &queue.current
                && current == request
            {
                cancelled.store(true, Ordering::SeqCst);
                drop(queue);
                self.shared.runs.wake_waiters();
            }
        }
    }

    fn requested(&self, id: &Json, method: &str, params: Json, line: &[u8]) {
        match method {
            "initialize" => {
                let asked = params.get("protocolVersion").and_then(Json::as_str);
                let [latest, _] = PROTOCOLS;
                let version = PROTOCOLS.into_iter().find(|version| Some(*version) == asked).unwrap_or(latest);
                self.shared.answer(
                    id,
                    json!({
                        "protocolVersion": version,
                        "capabilities": { "tools": { "listChanged": false } },
                        "serverInfo": { "name": "labcraft", "title": "LabCraft", "version": self.options.version },
                        "instructions": self.instructions,
                    }),
                );
            }
            "ping" => self.shared.answer(id, json!({})),
            "tools/list" => self.shared.answer(id, json!({ "tools": self.tools.iter().map(tools::listed).collect::<Vec<_>>() })),
            "tools/call" => self.call(id, params, line),
            _ => self.shared.fail(id, METHOD_NOT_FOUND, format!("the server has no method `{}`", labcraft_types::cut_text(method))),
        }
    }

    fn call(&self, id: &Json, mut params: Json, line: &[u8]) {
        let Some(name) = params.get("name").and_then(Json::as_str) else {
            return self.shared.fail(id, INVALID_PARAMS, "`tools/call` names its tool in `name`");
        };
        let Some(tool) = self.tools.iter().find(|tool| tool.name == name) else {
            let names: Vec<&str> = self.tools.iter().map(|tool| tool.name.as_str()).collect();
            let message = format!("no tool is named `{}`{}", labcraft_types::cut_text(name), labcraft_engine::error::did_you_mean(name, &names));
            return self.shared.fail(id, INVALID_PARAMS, message);
        };
        let command = tool.command.id;
        let progress = params.get("_meta").and_then(|meta| meta.get("progressToken")).filter(|token| token.is_string() || token.is_number()).cloned();
        // Wrong arguments are the command's to judge, and come back as a failed call that the model reads (SEP-1303).
        let arguments = params.as_object_mut().and_then(|params| params.remove("arguments"));
        if AT_ONCE.contains(&command) {
            let arguments = arguments.unwrap_or_else(|| Json::Object(Map::new()));
            let outcome = self.shared.runs.execute(self.ws, self.root, command, &arguments);
            return self.shared.answer(id, tools::result_of(outcome));
        }
        // The worker reads the request again: what was read here goes before the request waits.
        drop(arguments);
        drop(params);
        let mut queue = self.shared.queue();
        if queue.waiting.len() >= MAX_WAITING || queue.waiting_bytes.saturating_add(line.len()) > self.options.max_request {
            let held = (queue.waiting.len(), queue.waiting_bytes);
            drop(queue);
            let message = format!(
                "server busy: {} requests of {} bytes wait already, and at most {MAX_WAITING} requests of {} bytes in all wait; send this one again once one is answered",
                held.0, held.1, self.options.max_request
            );
            return self.shared.fail(id, BUSY, message);
        }
        queue.waiting_bytes += line.len();
        queue.waiting.push_back(Job { id: id.clone(), command, line: line.to_vec(), progress, cancelled: Arc::new(AtomicBool::new(false)) });
        drop(queue);
        self.shared.ready.notify_all();
    }
}

#[cfg(test)]
mod tests;
