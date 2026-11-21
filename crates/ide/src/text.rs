//! The `Text` view (ide.md 7; commands.md 9.1; decisions of the owner, 2026-11-21): the file of the function in the
//! monospace face, edited by hand. Its colours come from the lexer of `format`, so that they never disagree with
//! `check`. The text is written when it parses, after a pause of the keys or when the view loses the focus (`text.save`),
//! one step each; one that does not parse stays here, with a banner, and in the recovery folder, and the diagram of the
//! function cannot be edited until it parses again. The diagnostics underlined and completion come with step 7b.

use egui::{Color32, FontId, RichText};
use kit_text::{Colours, Field, Highlighter, Span};
use labcraft_format::{Lexer, TokenKind};
use labcraft_types::{Interner, Report};
use serde_json::{Value, json};

use crate::host::{Host, Reply};

/// At most this much text is read for the view *(proposed)*: past it, the view says the rest is cut.
pub const MAX_TEXT: usize = 16 * 1024 * 1024;
/// A text longer than this is shown and not written: what `text.save` takes (commands.md 6.7).
pub const MAX_EDIT: usize = labcraft_engine::text::MAX_SAVE;
/// The pause of the keys after which the text is written, in seconds (commands.md 9.1).
pub const PAUSE: f64 = 0.5;

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

/// Each class, in the order of their numbers in a [`Span`].
const CLASSES: [Class; 9] =
    [Class::Plain, Class::Keyword, Class::Name, Class::Number, Class::Text, Class::Comment, Class::Doc, Class::Punctuation, Class::Hole];

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

/// The highlighter of the language: a token never crosses a line (spec 14), so a line is lexed alone, from no state.
#[derive(Clone, Copy, Debug, Default)]
pub struct Lexed;

impl Highlighter for Lexed {
    type State = ();
    const STATELESS: bool = true;

    fn line(&mut self, _: &(), line: &str, spans: &mut Vec<Span>) {
        for (range, class) in classes(line) {
            if class != Class::Plain {
                let class = CLASSES.iter().position(|each| *each == class).and_then(|at| u16::try_from(at).ok()).unwrap_or(0);
                spans.push(Span { range, class });
            }
        }
    }
}

/// The look of the field in a theme.
pub fn style(dark: bool, select: Color32) -> kit_text_egui::Style {
    let text = colour(Class::Plain, dark);
    kit_text_egui::Style {
        font: FontId::new(13.0, egui::FontFamily::Monospace),
        text,
        classes: CLASSES.iter().map(|class| colour(*class, dark)).collect(),
        selection: select.gamma_multiply(0.35),
        cursor: text,
        current_line: text.gamma_multiply(0.05),
        gutter: colour(Class::Comment, dark),
    }
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

/// Where the text of the view stands against its file.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// Written, or not changed.
    Clean,
    /// It does not parse: the file keeps the last text that did, and the diagram cannot be edited.
    Unparsed { message: String, line: Option<u32> },
    /// The file changed while the text was being typed (decision of the owner, 2026-11-21).
    Stale,
    /// `text.save` failed.
    Failed(String),
}

/// What a banner of the view asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Asked {
    /// "Show the file": the text typed goes, kept in the recovery folder.
    ShowFile,
    /// "Keep my text": it is written over the file.
    KeepText,
    /// The text of the recovery folder comes back in the view.
    TakeBack,
    /// It is removed from the recovery folder.
    Discard,
}

/// The text of a file, as the view edits it.
pub struct Editing {
    pub file: String,
    pub field: Field,
    colours: Colours<Lexed>,
    /// The text of the file as the view last read or wrote it: the field had it at revision `field`.
    base: Option<Base>,
    reading: Option<Reading>,
    /// The check it was read for (`Published::checks`), so that a new check reads it again.
    checks: Option<u64>,
    saving: Option<Saving>,
    /// The revision of the field last sent to `text.save`.
    sent: Option<u64>,
    /// When the text last changed, and the revision of the field then.
    changed_at: f64,
    seen: u64,
    pub status: Status,
    /// The text of the file, read while the view held a text of its own.
    outside: Option<String>,
    /// The next read takes the file, whatever the view holds: "Show the file" with no text read yet.
    adopt: bool,
    /// The last text sent did not parse, and none was written since.
    unparsed: bool,
    /// The file is over [`MAX_EDIT`]: shown, not written.
    pub too_large: bool,
    /// The rest was not read: past [`MAX_TEXT`].
    pub cut: bool,
    pub failure: Option<String>,
    /// A text of the recovery folder newer than the file, offered back (architecture 12.6).
    pub recovered: Option<String>,
    /// The widget of the field, once drawn: whether it has the focus decides a write.
    pub field_id: Option<egui::Id>,
}

