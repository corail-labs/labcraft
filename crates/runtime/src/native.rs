//! What a library with native functions gives the runtime (`docs/architecture.md`, section 10.6): its name, its codes, how
//! it judges its part of a simulation file, and what one run of it holds. A call of an `extern rust` function is a small
//! state machine that the library keeps: the scheduler starts it, then asks it for its next step after each wait, so a
//! call can wait in its middle (a simulated instrument that is busy) and a replay is exact.

use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use labcraft_types::{Severity, Span};

pub use crate::io::Context;
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
    /// The endpoints of io.md 2 that its judged section describes (`serial:COM3`, `tcp:10.0.0.7:502`), for every library of
    /// the run: the host gives their union to each one (`Context::endpoints`), so that `serial.list` under `--sim` gives the
    /// serial endpoints of the whole file (io.md 6). None by default. Apart from [`NativeLibrary::resources`], which names
    /// what a file defines as a program opens it: for `visa` its `SIM::` names, which are no endpoints.
    fn endpoints(&self, simulation: &Simulation) -> Vec<String> {
        let _ = simulation;
        Vec::new()
    }
    /// What one run of it holds, from what [`Start`] gives it.
    fn start_run(&self, start: Start<'_>) -> Box<dyn LibraryRun>;
}

/// What one run of a library is given as it starts (`docs/architecture.md`, 10.6).
pub struct Start<'a> {
    /// Its index among the libraries of the run, which its handles and its keys carry.
    pub library: u32,
    /// What it judged of its section of the simulation file, when the file has one.
    pub simulation: Option<&'a Simulation>,
    /// The memory that every library of the run shares for what it keeps for the report.
    pub budget: LogBudget,
    pub mode: Mode,
    /// What the host says of the run: whether a simulation file was given, the project, who started it (io.md 7).
    pub context: &'a Context,
}

/// The mode of a run (spec 8): on a simulated clock, or on the real one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Deterministic,
    RealTime,
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

/// What a call can wait for alone: a session, an instrument. The scheduler gives it to the call that waits for it first in the
/// global order of spec 8, in real time too, since one thread steps every call. The runtime locks nothing on its own (spec
/// 8.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key {
    pub library: u32,
    pub id: u64,
}

/// Work that blocks (a read of a real instrument, later a file), which a call gives the runtime to do away from the thread of
/// the diagram: on a thread of the pool in real time, at once on the thread of the run in deterministic mode and without
/// threads. What it returns is given back to the call by [`LibraryRun::resume`] before its next step.
pub type Job = Box<dyn FnOnce() -> Box<dyn Any + Send> + Send>;

/// What a call does next.
pub enum Step {
    Done(NativeOutcome),
    /// Waits this many nanoseconds on the clock of the run (`LC8007` past its end).
    Sleep(i64),
    /// Waits until it holds the key alone; the keys it still holds are given back when it is done.
    Exclusive(Key),
    Release(Key),
    /// Waits for work that blocks: the keys it holds stay held meanwhile (`docs/architecture.md`, 10.6).
    Blocking(Job),
}

impl std::fmt::Debug for Step {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Step::Done(outcome) => f.debug_tuple("Done").field(outcome).finish(),
            Step::Sleep(ns) => f.debug_tuple("Sleep").field(ns).finish(),
            Step::Exclusive(key) => f.debug_tuple("Exclusive").field(key).finish(),
            Step::Release(key) => f.debug_tuple("Release").field(key).finish(),
            Step::Blocking(_) => f.write_str("Blocking(..)"),
        }
    }
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
    /// Its opaque type, as the library names it: `session`; that of one of its sub-libraries with its path, `sim.instrument`.
    pub kind: &'static str,
    /// `SIM::DMM0`
    pub designates: String,
    pub open: bool,
}

/// The commands a simulated instrument received, for the report (commands.md 7, `instruments`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InstrumentLog {
    pub resource: String,
    pub commands: Vec<CommandLog>,
    /// Commands past the ones kept.
    pub more: u64,
    /// The opens of a simulated endpoint with settings its device does not expect (`serial`, io.md 6), in order.
    pub settings: Vec<SettingsLog>,
    /// Opens past the ones kept.
    pub settings_more: u64,
}

/// An open of a simulated endpoint whose settings its device does not expect, for the report (commands.md 7,
/// `instruments`, `settings`): when, and each setting that differs.
#[derive(Clone, Debug, PartialEq)]
pub struct SettingsLog {
    /// When the port was opened, in nanoseconds on the clock of the run.
    pub at: i64,
    /// Only the settings that differ, in the order of the library.
    pub differ: Vec<SettingDiff>,
}

/// A setting that differs: its name (`baud`), what the device expects and what the open gave.
#[derive(Clone, Debug, PartialEq)]
pub struct SettingDiff {
    pub name: &'static str,
    pub expected: Setting,
    pub opened: Setting,
}

/// The value of a setting in the report: a number (`9600`, `2`) or a word (`"even"`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    Integer(i64),
    Text(&'static str),
}

