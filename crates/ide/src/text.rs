//! The `Text` view of 1a (ide.md 7; decisions of the owner, 2026-11-16): the file of the function in the monospace
//! face, read only, coloured by the lexer of `format`, so that its colours never disagree with `check`. The
//! diagnostics underlined, completion and editing come with `kit-text` (step 7).

use std::sync::Arc;

use egui::Galley;
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId};
use labcraft_format::{Lexer, TokenKind};
use labcraft_types::{Interner, Report};

use serde_json::{Value, json};

use crate::host::{Host, Reply};

/// At most this much text is read for the view *(proposed)*: past it, the view says the rest is cut.
pub const MAX_TEXT: usize = 16 * 1024 * 1024;

/// What a token is, for its colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Plain,
    Keyword,
    Name,
    Number,
    Text,
    Comment,
    Doc,
    Punctuation,
    /// A hole, `?` (spec 5.1).
    Hole,
}

/// The colours of the classes *(proposed)*, light and dark.
pub fn colour(class: Class, dark: bool) -> Color32 {
    let (light, night) = match class {
        Class::Plain => (0x1F2328, 0xE6E8EB),
        Class::Keyword => (0x8250DF, 0xC5A3FF),
        Class::Name => (0x1F2328, 0xE6E8EB),
        Class::Number => (0x0550AE, 0x79C0FF),
        Class::Text => (0x0A6E31, 0x7EE2A8),
        Class::Comment => (0x6E7781, 0x8B949E),
        Class::Doc => (0x57606A, 0xA5AFBA),
        Class::Punctuation => (0x57606A, 0xA5AFBA),
        Class::Hole => (0xBC4C00, 0xFFA657),
    };
    let rgb: u32 = if dark { night } else { light };
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

fn class_of(kind: &TokenKind) -> Class {
    match kind {
        TokenKind::Keyword(_) => Class::Keyword,
        TokenKind::Name(_) | TokenKind::DegreeName(_) => Class::Name,
        TokenKind::Int(_) | TokenKind::Float(_) => Class::Number,
        TokenKind::String(_) => Class::Text,
        TokenKind::Comment(_) => Class::Comment,
        TokenKind::Doc(_) => Class::Doc,
        TokenKind::Newline => Class::Plain,
        TokenKind::Question => Class::Hole,
        _ => Class::Punctuation,
    }
}

/// The classes of the text, as byte ranges in order, every byte covered: what follows a lexical error is plain.
pub fn classes(text: &str) -> Vec<(std::ops::Range<usize>, Class)> {
    let mut cursor = Cursor::new(text);
    let mut names = Interner::new();
    let mut report = Report::new(1);
    let mut ranges = Vec::new();
    let mut at = 0;
    for token in Lexer::new(text, &mut names, &mut report) {
        let Ok(token) = token else { break };
        let Some(start) = cursor.offset(token.span.start.line, token.span.start.column) else { break };
        let Some(end) = cursor.offset(token.span.end.line, token.span.end.column) else { break };
        if start < at || end < start {
            break;
        }
        if start > at {
            ranges.push((at..start, Class::Plain));
        }
        if end > start {
            ranges.push((start..end, class_of(&token.kind)));
        }
        at = end;
    }
    if at < text.len() {
        ranges.push((at..text.len(), Class::Plain));
    }
    ranges
}

/// Turns the positions of the lexer (line and column from 1, in characters) into byte offsets, walking the text forward:
/// the tokens come in order, so a whole text costs one walk, even on one long line.
struct Cursor<'a> {
    text: &'a str,
    /// The byte at which each line starts.
    starts: Vec<usize>,
    /// Where the walk is: a line, a column in it, and its byte.
    line: u32,
    column: u32,
    byte: usize,
}

impl<'a> Cursor<'a> {
    fn new(text: &'a str) -> Cursor<'a> {
        let mut starts = vec![0usize];
        starts.extend(text.char_indices().filter(|(_, c)| *c == '\n').map(|(at, _)| at + 1));
        Cursor { text, starts, line: 1, column: 1, byte: 0 }
    }

    /// The byte of the `column`-th character counted from the start of `line`; the end of the text past it.
    fn offset(&mut self, line: u32, column: u32) -> Option<usize> {
        if line != self.line || column < self.column {
            self.byte = *self.starts.get(usize::try_from(line).ok()?.checked_sub(1)?)?;
            self.line = line;
            self.column = 1;
        }
        while self.column < column {
            let Some(c) = self.text.get(self.byte..).and_then(|rest| rest.chars().next()) else {
                return Some(self.text.len());
            };
            self.byte += c.len_utf8();
            self.column += 1;
        }
        Some(self.byte)
    }
}

/// The text laid out with its colours.
pub fn job(text: &str, dark: bool, font: &FontId) -> LayoutJob {
    let mut job = LayoutJob::default();
    for (range, class) in classes(text) {
        if let Some(part) = text.get(range) {
            job.append(part, 0.0, TextFormat::simple(font.clone(), colour(class, dark)));
        }
    }
    job
}

/// The text of a file, as the view keeps it.
pub struct Kept {
    pub file: String,
    pub text: String,
    /// The rest was not read: past [`MAX_TEXT`].
    pub cut: bool,
    /// The check it was read for (`Published::checks`), so that a new check reads it again.
    checks: Option<u64>,
    /// The next part, asked for.
    asked: Option<Reply>,
    pub failure: Option<String>,
    /// The text laid out, for the theme and the scale it was made for: made once, not on each frame.
    galley: Option<(bool, u32, Arc<Galley>)>,
}

impl Kept {
    pub fn new(file: &str) -> Kept {
        Kept { file: file.to_owned(), text: String::new(), cut: false, checks: None, asked: None, failure: None, galley: None }
    }

