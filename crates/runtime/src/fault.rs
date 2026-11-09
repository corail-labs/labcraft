//! A fault ends the run where it happens (spec 8.2): the codes of `run` (cli.md 6.6) that a value or a primitive can reach.
//! The interpreter adds where it happened.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultCode {
    /// `LC8004`: a bug of the runtime, caught.
    Bug,
    /// `LC8005`: the memory limit of the run would be passed.
    Memory,
    /// `LC8008`: a value that its operation cannot take, which no `err` can say (Q-25): a number that its type cannot hold (an
    /// integer that overflows, `core.to_i32` or `core.round_to_i32` of `nan` or of a value out of the `i32`, a `duration` that
    /// is not finite or does not fit), or the empty separator of `string.split`.
    OutOfRange,
}

impl FaultCode {
    pub fn as_str(self) -> &'static str {
        match self {
            FaultCode::Bug => "LC8004",
            FaultCode::Memory => "LC8005",
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
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}
