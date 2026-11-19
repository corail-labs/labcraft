//! The `net` section of a simulation file, judged before the run (`docs/io.md`, section 6; `libs/net/README.md`,
//! "Simulation"): each problem is a diagnostic with its code, at its key or its value, and every problem of the file is
//! said, not the first only.

use std::collections::{BTreeMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

use labcraft_runtime::native::{Described, LibraryDiagnostic, Section, bytes_text};
use labcraft_runtime::simulation::{Places, float_of, integer_of, nearest};
use labcraft_types::Severity;
use toml::Spanned;
use toml::de::{DeTable, DeValue};

use crate::endpoint::{self, shown};

/// What a file says of its devices, judged, for every run that uses it.
#[derive(Debug)]
pub struct Devices {
    pub(crate) devices: Vec<Arc<DeviceSpec>>,
}

#[derive(Debug)]
pub(crate) struct DeviceSpec {
    /// As a program reaches it (io.md 2): `tcp:10.0.0.7:502`.
    pub(crate) endpoint: String,
    pub(crate) udp: bool,
    /// Nanoseconds on the clock of the run.
    pub(crate) latency: i64,
    /// In the order of the file.
    pub(crate) replies: Vec<Reply>,
    /// The indices of `replies` in the order of their requests, for a binary search.
    pub(crate) order: Vec<usize>,
    pub(crate) unknown: Option<Vec<u8>>,
    /// From that time on, nanoseconds on the clock of the run, the device takes no request and gives no response.
    pub(crate) hang: Option<i64>,
    /// When the device closes its connections (TCP).
    pub(crate) close: Option<i64>,
    /// Every connect gives `net.connection_refused` (TCP).
    pub(crate) refused: bool,
}

impl DeviceSpec {
    /// The request at a place of the order of the bytes.
    fn sorted(&self, at: usize) -> Option<&Reply> {
        self.replies.get(*self.order.get(at)?)
    }

    /// The exchange whose request starts `bytes`. No request is the start of another (`net:S007`), so the only one that can
    /// is the greatest request not above `bytes` in the order of the bytes: one between it and `bytes` would start with it.
    pub(crate) fn starting(&self, bytes: &[u8]) -> Option<&Reply> {
        let after = self.order.partition_point(|index| self.replies.get(*index).is_some_and(|reply| reply.request.as_slice() <= bytes));
        self.sorted(after.checked_sub(1)?).filter(|reply| bytes.starts_with(&reply.request))
    }

    /// Whether `bytes` is the start of some request: then of the least request not below it, as above.
    pub(crate) fn waits_for_more(&self, bytes: &[u8]) -> bool {
        let at = self.order.partition_point(|index| self.replies.get(*index).is_some_and(|reply| reply.request.as_slice() < bytes));
        self.sorted(at).is_some_and(|reply| reply.request.starts_with(bytes))
    }

    /// The exchange whose request is `bytes`, a datagram.
    pub(crate) fn equal(&self, bytes: &[u8]) -> Option<&Reply> {
        let at = self.order.partition_point(|index| self.replies.get(*index).is_some_and(|reply| reply.request.as_slice() < bytes));
        self.sorted(at).filter(|reply| reply.request == bytes)
    }

    /// When the device closes its connections: never over UDP, which has none.
    pub(crate) fn close_at(&self) -> Option<i64> {
        if self.udp { None } else { self.close }
    }
}

#[derive(Debug)]
pub(crate) struct Reply {
    pub(crate) request: Vec<u8>,
    /// None: the request is taken and nothing answers.
    pub(crate) response: Option<Vec<u8>>,
}

impl Devices {
    /// The device of an endpoint.
    pub(crate) fn find(&self, endpoint: &str) -> Option<Arc<DeviceSpec>> {
        self.devices.iter().find(|device| device.endpoint == endpoint).cloned()
    }

    /// Its endpoints, in the order of the file.
    pub fn endpoints(&self) -> Vec<String> {
        self.devices.iter().map(|device| device.endpoint.clone()).collect()
    }

    /// What the file defines, for `simulation.describe`: each device in the order of the file, its endpoint, its latency,
    /// its exchanges in their order, its answer to what matches nothing, and its faults; bytes as text when they are text,
    /// else in hexadecimal (io.md 6).
    pub fn describe(&self) -> Described {
        let map = |fields: Vec<(&str, Described)>| Described::Map(fields.into_iter().map(|(key, value)| (key.to_owned(), value)).collect());
        let bytes = |key: &str, bytes: &[u8]| {
            let (text, hex) = bytes_text(bytes);
            (if hex { format!("{key}_hex") } else { key.to_owned() }, Described::Text(text))
        };
        let time = |ns: Option<i64>| ns.map_or(Described::Null, |ns| Described::Number(ns as f64 / 1e6));
        let devices = self
            .devices
            .iter()
            .map(|device| {
                let replies = device
                    .replies
                    .iter()
                    .map(|reply| {
                        let mut fields = vec![bytes("request", &reply.request)];
                        if let Some(response) = &reply.response {
                            fields.push(bytes("response", response));
                        }
                        Described::Map(fields)
                    })
                    .collect();
                let unknown = match &device.unknown {
                    Some(unknown) => Described::Map(vec![bytes("response", unknown)]),
                    None => Described::Null,
                };
                map(vec![
                    ("endpoint", Described::Text(device.endpoint.clone())),
                    ("latency_ms", Described::Number(device.latency as f64 / 1e6)),
                    ("replies", Described::List(replies)),
                    ("unknown", unknown),
                    (
                        "faults",
                        map(vec![("hang_ms", time(device.hang)), ("close_ms", time(device.close)), ("refused", Described::Bool(device.refused))]),
                    ),
                ])
            })
            .collect();
        map(vec![("devices", Described::List(devices))])
    }
}

/// At most this many bytes in a request or a response *(proposed)*.
pub(crate) const MAX_BYTES: usize = 65_536;
const MAX_DEVICES: usize = 256;
const MAX_EXCHANGES: usize = 10_000;
const DAY_MS: f64 = 86_400_000.0;
const DEVICE_KEYS: &[&str] = &["endpoint", "latency_ms", "reply", "unknown", "unknown_hex", "faults"];
const REPLY_KEYS: &[&str] = &["request", "request_hex", "response", "response_hex"];
const FAULT_KEYS: &[&str] = &["hang_ms", "refused", "close_ms"];

type Value<'a> = Spanned<DeValue<'a>>;

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

struct Reader<'s> {
    places: &'s Places<'s>,
    problems: Vec<LibraryDiagnostic>,
}

