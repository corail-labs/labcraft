//! L6: the native bodies of the library `serial` (`libs/serial/README.md`), beside its declarations: serial ports in raw
//! bytes, over the crate `serial2` (`BSD-2-Clause OR Apache-2.0`, taken under Apache-2.0), and the devices of the `serial`
//! section of a simulation file (`docs/io.md`, section 6). Its reads and writes are those of `net` on a TCP connection,
//! through what `runtime::stream` gives both. Every call is a state machine that the scheduler steps
//! (`docs/architecture.md`, 10.6): a simulated device waits on the clock of the run, a real port blocks in the `Blocking`
//! work of its call. `serial2` does not build for WASM: there, every open and `list` give `io.unsupported`.
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

mod endpoint;
mod file;
#[cfg(not(target_arch = "wasm32"))]
mod real;
mod run;
mod settings;
#[cfg(test)]
mod tests;

pub use file::Devices;
pub use run::SerialRun;

/// The codes of `serial` for its part of a simulation file: the table of `libs/serial/README.md`, "Simulation", which
/// `cargo xtask codes` compares with this one.
pub const CODES: &[LibraryCode] = &[
    code("serial:S001", "A key that the format does not have; fix: the nearest key"),
    code("serial:S002", "A value of the wrong TOML type"),
    code("serial:S003", "A device without `endpoint`, or one that is not `serial:PORT` (io.md 2)"),
    code("serial:S004", "An endpoint defined again, at the second one (`serial:com3` is `serial:COM3`)"),
    code("serial:S005", "A time below 0 or above one day; a setting out of the range of `open` (`parity = \"mark\"`)"),
    code(
        "serial:S006",
        "A request or a response given in both forms or, for a request, in neither; an empty request; hexadecimal that is not pairs of digits; more than 65,536 bytes",
    ),
    code("serial:S007", "A request that equals another request of the same device, or is the start of one (io.md 6)"),
    code("serial:S008", "More than 256 devices, or 10,000 exchanges in one device"),
];

const fn code(code: &'static str, when: &'static str) -> LibraryCode {
    LibraryCode { code, severity: Severity::Error, when }
}

/// What `serial` judged of its section of a simulation file.
fn devices(simulation: &Simulation) -> Option<&Arc<Devices>> {
    simulation.downcast_ref::<Arc<Devices>>()
}

/// The library `serial`, as the run is given it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Serial;

impl NativeLibrary for Serial {
    fn name(&self) -> &str {
        "serial"
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

    /// Its devices are named by their endpoints of io.md 2: `serial:COM3`.
    fn endpoints(&self, simulation: &Simulation) -> Vec<String> {
        devices(simulation).map(|devices| devices.endpoints()).unwrap_or_default()
    }

    fn start_run(&self, start: Start<'_>) -> Box<dyn LibraryRun> {
        let devices = start.simulation.and_then(devices).cloned();
        Box::new(SerialRun::new(start.library, devices, start.budget, start.mode, start.context.clone()))
    }
}
