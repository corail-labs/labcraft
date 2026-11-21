//! The text of a field, its one cursor and the undo of the field (ide.md 7). Positions are characters from the start of
//! the text; a line ends at LF only, as the lexer of the language reads it.

use std::ops::Range;

use ropey::Rope;

/// The width of one level of indentation, in spaces: the canonical form of the language (spec 5.2).
pub const INDENT: usize = 4;
/// The undo of the field keeps at most this many steps *(proposed)*...
pub const MAX_STEPS: usize = 1_000;
/// ...and at most this much text in them *(proposed)*: the oldest go first.
pub const MAX_UNDO_BYTES: usize = 64 << 20;
/// Typing goes on in a step of at most this much text *(proposed)*: a key never copies a large paste.
const MAX_MERGED: usize = 4 << 10;

/// The cursor, `head`, and the other end of the selection, `anchor`: the same when nothing is selected.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

impl Selection {
    pub fn cursor(at: usize) -> Selection {
        Selection { anchor: at, head: at }
    }

    pub fn range(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
}

/// Where a key moves the cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    WordLeft,
    WordRight,
    Up,
    Down,
    /// The first character that is not a space, or the start of the line when the cursor is there already.
    Home,
    End,
    /// So many lines up or down: those that the view shows.
    PageUp(usize),
    PageDown(usize),
    TextStart,
    TextEnd,
}

/// What a key or a gesture does to the field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Text typed, pasted or given by an input method: it replaces the selection.
    Insert(String),
    /// Enter: a new line with the indentation of the line of the cursor.
    Newline,
    /// Tab: spaces to the next level, or one level more on each line of a selection over several lines.
    Indent,
    /// Shift+Tab: one level less on each line of the selection.
    Outdent,
    Backspace,
    Delete,
    BackspaceWord,
    DeleteWord,
    Move {
        motion: Motion,
        extend: bool,
    },
    SelectAll,
    /// The selection, as a gesture of the mouse gives it.
    Select(Selection),
    Undo,
    Redo,
    /// The text selected, for the clipboard; nothing changes.
    Copy,
    /// The text selected, for the clipboard, removed.
    Cut,
}

/// Lines of the text replaced by an edit: the `removed` lines from `line` became the `inserted` lines from `line`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Splice {
    pub line: usize,
    pub removed: usize,
    pub inserted: usize,
}

/// One change: at `at`, `removed` became `inserted`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Change {
    at: usize,
    removed: String,
    inserted: String,
}

/// A step of the undo of the field.
#[derive(Clone, Debug)]
struct Step {
    change: Change,
    before: Selection,
    after: Selection,
}

impl Step {
    fn bytes(&self) -> usize {
        self.change.removed.len() + self.change.inserted.len()
    }
}

/// What the last step was, so that typing goes on in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Typing {
    Insert,
    Backspace,
    Delete,
}

/// The text of a field.
#[derive(Clone, Debug, Default)]
pub struct Field {
    rope: Rope,
    selection: Selection,
    /// The column that Up and Down keep, in characters.
    goal: Option<usize>,
    undo: Vec<Step>,
    redo: Vec<Step>,
    undo_bytes: usize,
    typing: Option<Typing>,
    revision: u64,
    splices: Vec<Splice>,
    /// The most characters a line has had since the text was given: the width of the view never shrinks while one types.
    widest: usize,
}

impl Field {
    pub fn new(text: &str) -> Field {
        let mut field = Field::default();
        field.reset(text);
        field
    }

    /// A new text, with no undo: the text that a file holds when the field opens it.
    pub fn reset(&mut self, text: &str) {
        let old_lines = self.rope.len_lines();
        self.rope = Rope::from_str(text);
        self.selection = Selection::default();
        self.goal = None;
        self.undo.clear();
        self.redo.clear();
        self.undo_bytes = 0;
        self.typing = None;
        self.revision += 1;
        self.splices.push(Splice { line: 0, removed: old_lines, inserted: self.rope.len_lines() });
        self.widest = self.rope.lines().map(|line| line_len(&line)).max().unwrap_or(0);
    }