impl Reader<'_> {
    fn say(&mut self, code: &'static str, message: String, at: Range<usize>) {
        self.problems.push(LibraryDiagnostic { code, severity: Severity::Error, message, span: Some(self.places.span(at)), fix: None });
    }

    fn table<'v, 'a>(&mut self, value: &'v Value<'a>, what: &str) -> Option<&'v DeTable<'a>> {
        match value.get_ref() {
            DeValue::Table(table) => Some(table),
            other => {
                self.say("net:S002", format!("{what} is a table, and this is {}", other.type_str()), value.span());
                None
            }
        }
    }

    fn array<'v, 'a>(&mut self, value: &'v Value<'a>, what: &str) -> Option<&'v [Value<'a>]> {
        match value.get_ref() {
            DeValue::Array(items) => Some(items.as_ref()),
            other => {
                self.say("net:S002", format!("{what} is an array of tables, and this is {}", other.type_str()), value.span());
                None
            }
        }
    }

    /// The keys of a table that the format has; each other one is `net:S001`, with the nearest key.
    fn keys(&mut self, table: &DeTable<'_>, known: &[&str], what: &str) {
        for (key, _) in table.iter() {
            let name = key.get_ref().as_ref();
            if !known.contains(&name) {
                let fix = nearest(name, known);
                let hint = fix.map_or_else(String::new, |fix| format!(": did you mean `{fix}`?"));
                self.problems.push(LibraryDiagnostic {
                    code: "net:S001",
                    severity: Severity::Error,
                    message: format!("{what} has no key `{}`{hint}", shown(name)),
                    span: Some(self.places.span(key.span())),
                    fix: fix.map(str::to_owned),
                });
            }
        }
    }

    /// A time in milliseconds, from 0 to one day (simulation.md 3), as nanoseconds; `None` when absent or wrong.
    fn time(&mut self, table: &DeTable<'_>, key: &str, what: &str) -> Option<i64> {
        let value = get(table, key)?;
        let read = match value.get_ref() {
            DeValue::Integer(integer) => integer_of(integer).map(|x| x as f64),
            DeValue::Float(float) => float_of(float),
            other => {
                self.say("net:S002", format!("{what}: `{key}` is a number, and this is {}", other.type_str()), value.span());
                return None;
            }
        };
        match read {
            Some(x) if x.is_finite() && (0.0..=DAY_MS).contains(&x) => Some(nanoseconds(x)),
            _ => {
                self.say("net:S005", format!("{what}: `{key}` is a time from 0 to one day, 86,400,000 ms"), value.span());
                None
            }
        }
    }

    /// The bytes of `text_key` (a TOML string, its UTF-8 bytes) or of `hex_key` (pairs of hexadecimal digits, spaces between
    /// pairs allowed).
    fn bytes(&mut self, table: &DeTable<'_>, text_key: &str, hex_key: &str, what: &str) -> Given {
        let (key, value, hex) = match (get(table, text_key), get(table, hex_key)) {
            (Some(_), Some(hex)) => {
                self.say("net:S006", format!("{what}: `{text_key}` and `{hex_key}` are one value in two forms: give one"), hex.span());
                return Given::Wrong;
            }
            (None, None) => return Given::Absent,
            (Some(text), None) => (text_key, text, false),
            (None, Some(hex)) => (hex_key, hex, true),
        };
        let DeValue::String(string) = value.get_ref() else {
            self.say("net:S002", format!("{what}: `{key}` is a text, and this is {}", value.get_ref().type_str()), value.span());
            return Given::Wrong;
        };
        let bytes = if hex {
            let Some(bytes) = from_hex(string) else {
                self.say("net:S006", format!("{what}: `{key}` is pairs of hexadecimal digits, spaces between pairs allowed"), value.span());
                return Given::Wrong;
            };
            bytes
        } else {
            string.as_bytes().to_vec()
        };
        if bytes.len() > MAX_BYTES {
            self.say("net:S006", format!("{what}: `{key}` holds {} bytes, more than {MAX_BYTES}", bytes.len()), value.span());
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

fn get<'v, 'a>(table: &'v DeTable<'a>, key: &str) -> Option<&'v Value<'a>> {
    table.iter().find(|(name, _)| name.get_ref().as_ref() == key).map(|(_, value)| value)
}

/// Judges the `net` section: its devices, or every problem found.
pub(crate) fn read(section: &Section<'_>) -> Result<Devices, Vec<LibraryDiagnostic>> {
    let mut reader = Reader { places: section.places, problems: Vec::new() };
    let mut devices = Devices { devices: Vec::new() };
    if let Some(table) = reader.table(section.value, "`net`") {
        reader.keys(table, &["device"], "`net`");
        if let Some(value) = get(table, "device")
            && let Some(items) = reader.array(value, "`net.device`")
        {
            if items.len() > MAX_DEVICES {
                reader.say("net:S009", format!("{} devices, more than {MAX_DEVICES}", items.len()), value.span());
            } else {
                let mut seen = HashSet::new();
                for item in items {
                    if let Some(spec) = device(&mut reader, item, &mut seen) {
                        devices.devices.push(Arc::new(spec));
                    }
                }
            }
        }
    }
    if reader.problems.is_empty() { Ok(devices) } else { Err(reader.problems) }
}

/// The endpoint of a device and whether it is UDP, or `None` once its problem is said.
fn endpoint_of(reader: &mut Reader<'_>, table: &DeTable<'_>, item: &Value<'_>, seen: &mut HashSet<String>) -> Option<(String, bool)> {
    let Some(value) = get(table, "endpoint") else {
        reader.say("net:S003", "a device without `endpoint`".to_owned(), item.span());
        return None;
    };
    let DeValue::String(text) = value.get_ref() else {
        reader.say("net:S002", format!("`endpoint` is a text, and this is {}", value.get_ref().type_str()), value.span());
        return None;
    };
    let Some((endpoint, udp)) = endpoint::of_file(text) else {
        reader.say("net:S003", format!("the endpoint `{}` is not `tcp:HOST:PORT` or `udp:HOST:PORT` (io.md 2)", shown(text)), value.span());
        return None;
    };
    if !seen.insert(endpoint.clone()) {
        reader.say("net:S004", format!("the endpoint `{}` is defined again", shown(&endpoint)), value.span());
        return None;
    }
    Some((endpoint, udp))
}

/// A device; its other keys are judged too when its endpoint is wrong, so that every problem is said.
fn device(reader: &mut Reader<'_>, item: &Value<'_>, seen: &mut HashSet<String>) -> Option<DeviceSpec> {
    let table = reader.table(item, "a device")?;
    let endpoint = endpoint_of(reader, table, item, seen);
    let udp = endpoint.as_ref().map(|(_, udp)| *udp);
    let what = endpoint.as_ref().map_or_else(|| "a device".to_owned(), |(endpoint, _)| format!("`{}`", shown(endpoint)));
    reader.keys(table, DEVICE_KEYS, &what);
    let latency = reader.time(table, "latency_ms", &what).unwrap_or(0);
    let mut spec = DeviceSpec {
        endpoint: String::new(),
        udp: udp.unwrap_or(false),
        latency,
        replies: Vec::new(),
        order: Vec::new(),
        unknown: None,
        hang: None,
        close: None,
        refused: false,
    };
    if let Some(value) = get(table, "reply") {
        replies(reader, value, &what, &mut spec);
    }
    if let Given::Bytes(unknown, _) = reader.bytes(table, "unknown", "unknown_hex", &what) {
        spec.unknown = Some(unknown);
    }
    let label = format!("{what}: `faults`");
    if let Some(value) = get(table, "faults")
        && let Some(faults) = reader.table(value, &label)
    {
        reader.keys(faults, FAULT_KEYS, &label);
        spec.hang = reader.time(faults, "hang_ms", &label);
        spec.close = reader.time(faults, "close_ms", &label);
        if let Some(refused) = get(faults, "refused") {
            match refused.get_ref() {
                DeValue::Boolean(refused) => spec.refused = *refused,
                other => reader.say("net:S002", format!("{label}: `refused` is a boolean, and this is {}", other.type_str()), refused.span()),
            }
        }
        if udp == Some(true) {
            for (key, value) in faults.iter().filter(|(key, _)| matches!(key.get_ref().as_ref(), "refused" | "close_ms")) {
                let key = key.get_ref().as_ref();
                reader.say(
                    "net:S008",
                    format!("{label}: `{key}` is for a TCP device; a UDP socket has no connection to refuse or close"),
                    value.span(),
                );
            }
        }
    }
    spec.endpoint = endpoint?.0;
    Some(spec)
}

fn replies(reader: &mut Reader<'_>, value: &Value<'_>, what: &str, spec: &mut DeviceSpec) {
    let label = format!("{what}: `reply`");
    let Some(items) = reader.array(value, &label) else { return };
    if items.len() > MAX_EXCHANGES {
        reader.say("net:S009", format!("{label} holds {} exchanges, more than {MAX_EXCHANGES}", items.len()), value.span());
        return;
    }
    let mut given: Vec<(Reply, Range<usize>)> = Vec::new();
    for item in items {
        let Some(table) = reader.table(item, &label) else { continue };
        reader.keys(table, REPLY_KEYS, &label);
        let request = reader.bytes(table, "request", "request_hex", &label);
        let response = reader.bytes(table, "response", "response_hex", &label);
        let (request, at) = match request {
            Given::Bytes(request, at) if request.is_empty() => {
                reader.say("net:S006", format!("{label}: a request is not empty"), at);
                continue;
            }
            Given::Bytes(request, at) => (request, at),
            Given::Absent => {
                reader.say("net:S006", format!("{label}: an exchange without `request` or `request_hex`"), item.span());
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
        reader.say("net:S007", message, at);
    }
    spec.replies = given.into_iter().map(|(reply, _)| reply).collect();
    spec.order = order;
}

/// Why a request is refused (`net:S007`).
#[derive(Clone, Copy)]
enum Refused {
    Equal,
    Start,
}
