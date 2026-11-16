//! `kit-shell`: the shell of the window of an IDE, for any IDE (`docs/ide.md`, section 11): the top bar, the
//! workspaces with their planned entries, the start page, the command palette and its JSON form, the inspector as
//! a list of fields, the status bar. On egui; the docking and the detached windows come from `craft-ui` and
//! `craft-window`, the menus from `craft-menu`.
//!
//! It depends on no crate of LabCraft (`cargo xtask layers`): it knows no command, no file and no language. The host
//! gives what to show, as data, and turns what comes back into its own commands. Offered to `craft-libs` once stable.
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

pub mod frame;
pub mod fuzzy;
pub mod icon;
pub mod inspector;
pub mod palette;
mod planned;
pub mod start;
pub mod status;
pub mod theme;
pub mod top_bar;
pub mod workspaces;

pub use planned::{Card, Planned};