    /// Another text in place of this one, as one step of undo: what changed outside the field (a file written by another
    /// view or by an agent). Only the part that differs is replaced, so the cursor stays where it was when it is outside it.
    pub fn replace_all(&mut self, text: &str) {
        // The common start and end, walked on the rope and the text: no copy of either.
        let old_len = self.rope.len_chars();
        let new_len = text.chars().count();
        let prefix = self.rope.chars().zip(text.chars()).take_while(|(a, b)| a == b).count();
        if prefix == old_len && prefix == new_len {
            return;
        }
        let room = old_len.min(new_len) - prefix;
        let suffix = self.rope.chars_at(old_len).reversed().zip(text.chars().rev()).take(room).take_while(|(a, b)| a == b).count();
        let removed = self.rope.get_slice(prefix..old_len - suffix).map(|slice| slice.to_string()).unwrap_or_default();
        let inserted: String = text.chars().skip(prefix).take(new_len - prefix - suffix).collect();
        let before = self.selection;
        let change = Change { at: prefix, removed, inserted };
        let map = |at: usize| shifted(at, &change);
        let after = Selection { anchor: map(before.anchor), head: map(before.head) };
        self.apply_change(&change);
        self.selection = after;
        self.typing = None;
        self.push(Step { change, before, after }, None);
    }

    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    pub fn len_lines(&self) -> usize {
        self.rope.len_lines()
    }

    /// The text of a line, without its line ending; empty past the last line.
    pub fn line(&self, line: usize) -> String {
        let Some(slice) = self.rope.get_line(line) else { return String::new() };
        let mut text = slice.to_string();
        if text.ends_with('\n') {
            text.pop();
        }
        text
    }

    /// How many characters a line has, its line ending not counted.
    pub fn line_len(&self, line: usize) -> usize {
        self.rope.get_line(line).map_or(0, |slice| line_len(&slice))
    }

    pub fn widest(&self) -> usize {
        self.widest
    }

    pub fn selection(&self) -> Selection {
        self.selection
    }

    pub fn selected(&self) -> String {
        let range = self.selection.range();
        self.rope.get_slice(range).map(|slice| slice.to_string()).unwrap_or_default()
    }

    /// Changes with each change of the text.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The lines that changed since the last call, for whoever keeps something by line (the colours).
    pub fn take_splices(&mut self) -> Vec<Splice> {
        std::mem::take(&mut self.splices)
    }

    /// The line and the column, in characters, of a position.
    pub fn line_column(&self, at: usize) -> (usize, usize) {
        let at = at.min(self.rope.len_chars());
        let line = self.rope.try_char_to_line(at).unwrap_or(0);
        let start = self.rope.try_line_to_char(line).unwrap_or(0);
        (line, at - start)
    }

    /// The position of a line and a column, each kept within the text.
    pub fn position(&self, line: usize, column: usize) -> usize {
        let line = line.min(self.rope.len_lines().saturating_sub(1));
        let start = self.rope.try_line_to_char(line).unwrap_or(0);
        start + column.min(self.line_len(line))
    }

    /// The word around a position (a name, a number, a run of spaces or of the other characters): what a double click
    /// selects.
    pub fn word_at(&self, at: usize) -> Selection {
        let (line, column) = self.line_column(at);
        let chars: Vec<char> = self.line(line).chars().collect();
        let start = self.position(line, 0);
        let Some(&here) = chars.get(column).or_else(|| column.checked_sub(1).and_then(|before| chars.get(before))) else {
            return Selection::cursor(at);
        };
        let class = class_of(here);
        let column = if chars.get(column).is_some_and(|&c| class_of(c) == class) { column } else { column.saturating_sub(1) };
        let mut from = column;
        while from > 0 && chars.get(from - 1).is_some_and(|&c| class_of(c) == class) {
            from -= 1;
        }
        let mut to = column;
        while chars.get(to).is_some_and(|&c| class_of(c) == class) {
            to += 1;
        }
        Selection { anchor: start + from, head: start + to }
    }

