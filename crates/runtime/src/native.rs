//! What a library with native functions gives the runtime (`docs/architecture.md`, section 10.6): its name, its codes, how
//! it judges its part of a simulation file, and what one run of it holds. A call of an `extern rust` function is a small
//! state machine that the library keeps: the scheduler starts it, then asks it for its next step after each wait, so a
//! call can wait in its middle (a simulated instrument that is busy) and a replay is exact.

use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use labcraft_types::{Severity, Span};

use crate::value::{Handle, Value};

/// A code of a library (`visa:S001`), with what it means: `cargo xtask codes` compares them with the table of its document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LibraryCode {
    pub code: &'static str,
    pub severity: Severity,
    pub when: &'static str,
}

/// What a library finds in its part of a simulation file, at the key or the value it concerns. The tool joins these to the
/// diagnostics of a run (cli.md 6.1), with the code of the library (cli.md 4.4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibraryDiagnostic {
    pub code: &'static str,
    pub severity: Severity,
    pub message: String,
    pub span: Option<Span>,
    /// The nearest key, for an unknown one.
    pub fix: Option<String>,
}

/// What a library is given of a simulation file: its table, with the place of each key and value, and where each byte of the
/// file is, so that a diagnostic points at its key.
pub struct Section<'a> {
    pub value: &'a toml::Spanned<toml::de::DeValue<'a>>,
    pub places: &'a crate::simulation::Places<'a>,
}

/// What a library judged of its section and keeps for its runs: its own type, which only it reads.
pub type Simulation = Box<dyn Any + Send + Sync>;

pub trait NativeLibrary: Send + Sync {
    /// As its manifest names it: `visa`.
    fn name(&self) -> &str;
    fn codes(&self) -> &'static [LibraryCode];
    /// Whether it reads a section of a simulation file: a key at the top level of the file names a library that does
    /// (cli.md 6.2, `LC8110` otherwise).
    fn simulates(&self) -> bool {
        false
    }
    /// Its section of a simulation file, judged before the run starts (simulation.md 7): the run is refused on an error.
    fn simulate(&self, section: &Section<'_>) -> Result<Simulation, Vec<LibraryDiagnostic>> {
        let _ = section;
        Err(Vec::new())
    }
    /// What its judged section of a simulation file defines, for `simulation.describe` (commands.md 6.3): for `visa`, each
    /// instrument, its resource, its answers and its faults (simulation.md 9).
    fn describe(&self, simulation: &Simulation) -> Option<Described> {
        let _ = simulation;
        None
    }
    /// The resources its section defines, in the order of the file, for `project.describe`: `SIM::DMM0`.
    fn resources(&self, simulation: &Simulation) -> Vec<String> {
        let _ = simulation;
        Vec::new()
    }
    /// What one run of it holds, from its simulation if it has one. `library` is its index among the libraries of the run,
    /// which its handles and its keys carry.
    /// What one run of it holds. `budget` is the memory that every library of the run shares for what it keeps for the report.
    fn start_run(&self, library: u32, simulation: Option<&Simulation>, budget: LogBudget) -> Box<dyn LibraryRun>;
}

/// What a library says of its part of a simulation file, in the shape JSON gives it, which `engine` writes as it is.
#[derive(Clone, Debug, PartialEq)]
pub enum Described {
    Null,
    Bool(bool),
    Integer(i64),
    Number(f64),
    Text(String),
    List(Vec<Described>),
    /// Its fields in the order to show them.
    Map(Vec<(String, Described)>),
}

/// A call in progress, by its number in its run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CallId(pub u64);

/// What a call can wait for alone: a session, an instrument. In deterministic mode the scheduler gives it in the global order
/// of spec 8; in real time it is a lock. The runtime locks nothing on its own (spec 8.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key {
    pub library: u32,
    pub id: u64,
}

/// What a call does next.
#[derive(Debug)]
pub enum Step {
    Done(NativeOutcome),
    /// Waits this many nanoseconds on the clock of the run (`LC8007` past its end).
    Sleep(i64),
    /// Waits until it holds the key alone; the keys it still holds are given back when it is done.
    Exclusive(Key),
    Release(Key),
}

