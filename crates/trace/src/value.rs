//! Values in JSON (`docs/commands.md`, 5), with the bounded writer that the report, the trace and the viewer share: an
//! array of more than 64 elements shows `$len` and 16 elements of `$head`, a string or `bytes` over 1,000 code points
//! or bytes shows `$len` and the first 1,000, and each value has a budget of 64 KiB of JSON and 64 levels, past which
//! what is left of it shows as `{"$cut":true}`.
//!
//! The writer is driven by the caller, which knows its values (`runtime`, the report): it gives scalars, and begins
//! and ends arrays and objects. It decides as it goes, so a value of a million elements costs the 16 that it writes.

use std::fmt::Write as _;

use crate::number;

/// An array with more elements is written as `$len` and `$head`.
pub const ARRAY_MAX: usize = 64;
/// The elements that `$head` keeps of a long array.
pub const ARRAY_HEAD: usize = 16;
/// The code points of a string, or the bytes of `bytes`, written before `$len` and `$head` take over.
pub const TEXT_MAX: usize = 1000;
/// The JSON of one value, in bytes.
pub const VALUE_BUDGET: usize = 64 * 1024;
/// The levels of one value: an array or an object is a level, and so is a mark of a cut.
pub const VALUE_DEPTH: usize = 64;
/// The merged errors that `others` lists; `others_more` counts the rest.
pub const OTHERS_MAX: usize = 32;

/// What takes the place of what is left of a value.
pub const CUT: &str = r#"{"$cut":true}"#;
/// The same, as the member of an object whose next field did not fit.
const CUT_MEMBER: &str = r#""$cut":true"#;
/// Room kept for a comma and [`CUT`], so that a cut always fits.
const RESERVE: usize = CUT.len() + 1;

/// A value that can write itself in JSON: `runtime` implements it for its values, with their types.
pub trait WriteValue {
    fn write_value(&self, writer: &mut ValueWriter<'_>);
}

impl<F: Fn(&mut ValueWriter<'_>)> WriteValue for F {
    fn write_value(&self, writer: &mut ValueWriter<'_>) {
        self(writer);
    }
}

/// What writing a value did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Written {
    /// The budget or the depth cut it: some of it shows as `{"$cut":true}`.
    pub cut: bool,
    /// The budget, rather than the depth: nothing after the cut was written.
    pub spent: bool,
}

#[derive(Clone, Copy)]
enum Open {
    /// `head`: written as `{"$len":…,"$head":[`, which takes two levels and closes with `]}`.
    Array { first: bool, head: bool },
    /// `key`: a key was written, its value comes next.
    Object { first: bool, key: bool },
}

impl Open {
    fn closer(self) -> &'static str {
        match self {
            Open::Array { head: true, .. } => "]}",
            Open::Array { head: false, .. } => "]",
            Open::Object { .. } => "}",
        }
    }

    fn levels(self) -> usize {
        match self {
            Open::Array { head: true, .. } => 2,
            _ => 1,
        }
    }
}

/// Writes one value into a string, within its budget and its depth.
pub struct ValueWriter<'a> {
    out: &'a mut String,
    start: usize,
    budget: usize,
    open: Vec<Open>,
    depth: usize,
    closers: usize,
    /// Arrays and objects begun inside a part that was cut: their contents and their ends are not written.
    skip: usize,
    /// Whether the top level has its value.
    done: bool,
    written: Written,
}

impl<'a> ValueWriter<'a> {
    /// A writer that appends one value to `out`, in at most `budget` bytes ([`VALUE_BUDGET`] for a value of a report).
    /// A budget too small for [`CUT`] still writes it.
    pub fn new(out: &'a mut String, budget: usize) -> Self {
        let start = out.len();
        Self { out, start, budget, open: Vec::new(), depth: 0, closers: 0, skip: 0, done: false, written: Written::default() }
    }

    /// Ends the value: what is still open closes, and a value never given is a cut.
    pub fn finish(mut self) -> Written {
        if !self.done && self.open.is_empty() {
            self.out.push_str(CUT);
            self.written.cut = true;
        }
        self.skip = 0;
        while !self.open.is_empty() {
            self.end();
        }
        self.written
    }

    /// Whether nothing more will be written: a caller can stop giving elements.
    pub fn spent(&self) -> bool {
        self.written.spent
    }

    fn used(&self) -> usize {
        self.out.len() - self.start
    }

    fn fits(&self, more: usize) -> bool {
        self.used() + more + self.closers + RESERVE <= self.budget
    }

