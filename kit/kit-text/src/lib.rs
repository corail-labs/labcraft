//! `kit-text`: the text of a code field, for any IDE (`docs/ide.md`, sections 7 and 11): the text in a rope, one cursor
//! and its selection, the undo of the field, and the colours of its lines, which a [`Highlighter`] of the IDE gives
//! from the lexer of its language, kept line by line. With no UI toolkit: `kit-text-egui` paints it and takes its keys.
//!
//! It depends on no crate of LabCraft (`cargo xtask layers`). The language service (diagnostics, completions, hover,
//! definitions, in the types of `lsp-types`) comes with step 7b of M1.
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

pub mod colours;
pub mod field;

pub use colours::{Colours, Highlighter, Span};
pub use field::{Action, Field, Motion, Selection, Splice};

#[cfg(test)]
mod tests;
