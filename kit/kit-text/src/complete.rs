//! Completion (ide.md 7; decision of the owner, 2026-11-21): a list of what the language service says can be written,
//! opened from the first character of a name typed or by the key that asks for it, narrowed as the name grows, taken
//! with Enter or Tab. Only the state: the view draws it and gives it the keys.

use lsp_types::{CompletionItem, CompletionTextEdit};

use crate::field::{Action, Field};
use crate::service::{LanguageService, offset_of, position_of};

/// At most so many entries are kept *(proposed)*: typing narrows them.
pub const MAX_SHOWN: usize = 200;

/// The list, when it is open.
#[derive(Clone, Debug, Default)]
pub struct Completion {
    open: Option<Open>,
    /// Where a name typed started that the service had nothing for.
    declined: Option<usize>,
}

#[derive(Clone, Debug)]
struct Open {
    /// What the service gave, for the name from `start`.
    all: Vec<CompletionItem>,
    /// Those that the word typed keeps, by their index in `all`, best first.
    shown: Vec<usize>,
    chosen: usize,
    /// Where the name being completed starts.
    start: usize,
    /// Asked by its key: it stays open on an empty name.
    asked: bool,
}

/// Whether a character continues a name (spec 14.1: letters, marks, digits and `_`).
pub fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Where the name that ends at `at` starts, walking back over the characters of the field, with no copy of its line.
pub fn name_start(field: &Field, at: usize) -> usize {
    let mut start = at;
    while start > 0 && field.char_at(start - 1).is_some_and(is_name_char) {
        start -= 1;
    }
    start
}

impl Completion {
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// The entries shown, best first, and the one chosen.
    pub fn shown(&self) -> (Vec<&CompletionItem>, usize) {
        match &self.open {
            Some(open) => (open.shown.iter().filter_map(|&index| open.all.get(index)).collect(), open.chosen),
            None => (Vec::new(), 0),
        }
    }

    /// Where the name being completed starts: the list opens under it.
    pub fn start(&self) -> Option<usize> {
        self.open.as_ref().map(|open| open.start)
    }

    /// Asks the service at the cursor. `asked`: by its key, which opens the list on no name at all; typed, the list opens
    /// only on a name that starts with a letter or `_`, and stays closed when nothing starts as it does.
    pub fn open(&mut self, field: &Field, service: &mut dyn LanguageService, asked: bool) {
        self.open = None;
        let selection = field.selection();
        if !selection.is_empty() {
            return;
        }
        let at = selection.head;
        let start = name_start(field, at);
        if !asked && (self.declined == Some(start) || !field.char_at(start).is_some_and(|c| c.is_alphabetic() || c == '_')) {
            return;
        }
        let all = service.completion(field, position_of(field, at));
        let mut open = Open { all, shown: Vec::new(), chosen: 0, start, asked };
        narrow(&mut open, &field.slice(start..at));
        if open.shown.is_empty() {
            // Nothing for this name: the service is not asked again as it grows, until [`Completion::close`].
            self.declined = (!asked).then_some(start);
        } else {
            self.open = Some(open);
        }
    }

    /// After a key that changed the text or moved the cursor: narrows the list to the name typed, or closes it when the
    /// cursor left the name, or when nothing starts as it does.
    pub fn follow(&mut self, field: &Field) {
        let Some(open) = self.open.as_mut() else { return };
        let selection = field.selection();
        let at = selection.head;
        if !selection.is_empty() || at < open.start || name_start(field, at) != open.start {
            self.open = None;
            return;
        }
        let word = field.slice(open.start..at);
        if word.is_empty() && !open.asked {
            self.open = None;
            return;
        }
        let before = open.shown.get(open.chosen).copied();
        narrow(open, &word);
        open.chosen = before.and_then(|index| open.shown.iter().position(|&shown| shown == index)).unwrap_or(0);
        if open.shown.is_empty() {
            self.open = None;
        }
    }

    /// Closes the list; a name that had none may ask again.
    pub fn close(&mut self) {
        self.open = None;
        self.declined = None;
    }

    /// The next entry, or the one before; around the ends.
    pub fn choose(&mut self, by: isize) {
        let Some(open) = self.open.as_mut() else { return };
        let count = open.shown.len();
        if count == 0 {
            return;
        }
        let chosen = isize::try_from(open.chosen).unwrap_or(0) + by;
        open.chosen = usize::try_from(chosen.rem_euclid(isize::try_from(count).unwrap_or(1))).unwrap_or(0);
    }

    /// Chooses an entry by its place in the list shown.
    pub fn set_chosen(&mut self, chosen: usize) {
        if let Some(open) = self.open.as_mut()
            && chosen < open.shown.len()
        {
            open.chosen = chosen;
        }
    }

    /// Writes the entry chosen in place of the name typed, as one step of undo, and closes the list. Whether it wrote.
    pub fn take(&mut self, field: &mut Field) -> bool {
        let Some(open) = self.open.take() else { return false };
        let Some(item) = open.shown.get(open.chosen).and_then(|&index| open.all.get(index)) else { return false };
        let at = field.selection().head;
        let (range, text) = match &item.text_edit {
            Some(CompletionTextEdit::Edit(edit)) => (offset_of(field, edit.range.start)..offset_of(field, edit.range.end), edit.new_text.clone()),
            Some(CompletionTextEdit::InsertAndReplace(edit)) => {
                (offset_of(field, edit.replace.start)..offset_of(field, edit.replace.end), edit.new_text.clone())
            }
            None => (open.start..at, item.insert_text.clone().unwrap_or_else(|| item.label.clone())),
        };
        field.apply(Action::Replace { range, text });
        true
    }
}

/// Keeps the entries that start as the word does, its case aside, then, in a list asked for by its key, those that hold
/// it; in the order of their `sort_text` (their label without one), shorter first.
fn narrow(open: &mut Open, word: &str) {
    let word = word.to_lowercase();
    // A list opened by typing keeps what starts as the word: Enter never writes another word in its place.
    let asked = open.asked;
    let mut kept: Vec<(u8, usize)> = open
        .all
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let text = item.filter_text.as_deref().unwrap_or(&item.label).to_lowercase();
            if text.starts_with(&word) {
                Some((0, index))
            } else if asked && !word.is_empty() && text.contains(&word) {
                Some((1, index))
            } else {
                None
            }
        })
        .collect();
    let key = |index: usize| open.all.get(index).map(|item| (item.sort_text.clone().unwrap_or_default(), item.label.len(), item.label.clone()));
    kept.sort_by_key(|&(rank, index)| (rank, key(index)));
    open.shown = kept.into_iter().map(|(_, index)| index).take(MAX_SHOWN).collect();
    open.chosen = 0;
}
