//! What the libraries of raw streams share (`docs/io.md`, sections 6 and 7; `libs/net/README.md`, "Reading" and "Writing"),
//! so that `net` on a TCP connection and `serial` on a port read and write the same: the reads and writes a program asks for
//! and how they end, what a read needs of the bytes that wait and what it takes, the simulated device of a stream on the
//! clock of the run, the real stream behind a system's port or socket, and the reader of the keys that every device of a
//! simulation file has (`latency_ms`, `reply`, `unknown`), with the codes of the library that reads its section. What is a
//! library's own (its endpoints, its faults, the datagrams of UDP, the settings of a port) stays in the library.

use std::sync::Arc;

use labcraft_trace::number::duration_text;

use crate::io::{CLOSED, INVALID_ARGUMENT, INVALID_TEXT, TIMEOUT, TOO_LONG};
use crate::native::{EndpointLogs, NativeOutcome};
use crate::value::{Handle, Value};

mod device;
mod file;
#[cfg(not(target_arch = "wasm32"))]
pub mod real;

pub use device::{Clock, Device, DeviceSpec, MAX_WAITING, Next, Reply, described_time, read_step, write_step};
pub use file::{Codes, Reader, TomlValue, devices, endpoint, exchanges, get};

/// An error that a call gives, from a simulated device or from the system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub category: &'static str,
    /// That of the system for a real error, 0 for a simulated one (io.md 7).
    pub code: i64,
    pub message: String,
}

impl Failure {
    /// An error with no code of the system: simulated, or found before the system was asked.
    pub fn new(category: &'static str, message: String) -> Failure {
        Failure { category, code: 0, message }
    }

    pub fn outcome(self) -> NativeOutcome {
        NativeOutcome::Fail { category: self.category, code: self.code, message: self.message }
    }

    /// `io.timeout` of a read or a write that waited `timeout`, code 0.
    pub fn timed_out(label: &str, what: &str, timeout: i64) -> Failure {
        Failure::new(TIMEOUT, format!("{label}: {what} within {}", duration_text(timeout)))
    }

    /// `io.closed`, code 0, as a simulated device gives it once it has ended the stream.
    pub fn closed(label: &str) -> Failure {
        Failure::new(CLOSED, format!("{label}: the device closed the connection"))
    }
}

/// `io.closed` for a handle never opened, the default value of its type (`net.connection`), which a message calls `noun`.
pub fn never_opened(noun: &str, type_name: &str) -> NativeOutcome {
    Failure::new(CLOSED, format!("no {noun}: this one was never opened (the default of a `{type_name}`)")).outcome()
}

/// With `--sim`, the file is the whole bench (io.md 3): a name it does not describe is `category`, never reached. `section`
/// is the section of the library, which the file may not have at all.
pub fn not_found(category: &'static str, label: &str, section: &str, has_section: bool) -> NativeOutcome {
    let message = if has_section {
        format!("{label}: the simulation file describes no such device, and a run with one reaches no hardware")
    } else {
        format!("{label}: no device is simulated: the simulation file (--sim) has no `[{section}]`, and a run with one reaches no hardware")
    };
    Failure::new(category, message).outcome()
}

/// What a read of a stream needs.
#[derive(Clone, Debug)]
pub enum Need {
    /// `read`: exactly this many bytes.
    Exactly(usize),
    /// `read_until`, `read_line`: up to the terminator, included, within `max` bytes.
    Until(Terminator, usize),
    /// `read_available`: what is there, up to `max` bytes.
    Available(usize),
}

/// What the bytes that wait give a read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Got {
    /// It takes this many bytes.
    Take(usize),
    /// `max` bytes without the terminator: `io.too_long`, and it takes them.
    TooLong(usize),
    /// It needs more than is there.
    More,
}

/// A terminator, with the table of Knuth, Morris and Pratt that lets a search go on where it stopped: a read that waits looks
/// at each byte that arrives once, whatever the terminator (`libs/net/README.md`, `read_until`).
#[derive(Clone, Debug)]
pub struct Terminator {
    pub bytes: Vec<u8>,
    /// For each length matched, the longest proper prefix of the terminator that is also a suffix of that much of it.
    fallback: Vec<usize>,
}

