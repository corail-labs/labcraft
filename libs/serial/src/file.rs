//! The `serial` section of a simulation file, judged before the run (`docs/io.md`, section 6; `libs/serial/README.md`,
//! "Simulation"): each problem is a diagnostic with its code, at its key or its value, and every problem of the file is
//! said, not the first only. The keys that every stream has are read by `runtime::stream`, with the codes of `serial`;
//! the settings a device expects and its one fault, `hang_ms` (decision of the owner, 2026-11-20), are read here.

use std::collections::HashSet;
use std::sync::Arc;

use labcraft_runtime::native::{Described, LibraryDiagnostic, Section, Setting};
use labcraft_runtime::stream::{self, Codes, Reader, TomlValue, described_time};
use toml::de::DeTable;

use crate::endpoint;
use crate::settings::{DATA_BITS, Expected, FLOW, NAMES, PARITY, StopBits};

/// The codes of `serial` for what the reader of `runtime::stream` finds: it has no fault of UDP, so its bounds are S008.
const CODES: Codes = Codes {
    unknown_key: "serial:S001",
    wrong_type: "serial:S002",
    endpoint: "serial:S003",
    again: "serial:S004",
    range: "serial:S005",
    bytes: "serial:S006",
    overlap: "serial:S007",
    bound: "serial:S008",
};

/// What a file says of its devices, judged, for every run that uses it.
#[derive(Debug)]
pub struct Devices {
    pub(crate) devices: Vec<Arc<DeviceSpec>>,
}

#[derive(Debug)]
pub(crate) struct DeviceSpec {
    /// Its endpoint, its exchanges and its `hang_ms`.
    pub(crate) exchanges: stream::DeviceSpec,
    /// The settings it expects.
    pub(crate) expected: Expected,
}

impl Devices {
    /// The device of an endpoint.
    pub(crate) fn find(&self, endpoint: &str) -> Option<Arc<DeviceSpec>> {
        self.devices.iter().find(|device| device.exchanges.endpoint == endpoint).cloned()
    }

    /// Its endpoints, in the order of the file.
    pub fn endpoints(&self) -> Vec<String> {
        self.devices.iter().map(|device| device.exchanges.endpoint.clone()).collect()
    }

    /// What the file defines, for `simulation.describe`: each device in the order of the file, its endpoint, its latency,
    /// its exchanges in their order, its answer to what matches nothing, its fault, and the settings it expects (`null`:
    /// any); bytes as text when they are text, else in hexadecimal (io.md 6).
    pub fn describe(&self) -> Described {
        let devices = self
            .devices
            .iter()
            .map(|device| {
                let mut fields = device.exchanges.described();
                fields.push(("faults".to_owned(), Described::Map(vec![("hang_ms".to_owned(), described_time(device.exchanges.hang))])));
                let settings = NAMES
                    .into_iter()
                    .zip(device.expected.all())
                    .map(|(name, expected)| {
                        let described = match expected {
                            Some(Setting::Integer(number)) => Described::Integer(number),
                            Some(Setting::Text(word)) => Described::Text(word.to_owned()),
                            None => Described::Null,
                        };
                        (name.to_owned(), described)
                    })
                    .collect();
                fields.push(("settings".to_owned(), Described::Map(settings)));
                Described::Map(fields)
            })
            .collect();
        Described::Map(vec![("devices".to_owned(), Described::List(devices))])
    }
}

const DEVICE_KEYS: &[&str] =
    &["endpoint", "latency_ms", "reply", "unknown", "unknown_hex", "faults", "baud", "data_bits", "parity", "stop_bits", "flow"];
/// A serial device has one fault (decision of the owner, 2026-11-20): `refused` and `close_ms` are `net`'s.
const FAULT_KEYS: &[&str] = &["hang_ms"];

/// Judges the `serial` section: its devices, or every problem found.
pub(crate) fn read(section: &Section<'_>) -> Result<Devices, Vec<LibraryDiagnostic>> {
    let mut seen = HashSet::new();
    let devices = stream::devices(section, "serial", CODES, |reader, item| device(reader, item, &mut seen).map(Arc::new))?;
    Ok(Devices { devices })
}

/// A device; its other keys are judged too when its endpoint is wrong, so that every problem is said.
fn device(reader: &mut Reader<'_>, item: &TomlValue<'_>, seen: &mut HashSet<String>) -> Option<DeviceSpec> {
    let table = reader.table(item, "a device")?;
    let endpoint = stream::endpoint(reader, table, item, seen, "`serial:PORT`", endpoint::of_file);
    let what = endpoint.as_ref().map_or_else(|| "a device".to_owned(), |(endpoint, ())| format!("`{}`", stream::shown(endpoint)));
    reader.keys(table, DEVICE_KEYS, &what);
    let mut spec = DeviceSpec { exchanges: stream::exchanges(reader, table, &what), expected: Expected::default() };
    reader.faults(table, &what, FAULT_KEYS, &mut spec.exchanges);
    spec.expected = expected(reader, table, &what);
    spec.exchanges.endpoint = endpoint?.0;
    Some(spec)
}

/// The settings a device expects: each key absent is any (io.md 6); one out of the range of `open` is S005.
fn expected(reader: &mut Reader<'_>, table: &DeTable<'_>, what: &str) -> Expected {
    let says = |range: &str| format!("{range}, as `open` takes it");
    let bits = i64::from(*DATA_BITS.start())..=i64::from(*DATA_BITS.end());
    let baud = reader.integer(table, "baud", what, 1..=i64::from(u32::MAX), &says("a rate above 0, at most 4,294,967,295"));
    let data_bits = reader.integer(table, "data_bits", what, bits, &says("5 to 8"));
    let parity = reader.word(table, "parity", what, &PARITY);
    let stop_bits = reader.integer(table, "stop_bits", what, 1..=2, &says("1 or 2"));
    let flow = reader.word(table, "flow", what, &FLOW);
    Expected {
        baud: baud.and_then(|baud| u32::try_from(baud).ok()),
        data_bits: data_bits.and_then(|bits| u8::try_from(bits).ok()),
        parity,
        stop_bits: stop_bits.and_then(StopBits::of_number),
        flow,
    }
}
