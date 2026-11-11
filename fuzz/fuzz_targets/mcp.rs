//! On any input, the MCP server reads it to its end and stops: never a panic, and every line it writes is a message of
//! JSON-RPC 2.0, an answer with the `id` of a request it read (or null for a line it could not read as one), or a
//! notification (`docs/mcp.md`). Scenario 4, family MCP: malformed JSON-RPC, unknown tools, parameters of the wrong type.
//!
//! The input is what a client writes on stdin, one request per line, each request waiting for its answer before the next
//! line, as a client does: at the end of its input the server drops the requests that wait (mcp.md 4), so that an input
//! given at once would run hardly any tool. The waits have 5 s in all for one input. The session holds a small project in memory, a
//! function that counts and one that doubles, with the shipped libraries and the simulated instruments of `visa`, so that
//! the tools have something to check, list and run. Requests are bounded to 64 KiB, the input of libFuzzer, instead of
//! 16 MiB. It runs on the stack of a host (`format::STACK_SIZE`), as the program does.
#![no_main]

use std::io::{BufReader, Read, Write};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use labcraft_engine::{Config, Memory, Session, Shipped, ShippedFile, ShippedLibrary, Still};
use labcraft_format::STACK_SIZE;
use labcraft_mcp::{Options, serve};
use labcraft_types::Limits;
use libfuzzer_sys::fuzz_target;
use serde_json::Value as Json;

const COUNT: &str = "labcraft 1\n\nuse core\n\n/// Counts to 10.\nfn count() -> (n: f64) {\n    n = loop carry i = 0.0 {\n        m = core.add(i, 1.0)\n        done = core.ge(m, 10.0)\n        next i = m\n        until done\n    }\n}\n";
const DOUBLE: &str = "labcraft 1\n\nuse core\n\n/// Doubles a number.\nfn double(x: f64) -> (y: f64) {\n    y = core.mul(x, 2)\n}\n";

fn shipped() -> Shipped {
    let files = |files: &'static [labcraft_libs::File]| files.iter().map(|file| ShippedFile { name: file.name, bytes: file.bytes }).collect();
    Shipped::new(
        labcraft_libs::LIBRARIES
            .iter()
            .map(|library| ShippedLibrary {
                name: library.name,
                manifest: library.manifest,
                files: files(library.files),
                simulations: files(library.simulations),
            })
            .collect(),
    )
}

/// What the server writes, kept for the checks after it stops, and the answers it gave so far, which the input waits for.
#[derive(Clone, Default)]
struct Out(Arc<(Mutex<Written>, Condvar)>);

#[derive(Default)]
struct Written {
    bytes: Vec<u8>,
    /// Where the line that is not ended yet starts: the lines before it are counted.
    counted: usize,
    /// The lines that answer a request, with an `id` that is not null.
    answers: usize,
}

