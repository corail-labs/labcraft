//! L5: what runs a program (`docs/architecture.md`, section 10). So far its values (10.3), what a wire does to them, the
//! value of each constant, and the bodies of the primitives (10.5); the scheduler, the interpreter and native libraries come
//! next (10.11, steps 4 and 5).
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
pub mod primitives;
mod text;
mod value;

pub use convert::{Env, constant, convert, duration_of_seconds, grid, read};
pub use fault::{Fault, FaultCode};
pub use primitives::{Call, Host, Outcome, outputs_on_error};
pub use text::{Text, Writer, error_text, quoted};
pub use value::{ErrorData, Grid, Handle, Items, Origin, Place, VALUE_COST, Value, VariantValue, logical_size};
