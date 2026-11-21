//! The reader of a section of raw devices in a simulation file (`docs/io.md`, section 6): `[[LIBRARY.device]]`, each with
//! the keys that every stream has, judged before the run. Each problem is a diagnostic with the code of the library that reads
//! its section, at its key or its value, and every problem of the file is said, not the first only.

use std::collections::{BTreeMap, HashSet};
use std::ops::{Range, RangeInclusive};

use labcraft_types::Severity;
use toml::Spanned;
use toml::de::{DeTable, DeValue};

use super::device::{DeviceSpec, Reply};
use super::shown;
use crate::native::{LibraryDiagnostic, Section};
use crate::simulation::{Places, float_of, integer_of, nearest};

/// The codes of a library for the problems this reader finds: they are numbered differently in each library.
#[derive(Clone, Copy, Debug)]
pub struct Codes {
    /// A key that the format does not have; fix: the nearest key.
    pub unknown_key: &'static str,
    /// A value of the wrong TOML type.
    pub wrong_type: &'static str,
    /// A device without `endpoint`, or one that is not an endpoint of the library.
    pub endpoint: &'static str,
    /// An endpoint defined again.
    pub again: &'static str,
    /// A time below 0 or above one day, or another value out of its range.
    pub range: &'static str,
    /// A request or a response given in both forms or, for a request, in neither; an empty request; hexadecimal that is not
    /// pairs of digits; more than 65,536 bytes.
    pub bytes: &'static str,
    /// A request that equals another request of the same device, or is the start of one.
    pub overlap: &'static str,
    /// More than 256 devices, or 10,000 exchanges in one device.
    pub bound: &'static str,
}

/// At most this many bytes in a request or a response *(proposed)*.
const MAX_BYTES: usize = 65_536;
/// At most this many devices in a section *(proposed)*.
const MAX_DEVICES: usize = 256;
/// At most this many exchanges in a device *(proposed)*.
const MAX_EXCHANGES: usize = 10_000;
/// One day, the longest time of a file (simulation.md 3).
const DAY_MS: f64 = 86_400_000.0;
const REPLY_KEYS: &[&str] = &["request", "request_hex", "response", "response_hex"];

/// A value of a TOML table, with its place.
pub type TomlValue<'a> = Spanned<DeValue<'a>>;

/// A time of the file in milliseconds, as nanoseconds (simulation.md 3): times 10^6 as an `f64`, then to the nearest integer,
/// ties to even. A time is at most one day, so it fits.
fn nanoseconds(ms: f64) -> i64 {
    (ms * 1e6).round_ties_even() as i64
}

/// A request or a response of the file.
enum Given {
    Absent,
    Bytes(Vec<u8>, Range<usize>),
    /// Its problem is said.
    Wrong,
}

/// What reads a section: where each byte of the file is, the problems found so far, and the codes of the library.
pub struct Reader<'s> {
    places: &'s Places<'s>,
    problems: Vec<LibraryDiagnostic>,
    codes: Codes,
}

impl<'s> Reader<'s> {
    fn new(places: &'s Places<'s>, codes: Codes) -> Self {
        Reader { places, problems: Vec::new(), codes }
    }

    pub fn say(&mut self, code: &'static str, message: String, at: Range<usize>) {
        self.problems.push(LibraryDiagnostic { code, severity: Severity::Error, message, span: Some(self.places.span(at)), fix: None });
    }

    /// The value as a table, or the code of a wrong type.
    pub fn table<'v, 'a>(&mut self, value: &'v TomlValue<'a>, what: &str) -> Option<&'v DeTable<'a>> {
        match value.get_ref() {
            DeValue::Table(table) => Some(table),
            other => {
                self.say(self.codes.wrong_type, format!("{what} is a table, and this is {}", other.type_str()), value.span());
                None
            }
        }
    }

