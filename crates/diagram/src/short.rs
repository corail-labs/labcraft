//! The short form of a value of a run, as its bubble shows it (notation 6.1), from the JSON text that the trace gave and
//! its type as the trace writes it (`docs/commands.md`, 5). It reads the text with the scanner of `trace`, which does not
//! recurse, and builds no tree: a value is at most 64 KiB, and only its first level and a few fields are looked at.

use labcraft_trace::json::Scanner;
use labcraft_types::text::cut;

/// The levels the scanner reads: those of a line that the reader of a trace reads (`ReadLimits::nesting`), more than a
/// value of a trace nests (`VALUE_DEPTH`), so that a value it kept always reads.
pub(crate) const DEPTH: usize = 256;
/// A string, a resource, a category: whole up to 12 cells, beyond that 11 and `…` (notation 6.1).
const STRING_CELLS: u64 = 12;

/// What a bubble shows of a value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Short {
    pub text: String,
    /// An error that carries an error: its bubble shows its category in `wire.error`.
    pub error: bool,
}

impl Short {
    fn plain(text: String) -> Option<Short> {
        Some(Short { text, error: false })
    }
}

/// The short form of `json`, a value of type `ty`; `None` for a value that shows nothing: an error that carries none, or a
/// text that does not read.
pub fn short(json: &str, ty: &str) -> Option<Short> {
    // Whatever it holds, a unit of 60 KiB included, a bubble shows at most the cells of any text (notation 2.3).
    let cut_short = |short: Short| Short { text: cut(&short.text, ANY_TEXT).into_owned(), ..short };
    shown(json.trim(), ty).map(cut_short)
}

/// The cells of a text that no other rule cuts (`text.any_text`, notation 10).
const ANY_TEXT: u64 = 40;

fn shown(raw: &str, ty: &str) -> Option<Short> {
    match raw.bytes().next()? {
        b'n' => None,
        b't' | b'f' => Short::plain(raw.to_owned()),
        b'"' => {
            let text = Scanner::new(raw, DEPTH).string().ok()?;
            string_form(&text, ty)
        }
        b'-' | b'0'..=b'9' => Short::plain(number(raw, ty)),
        b'[' => Short::plain(array(raw, dimensions(ty))?),
        b'{' => object(raw, ty),
        _ => None,
    }
}

/// The base of a type: `f64` for `f64[V]`.
fn kind_of(ty: &str) -> &str {
    ty.split('[').next().unwrap_or(ty)
}

fn is_integer(ty: &str) -> bool {
    matches!(kind_of(ty), "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64")
}

fn is_number(ty: &str) -> bool {
    is_integer(ty) || matches!(kind_of(ty), "f32" | "f64" | "c64" | "c128")
}

/// The dimensions of an array type: `[f64]` 1, `[f64; 2]` 2; 1 when it does not say.
fn dimensions(ty: &str) -> u32 {
    ty.strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .and_then(|inner| inner.rsplit_once("; "))
        .and_then(|(_, count)| count.parse().ok())
        .unwrap_or(1)
}

/// A string of the trace, by its type: a text in quotes, a number with its unit, a duration, a time of day, a variant.
fn string_form(text: &str, ty: &str) -> Option<Short> {
    match kind_of(ty) {
        "string" => Short::plain(quoted(text)),
        "duration" => Short::plain(with_unit(text, false)),
        "timestamp" => Short::plain(time_of_day(text)),
        _ if is_number(ty) => Short::plain(with_unit(text, is_integer(ty))),
        // A variant without values, `Verdict.Pass`, or a type that an error left unknown.
        _ => Short::plain(cut(text, 40).into_owned()),
    }
}

/// A text in quotes, whole up to 12 cells, beyond that 11 and `…`, each control as the notation shows it (a line break
/// `↵`, notation 2.3), counted in the cells of its sign.
fn quoted(text: &str) -> String {
    format!("\"{}\"", labcraft_types::text::show(text, STRING_CELLS).text())
}

