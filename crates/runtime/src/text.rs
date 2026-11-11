//! The text of a value, as `string.format` writes it (spec 5.4; decision of the owner, 2026-11-09): as the reports write
//! values, so that what a run shows and what a program writes agree. A number with the fewest digits that read back the
//! same, a float with no fraction keeping `.0` (`12.0`), then its unit; a `duration` in seconds; a `timestamp` in RFC 3339;
//! a variant as `Verdict.Pass` or `Reading.Value(230.0 V)`; a struct as `Limits { low: 220.0 V, high: 240.0 V }`; an array
//! in brackets; `bytes` in hexadecimal; an `error` as `none`, or its category, its code and its message. A `string` is its
//! text alone, and written as a literal, between quotes, inside an array, a struct or a variant.
//!
//! A format (`{u:.3 mV}`) applies to each value inside: decimals, a letter, a sign, a unit and a width go to each number,
//! a width to each text. The form is that of C++20, with the two differences of 5.4.

use std::fmt::Write as _;

use labcraft_check::{Align, FieldFormat};
use labcraft_ir::{apply_scale, scale_between};
use labcraft_trace::number::{duration_text, f32_text, f64_text, timestamp_text};
use labcraft_types::{LibraryId, Ty, TyId, Unit};

use crate::convert::Env;
use crate::fault::Fault;
use crate::value::{ErrorData, Value};

/// Text written within a limit of bytes: the room that the memory of the run has left (cli.md 6.2).
pub struct Text {
    out: String,
    limit: usize,
}

impl Text {
    pub fn new(limit: usize) -> Self {
        Text { out: String::new(), limit }
    }

    pub fn push(&mut self, text: &str) -> Result<(), Fault> {
        if self.out.len().saturating_add(text.len()) > self.limit {
            return Err(Fault::memory("the text that `string.format` writes would pass the memory limit of the run"));
        }
        self.out.push_str(text);
        Ok(())
    }

    pub fn finish(self) -> String {
        self.out
    }
}

/// What writes a value: its types, and the library it is seen from, which names its types (`Verdict`, `scpi.Reading`).
#[derive(Clone, Copy)]
pub struct Writer<'a> {
    pub env: Env<'a>,
    pub viewer: LibraryId,
    /// Cut as a report cuts a value (`docs/commands.md`, section 5): an array of more than 64 elements shows its first 16,
    /// a string of more than 1,000 code points and `bytes` of more than 1,000 bytes their start, each followed by how
    /// much there is. For the human form of `labcraft-cli run` (cli.md 6.7); `string.format` writes everything.
    pub cut: bool,
}

/// An array longer than this is cut, to [`HEAD`] elements (commands.md 5).
const CUT_LEN: usize = 64;
const HEAD: usize = 16;
/// A string or `bytes` longer than this is cut to it (commands.md 5).
const CUT_TEXT: usize = 1000;

/// The text of a value as the language writes a constant (a `string` between quotes), cut as a report cuts it, within
/// `limit` bytes, past which it ends with `…`; `None` for a value that has no text, which holds a session or a function.
pub fn constant_text(env: Env<'_>, viewer: LibraryId, value: &Value, ty: TyId, limit: usize) -> Option<String> {
    let writer = Writer { env, viewer, cut: true };
    let mut out = Text::new(limit);
    match writer.value(&mut out, value, ty, None, false) {
        Ok(()) => Some(out.finish()),
        Err(fault) if fault.code == crate::fault::FaultCode::Memory => {
            let mut text = out.finish();
            text.push('…');
            Some(text)
        }
        Err(_) => None,
    }
}