    /// The value as an array, or the code of a wrong type.
    fn array<'v, 'a>(&mut self, value: &'v TomlValue<'a>, what: &str) -> Option<&'v [TomlValue<'a>]> {
        match value.get_ref() {
            DeValue::Array(items) => Some(items.as_ref()),
            other => {
                self.say(self.codes.wrong_type, format!("{what} is an array of tables, and this is {}", other.type_str()), value.span());
                None
            }
        }
    }

    /// The keys of a table that the format has; each other one is the code of an unknown key, with the nearest key.
    pub fn keys(&mut self, table: &DeTable<'_>, known: &[&str], what: &str) {
        for (key, _) in table.iter() {
            let name = key.get_ref().as_ref();
            if !known.contains(&name) {
                let fix = nearest(name, known);
                let hint = fix.map_or_else(String::new, |fix| format!(": did you mean `{fix}`?"));
                self.problems.push(LibraryDiagnostic {
                    code: self.codes.unknown_key,
                    severity: Severity::Error,
                    message: format!("{what} has no key `{}`{hint}", shown(name)),
                    span: Some(self.places.span(key.span())),
                    fix: fix.map(str::to_owned),
                });
            }
        }
    }

    /// A time in milliseconds, from 0 to one day (simulation.md 3), as nanoseconds; `None` when absent or wrong.
    pub fn time(&mut self, table: &DeTable<'_>, key: &str, what: &str) -> Option<i64> {
        let value = get(table, key)?;
        let read = match value.get_ref() {
            DeValue::Integer(integer) => integer_of(integer).map(|x| x as f64),
            DeValue::Float(float) => float_of(float),
            other => {
                self.say(self.codes.wrong_type, format!("{what}: `{key}` is a number, and this is {}", other.type_str()), value.span());
                return None;
            }
        };
        match read {
            Some(x) if x.is_finite() && (0.0..=DAY_MS).contains(&x) => Some(nanoseconds(x)),
            _ => {
                self.say(self.codes.range, format!("{what}: `{key}` is a time from 0 to one day, 86,400,000 ms"), value.span());
                None
            }
        }
    }

    /// An integer in `range`, or `None` when absent or once its problem is said: the wrong type, or out of `range`, which
    /// `says` names (`"1 or 2"`).
    pub fn integer(&mut self, table: &DeTable<'_>, key: &str, what: &str, range: RangeInclusive<i64>, says: &str) -> Option<i64> {
        let value = get(table, key)?;
        let DeValue::Integer(integer) = value.get_ref() else {
            self.say(self.codes.wrong_type, format!("{what}: `{key}` is an integer, and this is {}", value.get_ref().type_str()), value.span());
            return None;
        };
        let found = integer_of(integer).filter(|number| range.contains(number));
        if found.is_none() {
            self.say(self.codes.range, format!("{what}: `{key}` is {says}"), value.span());
        }
        found
    }

    /// One of `words`, by its text, or `None` when absent or once its problem is said: the wrong type, or another word.
    pub fn word<T: Copy>(&mut self, table: &DeTable<'_>, key: &str, what: &str, words: &[(&str, T)]) -> Option<T> {
        let value = get(table, key)?;
        let DeValue::String(text) = value.get_ref() else {
            self.say(self.codes.wrong_type, format!("{what}: `{key}` is a text, and this is {}", value.get_ref().type_str()), value.span());
            return None;
        };
        let found = words.iter().find(|(word, _)| *word == text.as_ref()).map(|(_, value)| *value);
        if found.is_none() {
            let listed: Vec<String> = words.iter().map(|(word, _)| format!("`\"{word}\"`")).collect();
            self.say(self.codes.range, format!("{what}: `{key}` is {}, and this is `\"{}\"`", listed.join(", "), shown(text)), value.span());
        }
        found
    }

    /// A boolean, or `None` when absent or once its wrong type is said.
    pub fn boolean(&mut self, table: &DeTable<'_>, key: &str, what: &str) -> Option<bool> {
        let value = get(table, key)?;
        match value.get_ref() {
            DeValue::Boolean(boolean) => Some(*boolean),
            other => {
                self.say(self.codes.wrong_type, format!("{what}: `{key}` is a boolean, and this is {}", other.type_str()), value.span());
                None
            }
        }
    }

    /// The `faults` table of a device, its keys judged against `known`, and its `hang_ms` read into `spec`: the table and what
    /// a message calls it, for the library to read its other faults; `None` when absent or not a table.
    pub fn faults<'v, 'a>(&mut self, table: &'v DeTable<'a>, what: &str, known: &[&str], spec: &mut DeviceSpec) -> Option<(&'v DeTable<'a>, String)> {
        let label = format!("{what}: `faults`");
        let faults = self.table(get(table, "faults")?, &label)?;
        self.keys(faults, known, &label);
        spec.hang = self.time(faults, "hang_ms", &label);
        Some((faults, label))
    }

    /// The bytes of `text_key` (a TOML string, its UTF-8 bytes) or of `hex_key` (pairs of hexadecimal digits, spaces between
    /// pairs allowed).
    fn bytes(&mut self, table: &DeTable<'_>, text_key: &str, hex_key: &str, what: &str) -> Given {
        let code = self.codes.bytes;
        let (key, value, hex) = match (get(table, text_key), get(table, hex_key)) {
            (Some(_), Some(hex)) => {
                self.say(code, format!("{what}: `{text_key}` and `{hex_key}` are one value in two forms: give one"), hex.span());
                return Given::Wrong;
            }
            (None, None) => return Given::Absent,
            (Some(text), None) => (text_key, text, false),
            (None, Some(hex)) => (hex_key, hex, true),
        };
        let DeValue::String(string) = value.get_ref() else {
            self.say(self.codes.wrong_type, format!("{what}: `{key}` is a text, and this is {}", value.get_ref().type_str()), value.span());
            return Given::Wrong;
        };
        let bytes = if hex {
            let Some(bytes) = from_hex(string) else {
                self.say(code, format!("{what}: `{key}` is pairs of hexadecimal digits, spaces between pairs allowed"), value.span());
                return Given::Wrong;
            };
            bytes
        } else {
            string.as_bytes().to_vec()
        };
        if bytes.len() > MAX_BYTES {
            self.say(code, format!("{what}: `{key}` holds {} bytes, more than {MAX_BYTES}", bytes.len()), value.span());
            return Given::Wrong;
        }
        Given::Bytes(bytes, value.span())
    }
}