/// A command as the report shows it (commands.md 7): when it was handled, what was sent and what was answered. An entry of a
/// stream (`net`, io.md 6) is a write, its `sent` and no answer, or a read, its `answer` and no `sent`.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandLog {
    /// When the command was handled, after its latency, in nanoseconds.
    pub at: i64,
    /// `None` for a read of a stream.
    pub sent: Option<LogText>,
    /// `None` for a write, and for a query never answered.
    pub answer: Option<LogText>,
}

/// A text of the report: its first 1,000 code points when it is kept (commands.md 5), with its whole length, so that what a
/// program sends cannot grow the memory of the run beyond `--max-memory`.
#[derive(Clone, Debug, PartialEq)]
pub struct LogText {
    pub kept: String,
    /// In code points.
    pub length: u64,
    /// The text is the bytes in lowercase hexadecimal, since they are not text (io.md 6): `"hex": true` in the report.
    pub hex: bool,
}

/// How many entries the log of one resource keeps for the report, of its commands and of its opens with settings its device
/// does not expect *(proposed)*.
pub const KEPT_ENTRIES: usize = 1000;
/// How much of a text the report shows (commands.md 5).
pub const KEPT_TEXT: usize = 1000;

/// What a library gives the log of a resource: a text, or bytes, which the log writes as text when they are UTF-8 without
/// control characters other than `\r`, `\n` and `\t`, else in hexadecimal (io.md 6).
#[derive(Clone, Copy, Debug)]
pub enum Logged<'a> {
    Text(&'a str),
    Bytes(&'a [u8]),
}

fn shown_as_text(c: char) -> bool {
    !c.is_control() || matches!(c, '\r' | '\n' | '\t')
}

/// Bytes as the report and a description write them (io.md 6): their text when they are UTF-8 without control characters
/// other than `\r`, `\n` and `\t`, else lowercase hexadecimal (`true`).
pub fn bytes_text(bytes: &[u8]) -> (String, bool) {
    match std::str::from_utf8(bytes) {
        Ok(text) if text.chars().all(shown_as_text) => (text.to_owned(), false),
        _ => (crate::text::hex(bytes), true),
    }
}

/// How an entry writes what it was given: its text, or its bytes in hexadecimal.
#[derive(Clone, Copy)]
enum Written<'a> {
    Text(&'a str),
    Hex(&'a [u8]),
}

impl<'a> Logged<'a> {
    fn written(self) -> Written<'a> {
        match self {
            Logged::Text(text) => Written::Text(text),
            Logged::Bytes(bytes) => match std::str::from_utf8(bytes) {
                Ok(text) if text.chars().all(shown_as_text) => Written::Text(text),
                _ => Written::Hex(bytes),
            },
        }
    }
}

impl Written<'_> {
    /// What it costs the budget, before anything is copied: its size in bytes as written, at most four bytes a code point
    /// kept.
    fn cost(self) -> u64 {
        let size = match self {
            Written::Text(text) => text.len(),
            Written::Hex(bytes) => bytes.len().saturating_mul(2),
        };
        size.min(4 * KEPT_TEXT) as u64
    }

    /// What the report keeps of it: its first 1,000 code points, and its length.
    fn kept(self) -> LogText {
        match self {
            Written::Text(text) => {
                let mut length = 0_u64;
                let mut kept = String::new();
                for c in text.chars() {
                    if length < KEPT_TEXT as u64 {
                        kept.push(c);
                    }
                    length += 1;
                }
                LogText { kept, length, hex: false }
            }
            Written::Hex(bytes) => LogText {
                kept: crate::text::hex(bytes.get(..bytes.len().min(KEPT_TEXT / 2)).unwrap_or_default()),
                length: u64::try_from(bytes.len()).unwrap_or(u64::MAX).saturating_mul(2),
                hex: true,
            },
        }
    }
}

/// The start of what a log was given, then a count: from the first item that does not fit the budget of the run, or past
/// [`KEPT_ENTRIES`], items are counted, not kept.
#[derive(Clone, Debug, PartialEq)]
pub struct Bounded<T> {
    pub items: Vec<T>,
    /// Items counted, not kept.
    pub more: u64,
}

impl<T> Default for Bounded<T> {
    fn default() -> Self {
        Bounded { items: Vec::new(), more: 0 }
    }
}

impl<T> Bounded<T> {
    /// Keeps the item `item` makes, at `cost` bytes from `budget`, or counts it.
    pub fn push(&mut self, budget: &LogBudget, cost: u64, item: impl FnOnce() -> T) {
        if self.full() || !budget.take(cost) {
            self.more = self.more.saturating_add(1);
            return;
        }
        self.items.push(item());
    }

    /// Whether it keeps no more: it counted one already, or holds [`KEPT_ENTRIES`].
    pub fn full(&self) -> bool {
        self.more > 0 || self.items.len() >= KEPT_ENTRIES
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty() && self.more == 0
    }
}