impl Write for Out {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let (written, changed) = &*self.0;
        let mut written = written.lock().map_err(|_| std::io::Error::other("poisoned"))?;
        let before = written.bytes.len();
        written.bytes.extend_from_slice(bytes);
        // Only the lines this write ended are read, each once: a long output costs no more than its length.
        let start = written.counted;
        let Some(end) = bytes.iter().rposition(|byte| *byte == b'\n').map(|at| before + at) else { return Ok(bytes.len()) };
        let ended = written.bytes.get(start..end).map(|lines| String::from_utf8_lossy(lines).lines().filter(|line| answers_a_request(line)).count());
        written.answers += ended.unwrap_or(0);
        written.counted = end + 1;
        changed.notify_all();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn answers_a_request(line: &str) -> bool {
    serde_json::from_str::<Json>(line).is_ok_and(|message| message.get("method").is_none() && message.get("id").is_some_and(|id| !id.is_null()))
}

/// The id of a line that the server reads as a request and answers, with that id: a JSON object whose `id` is not null,
/// with a `method` or without one (`-32600`, "a request has a `method`").
fn request_id(line: &[u8]) -> Option<Json> {
    let message: Json = serde_json::from_slice(line).ok()?;
    message.as_object()?;
    message.get("id").filter(|id| !id.is_null()).cloned()
}

/// The input of the server, as a client writes it: a line, then, for a request, its answer waited for, within 5 s for the
/// whole input, so that a request that waits long (`run_start` with a long `wait`) cannot make libFuzzer time out. Then the
/// end of the input, which ends the server (`docs/mcp.md`, 4).
struct Client {
    lines: std::vec::IntoIter<Vec<u8>>,
    held: Vec<u8>,
    at: usize,
    out: Out,
    sent: usize,
    deadline: Instant,
}

impl Read for Client {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.at == self.held.len() {
            let (written, changed) = &*self.out.0;
            let mut seen = written.lock().map_err(|_| std::io::Error::other("poisoned"))?;
            while seen.answers < self.sent {
                let Some(left) = self.deadline.checked_duration_since(Instant::now()) else { break };
                seen = changed.wait_timeout(seen, left).map_err(|_| std::io::Error::other("poisoned"))?.0;
            }
            drop(seen);
            let Some(mut line) = self.lines.next() else { return Ok(0) };
            self.sent += usize::from(request_id(&line).is_some());
            line.push(b'\n');
            (self.held, self.at) = (line, 0);
        }
        let rest = self.held.get(self.at..).unwrap_or_default();
        let count = rest.len().min(buffer.len());
        buffer.get_mut(..count).unwrap_or_default().copy_from_slice(rest.get(..count).unwrap_or_default());
        self.at += count;
        Ok(count)
    }
}

fuzz_target!(|bytes: &[u8]| {
    let lines: Vec<Vec<u8>> = bytes.split(|byte| *byte == b'\n').map(<[u8]>::to_vec).collect();
    let ids: Vec<Json> = lines.iter().filter_map(|line| request_id(line)).collect();
    let out = Out::default();
    let written = out.clone();
    let client =
        Client { lines: lines.into_iter(), held: Vec::new(), at: 0, out: out.clone(), sent: 0, deadline: Instant::now() + Duration::from_secs(5) };
    let work = move || {
        let memory = Memory::new();
        memory.put("/p/count.lcf", COUNT);
        memory.put("/p/double.lcf", DOUBLE);
        let config = Config {
            root: "/p".into(),
            lib_paths: Vec::new(),
            limits: Limits::default(),
            shipped: shipped(),
            natives: vec![Arc::new(labcraft_visa::Visa)],
            clock: Arc::new(Still),
            process: 0,
        };
        let session = Session::new(Arc::new(memory) as Arc<dyn labcraft_engine::Workspace>, config, false).expect("a session in memory");
        let options = Options { viewer: false, version: "0.0.0-fuzz".to_owned(), max_request: 65_536 };
        serve(session, &mut BufReader::new(client), Box::new(written), &options);
    };
    std::thread::Builder::new().stack_size(STACK_SIZE).spawn(work).expect("a thread").join().expect("the server");
    let text = out.0.0.lock().expect("the output").bytes.clone();
    let text = String::from_utf8(text).expect("the server writes UTF-8");
    for line in text.lines() {
        let message: Json = serde_json::from_str(line).unwrap_or_else(|error| panic!("a line that is not JSON ({error}): {line}"));
        assert_eq!(message["jsonrpc"], "2.0", "{line}");
        let answer = message.get("result").is_some() || message.get("error").is_some();
        assert!(answer != message.get("method").is_some(), "neither an answer nor a notification: {line}");
        if answer {
            let id = message.get("id").unwrap_or_else(|| panic!("an answer without an id: {line}"));
            // An answer has the id of a request it read, or null for a line it could not read as one (JSON-RPC 2.0, 5).
            assert!(id.is_null() || ids.contains(id), "an answer to no request: {line}");
        }
    }
});
