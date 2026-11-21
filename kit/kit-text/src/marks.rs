//! The diagnostics underlined in a field (ide.md 7; decision of the owner, 2026-11-21): each is placed on the text it
//! was given for, then follows the edits typed after it, so that nothing blinks between two checks. A new set of a
//! source replaces the old one of that source; the others stay.

use std::ops::Range;

use lsp_types::{Diagnostic, DiagnosticSeverity};

use crate::field::{Edit, Field};
use crate::service::Lines;

/// A diagnostic on the text: its range in characters of the text the field holds now.
#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    pub range: Range<usize>,
    pub diagnostic: Diagnostic,
    /// Who gave it, as [`Marks::publish`] names it.
    pub source: u8,
}

impl Mark {
    /// An error unless the diagnostic says otherwise, as LSP reads a diagnostic with no severity.
    pub fn severity(&self) -> DiagnosticSeverity {
        self.diagnostic.severity.unwrap_or(DiagnosticSeverity::ERROR)
    }

    /// Whether it lies under a position: a range over it, or a point at it or just before it.
    pub fn covers(&self, at: usize) -> bool {
        if self.range.is_empty() { at == self.range.start || at == self.range.start + 1 } else { self.range.contains(&at) }
    }
}

/// The marks of a field, at its revision `revision`.
#[derive(Clone, Debug, Default)]
pub struct Marks {
    marks: Vec<Mark>,
    revision: u64,
}

impl Marks {
    /// The diagnostics of `source`, given for `text`, which the field had at its revision `revision`, in place of those
    /// it gave before. Their positions are those of LSP, lines and characters ([`crate::ENCODING`]); one past the text is
    /// kept at its end. When the field no longer keeps the edits since that revision, they are dropped: a later set will
    /// come.
    pub fn publish(&mut self, field: &Field, source: u8, revision: u64, text: &str, diagnostics: Vec<Diagnostic>) {
        self.follow(field);
        self.marks.retain(|mark| mark.source != source);
        let Some(edits) = field.edits_since(revision) else { return };
        let lines = Lines::new(text);
        let mut placed: Vec<Mark> = diagnostics
            .into_iter()
            .map(|diagnostic| {
                let start = lines.offset(diagnostic.range.start);
                let end = lines.offset(diagnostic.range.end).max(start);
                Mark { range: start..end, diagnostic, source }
            })
            .collect();
        shift(&mut placed, edits);
        self.marks.extend(placed);
        self.marks.sort_by_key(|mark| (mark.range.start, mark.range.end));
    }

    /// Moves the marks with the edits made since the last call.
    pub fn follow(&mut self, field: &Field) {
        if self.revision == field.revision() {
            return;
        }
        match field.edits_since(self.revision) {
            Some(edits) => shift(&mut self.marks, edits),
            None => self.marks.clear(),
        }
        self.revision = field.revision();
    }

    /// Removes the marks of a source.
    pub fn clear(&mut self, source: u8) {
        self.marks.retain(|mark| mark.source != source);
    }

    /// In the order of the text.
    pub fn marks(&self) -> &[Mark] {
        &self.marks
    }

    /// Those under a position, errors first.
    pub fn at(&self, at: usize) -> Vec<&Mark> {
        let mut found: Vec<&Mark> = self.marks.iter().filter(|mark| mark.covers(at)).collect();
        found.sort_by_key(|mark| mark.severity());
        found
    }

    /// The revision of the field the marks stand at.
    pub fn revision(&self) -> u64 {
        self.revision
    }
}

/// Moves marks by edits, in their order; a mark whose text an edit removed goes.
fn shift<'a>(marks: &mut Vec<Mark>, edits: impl Iterator<Item = &'a Edit>) {
    for edit in edits {
        marks.retain_mut(|mark| match edit.range(mark.range.clone()) {
            Some(range) => {
                mark.range = range;
                true
            }
            None => false,
        });
    }
}