/// A name or a text from the trace, cut as a string is, without quotes.
pub fn cut_name(text: &str) -> String {
    cut(text, STRING_CELLS).into_owned()
}

/// `230.071217 V` as `230.1 V`; an integer in full; `nan`, `inf` as they are.
fn with_unit(text: &str, integer: bool) -> String {
    let (value, unit) = text.split_once(' ').unwrap_or((text, ""));
    let shown = if integer { value.to_owned() } else { float_text(value) };
    if unit.is_empty() { shown } else { format!("{shown} {unit}") }
}

/// A number as text, in short; as it is when it is not one.
fn float_text(text: &str) -> String {
    text.parse::<f64>().map_or_else(|_| text.to_owned(), float)
}

/// A number of JSON: an integer in full, a float in short.
fn number(raw: &str, ty: &str) -> String {
    if is_integer(ty) && !raw.contains(['.', 'e', 'E']) {
        return raw.to_owned();
    }
    float_text(raw)
}

/// A float with 4 significant digits, zeros after the decimal point dropped; an exponent when its magnitude is below 0.001
/// or from 1,000,000 on; `0` as `0` (notation 6.1).
pub fn float(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_owned();
    }
    if value.is_infinite() {
        return if value > 0.0 { "inf" } else { "-inf" }.to_owned();
    }
    if value == 0.0 {
        return "0".to_owned();
    }
    let magnitude = value.abs();
    if !(0.001..1_000_000.0).contains(&magnitude) {
        // `1.200e-6` as `1.2e-6`.
        let text = format!("{value:.3e}");
        let (mantissa, exponent) = text.split_once('e').unwrap_or((&text, "0"));
        return format!("{}e{exponent}", trimmed(mantissa));
    }
    let exponent = magnitude.log10().floor() as i32;
    if exponent >= 4 {
        // 12345 as 12350: rounded to its fourth digit, half away from zero.
        let step = 10f64.powi(exponent - 3);
        return format!("{:.0}", (value / step).round() * step);
    }
    let decimals = usize::try_from(3 - exponent).unwrap_or(0);
    trimmed(&format!("{value:.decimals$}")).to_owned()
}

/// Zeros after the decimal point dropped, and the point when nothing follows it.
fn trimmed(text: &str) -> &str {
    if text.contains('.') { text.trim_end_matches('0').trim_end_matches('.') } else { text }
}

/// `2026-11-08T14:02:31.250000000Z` as `14:02:31.250Z`.
fn time_of_day(text: &str) -> String {
    let Some((_, time)) = text.split_once('T') else { return cut_name(text) };
    let time = time.trim_end_matches('Z');
    let (clock, fraction) = time.split_once('.').unwrap_or((time, ""));
    let millis: String = fraction.chars().chain(std::iter::repeat('0')).take(3).collect();
    cut(&format!("{clock}.{millis}Z"), 40).into_owned()
}

/// `[3 elements]`, `[3×4]` for more than one dimension, `[… elements]` when the trace cut it without its length.
fn array(raw: &str, dimensions: u32) -> Option<String> {
    let (count, cut, first) = elements(raw)?;
    if cut {
        return Some("[\u{2026} elements]".to_owned());
    }
    Some(sized(count, dimensions, first))
}

/// An array by its length, or its sizes when it has more than one dimension, the inner ones read from its first row.
fn sized(count: u64, dimensions: u32, first: Option<&str>) -> String {
    if dimensions > 1 {
        let mut sizes = vec![count.to_string()];
        let mut row = first;
        for _ in 1..dimensions.min(8) {
            let inner = row.and_then(|row| {
                let trimmed = row.trim();
                if trimmed.starts_with('{') {
                    head_of(trimmed).map(|(len, head)| (len, Some(head)))
                } else {
                    elements(trimmed).map(|(n, _, f)| (n, f))
                }
            });
            let Some((length, next)) = inner else {
                sizes.push("\u{2026}".to_owned());
                break;
            };
            sizes.push(length.to_string());
            row = next.and_then(|head| elements(head).and_then(|(_, _, first)| first)).or(next);
        }
        return format!("[{}]", sizes.join("\u{D7}"));
    }
    match count {
        1 => "[1 element]".to_owned(),
        _ => format!("[{count} elements]"),
    }
}

