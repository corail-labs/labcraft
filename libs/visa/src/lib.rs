//! L6: the native bodies of the library `visa` (`libs/visa/README.md`), beside its declarations. A session on a `SIM::`
//! resource talks to an instrument of the simulation file (`libs/visa/simulation.md`), in deterministic mode: every wait is
//! on the clock of the run, and every call is a state machine that the scheduler steps. Real instruments come with M4.
//!
//! No crate of `crates/` depends on this one (`cargo xtask libs`): the apps and the harness link it and give it to the run.
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

use labcraft_runtime::native::{Described, LibraryCode, LibraryDiagnostic, LibraryRun, LogBudget, NativeLibrary, Section, Simulation};
use labcraft_types::Severity;

mod file;
mod instrument;
mod run;
#[cfg(test)]
mod tests;

pub use file::{Bench, Commands};
pub use run::VisaRun;

/// The codes of `visa` for its part of a simulation file: the table of `libs/visa/simulation.md`, section 7, which
/// `cargo xtask codes` compares with this one.
pub const CODES: &[LibraryCode] = &[
    code("visa:S001", "A key that the format does not have; fix: the nearest key"),
    code("visa:S002", "A value of the wrong TOML type"),
    code("visa:S003", "An instrument without `resource`, a resource that does not start with `SIM::`, or a quantity with a noise and no `seed`"),
    code("visa:S004", "A resource defined twice"),
    code("visa:S005", "A number out of its range"),
    code("visa:S006", "`measure` or `set` names a quantity the instrument does not have, or `set` names a quantity that has a `source`"),
    code("visa:S007", "A `source` that names no quantity of the file, a chain of sources that loops, or a `source` with a value or a move"),
    code("visa:S008", "A text over 1,000 code points, with a control character, or a quantity name that is not an ASCII name"),
    code("visa:S009", "An entry of `error_on` or `error_queue` that is not `[code, \"message\"]`"),
    code("visa:S010", "Too many instruments, quantities, entries of a table or an array, or errors in `error_queue`"),
    code(
        "visa:S011",
        "A command key that is empty, has a space at an end, or is answered by the simulator; a query key that is not a query; a query in both `commands` and `measure`",
    ),
];

const fn code(code: &'static str, when: &'static str) -> LibraryCode {
    LibraryCode { code, severity: Severity::Error, when }
}

/// The library `visa`, as the run is given it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Visa;

impl NativeLibrary for Visa {
    fn name(&self) -> &str {
        "visa"
    }

    fn codes(&self) -> &'static [LibraryCode] {
        CODES
    }

    fn simulates(&self) -> bool {
        true
    }

    fn simulate(&self, section: &Section<'_>) -> Result<Simulation, Vec<LibraryDiagnostic>> {
        file::read(section).map(|bench| Box::new(Arc::new(bench)) as Simulation)
    }

    fn describe(&self, simulation: &Simulation) -> Option<Described> {
        simulation.downcast_ref::<Arc<Bench>>().map(|bench| bench.describe())
    }

    fn resources(&self, simulation: &Simulation) -> Vec<String> {
        simulation
            .downcast_ref::<Arc<Bench>>()
            .map(|bench| bench.commands().into_iter().map(|commands| commands.resource).collect())
            .unwrap_or_default()
    }

    fn start_run(&self, library: u32, simulation: Option<&Simulation>, budget: LogBudget) -> Box<dyn LibraryRun> {
        let bench = simulation.and_then(|simulation| simulation.downcast_ref::<Arc<Bench>>()).cloned();
        Box::new(VisaRun::new(library, bench, budget))
    }
}