impl Terminator {
    pub fn new(bytes: Vec<u8>) -> Self {
        let mut fallback = vec![0; bytes.len()];
        let mut matched = 0;
        for (at, byte) in bytes.iter().enumerate().skip(1) {
            while matched > 0 && bytes.get(matched) != Some(byte) {
                matched = fallback.get(matched - 1).copied().unwrap_or(0);
            }
            if bytes.get(matched) == Some(byte) {
                matched += 1;
            }
            if let Some(slot) = fallback.get_mut(at) {
                *slot = matched;
            }
        }
        Terminator { bytes, fallback }
    }
}

/// Where the search of a read stopped: the bytes of what waits it has looked at, and how much of the terminator ends them.
/// What waits only grows while the read holds its connection or its port, so the search goes on from there.
#[derive(Clone, Copy, Debug, Default)]
pub struct Scan {
    looked: usize,
    matched: usize,
}

impl Need {
    pub fn judge(&self, waiting: &[u8], scan: &mut Scan) -> Got {
        match self {
            Need::Exactly(count) if waiting.len() >= *count => Got::Take(*count),
            Need::Exactly(_) => Got::More,
            Need::Until(terminator, max) => {
                let within = waiting.get(..waiting.len().min(*max)).unwrap_or_default();
                let bytes = &terminator.bytes;
                for (at, byte) in within.iter().enumerate().skip(scan.looked) {
                    while scan.matched > 0 && bytes.get(scan.matched) != Some(byte) {
                        scan.matched = terminator.fallback.get(scan.matched - 1).copied().unwrap_or(0);
                    }
                    if bytes.get(scan.matched) == Some(byte) {
                        scan.matched += 1;
                    }
                    if scan.matched == bytes.len() && !bytes.is_empty() {
                        return Got::Take(at + 1);
                    }
                }
                scan.looked = scan.looked.max(within.len());
                if waiting.len() >= *max { Got::TooLong(*max) } else { Got::More }
            }
            Need::Available(max) => Got::Take(waiting.len().min(*max)),
        }
    }

    /// The most bytes a read looks at: what a drain of the system stops at.
    #[cfg_attr(target_arch = "wasm32", expect(dead_code, reason = "read by the real streams only, which WASM has not"))]
    fn limit(&self) -> usize {
        match self {
            Need::Exactly(count) => *count,
            Need::Until(_, max) | Need::Available(max) => *max,
        }
    }

    /// What a read that timed out waited for, for its message.
    fn wanted(&self) -> String {
        match self {
            Need::Exactly(count) => format!("{count} bytes did not come"),
            Need::Until(_, _) => "the terminator did not come".to_owned(),
            Need::Available(_) => "nothing came".to_owned(),
        }
    }
}

/// What a read took, simulated or real.
#[derive(Debug, PartialEq, Eq)]
pub enum Taken {
    Bytes(Vec<u8>),
    /// `io.too_long`: the `max` bytes taken without the terminator.
    TooLong(Vec<u8>),
    /// `net.datagram_too_long`: the datagram, dropped (`net`, UDP only, which says it itself).
    DatagramTooLong(Vec<u8>),
}

/// A read or a write of a stream, from the inputs of its call (`libs/net/README.md`, "Writing" and "Reading").
#[derive(Debug)]
pub enum Op {
    Write(Arc<[u8]>),
    /// `read_line` (`line`) strips its terminator and gives text.
    Read {
        need: Need,
        line: bool,
    },
}

impl Op {
    /// The terminator that `read_line` strips, for [`read_done`].
    pub fn stripped(&self) -> Option<&[u8]> {
        match self {
            Op::Read { need: Need::Until(terminator, _), line: true } => Some(&terminator.bytes),
            _ => None,
        }
    }
}

/// A `count` or a `max` above this is `io.invalid_argument`, since a read holds what it takes in the memory of the run
/// *(proposed)*.
const MAX_READ: i64 = 16 * 1024 * 1024;

/// The bytes of an input: a `bytes`, or the UTF-8 bytes of a `string`.
pub fn bytes_at(inputs: &[Value], at: usize) -> Arc<[u8]> {
    match inputs.get(at) {
        Some(Value::Bytes(bytes)) => Arc::clone(bytes),
        Some(Value::String(text)) => Arc::from(text.as_bytes()),
        _ => Arc::from(&[][..]),
    }
}