/// The elements of an array: how many, whether the trace cut it (`{ "$cut": true }` in the place of the rest), and the
/// text of the first.
fn elements(raw: &str) -> Option<(u64, bool, Option<&str>)> {
    let mut scanner = Scanner::new(raw, DEPTH);
    scanner.begin_array().ok()?;
    let mut first_flag = true;
    let (mut count, mut cut_here, mut first) = (0u64, false, None);
    while scanner.element(&mut first_flag).ok()? {
        let value = scanner.value().ok()?;
        if is_cut_mark(value) {
            cut_here = true;
            continue;
        }
        if first.is_none() {
            first = Some(value);
        }
        count = count.saturating_add(1);
    }
    Some((count, cut_here, first))
}

/// `{ "$cut": true }`, the place of what the trace cut, as the writer writes it.
pub(crate) fn is_cut_mark(value: &str) -> bool {
    value.trim() == labcraft_trace::value::CUT
}

/// `{ "$len": N, "$head": … }`: N and the text of the head.
pub(crate) fn head_of(raw: &str) -> Option<(u64, &str)> {
    let mut scanner = Scanner::new(raw, DEPTH);
    scanner.begin_object().ok()?;
    let mut first = true;
    let (mut len, mut head) = (None, None);
    while let Some(key) = scanner.key(&mut first).ok()? {
        match key.as_ref() {
            "$len" => len = Some(scanner.uint().ok()?),
            "$head" => head = Some(scanner.value().ok()?),
            _ => {
                scanner.value().ok()?;
            }
        }
    }
    Some((len?, head?))
}

