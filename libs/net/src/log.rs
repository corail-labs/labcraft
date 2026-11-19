//! What a run of `net` keeps for the report (`docs/commands.md`, section 7, `instruments`; `docs/io.md`, section 6): for each
//! endpoint reached, simulated or real, one entry per write and one per read, in order, each kept as `runtime::native`
//! keeps the log of a resource.

use std::collections::BTreeMap;

use labcraft_runtime::native::{InstrumentLog, LogBudget, Logged, ResourceLog};

/// The entries of every endpoint of a run.
#[derive(Debug)]
pub(crate) struct Log {
    endpoints: BTreeMap<String, ResourceLog>,
    budget: LogBudget,
}

impl Log {
    pub(crate) fn new(budget: LogBudget) -> Self {
        Log { endpoints: BTreeMap::new(), budget }
    }

    /// One write (`sent`) or one read, at `at` on the clock of the run.
    pub(crate) fn entry(&mut self, endpoint: &str, at: i64, sent: bool, bytes: &[u8]) {
        let logged = Some(Logged::Bytes(bytes));
        let (sent, answer) = if sent { (logged, None) } else { (None, logged) };
        match self.endpoints.get_mut(endpoint) {
            Some(log) => log.entry(&self.budget, at, sent, answer),
            None => self.endpoints.entry(endpoint.to_owned()).or_default().entry(&self.budget, at, sent, answer),
        }
    }

    /// In code point order of the endpoints.
    pub(crate) fn instruments(&self) -> Vec<InstrumentLog> {
        self.endpoints.iter().filter(|(_, log)| !log.is_empty()).map(|(endpoint, log)| log.instrument(endpoint)).collect()
    }
}