    /// Does what the action says; the text for the clipboard, for `Copy` and `Cut`.
    pub fn apply(&mut self, action: Action) -> Option<String> {
        let typing = self.typing.take();
        if !matches!(action, Action::Move { motion: Motion::Up | Motion::Down | Motion::PageUp(_) | Motion::PageDown(_), .. }) {
            self.goal = None;
        }
        match action {
            Action::Insert(text) => {
                let text = text.replace("\r\n", "\n");
                let continues = typing == Some(Typing::Insert) && self.selection.is_empty() && !starts_a_word(&text, self.char_before());
                self.replace_selection(&text, Some(Typing::Insert), continues);
            }
            Action::Newline => {
                let (line, _) = self.line_column(self.selection.range().start);
                let indent: String = self.line(line).chars().take_while(|&c| c == ' ' || c == '\t').collect();
                self.replace_selection(&format!("\n{indent}"), None, false);
            }
            Action::Indent => self.indent(typing),
            Action::Outdent => self.outdent(),
            Action::Backspace => {
                let at = self.selection.head;
                let from = if at > 0 { self.backspace_from(at) } else { at };
                self.delete(from..at, Some(Typing::Backspace), typing);
            }
            Action::Delete => {
                let at = self.selection.head;
                self.delete(at..(at + 1).min(self.rope.len_chars()), Some(Typing::Delete), typing);
            }
            Action::BackspaceWord => {
                let at = self.selection.head;
                let from = self.moved(at, Motion::WordLeft);
                self.delete(from..at, None, typing);
            }
            Action::DeleteWord => {
                let at = self.selection.head;
                let to = self.moved(at, Motion::WordRight);
                self.delete(at..to, None, typing);
            }
            Action::Move { motion, extend } => {
                let head = if !extend && !self.selection.is_empty() && matches!(motion, Motion::Left | Motion::Right) {
                    // An arrow ends a selection at its side, as in every text field.
                    let range = self.selection.range();
                    if motion == Motion::Left { range.start } else { range.end }
                } else {
                    self.moved(self.selection.head, motion)
                };
                self.selection = if extend { Selection { anchor: self.selection.anchor, head } } else { Selection::cursor(head) };
            }
            Action::SelectAll => self.selection = Selection { anchor: 0, head: self.rope.len_chars() },
            Action::Select(selection) => {
                let len = self.rope.len_chars();
                self.selection = Selection { anchor: selection.anchor.min(len), head: selection.head.min(len) };
            }
            Action::Undo => self.undo(),
            Action::Redo => self.redo(),
            Action::Copy => return Some(self.selected()).filter(|text| !text.is_empty()),
            Action::Cut => {
                let text = self.selected();
                if text.is_empty() {
                    return None;
                }
                self.replace_selection("", None, false);
                return Some(text);
            }
        }
        None
    }

    fn char_before(&self) -> Option<char> {
        self.selection.head.checked_sub(1).and_then(|at| self.rope.get_char(at))
    }

    /// Backspace in the indentation of a line takes a level of it, as Tab gave it.
    fn backspace_from(&self, at: usize) -> usize {
        let (line, column) = self.line_column(at);
        let before: String = self.line(line).chars().take(column).collect();
        if column > 0 && before.chars().all(|c| c == ' ') {
            let level = (column - 1) / INDENT * INDENT;
            return at - (column - level);
        }
        at - 1
    }

    fn indent(&mut self, typing: Option<Typing>) {
        let range = self.selection.range();
        let (first, column) = self.line_column(range.start);
        let (last, _) = self.line_column(range.end);
        if first == last {
            let spaces = INDENT - column % INDENT;
            self.replace_selection(&" ".repeat(spaces), Some(Typing::Insert), typing == Some(Typing::Insert));
            return;
        }
        // An empty line stays empty: no spaces at the end of a line.
        self.each_line(first, last, |line| (!line.is_empty()).then(|| format!("{}{line}", " ".repeat(INDENT))));
    }

    fn outdent(&mut self) {
        let range = self.selection.range();
        let (first, _) = self.line_column(range.start);
        let (last, _) = self.line_column(range.end);
        self.each_line(first, last, |line| {
            let spaces = line.chars().take(INDENT).take_while(|&c| c == ' ').count();
            (spaces > 0).then(|| line.chars().skip(spaces).collect())
        });
    }

