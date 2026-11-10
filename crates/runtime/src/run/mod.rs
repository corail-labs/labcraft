//! A run of a program, in deterministic mode (`docs/architecture.md`, 10.4 and 10.5; spec 8): the scheduler, which runs the
//! ready node first in the global order on a simulated clock, and the interpreter, which runs each node by its kind. It
//! ends in one of the statuses of spec 8.2: `ok` or `error` when the function finishes, `aborted`, `fault`.
//!
//! **One thread runs the diagram**: the scheduler and the nodes share it. A native call does not block in deterministic
//! mode: it says how long it waits, and its node sleeps that long, so the waits of parallel branches overlap and a replay is
//! exact. Calls are frames on the heap, not calls of the interpreter on the stack.

use labcraft_check::CheckedFile;
use labcraft_graph::Id;
use labcraft_ir::{ElementId, FunctionId, Node, Program};
use labcraft_types::{Interner, LibraryId, Name};

use crate::fault::Fault;
use crate::json::Describe;
use crate::native::{HandleInfo, InstrumentLog, Logs, NativeLibrary, Simulation};
use crate::value::{Handle, Value};

mod machine;
mod memory;
mod nodes;
mod plan;
mod position;
#[cfg(test)]
mod tests;

pub use memory::NODE_COST;

/// The end of the clock of a run, 2^62 ns, about 146 years *(proposed, cli.md 6.2)*: past it, `LC8007`.
pub const CLOCK_END: i64 = 1 << 62;

/// How a run is bounded and what it is given besides its inputs (cli.md 6.2). The defaults are those of a deterministic run.
#[derive(Clone, Debug)]
pub struct Settings {
    /// `--schedule-seed`: the order among the nodes ready at one time is drawn from it instead of the global order.
    pub schedule_seed: Option<u64>,
    /// `--timeout`, in nanoseconds on the clock of the run: an abort when it is reached.
    pub timeout: Option<i64>,
    /// `--max-iterations`: of each `loop`, counted again each time the loop starts.
    pub max_iterations: Option<u64>,
    /// `--max-nodes`: nodes run in all, skipped ones and the nodes of calls included.
    pub max_nodes: Option<u64>,
    /// `--max-call-depth`: the top-level function is at depth 1.
    pub max_call_depth: u32,
    /// `--max-memory`, in bytes of logical size.
    pub max_memory: u64,
    /// `--max-real-time`, in nanoseconds of real time.
    pub max_real_time: Option<u64>,
    /// What a native call in progress is given to end after an abort or a fault, on the clock of the run *(proposed, 5 s)*.
    pub grace: i64,
    /// The settings of panel elements of the top-level function (`--set`, `set` of `run.start`), in the order given.
    pub panel: Vec<PanelSetting>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            schedule_seed: None,
            timeout: None,
            max_iterations: Some(10_000),
            max_nodes: Some(1_000_000),
            max_call_depth: 1024,
            max_memory: 1 << 30,
            max_real_time: Some(600_000_000_000),
            grace: 5_000_000_000,
            panel: Vec::new(),
        }
    }
}

/// A panel element of the top-level function set at a time of the run (cli.md 6.4).
#[derive(Clone, Debug)]
pub struct PanelSetting {
    pub element: ElementId,
    /// In the type of the element, converted by the caller.
    pub value: Value,
    /// Nanoseconds on the clock of the run.
    pub at: i64,
    pub by: SetBy,
}

/// What changed a panel element (commands.md 8, the event `set`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetBy {
    /// `ui.set`, a node of the program.
    Program,
    /// `--set`, or the `set` of `run.start`.
    Set,
    /// `run.set_element`, while the run goes.
    RunSetElement,
}

/// What asked for an abort (commands.md 7, `abort.by`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AbortBy {
    Request,
    Timeout,
    Interrupt,
}

/// What a run asks of the app around it: the runtime never reads a clock itself (10.4).
pub trait Control {
    /// The real time since the run started, in nanoseconds: what `--max-real-time` bounds.
    fn real_ns(&mut self) -> u64;
    /// An abort asked from outside since the last time it was asked: Ctrl-C, `run.abort`.
    fn abort(&mut self) -> Option<AbortBy> {
        None
    }
}

/// A native library of the run, by the id of its library in the table of types, with what it judged of its section of the
/// simulation file.
pub struct Native<'a> {
    pub library: LibraryId,
    pub native: &'a dyn NativeLibrary,
    pub simulation: Option<&'a Simulation>,
}

/// The calling node of a frame, for the event `call`.
pub struct Caller<'a> {
    pub frame: u64,
    pub node: &'a Node,
    pub iter: &'a [u64],
}

