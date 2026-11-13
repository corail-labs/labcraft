//! The converter of arguments and panel settings (`docs/cli.md`, 6.3 and 6.4): a value in its JSON form (commands.md 5),
//! read against the type of its input or element, as a literal of the language is (spec 3.4). A number reads once in the
//! kind of its type and converts as a wire does, through `ir::literal_number`, so `"0.22 kV"` reaches an `f64[V]` with the
//! bits that the literal `0.22 kV` would have. What does not convert is a [`Problem`], whose code is one of cli.md 6.6.

use std::sync::Arc;

use labcraft_graph::{Numeral, NumeralKind};
use labcraft_ir::{ConstValue, literal_number};
use std::collections::HashMap;

use labcraft_runtime::{Items, Value, VariantValue, grid};
use labcraft_types::{Interner, LibraryId, NumberKind, Ty, TyId, TypeTable, Unit, parse_unit};
use serde_json::Value as Json;

/// The types a value is read against: those of a `Checked`, before the run is lowered, or of its `Program`.
pub(crate) struct Types<'a> {
    pub table: &'a TypeTable,
    /// What a type declared in a `.lct` file stands for; `None` for an opaque type, which stays itself.
    pub definitions: &'a HashMap<TyId, Option<TyId>>,
    pub names: &'a Interner,
    /// The library whose types are named without it, in messages.
    pub viewer: LibraryId,
}

impl Types<'_> {
    fn ty(&self, id: TyId) -> &Ty {
        match self.table.get(id) {
            Ty::Named { .. } => match self.definitions.get(&id) {
                Some(Some(definition)) => self.table.get(*definition),
                _ => self.table.get(id),
            },
            other => other,
        }
    }
}

/// A value nests at most `max_nesting` levels, like a file (`--max-nesting`), whatever its type allows.
pub(crate) struct Converter<'a> {
    pub types: Types<'a>,
    pub max_nesting: u32,
}

/// Why a value does not convert: `LC8104` (does not read), `LC8105` (unit), `LC8106` (range), `LC8107` (an opaque type
/// or a function reference, which no argument gives).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Problem {
    pub code: &'static str,
    pub message: String,
}

fn problem(code: &'static str, message: impl Into<String>) -> Problem {
    Problem { code, message: message.into() }
}

fn not_read(message: impl Into<String>) -> Problem {
    problem("LC8104", message)
}

/// A JSON value, shown in a message: cut, so that a message never repeats a large argument.
fn shown(value: &Json) -> String {
    cut(&value.to_string())
}

/// A text shown in a message, cut at 60 code points.
fn cut(text: &str) -> String {
    if text.chars().count() <= 60 { text.to_owned() } else { format!("{}…", text.chars().take(60).collect::<String>()) }
}