    /// Rewrites the lines `first..=last` as one step; the selection covers them after. A line that `rewrite` leaves
    /// (`None`) stays as it is.
    fn each_line(&mut self, first: usize, last: usize, rewrite: impl Fn(&str) -> Option<String>) {
        let start = self.position(first, 0);
        let end = self.position(last, usize::MAX);
        let mut changed = false;
        let lines: Vec<String> = (first..=last)
            .map(|line| {
                let text = self.line(line);
                match rewrite(&text) {
                    Some(new) => {
                        changed = true;
                        new
                    }
                    None => text,
                }
            })
            .collect();
        if !changed {
            return;
        }
        let text = lines.join("\n");
        let before = self.selection;
        let removed = self.rope.get_slice(start..end).map(|slice| slice.to_string()).unwrap_or_default();
        let change = Change { at: start, removed, inserted: text };
        let after = Selection { anchor: start, head: start + change.inserted.chars().count() };
        self.apply_change(&change);
        self.selection = after;
        self.push(Step { change, before, after }, None);
    }

    /// A key that deletes: the selection when there is one, else `range`, as part of the step of the same keys before it.
    fn delete(&mut self, range: Range<usize>, kind: Option<Typing>, typing: Option<Typing>) {
        if !self.selection.is_empty() {
            self.replace_selection("", None, false);
        } else if !range.is_empty() {
            self.replace(range, "", kind, kind.is_some() && typing == kind);
        }
    }

    fn replace_selection(&mut self, text: &str, typing: Option<Typing>, continues: bool) {
        let range = self.selection.range();
        self.replace(range, text, typing, continues);
    }

    /// Replaces a range with a text, the cursor after it, as a step of undo or as part of the step of the typing it goes
    /// on with.
    fn replace(&mut self, range: Range<usize>, text: &str, typing: Option<Typing>, continues: bool) {
        let len = self.rope.len_chars();
        let range = range.start.min(len)..range.end.min(len);
        let removed = self.rope.get_slice(range.clone()).map(|slice| slice.to_string()).unwrap_or_default();
        if removed.is_empty() && text.is_empty() {
            return;
        }
        let before = self.selection;
        let change = Change { at: range.start, removed, inserted: text.to_owned() };
        let after = Selection::cursor(range.start + text.chars().count());
        self.apply_change(&change);
        self.selection = after;
        self.push(Step { change, before, after }, typing.filter(|_| continues));
        self.typing = typing;
    }

    /// Applies a change to the text and says which lines it changed.
    fn apply_change(&mut self, change: &Change) {
        let line = self.rope.try_char_to_line(change.at).unwrap_or(0);
        let removed_chars = change.removed.chars().count();
        let _ = self.rope.try_remove(change.at..change.at + removed_chars);
        let _ = self.rope.try_insert(change.at, &change.inserted);
        let removed = change.removed.matches('\n').count() + 1;
        let inserted = change.inserted.matches('\n').count() + 1;
        self.splices.push(Splice { line, removed, inserted });
        for line in line..line + inserted {
            self.widest = self.widest.max(self.line_len(line));
        }
        self.revision += 1;
    }

    /// Keeps a step: merged into the last one when typing goes on, the redo gone.
    fn push(&mut self, step: Step, merge: Option<Typing>) {
        self.redo.clear();
        if let (Some(typing), Some(last)) = (merge, self.undo.last_mut())
            && last.bytes() < MAX_MERGED
            && merge_into(&mut last.change, &step.change, typing)
        {
            self.undo_bytes += step.bytes();
            last.after = step.after;
            return;
        }
        self.undo_bytes += step.bytes();
        self.undo.push(step);
        while self.undo.len() > MAX_STEPS || (self.undo_bytes > MAX_UNDO_BYTES && self.undo.len() > 1) {
            let oldest = self.undo.remove(0);
            self.undo_bytes -= oldest.bytes();
        }
    }

    fn undo(&mut self) {
        let Some(step) = self.undo.pop() else { return };
        self.undo_bytes -= step.bytes();
        let inverse = Change { at: step.change.at, removed: step.change.inserted.clone(), inserted: step.change.removed.clone() };
        self.apply_change(&inverse);
        self.selection = step.before;
        self.redo.push(step);
    }