    /// Reads the file again when the session has checked again since; takes the parts that came.
    pub fn refresh(&mut self, host: &dyn Host, checks: u64) {
        if self.checks != Some(checks) && self.asked.is_none() {
            self.checks = Some(checks);
            self.text.clear();
            self.cut = false;
            self.failure = None;
            self.galley = None;
            self.asked = Some(host.send("file.read", json!({ "path": self.file })));
        }
        let Some(result) = self.asked.as_ref().and_then(Reply::take) else { return };
        self.asked = None;
        match result {
            Ok(part) => {
                self.text.push_str(part.get("text").and_then(Value::as_str).unwrap_or_default());
                self.galley = None;
                let truncated = part.get("truncated").and_then(Value::as_bool).unwrap_or(false);
                let from = part.get("from").and_then(Value::as_u64).unwrap_or(1);
                let lines = part.get("lines").and_then(Value::as_u64).unwrap_or(0);
                if truncated && self.text.len() < MAX_TEXT && lines > 0 {
                    self.asked = Some(host.send("file.read", json!({ "path": self.file, "from": from + lines })));
                } else {
                    self.cut = truncated;
                }
            }
            Err(error) => self.failure = Some(error.message),
        }
    }

    pub fn loading(&self) -> bool {
        self.asked.is_some()
    }

    /// The text laid out in its colours, unwrapped, made once per text, theme and scale.
    pub fn galley(&mut self, ui: &egui::Ui, dark: bool, font: &FontId) -> Arc<Galley> {
        let scale = ui.ctx().pixels_per_point().to_bits();
        match &self.galley {
            Some((made_for, made_at, galley)) if *made_for == dark && *made_at == scale => galley.clone(),
            _ => {
                let mut job = self::job(&self.text, dark, font);
                job.wrap.max_width = f32::INFINITY;
                let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
                self.galley = Some((dark, scale, galley.clone()));
                galley
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classes_of(text: &str) -> Vec<(&str, Class)> {
        classes(text).into_iter().map(|(range, class)| (text.get(range).unwrap(), class)).collect()
    }

    #[test]
    fn each_token_takes_its_class_and_every_byte_is_covered() {
        let text = "/// Doubles.\nfn double(x: int) -> int {\n  y = core.mul(x, 2) // twice\n}\n";
        let found = classes_of(text);
        assert_eq!(found.iter().map(|(part, _)| *part).collect::<String>(), text);
        assert!(found.contains(&("fn", Class::Keyword)), "{found:?}");
        assert!(found.contains(&("double", Class::Name)), "{found:?}");
        assert!(found.contains(&("2", Class::Number)), "{found:?}");
        assert!(found.iter().any(|(part, class)| *class == Class::Comment && part.contains("twice")), "{found:?}");
        assert!(found.iter().any(|(part, class)| *class == Class::Doc && part.contains("Doubles")), "{found:?}");
    }

    #[test]
    fn what_follows_a_lexical_error_is_plain() {
        let text = "fn main() -> () {\n  x = \"no end\n}\n";
        let found = classes_of(text);
        assert_eq!(found.iter().map(|(part, _)| *part).collect::<String>(), text);
        assert_eq!(found.last().map(|(_, class)| *class), Some(Class::Plain));
    }

    #[test]
    fn a_long_line_is_coloured_in_one_walk() {
        // 200,000 tokens on one line: a walk from the start of the line for each would take minutes.
        let line = "a, ".repeat(100_000);
        let text = format!("fn f() -> () {{ x = core.max({line}b) }}\n");
        let found = classes(&text);
        assert_eq!(found.iter().filter_map(|(range, _)| text.get(range.clone())).collect::<String>(), text);
        assert!(found.iter().any(|(range, class)| *class == Class::Name && text.get(range.clone()) == Some("b")));
    }

    #[test]
    fn a_text_beyond_ascii_is_cut_on_characters() {
        let text = "fn é() -> () { s = \"°C ✓\" }\n";
        let found = classes_of(text);
        assert_eq!(found.iter().map(|(part, _)| *part).collect::<String>(), text);
        assert!(found.iter().any(|(part, class)| *class == Class::Text && part.contains('✓')), "{found:?}");
    }
}
