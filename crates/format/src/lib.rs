//! L2: reading and writing the text format (`docs/architecture.md`, section 4). It takes bytes and
//! returns values: only the command line touches the file system.
//!
//! Diagnostics made here have an empty `file`, since this crate does not know where the bytes came
//! from. Each file has a `Report` of its own, which the caller names with `Report::set_file` before
//! merging it. A file that gets a code that stands alone (`docs/cli.md`, section 1) keeps that code
//! only, whatever warnings came before: `read` gives it a new report.
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
mod parser;
mod read;

pub use decode::decode;
pub use lexer::{Keyword, Lexer, Token, TokenKind};
pub use read::{Read, STACK_SIZE, read};