/// A `count` or a `max` of an input: below 0, or above 16 MiB, is `io.invalid_argument`.
pub fn count_at(inputs: &[Value], at: usize, name: &str) -> Result<usize, Failure> {
    let number = match inputs.get(at) {
        Some(Value::I32(number)) => i64::from(*number),
        Some(Value::I64(number)) => *number,
        _ => 0,
    };
    usize::try_from(number)
        .ok()
        .filter(|_| number <= MAX_READ)
        .ok_or_else(|| Failure::new(INVALID_ARGUMENT, format!("`{name}` is from 0 to 16 MiB (16,777,216), and this is {number}")))
}

/// What `write`, `write_line`, `read`, `read_available`, `read_until` or `read_line` does, from its inputs, the handle
/// first; its arguments judged. A function that is none of the others is `read_until` or `read_line`.
pub fn op_of(function: &str, inputs: &[Value]) -> Result<Op, Failure> {
    Ok(match function {
        "write" => Op::Write(bytes_at(inputs, 1)),
        "write_line" => {
            let (text, terminator) = (bytes_at(inputs, 1), bytes_at(inputs, 2));
            Op::Write(text.iter().chain(terminator.iter()).copied().collect())
        }
        "read" => Op::Read { need: Need::Exactly(count_at(inputs, 1, "count")?), line: false },
        "read_available" => Op::Read { need: Need::Available(count_at(inputs, 1, "max")?), line: false },
        _ => {
            let max = count_at(inputs, 2, "max")?;
            let terminator = bytes_at(inputs, 1).to_vec();
            if terminator.is_empty() {
                return Err(Failure::new(INVALID_ARGUMENT, "an empty terminator: a read would never find it".to_owned()));
            }
            Op::Read { need: Need::Until(Terminator::new(terminator), max), line: function == "read_line" }
        }
    })
}

/// The endpoint of a connection or a port: whole, for the report, and its start, for a message.
#[derive(Clone, Copy, Debug)]
pub struct Names<'a> {
    pub endpoint: &'a str,
    pub label: &'a str,
}

/// A write that went: its entry in the report, and the handle passes through.
pub fn wrote(log: &mut EndpointLogs, names: Names<'_>, handle: Handle, data: &[u8], now: i64) -> NativeOutcome {
    log.entry(names.endpoint, now, true, data);
    NativeOutcome::Outputs(vec![Value::Handle(handle), Value::NONE])
}

/// A read that ended, `stripped` the terminator of a `read_line`: what it took has its entry in the report (decision of the owner, 2026-11-19: a read that took
/// nothing has none, one that failed and took bytes has its), and its outputs. A datagram dropped is the library's to say:
/// here it is as bytes taken without a terminator.
pub fn read_done(log: &mut EndpointLogs, names: Names<'_>, handle: Handle, stripped: Option<&[u8]>, now: i64, taken: Taken) -> NativeOutcome {
    let (Taken::Bytes(bytes) | Taken::TooLong(bytes) | Taken::DatagramTooLong(bytes)) = &taken;
    if !bytes.is_empty() {
        log.entry(names.endpoint, now, false, bytes);
    }
    let label = names.label;
    let handle = Value::Handle(handle);
    match (taken, stripped) {
        (Taken::TooLong(bytes) | Taken::DatagramTooLong(bytes), _) => {
            Failure::new(TOO_LONG, format!("{label}: no terminator in {} bytes, which are taken: the next read starts after them", bytes.len()))
                .outcome()
        }
        (Taken::Bytes(bytes), Some(terminator)) => match String::from_utf8(line(bytes, terminator)) {
            Ok(text) => NativeOutcome::Outputs(vec![Value::string(text), handle, Value::NONE]),
            Err(_) => Failure::new(INVALID_TEXT, format!("{label}: the line read is not UTF-8; its bytes are taken")).outcome(),
        },
        (Taken::Bytes(bytes), None) => NativeOutcome::Outputs(vec![Value::Bytes(bytes.into()), handle, Value::NONE]),
    }
}

/// A line without its terminator and, when the terminator is `"\n"`, one `"\r"` before it (`libs/net/README.md`,
/// `read_line`).
fn line(mut bytes: Vec<u8>, terminator: &[u8]) -> Vec<u8> {
    if bytes.ends_with(terminator) {
        bytes.truncate(bytes.len() - terminator.len());
        if terminator == b"\n" && bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
    }
    bytes
}

/// What a message shows of a text the program or a file gave: its first 64 code points, its control characters escaped.
pub fn shown(text: &str) -> String {
    let start: String = text.chars().take(64).flat_map(char::escape_debug).collect();
    if text.chars().count() > 64 { format!("{start}…") } else { start }
}