/// How a call ended: its outputs, or a new error, whose outputs are those of a node that fails (spec 7.2). The host gives the
/// error its origin.
#[derive(Debug)]
pub enum NativeOutcome {
    Outputs(Vec<Value>),
    Fail { category: &'static str, code: i64, message: String },
}

/// What the scheduler tells a call when it asks for its next step.
#[derive(Clone, Copy, Debug)]
pub struct CallContext {
    /// The clock of the run, in nanoseconds.
    pub now: i64,
}

/// What a handle designates, for the report (commands.md 7, `resources`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HandleInfo {
    /// Its opaque type, as the library names it: `session`.
    pub kind: &'static str,
    /// `SIM::DMM0`
    pub designates: String,
    pub open: bool,
}

/// The commands a simulated instrument received, for the report (commands.md 7, `instruments`).
#[derive(Clone, Debug, PartialEq)]
pub struct InstrumentLog {
    pub resource: String,
    pub commands: Vec<CommandLog>,
    /// Commands past the ones kept.
    pub more: u64,
}

/// A command as the report shows it: each text cut to its first 1,000 code points when it is kept (commands.md 5), with its
/// whole length, so that what a program sends cannot grow the memory of the run beyond `--max-memory`.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandLog {
    /// When the command was handled, after its latency, in nanoseconds.
    pub at: i64,
    pub sent: String,
    /// In code points.
    pub sent_length: u64,
    /// `None` for a write, and for a query never answered.
    pub answer: Option<String>,
    pub answer_length: u64,
}

/// What the libraries of a run may keep for its report (commands.md 7, `instruments` and `logs`), in bytes: one budget for the
/// run, which each of its libraries takes from, so that what a program sends or logs stays within 16 MiB and `--max-memory`. A
/// library that finds it spent counts what it no longer keeps, from the first entry that does not fit.
#[derive(Clone, Debug)]
pub struct LogBudget(Arc<AtomicU64>);

/// The budget of a run whose `--max-memory` is higher *(proposed)*.
pub const LOG_BUDGET: u64 = 16 * 1024 * 1024;

impl LogBudget {
    pub fn new(bytes: u64) -> Self {
        LogBudget(Arc::new(AtomicU64::new(bytes)))
    }

    /// The budget of a run: 16 MiB, or `--max-memory` when lower.
    pub fn of_run(max_memory: u64) -> Self {
        LogBudget::new(LOG_BUDGET.min(max_memory))
    }

    /// Takes `cost` bytes, or nothing when fewer are left.
    pub fn take(&self, cost: u64) -> bool {
        let mut left = self.0.load(Ordering::Relaxed);
        loop {
            let Some(after) = left.checked_sub(cost) else { return false };
            match self.0.compare_exchange_weak(left, after, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => return true,
                Err(now) => left = now,
            }
        }
    }

    pub fn left(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

/// What a library kept of a run for the report (commands.md 7, `logs`): its entries, in the order of its calls, and a count of
/// the entries past its bounds.
#[derive(Clone, Debug, Default)]
pub struct Logs {
    pub entries: Vec<LogEntry>,
    /// Entries counted, not kept.
    pub more: u64,
}

/// One entry of the log of a library: the call that wrote it, when, and the value it was given.
#[derive(Clone, Debug)]
pub struct LogEntry {
    /// The `extern rust` function of the call.
    pub function: String,
    /// Which input of the call it holds, by its place in the signature: a value carries no type, so the host writes it against
    /// the type of that input (commands.md 5).
    pub input: usize,
    /// In nanoseconds on the clock of the run.
    pub at: i64,
    pub value: Value,
}

/// What one run of a library holds: its sessions and its simulated instruments live and end with the run.
pub trait LibraryRun: Send {
    /// Starts a call of its `extern rust` function `function`, with the inputs of its node in the order of its signature.
    /// `None` when the library has no such function.
    fn call(&mut self, function: &str, inputs: Vec<Value>) -> Option<CallId>;
    /// The next step of a call, asked again after each wait.
    fn step(&mut self, call: CallId, cx: CallContext) -> Step;
    /// A call that the run abandons: it ends now, and gives back what it held.
    fn cancel(&mut self, call: CallId) {
        let _ = call;
    }
    /// What a handle it made designates.
    fn describe(&self, handle: Handle) -> Option<HandleInfo>;
    /// After an abort or a fault: closes what the handle holds.
    fn release(&mut self, handle: Handle);
    /// The commands of its simulated instruments, in code point order of their resources.
    fn instruments(&self) -> Vec<InstrumentLog> {
        Vec::new()
    }
    /// What it kept of the values it was given, for the report.
    fn logs(&self) -> Logs {
        Logs::default()
    }
}