    fn redo(&mut self) {
        let Some(step) = self.redo.pop() else { return };
        self.apply_change(&step.change);
        self.selection = step.after;
        self.undo_bytes += step.bytes();
        self.undo.push(step);
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Where a motion takes the cursor from `at`.
    fn moved(&mut self, at: usize, motion: Motion) -> usize {
        let len = self.rope.len_chars();
        let (line, column) = self.line_column(at);
        match motion {
            Motion::Left => at.saturating_sub(1),
            Motion::Right => (at + 1).min(len),
            Motion::WordLeft => {
                let mut at = at;
                while at > 0 && self.rope.get_char(at - 1).is_some_and(|c| c == ' ' || c == '\t') {
                    at -= 1;
                }
                let Some(class) = at.checked_sub(1).and_then(|before| self.rope.get_char(before)).map(class_of) else { return at };
                while at > 0 && self.rope.get_char(at - 1).is_some_and(|c| class_of(c) == class && c != '\n') {
                    at -= 1;
                }
                if at > 0 && self.rope.get_char(at - 1) == Some('\n') && class == Class::Line {
                    at -= 1;
                }
                at
            }
            Motion::WordRight => {
                let mut at = at;
                while self.rope.get_char(at).is_some_and(|c| c == ' ' || c == '\t') {
                    at += 1;
                }
                let Some(class) = self.rope.get_char(at).map(class_of) else { return at };
                if class == Class::Line {
                    return at + 1;
                }
                while self.rope.get_char(at).is_some_and(|c| class_of(c) == class) {
                    at += 1;
                }
                at
            }
            Motion::Up | Motion::Down | Motion::PageUp(_) | Motion::PageDown(_) => {
                let goal = *self.goal.get_or_insert(column);
                let target = match motion {
                    Motion::Up => line.checked_sub(1),
                    Motion::Down => Some(line + 1).filter(|&next| next < self.rope.len_lines()),
                    Motion::PageUp(lines) => Some(line.saturating_sub(lines.max(1))),
                    Motion::PageDown(lines) => Some(line.saturating_add(lines.max(1)).min(self.rope.len_lines().saturating_sub(1))),
                    _ => None,
                };
                match target {
                    Some(target) => self.position(target, goal),
                    // Up on the first line goes to its start, Down on the last to its end.
                    None if motion == Motion::Up => 0,
                    None => len,
                }
            }
            Motion::Home => {
                let first = self.line(line).chars().take_while(|&c| c == ' ' || c == '\t').count();
                let start = self.position(line, 0);
                if column == first { start } else { start + first }
            }
            Motion::End => self.position(line, usize::MAX),
            Motion::TextStart => 0,
            Motion::TextEnd => len,
        }
    }
}

/// What a word is made of, for the motions by word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Name,
    Space,
    Line,
    Other,
}

fn class_of(c: char) -> Class {
    if c.is_alphanumeric() || c == '_' {
        Class::Name
    } else if c == '\n' {
        Class::Line
    } else if c.is_whitespace() {
        Class::Space
    } else {
        Class::Other
    }
}

/// A character typed after another of another kind starts a new step of undo: a space after a word, a word after a
/// space, a line.
fn starts_a_word(text: &str, before: Option<char>) -> bool {
    let mut chars = text.chars();
    let (Some(first), None) = (chars.next(), chars.next()) else { return true };
    match before {
        Some(before) => class_of(first) != class_of(before) && class_of(first) != Class::Other,
        None => false,
    }
}

/// The step of typing that goes on, in place: an insert after the last one, a Backspace before it, a Delete at it.
/// Whether it went on.
fn merge_into(last: &mut Change, next: &Change, typing: Typing) -> bool {
    match typing {
        Typing::Insert if next.removed.is_empty() && next.at == last.at + last.inserted.chars().count() => {
            last.inserted.push_str(&next.inserted);
            true
        }
        Typing::Backspace if last.inserted.is_empty() && next.inserted.is_empty() && next.at + next.removed.chars().count() == last.at => {
            last.removed.insert_str(0, &next.removed);
            last.at = next.at;
            true
        }
        Typing::Delete if last.inserted.is_empty() && next.inserted.is_empty() && next.at == last.at => {
            last.removed.push_str(&next.removed);
            true
        }
        _ => false,
    }
}

/// Where a position goes when a change is made: after the change when it was in what it replaced.
fn shifted(at: usize, change: &Change) -> usize {
    let removed = change.removed.chars().count();
    let inserted = change.inserted.chars().count();
    if at < change.at {
        at
    } else if at >= change.at + removed {
        at - removed + inserted
    } else {
        change.at + inserted
    }
}

fn line_len(slice: &ropey::RopeSlice<'_>) -> usize {
    let len = slice.len_chars();
    if len > 0 && slice.get_char(len - 1) == Some('\n') { len - 1 } else { len }
}
