//! The desktop IDE (`docs/architecture.md`, 12.9): its command line, the session of a project on its thread, the
//! watching of the folder, the file of values of `--style`, the examples it copies, and the time of its frames
//! (`LABCRAFT_FRAMES`). The window is `ide`; the binary puts it on eframe.
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
pub mod examples;
pub mod frames;
pub mod session;
pub mod style_file;
pub mod watch;