/// Pairs of hexadecimal digits, spaces between pairs allowed: `0001 0000 0006`.
fn from_hex(text: &str) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    for group in text.split(' ').filter(|group| !group.is_empty()) {
        let digits = group.as_bytes();
        if digits.len() % 2 != 0 {
            return None;
        }
        for pair in digits.chunks(2) {
            let (high, low) = (digit(*pair.first()?)?, digit(*pair.get(1)?)?);
            bytes.push(high * 16 + low);
        }
    }
    Some(bytes)
}

fn digit(byte: u8) -> Option<u8> {
    char::from(byte).to_digit(16).and_then(|digit| u8::try_from(digit).ok())
}

/// The value of a key of a table.
pub fn get<'v, 'a>(table: &'v DeTable<'a>, key: &str) -> Option<&'v TomlValue<'a>> {
    table.iter().find(|(name, _)| name.get_ref().as_ref() == key).map(|(_, value)| value)
}

/// Judges the section of `library`, a table whose only key is `device`, an array of at most 256 tables: each one is given
/// to `device`, which reads it or says its problems. Its devices, or every problem found.
pub fn devices<T>(
    section: &Section<'_>,
    library: &str,
    codes: Codes,
    mut device: impl FnMut(&mut Reader<'_>, &TomlValue<'_>) -> Option<T>,
) -> Result<Vec<T>, Vec<LibraryDiagnostic>> {
    let mut reader = Reader::new(section.places, codes);
    let mut devices = Vec::new();
    let top = format!("`{library}`");
    if let Some(table) = reader.table(section.value, &top) {
        reader.keys(table, &["device"], &top);
        if let Some(value) = get(table, "device")
            && let Some(items) = reader.array(value, &format!("`{library}.device`"))
        {
            if items.len() > MAX_DEVICES {
                reader.say(codes.bound, format!("{} devices, more than {MAX_DEVICES}", items.len()), value.span());
            } else {
                for item in items {
                    if let Some(found) = device(&mut reader, item) {
                        devices.push(found);
                    }
                }
            }
        }
    }
    if reader.problems.is_empty() { Ok(devices) } else { Err(reader.problems) }
}

/// The endpoint of a device, as `parse` reads it (`None`: not an endpoint of the library, whose form `form` says), with
/// what `parse` gives beside it; `None` once its problem is said: absent, not a text, not an endpoint, or one in `seen`.
pub fn endpoint<X>(
    reader: &mut Reader<'_>,
    table: &DeTable<'_>,
    item: &TomlValue<'_>,
    seen: &mut HashSet<String>,
    form: &str,
    parse: impl FnOnce(&str) -> Option<(String, X)>,
) -> Option<(String, X)> {
    let Some(value) = get(table, "endpoint") else {
        reader.say(reader.codes.endpoint, "a device without `endpoint`".to_owned(), item.span());
        return None;
    };
    let DeValue::String(text) = value.get_ref() else {
        reader.say(reader.codes.wrong_type, format!("`endpoint` is a text, and this is {}", value.get_ref().type_str()), value.span());
        return None;
    };
    let Some((endpoint, extra)) = parse(text) else {
        reader.say(reader.codes.endpoint, format!("the endpoint `{}` is not {form} (io.md 2)", shown(text)), value.span());
        return None;
    };
    if !seen.insert(endpoint.clone()) {
        reader.say(reader.codes.again, format!("the endpoint `{}` is defined again", shown(&endpoint)), value.span());
        return None;
    }
    Some((endpoint, extra))
}