    /// The text before a value at this position, or `None` when it is not to be written: inside a cut, after the
    /// budget, a second value at the top level, or a value in an object without its key.
    fn position(&mut self) -> Option<&'static str> {
        if self.skip > 0 || self.written.spent {
            return None;
        }
        match self.open.last_mut() {
            None if self.done => None,
            None => {
                self.done = true;
                Some("")
            }
            Some(Open::Array { first, .. }) => Some(if std::mem::replace(first, false) { "" } else { "," }),
            Some(Open::Object { key, .. }) => std::mem::replace(key, false).then_some(""),
        }
    }

    /// A part too deep for the depth left: the mark takes its place, within the budget like any other text.
    fn too_deep(&mut self, prefix: &str) {
        if self.fits(prefix.len() + CUT.len()) {
            self.out.push_str(prefix);
            self.out.push_str(CUT);
            self.written.cut = true;
        } else {
            self.spend(prefix);
        }
    }

    /// The budget is spent here: the mark takes the place of the value, and nothing more is written.
    fn spend(&mut self, prefix: &str) {
        self.out.push_str(prefix);
        self.out.push_str(CUT);
        self.written = Written { cut: true, spent: true };
    }

    /// A value of one piece of text, which `write` writes in place: taken back for a cut when it does not fit, as
    /// [`ValueWriter::scalar`] judges it. No text is made apart, since a trace writes millions of numbers.
    fn scalar_in_place(&mut self, write: impl FnOnce(&mut String)) {
        let Some(prefix) = self.position() else { return };
        let mark = self.out.len();
        self.out.push_str(prefix);
        write(self.out);
        if !self.fits(0) {
            self.out.truncate(mark);
            self.spend(prefix);
        }
    }

    /// A value of one piece of text.
    fn scalar(&mut self, text: &str) {
        let Some(prefix) = self.position() else { return };
        if self.fits(prefix.len() + text.len()) {
            self.out.push_str(prefix);
            self.out.push_str(text);
        } else {
            self.spend(prefix);
        }
    }

    /// A small object of `levels` levels written whole (`bytes`, a complex number, a cut string): a mark where the
    /// depth has no room for it.
    fn leaf(&mut self, text: &str, levels: usize) {
        let Some(prefix) = self.position() else { return };
        if self.depth + levels > VALUE_DEPTH {
            self.too_deep(prefix);
        } else if self.fits(prefix.len() + text.len()) {
            self.out.push_str(prefix);
            self.out.push_str(text);
        } else {
            self.spend(prefix);
        }
    }

    fn begin(&mut self, open: Open, text: &str) -> bool {
        let Some(prefix) = self.position() else {
            self.skip += 1;
            return false;
        };
        // An array or an object keeps room for one more level, the mark of a cut inside it.
        if self.depth + open.levels() >= VALUE_DEPTH {
            self.too_deep(prefix);
            self.skip += 1;
            return false;
        }
        if !self.fits(prefix.len() + text.len() + open.closer().len()) {
            self.spend(prefix);
            self.skip += 1;
            return false;
        }
        self.out.push_str(prefix);
        self.out.push_str(text);
        self.depth += open.levels();
        self.closers += open.closer().len();
        self.open.push(open);
        true
    }

    /// Begins an array of `len` elements, and returns how many of them to give: all of them, the 16 of `$head` past 64,
    /// or none where it was cut.
    pub fn begin_array(&mut self, len: usize) -> usize {
        let head = len > ARRAY_MAX;
        let text = if head { format!(r#"{{"$len":{len},"$head":["#) } else { "[".to_owned() };
        if !self.begin(Open::Array { first: true, head }, &text) {
            return 0;
        }
        if head { ARRAY_HEAD } else { len }
    }

    /// Begins an object: a struct, a waveform, an error, an opaque value. Each field is a [`ValueWriter::key`], then its value.
    pub fn begin_object(&mut self) {
        self.begin(Open::Object { first: true, key: false }, "{");
    }

    /// The name of the next field of the object begun last. A name that does not fit ends the object with the member
    /// `"$cut":true`.
    pub fn key(&mut self, name: &str) {
        if self.skip > 0 || self.written.spent {
            return;
        }
        let Some(Open::Object { first, key }) = self.open.last_mut() else { return };
        if *key {
            return;
        }
        let prefix = if std::mem::replace(first, false) { "" } else { "," };
        let mut text = String::with_capacity(name.len() + 4);
        text.push_str(prefix);
        push_string(&mut text, name);
        text.push(':');
        if self.fits(text.len()) {
            self.out.push_str(&text);
            if let Some(Open::Object { key, .. }) = self.open.last_mut() {
                *key = true;
            }
        } else {
            self.out.push_str(prefix);
            self.out.push_str(CUT_MEMBER);
            self.written = Written { cut: true, spent: true };
        }
    }

    /// Ends the array or object begun last.
    pub fn end(&mut self) {
        if self.skip > 0 {
            self.skip -= 1;
            return;
        }
        let Some(open) = self.open.pop() else { return };
        // A key whose value never came: the value is a cut, so that the text still reads.
        if let Open::Object { key: true, .. } = open {
            self.out.push_str(CUT);
            self.written.cut = true;
        }
        self.out.push_str(open.closer());
        self.depth -= open.levels();
        self.closers -= open.closer().len();
    }

    pub fn null(&mut self) {
        self.scalar("null");
    }

    pub fn bool(&mut self, value: bool) {
        self.scalar(if value { "true" } else { "false" });
    }

    /// An integer, with the unit of its type if it has one: `12`, `"9007199254740993"`, `"-3 mV"`.
    pub fn i64(&mut self, value: i64, unit: Option<&str>) {
        self.number_with(number::i64_is_exact(value), unit, |out| {
            let _ = write!(out, "{value}");
        });
    }

    pub fn u64(&mut self, value: u64, unit: Option<&str>) {
        self.number_with(number::u64_is_exact(value), unit, |out| {
            let _ = write!(out, "{value}");
        });
    }

    /// An `f64`, with the unit of its type if it has one: `0.5`, `"nan"`, `"230.03 V"`.
    pub fn f64(&mut self, value: f64, unit: Option<&str>) {
        number::with_f64_text(value, |digits| self.number(digits, value.is_finite(), unit));
    }

    pub fn f32(&mut self, value: f32, unit: Option<&str>) {
        self.number(&number::f32_text(value), value.is_finite(), unit);
    }

    /// A number as JSON gives it when it reads back the same and has no unit; otherwise a string, as a constant of the
    /// language.
    fn number(&mut self, digits: &str, plain: bool, unit: Option<&str>) {
        self.number_with(plain, unit, |out| out.push_str(digits));
    }

    /// [`ValueWriter::number`], its digits written by `digits`, all in place: `12`, `"9007199254740993"`, `"230.03 V"`.
    fn number_with(&mut self, plain: bool, unit: Option<&str>, digits: impl FnOnce(&mut String)) {
        self.scalar_in_place(|out| match unit {
            None if plain => digits(out),
            None => {
                out.push('"');
                push_escaped(out, digits);
                out.push('"');
            }
            Some(unit) => {
                out.push('"');
                push_escaped(out, digits);
                out.push(' ');
                push_escaped(out, |out| out.push_str(unit));
                out.push('"');
            }
        });
    }

    /// A `c64` or a `c128`, as `{"re":…,"im":…}`; `f32` says which, for the digits of each part.
    pub fn complex(&mut self, re: f64, im: f64, f32: bool, unit: Option<&str>) {
        let part = |value: f64| {
            let digits = if f32 { number::f32_text(value as f32) } else { number::f64_text(value) };
            match unit {
                None if value.is_finite() => digits,
                None => quoted(&digits),
                Some(unit) => quoted(&format!("{digits} {unit}")),
            }
        };
        self.leaf(&format!(r#"{{"re":{},"im":{}}}"#, part(re), part(im)), 1);
    }

    /// A `duration`, from its whole nanoseconds: `"0.1 s"`.
    pub fn duration(&mut self, nanos: i64) {
        self.scalar(&quoted(&number::duration_text(nanos)));
    }

    /// A `timestamp`, from nanoseconds since 1970-01-01T00:00:00Z.
    pub fn timestamp(&mut self, nanos: i64) {
        self.scalar(&quoted(&number::timestamp_text(nanos)));
    }

    /// A string; past 1,000 code points, `{"$len":…,"$head":"…"}` with the first 1,000.
    pub fn string(&mut self, value: &str) {
        let len = value.chars().count();
        if len <= TEXT_MAX {
            self.scalar(&quoted(value));
            return;
        }
        let end = value.char_indices().nth(TEXT_MAX).map_or(value.len(), |(index, _)| index);
        let head = value.get(..end).unwrap_or_default();
        self.leaf(&format!(r#"{{"$len":{len},"$head":{}}}"#, quoted(head)), 1);
    }

    /// `bytes`, as `{"base64":"…"}`; past 1,000 bytes, `$len` and the first 1,000.
    pub fn bytes(&mut self, value: &[u8]) {
        if value.len() <= TEXT_MAX {
            self.leaf(&format!(r#"{{"base64":"{}"}}"#, number::base64(value)), 1);
            return;
        }
        let head = number::base64(value.get(..TEXT_MAX).unwrap_or_default());
        self.leaf(&format!(r#"{{"$len":{},"$head":{{"base64":"{head}"}}}}"#, value.len()), 2);
    }

    /// A variant without values, `"Verdict.Pass"`.
    pub fn variant(&mut self, name: &str) {
        self.scalar(&quoted(name));
    }

    /// Begins a variant with values, `{"Reading.Value":[…]}`: give its values, then [`ValueWriter::end_variant`].
    pub fn begin_variant(&mut self, name: &str, values: usize) -> usize {
        self.begin_object();
        self.key(name);
        self.begin_array(values)
    }

    pub fn end_variant(&mut self) {
        self.end();
        self.end();
    }

    /// An error of spec 7.1; `none` is [`ValueWriter::null`].
    pub fn error(&mut self, error: &ErrorValue<'_>) {
        self.error_fields(error, true);
    }

    fn error_fields(&mut self, error: &ErrorValue<'_>, with_others: bool) {
        self.begin_object();
        self.key("category");
        self.string(error.category);
        self.key("code");
        self.i64(error.code, None);
        self.key("message");
        self.string(error.message);
        if let Some(origin) = &error.origin {
            self.key("origin");
            self.begin_object();
            self.place_fields(&origin.place);
            self.key("callers");
            let count = self.begin_array(origin.callers.len());
            for caller in origin.callers.iter().take(count) {
                self.begin_object();
                self.place_fields(caller);
                self.end();
            }
            self.end();
            self.end();
        }
        if with_others {
            // Flat: the errors that `errors.merge` gathered, each without its own.
            self.key("others");
            let count = self.begin_array(error.others.len().min(OTHERS_MAX));
            for other in error.others.iter().take(count) {
                self.error_fields(other, false);
            }
            self.end();
            let more = (error.others.len().saturating_sub(OTHERS_MAX) as u64).saturating_add(error.others_left);
            if more > 0 {
                self.key("others_more");
                self.u64(more, None);
            }
        }
        self.end();
    }

    fn place_fields(&mut self, place: &Place<'_>) {
        self.key("function");
        self.string(place.function);
        self.key("node");
        self.string(place.node);
        self.key("file");
        self.string(place.file);
        self.key("line");
        self.u64(place.line, None);
    }
}

/// An error of spec 7.1, as plain data for [`ValueWriter::error`].
#[derive(Clone, Debug)]
pub struct ErrorValue<'a> {
    pub category: &'a str,
    pub code: i64,
    pub message: &'a str,
    pub origin: Option<Origin<'a>>,
    /// The errors that `errors.merge` gathered, or the first of them; their own `others` are not written.
    pub others: &'a [ErrorValue<'a>],
    /// The errors gathered past those of `others`, which the caller did not give: so that it gives only the first
    /// [`OTHERS_MAX`] of a long merge, and writing it costs no more than what is written.
    pub others_left: u64,
}

/// Where an error was made, and the chain of its callers, innermost first.
#[derive(Clone, Debug)]
pub struct Origin<'a> {
    pub place: Place<'a>,
    pub callers: &'a [Place<'a>],
}

#[derive(Clone, Debug)]
pub struct Place<'a> {
    pub function: &'a str,
    pub node: &'a str,
    pub file: &'a str,
    pub line: u64,
}

/// One value, within [`VALUE_BUDGET`]: what a report holds for an output, an argument or a panel element.
pub fn write_value(out: &mut String, value: &dyn WriteValue) -> Written {
    let mut writer = ValueWriter::new(out, VALUE_BUDGET);
    value.write_value(&mut writer);
    writer.finish()
}

/// A JSON string of this text.
pub fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    push_string(&mut out, text);
    out
}

/// Appends what `write` writes, escaped as in a JSON string, without the quotes: digits and units have nothing to escape,
/// and are then left as written.
fn push_escaped(out: &mut String, write: impl FnOnce(&mut String)) {
    let start = out.len();
    write(out);
    let plain = out.get(start..).is_none_or(|text| !text.chars().any(|c| matches!(c, '"' | '\\') || u32::from(c) < 0x20));
    if !plain {
        let text = out.split_off(start);
        push_string(out, &text);
        // Without the quotes that `push_string` adds.
        out.remove(start);
        out.pop();
    }
}

/// Appends a JSON string of this text: `"`, `\` and the control characters escaped, the rest as it is.
pub fn push_string(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests;
