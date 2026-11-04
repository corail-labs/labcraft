//! L0: what everything needs, with no knowledge of files (`docs/architecture.md`, section 2).
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

mod code;
mod limits;
mod name;
mod report;
mod ty;
mod unit;

pub use code::{Code, Severity};
pub use limits::{Limits, MAX_NESTING};
pub use name::{Interner, Name, normalize};
pub use report::{
    CheckCounts, CheckDocument, CheckSummary, Diagnostic, Edit, FileEntry, FileResult, Fix, FmtDocument, FmtSummary, MAX_LIST, MAX_TEXT, Node,
    Position, Related, Report, Span, Status, Wire, cut_text,
};
pub use ty::{FnParam, LibraryId, MAX_ARRAY_DIMENSIONS, NumberKind, Ty, TyId, TypeOutOfBounds, TypeTable, Variant};
pub use unit::{Unit, UnitError, parse_unit};