impl Converter<'_> {
    /// Whether a value of this type cannot be given by an argument: it holds an opaque type or a function reference
    /// (cli.md 6.3, `LC8107`), at any depth.
    pub fn holds_opaque(&self, ty: TyId) -> bool {
        let mut pending = vec![ty];
        let mut seen = Vec::new();
        while let Some(ty) = pending.pop() {
            if seen.contains(&ty) {
                continue;
            }
            seen.push(ty);
            match self.types.ty(ty) {
                Ty::Named { .. } | Ty::Function { .. } | Ty::Queue(_) | Ty::Ref(_) => return true,
                Ty::Array { element, .. } | Ty::Waveform(element) => pending.push(*element),
                Ty::Struct(fields) => pending.extend(fields.iter().map(|(_, ty)| *ty)),
                Ty::Enum(variants) => pending.extend(variants.iter().flat_map(|variant| variant.values.iter().copied())),
                _ => {}
            }
        }
        false
    }

    pub fn value(&self, json: &Json, ty: TyId) -> Result<Value, Problem> {
        if depth(json) > self.max_nesting as usize {
            return Err(not_read(format!("the value nests deeper than {} levels (--max-nesting)", self.max_nesting)));
        }
        if self.holds_opaque(ty) {
            return Err(problem("LC8107", "its type holds an opaque type or a function reference, which no argument can give"));
        }
        self.convert(json, ty)
    }

    /// A value written on the command line (cli.md 6.3): the JSON form, with two more freedoms, the quotes of what JSON
    /// writes as a string may be left out, and the space before a unit. Which applies depends on the type: the text of a
    /// `string` is the string, always; a number with a unit, a `duration`, a `timestamp` and a variant without values are
    /// strings in JSON, so their text is taken as it is unless it is quoted; an `i64` or a `u64` written as digits is
    /// the string of digits of commands.md 5; anything else is JSON. Then the value converts as `args` do.
    pub fn text_value(&self, text: &str, ty: TyId) -> Result<Value, Problem> {
        let quoted = || text.trim_start().starts_with('"');
        let as_json = |what: &str| -> Result<Json, Problem> {
            super::json_text::parse(text, self.max_nesting).map(|parsed| parsed.value).map_err(|bad| match bad {
                super::json_text::Bad::TooDeep => not_read(format!("the value nests deeper than {} levels (--max-nesting)", self.max_nesting)),
                super::json_text::Bad::OutOfRange(range) => {
                    problem("LC8106", format!("`{}` is out of the range of {}", cut(text.get(range).unwrap_or(text)), self.type_text(ty)))
                }
                _ => not_read(format!("`{}` does not read as {what}, for {}", cut(text), self.type_text(ty))),
            })
        };
        let json = match self.types.ty(ty) {
            Ty::String => Json::String(text.to_owned()),
            Ty::Number { kind, unit } if *unit == Unit::ONE => {
                let digits = text.strip_prefix('-').unwrap_or(text);
                let wide =
                    matches!(kind, NumberKind::I64 | NumberKind::U64) && !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit());
                if wide || (!kind.is_integer() && matches!(text, "nan" | "inf" | "-inf")) {
                    Json::String(text.to_owned())
                } else if kind.is_integer() && !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()) && text.parse::<i64>().is_err()
                {
                    // Digits that JSON would read as a float, `99999999999999999999`: an integer out of the range of any
                    // integer type but the wide ones, which take them above.
                    return Err(problem("LC8106", format!("`{}` is out of the range of {}", cut(text), self.type_text(ty))));
                } else {
                    as_json("a number")?
                }
            }
            Ty::Number { kind, .. } if kind.is_complex() && text.trim_start().starts_with('{') => as_json("JSON")?,
            Ty::Number { .. } | Ty::Duration | Ty::Timestamp if !quoted() => Json::String(text.to_owned()),
            Ty::Enum(_) if !quoted() && !text.trim_start().starts_with('{') => Json::String(text.to_owned()),
            Ty::ErrorValue if matches!(text, "none" | "null") => Json::Null,
            Ty::Bool => as_json("`true` or `false`")?,
            _ => as_json("JSON")?,
        };
        self.value(&json, ty)
    }

    pub fn type_text(&self, ty: TyId) -> String {
        self.types.table.display(ty, self.types.viewer, self.types.names)
    }

    fn duration(&self, json: &Json) -> Result<Value, Problem> {
        duration_of(json, self.max_nesting).map(Value::Duration)
    }

    fn convert(&self, json: &Json, ty: TyId) -> Result<Value, Problem> {
        let wrong = || not_read(format!("{} does not read as a value of {}", shown(json), self.type_text(ty)));
        match self.types.ty(ty) {
            Ty::Bool => match json {
                Json::Bool(value) => Ok(Value::Bool(*value)),
                _ => Err(wrong()),
            },
            target @ Ty::Number { kind, unit } => self.number(json, *kind, unit, target, ty),
            Ty::Duration => self.duration(json),
            Ty::String => match json {
                Json::String(text) => Ok(Value::string(text.as_str())),
                _ => Err(wrong()),
            },
            Ty::Bytes => match json.as_object().and_then(|object| (object.len() == 1).then(|| object.get("base64")).flatten()) {
                Some(Json::String(text)) => base64(text).map(|bytes| Value::Bytes(Arc::from(bytes))).ok_or_else(wrong),
                _ => Err(wrong()),
            },
            Ty::Timestamp => match json {
                Json::String(text) => rfc3339(text)
                    .map(Value::Timestamp)
                    .ok_or_else(|| not_read(format!("`{text}` is no timestamp: RFC 3339, `2026-11-08T12:00:00Z`, from 1677 to 2262"))),
                _ => Err(wrong()),
            },
            Ty::ErrorValue => match json {
                Json::Null => Ok(Value::Error(None)),
                Json::String(text) if text == "none" => Ok(Value::Error(None)),
                _ => Err(not_read("an `error` input takes `none` (or `null`) only: a run cannot be started with an error on it")),
            },
            Ty::Array { element, dimensions } => {
                let Json::Array(items) = json else { return Err(wrong()) };
                if *dimensions <= 1 {
                    let items = items.iter().map(|item| self.convert(item, *element)).collect::<Result<Vec<_>, _>>()?;
                    return Ok(Value::Array(Arc::new(Items::new(items))));
                }
                let rows = items.iter().map(|row| self.rows(row, *element, usize::from(*dimensions) - 1)).collect::<Result<Vec<_>, _>>()?;
                grid(rows, usize::from(*dimensions))
                    .ok_or_else(|| not_read(format!("the rows of {} are not all of one shape, for {}", shown(json), self.type_text(ty))))
            }
            Ty::Struct(fields) => {
                let Json::Object(given) = json else { return Err(wrong()) };
                let names: Vec<&str> = fields.iter().map(|(name, _)| self.types.names.resolve(*name)).collect();
                if let Some(unknown) = given.keys().find(|key| !names.contains(&key.as_str())) {
                    return Err(not_read(format!("{} has no field `{unknown}`", self.type_text(ty))));
                }
                let mut values = Vec::with_capacity(fields.len());
                for ((_, field_ty), name) in fields.iter().zip(&names) {
                    let Some(value) = given.get(*name) else {
                        return Err(not_read(format!("the field `{name}` of {} is missing", self.type_text(ty))));
                    };
                    values.push(self.convert(value, *field_ty)?);
                }
                Ok(Value::Struct(Arc::from(values)))
            }
            Ty::Waveform(element) => {
                let Json::Object(given) = json else { return Err(wrong()) };
                if let Some(unknown) = given.keys().find(|key| !matches!(key.as_str(), "t0" | "dt" | "samples")) {
                    return Err(not_read(format!("a waveform has no field `{unknown}`: `t0`, `dt`, `samples`")));
                }
                let field = |name: &str| given.get(name).ok_or_else(|| not_read(format!("the field `{name}` of the waveform is missing")));
                let t0 = match field("t0")? {
                    Json::String(text) => rfc3339(text).map(Value::Timestamp).ok_or_else(|| not_read(format!("`{text}` is no timestamp")))?,
                    other => return Err(not_read(format!("{} is no timestamp", shown(other)))),
                };
                let dt = self.duration(field("dt")?)?;
                let Json::Array(samples) = field("samples")? else { return Err(not_read("the samples of a waveform are an array")) };
                let samples = samples.iter().map(|sample| self.convert(sample, *element)).collect::<Result<Vec<_>, _>>()?;
                Ok(Value::Struct(Arc::from([t0, dt, Value::Array(Arc::new(Items::new(samples)))])))
            }
            Ty::Enum(variants) => {
                let (name, values): (&str, Option<&Vec<Json>>) = match json {
                    Json::String(text) => (text.as_str(), None),
                    Json::Object(object) if object.len() == 1 => match object.iter().next() {
                        Some((name, Json::Array(values))) => (name.as_str(), Some(values)),
                        _ => return Err(wrong()),
                    },
                    _ => return Err(wrong()),
                };
                let type_name = self.type_text(ty);
                // `Verdict.Pass` or `Pass`; the type named by its last segment, as the report writes it.
                let bare = match name.rsplit_once('.') {
                    Some((prefix, variant)) if prefix == type_name || type_name.ends_with(&format!(".{prefix}")) => variant,
                    Some(_) => return Err(not_read(format!("`{name}` is no variant of {type_name}"))),
                    None => name,
                };
                let found = variants.iter().enumerate().find(|(_, variant)| self.types.names.resolve(variant.name) == bare);
                let Some((index, variant)) = found else {
                    let known: Vec<&str> = variants.iter().map(|variant| self.types.names.resolve(variant.name)).collect();
                    return Err(not_read(format!("`{name}` is no variant of {type_name}: {}", known.join(", "))));
                };
                let given = values.map_or(&[][..], Vec::as_slice);
                if given.len() != variant.values.len() {
                    return Err(not_read(format!("`{bare}` holds {} values, not {}", variant.values.len(), given.len())));
                }
                let values = given.iter().zip(&variant.values).map(|(value, ty)| self.convert(value, *ty)).collect::<Result<Vec<_>, _>>()?;
                let index = u32::try_from(index).map_err(|_| wrong())?;
                Ok(Value::Variant(Arc::new(VariantValue { index, values: values.into_boxed_slice() })))
            }
            Ty::Error | Ty::Variable(_) => Err(not_read(format!("the type of this value is not settled, so {} cannot convert to it", shown(json)))),
            Ty::Named { .. } | Ty::Function { .. } | Ty::Queue(_) | Ty::Ref(_) => {
                Err(problem("LC8107", "its type holds an opaque type or a function reference, which no argument can give"))
            }
        }
    }

    /// A row of a rectangular array: one level less.
    fn rows(&self, json: &Json, element: TyId, dimensions: usize) -> Result<Value, Problem> {
        let Json::Array(items) = json else { return Err(not_read(format!("{} is not a row of an array", shown(json)))) };
        if dimensions <= 1 {
            let items = items.iter().map(|item| self.convert(item, element)).collect::<Result<Vec<_>, _>>()?;
            return Ok(Value::Array(Arc::new(Items::new(items))));
        }
        let rows = items.iter().map(|row| self.rows(row, element, dimensions - 1)).collect::<Result<Vec<_>, _>>()?;
        grid(rows, dimensions).ok_or_else(|| not_read("the rows of an array are not all of one shape"))
    }

    fn number(&self, json: &Json, kind: NumberKind, unit: &Unit, target: &Ty, ty: TyId) -> Result<Value, Problem> {
        if kind.is_complex()
            && let Json::Object(parts) = json
        {
            let part =
                |name: &str| parts.get(name).ok_or_else(|| not_read(format!("a complex number is `{{\"re\": …, \"im\": …}}`: `{name}` is missing")));
            if parts.len() != 2 {
                return Err(not_read("a complex number is `{\"re\": …, \"im\": …}`"));
            }
            let real = if kind == NumberKind::C64 { NumberKind::F32 } else { NumberKind::F64 };
            let real_ty = Ty::Number { kind: real, unit: *unit };
            let re = self.number(part("re")?, real, unit, &real_ty, ty)?;
            let im = self.number(part("im")?, real, unit, &real_ty, ty)?;
            return Ok(match (re, im) {
                (Value::F32(re), Value::F32(im)) => Value::C64(re, im),
                (Value::F64(re), Value::F64(im)) => Value::C128(re, im),
                _ => return Err(not_read("a complex number that does not read")),
            });
        }
        let text = match json {
            Json::Number(number) => number.to_string(),
            Json::String(text) => text.clone(),
            other => return Err(not_read(format!("{} is not a number, for {}", shown(other), self.type_text(ty)))),
        };
        let has_unit = *unit != Unit::ONE;
        // `args` take the JSON form exactly (decision of the owner, 2026-11-10): a number without a unit is a JSON number,
        // but for `nan`, `inf`, `-inf` and an `i64` or a `u64` written as a string of digits (commands.md 5).
        if !has_unit && json.is_string() {
            let digits = text.strip_prefix('-').unwrap_or(&text);
            let special = !kind.is_integer() && matches!(text.as_str(), "nan" | "inf" | "-inf");
            let wide = matches!(kind, NumberKind::I64 | NumberKind::U64) && !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit());
            if !special && !wide {
                return Err(not_read(format!("`\"{text}\"` is a string: a number without a unit is a JSON number, for {}", self.type_text(ty))));
            }
        }
        let (mut numeral, written) = numeral(&text, &self.type_text(ty), self.max_nesting)?;
        // Digits beyond what an integer of 128 bits holds still read as a real number.
        if !kind.is_integer()
            && let NumeralKind::Int(digits) = numeral.numeral.kind
        {
            numeral.numeral.kind = NumeralKind::Float(digits);
        }
        let written = match written {
            Some(written) => {
                if !written.same_dimension(unit) || (!has_unit && written != Unit::ONE) {
                    return Err(problem("LC8105", format!("`{text}` is not in a unit of {}", self.type_text(ty))));
                }
                written
            }
            None if has_unit => {
                return Err(problem("LC8105", format!("`{text}` has no unit, for {}: the unit is what the run cannot guess", self.type_text(ty))));
            }
            None => Unit::ONE,
        };
        if kind.is_integer() && written != *unit {
            return Err(problem("LC8105", format!("`{text}` would change the scale of an integer, for {} (spec 3.4)", self.type_text(ty))));
        }
        let finite_text = matches!(numeral.numeral.kind, NumeralKind::Int(_) | NumeralKind::Float(_));
        let converted = literal_number(numeral.numeral, &written, target, &numeral.names).map_err(|_| match numeral.numeral.kind {
            NumeralKind::Float(_) if kind.is_integer() => {
                not_read(format!("`{text}` is not an integer, for {}: write it without `.` or exponent", self.type_text(ty)))
            }
            NumeralKind::Int(_) if kind.is_integer() => problem("LC8106", format!("`{text}` is out of the range of {}", self.type_text(ty))),
            _ => not_read(format!("`{text}` does not read as {}", self.type_text(ty))),
        })?;
        let out_of_range = || problem("LC8106", format!("`{text}` is out of the range of {}", self.type_text(ty)));
        match converted {
            ConstValue::Int(value) => {
                // An i64 or a u64 beyond 2^53-1 is a string of digits (commands.md 5): a JSON number there may have lost digits.
                if matches!(json, Json::Number(number) if number.is_f64()) || (value.unsigned_abs() > (1 << 53) - 1 && json.is_number()) {
                    return Err(problem("LC8106", format!("`{text}` is not exact: an integer beyond 2^53-1 is written as a string of digits")));
                }
                labcraft_runtime::integer(kind, value).ok_or_else(out_of_range)
            }
            ConstValue::Float(x) => {
                if finite_text && !x.is_finite() {
                    return Err(out_of_range());
                }
                Ok(match kind {
                    NumberKind::F32 => Value::F32(x as f32),
                    NumberKind::C64 => Value::C64(x as f32, 0.0),
                    NumberKind::C128 => Value::C128(x, 0.0),
                    _ => Value::F64(x),
                })
            }
            ConstValue::Complex { re, im } => {
                if finite_text && !(re.is_finite() && im.is_finite()) {
                    return Err(out_of_range());
                }
                Ok(if kind == NumberKind::C64 { Value::C64(re as f32, im as f32) } else { Value::C128(re, im) })
            }
            _ => Err(not_read(format!("`{text}` does not read as {}", self.type_text(ty)))),
        }
    }
}