struct Base {
    text: String,
    revision: String,
    field: u64,
}

struct Reading {
    text: String,
    reply: Reply,
}

/// How a text is sent to `text.save`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Save {
    /// Against the revision of the file it was made from.
    Write,
    /// Over whatever the file holds: "Keep my text".
    Over,
    /// Against that revision, only to keep it in the recovery folder: the file changed under it.
    Keep,
}

struct Saving {
    reply: Reply,
    text: String,
    field: u64,
    /// Sent only to keep the text in the recovery folder: its answer changes nothing here.
    keep_only: bool,
}

impl Editing {
    pub fn new(file: &str) -> Editing {
        Editing {
            file: file.to_owned(),
            field: Field::default(),
            colours: Colours::new(Lexed),
            base: None,
            reading: None,
            checks: None,
            saving: None,
            sent: None,
            changed_at: 0.0,
            seen: 0,
            status: Status::Clean,
            outside: None,
            adopt: false,
            unparsed: false,
            too_large: false,
            cut: false,
            failure: None,
            recovered: None,
            field_id: None,
        }
    }

    pub fn loading(&self) -> bool {
        self.base.is_none()
    }

    /// The text differs from what the file was when the view read or wrote it.
    pub fn dirty(&self) -> bool {
        self.base.as_ref().is_some_and(|base| base.field != self.field.revision())
    }

    /// The diagram of the function cannot be edited: its text does not parse (commands.md 9.1).
    pub fn locks_diagram(&self) -> bool {
        matches!(self.status, Status::Unparsed { .. }) || (self.status == Status::Stale && self.unparsed)
    }

    /// Once a frame, shown or not: takes the replies, reads the file again after a check, writes after a pause.
    /// Whether it waits for something: a reply, or a pause to end.
    pub fn update(&mut self, host: &dyn Host, checks: u64, now: f64, focused: bool) -> bool {
        if self.field.revision() != self.seen {
            self.seen = self.field.revision();
            self.changed_at = now;
        }
        self.take_saved();
        self.take_read(host);
        if self.checks != Some(checks) && self.reading.is_none() && self.saving.is_none() {
            self.checks = Some(checks);
            self.reading = Some(Reading { text: String::new(), reply: host.send("file.read", json!({ "path": self.file })) });
        }
        let paused = now - self.changed_at >= PAUSE || !focused;
        let fresh = self.sent != Some(self.field.revision());
        if self.dirty() && fresh && paused && !self.too_large && self.status != Status::Stale && self.saving.is_none() && self.reading.is_none() {
            self.save(host, Save::Write);
        }
        // Under the banner of a file changed elsewhere, what is typed is kept in the recovery folder (decision of the owner,
        // 2026-11-21).
        if self.dirty() && fresh && paused && self.status == Status::Stale && self.saving.is_none() {
            self.save(host, Save::Keep);
        }
        self.reading.is_some() || self.saving.is_some() || (self.dirty() && fresh && !self.too_large)
    }

    fn save(&mut self, host: &dyn Host, how: Save) {
        let text = self.field.text();
        let mut params = json!({ "path": self.file, "text": text });
        if let (false, Some(base), Some(object)) = (how == Save::Over, &self.base, params.as_object_mut()) {
            object.insert("expected_revision".to_owned(), json!(base.revision));
        }
        if let (Save::Keep, Some(object)) = (how, params.as_object_mut()) {
            object.insert("keep".to_owned(), json!(true));
        }
        let field = self.field.revision();
        self.sent = Some(field);
        self.saving = Some(Saving { reply: host.send("text.save", params), text, field, keep_only: how == Save::Keep });
    }

