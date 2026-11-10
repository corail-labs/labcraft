//! L6: the native body of `results_log.append_csv`, a library of the examples (spec A.4). It does not write a file: it keeps
//! the results it is given in memory, as the proof of concept does, and the report of the run shows them (commands.md 7,
//! `logs`; decision of the owner, 2026-11-09). A call does not wait and holds no key.
//!
//! No crate of `crates/` depends on this one (`cargo xtask libs`): the apps and the harness will link it and give it to the
//! run (steps 5 and 7). A call that the run steps once it has ended, which only a defect of the scheduler does, gets the error
//! `results_log.not_in_progress` (architecture 10.6).
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

use std::collections::HashMap;

use labcraft_runtime::native::{
    CallContext, CallId, HandleInfo, LibraryCode, LibraryRun, LogBudget, LogEntry, Logs, NativeLibrary, NativeOutcome, Simulation, Step,
};
use labcraft_runtime::{Handle, Value, logical_size};

#[cfg(test)]
mod tests;

/// The entries a run keeps *(proposed)*; the ones after them are counted. Each entry kept also costs the logical size of its
/// value (cli.md 6.2) from the budget of the run (`LogBudget`), which it shares with its other libraries: from the first entry
/// that does not fit, entries are counted.
pub const KEPT_ENTRIES: usize = 1000;

/// The library `results_log`, as the run is given it.
#[derive(Clone, Copy, Debug, Default)]
pub struct ResultsLog;

impl NativeLibrary for ResultsLog {
    fn name(&self) -> &str {
        "results_log"
    }

    /// It reads no part of a simulation file, so it has no code of its own.
    fn codes(&self) -> &'static [LibraryCode] {
        &[]
    }

    fn start_run(&self, _library: u32, _simulation: Option<&Simulation>, budget: LogBudget) -> Box<dyn LibraryRun> {
        Box::new(ResultsLogRun { calls: HashMap::new(), next_call: 0, logs: Logs::default(), budget })
    }
}

/// One run of `results_log`: the calls started and not yet stepped, and the log.
#[derive(Debug)]
pub struct ResultsLogRun {
    calls: HashMap<CallId, Value>,
    next_call: u64,
    logs: Logs,
    budget: LogBudget,
}

impl LibraryRun for ResultsLogRun {
    /// `append_csv(results, err)`: the run calls it only when `err` holds no error (spec 7.2), so only `results` counts.
    fn call(&mut self, function: &str, inputs: Vec<Value>) -> Option<CallId> {
        if function != "append_csv" {
            return None;
        }
        let results = inputs.into_iter().next().unwrap_or_else(|| Value::array(Vec::new()));
        let id = CallId(self.next_call);
        self.next_call = self.next_call.saturating_add(1);
        self.calls.insert(id, results);
        Some(id)
    }

    fn step(&mut self, call: CallId, cx: CallContext) -> Step {
        let Some(results) = self.calls.remove(&call) else {
            return Step::Done(NativeOutcome::Fail {
                category: "results_log.not_in_progress",
                code: -1,
                message: "a call that is not in progress".to_owned(),
            });
        };
        let size = logical_size(&results);
        // The log is the first entries: once one is counted, so are the ones after it.
        if self.logs.more == 0 && self.logs.entries.len() < KEPT_ENTRIES && self.budget.take(size) {
            self.logs.entries.push(LogEntry { function: "append_csv".to_owned(), input: 0, at: cx.now, value: results });
        } else {
            self.logs.more = self.logs.more.saturating_add(1);
        }
        Step::Done(NativeOutcome::Outputs(vec![Value::NONE]))
    }

    fn cancel(&mut self, call: CallId) {
        self.calls.remove(&call);
    }

    /// It makes no handle.
    fn describe(&self, _handle: Handle) -> Option<HandleInfo> {
        None
    }

    fn release(&mut self, _handle: Handle) {}

    fn logs(&self) -> Logs {
        self.logs.clone()
    }
}