/// A duration: a string with a unit of time, `"100 ms"`; a number alone is refused, the unit being what a run cannot guess.
fn duration_of(json: &Json, max_nesting: u32) -> Result<i64, Problem> {
    // A JSON number has no unit (`LC8105`); a `bool`, `null`, an array or an object does not read as a duration at all
    // (`LC8104`), as for a number with a unit (decision of the owner, 2026-11-13).
    let text = match json {
        Json::String(text) => text,
        Json::Number(_) => return Err(problem("LC8105", format!("{} has no unit: a duration is written with one, `\"100 ms\"`", shown(json)))),
        other => return Err(problem("LC8104", format!("{} is not a duration: it is written with a unit of time, `\"100 ms\"`", shown(other)))),
    };
    let (numeral, unit) = numeral(text, "a duration", max_nesting)?;
    let Some(unit) = unit else {
        return Err(problem("LC8105", format!("`{text}` has no unit: a duration is written with one, `100 ms`")));
    };
    if !unit.same_dimension(&Unit::SECOND) {
        return Err(problem("LC8105", format!("`{text}` is not a time, for a duration")));
    }
    match literal_number(numeral.numeral, &unit, &Ty::Duration, &numeral.names) {
        Ok(ConstValue::Duration(ns)) => Ok(ns),
        _ => Err(problem("LC8106", format!("`{text}` is no duration: a whole number of nanoseconds that an i64 holds"))),
    }
}

