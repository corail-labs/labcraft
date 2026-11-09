//! The writer of a trace (`labcraft-trace/1`, `docs/commands.md`, 8): one JSON object per line, to a sink that the
//! caller gives, within the bounds of a line and of a file.

use std::fmt::Write as _;
use std::io;

use crate::number;
use crate::value::{CUT, VALUE_BUDGET, ValueWriter, WriteValue, push_string};

/// The name and version of the schema, on the first line.
pub const SCHEMA: &str = "labcraft-trace/1";

/// The bounds of a trace *(proposed figures)*.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WriteLimits {
    /// A whole line, `types` and `files` included.
    pub line: usize,
    /// The events, after which `truncated` is written, then only `end`.
    pub events: u64,
    /// The bytes of the file, after which the same.
    pub file: u64,
}

impl Default for WriteLimits {
    fn default() -> Self {
        Self { line: 16 << 20, events: 1_000_000, file: 256 << 20 }
    }
}

/// The first line: the run, the function and the files it read, each with the SHA-256 of its bytes ([`crate::sha256_hex`]).
#[derive(Clone, Copy, Debug)]
pub struct Header<'a> {
    pub run: &'a str,
    /// Named from the root of the project: `dmm_driver.measure`.
    pub function: &'a str,
    /// `deterministic` or `real_time`.
    pub mode: &'a str,
    /// `(file, sha256)`, as `check` names the file: `check_voltage.lcf`, `<labcraft>/scpi/query_f64.lcf`.
    pub files: &'a [(&'a str, &'a str)],
}

/// A value of an event, with its name and its type as the trace writes it (`f64[V]`).
pub struct Input<'a> {
    pub name: &'a str,
    pub ty: &'a str,
    pub value: &'a dyn WriteValue,
}

/// The node of the caller of a frame, and its iteration there.
#[derive(Clone, Copy, Debug)]
pub struct Caller<'a> {
    pub frame: u64,
    pub node: &'a str,
    pub iter: &'a [u64],
}

/// An event, as plain data: [`TraceWriter::event`] numbers it, dates it and writes it.
pub enum Event<'a> {
    /// A call of a function written in LabCraft begins: frame 0 for the function run, which has no caller.
    Call {
        frame: u64,
        caller: Option<Caller<'a>>,
        function: &'a str,
        /// In the order of the signature.
        inputs: &'a [Input<'a>],
        /// The type of each value that its body binds, by name; a name bound with two types is left out, and each
        /// event that binds it gives its type.
        types: &'a [(&'a str, &'a str)],
    },
    /// A node ran: each value it bound, by the name of its wire.
    Ran { frame: u64, node: &'a str, iter: &'a [u64], out: &'a [(&'a str, &'a dyn WriteValue)], types: &'a [(&'a str, &'a str)] },
    /// A node was skipped (spec 7.2): its defaults and the error passed on.
    Skipped { frame: u64, node: &'a str, iter: &'a [u64], out: &'a [(&'a str, &'a dyn WriteValue)], types: &'a [(&'a str, &'a str)] },
    /// A `match` or a `handle` took this arm, written as its pattern.
    Arm { frame: u64, node: &'a str, iter: &'a [u64], arm: &'a str },
    /// A panel element changed: by `ui.set`, `set` or `run.set_element`.
    Set { frame: u64, element: &'a str, value: &'a dyn WriteValue, by: &'a str },
}

impl Event<'_> {
    fn name(&self) -> &'static str {
        match self {
            Event::Call { .. } => "call",
            Event::Ran { .. } => "ran",
            Event::Skipped { .. } => "skipped",
            Event::Arm { .. } => "arm",
            Event::Set { .. } => "set",
        }
    }

    fn frame(&self) -> u64 {
        match self {
            Event::Call { frame, .. }
            | Event::Ran { frame, .. }
            | Event::Skipped { frame, .. }
            | Event::Arm { frame, .. }
            | Event::Set { frame, .. } => *frame,
        }
    }

    /// The node the line names: its own, or for a call that of the caller.
    fn node(&self) -> Option<&str> {
        match self {
            Event::Call { caller, .. } => caller.map(|caller| caller.node),
            Event::Ran { node, .. } | Event::Skipped { node, .. } | Event::Arm { node, .. } => Some(node),
            Event::Set { .. } => None,
        }
    }
}

/// A line too long even with its values cut and its entries dropped.
struct Overflow;

/// A line being written within its bound: values are cut, then entries dropped, as the line fills.
struct Line<'a> {
    text: &'a mut String,
    limit: usize,
    /// The values left show as cuts.
    values_cut: bool,
    /// Some value or entry was cut or dropped: the line says `"truncated":true`.
    truncated: bool,
}

/// Room kept at the end of a line for what closes it and `,"truncated":true`.
const TAIL: usize = 64;