/// The keys of a device that every stream has, in this order: `latency_ms`, `reply`, then `unknown` or `unknown_hex`. Its
/// endpoint and its faults are left for the library to give.
pub fn exchanges(reader: &mut Reader<'_>, table: &DeTable<'_>, what: &str) -> DeviceSpec {
    let mut spec = DeviceSpec { latency: reader.time(table, "latency_ms", what).unwrap_or(0), ..DeviceSpec::default() };
    if let Some(value) = get(table, "reply") {
        replies(reader, value, what, &mut spec);
    }
    if let Given::Bytes(unknown, _) = reader.bytes(table, "unknown", "unknown_hex", what) {
        spec.unknown = Some(unknown);
    }
    spec
}

fn replies(reader: &mut Reader<'_>, value: &TomlValue<'_>, what: &str, spec: &mut DeviceSpec) {
    let label = format!("{what}: `reply`");
    let Some(items) = reader.array(value, &label) else { return };
    if items.len() > MAX_EXCHANGES {
        reader.say(reader.codes.bound, format!("{label} holds {} exchanges, more than {MAX_EXCHANGES}", items.len()), value.span());
        return;
    }
    let code = reader.codes.bytes;
    let mut given: Vec<(Reply, Range<usize>)> = Vec::new();
    for item in items {
        let Some(table) = reader.table(item, &label) else { continue };
        reader.keys(table, REPLY_KEYS, &label);
        let request = reader.bytes(table, "request", "request_hex", &label);
        let response = reader.bytes(table, "response", "response_hex", &label);
        let (request, at) = match request {
            Given::Bytes(request, at) if request.is_empty() => {
                reader.say(code, format!("{label}: a request is not empty"), at);
                continue;
            }
            Given::Bytes(request, at) => (request, at),
            Given::Absent => {
                reader.say(code, format!("{label}: an exchange without `request` or `request_hex`"), item.span());
                continue;
            }
            Given::Wrong => continue,
        };
        let response = match response {
            Given::Bytes(response, _) => Some(response),
            Given::Absent => None,
            Given::Wrong => continue,
        };
        given.push((Reply { request, response }, at));
    }
    // io.md 6: a request equal to another, or the start of one, could never match. In the order of the bytes, a request that
    // starts another starts the one that follows it, so neighbours are enough: each request that its neighbour starts with is
    // said, and of two equal ones both.
    let mut order: Vec<usize> = (0..given.len()).collect();
    order.sort_by(|a, b| given.get(*a).map(|(r, _)| &r.request).cmp(&given.get(*b).map(|(r, _)| &r.request)).then(a.cmp(b)));
    let mut refused = BTreeMap::new();
    for pair in order.windows(2) {
        let (Some(&first), Some(&second)) = (pair.first(), pair.get(1)) else { continue };
        let (Some((a, _)), Some((b, _))) = (given.get(first), given.get(second)) else { continue };
        if a.request == b.request {
            refused.insert(first, Refused::Equal);
            refused.insert(second, Refused::Equal);
        } else if b.request.starts_with(&a.request) {
            refused.entry(first).or_insert(Refused::Start);
        }
    }
    for (index, why) in refused {
        let at = given.get(index).map_or_else(|| value.span(), |(_, at)| at.clone());
        let message = match why {
            Refused::Equal => format!("{label}: this request equals another request of the device"),
            Refused::Start => format!("{label}: this request is the start of another request of the device, which could never match"),
        };
        reader.say(reader.codes.overlap, message, at);
    }
    spec.replies = given.into_iter().map(|(reply, _)| reply).collect();
    spec.order = order;
}

/// Why a request is refused (S007).
#[derive(Clone, Copy)]
enum Refused {
    Equal,
    Start,
}