/// A duration given to a command, `wait`, `timeout` or the time of a setting: `"30 s"`, `"500 ms"` (commands.md 5).
pub(crate) fn duration_text(text: &str, max_nesting: u32) -> Result<i64, Problem> {
    duration_of(&Json::String(text.to_owned()), max_nesting)
}

/// The number of a literal and its unit, if it has one: `230 V`, `230V`, `-0.5`, `1e3 mV`, `inf`, `-nan`.
fn numeral(text: &str, what: &str, max_nesting: u32) -> Result<(Lexed, Option<Unit>), Problem> {
    let lexed = lex(text).ok_or_else(|| not_read(format!("`{text}` does not read as a number, for {what}")))?;
    let rest = lexed.rest.trim();
    if rest.is_empty() {
        return Ok((lexed, None));
    }
    match parse_unit(rest, max_nesting) {
        Ok(unit) => Ok((lexed, Some(unit))),
        Err(_) => Err(not_read(format!("`{rest}` is no unit, in `{text}`"))),
    }
}

/// A numeral, with the names its digits are interned in, and what follows it.
struct Lexed {
    numeral: Numeral,
    names: Interner,
    rest: String,
}

/// INT and FLOAT of spec 14, with a sign, or `inf` and `nan`; what follows is the unit.
fn lex(text: &str) -> Option<Lexed> {
    let text = text.trim_start();
    let (negative, body) = match text.strip_prefix('-') {
        Some(body) => (true, body),
        None => (false, text),
    };
    let mut names = Interner::new();
    for (word, kind) in [("inf", NumeralKind::Inf), ("nan", NumeralKind::Nan)] {
        // `-nan` is no number of the language (spec 14).
        if negative && kind == NumeralKind::Nan {
            continue;
        }
        if let Some(rest) = body.strip_prefix(word)
            && !rest.starts_with(|c: char| c.is_alphanumeric() || c == '_')
        {
            return Some(Lexed { numeral: Numeral { negative, kind }, names, rest: rest.to_owned() });
        }
    }
    let bytes = body.as_bytes();
    let int = |from: usize| -> usize {
        let mut at = from;
        if bytes.get(at).is_some_and(u8::is_ascii_digit) {
            while bytes.get(at).is_some_and(|byte| byte.is_ascii_digit() || *byte == b'_') {
                at += 1;
            }
        }
        at
    };
    let mut end = int(0);
    if end == 0 {
        return None;
    }
    let mut float = false;
    if bytes.get(end) == Some(&b'.') {
        let after = int(end + 1);
        if after == end + 1 {
            return None;
        }
        end = after;
        float = true;
    }
    if matches!(bytes.get(end), Some(b'e' | b'E')) {
        let mut at = end + 1;
        // `1e+20`: how JSON writes a large number, as `serde_json` gives it back.
        if matches!(bytes.get(at), Some(b'-' | b'+')) {
            at += 1;
        }
        let after = int(at);
        // `1e` is a number and the unit `e`? No unit is named `e`: an exponent must have digits.
        if after > at {
            end = after;
            float = true;
        }
    }
    let digits = body.get(..end)?;
    let name = names.intern(digits);
    let kind = if float { NumeralKind::Float(name) } else { NumeralKind::Int(name) };
    Some(Lexed { numeral: Numeral { negative, kind }, names, rest: body.get(end..)?.to_owned() })
}

