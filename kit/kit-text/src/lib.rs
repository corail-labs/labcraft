//! `kit-text`: the text of a code field, for any IDE (`docs/ide.md`, sections 7 and 11): the text in a rope, one cursor
//! and its selection, the undo of the field, and the colours of its lines, which a [`Highlighter`] of the IDE gives
//! from the lexer of its language, kept line by line. With no UI toolkit: `kit-text-egui` paints it and takes its keys.
//!
//! The language service of the IDE ([`LanguageService`]) gives diagnostics, completions, hover and definitions, in the
//! types of `lsp-types`; [`Marks`] keeps the diagnostics on the text as it is typed, and [`Completion`] the list of what
//! can be written.
//!
//! It depends on no crate of LabCraft (`cargo xtask layers`).
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
pub mod complete;
pub mod field;
pub mod marks;
pub mod service;

pub use colours::{Colours, Highlighter, Span};
pub use complete::Completion;
pub use field::{Action, Edit, Field, Motion, Selection, Splice};
pub use lsp_types;
pub use marks::{Mark, Marks};
pub use service::{ENCODING, LanguageService, Published, Silent, offset_in, offset_of, position_of};

/// What a code field holds: its text, the colours of its lines, the diagnostics on it and the list of completion.
#[derive(Clone)]
pub struct Code<H: Highlighter> {
    pub field: Field,
    pub colours: Colours<H>,
    pub marks: Marks,
    pub completion: Completion,
}

impl<H: Highlighter> Code<H> {
    pub fn new(highlighter: H) -> Code<H> {
        Code { field: Field::default(), colours: Colours::new(highlighter), marks: Marks::default(), completion: Completion::default() }
    }

    /// Takes what the service published into the marks.
    pub fn take_published(&mut self, service: &mut dyn LanguageService) {
        for published in service.diagnostics() {
            self.marks.publish(&self.field, published.source, published.revision, &published.text, published.diagnostics);
        }
        self.marks.follow(&self.field);
    }
}

#[cfg(test)]
mod tests;
