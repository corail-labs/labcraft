//! The language service of a field (ide.md 7): what the language says of the text, in the types of LSP (`lsp-types`),
//! so that another IDE gives its own and a server of the protocol could stand behind one. Positions are lines and
//! characters, both from 0, a character being a code point ([`ENCODING`], as the field counts them).

use lsp_types::{CompletionItem, Diagnostic, GotoDefinitionResponse, Hover, Position, PositionEncodingKind};

use crate::field::Field;

/// How a [`Position`] counts the characters of a line: code points, as the field does.
pub const ENCODING: PositionEncodingKind = PositionEncodingKind::UTF32;

/// New diagnostics of a source, given for `text`, which the field had at its revision `revision`.
#[derive(Clone, Debug, PartialEq)]
pub struct Published {
    pub source: u8,
    pub revision: u64,
    pub text: std::sync::Arc<str>,
    pub diagnostics: Vec<Diagnostic>,
}

/// What the language says of the text of a field. The view asks it on the gestures of the user; each answer is for the
/// text the field holds then.
pub trait LanguageService {
    /// The diagnostics given since the last call, each set in place of the one of its source before it.
    fn diagnostics(&mut self) -> Vec<Published> {
        Vec::new()
    }

    /// What can be written at `at`, which ends the word typed: the field keeps those that start as it does.
    fn completion(&mut self, field: &Field, at: Position) -> Vec<CompletionItem>;

    /// What a hover over `at` shows, besides the diagnostics there.
    fn hover(&mut self, field: &Field, at: Position) -> Option<Hover>;

    /// Where what is at `at` is defined.
    fn definition(&mut self, field: &Field, at: Position) -> Option<GotoDefinitionResponse>;
}

/// A service that says nothing: a field of plain text.
#[derive(Clone, Copy, Debug, Default)]
pub struct Silent;

impl LanguageService for Silent {
    fn completion(&mut self, _: &Field, _: Position) -> Vec<CompletionItem> {
        Vec::new()
    }

    fn hover(&mut self, _: &Field, _: Position) -> Option<Hover> {
        None
    }

    fn definition(&mut self, _: &Field, _: Position) -> Option<GotoDefinitionResponse> {
        None
    }
}

/// The position of a character of the field.
pub fn position_of(field: &Field, at: usize) -> Position {
    let (line, column) = field.line_column(at);
    Position { line: u32::try_from(line).unwrap_or(u32::MAX), character: u32::try_from(column).unwrap_or(u32::MAX) }
}

/// The character of the field at a position, kept within its line and the text.
pub fn offset_of(field: &Field, position: Position) -> usize {
    let line = usize::try_from(position.line).unwrap_or(usize::MAX);
    if line >= field.len_lines() {
        return field.len_chars();
    }
    field.position(line, usize::try_from(position.character).unwrap_or(usize::MAX))
}

/// The character of `text` at a position, kept within its line and the text, lines ending at LF.
pub fn offset_in(text: &str, position: Position) -> usize {
    Lines::new(text).offset(position)
}

/// Where each line of a text starts, in characters: many positions of one text placed in one walk of it.
#[derive(Clone, Debug)]
pub struct Lines {
    /// The character at which each line starts, then the length of the text plus one: where a line after the last
    /// would start.
    starts: Vec<usize>,
}

impl Lines {
    pub fn new(text: &str) -> Lines {
        let mut starts = vec![0];
        let mut count = 0;
        for c in text.chars() {
            count += 1;
            if c == '\n' {
                starts.push(count);
            }
        }
        starts.push(count + 1);
        Lines { starts }
    }

    /// The character at a position, kept within its line and the text.
    pub fn offset(&self, position: Position) -> usize {
        let line = usize::try_from(position.line).unwrap_or(usize::MAX);
        let end_of_text = self.starts.last().map_or(0, |end| end.saturating_sub(1));
        let (Some(&start), Some(&next)) = (self.starts.get(line), self.starts.get(line.saturating_add(1))) else {
            return end_of_text;
        };
        // The next line starts after the line ending; the last "line" ends at the text.
        let length = next.saturating_sub(start + 1);
        start + length.min(usize::try_from(position.character).unwrap_or(usize::MAX))
    }
}
