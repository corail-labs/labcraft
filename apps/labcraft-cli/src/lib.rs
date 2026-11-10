//! The command line (`docs/cli.md`), as a library too: the viewer (`apps/labcraft`) reads and checks a folder, and draws
//! its functions, as `diagram` does (`folder`), until the session of `engine` does it for both (architecture 11.5).
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

pub mod args;
pub mod check;
pub mod diagram;
pub mod fmt;
pub mod folder;
pub mod output;
pub mod walk;
