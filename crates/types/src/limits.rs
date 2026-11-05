//! Reaching a limit is a diagnostic with a code, never a crash (`docs/cli.md`, section 1).

const MIB: u64 = 1024 * 1024;

/// The hard ceiling of nesting: a stack is sized from it, so no option can go beyond it. WASM has less: the
/// engine also keeps a call stack of its own, which no build or host setting can enlarge in a browser
/// (`docs/architecture.md`, section 4).
pub const MAX_NESTING: u32 = if cfg!(target_arch = "wasm32") { 256 } else { 1024 };

/// The most that the command line lets `--max-file-size` ask for, as much as a whole library holds: reading and
/// formatting take many times the size of a file, and running out of memory ends the process.
pub const MAX_FILE_SIZE: u64 = 1024 * MIB;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Size of one file (`LC0006`).
    pub max_file_size: u64,
    /// Total size of a library, since a caller hands all its files at once.
    pub max_library_size: u64,
    /// Diagnostics listed in a report; the counts stay complete.
    pub max_report: usize,
    max_nesting: u32,
}

impl Default for Limits {
    /// The defaults proposed by `docs/cli.md`, section 1, and `docs/architecture.md`, section 2.
    fn default() -> Self {
        Limits { max_file_size: 16 * MIB, max_library_size: 1024 * MIB, max_report: 1000, max_nesting: 256 }
    }
}

impl Limits {
    /// Depth of nested structures, parentheses and types (`LC0007`).
    pub fn max_nesting(&self) -> u32 {
        self.max_nesting
    }

    /// `None` beyond [`MAX_NESTING`], which the command line reports as wrong usage.
    #[must_use]
    pub fn with_max_nesting(self, max_nesting: u32) -> Option<Self> {
        (max_nesting <= MAX_NESTING).then_some(Limits { max_nesting, ..self })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_those_of_the_command_line() {
        let limits = Limits::default();
        assert_eq!(limits.max_file_size, 16 * 1024 * 1024);
        assert_eq!(limits.max_nesting(), 256);
        assert_eq!(limits.max_report, 1000);
    }

    #[test]
    fn nesting_has_a_hard_ceiling() {
        assert_eq!(Limits::default().with_max_nesting(MAX_NESTING).map(|limits| limits.max_nesting()), Some(MAX_NESTING));
        assert_eq!(Limits::default().with_max_nesting(MAX_NESTING + 1), None);
    }
}