impl Line<'_> {
    fn room(&self) -> usize {
        self.limit.saturating_sub(self.text.len() + TAIL)
    }

    /// Text that the line cannot do without: a name, a type of an input.
    fn needed(&mut self, text: &str) -> Result<(), Overflow> {
        if text.len() + CUT.len() > self.room() {
            return Err(Overflow);
        }
        self.text.push_str(text);
        Ok(())
    }

    fn string(&mut self, text: &str) -> Result<(), Overflow> {
        let mut quoted = String::with_capacity(text.len() + 2);
        push_string(&mut quoted, text);
        self.needed(&quoted)
    }

    fn value(&mut self, value: &dyn WriteValue) {
        let room = self.room();
        if self.values_cut || room < CUT.len() {
            self.text.push_str(CUT);
            self.values_cut = true;
            self.truncated = true;
            return;
        }
        let budget = room.min(VALUE_BUDGET);
        let mut writer = ValueWriter::new(self.text, budget);
        value.write_value(&mut writer);
        let written = writer.finish();
        if written.spent && budget < VALUE_BUDGET {
            self.values_cut = true;
            self.truncated = true;
        }
    }

    /// `"types":{…}` or the `files` of the first line: entries that do not fit are dropped, with the rest.
    fn entries(&mut self, key: &str, entries: &[(&str, &str)], object: bool) -> Result<(), Overflow> {
        self.needed(key)?;
        self.text.push(if object { '{' } else { '[' });
        let mut first = true;
        for (name, value) in entries {
            let mut entry = String::new();
            if !first {
                entry.push(',');
            }
            if object {
                push_string(&mut entry, name);
                entry.push(':');
                push_string(&mut entry, value);
            } else {
                entry.push_str("{\"file\":");
                push_string(&mut entry, name);
                entry.push_str(",\"sha256\":");
                push_string(&mut entry, value);
                entry.push('}');
            }
            if entry.len() > self.room() {
                self.truncated = true;
                break;
            }
            self.text.push_str(&entry);
            first = false;
        }
        self.text.push(if object { '}' } else { ']' });
        Ok(())
    }
}

/// Writes a trace, line by line, to `sink`. A sink that fails stops the trace, never the run: [`TraceWriter::error`].
pub struct TraceWriter<W: io::Write> {
    sink: W,
    limits: WriteLimits,
    seq: u64,
    /// The events written, `truncated` and `end` left out.
    written: u64,
    bytes: u64,
    truncated: bool,
    ended: bool,
    error: Option<io::Error>,
    line: String,
}

impl<W: io::Write> TraceWriter<W> {
    pub fn new(sink: W, header: &Header<'_>) -> Self {
        Self::with_limits(sink, header, WriteLimits::default())
    }

    pub fn with_limits(sink: W, header: &Header<'_>, limits: WriteLimits) -> Self {
        let mut writer = Self { sink, limits, seq: 0, written: 0, bytes: 0, truncated: false, ended: false, error: None, line: String::new() };
        writer.header(header);
        writer
    }

    /// The lines written after the first, `truncated` and `end` included.
    pub fn events(&self) -> u64 {
        self.seq
    }

    /// Whether the bounds of the file cut the trace.
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    /// Why the sink stopped taking lines, if it did.
    pub fn error(&self) -> Option<&io::Error> {
        self.error.as_ref()
    }

    pub fn into_inner(self) -> W {
        self.sink
    }

    fn header(&mut self, header: &Header<'_>) {
        let mut text = std::mem::take(&mut self.line);
        text.clear();
        // What names the trace is written whole, whatever its length: only the files can be dropped.
        text.push_str("{\"schema\":");
        push_string(&mut text, SCHEMA);
        for (key, value) in [(",\"run\":", header.run), (",\"function\":", header.function), (",\"mode\":", header.mode)] {
            text.push_str(key);
            push_string(&mut text, value);
        }
        let mut line = Line { text: &mut text, limit: self.limits.line, values_cut: false, truncated: false };
        if line.entries(",\"files\":", header.files, false).is_err() {
            line.text.push_str(",\"files\":[]");
            line.truncated = true;
        }
        if line.truncated {
            text.push_str(",\"truncated\":true");
        }
        text.push('}');
        self.emit(&text);
        self.line = text;
    }

    /// Writes an event at `t_ns` on the clock of the run. Past the bounds of the file, writes `truncated` once, and
    /// nothing more until [`TraceWriter::end`].
    pub fn event(&mut self, t_ns: u64, event: &Event<'_>) {
        if self.truncated || self.ended || self.error.is_some() {
            return;
        }
        let mut text = std::mem::take(&mut self.line);
        text.clear();
        let seq = self.seq + 1;
        if Self::full(&mut text, seq, t_ns, event, self.limits.line).is_err() {
            text.clear();
            Self::minimal(&mut text, seq, t_ns, event, self.limits.line);
        }
        // Room for `truncated` and `end` after it, with the longest `seq` and `t_ms` and a status.
        let room_after = 4 * TAIL as u64;
        if self.written >= self.limits.events || self.bytes + text.len() as u64 + 1 + room_after > self.limits.file {
            text.clear();
            text.push_str("{\"seq\":");
            let _ = write!(text, "{seq},\"t_ms\":{},\"event\":\"truncated\"}}", number::millis_text(t_ns));
            self.truncated = true;
        } else {
            self.written += 1;
        }
        self.seq = seq;
        self.emit(&text);
        self.line = text;
    }

