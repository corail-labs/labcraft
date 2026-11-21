//! The keys of a text field (notation 2.6), as the field reads them: each expected action written by hand.

use egui::{Event, Key, Modifiers};
use kit_text::{Action, Motion};

use super::action_of;

fn key(key: Key, modifiers: Modifiers) -> Option<Action> {
    action_of(&Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }, 10)
}

/// Tab and Shift+Tab indent and outdent: the field keeps them, they move no focus.
#[test]
fn tab_indents_and_shift_tab_outdents() {
    assert_eq!(key(Key::Tab, Modifiers::NONE), Some(Action::Indent));
    assert_eq!(key(Key::Tab, Modifiers::SHIFT), Some(Action::Outdent));
}

/// Ctrl+Z undoes in the field, Ctrl+Shift+Z redoes; Ctrl+A selects the whole text.
#[test]
fn the_keys_of_undo_and_of_the_selection_act_in_the_field() {
    assert_eq!(key(Key::Z, Modifiers::COMMAND), Some(Action::Undo));
    assert_eq!(key(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT), Some(Action::Redo));
    assert_eq!(key(Key::A, Modifiers::COMMAND), Some(Action::SelectAll));
}

/// Shift extends the selection; Page Down moves by the lines in view; a key released does nothing.
#[test]
fn shift_extends_and_a_page_is_the_lines_in_view() {
    assert_eq!(key(Key::ArrowDown, Modifiers::SHIFT), Some(Action::Move { motion: Motion::Down, extend: true }));
    assert_eq!(key(Key::PageDown, Modifiers::NONE), Some(Action::Move { motion: Motion::PageDown(10), extend: false }));
    let released = Event::Key { key: Key::A, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE };
    assert_eq!(action_of(&released, 10), None);
}

/// Text typed is inserted, a line ending in it too; another control character is not.
#[test]
fn text_is_inserted_but_a_control_character() {
    assert_eq!(action_of(&Event::Text("é ✓".to_owned()), 10), Some(Action::Insert("é ✓".to_owned())));
    assert_eq!(action_of(&Event::Text("a\nb".to_owned()), 10), Some(Action::Insert("a\nb".to_owned())));
    assert_eq!(action_of(&Event::Text("\u{7}".to_owned()), 10), None);
}