    fn take_saved(&mut self) {
        let Some(result) = self.saving.as_ref().and_then(|saving| saving.reply.take()) else { return };
        let Some(saving) = self.saving.take() else { return };
        if saving.keep_only {
            return;
        }
        match result {
            Ok(saved) if saved.get("written").and_then(Value::as_bool) == Some(true) => {
                let revision = saved.get("revision").and_then(Value::as_str).unwrap_or_default().to_owned();
                self.base = Some(Base { text: saving.text, revision, field: saving.field });
                self.status = Status::Clean;
                self.unparsed = false;
                self.outside = None;
            }
            Ok(saved) if saved.get("stale").and_then(Value::as_bool) == Some(true) => self.status = Status::Stale,
            Ok(saved) => {
                let error = saved.get("error");
                let message = error.and_then(|error| error.get("message")).and_then(Value::as_str).unwrap_or("the text does not parse").to_owned();
                let line =
                    error.and_then(|error| error.pointer("/span/start/line")).and_then(Value::as_u64).and_then(|line| u32::try_from(line).ok());
                self.status = Status::Unparsed { message, line };
                self.unparsed = true;
            }
            Err(error) => self.status = Status::Failed(error.message),
        }
    }

    fn take_read(&mut self, host: &dyn Host) {
        let Some(result) = self.reading.as_ref().and_then(|reading| reading.reply.take()) else { return };
        let Some(mut reading) = self.reading.take() else { return };
        match result {
            Ok(part) => {
                reading.text.push_str(part.get("text").and_then(Value::as_str).unwrap_or_default());
                let truncated = part.get("truncated").and_then(Value::as_bool).unwrap_or(false);
                let from = part.get("from").and_then(Value::as_u64).unwrap_or(1);
                let lines = part.get("lines").and_then(Value::as_u64).unwrap_or(0);
                // A line longer than a part is cut there: the text is not whole, and is shown read only.
                let line_cut = truncated && !reading.text.ends_with('\n');
                if truncated && !line_cut && reading.text.len() < MAX_TEXT && lines > 0 {
                    reading.reply = host.send("file.read", json!({ "path": self.file, "from": from + lines }));
                    self.reading = Some(reading);
                } else {
                    self.cut = truncated;
                    self.failure = None;
                    // The revision of the bytes of the file, which `expected_revision` names, before CR LF is made LF.
                    let revision = labcraft_engine::revision(reading.text.as_bytes());
                    if self.read(reading.text.replace("\r\n", "\n"), revision) {
                        self.save(host, Save::Keep);
                    }
                }
            }
            Err(error) => self.failure = Some(error.message),
        }
    }

    /// The file as it is now. Whether the text of the view is to be kept in the recovery folder: the file changed under it.
    fn read(&mut self, text: String, revision: String) -> bool {
        // Once cut, the view never writes what it shows: it is not the file.
        self.too_large = self.too_large || self.cut || text.len() > MAX_EDIT;
        let adopt = std::mem::take(&mut self.adopt);
        match &self.base {
            None => {
                self.field.reset(&text);
                self.adopt_file(text, revision);
            }
            // "Show the file": the view takes it, whatever it holds.
            _ if adopt => {
                self.field.replace_all(&text);
                self.adopt_file(text, revision);
            }
            Some(base) if base.text == text => return false,
            // The view follows the file: one step of the undo of the field (commands.md 9.1).
            _ if !self.dirty() => {
                self.field.replace_all(&text);
                self.adopt_file(text, revision);
            }
            // The same text of the file as at the last read: kept already.
            _ if self.outside.as_deref() == Some(text.as_str()) => return false,
            // A text of its own: kept, the banner offers the file.
            _ => {
                self.outside = Some(text);
                self.status = Status::Stale;
                return true;
            }
        }
        false
    }

    fn adopt_file(&mut self, text: String, revision: String) {
        self.base = Some(Base { text, revision, field: self.field.revision() });
        self.sent = Some(self.field.revision());
        self.seen = self.field.revision();
        self.status = Status::Clean;
        self.outside = None;
    }

