//! The real clock of the command line (`docs/architecture.md`, 10.10), which `run` and `mcp` give their session.

use std::time::Instant;

use labcraft_engine::Clock;

/// The real clock, monotonic, from the moment it is made.
pub struct Real(Instant);

impl Real {
    #[expect(clippy::disallowed_methods, reason = "the command line, never built for WASM, where the clock traps")]
    pub fn new() -> Real {
        Real(Instant::now())
    }
}

impl Default for Real {
    fn default() -> Real {
        Real::new()
    }
}

impl Clock for Real {
    fn real_ns(&self) -> u64 {
        u64::try_from(self.0.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }

    #[expect(clippy::disallowed_methods, reason = "the command line, never built for WASM, where the clock traps")]
    fn utc_ns(&self) -> i64 {
        let since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        i64::try_from(since.as_nanos()).unwrap_or(i64::MAX)
    }
}