impl Writer<'_> {
    /// The text of `value`, of type `ty`, with the format of its field if it has one.
    pub fn write(&self, out: &mut Text, value: &Value, ty: TyId, format: Option<&FieldFormat<'_>>) -> Result<(), Fault> {
        self.value(out, value, ty, format, true)
    }

    fn value(&self, out: &mut Text, value: &Value, ty: TyId, format: Option<&FieldFormat<'_>>, top: bool) -> Result<(), Fault> {
        let wrong = || Fault::bug("a value that does not hold its type, written as text");
        let defined = self.env.ty(ty);
        match (value, defined) {
            (Value::Array(items), Ty::Array { element, .. }) => {
                out.push("[")?;
                let all = items.items();
                let shown = if self.cut && all.len() > CUT_LEN { HEAD } else { all.len() };
                for (index, item) in all.iter().take(shown).enumerate() {
                    if index > 0 {
                        out.push(", ")?;
                    }
                    self.value(out, item, *element, format, false)?;
                }
                if shown < all.len() {
                    out.push(&format!(", … {} elements in all", all.len()))?;
                }
                out.push("]")
            }
            (Value::Grid(grid), Ty::Array { element, .. }) => self.grid_level(out, grid.items(), grid.shape(), *element, format),
            (Value::Struct(fields), Ty::Struct(declared)) => {
                if let Some(name) = self.type_name(ty) {
                    out.push(&name)?;
                    out.push(" ")?;
                }
                out.push("{ ")?;
                for (index, (field, (name, field_ty))) in fields.iter().zip(declared).enumerate() {
                    if index > 0 {
                        out.push(", ")?;
                    }
                    out.push(self.env.names.resolve(*name))?;
                    out.push(": ")?;
                    self.value(out, field, *field_ty, format, false)?;
                }
                out.push(" }")
            }
            (Value::Struct(fields), Ty::Waveform(element)) => {
                // Its format goes to its samples: `t0` and `dt` keep theirs (5.4).
                let [t0, dt, samples] = &**fields else { return Err(wrong()) };
                out.push("{ t0: ")?;
                self.scalar(out, t0, TyId::ERROR, None, false)?;
                out.push(", dt: ")?;
                self.scalar(out, dt, TyId::ERROR, None, false)?;
                out.push(", samples: ")?;
                match samples {
                    Value::Array(items) => {
                        out.push("[")?;
                        for (index, item) in items.items().iter().enumerate() {
                            if index > 0 {
                                out.push(", ")?;
                            }
                            self.value(out, item, *element, format, false)?;
                        }
                        out.push("]")?;
                    }
                    _ => return Err(wrong()),
                }
                out.push(" }")
            }
            (Value::Variant(variant), Ty::Enum(variants)) => {
                let declared = variants.get(variant.index as usize).ok_or_else(wrong)?;
                if let Some(name) = self.type_name(ty) {
                    out.push(&name)?;
                    out.push(".")?;
                }
                out.push(self.env.names.resolve(declared.name))?;
                if !variant.values.is_empty() {
                    out.push("(")?;
                    for (index, (value, value_ty)) in variant.values.iter().zip(&declared.values).enumerate() {
                        if index > 0 {
                            out.push(", ")?;
                        }
                        self.value(out, value, *value_ty, format, false)?;
                    }
                    out.push(")")?;
                }
                Ok(())
            }
            _ => self.scalar(out, value, ty, format, top),
        }
    }

    /// One level of a rectangular array, in brackets: its rows, or at the last level its items. A grid has at most 32 levels.
    fn grid_level(&self, out: &mut Text, items: &[Value], shape: &[usize], element: TyId, format: Option<&FieldFormat<'_>>) -> Result<(), Fault> {
        let Some((length, inner)) = shape.split_first() else { return Ok(()) };
        out.push("[")?;
        let stride: usize = inner.iter().product();
        let shown = if self.cut && *length > CUT_LEN { HEAD } else { *length };
        for index in 0..shown {
            if index > 0 {
                out.push(", ")?;
            }
            if inner.is_empty() {
                let item = items.get(index).ok_or_else(|| Fault::bug("a grid shorter than its shape"))?;
                self.value(out, item, element, format, false)?;
            } else {
                let start = index.saturating_mul(stride);
                let part = items.get(start..start.saturating_add(stride)).ok_or_else(|| Fault::bug("a grid shorter than its shape"))?;
                self.grid_level(out, part, inner, element, format)?;
            }
        }
        if shown < *length {
            out.push(&format!(", … {length} in all"))?;
        }
        out.push("]")
    }

    /// The name of a declared type, as its library or another one sees it; `None` for a type written in place.
    fn type_name(&self, ty: TyId) -> Option<String> {
        match self.env.program.types.get(ty) {
            Ty::Named { .. } => Some(self.env.program.types.display(ty, self.viewer, self.env.names)),
            _ => None,
        }
    }

    /// A value with no parts: the format applies to it whole.
    fn scalar(&self, out: &mut Text, value: &Value, ty: TyId, format: Option<&FieldFormat<'_>>, top: bool) -> Result<(), Fault> {
        let unit = match self.env.ty(ty) {
            Ty::Number { unit, .. } => *unit,
            _ => Unit::ONE,
        };
        let (text, numeric) = match value {
            Value::Bool(value) => ((if *value { "true" } else { "false" }).to_owned(), false),
            Value::String(text) if top => (text.to_string(), false),
            Value::String(text) if self.cut && text.chars().count() > CUT_TEXT => {
                let head: String = text.chars().take(CUT_TEXT).collect();
                (format!("{}… {} code points in all", quoted(&head), text.chars().count()), false)
            }
            Value::String(text) => (quoted(text), false),
            Value::Bytes(bytes) if self.cut && bytes.len() > CUT_TEXT => {
                (format!("{}… {} bytes in all", hex(bytes.get(..CUT_TEXT).unwrap_or_default()), bytes.len()), false)
            }
            Value::Bytes(bytes) => (hex(bytes), false),
            Value::Duration(ns) => match format {
                Some(format) if format.needs_number() => (number(&Number::Real(*ns as f64 / 1e9, false), Unit::SECOND, format)?, true),
                Some(format) if format.unit.is_some() => (number(&Number::Real(*ns as f64 / 1e9, false), Unit::SECOND, format)?, true),
                _ => (duration_text(*ns), true),
            },
            Value::Timestamp(ns) => (timestamp_text(*ns), false),
            Value::Error(error) => (error_text(error.as_deref()), false),
            Value::Function(_) | Value::Handle(_) | Value::ToTheEnd => return Err(Fault::bug("a value with no text, written as text (5.4)")),
            Value::Array(_) | Value::Grid(_) | Value::Struct(_) | Value::Variant(_) => {
                return Err(Fault::bug("a value that does not hold its type, written as text"));
            }
            number_value => {
                let held = Number::of(number_value).ok_or_else(|| Fault::bug("a number of no kind"))?;
                match format {
                    Some(format) => (number(&held, unit, format)?, true),
                    None => (plain(&held, unit), true),
                }
            }
        };
        match format.and_then(|format| format.width.map(|width| (format, width))) {
            Some((format, width)) => out.push(&pad(&text, format, width as usize, numeric)),
            None => out.push(&text),
        }
    }
}