    /// What a banner asked for.
    pub fn answer(&mut self, host: &dyn Host, asked: Asked) {
        match asked {
            Asked::ShowFile => {
                // Kept in the recovery folder first, if typed since: `text.save` against the revision it was made from.
                if self.sent != Some(self.field.revision()) && self.saving.is_none() {
                    self.save(host, Save::Keep);
                }
                match self.outside.take() {
                    Some(text) => {
                        self.field.replace_all(&text);
                        let revision = labcraft_engine::revision(text.as_bytes());
                        self.adopt_file(text, revision);
                    }
                    None => {
                        self.adopt = true;
                        self.checks = None;
                    }
                }
            }
            Asked::KeepText => self.save(host, Save::Over),
            Asked::TakeBack => {
                if let Some(text) = self.recovered.take() {
                    self.field.replace_all(&text);
                }
            }
            Asked::Discard => {
                self.recovered = None;
                drop(host.send("text.discard", json!({ "path": self.file })));
            }
        }
    }

    /// The view: its banner, then the field. What a banner asked for.
    pub fn show(&mut self, ui: &mut egui::Ui, dark: bool, select: Color32, tints: &[(usize, Color32)]) -> Option<Asked> {
        let mut asked = None;
        if let Some(failure) = &self.failure {
            ui.label(RichText::new(failure).color(ui.visuals().warn_fg_color));
            return None;
        }
        if self.loading() {
            ui.label(RichText::new("Reading…").weak());
            return None;
        }
        let warn = ui.visuals().warn_fg_color;
        if self.recovered.is_some() {
            banner(
                ui,
                warn,
                "A text of this file that did not read was kept, newer than the file",
                &[("Take it back", Asked::TakeBack), ("Discard", Asked::Discard)],
                &mut asked,
            );
        }
        match &self.status {
            Status::Clean => {}
            Status::Unparsed { message, line } => {
                let place = line.map(|line| format!(" (line {line})")).unwrap_or_default();
                banner(
                    ui,
                    warn,
                    &format!("The text does not read{place}: {message}. The file keeps the last text that read; fix it here"),
                    &[],
                    &mut asked,
                );
            }
            // The buttons wait for the answer of a save in flight.
            Status::Stale if self.saving.is_some() => banner(ui, warn, "The file changed while you typed", &[], &mut asked),
            Status::Stale => {
                banner(
                    ui,
                    warn,
                    "The file changed while you typed",
                    &[("Show the file", Asked::ShowFile), ("Keep my text", Asked::KeepText)],
                    &mut asked,
                );
            }
            Status::Failed(message) => banner(ui, warn, &format!("The text was not written: {message}"), &[], &mut asked),
        }
        if self.too_large {
            let why = if self.cut {
                format!("The rest of the file is not shown: past {} MiB. Read only", MAX_TEXT / 1024 / 1024)
            } else {
                format!("This file is over {} MiB: the Text view shows it, read only", MAX_EDIT / 1024 / 1024)
            };
            ui.label(RichText::new(why).weak());
        }
        let style = style(dark, select);
        let label = format!("Text of {}", self.file);
        let shown = kit_text_egui::Shown { read_only: self.too_large, tints, reveal: None, label: &label };
        let response = kit_text_egui::show(ui, ("ide-text", &self.file), &mut self.field, &mut self.colours, &style, &shown);
        self.field_id = Some(response.id);
        asked
    }
}

fn banner(ui: &mut egui::Ui, colour: Color32, text: &str, buttons: &[(&str, Asked)], asked: &mut Option<Asked>) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(text).color(colour));
        for (label, answer) in buttons {
            if ui.button(*label).clicked() {
                *asked = Some(*answer);
            }
        }
    });
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

    /// A line is coloured alone, as the whole text colours it: no token crosses a line.
    #[test]
    fn a_line_alone_is_coloured_as_in_its_text() {
        let text = "fn f() -> () {\n    s = \"a\" // b\n}\n";
        let mut spans = Vec::new();
        Lexed.line(&(), "    s = \"a\" // b", &mut spans);
        let whole: Vec<(String, Class)> = classes_of(text)
            .into_iter()
            .filter(|(_, class)| *class != Class::Plain)
            .skip_while(|(part, _)| *part != "s")
            .take(spans.len())
            .map(|(part, class)| (part.to_owned(), class))
            .collect();
        let line = "    s = \"a\" // b";
        let alone: Vec<(String, Class)> =
            spans.iter().map(|span| (line.get(span.range.clone()).unwrap().to_owned(), CLASSES[usize::from(span.class)])).collect();
        assert_eq!(alone, whole);
    }
}