fn depth(json: &Json) -> usize {
    let mut deepest = 0;
    let mut pending = vec![(json, 1_usize)];
    while let Some((value, level)) = pending.pop() {
        match value {
            Json::Array(items) => {
                deepest = deepest.max(level);
                pending.extend(items.iter().map(|item| (item, level + 1)));
            }
            Json::Object(fields) => {
                deepest = deepest.max(level);
                pending.extend(fields.values().map(|item| (item, level + 1)));
            }
            _ => deepest = deepest.max(level.saturating_sub(1)),
        }
    }
    deepest
}

/// Standard base64, with its padding (RFC 4648, 4).
fn base64(text: &str) -> Option<Vec<u8>> {
    let value = |byte: u8| -> Option<u32> {
        Some(u32::from(match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        }))
    };
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for (index, chunk) in bytes.chunks(4).enumerate() {
        let last = index + 1 == bytes.len() / 4;
        let pad = chunk.iter().rev().take_while(|byte| **byte == b'=').count();
        if pad > 2 || (pad > 0 && !last) {
            return None;
        }
        let mut word = 0_u32;
        for byte in chunk.iter().take(4 - pad) {
            word = (word << 6) | value(*byte)?;
        }
        word <<= 6 * pad as u32;
        let three = [(word >> 16) as u8, (word >> 8) as u8, word as u8];
        out.extend(three.iter().take(3 - pad));
    }
    Some(out)
}