trait NeedsNumber {
    fn needs_number(&self) -> bool;
}

impl NeedsNumber for FieldFormat<'_> {
    fn needs_number(&self) -> bool {
        self.plus || self.zero || self.decimals.is_some() || self.letter.is_some()
    }
}

/// A number as a format reads it.
enum Number {
    Integer(i128),
    /// With whether it is an `f32`, whose shortest digits are its own.
    Real(f64, bool),
    Complex(f64, f64, bool),
}

impl Number {
    fn of(value: &Value) -> Option<Number> {
        Some(match value {
            Value::I8(x) => Number::Integer(i128::from(*x)),
            Value::I16(x) => Number::Integer(i128::from(*x)),
            Value::I32(x) => Number::Integer(i128::from(*x)),
            Value::I64(x) => Number::Integer(i128::from(*x)),
            Value::U8(x) => Number::Integer(i128::from(*x)),
            Value::U16(x) => Number::Integer(i128::from(*x)),
            Value::U32(x) => Number::Integer(i128::from(*x)),
            Value::U64(x) => Number::Integer(i128::from(*x)),
            Value::F32(x) => Number::Real(f64::from(*x), true),
            Value::F64(x) => Number::Real(*x, false),
            Value::C64(re, im) => Number::Complex(f64::from(*re), f64::from(*im), true),
            Value::C128(re, im) => Number::Complex(*re, *im, false),
            _ => return None,
        })
    }
}

fn real_text(x: f64, single: bool) -> String {
    if single { f32_text(x as f32) } else { f64_text(x) }
}

fn with_unit(mut text: String, unit: &str) -> String {
    if !unit.is_empty() {
        text.push(' ');
        text.push_str(unit);
    }
    text
}

fn unit_text(unit: Unit) -> String {
    if unit == Unit::ONE { String::new() } else { unit.to_string() }
}

/// With no format: as a report writes it.
fn plain(number: &Number, unit: Unit) -> String {
    let body = match number {
        Number::Integer(x) => x.to_string(),
        Number::Real(x, single) => real_text(*x, *single),
        Number::Complex(re, im, single) => complex_text(&real_text(*re, *single), *im, |im| real_text(im, *single)),
    };
    with_unit(body, &unit_text(unit))
}

/// `1.0 + 2.0i`, `1.0 - 2.0i`.
fn complex_text(re: &str, im: f64, text: impl Fn(f64) -> String) -> String {
    let (sign, magnitude) = if im.is_sign_negative() && !im.is_nan() { ('-', -im) } else { ('+', im) };
    format!("{re} {sign} {}i", text(magnitude))
}

