//! The colours of the lines of a field (ide.md 7): a [`Highlighter`] that the IDE gives, from the lexer of its language,
//! and [`Colours`], which keeps what it gave line by line, so that a key colours again only the lines it changed, and
//! those after it whose state at their start changed.

use std::ops::Range;

use crate::field::Splice;

/// A part of a line and what it is: `class` is the highlighter's, which the view turns into a colour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    /// Bytes of the line.
    pub range: Range<usize>,
    pub class: u16,
}

/// Colours a line from the state that the lines before it leave. A language whose tokens never cross a line has a state
/// of `()`.
pub trait Highlighter {
    type State: Clone + PartialEq + Default;

    /// Each line starts from the same state: a line far down is coloured alone, without the lines before it.
    const STATELESS: bool = false;

    /// The spans of `line`, which has no line ending, in order; what they do not cover is plain. The state after it.
    fn line(&mut self, state: &Self::State, line: &str, spans: &mut Vec<Span>) -> Self::State;
}

#[derive(Clone, Debug)]
struct Line<S> {
    start: S,
    spans: Vec<Span>,
    end: S,
}

/// The spans of each line, kept.
#[derive(Clone, Debug)]
pub struct Colours<H: Highlighter> {
    highlighter: H,
    lines: Vec<Option<Line<H::State>>>,
    /// The lines before this one are coloured from the right state.
    valid: usize,
}

impl<H: Highlighter> Colours<H> {
    pub fn new(highlighter: H) -> Colours<H> {
        Colours { highlighter, lines: Vec::new(), valid: 0 }
    }

    /// Follows the edits of the text: the lines they replaced are coloured again when asked for.
    pub fn splice(&mut self, splices: &[Splice]) {
        for splice in splices {
            let start = splice.line.min(self.lines.len());
            let end = (splice.line + splice.removed).min(self.lines.len());
            self.lines.splice(start..end, std::iter::repeat_with(|| None).take(splice.inserted));
            self.valid = self.valid.min(splice.line);
        }
    }

    /// The spans of a line; `text` gives the text of a line.
    pub fn spans(&mut self, line: usize, lines: usize, text: impl Fn(usize) -> String) -> &[Span] {
        if self.lines.len() < lines {
            self.lines.resize_with(lines, || None);
        }
        self.lines.truncate(lines);
        let line = line.min(lines.saturating_sub(1));
        if H::STATELESS {
            if let Some(slot) = self.lines.get_mut(line)
                && slot.is_none()
            {
                let mut spans = Vec::new();
                let end = self.highlighter.line(&H::State::default(), &text(line), &mut spans);
                *slot = Some(Line { start: H::State::default(), spans, end });
            }
            return match self.lines.get(line) {
                Some(Some(kept)) => &kept.spans,
                _ => &[],
            };
        }
        while self.valid <= line {
            let index = self.valid;
            let start = match index.checked_sub(1).and_then(|before| self.lines.get(before)) {
                Some(Some(before)) => before.end.clone(),
                _ => H::State::default(),
            };
            let kept = matches!(self.lines.get(index), Some(Some(kept)) if kept.start == start);
            if !kept {
                let mut spans = Vec::new();
                let end = self.highlighter.line(&start, &text(index), &mut spans);
                if let Some(slot) = self.lines.get_mut(index) {
                    *slot = Some(Line { start, spans, end });
                }
            }
            self.valid += 1;
        }
        match self.lines.get(line) {
            Some(Some(kept)) => &kept.spans,
            _ => &[],
        }
    }

    pub fn highlighter(&self) -> &H {
        &self.highlighter
    }
}
