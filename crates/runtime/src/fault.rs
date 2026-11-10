//! A fault ends the run where it happens (spec 8.2): the codes of `run` (cli.md 6.6), of a limit, a value or a bug. The
//! interpreter adds where it happened.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultCode {
    /// `LC8001`: the iteration limit of a `loop` is reached.
    Iterations,
    /// `LC8002`: the limit of nodes run is reached.
    Nodes,
    /// `LC8003`: the call depth limit is reached.
    Depth,
    /// `LC8004`: a bug of the runtime, caught.
    Bug,
    /// `LC8005`: the memory limit of the run would be passed.
    Memory,
    /// `LC8008`: a value that its operation cannot take, which no `err` can say (Q-25): a number that its type cannot hold (an
    /// integer that overflows, `core.to_i32` or `core.round_to_i32` of `nan` or of a value out of the `i32`, a `duration` that
    /// is not finite or does not fit), or the empty separator of `string.split`.
    OutOfRange,
    /// `LC8006`: the real time of a deterministic run is spent, which depends on the machine.
    RealTime,
    /// `LC8007`: the clock of the run would pass its end, 2^62 ns.
    Clock,
}

impl FaultCode {
    pub fn as_str(self) -> &'static str {
        match self {
            FaultCode::Iterations => "LC8001",
            FaultCode::Nodes => "LC8002",
            FaultCode::Depth => "LC8003",
            FaultCode::Bug => "LC8004",
            FaultCode::Memory => "LC8005",
            FaultCode::RealTime => "LC8006",
            FaultCode::Clock => "LC8007",
            FaultCode::OutOfRange => "LC8008",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fault {
    pub code: FaultCode,
    pub message: String,
}

impl Fault {
    pub fn bug(message: impl Into<String>) -> Self {
        Fault { code: FaultCode::Bug, message: message.into() }
    }

    pub fn out_of_range(message: impl Into<String>) -> Self {
        Fault { code: FaultCode::OutOfRange, message: message.into() }
    }

    pub fn memory(message: impl Into<String>) -> Self {
        Fault { code: FaultCode::Memory, message: message.into() }
    }

    pub fn new(code: FaultCode, message: impl Into<String>) -> Self {
        Fault { code, message: message.into() }
    }
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}