    /// The last line, with the status of the run (`ok`, `error`, `aborted`, `fault`).
    pub fn end(&mut self, t_ns: u64, status: &str) {
        if self.ended || self.error.is_some() {
            return;
        }
        self.ended = true;
        self.seq += 1;
        let mut text = format!("{{\"seq\":{},\"t_ms\":{},\"event\":\"end\",\"status\":", self.seq, number::millis_text(t_ns));
        push_string(&mut text, status);
        text.push('}');
        self.emit(&text);
        self.flush();
    }

    /// Gives the sink what it holds, so that a run can be followed as it goes; [`TraceWriter::end`] does it too. A
    /// sink that fails here stops the trace, as on a write.
    pub fn flush(&mut self) {
        if self.error.is_none()
            && let Err(error) = self.sink.flush()
        {
            self.error = Some(error);
        }
    }

    fn emit(&mut self, text: &str) {
        if self.error.is_some() {
            return;
        }
        let result = self.sink.write_all(text.as_bytes()).and_then(|()| self.sink.write_all(b"\n"));
        match result {
            Ok(()) => self.bytes += text.len() as u64 + 1,
            Err(error) => self.error = Some(error),
        }
    }

    fn start(text: &mut String, seq: u64, t_ns: u64, event: &Event<'_>) {
        let _ = write!(text, "{{\"seq\":{seq},\"t_ms\":{},\"event\":\"{}\",\"frame\":{}", number::millis_text(t_ns), event.name(), event.frame());
    }

    fn full(text: &mut String, seq: u64, t_ns: u64, event: &Event<'_>, limit: usize) -> Result<(), Overflow> {
        Self::start(text, seq, t_ns, event);
        let mut line = Line { text, limit, values_cut: false, truncated: false };
        match event {
            Event::Call { caller, function, inputs, types, .. } => {
                if let Some(caller) = caller {
                    line.needed(&format!(",\"parent\":{}", caller.frame))?;
                    line.needed(",\"node\":")?;
                    line.string(caller.node)?;
                    iter(&mut line, caller.iter)?;
                }
                line.needed(",\"function\":")?;
                line.string(function)?;
                line.needed(",\"in\":[")?;
                for (index, input) in inputs.iter().enumerate() {
                    line.needed(if index == 0 { "{\"name\":" } else { ",{\"name\":" })?;
                    line.string(input.name)?;
                    line.needed(",\"type\":")?;
                    line.string(input.ty)?;
                    line.needed(",\"value\":")?;
                    line.value(input.value);
                    line.text.push('}');
                }
                line.text.push(']');
                line.entries(",\"types\":", types, true)?;
            }
            Event::Ran { node, iter: at, out, types, .. } | Event::Skipped { node, iter: at, out, types, .. } => {
                line.needed(",\"node\":")?;
                line.string(node)?;
                iter(&mut line, at)?;
                line.needed(",\"out\":{")?;
                for (index, (name, value)) in out.iter().enumerate() {
                    if index > 0 {
                        line.needed(",")?;
                    }
                    line.string(name)?;
                    line.needed(":")?;
                    line.value(*value);
                }
                line.text.push('}');
                if !types.is_empty() {
                    line.entries(",\"types\":", types, true)?;
                }
            }
            Event::Arm { node, iter: at, arm, .. } => {
                line.needed(",\"node\":")?;
                line.string(node)?;
                iter(&mut line, at)?;
                line.needed(",\"arm\":")?;
                line.string(arm)?;
            }
            Event::Set { element, value, by, .. } => {
                line.needed(",\"element\":")?;
                line.string(element)?;
                line.needed(",\"value\":")?;
                line.value(*value);
                line.needed(",\"by\":")?;
                line.string(by)?;
            }
        }
        if line.truncated {
            line.text.push_str(",\"truncated\":true");
        }
        line.text.push('}');
        Ok(())
    }

    /// What a line keeps when even its cut values pass the bound: `seq`, `t_ms`, `event`, `frame` and `node`.
    fn minimal(text: &mut String, seq: u64, t_ns: u64, event: &Event<'_>, limit: usize) {
        Self::start(text, seq, t_ns, event);
        if let Some(node) = event.node() {
            let mut quoted = String::new();
            push_string(&mut quoted, node);
            if text.len() + quoted.len() + TAIL < limit {
                text.push_str(",\"node\":");
                text.push_str(&quoted);
            }
        }
        text.push_str(",\"truncated\":true}");
    }
}

/// `,"iter":[…]`, when the node is in loops of its function.
fn iter(line: &mut Line<'_>, iter: &[u64]) -> Result<(), Overflow> {
    if iter.is_empty() {
        return Ok(());
    }
    let mut text = String::from(",\"iter\":[");
    for (index, iteration) in iter.iter().enumerate() {
        if index > 0 {
            text.push(',');
        }
        let _ = write!(text, "{iteration}");
    }
    text.push(']');
    line.needed(&text)
}

#[cfg(test)]
mod tests;
