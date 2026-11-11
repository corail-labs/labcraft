//! L6: the session and the registry of commands (`docs/architecture.md`, 10.8; `docs/commands.md`). Everything LabCraft
//! does is a command (D-18): the command line, the MCP server and the viewer call [`Session::execute`] with an id and
//! JSON parameters. The files of a project are reached through the [`Workspace`] that the app gives: only an app
//! touches the file system.
//!
//! `labcraft-cli check` and `fmt` take folders and files anywhere on the disk, named from the current folder, which a
//! command does not (`docs/commands.md`, section 2): they call [`folders::check_folders`] and [`fmt::format_paths`],
//! the code behind `library.check` and `file.format`.
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

mod bound;
mod catalog;
pub mod drawings;
pub mod error;
mod files;
pub mod fmt;
pub mod folders;
mod functions;
mod kept;
pub mod panics;
mod params;
pub mod paths;
mod project;
pub mod registry;
pub mod runs;
pub mod session;
pub mod shipped;
mod simulations;
pub mod view;
pub mod walk;
pub mod workspace;

pub use error::CommandError;
pub use runs::{Clock, Runs, Still, run_to_end};
pub use session::{Config, Session};
pub use shipped::{Shipped, ShippedFile, ShippedLibrary};
pub use workspace::{Memory, ReadOnly, Workspace};

#[cfg(test)]
mod tests;