/// A number with a format: its unit first, then its letter, its decimals and its sign.
fn number(number: &Number, unit: Unit, format: &FieldFormat<'_>) -> Result<String, Fault> {
    // In the unit of the field: converted as a wire is (3.3), in `f64`, an `f32` rounded once.
    let (number, shown_unit) = match format.unit {
        Some((to, written)) => {
            let scale = scale_between(&unit, &to);
            let converted = match number {
                Number::Integer(x) => Number::Real(apply_scale(*x as f64, &scale, false), false),
                Number::Real(x, single) => Number::Real(apply_scale(*x, &scale, *single), *single),
                Number::Complex(re, im, single) => Number::Complex(apply_scale(*re, &scale, *single), apply_scale(*im, &scale, *single), *single),
            };
            (converted, written.to_owned())
        }
        None => (clone_number(number), unit_text(unit)),
    };
    let body = match &number {
        Number::Complex(re, im, single) => {
            let first = real(*re, *single, format, true)?;
            let text = complex_text(&first.text, *im, |im| real(im, *single, format, false).map(|body| body.text).unwrap_or_default());
            Body { text, lead: first.lead, finite: re.is_finite() && im.is_finite() }
        }
        Number::Integer(x) => integer(*x, format)?,
        Number::Real(x, single) => real(*x, *single, format, true)?,
    };
    let mut text = with_unit(body.text, &shown_unit);
    // `0` fills the width with zeros after the sign and the prefix: the width counts the whole text, its unit included (5.4),
    // and a complex number is one text. Never before `nan` or `inf`, which the width pads with spaces (C++20).
    if format.zero
        && body.finite
        && let Some(width) = format.width
    {
        let missing = (width as usize).saturating_sub(text.chars().count());
        if missing > 0 && text.is_char_boundary(body.lead) {
            text.insert_str(body.lead, &"0".repeat(missing));
        }
    }
    Ok(text)
}

/// The text of a number before its unit, and where zeros go: after its sign and its prefix (`-`, `+`, `0x`).
struct Body {
    text: String,
    lead: usize,
    finite: bool,
}

fn body(sign: &str, prefix: &str, digits: &str) -> Body {
    Body { text: format!("{sign}{prefix}{digits}"), lead: sign.len() + prefix.len(), finite: true }
}

fn clone_number(number: &Number) -> Number {
    match number {
        Number::Integer(x) => Number::Integer(*x),
        Number::Real(x, single) => Number::Real(*x, *single),
        Number::Complex(re, im, single) => Number::Complex(*re, *im, *single),
    }
}

fn sign(negative: bool, plus: bool) -> &'static str {
    match (negative, plus) {
        (true, _) => "-",
        (false, true) => "+",
        (false, false) => "",
    }
}

/// An integer: `d` and no letter in decimal, `x`, `X`, `o`, `b` in their base; decimals or another letter make it a real.
fn integer(x: i128, format: &FieldFormat<'_>) -> Result<Body, Fault> {
    let base = match format.letter {
        None | Some('d') if format.decimals.is_none() => 10,
        Some('x' | 'X') => 16,
        Some('o') => 8,
        Some('b') => 2,
        _ => return real(x as f64, false, format, true),
    };
    let magnitude = x.unsigned_abs();
    let digits = match (base, format.letter) {
        (16, Some('X')) => format!("{magnitude:X}"),
        (16, _) => format!("{magnitude:x}"),
        (8, _) => format!("{magnitude:o}"),
        (2, _) => format!("{magnitude:b}"),
        _ => magnitude.to_string(),
    };
    // C++20: `#x` writes `0x`, `#X` `0X`, `#b` `0b`, and `#o` a leading `0` that a zero does not repeat.
    let prefix = match (format.alternate, format.letter) {
        (true, Some('x')) => "0x",
        (true, Some('X')) => "0X",
        (true, Some('b')) => "0b",
        (true, Some('o')) if magnitude != 0 => "0",
        _ => "",
    };
    Ok(body(sign(x < 0, format.plus), prefix, &digits))
}

/// A real number by its letter: `f` fixed, `e` and `E` scientific, `g` general; decimals alone fix them.
fn real(x: f64, single: bool, format: &FieldFormat<'_>, with_sign: bool) -> Result<Body, Fault> {
    let plus = format.plus && with_sign;
    if !x.is_finite() {
        let text = if x.is_nan() { "nan" } else { "inf" };
        let text = if format.letter == Some('E') { text.to_uppercase() } else { text.to_owned() };
        return Ok(Body { finite: false, ..body(sign(x.is_sign_negative() && !x.is_nan(), plus), "", &text) });
    }
    let negative = x.is_sign_negative();
    let magnitude = x.abs();
    let digits = match (format.letter, format.decimals) {
        (None, None) => real_text(magnitude, single),
        (None | Some('f'), decimals) => {
            let decimals = decimals.unwrap_or(6) as usize;
            format!("{magnitude:.decimals$}")
        }
        (Some(letter @ ('e' | 'E')), decimals) => scientific(magnitude, decimals.unwrap_or(6) as usize, letter == 'E'),
        (Some('g'), decimals) => general(magnitude, decimals.unwrap_or(6)),
        _ => return Err(Fault::bug("a letter that `check` refuses on a real number")),
    };
    Ok(body(sign(negative, plus), "", &digits))
}