/// What the run tells as it goes: the trace hears it (commands.md 8). `iter` gives the iteration of each loop of the function
/// around the node, outermost first, from 0. Frame 0 is the top-level function.
pub trait Observer {
    fn call(&mut self, at: i64, frame: u64, caller: Option<Caller<'_>>, function: FunctionId, inputs: &[Value], handles: &dyn Describe) {
        let _ = (at, frame, caller, function, inputs, handles);
    }
    /// A node ended: it ran, or it skipped on an error of its `err` input (spec 7.2).
    fn ran(&mut self, at: i64, frame: u64, node: &Node, iter: &[u64], outputs: Outputs<'_>, handles: &dyn Describe) {
        let _ = (at, frame, node, iter, outputs, handles);
    }
    /// A `match` or a `handle` took the arm at this index.
    fn arm(&mut self, at: i64, frame: u64, node: &Node, iter: &[u64], arm: usize) {
        let _ = (at, frame, node, iter, arm);
    }
    fn set(&mut self, at: i64, frame: u64, element: ElementId, value: &Value, by: SetBy, handles: &dyn Describe) {
        let _ = (at, frame, element, value, by, handles);
    }
}

/// What a node gave when it ended: its outputs, in the order of its signature, and whether it skipped (spec 7.2), when they
/// are its defaults and the error passed on.
#[derive(Clone, Copy, Debug)]
pub struct Outputs<'a> {
    pub values: &'a [Value],
    pub skipped: bool,
}

/// An observer that hears nothing.
pub struct Unobserved;

impl Observer for Unobserved {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ok,
    /// The function finished with an error on its output `err` (spec 7.5).
    Error,
    Aborted,
    Fault,
}

/// A loop where it was: its function, its node, and its iteration, from 0.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoopAt {
    pub function: FunctionId,
    pub node: Name,
    pub iteration: u64,
}

/// Where a fault happened (commands.md 7, `fault`).
#[derive(Clone, Debug)]
pub struct FaultAt {
    pub fault: Fault,
    pub function: FunctionId,
    /// `None` for a fault at no node: a bug found between nodes.
    pub node: Option<Name>,
    /// The statement of the node, in the file of its function.
    pub origin: Option<Id>,
    /// The loops around it, from the top-level function down, outermost first.
    pub loops: Vec<LoopAt>,
    /// For `LC8001`, the iteration of the loop itself that would pass the limit, from 0.
    pub iteration: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct AbortAt {
    pub by: AbortBy,
    /// Nanoseconds on the clock of the run.
    pub at: i64,
    /// Each loop going when the abort came, outermost first.
    pub loops: Vec<LoopAt>,
}

/// A resource the run opened (spec 8.1): a handle that a native call gave.
#[derive(Clone, Debug)]
pub struct Resource {
    pub handle: Handle,
    /// What its library says of it when the run ended.
    pub info: Option<HandleInfo>,
    /// The node that opened it.
    pub function: FunctionId,
    pub node: Option<Name>,
    pub origin: Option<Id>,
    /// An abort or a fault closed it through its library.
    pub released: bool,
    /// A call that held it was abandoned when the grace period ended.
    pub leaked: bool,
}

/// A setting of the panel, and whether it applied before the run ended.
#[derive(Clone, Debug)]
pub struct Applied {
    pub setting: PanelSetting,
    pub applied: bool,
}

/// How a run ended: what its report says (commands.md 7).
#[derive(Debug)]
pub struct RunEnd {
    pub status: Status,
    /// The outputs of the function, in the order of its signature; none when it did not finish.
    pub outputs: Vec<Value>,
    pub fault: Option<FaultAt>,
    pub abort: Option<AbortAt>,
    /// The clock of the run when it ended, in nanoseconds.
    pub clock: i64,
    /// In the order they were opened.
    pub resources: Vec<Resource>,
    /// Of every library, in code point order of their resources.
    pub instruments: Vec<InstrumentLog>,
    /// What each library kept of the values it was given, by its name.
    pub logs: Vec<(String, Logs)>,
    pub panel: Vec<Applied>,
    /// Nodes run, skipped ones included.
    pub nodes: u64,
    /// The logical size still counted when the run ended: 0 when it finished, since its outputs are given back.
    pub held: u64,
}

/// What a run is given.
pub struct Run<'a> {
    pub program: &'a Program,
    pub names: &'a Interner,
    /// The files of the `Checked` the program was lowered from, which name where an error was made.
    pub files: &'a [CheckedFile],
    pub natives: &'a [Native<'a>],
    /// The inputs of the top-level function, in the order of its signature, in their types.
    pub inputs: Vec<Value>,
    pub settings: Settings,
}

/// Runs `functions[0]` of the program to its end, in deterministic mode. It never panics: a panic of a primitive or of a
/// native library is caught and ends the run with the fault `LC8004`.
pub fn run(run: Run<'_>, control: &mut dyn Control, observer: &mut dyn Observer) -> RunEnd {
    machine::Machine::new(run, control, observer).go()
}