/// An object: a cut value, an opaque value, a function, a complex number, bytes, an error, a waveform, a variant with
/// values, or a struct.
fn object(raw: &str, ty: &str) -> Option<Short> {
    let mut scanner = Scanner::new(raw, DEPTH);
    scanner.begin_object().ok()?;
    let mut first = true;
    let mut fields: Vec<(std::borrow::Cow<'_, str>, &str)> = Vec::new();
    while let Some(key) = scanner.key(&mut first).ok()? {
        let value = scanner.value().ok()?;
        // Enough to tell what it is: a struct of many fields is `{…}` whatever they hold.
        if fields.len() < 8 {
            fields.push((key, value));
        }
    }
    let field = |name: &str| fields.iter().find(|(key, _)| key == name).map(|(_, value)| *value);
    let string = |value: &str| Scanner::new(value, DEPTH).string().ok().map(std::borrow::Cow::into_owned);
    if let Some(len) = field("$len") {
        let len: u64 = len.trim().parse().ok()?;
        let head = field("$head").unwrap_or("");
        return Short::plain(match head.trim().bytes().next() {
            Some(b'"') => format!(
                "\"{}\u{2026}\"",
                string(head).map(|text| cut(&text, STRING_CELLS - 1).trim_end_matches('\u{2026}').to_owned()).unwrap_or_default()
            ),
            Some(b'{') => format!("[{len} bytes]"),
            _ => sized(len, dimensions(ty), elements(head).and_then(|(_, _, first)| first)),
        });
    }
    if field("$cut").is_some() {
        return Short::plain("\u{2026}".to_owned());
    }
    if let Some(opaque) = field("opaque") {
        let open = field("open").is_none_or(|open| open.trim() != "false");
        return Short::plain(match field("resource").and_then(string) {
            Some(resource) if open => cut_name(&resource),
            Some(resource) => format!("{} (closed)", cut_name(&resource)),
            None => format!("\u{27E8}{}\u{27E9}", string(opaque).map(|name| cut(&name, 40).into_owned()).unwrap_or_default()),
        });
    }
    if let Some(function) = field("function") {
        return Short::plain(match string(function) {
            Some(name) => format!("\u{192} {}", cut(&name, 40)),
            None => "\u{192} none".to_owned(),
        });
    }
    if let (Some(re), Some(im)) = (field("re"), field("im")) {
        return Short::plain(complex(re, im));
    }
    if let Some(base64) = field("base64") {
        let text = string(base64).unwrap_or_default();
        let padding = text.bytes().rev().take_while(|byte| *byte == b'=').count();
        let bytes = (text.len() / 4 * 3).saturating_sub(padding);
        return Short::plain(format!("[{bytes} bytes]"));
    }
    if kind_of(ty) == "error" || field("category").is_some() && field("origin").is_some() {
        let category = field("category").and_then(string).unwrap_or_default();
        return Some(Short { text: cut_name(&category), error: true });
    }
    if let Some(samples) = field("samples").filter(|_| ty.starts_with("waveform")) {
        let trimmed = samples.trim();
        let count = if trimmed.starts_with('{') { head_of(trimmed).map(|(len, _)| len) } else { elements(trimmed).map(|(count, _, _)| count) };
        return Short::plain(match count {
            Some(1) => "[1 sample]".to_owned(),
            Some(count) => format!("[{count} samples]"),
            None => "[\u{2026} samples]".to_owned(),
        });
    }
    // A variant with values: an object of one key, `{ "Reading.Value": [...] }`.
    if let [(key, value)] = fields.as_slice()
        && key.contains('.')
        && value.trim_start().starts_with('[')
        && !ty.starts_with("struct")
    {
        return Short::plain(format!("{}(\u{2026})", cut(key, 40)));
    }
    Short::plain("{\u{2026}}".to_owned())
}

/// `1.5+2i`, with its unit.
fn complex(re: &str, im: &str) -> String {
    let part = |raw: &str| -> (String, String) {
        let raw = raw.trim();
        let text = if raw.starts_with('"') {
            Scanner::new(raw, DEPTH).string().map(std::borrow::Cow::into_owned).unwrap_or_default()
        } else {
            raw.to_owned()
        };
        let (value, unit) = text.split_once(' ').unwrap_or((&text, ""));
        (float_text(value), unit.to_owned())
    };
    let ((re, unit), (im, _)) = (part(re), part(im));
    let sign = if im.starts_with('-') { "" } else { "+" };
    if unit.is_empty() { format!("{re}{sign}{im}i") } else { format!("{re}{sign}{im}i {unit}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(json: &str, ty: &str) -> String {
        short(json, ty).map(|short| short.text).unwrap_or_else(|| "(nothing)".to_owned())
    }

    /// Notation 6.1, the short forms, each worked out by hand from its rule.
    #[test]
    fn each_value_has_its_short_form() {
        // Four significant digits, then the unit.
        assert_eq!(text("\"230.071217 V\"", "f64[V]"), "230.1 V");
        assert_eq!(text("0.5", "f64"), "0.5");
        assert_eq!(text("\"0.5 s\"", "duration"), "0.5 s");
        assert_eq!(text("\"12345.0 Hz\"", "f64[Hz]"), "12350 Hz");
        assert_eq!(text("\"0.0000012 A\"", "f64[A]"), "1.2e-6 A");
        assert_eq!(text("1250000.0", "f64"), "1.25e6");
        assert_eq!(text("0.0", "f64"), "0");
        assert_eq!(text("\"nan\"", "f64"), "nan");
        assert_eq!(text("\"-inf V\"", "f64[V]"), "-inf V");
        assert_eq!(text("2.0", "f64"), "2");
        assert_eq!(text("460.04027199999996", "f64"), "460");
        // An integer in full.
        assert_eq!(text("1250000", "i64"), "1250000");
        assert_eq!(text("\"-3 mV\"", "i32[mV]"), "-3 mV");
        assert_eq!(text("\"9007199254740993\"", "u64"), "9007199254740993");
        assert_eq!(text(r#"{"re":1.5,"im":2.0}"#, "c64"), "1.5+2i");
        assert_eq!(text(r#"{"re":"1.0 V","im":"-0.5 V"}"#, "c64[V]"), "1-0.5i V");
        assert_eq!(text("\"2026-11-08T14:02:31.250000000Z\"", "timestamp"), "14:02:31.250Z");
        // A string in quotes, 12 cells at most, a line break as `↵`.
        assert_eq!(text("\"MEAS:VOLT\"", "string"), "\"MEAS:VOLT\"");
        assert_eq!(text("\"MEAS:VOLT:DC?\"", "string"), "\"MEAS:VOLT:D\u{2026}\"");
        assert_eq!(text("\"a\\nb\"", "string"), "\"a\u{21B5}b\"");
        assert_eq!(text(r#"{"base64":"AAECAwQFBgcICQoLDA0ODw=="}"#, "bytes"), "[16 bytes]");
        assert_eq!(text("true", "bool"), "true");
        assert_eq!(text("\"Verdict.Pass\"", "Verdict"), "Verdict.Pass");
        assert_eq!(text(r#"{"Reading.Value":["230.0 V"]}"#, "Reading"), "Reading.Value(\u{2026})");
        // Arrays by their length or their sizes.
        assert_eq!(text("[1.0]", "[f64]"), "[1 element]");
        assert_eq!(text("[1.0, 2.0, 3.0]", "[f64]"), "[3 elements]");
        assert_eq!(text("[[1, 2, 3, 4], [5, 6, 7, 8], [9, 10, 11, 12]]", "[i32; 2]"), "[3\u{D7}4]");
        assert_eq!(text(r#"{"$len":100000,"$head":[1,2,3]}"#, "[i32]"), "[100000 elements]");
        assert_eq!(text(r#"[1, 2, {"$cut":true}]"#, "[i32]"), "[\u{2026} elements]");
        assert_eq!(
            text(r#"{"t0":"1970-01-01T00:00:00.000000000Z","dt":"0.001 s","samples":{"$len":1000,"$head":[0.0]}}"#, "waveform[f64]"),
            "[1000 samples]"
        );
        assert_eq!(text(r#"{"low":"220.0 V","high":"240.0 V"}"#, "Limits"), "{\u{2026}}");
        // A session by its resource; another opaque value by its type; a function by its name.
        assert_eq!(text(r#"{"opaque":"visa.session","resource":"SIM::DMM0","open":true}"#, "visa.session"), "SIM::DMM0");
        assert_eq!(text(r#"{"opaque":"visa.session","resource":"SIM::DMM0","open":false}"#, "visa.session"), "SIM::DMM0 (closed)");
        assert_eq!(text(r#"{"opaque":"queue"}"#, "queue[f64]"), "\u{27E8}queue\u{27E9}");
        assert_eq!(text(r#"{"function":"scale"}"#, "fn(f64) -> f64"), "\u{192} scale");
        // An error that carries none shows nothing; one that carries an error, its category.
        assert_eq!(text("null", "error"), "(nothing)");
        let error = short(r#"{"category":"io.timeout","code":-1073807339,"message":"no answer","origin":{},"others":[]}"#, "error");
        assert_eq!(error, Some(Short { text: "io.timeout".to_owned(), error: true }));
    }

    #[test]
    fn a_text_that_does_not_read_shows_nothing() {
        for bad in ["", "{", "[1,", "\"unterminated", "nul", "{\"a\":}"] {
            assert_eq!(short(bad, "f64"), None, "{bad}");
        }
    }
}
