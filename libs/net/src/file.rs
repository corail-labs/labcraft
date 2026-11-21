//! The `net` section of a simulation file, judged before the run (`docs/io.md`, section 6; `libs/net/README.md`,
//! "Simulation"): each problem is a diagnostic with its code, at its key or its value, and every problem of the file is
//! said, not the first only. The keys that every stream has are read by `runtime::stream`, with the codes of `net`.

use std::collections::HashSet;
use std::sync::Arc;

use labcraft_runtime::native::{Described, LibraryDiagnostic, Section};
use labcraft_runtime::stream::{self, Codes, Reader, TomlValue, described_time};

use crate::endpoint;

/// The codes of `net` for what the reader of `runtime::stream` finds.
const CODES: Codes = Codes {
    unknown_key: "net:S001",
    wrong_type: "net:S002",
    endpoint: "net:S003",
    again: "net:S004",
    range: "net:S005",
    bytes: "net:S006",
    overlap: "net:S007",
    bound: "net:S009",
};

/// What a file says of its devices, judged, for every run that uses it.
#[derive(Debug)]
pub struct Devices {
    pub(crate) devices: Vec<Arc<DeviceSpec>>,
}

#[derive(Debug)]
pub(crate) struct DeviceSpec {
    /// Its endpoint, its exchanges and its `hang_ms` and `close_ms` (TCP).
    pub(crate) exchanges: stream::DeviceSpec,
    pub(crate) udp: bool,
    /// Every connect gives `net.connection_refused` (TCP).
    pub(crate) refused: bool,
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
    /// its exchanges in their order, its answer to what matches nothing, and its faults; bytes as text when they are text,
    /// else in hexadecimal (io.md 6).
    pub fn describe(&self) -> Described {
        let devices = self
            .devices
            .iter()
            .map(|device| {
                let mut fields = device.exchanges.described();
                let faults = vec![
                    ("hang_ms".to_owned(), described_time(device.exchanges.hang)),
                    ("close_ms".to_owned(), described_time(device.exchanges.close)),
                    ("refused".to_owned(), Described::Bool(device.refused)),
                ];
                fields.push(("faults".to_owned(), Described::Map(faults)));
                Described::Map(fields)
            })
            .collect();
        Described::Map(vec![("devices".to_owned(), Described::List(devices))])
    }
}

const DEVICE_KEYS: &[&str] = &["endpoint", "latency_ms", "reply", "unknown", "unknown_hex", "faults"];
const FAULT_KEYS: &[&str] = &["hang_ms", "refused", "close_ms"];

/// Judges the `net` section: its devices, or every problem found.
pub(crate) fn read(section: &Section<'_>) -> Result<Devices, Vec<LibraryDiagnostic>> {
    let mut seen = HashSet::new();
    let devices = stream::devices(section, "net", CODES, |reader, item| device(reader, item, &mut seen).map(Arc::new))?;
    Ok(Devices { devices })
}

/// A device; its other keys are judged too when its endpoint is wrong, so that every problem is said.
fn device(reader: &mut Reader<'_>, item: &TomlValue<'_>, seen: &mut HashSet<String>) -> Option<DeviceSpec> {
    let table = reader.table(item, "a device")?;
    let endpoint = stream::endpoint(reader, table, item, seen, "`tcp:HOST:PORT` or `udp:HOST:PORT`", endpoint::of_file);
    let udp = endpoint.as_ref().map(|(_, udp)| *udp);
    let what = endpoint.as_ref().map_or_else(|| "a device".to_owned(), |(endpoint, _)| format!("`{}`", stream::shown(endpoint)));
    reader.keys(table, DEVICE_KEYS, &what);
    let mut spec = DeviceSpec { exchanges: stream::exchanges(reader, table, &what), udp: udp.unwrap_or(false), refused: false };
    if let Some((faults, label)) = reader.faults(table, &what, FAULT_KEYS, &mut spec.exchanges) {
        spec.exchanges.close = reader.time(faults, "close_ms", &label);
        spec.refused = reader.boolean(faults, "refused", &label).unwrap_or(false);
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
    spec.exchanges.endpoint = endpoint?.0;
    Some(spec)
}
