//! L6: the native bodies of the library `net` (`libs/net/README.md`), beside its declarations: TCP connections and UDP
//! sockets in raw bytes, over the standard library of Rust, and the devices of the `net` section of a simulation file
//! (`docs/io.md`, section 6). Every call is a state machine that the scheduler steps (`docs/architecture.md`, 10.6): a
//! simulated device waits on the clock of the run, a real socket blocks in the `Blocking` work of its call.
//!
//! No crate of `crates/` depends on this one (`cargo xtask libs`): the apps link it and give it to the run.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable,
    clippy::indexing_slicing,
    clippy::string_slice
)]
#![forbid(unsafe_code)]

use std::sync::Arc;

use labcraft_runtime::native::{Described, LibraryCode, LibraryDiagnostic, LibraryRun, NativeLibrary, Section, Simulation, Start};
use labcraft_types::Severity;

mod device;
mod endpoint;
mod file;
#[cfg(not(target_arch = "wasm32"))]
mod real;
mod run;
#[cfg(test)]
mod tests;

pub use file::Devices;
pub use run::NetRun;

/// The codes of `net` for its part of a simulation file: the table of `libs/net/README.md`, "Simulation", which
/// `cargo xtask codes` compares with this one.
pub const CODES: &[LibraryCode] = &[
    code("net:S001", "A key that the format does not have; fix: the nearest key"),
    code("net:S002", "A value of the wrong TOML type"),
    code(
        "net:S003",
        "A device without `endpoint`, or one that is not `tcp:HOST:PORT` or `udp:HOST:PORT` (io.md 2), its host a host as \"Functions\" says",
    ),
    code("net:S004", "An endpoint defined again, at the second one"),
    code("net:S005", "A time below 0 or above one day, as in simulation.md 3"),
    code(
        "net:S006",
        "A request or a response given in both forms or, for a request, in neither; an empty request; hexadecimal that is not pairs of digits; more than 65,536 bytes",
    ),
    code("net:S007", "A request that equals another request of the same device, or is the start of one (io.md 6)"),
    code("net:S008", "`refused` or `close_ms` on a UDP device"),
    code("net:S009", "More than 256 devices, or 10,000 exchanges in one device"),
];

const fn code(code: &'static str, when: &'static str) -> LibraryCode {
    LibraryCode { code, severity: Severity::Error, when }
}

/// What `net` judged of its section of a simulation file.
fn devices(simulation: &Simulation) -> Option<&Arc<Devices>> {
    simulation.downcast_ref::<Arc<Devices>>()
}

/// The library `net`, as the run is given it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Net;

impl NativeLibrary for Net {
    fn name(&self) -> &str {
        "net"
    }

    fn codes(&self) -> &'static [LibraryCode] {
        CODES
    }

    fn simulates(&self) -> bool {
        true
    }

    fn simulate(&self, section: &Section<'_>) -> Result<Simulation, Vec<LibraryDiagnostic>> {
        file::read(section).map(|devices| Box::new(Arc::new(devices)) as Simulation)
    }

    fn describe(&self, simulation: &Simulation) -> Option<Described> {
        devices(simulation).map(|devices| devices.describe())
    }

    fn resources(&self, simulation: &Simulation) -> Vec<String> {
        devices(simulation).map(|devices| devices.endpoints()).unwrap_or_default()
    }

    /// Its devices are endpoints of io.md 2 already: `tcp:…` and `udp:…`.
    fn endpoints(&self, simulation: &Simulation) -> Vec<String> {
        self.resources(simulation)
    }

    fn start_run(&self, start: Start<'_>) -> Box<dyn LibraryRun> {
        let devices = start.simulation.and_then(devices).cloned();
        Box::new(NetRun::new(start.library, devices, start.budget, start.mode, start.context.clone()))
    }
}