/// RFC 3339 to nanoseconds since 1970-01-01T00:00:00Z, which an i64 holds: `2026-11-08T12:00:00.000000250Z`,
/// `2026-11-08T13:00:00+01:00`.
fn rfc3339(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    let number = |from: usize, len: usize| -> Option<i64> {
        let digits = text.get(from..from + len)?;
        digits.bytes().all(|byte| byte.is_ascii_digit()).then(|| digits.parse().ok()).flatten()
    };
    let (year, month, day) = (number(0, 4)?, number(5, 2)?, number(8, 2)?);
    let (hour, minute, second) = (number(11, 2)?, number(14, 2)?, number(17, 2)?);
    if bytes.get(4) != Some(&b'-') || bytes.get(7) != Some(&b'-') || !matches!(bytes.get(10), Some(b'T' | b't' | b' ')) {
        return None;
    }
    if bytes.get(13) != Some(&b':') || bytes.get(16) != Some(&b':') {
        return None;
    }
    let mut at = 19;
    let mut nanos = 0_i64;
    if bytes.get(at) == Some(&b'.') {
        at += 1;
        let start = at;
        while bytes.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
        }
        let fraction = text.get(start..at)?;
        if fraction.is_empty() || fraction.len() > 9 {
            return None;
        }
        nanos = format!("{fraction:0<9}").parse().ok()?;
    }
    let offset = match text.get(at..)? {
        "Z" | "z" => 0,
        zone => {
            let sign = match zone.as_bytes().first()? {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            if zone.len() != 6 || zone.as_bytes().get(3) != Some(&b':') {
                return None;
            }
            sign * (number(at + 1, 2)? * 3600 + number(at + 4, 2)? * 60)
        }
    };
    let days_in_month = [31, if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let month_days = *days_in_month.get(usize::try_from(month).ok()?.checked_sub(1)?)?;
    if !(1..=month_days).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    // Days from the civil date (H. Hinnant's algorithm, public domain).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let seconds = days.checked_mul(86_400)?.checked_add(hour * 3600 + minute * 60 + second)?.checked_sub(offset)?;
    seconds.checked_mul(1_000_000_000)?.checked_add(nanos)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_reads_the_examples_of_rfc_4648() {
        for (text, bytes) in [("", ""), ("Zg==", "f"), ("Zm8=", "fo"), ("Zm9v", "foo"), ("Zm9vYg==", "foob"), ("Zm9vYmFy", "foobar")] {
            assert_eq!(base64(text).as_deref(), Some(bytes.as_bytes()), "{text}");
        }
        assert_eq!(base64("Zg="), None);
        assert_eq!(base64("Z==="), None);
    }

    /// 2026-11-08 is day 20,765 since 1970-01-01, as Python's `datetime` counts it (`(date(2026, 11, 8) - date(1970, 1, 1)).days`).
    #[test]
    fn rfc_3339_gives_nanoseconds_since_1970() {
        assert_eq!(rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(rfc3339("1970-01-01T00:00:00.100000000Z"), Some(100_000_000));
        assert_eq!(rfc3339("2026-11-08T12:00:00.000000250Z"), Some((20_765 * 86_400 + 12 * 3600) * 1_000_000_000 + 250));
        assert_eq!(rfc3339("2026-11-08T13:00:00+01:00"), rfc3339("2026-11-08T12:00:00Z"));
        assert_eq!(rfc3339("1969-12-31T23:59:59Z"), Some(-1_000_000_000));
        assert_eq!(rfc3339("2026-02-29T00:00:00Z"), None, "2026 is no leap year");
        assert_eq!(rfc3339("2026-11-08"), None);
        assert_eq!(rfc3339("3000-01-01T00:00:00Z"), None, "past what an i64 of nanoseconds holds");
    }

    #[test]
    fn a_numeral_is_read_with_its_unit_and_its_sign() {
        let lexed = lex("230V").unwrap();
        assert_eq!((lexed.numeral.negative, lexed.rest.as_str()), (false, "V"));
        let lexed = lex("-0.22 kV").unwrap();
        assert!(lexed.numeral.negative && matches!(lexed.numeral.kind, NumeralKind::Float(_)));
        assert_eq!(lexed.rest, " kV");
        assert!(matches!(lex("1e3 mV").unwrap().numeral.kind, NumeralKind::Float(_)));
        assert!(matches!(lex("-inf").unwrap().numeral.kind, NumeralKind::Inf));
        assert!(matches!(lex("10").unwrap().numeral.kind, NumeralKind::Int(_)));
        assert!(lex("V").is_none() && lex("1.").is_none() && lex(".5").is_none());
    }

    #[test]
    fn depth_counts_arrays_and_objects() {
        assert_eq!(depth(&serde_json::json!(1)), 0);
        assert_eq!(depth(&serde_json::json!([1, 2])), 1);
        assert_eq!(depth(&serde_json::json!({ "a": [[1]] })), 3);
    }
}