/// `1.234000e+00`: C++ writes the sign of the exponent and two digits at least.
fn scientific(magnitude: f64, decimals: usize, upper: bool) -> String {
    let text = format!("{magnitude:.decimals$e}");
    let (mantissa, exponent) = text.split_once('e').unwrap_or((&text, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let letter = if upper { 'E' } else { 'e' };
    let sign = if exponent < 0 { '-' } else { '+' };
    format!("{mantissa}{letter}{sign}{:02}", exponent.unsigned_abs())
}

/// `g` with `precision` significant digits, at least one: scientific when the exponent is below -4 or at least the
/// precision, fixed otherwise; then the zeros that end the fraction, and its point, go (C++, `printf`).
fn general(magnitude: f64, precision: u32) -> String {
    let precision = precision.max(1) as usize;
    let exponent: i32 = if magnitude == 0.0 {
        0
    } else {
        let text = format!("{magnitude:.prec$e}", prec = precision - 1);
        text.split_once('e').and_then(|(_, exponent)| exponent.parse().ok()).unwrap_or(0)
    };
    if exponent < -4 || exponent >= i32::try_from(precision).unwrap_or(i32::MAX) {
        let text = scientific(magnitude, precision - 1, false);
        let (mantissa, rest) = text.split_once('e').unwrap_or((&text, ""));
        format!("{}e{rest}", trim_fraction(mantissa))
    } else {
        let decimals = usize::try_from(i32::try_from(precision).unwrap_or(i32::MAX) - 1 - exponent).unwrap_or(0);
        trim_fraction(&format!("{magnitude:.decimals$}")).to_owned()
    }
}

fn trim_fraction(text: &str) -> &str {
    if text.contains('.') { text.trim_end_matches('0').trim_end_matches('.') } else { text }
}

/// The text in its width, by its fill and its alignment: a number to the right, anything else to the left, unless the format
/// says; what `^` cannot split evenly goes to the right. Zeros already filled a number that asked for them.
fn pad(text: &str, format: &FieldFormat<'_>, width: usize, numeric: bool) -> String {
    let length = text.chars().count();
    if length >= width {
        return text.to_owned();
    }
    let missing = width - length;
    let fill = format.fill.unwrap_or(' ');
    let align = format.align.unwrap_or(if numeric { Align::Right } else { Align::Left });
    let (before, after) = match align {
        Align::Left => (0, missing),
        Align::Right => (missing, 0),
        Align::Center => (missing / 2, missing - missing / 2),
    };
    let mut padded = String::with_capacity(text.len() + missing * fill.len_utf8());
    padded.extend(std::iter::repeat_n(fill, before));
    padded.push_str(text);
    padded.extend(std::iter::repeat_n(fill, after));
    padded
}

/// A text between its quotes, as the language writes a literal: with its escapes, `\"`, `\\`, `\n`, `\t` and `\r` (14.1); a
/// control character, which no file holds (section 2), as `\u{1b}`, so that a log stays readable and cannot drive a terminal
/// (decision of the owner, 2026-11-09: the language does not read that form back yet); every other character as it is.
pub fn quoted(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('"');
    for c in text.chars() {
        match c {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\n' => quoted.push_str("\\n"),
            '\t' => quoted.push_str("\\t"),
            '\r' => quoted.push_str("\\r"),
            control if control.is_control() => {
                let _ = write!(quoted, "\\u{{{:x}}}", u32::from(control));
            }
            other => quoted.push(other),
        }
    }
    quoted.push('"');
    quoted
}

fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

/// `none`; or `io.timeout (-1073807339): no answer`, its code only when it has one, its message only when it is not empty,
/// then each of its others after `; `.
pub fn error_text(error: Option<&ErrorData>) -> String {
    let Some(error) = error else { return "none".to_owned() };
    let mut text = one_error(error);
    for other in error.others.iter() {
        text.push_str("; ");
        text.push_str(&one_error(other));
    }
    text
}

fn one_error(error: &ErrorData) -> String {
    let mut text = error.category.to_string();
    if error.code != 0 {
        let _ = write!(text, " ({})", error.code);
    }
    if !error.message.is_empty() {
        text.push_str(": ");
        text.push_str(&error.message);
    }
    text
}