/// The log of one resource for the report (commands.md 7, `instruments`): the start of what it saw, then a count. Each entry
/// kept costs its texts, at most four bytes a code point kept, and 64 bytes, from the budget of the run; each open with
/// settings its device does not expect, 64 bytes and 64 a setting.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResourceLog {
    pub entries: Bounded<CommandLog>,
    /// The opens with settings the device does not expect.
    pub settings: Bounded<SettingsLog>,
}

impl ResourceLog {
    /// An open with settings the device does not expect.
    pub fn settings(&mut self, budget: &LogBudget, at: i64, differ: Vec<SettingDiff>) {
        let cost = 64_u64.saturating_add(64_u64.saturating_mul(differ.len() as u64));
        self.settings.push(budget, cost, || SettingsLog { at, differ });
    }

    pub fn entry(&mut self, budget: &LogBudget, at: i64, sent: Option<Logged<'_>>, answer: Option<Logged<'_>>) {
        // Full, it counts the entry without looking at its texts.
        if self.entries.full() {
            self.entries.more = self.entries.more.saturating_add(1);
            return;
        }
        let (sent, answer) = (sent.map(Logged::written), answer.map(Logged::written));
        let cost = sent.map_or(0, Written::cost) + answer.map_or(0, Written::cost) + 64;
        self.entries.push(budget, cost, || CommandLog { at, sent: sent.map(Written::kept), answer: answer.map(Written::kept) });
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.settings.is_empty()
    }

    /// For the report, by the name of its resource.
    pub fn instrument(&self, resource: &str) -> InstrumentLog {
        InstrumentLog {
            resource: resource.to_owned(),
            commands: self.entries.items.clone(),
            more: self.entries.more,
            settings: self.settings.items.clone(),
            settings_more: self.settings.more,
        }
    }
}

/// What a library of raw streams (`net`, `serial`) keeps for the report (commands.md 7, `instruments`; io.md 6): for each
/// endpoint reached, simulated or real, one entry per write and one per read, in order, and each open with settings its
/// simulated device does not expect.
#[derive(Debug)]
pub struct EndpointLogs {
    endpoints: std::collections::BTreeMap<String, ResourceLog>,
    budget: LogBudget,
}

impl EndpointLogs {
    pub fn new(budget: LogBudget) -> Self {
        EndpointLogs { endpoints: std::collections::BTreeMap::new(), budget }
    }

    /// Gives the log of `endpoint`, made at its first use, and the budget.
    fn with(&mut self, endpoint: &str, write: impl FnOnce(&mut ResourceLog, &LogBudget)) {
        let EndpointLogs { endpoints, budget } = self;
        match endpoints.get_mut(endpoint) {
            Some(log) => write(log, budget),
            None => write(endpoints.entry(endpoint.to_owned()).or_default(), budget),
        }
    }

    /// One write (`sent`) or one read, at `at` on the clock of the run.
    pub fn entry(&mut self, endpoint: &str, at: i64, sent: bool, bytes: &[u8]) {
        let logged = Some(Logged::Bytes(bytes));
        let (sent, answer) = if sent { (logged, None) } else { (None, logged) };
        self.with(endpoint, |log, budget| log.entry(budget, at, sent, answer));
    }

    /// An open at `at` with settings the device does not expect.
    pub fn settings(&mut self, endpoint: &str, at: i64, differ: Vec<SettingDiff>) {
        self.with(endpoint, |log, budget| log.settings(budget, at, differ));
    }

    /// In code point order of the endpoints; one that saw nothing is not listed.
    pub fn instruments(&self) -> Vec<InstrumentLog> {
        self.endpoints.iter().filter(|(_, log)| !log.is_empty()).map(|(endpoint, log)| log.instrument(endpoint)).collect()
    }
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
    /// What the [`Job`] of a call returned, given back before its next step.
    fn resume(&mut self, call: CallId, done: Box<dyn Any + Send>) {
        let _ = (call, done);
    }
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::{LogBudget, Logged, ResourceLog};

    /// commands.md 7: an entry costs its texts, at most four bytes a code point kept, and 64 bytes. By hand: 2,500 `é` are
    /// 5,000 bytes, so 4,000 + 64 = 4,064 bytes; 3 bytes not text, `00 ff 07` in hexadecimal, 6 + 64 = 70.
    #[test]
    fn an_entry_costs_its_size_as_written() {
        let text = "é".repeat(2500);
        for (budget, kept) in [(4063, 0), (4064, 1)] {
            let mut log = ResourceLog::default();
            log.entry(&LogBudget::new(budget), 0, Some(Logged::Text(&text)), None);
            assert_eq!((log.entries.items.len(), log.entries.more), (kept, 1 - kept as u64), "{budget}");
        }
        let budget = LogBudget::new(70);
        let mut log = ResourceLog::default();
        log.entry(&budget, 0, None, Some(Logged::Bytes(&[0, 255, 7])));
        let answer = log.entries.items[0].answer.as_ref().unwrap();
        assert_eq!((answer.kept.as_str(), answer.length, answer.hex, budget.left()), ("00ff07", 6, true, 0));
    }
}
