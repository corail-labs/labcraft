//! The desktop viewer of M0v (`docs/architecture.md`, 11.5), read-only: its command line, its session, which holds the
//! state of the viewer and answers the `view.*` commands (`docs/commands.md`, 6.5), the watching of the folder, the file
//! of values of `--style`, and the time of its frames (`LABCRAFT_FRAMES`).
//! The window around the canvas is the binary.
//!
//! The session is provisional (decided at the start of step 6): it goes into `engine::view` once M0b merges `engine`,
//! with the same commands and results.
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
pub mod frames;
pub mod session;
pub mod style_file;
pub mod watch;
