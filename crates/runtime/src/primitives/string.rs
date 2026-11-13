//! `string` (appendix A.2, spec 5.4).

use std::collections::HashMap;
use std::sync::Arc;

use labcraft_check::{Piece, template_fields, template_pieces};
use labcraft_ir::Static;

use super::{Call, Host, Outcome, input, input_type, room_for, variadic, wrong};
use crate::fault::Fault;
use crate::text::{Text, Writer, quoted};
use crate::value::{VALUE_COST, Value};

/// How much of a text a message quotes: the answer of an instrument can be long.
const QUOTED: usize = 64;

fn text_of<'v>(value: &'v Value, name: &str) -> Result<&'v Arc<str>, Fault> {
    match value {
        Value::String(text) => Ok(text),
        _ => Err(wrong(name)),
    }
}

fn joined<'v>(parts: impl Iterator<Item = &'v Value> + Clone, host: &dyn Host, name: &str) -> Result<Value, Fault> {
    let mut length = 0_usize;
    for part in parts.clone() {
        length = length.saturating_add(text_of(part, name)?.len());
    }
    room_for(host, VALUE_COST.saturating_add(u64::try_from(length).unwrap_or(u64::MAX)), name)?;
    let mut text = String::with_capacity(length);
    for part in parts {
        text.push_str(text_of(part, name)?);
    }
    Ok(Value::string(text))
}

pub fn concat(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    let parts = variadic(call)?;
    Ok(Outcome::Done(vec![joined(parts.iter(), host, "string.concat")?]))
}

/// The parts one after the other, with nothing between them.
pub fn join(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    let Value::Array(parts) = input(call, 0)? else { return Err(wrong("string.join")) };
    Ok(Outcome::Done(vec![joined(parts.items().iter(), host, "string.join")?]))
}

/// The template, each field written as its value's text with the format of its place (5.4). The inputs are the fields, each
/// once, in the order they first appear.
pub fn format(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    let Some(Static::Template(template)) = call.statics.first() else { return Err(wrong("string.format")) };
    let pieces = template_pieces(template).ok_or_else(|| Fault::bug("a template that `check` refuses, run"))?;
    // Each field once, in the order of its inputs: found by name in a table, so that a template of many fields is linear.
    let fields = template_fields(template);
    let names: HashMap<&str, usize> = fields.iter().enumerate().map(|(index, name)| (name.as_str(), index)).collect();
    let room = usize::try_from(host.room().saturating_sub(VALUE_COST)).unwrap_or(usize::MAX);
    let mut out = Text::new(room);
    let writer = Writer { env: call.env, viewer: call.viewer, cut: false };
    for piece in &pieces {
        match piece {
            Piece::Text(text) => out.push(text)?,
            Piece::Field { name, format } => {
                let index = names.get(name.as_ref()).copied().ok_or_else(|| wrong("string.format"))?;
                let has_format = *format != labcraft_check::FieldFormat::default();
                writer.write(&mut out, input(call, index)?, input_type(call, index)?, has_format.then_some(format))?;
            }
        }
    }
    Ok(Outcome::Done(vec![Value::string(out.finish())]))
}

/// The parts between each separator, in order: `"a,,b"` at `","` gives `["a", "", "b"]`. An empty separator splits nothing,
/// and no `err` can say it: `LC8008`, as the proof of concept refuses it.
pub fn split(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    let text = text_of(input(call, 0)?, "string.split")?;
    let separator = text_of(input(call, 1)?, "string.split")?;
    if separator.is_empty() {
        return Err(Fault::out_of_range("string.split: an empty separator splits nothing"));
    }
    let count = text.matches(&**separator).count().saturating_add(1);
    let size = VALUE_COST
        .saturating_add(u64::try_from(count).unwrap_or(u64::MAX).saturating_mul(VALUE_COST))
        .saturating_add(u64::try_from(text.len()).unwrap_or(u64::MAX));
    room_for(host, size, "string.split")?;
    let parts = text.split(&**separator).map(Value::string).collect();
    Ok(Outcome::Done(vec![Value::array(parts)]))
}

/// What a message shows of a text it could not read: between quotes, escaped, its start only.
fn shown(text: &str) -> String {
    let start: String = text.chars().take(QUOTED).collect();
    let mut shown = quoted(&start);
    if start.len() < text.len() {
        shown.push('…');
    }
    shown
}

/// An optional sign, digits with decimals and an exponent, or `nan` and `inf` in any case.
fn is_real(text: &str) -> bool {
    let body = text.strip_prefix(['+', '-']).unwrap_or(text);
    if body.eq_ignore_ascii_case("nan") || body.eq_ignore_ascii_case("inf") {
        return true;
    }
    let (mantissa, exponent) = match body.find(['e', 'E']) {
        Some(at) => (body.get(..at).unwrap_or_default(), Some(body.get(at + 1..).unwrap_or_default())),
        None => (body, None),
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = |part: &str| part.bytes().all(|byte| byte.is_ascii_digit());
    let mantissa_ok = digits(whole) && digits(fraction) && !(whole.is_empty() && fraction.is_empty());
    let exponent_ok = exponent.is_none_or(|exponent| {
        let exponent = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
        !exponent.is_empty() && digits(exponent)
    });
    mantissa_ok && exponent_ok
}

/// The nearest `f64` to the decimal text, ties to even, as Rust and Python both read it; the Unicode white space around it
/// does not count (spec A.2). A finite text beyond the `f64` fails rather than give an infinity that would look measured;
/// one too small gives zero with its sign (decision of the owner, 2026-11-13).
pub fn to_f64(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    let text = text_of(input(call, 0)?, "string.to_f64")?;
    let trimmed = text.trim();
    match is_real(trimmed).then(|| trimmed.parse::<f64>().ok()).flatten() {
        Some(value) if value.is_infinite() && !trimmed.trim_start_matches(['+', '-']).eq_ignore_ascii_case("inf") => {
            Ok(Outcome::Fail(host.error("string.parse", 0, format!("beyond an f64: {}", shown(text)))))
        }
        Some(value) => Ok(Outcome::Done(vec![Value::F64(value), Value::NONE])),
        None => Ok(Outcome::Fail(host.error("string.parse", 0, format!("not a number: {}", shown(text))))),
    }
}

pub fn to_i32(call: &mut Call<'_>, host: &mut dyn Host) -> Result<Outcome, Fault> {
    let text = text_of(input(call, 0)?, "string.to_i32")?;
    let trimmed = text.trim();
    let digits = trimmed.strip_prefix(['+', '-']).unwrap_or(trimmed);
    let read = (!digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())).then(|| trimmed.parse::<i32>().ok()).flatten();
    match read {
        Some(value) => Ok(Outcome::Done(vec![Value::I32(value), Value::NONE])),
        None => Ok(Outcome::Fail(host.error("string.parse", 0, format!("not an i32: {}", shown(text))))),
    }
}
