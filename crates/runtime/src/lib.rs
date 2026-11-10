//! L5: what runs a program (`docs/architecture.md`, section 10): its values (10.3), what a wire does to them, the value of
//! each constant, the bodies of the primitives (10.5), the interface of native libraries (10.6), the scheduler and the
//! interpreter in deterministic mode (10.4, 10.5), and the trace and the report of a run (10.7).
//!
//! Nothing here recurses on what a run can make deep but a type: a value nests as deep as its type, as `check` and `ir`
//! allow it, on the stack of the thread of the run (`format::STACK_SIZE`).
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

mod convert;
mod fault;
pub mod json;
pub mod native;
pub mod primitives;
pub mod report;
pub mod run;
pub mod simulation;
mod text;
pub mod trace;
mod value;

pub use convert::{Env, constant, convert, duration_of_seconds, grid, integer, read};
pub use fault::{Fault, FaultCode};
pub use primitives::{Call, Host, Outcome, equal, outputs_on_error};
pub use text::{Text, Writer, error_text, quoted};
pub use value::{ErrorData, Grid, Handle, Items, Origin, Place, VALUE_COST, Value, VariantValue, logical_size};
