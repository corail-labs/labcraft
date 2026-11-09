//! L1: the trace of a run (`labcraft-trace/1`, `docs/commands.md`, 8) and the JSON form of values (section 5), shared by
//! the runtime that writes them, the report of a run, and the viewer that reads them (`docs/architecture.md`, 10.7 and 11.2),
//! so that what a run writes and what the viewer reads cannot drift apart. It sees no other crate of LabCraft: `runtime`
//! turns its values into this form.
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

mod json;
pub mod number;
mod read;
mod stale;
pub mod value;
mod write;

pub use read::{ArmOf, Expected, ExpectedNode, ExpectedWire, HeaderProblem, Limit, ReadLimits, Reader, Stop, Trace, TraceHeader};
pub use stale::{sha256_hex, stale};
pub use value::{ErrorValue, Origin, Place, ValueWriter, WriteValue, Written, write_value};
pub use write::{Caller, Event, Header, Input, SCHEMA, TraceWriter, WriteLimits};
