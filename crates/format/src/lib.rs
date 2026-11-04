//! L2: reading and writing the text format (`docs/architecture.md`, section 4). It takes bytes and
//! returns values: only the command line touches the file system.
//!
//! Diagnostics made here have an empty `file`, since this crate does not know where the bytes came
//! from. Each file has a `Report` of its own, which the caller names with `Report::set_file` before
//! merging it. A file that gets a code that stands alone (`docs/cli.md`, section 1) keeps that code
//! only, whatever warnings came before, so its report is replaced rather than merged.
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

mod decode;
mod lexer;

pub use decode::decode;
pub use lexer::{Keyword, Lexer, Token, TokenKind};
