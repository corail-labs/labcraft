//! Tests of `kit-text`: the field (text, cursor, actions, undo, splices) and the colours of its lines. Each expected
//! value is worked out by hand from the doc comments of `field.rs` and `colours.rs`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing, clippy::string_slice)]

use crate::colours::{Colours, Highlighter, Span};
use crate::field::{Action, Field, Motion, Selection, Splice};

// ---- helpers ----

fn field_at(text: &str, at: usize) -> Field {
    let mut field = Field::new(text);
    field.apply(Action::Select(Selection::cursor(at)));
    field
}

fn select(field: &mut Field, anchor: usize, head: usize) {
    field.apply(Action::Select(Selection { anchor, head }));
}

fn type_chars(field: &mut Field, text: &str) {
    for c in text.chars() {
        field.apply(Action::Insert(c.to_string()));
    }
}

fn go(field: &mut Field, motion: Motion) {
    field.apply(Action::Move { motion, extend: false });
}

fn extend(field: &mut Field, motion: Motion) {
    field.apply(Action::Move { motion, extend: true });
}

fn head(field: &Field) -> usize {
    field.selection().head
}

fn sel(anchor: usize, head: usize) -> Selection {
    Selection { anchor, head }
}

// ---- reading the text ----

/// A new field gives its lines without line endings, and an empty line past the last one.
#[test]
fn a_new_field_gives_its_lines_without_line_endings() {
    let field = Field::new("ab\ncd");
    assert_eq!(field.len_lines(), 2);
    assert_eq!(field.len_chars(), 5);
    assert_eq!(field.line(0), "ab");
    assert_eq!(field.line(1), "cd");
    assert_eq!(field.line(2), "");
    assert_eq!(field.line_len(0), 2);
    assert_eq!(field.line_len(1), 2);
    assert_eq!(field.line_len(9), 0);
    assert_eq!(field.text(), "ab\ncd");
}

/// A text that ends with a line ending has a last, empty line.
#[test]
fn a_trailing_line_ending_gives_an_empty_last_line() {
    let field = Field::new("ab\n");
    assert_eq!(field.len_lines(), 2);
    assert_eq!(field.line(0), "ab");
    assert_eq!(field.line(1), "");
    assert_eq!(field.line_len(1), 0);
    assert_eq!(Field::new("").len_lines(), 1);
}

/// `line_column` gives the line and the column in characters, and a position past the text is kept within it.
#[test]
fn line_column_counts_characters_and_clamps() {
    let field = Field::new("ab\ncd");
    assert_eq!(field.line_column(0), (0, 0));
    assert_eq!(field.line_column(2), (0, 2));
    assert_eq!(field.line_column(3), (1, 0));
    assert_eq!(field.line_column(5), (1, 2));
    assert_eq!(field.line_column(99), (1, 2));
    assert_eq!(Field::new("ab\n").line_column(3), (1, 0));
}

/// `position` keeps the line and the column within the text.
#[test]
fn position_clamps_the_line_and_the_column() {
    let field = Field::new("ab\ncd");
    assert_eq!(field.position(0, 0), 0);
    assert_eq!(field.position(0, 99), 2);
    assert_eq!(field.position(1, 1), 4);
    assert_eq!(field.position(9, 0), 3);
    assert_eq!(field.position(9, 99), 5);
}

/// Positions, columns and lengths are in characters, not bytes: e, a check mark and an emoji count one each.
#[test]
fn non_ascii_text_is_counted_in_characters() {
    let field = Field::new("é✓😀\nx");
    assert_eq!(field.len_chars(), 5);
    assert_eq!(field.line(0), "é✓😀");
    assert_eq!(field.line_len(0), 3);
    assert_eq!(field.line_column(3), (0, 3));
    assert_eq!(field.line_column(4), (1, 0));
    assert_eq!(field.position(0, 99), 3);
    assert_eq!(field.position(1, 0), 4);
    assert_eq!(field.widest(), 3);
}

/// Backspace over a character takes one character, whatever its size in bytes.
#[test]
fn backspace_removes_one_non_ascii_character() {
    let mut field = field_at("é✓😀", 3);
    field.apply(Action::Backspace);
    assert_eq!(field.text(), "é✓");
    assert_eq!(head(&field), 2);
    field.apply(Action::Backspace);
    assert_eq!(field.text(), "é");
}

/// `Select` keeps both ends within the text, and `SelectAll` selects the whole text.
#[test]
fn select_is_clamped_and_select_all_covers_the_text() {
    let mut field = Field::new("abc");
    field.apply(Action::Select(sel(99, 2)));
    assert_eq!(field.selection(), sel(3, 2));
    field.apply(Action::SelectAll);
    assert_eq!(field.selection(), sel(0, 3));
    assert_eq!(field.selected(), "abc");
}

/// The revision changes with the text, not with the cursor.
#[test]
fn the_revision_changes_with_the_text_only() {
    let mut field = Field::new("abc");
    let first = field.revision();
    go(&mut field, Motion::Right);
    assert_eq!(field.revision(), first);
    field.apply(Action::Insert("x".to_string()));
    assert_ne!(field.revision(), first);
}

// ---- insert, newline, indent, delete ----

/// Insert replaces the selection and leaves the cursor after the text.
#[test]
fn insert_replaces_the_selection() {
    let mut field = Field::new("hello world");
    select(&mut field, 0, 5);
    field.apply(Action::Insert("bye".to_string()));
    assert_eq!(field.text(), "bye world");
    assert_eq!(field.selection(), Selection::cursor(3));
}

/// A line ending of the inserted text is LF only: CRLF becomes LF.
#[test]
fn insert_turns_crlf_into_lf() {
    let mut field = Field::new("");
    field.apply(Action::Insert("a\r\nb".to_string()));
    assert_eq!(field.text(), "a\nb");
    assert_eq!(field.len_lines(), 2);
}

/// Enter makes a new line with the indentation of the line of the cursor.
#[test]
fn newline_keeps_the_indentation_of_the_line() {
    let mut field = field_at("    foo", 7);
    field.apply(Action::Newline);
    assert_eq!(field.text(), "    foo\n    ");
    assert_eq!(head(&field), 12);
}

/// Enter in the middle of a line carries the rest of the line to the new one, indented.
#[test]
fn newline_in_the_middle_of_a_line_splits_it() {
    let mut field = field_at("  ab", 3);
    field.apply(Action::Newline);
    assert_eq!(field.text(), "  a\n  b");
    assert_eq!(field.selection(), Selection::cursor(6));
}

/// Enter replaces the selection, and a tab of the indentation is kept as it is.
#[test]
fn newline_replaces_the_selection_and_keeps_a_tab() {
    let mut field = Field::new("\tfoo bar");
    select(&mut field, 4, 8);
    field.apply(Action::Newline);
    assert_eq!(field.text(), "\tfoo\n\t");
    assert_eq!(head(&field), 6);
}

/// Tab with nothing selected inserts spaces up to the next multiple of 4.
#[test]
fn indent_with_a_cursor_goes_to_the_next_level() {
    let mut field = Field::new("");
    field.apply(Action::Indent);
    assert_eq!(field.text(), "    ");
    let mut field = field_at("ab", 2);
    field.apply(Action::Indent);
    assert_eq!(field.text(), "ab  ");
    assert_eq!(head(&field), 4);
    let mut field = field_at("abcd", 4);
    field.apply(Action::Indent);
    assert_eq!(field.text(), "abcd    ");
    let mut field = field_at("abcd", 1);
    field.apply(Action::Indent);
    assert_eq!(field.text(), "a   bcd");
}

/// Tab over several lines puts 4 spaces at the start of each, and the selection covers the lines after, as one step.
#[test]
fn indent_over_lines_adds_a_level_to_each_and_selects_them() {
    let mut field = Field::new("a\nb\nc");
    select(&mut field, 0, 3);
    field.apply(Action::Indent);
    assert_eq!(field.text(), "    a\n    b\nc");
    assert_eq!(field.selection(), sel(0, 11));
    field.apply(Action::Undo);
    assert_eq!(field.text(), "a\nb\nc");
    assert_eq!(field.selection(), sel(0, 3));
    assert!(!field.can_undo());
}

/// Shift+Tab takes one level (up to 4 spaces) from each line of the selection, and the selection covers the lines after.
#[test]
fn outdent_takes_a_level_from_each_line() {
    let mut field = Field::new("        a\n  b\nc");
    field.apply(Action::SelectAll);
    field.apply(Action::Outdent);
    assert_eq!(field.text(), "    a\nb\nc");
    assert_eq!(field.selection(), sel(0, 9));
}

/// Shift+Tab on a cursor outdents the line of the cursor.
#[test]
fn outdent_works_on_the_line_of_a_cursor() {
    let mut field = field_at("      x", 7);
    field.apply(Action::Outdent);
    assert_eq!(field.text(), "  x");
    assert_eq!(field.selection(), sel(0, 3));
}

/// Shift+Tab with nothing to take does not change the text and keeps no step.
#[test]
fn outdent_with_nothing_to_take_does_nothing() {
    let mut field = Field::new("a\nb");
    field.apply(Action::SelectAll);
    field.apply(Action::Outdent);
    assert_eq!(field.text(), "a\nb");
    assert!(!field.can_undo());
    assert_eq!(field.selection(), sel(0, 3));
}

/// Backspace in the indentation takes the spaces back to the previous level.
#[test]
fn backspace_in_the_indentation_takes_a_level() {
    let mut field = field_at("        x", 8);
    field.apply(Action::Backspace);
    assert_eq!(field.text(), "    x");
    assert_eq!(head(&field), 4);
    field.apply(Action::Backspace);
    assert_eq!(field.text(), "x");
    assert_eq!(head(&field), 0);
}

/// Backspace in the indentation, between two levels, takes only up to the level before.
#[test]
fn backspace_between_two_levels_stops_at_the_level_before() {
    let mut field = field_at("      x", 6);
    field.apply(Action::Backspace);
    assert_eq!(field.text(), "    x");
    let mut field = field_at("  x", 2);
    field.apply(Action::Backspace);
    assert_eq!(field.text(), "x");
}

/// The indentation rule holds on any line, not only the first.
#[test]
fn backspace_in_the_indentation_of_a_later_line() {
    let mut field = field_at("a\n    b", 6);
    field.apply(Action::Backspace);
    assert_eq!(field.text(), "a\nb");
}

/// Backspace after text takes one character, even when spaces follow the cursor's left side.
#[test]
fn backspace_after_text_takes_one_character() {
    let mut field = field_at("ab    ", 6);
    field.apply(Action::Backspace);
    assert_eq!(field.text(), "ab   ");
}

/// Backspace at the start of a line joins it to the line before.
#[test]
fn backspace_at_a_line_start_joins_the_lines() {
    let mut field = field_at("a\nb", 2);
    field.apply(Action::Backspace);
    assert_eq!(field.text(), "ab");
    assert_eq!(head(&field), 1);
}

/// Backspace at the start of the text, and Delete at its end, do nothing and keep no step.
#[test]
fn backspace_and_delete_at_the_ends_do_nothing() {
    let mut field = Field::new("abc");
    field.apply(Action::Backspace);
    assert_eq!(field.text(), "abc");
    go(&mut field, Motion::TextEnd);
    field.apply(Action::Delete);
    assert_eq!(field.text(), "abc");
    assert!(!field.can_undo());
    let mut empty = Field::new("");
    empty.apply(Action::Backspace);
    empty.apply(Action::Delete);
    empty.apply(Action::BackspaceWord);
    empty.apply(Action::DeleteWord);
    assert!(!empty.can_undo());
}

/// Delete takes the character after the cursor, a line ending too.
#[test]
fn delete_takes_the_next_character() {
    let mut field = field_at("a\nb", 1);
    field.apply(Action::Delete);
    assert_eq!(field.text(), "ab");
    assert_eq!(head(&field), 1);
}

/// Backspace and Delete over a selection remove it.
#[test]
fn backspace_and_delete_remove_a_selection() {
    let mut field = Field::new("hello world");
    select(&mut field, 5, 11);
    field.apply(Action::Backspace);
    assert_eq!(field.text(), "hello");
    assert_eq!(field.selection(), Selection::cursor(5));
    let mut field = Field::new("hello world");
    select(&mut field, 11, 5);
    field.apply(Action::Delete);
    assert_eq!(field.text(), "hello");
}

/// Ctrl+Backspace removes the word before the cursor, then the spaces and the word before that.
#[test]
fn backspace_word_removes_the_word_before() {
    let mut field = field_at("foo bar", 7);
    field.apply(Action::BackspaceWord);
    assert_eq!(field.text(), "foo ");
    field.apply(Action::BackspaceWord);
    assert_eq!(field.text(), "");
}

/// Ctrl+Delete removes the word after the cursor; the spaces before the next one go with it.
#[test]
fn delete_word_removes_the_word_after() {
    let mut field = field_at("foo bar", 0);
    field.apply(Action::DeleteWord);
    assert_eq!(field.text(), " bar");
    field.apply(Action::DeleteWord);
    assert_eq!(field.text(), "");
}

/// Ctrl+Delete at the end of a line takes the line ending; over a selection the two remove the selection.
#[test]
fn delete_word_at_a_line_end_takes_the_line_ending() {
    let mut field = field_at("a\nb", 1);
    field.apply(Action::DeleteWord);
    assert_eq!(field.text(), "ab");
    let mut field = Field::new("foo bar");
    select(&mut field, 2, 5);
    field.apply(Action::BackspaceWord);
    assert_eq!(field.text(), "foar");
    let mut field = Field::new("foo bar");
    select(&mut field, 2, 5);
    field.apply(Action::DeleteWord);
    assert_eq!(field.text(), "foar");
}

// ---- motions ----

/// An arrow ends a selection at its side without extend, and moves one character otherwise.
#[test]
fn left_and_right_end_a_selection_at_its_side() {
    let mut field = Field::new("hello");
    select(&mut field, 1, 4);
    go(&mut field, Motion::Left);
    assert_eq!(field.selection(), Selection::cursor(1));
    select(&mut field, 1, 4);
    go(&mut field, Motion::Right);
    assert_eq!(field.selection(), Selection::cursor(4));
    select(&mut field, 4, 1);
    go(&mut field, Motion::Right);
    assert_eq!(field.selection(), Selection::cursor(4));
    select(&mut field, 4, 1);
    go(&mut field, Motion::Left);
    assert_eq!(field.selection(), Selection::cursor(1));
}

/// Left and Right move one character and stay within the text.
#[test]
fn left_and_right_stay_within_the_text() {
    let mut field = Field::new("ab");
    go(&mut field, Motion::Left);
    assert_eq!(head(&field), 0);
    go(&mut field, Motion::Right);
    go(&mut field, Motion::Right);
    go(&mut field, Motion::Right);
    assert_eq!(head(&field), 2);
}

/// With extend, the anchor stays and the head moves, even over a selection.
#[test]
fn extend_keeps_the_anchor() {
    let mut field = field_at("hello", 2);
    extend(&mut field, Motion::Right);
    assert_eq!(field.selection(), sel(2, 3));
    extend(&mut field, Motion::Left);
    extend(&mut field, Motion::Left);
    assert_eq!(field.selection(), sel(2, 1));
    select(&mut field, 1, 4);
    extend(&mut field, Motion::Left);
    assert_eq!(field.selection(), sel(1, 3));
    let mut field = field_at("abc\ndef", 1);
    extend(&mut field, Motion::Down);
    assert_eq!(field.selection(), sel(1, 5));
    extend(&mut field, Motion::TextEnd);
    assert_eq!(field.selection(), sel(1, 7));
}

/// Up and Down keep the column the cursor had at the start, across a line that is shorter.
#[test]
fn up_and_down_keep_the_goal_column() {
    let mut field = field_at("abcdef\nab\nabcdef", 5);
    go(&mut field, Motion::Down);
    assert_eq!(head(&field), 9);
    go(&mut field, Motion::Down);
    assert_eq!(head(&field), 15);
    go(&mut field, Motion::Up);
    assert_eq!(head(&field), 9);
    go(&mut field, Motion::Up);
    assert_eq!(head(&field), 5);
}

/// Another motion forgets the goal column.
#[test]
fn another_motion_forgets_the_goal_column() {
    let mut field = field_at("abcdef\nab\nabcdef", 5);
    go(&mut field, Motion::Down);
    assert_eq!(head(&field), 9);
    go(&mut field, Motion::Left);
    assert_eq!(head(&field), 8);
    go(&mut field, Motion::Down);
    assert_eq!(head(&field), 11);
}

/// Up on the first line goes to the start of the text, Down on the last line to its end.
#[test]
fn up_on_the_first_line_goes_to_the_start_and_down_on_the_last_to_the_end() {
    let mut field = field_at("abc\ndef", 2);
    go(&mut field, Motion::Up);
    assert_eq!(head(&field), 0);
    let mut field = field_at("abc\ndef", 5);
    go(&mut field, Motion::Down);
    assert_eq!(head(&field), 7);
}

/// Home goes to the first character that is not a space, then to the start of the line, and back.
#[test]
fn home_toggles_between_the_first_character_and_the_start() {
    let mut field = field_at("    foo bar", 9);
    go(&mut field, Motion::Home);
    assert_eq!(head(&field), 4);
    go(&mut field, Motion::Home);
    assert_eq!(head(&field), 0);
    go(&mut field, Motion::Home);
    assert_eq!(head(&field), 4);
}

/// Home on a later line counts from the start of that line; with no indentation it goes to the start.
#[test]
fn home_works_on_the_line_of_the_cursor() {
    let mut field = field_at("x\n  ab", 5);
    go(&mut field, Motion::Home);
    assert_eq!(head(&field), 4);
    let mut field = field_at("abc", 2);
    go(&mut field, Motion::Home);
    assert_eq!(head(&field), 0);
}

/// End goes to the end of the line, before its line ending.
#[test]
fn end_goes_to_the_end_of_the_line() {
    let mut field = field_at("ab\ncd", 0);
    go(&mut field, Motion::End);
    assert_eq!(head(&field), 2);
    go(&mut field, Motion::Down);
    go(&mut field, Motion::End);
    assert_eq!(head(&field), 5);
}

const WORDS: &str = "foo  bar_1 + x\nnext";

/// Ctrl+Right goes to the end of a name, over spaces first; a run of punctuation is a word; a line end is one stop.
#[test]
fn word_right_goes_over_names_spaces_punctuation_and_line_ends() {
    let mut field = field_at(WORDS, 0);
    let mut stops = Vec::new();
    for _ in 0..7 {
        go(&mut field, Motion::WordRight);
        stops.push(head(&field));
    }
    // "foo", "  bar_1", " +", " x", the line ending, "next", then the end of the text.
    assert_eq!(stops, [3, 10, 12, 14, 15, 19, 19]);
}

/// Ctrl+Left is the mirror of Ctrl+Right: to the start of a name, over spaces first, and the line ending is one stop.
#[test]
fn word_left_goes_over_names_spaces_punctuation_and_line_ends() {
    let mut field = field_at(WORDS, 19);
    let mut stops = Vec::new();
    for _ in 0..8 {
        go(&mut field, Motion::WordLeft);
        stops.push(head(&field));
    }
    assert_eq!(stops, [15, 14, 13, 11, 5, 0, 0, 0]);
}

/// PageDown and PageUp move by the lines of the view, within the text, keeping the goal column.
#[test]
fn page_motions_move_by_lines() {
    let mut field = field_at("a\nb\nc\nd\ne", 0);
    go(&mut field, Motion::PageDown(2));
    assert_eq!(head(&field), 4);
    go(&mut field, Motion::PageDown(10));
    assert_eq!(head(&field), 8);
    go(&mut field, Motion::PageUp(2));
    assert_eq!(head(&field), 4);
    go(&mut field, Motion::PageUp(10));
    assert_eq!(head(&field), 0);
    let mut field = field_at("abcd\nx\nabcd\ny\nabcd", 3);
    go(&mut field, Motion::PageDown(2));
    assert_eq!(head(&field), 10);
    go(&mut field, Motion::PageUp(2));
    assert_eq!(head(&field), 3);
}

/// A page of 0 lines moves by one line.
#[test]
fn a_page_of_no_lines_moves_one_line() {
    let mut field = field_at("a\nb\nc", 0);
    go(&mut field, Motion::PageDown(0));
    assert_eq!(head(&field), 2);
    go(&mut field, Motion::PageUp(0));
    assert_eq!(head(&field), 0);
}

/// Ctrl+Home and Ctrl+End go to the two ends of the text.
#[test]
fn text_start_and_text_end_go_to_the_ends() {
    let mut field = field_at("ab\ncd", 3);
    go(&mut field, Motion::TextEnd);
    assert_eq!(head(&field), 5);
    go(&mut field, Motion::TextStart);
    assert_eq!(head(&field), 0);
}

// ---- undo and redo ----

/// Typing a word character by character is one step of undo, and undo and redo restore the selection.
#[test]
fn typing_a_word_is_one_step_of_undo() {
    let mut field = Field::new("");
    type_chars(&mut field, "hello");
    assert_eq!(field.text(), "hello");
    assert!(field.can_undo());
    assert!(!field.can_redo());
    field.apply(Action::Undo);
    assert_eq!(field.text(), "");
    assert!(!field.can_undo());
    assert!(field.can_redo());
    assert_eq!(field.selection(), Selection::cursor(0));
    field.apply(Action::Redo);
    assert_eq!(field.text(), "hello");
    assert_eq!(field.selection(), Selection::cursor(5));
    assert!(!field.can_redo());
}

/// A space after a word, and a word after a space, start a new step: "ab cd" takes three undos.
#[test]
fn typing_is_split_at_the_word_boundaries() {
    let mut field = Field::new("");
    type_chars(&mut field, "ab cd");
    field.apply(Action::Undo);
    assert_eq!(field.text(), "ab ");
    field.apply(Action::Undo);
    assert_eq!(field.text(), "ab");
    field.apply(Action::Undo);
    assert_eq!(field.text(), "");
    assert!(!field.can_undo());
}

/// A name after punctuation starts a new step, punctuation after a name does not.
#[test]
fn punctuation_goes_on_the_step_of_the_word_before_it() {
    let mut field = Field::new("");
    type_chars(&mut field, "a+b");
    field.apply(Action::Undo);
    assert_eq!(field.text(), "a+");
    field.apply(Action::Undo);
    assert_eq!(field.text(), "");
}

/// A text pasted (more than one character) is a step of its own.
#[test]
fn a_paste_is_a_step_of_its_own() {
    let mut field = Field::new("");
    field.apply(Action::Insert("ab".to_string()));
    field.apply(Action::Insert("cd".to_string()));
    field.apply(Action::Undo);
    assert_eq!(field.text(), "ab");
    field.apply(Action::Undo);
    assert_eq!(field.text(), "");
}

/// Enter ends the typing step: what follows is another step.
#[test]
fn a_newline_ends_the_typing_step() {
    let mut field = Field::new("");
    type_chars(&mut field, "ab");
    field.apply(Action::Newline);
    type_chars(&mut field, "cd");
    field.apply(Action::Undo);
    assert_eq!(field.text(), "ab\n");
    field.apply(Action::Undo);
    assert_eq!(field.text(), "ab");
    field.apply(Action::Undo);
    assert_eq!(field.text(), "");
}

/// Consecutive Backspaces are one step; undo restores the text and the selection from before the first.
#[test]
fn consecutive_backspaces_are_one_step() {
    let mut field = field_at("abcd", 4);
    field.apply(Action::Backspace);
    field.apply(Action::Backspace);
    field.apply(Action::Backspace);
    assert_eq!(field.text(), "a");
    field.apply(Action::Undo);
    assert_eq!(field.text(), "abcd");
    assert_eq!(field.selection(), Selection::cursor(4));
    assert!(!field.can_undo());
    field.apply(Action::Redo);
    assert_eq!(field.text(), "a");
    assert_eq!(field.selection(), Selection::cursor(1));
}

/// Consecutive Deletes are one step too.
#[test]
fn consecutive_deletes_are_one_step() {
    let mut field = field_at("abcd", 0);
    field.apply(Action::Delete);
    field.apply(Action::Delete);
    assert_eq!(field.text(), "cd");
    field.apply(Action::Undo);
    assert_eq!(field.text(), "abcd");
    assert_eq!(field.selection(), Selection::cursor(0));
    assert!(!field.can_undo());
}

/// A Backspace then a Delete are two steps.
#[test]
fn a_backspace_then_a_delete_are_two_steps() {
    let mut field = field_at("abcd", 2);
    field.apply(Action::Backspace);
    field.apply(Action::Delete);
    assert_eq!(field.text(), "ad");
    field.apply(Action::Undo);
    assert_eq!(field.text(), "acd");
    field.apply(Action::Undo);
    assert_eq!(field.text(), "abcd");
}

/// A motion between two Backspaces breaks the merge.
#[test]
fn a_move_breaks_the_merge() {
    let mut field = field_at("abcd", 4);
    field.apply(Action::Backspace);
    go(&mut field, Motion::Left);
    go(&mut field, Motion::Right);
    field.apply(Action::Backspace);
    assert_eq!(field.text(), "ab");
    field.apply(Action::Undo);
    assert_eq!(field.text(), "abc");
    field.apply(Action::Undo);
    assert_eq!(field.text(), "abcd");
}

/// A motion between two typed characters breaks the merge as well.
#[test]
fn a_move_breaks_the_typing_step() {
    let mut field = Field::new("");
    type_chars(&mut field, "a");
    go(&mut field, Motion::Left);
    go(&mut field, Motion::Right);
    type_chars(&mut field, "b");
    assert_eq!(field.text(), "ab");
    field.apply(Action::Undo);
    assert_eq!(field.text(), "a");
    field.apply(Action::Undo);
    assert_eq!(field.text(), "");
}

/// A new edit clears the redo.
#[test]
fn a_new_edit_clears_the_redo() {
    let mut field = Field::new("");
    type_chars(&mut field, "a");
    field.apply(Action::Undo);
    assert!(field.can_redo());
    type_chars(&mut field, "b");
    assert!(!field.can_redo());
    field.apply(Action::Redo);
    assert_eq!(field.text(), "b");
}

/// Undo restores the selection from before the edit, redo the cursor after it.
#[test]
fn undo_restores_the_selection_before_and_redo_the_one_after() {
    let mut field = Field::new("hello world");
    select(&mut field, 0, 5);
    field.apply(Action::Insert("X".to_string()));
    assert_eq!(field.text(), "X world");
    field.apply(Action::Undo);
    assert_eq!(field.text(), "hello world");
    assert_eq!(field.selection(), sel(0, 5));
    field.apply(Action::Redo);
    assert_eq!(field.text(), "X world");
    assert_eq!(field.selection(), Selection::cursor(1));
}

/// Undo and redo with nothing to do change nothing.
#[test]
fn undo_and_redo_with_no_step_do_nothing() {
    let mut field = Field::new("abc");
    field.apply(Action::Undo);
    field.apply(Action::Redo);
    assert_eq!(field.text(), "abc");
    assert!(!field.can_undo());
    assert!(!field.can_redo());
}

/// Undo goes back through several steps and redo forward through them again.
#[test]
fn undo_and_redo_go_through_the_steps() {
    let mut field = Field::new("");
    field.apply(Action::Insert("one".to_string()));
    field.apply(Action::Insert(" two".to_string()));
    field.apply(Action::Undo);
    field.apply(Action::Undo);
    assert_eq!(field.text(), "");
    field.apply(Action::Redo);
    assert_eq!(field.text(), "one");
    field.apply(Action::Redo);
    assert_eq!(field.text(), "one two");
    assert!(!field.can_redo());
}

// ---- replace_all ----

/// An outside change is one step of undo, which restores the old text.
#[test]
fn replace_all_is_one_step_of_undo() {
    let mut field = Field::new("hello world");
    field.replace_all("hello there");
    assert_eq!(field.text(), "hello there");
    field.apply(Action::Undo);
    assert_eq!(field.text(), "hello world");
    assert!(!field.can_undo());
    field.apply(Action::Redo);
    assert_eq!(field.text(), "hello there");
}

/// The cursor stays where it was when the change is after it, and follows the text when the change is before it.
#[test]
fn replace_all_keeps_the_cursor_when_the_change_is_elsewhere() {
    let mut field = field_at("hello world", 2);
    field.replace_all("hello there");
    assert_eq!(field.selection(), Selection::cursor(2));
    let mut field = field_at("ab", 0);
    field.replace_all("aXXb");
    assert_eq!(field.selection(), Selection::cursor(0));
    let mut field = field_at("ab", 2);
    field.replace_all("aXXb");
    assert_eq!(field.selection(), Selection::cursor(4));
}

/// A cursor inside the replaced part goes after the change.
#[test]
fn replace_all_puts_a_cursor_inside_the_change_after_it() {
    let mut field = field_at("hello world", 8);
    field.replace_all("hello there!");
    assert_eq!(field.text(), "hello there!");
    // The part that differs is "world" -> "there!" at 6; the cursor was in it.
    assert_eq!(field.selection(), Selection::cursor(12));
}

/// Both ends of a selection are moved.
#[test]
fn replace_all_moves_both_ends_of_the_selection() {
    let mut field = Field::new("abc def");
    select(&mut field, 4, 7);
    field.replace_all("xxabc def");
    assert_eq!(field.selection(), sel(6, 9));
}

/// The same text is no change: no step, no new revision, no splice.
#[test]
fn replace_all_with_the_same_text_does_nothing() {
    let mut field = Field::new("abc");
    field.take_splices();
    let revision = field.revision();
    field.replace_all("abc");
    assert!(!field.can_undo());
    assert_eq!(field.revision(), revision);
    assert!(field.take_splices().is_empty());
}

/// Replacing the text of a field with another that shares a start and an end changes only the middle.
#[test]
fn replace_all_changes_only_the_part_that_differs() {
    let mut field = Field::new("a\nb\nc\nd");
    field.take_splices();
    field.replace_all("a\nb\nX\nY\nc\nd");
    assert_eq!(field.take_splices(), [Splice { line: 2, removed: 1, inserted: 3 }]);
}

/// An outside change after a step of typing is a step of its own.
#[test]
fn replace_all_does_not_merge_with_typing() {
    let mut field = Field::new("");
    type_chars(&mut field, "a");
    field.replace_all("ab");
    field.apply(Action::Undo);
    assert_eq!(field.text(), "a");
    field.apply(Action::Undo);
    assert_eq!(field.text(), "");
}

// ---- reset ----

/// A new text has no undo and no redo, the cursor at the start.
#[test]
fn reset_clears_the_undo_and_the_redo() {
    let mut field = Field::new("");
    type_chars(&mut field, "abc");
    field.apply(Action::Undo);
    assert!(field.can_redo());
    let revision = field.revision();
    field.reset("x\ny");
    assert_eq!(field.text(), "x\ny");
    assert!(!field.can_undo());
    assert!(!field.can_redo());
    assert_eq!(field.selection(), Selection::cursor(0));
    assert_ne!(field.revision(), revision);
    assert_eq!(field.len_lines(), 2);
}

/// Typing after a reset starts from a clean undo.
#[test]
fn typing_after_a_reset_is_a_new_step() {
    let mut field = Field::new("");
    type_chars(&mut field, "abc");
    field.reset("x");
    type_chars(&mut field, "y");
    assert_eq!(field.text(), "yx");
    field.apply(Action::Undo);
    assert_eq!(field.text(), "x");
    assert!(!field.can_undo());
}

// ---- clipboard and double click ----

/// Copy gives the selection and changes nothing; with nothing selected it gives nothing.
#[test]
fn copy_gives_the_selection() {
    let mut field = Field::new("hello world");
    assert_eq!(field.apply(Action::Copy), None);
    select(&mut field, 6, 11);
    assert_eq!(field.apply(Action::Copy), Some("world".to_string()));
    assert_eq!(field.text(), "hello world");
    assert!(!field.can_undo());
    select(&mut field, 11, 6);
    assert_eq!(field.apply(Action::Copy), Some("world".to_string()));
}

/// Cut gives the selection and removes it, as a step of undo; with nothing selected it does nothing.
#[test]
fn cut_gives_the_selection_and_removes_it() {
    let mut field = Field::new("hello world");
    assert_eq!(field.apply(Action::Cut), None);
    assert!(!field.can_undo());
    select(&mut field, 6, 11);
    assert_eq!(field.apply(Action::Cut), Some("world".to_string()));
    assert_eq!(field.text(), "hello ");
    assert_eq!(field.selection(), Selection::cursor(6));
    field.apply(Action::Undo);
    assert_eq!(field.text(), "hello world");
    assert_eq!(field.selection(), sel(6, 11));
}

/// A double click selects a name, a run of spaces or a run of the other characters.
#[test]
fn word_at_selects_a_name_a_run_of_spaces_or_of_punctuation() {
    let field = Field::new("foo  bar_1 + x");
    assert_eq!(field.word_at(1), sel(0, 3));
    assert_eq!(field.word_at(0), sel(0, 3));
    assert_eq!(field.word_at(3), sel(3, 5));
    assert_eq!(field.word_at(7), sel(5, 10));
    assert_eq!(field.word_at(10), sel(10, 11));
    assert_eq!(field.word_at(11), sel(11, 12));
}

/// At the end of a line the word before the cursor is selected; on an empty line, nothing.
#[test]
fn word_at_the_end_of_a_line_takes_the_word_before() {
    let field = Field::new("foo  bar_1 + x");
    assert_eq!(field.word_at(14), sel(13, 14));
    let field = Field::new("ab\ncd\n\nef");
    assert_eq!(field.word_at(2), sel(0, 2));
    assert_eq!(field.word_at(4), sel(3, 5));
    assert_eq!(field.word_at(6), Selection::cursor(6));
}

/// The word of a double click is in characters, whatever their size, and the offset of its line is kept.
#[test]
fn word_at_counts_characters() {
    let field = Field::new("x\nété ✓");
    assert_eq!(field.word_at(3), sel(2, 5));
    assert_eq!(field.word_at(6), sel(6, 7));
}

// ---- splices and widest ----

/// A new text is given as one splice: the old lines (one, for an empty field) became the new ones. Taking them empties.
#[test]
fn a_new_field_has_one_splice_and_taking_empties_it() {
    let mut field = Field::new("a\nb");
    assert_eq!(field.take_splices(), [Splice { line: 0, removed: 1, inserted: 2 }]);
    assert!(field.take_splices().is_empty());
}

/// Typing in a line changes that line; a line ending inserted makes the line two.
#[test]
fn an_insert_gives_a_splice_of_the_lines_it_changed() {
    let mut field = field_at("a\nb", 3);
    field.take_splices();
    field.apply(Action::Insert("x".to_string()));
    assert_eq!(field.take_splices(), [Splice { line: 1, removed: 1, inserted: 1 }]);
    let mut field = field_at("a\nb", 1);
    field.take_splices();
    field.apply(Action::Insert("\nc".to_string()));
    assert_eq!(field.text(), "a\nc\nb");
    assert_eq!(field.take_splices(), [Splice { line: 0, removed: 1, inserted: 2 }]);
}

/// A removal across lines: three lines became one, and an undo gives them back.
#[test]
fn a_removal_across_lines_and_its_undo_give_splices() {
    let mut field = Field::new("a\nb\nc\nd");
    field.take_splices();
    select(&mut field, 1, 5);
    field.apply(Action::Backspace);
    assert_eq!(field.text(), "a\nd");
    assert_eq!(field.take_splices(), [Splice { line: 0, removed: 3, inserted: 1 }]);
    field.apply(Action::Undo);
    assert_eq!(field.text(), "a\nb\nc\nd");
    assert_eq!(field.take_splices(), [Splice { line: 0, removed: 1, inserted: 3 }]);
}

/// Splices come in the order of the edits, until taken.
#[test]
fn splices_pile_up_in_order_until_taken() {
    let mut field = field_at("a\nb", 1);
    field.take_splices();
    field.apply(Action::Insert("\n".to_string()));
    field.apply(Action::Insert("x".to_string()));
    assert_eq!(field.text(), "a\nx\nb");
    assert_eq!(field.take_splices(), [Splice { line: 0, removed: 1, inserted: 2 }, Splice { line: 1, removed: 1, inserted: 1 }]);
}

/// Enter gives a splice of two lines; an indented block, one of as many lines as it holds.
#[test]
fn newline_and_indent_give_splices() {
    let mut field = field_at("  a", 3);
    field.take_splices();
    field.apply(Action::Newline);
    assert_eq!(field.take_splices(), [Splice { line: 0, removed: 1, inserted: 2 }]);
    let mut field = Field::new("a\nb\nc");
    select(&mut field, 0, 3);
    field.take_splices();
    field.apply(Action::Indent);
    assert_eq!(field.take_splices(), [Splice { line: 0, removed: 2, inserted: 2 }]);
}

/// The width of the view never shrinks while one types, not by a removal, not by an undo.
#[test]
fn widest_never_shrinks_while_typing() {
    let mut field = Field::new("abcdef\nab");
    assert_eq!(field.widest(), 6);
    select(&mut field, 0, 7);
    field.apply(Action::Backspace);
    assert_eq!(field.text(), "ab");
    assert_eq!(field.widest(), 6);
    field.apply(Action::Insert("abcdefgh".to_string()));
    assert_eq!(field.widest(), 10);
    field.apply(Action::Undo);
    assert_eq!(field.widest(), 10);
}

/// A new text gives a new width.
#[test]
fn widest_follows_a_new_text() {
    let mut field = Field::new("abcdef");
    field.reset("é✓\nx");
    assert_eq!(field.widest(), 2);
}

// ---- colours ----

/// A highlighter whose state says whether a block comment `/* ... */` is open at the start of a line. A comment is
/// a span of class 1; it counts its calls.
struct Comments {
    calls: usize,
}

impl Highlighter for Comments {
    type State = bool;

    fn line(&mut self, state: &bool, line: &str, spans: &mut Vec<Span>) -> bool {
        self.calls += 1;
        let bytes = line.as_bytes();
        let mut open = *state;
        let mut start = 0;
        let mut i = 0;
        while i < bytes.len() {
            if !open && bytes[i..].starts_with(b"/*") {
                open = true;
                start = i;
                i += 2;
            } else if open && bytes[i..].starts_with(b"*/") {
                spans.push(Span { range: start..i + 2, class: 1 });
                open = false;
                i += 2;
            } else {
                i += 1;
            }
        }
        if open && start < bytes.len() {
            spans.push(Span { range: start..bytes.len(), class: 1 });
        }
        open
    }
}

fn comment(range: std::ops::Range<usize>) -> Span {
    Span { range, class: 1 }
}

fn setup(text: &str) -> (Field, Colours<Comments>) {
    let mut field = Field::new(text);
    let mut colours = Colours::new(Comments { calls: 0 });
    colours.splice(&field.take_splices());
    (field, colours)
}

fn sync(colours: &mut Colours<Comments>, field: &mut Field) {
    colours.splice(&field.take_splices());
}

fn spans_of(colours: &mut Colours<Comments>, field: &Field, line: usize) -> Vec<Span> {
    colours.spans(line, field.len_lines(), |l| field.line(l)).to_vec()
}

fn calls(colours: &Colours<Comments>) -> usize {
    colours.highlighter().calls
}

/// The state that a line leaves goes to the next: a comment opened on one line colours the lines after it, up to its end.
#[test]
fn a_comment_that_crosses_lines_colours_each_of_them() {
    let (field, mut colours) = setup("a /* b\nc\nd */ e\nf");
    assert_eq!(spans_of(&mut colours, &field, 0), [comment(2..6)]);
    assert_eq!(spans_of(&mut colours, &field, 1), [comment(0..1)]);
    assert_eq!(spans_of(&mut colours, &field, 2), [comment(0..4)]);
    assert!(spans_of(&mut colours, &field, 3).is_empty());
}

/// Asking for a line colours the lines up to it, once; asking again does not call the highlighter.
#[test]
fn spans_are_kept_and_asked_once() {
    let (field, mut colours) = setup("a /* b\nc\nd */ e\nf");
    spans_of(&mut colours, &field, 3);
    assert_eq!(calls(&colours), 4);
    spans_of(&mut colours, &field, 3);
    spans_of(&mut colours, &field, 0);
    assert_eq!(calls(&colours), 4);
}

/// A line is coloured only when asked for: lines after it are not.
#[test]
fn only_the_lines_up_to_the_one_asked_for_are_coloured() {
    let (field, mut colours) = setup("a\nb\nc\nd");
    spans_of(&mut colours, &field, 1);
    assert_eq!(calls(&colours), 2);
}

/// A line number past the end gives the last line.
#[test]
fn a_line_past_the_end_gives_the_last_line() {
    let (field, mut colours) = setup("a\n/* b");
    assert_eq!(spans_of(&mut colours, &field, 99), [comment(0..4)]);
}

/// Opening a comment on line 0 colours again the lines after it, whose state at their start changed.
#[test]
fn opening_a_comment_colours_again_the_lines_after_it() {
    let (mut field, mut colours) = setup("a\nb\nc\nd");
    spans_of(&mut colours, &field, 3);
    assert_eq!(calls(&colours), 4);
    go(&mut field, Motion::End);
    field.apply(Action::Insert(" /*".to_string()));
    assert_eq!(field.line(0), "a /*");
    sync(&mut colours, &mut field);
    assert_eq!(spans_of(&mut colours, &field, 0), [comment(2..4)]);
    assert_eq!(spans_of(&mut colours, &field, 1), [comment(0..1)]);
    assert_eq!(spans_of(&mut colours, &field, 2), [comment(0..1)]);
    assert_eq!(spans_of(&mut colours, &field, 3), [comment(0..1)]);
    assert_eq!(calls(&colours), 8);
}

/// Closing the comment on line 2 colours again line 2 and line 3, not the lines before it.
#[test]
fn closing_a_comment_colours_again_from_the_changed_line_on() {
    let (mut field, mut colours) = setup("a /*\nb\nc\nd");
    spans_of(&mut colours, &field, 3);
    assert_eq!(calls(&colours), 4);
    field.apply(Action::Select(Selection::cursor(8)));
    field.apply(Action::Insert("*/".to_string()));
    assert_eq!(field.line(2), "c*/");
    sync(&mut colours, &mut field);
    assert_eq!(spans_of(&mut colours, &field, 2), [comment(0..3)]);
    assert!(spans_of(&mut colours, &field, 3).is_empty());
    assert_eq!(spans_of(&mut colours, &field, 1), [comment(0..1)]);
    assert_eq!(calls(&colours), 6);
}

/// An edit that leaves the state at the start of the next lines as it was colours only the edited line.
#[test]
fn lines_with_the_same_start_state_are_not_coloured_again() {
    let (mut field, mut colours) = setup("a\nb\nc\nd");
    spans_of(&mut colours, &field, 3);
    assert_eq!(calls(&colours), 4);
    field.apply(Action::Select(Selection::cursor(3)));
    field.apply(Action::Insert("b".to_string()));
    assert_eq!(field.line(1), "bb");
    sync(&mut colours, &mut field);
    spans_of(&mut colours, &field, 3);
    assert_eq!(calls(&colours), 5);
}

/// The lines before an edit are not coloured again, even when they are asked for after it.
#[test]
fn the_lines_before_an_edit_are_not_coloured_again() {
    let (mut field, mut colours) = setup("a\nb\nc\nd");
    spans_of(&mut colours, &field, 3);
    field.apply(Action::Select(Selection::cursor(3)));
    field.apply(Action::Insert("b".to_string()));
    sync(&mut colours, &mut field);
    spans_of(&mut colours, &field, 0);
    assert_eq!(calls(&colours), 4);
}

/// Undo of the opening of a comment gives back the plain lines after it.
#[test]
fn undo_gives_back_the_colours_of_the_lines() {
    let (mut field, mut colours) = setup("a\nb\nc\nd");
    spans_of(&mut colours, &field, 3);
    go(&mut field, Motion::End);
    field.apply(Action::Insert(" /*".to_string()));
    sync(&mut colours, &mut field);
    assert_eq!(spans_of(&mut colours, &field, 3), [comment(0..1)]);
    field.apply(Action::Undo);
    sync(&mut colours, &mut field);
    for line in 0..4 {
        assert!(spans_of(&mut colours, &field, line).is_empty(), "line {line}");
    }
}

/// A splice that adds a line shifts the spans of the lines after it with them, and colours the new one.
#[test]
fn a_line_added_keeps_the_spans_of_the_other_lines() {
    let (mut field, mut colours) = setup("a\n/* x\ny */\nb");
    spans_of(&mut colours, &field, 3);
    assert_eq!(calls(&colours), 4);
    field.apply(Action::Select(Selection::cursor(1)));
    field.apply(Action::Insert("\nq".to_string()));
    assert_eq!(field.text(), "a\nq\n/* x\ny */\nb");
    sync(&mut colours, &mut field);
    assert!(spans_of(&mut colours, &field, 0).is_empty());
    assert!(spans_of(&mut colours, &field, 1).is_empty());
    assert_eq!(spans_of(&mut colours, &field, 2), [comment(0..4)]);
    assert_eq!(spans_of(&mut colours, &field, 3), [comment(0..4)]);
    assert!(spans_of(&mut colours, &field, 4).is_empty());
    // Only the two lines that the splice made were coloured again; the three after them kept their spans.
    assert_eq!(calls(&colours), 6);
}

/// A splice that removes lines drops their spans: the comment that was in them no longer colours the lines after.
#[test]
fn lines_removed_drop_their_spans() {
    let (mut field, mut colours) = setup("a\nq\n/* x\ny */\nb");
    spans_of(&mut colours, &field, 4);
    assert_eq!(calls(&colours), 5);
    select(&mut field, 2, 9);
    field.apply(Action::Backspace);
    assert_eq!(field.text(), "a\ny */\nb");
    sync(&mut colours, &mut field);
    assert!(spans_of(&mut colours, &field, 0).is_empty());
    assert!(spans_of(&mut colours, &field, 1).is_empty());
    assert!(spans_of(&mut colours, &field, 2).is_empty());
    // Line 1 alone was coloured again: line 2 had the same state at its start.
    assert_eq!(calls(&colours), 6);
}

/// A comment opened by a removal of lines: the lines that follow are coloured from the new state.
#[test]
fn a_removal_that_joins_a_comment_to_the_lines_after_it_colours_them() {
    let (mut field, mut colours) = setup("/* a\nb\n*/ c\nd");
    assert_eq!(spans_of(&mut colours, &field, 2), [comment(0..2)]);
    // Remove "b\n" (positions 5..7): "*/ c" now follows "/* a" directly.
    select(&mut field, 5, 7);
    field.apply(Action::Backspace);
    assert_eq!(field.text(), "/* a\n*/ c\nd");
    sync(&mut colours, &mut field);
    assert_eq!(spans_of(&mut colours, &field, 0), [comment(0..4)]);
    assert_eq!(spans_of(&mut colours, &field, 1), [comment(0..2)]);
    assert!(spans_of(&mut colours, &field, 2).is_empty());
}

/// Tab over several lines leaves an empty line empty: no spaces at the end of a line.
#[test]
fn indent_over_lines_leaves_an_empty_line_empty() {
    let mut field = Field::new("a\n\nb");
    select(&mut field, 0, 4);
    field.apply(Action::Indent);
    assert_eq!(field.text(), "    a\n\n    b");
}

/// A change from outside ends the typing: the next character typed is a step of its own.
#[test]
fn typing_after_a_change_from_outside_is_a_step_of_its_own() {
    let mut field = Field::new("");
    field.apply(Action::Insert("a".to_owned()));
    field.replace_all("ab");
    field.apply(Action::Move { motion: Motion::TextEnd, extend: false });
    field.replace_all("ab");
    field.apply(Action::Insert("c".to_owned()));
    field.apply(Action::Undo);
    assert_eq!(field.text(), "ab");
}

/// A stateless highlighter colours a line far down alone: the lines before it are not lexed.
#[test]
fn a_stateless_highlighter_colours_only_the_line_asked_for() {
    use std::cell::Cell;
    use std::rc::Rc;
    struct Counted(Rc<Cell<usize>>);
    impl Highlighter for Counted {
        type State = ();
        const STATELESS: bool = true;
        fn line(&mut self, _: &(), _: &str, _: &mut Vec<Span>) {
            self.0.set(self.0.get() + 1);
        }
    }
    let calls = Rc::new(Cell::new(0));
    let mut colours = Colours::new(Counted(calls.clone()));
    let _ = colours.spans(999, 1_000, |_| String::new());
    let _ = colours.spans(999, 1_000, |_| String::new());
    assert_eq!(calls.get(), 1);
}

// ---- edits, marks, positions, completion (written from the doc comments) ----

mod service_tests {
    use crate::complete::{Completion, is_name_char};
    use crate::field::{Action, Edit, Field, MAX_EDITS, Selection};
    use crate::lsp_types::{CompletionItem, Diagnostic, DiagnosticSeverity, Position, Range as LspRange};
    use crate::marks::{Mark, Marks};
    use crate::service::{ENCODING, LanguageService, Published, Silent, offset_in, offset_of, position_of};
    use crate::{Code, Colours, Highlighter, Span};

    fn at_cursor(text: &str, at: usize) -> Field {
        let mut field = Field::new(text);
        field.apply(Action::Select(Selection::cursor(at)));
        field
    }

    fn type_text(field: &mut Field, text: &str) {
        for c in text.chars() {
            field.apply(Action::Insert(c.to_string()));
        }
    }

    fn pos(line: u32, character: u32) -> Position {
        Position { line, character }
    }

    fn diagnostic(line: u32, from: u32, to: u32, severity: Option<DiagnosticSeverity>, message: &str) -> Diagnostic {
        Diagnostic { range: LspRange { start: pos(line, from), end: pos(line, to) }, severity, message: message.to_string(), ..Diagnostic::default() }
    }

    fn item(label: &str) -> CompletionItem {
        CompletionItem { label: label.to_string(), ..CompletionItem::default() }
    }

    /// A service that gives a fixed list of completions and the diagnostics it was loaded with, once.
    #[derive(Default)]
    struct Fixed {
        items: Vec<CompletionItem>,
        published: Vec<Published>,
    }

    impl LanguageService for Fixed {
        fn diagnostics(&mut self) -> Vec<Published> {
            std::mem::take(&mut self.published)
        }

        fn completion(&mut self, _: &Field, _: Position) -> Vec<CompletionItem> {
            self.items.clone()
        }

        fn hover(&mut self, _: &Field, _: Position) -> Option<crate::lsp_types::Hover> {
            None
        }

        fn definition(&mut self, _: &Field, _: Position) -> Option<crate::lsp_types::GotoDefinitionResponse> {
            None
        }
    }

    fn fixed(labels: &[&str]) -> Fixed {
        Fixed { items: labels.iter().map(|label| item(label)).collect(), published: Vec::new() }
    }

    fn labels(completion: &Completion) -> Vec<String> {
        completion.shown().0.iter().map(|item| item.label.clone()).collect()
    }

    // ---- Edit::range ----

    /// An edit before a range moves it by the difference of lengths.
    #[test]
    fn edit_before_a_range_moves_it() {
        assert_eq!(Edit { at: 0, removed: 0, inserted: 2 }.range(5..8), Some(7..10));
        assert_eq!(Edit { at: 0, removed: 2, inserted: 0 }.range(5..8), Some(3..6));
        assert_eq!(Edit { at: 1, removed: 2, inserted: 5 }.range(5..8), Some(8..11));
    }

    /// An edit after a range leaves it as it was.
    #[test]
    fn edit_after_a_range_leaves_it() {
        assert_eq!(Edit { at: 10, removed: 0, inserted: 2 }.range(5..8), Some(5..8));
        assert_eq!(Edit { at: 9, removed: 3, inserted: 1 }.range(5..8), Some(5..8));
    }

    /// Typing inside a range grows it; removing inside shrinks it.
    #[test]
    fn edit_inside_a_range_grows_or_shrinks_it() {
        assert_eq!(Edit { at: 6, removed: 0, inserted: 3 }.range(5..8), Some(5..11));
        assert_eq!(Edit { at: 6, removed: 1, inserted: 0 }.range(5..8), Some(5..7));
    }

    /// Text typed right after the end of a range does not extend it.
    #[test]
    fn typing_at_the_end_of_a_range_does_not_extend_it() {
        assert_eq!(Edit { at: 8, removed: 0, inserted: 1 }.range(5..8), Some(5..8));
    }

    /// An edit that removes the whole range drops it.
    #[test]
    fn edit_over_a_whole_range_drops_it() {
        assert_eq!(Edit { at: 5, removed: 3, inserted: 0 }.range(5..8), None);
        assert_eq!(Edit { at: 4, removed: 6, inserted: 0 }.range(5..8), None);
    }

    /// A point at the place of an insertion stays where it is.
    #[test]
    fn a_point_at_an_insertion_stays() {
        assert_eq!(Edit { at: 5, removed: 0, inserted: 1 }.range(5..5), Some(5..5));
    }

    // ---- Field::edits_since ----

    /// For the current revision there is nothing since: an empty list, not `None`.
    #[test]
    fn edits_since_the_current_revision_is_empty() {
        let field = Field::new("abc");
        assert_eq!(field.edits_since(field.revision()).map(|edits| edits.count()), Some(0));
    }

    /// The edits since an older revision come oldest first, in characters.
    #[test]
    fn edits_since_an_older_revision_lists_them_oldest_first() {
        let mut field = at_cursor("abc", 3);
        let before = field.revision();
        type_text(&mut field, "xy");
        let all: Vec<Edit> = field.edits_since(before).unwrap().copied().collect();
        assert_eq!(all, vec![Edit { at: 3, removed: 0, inserted: 1 }, Edit { at: 4, removed: 0, inserted: 1 }]);
        let last: Vec<Edit> = field.edits_since(before + 1).unwrap().copied().collect();
        assert_eq!(last, vec![Edit { at: 4, removed: 0, inserted: 1 }]);
    }

    /// A revision the field never had (one to come) gives `None`.
    #[test]
    fn edits_since_a_revision_to_come_is_none() {
        let field = Field::new("abc");
        assert!(field.edits_since(field.revision() + 5).is_none());
    }

    /// Past `MAX_EDITS` edits, the oldest are forgotten: `None` for a revision before them.
    #[test]
    fn edits_since_a_revision_past_the_limit_is_none() {
        let mut field = at_cursor("", 0);
        let start = field.revision();
        for _ in 0..=MAX_EDITS {
            field.apply(Action::Insert("a".to_string()));
        }
        assert!(field.edits_since(start).is_none());
        assert_eq!(field.edits_since(start + 1).map(|edits| edits.count()), Some(MAX_EDITS));
    }

    /// A `Replace` is one edit, and one step of undo, with the cursor after the new text.
    #[test]
    fn replace_is_one_edit_and_one_undo_step() {
        let mut field = at_cursor("hello world", 0);
        let before = field.revision();
        field.apply(Action::Replace { range: 0..5, text: "howdy!".to_string() });
        assert_eq!(field.text(), "howdy! world");
        assert_eq!(field.selection(), Selection::cursor(6));
        let all: Vec<Edit> = field.edits_since(before).unwrap().copied().collect();
        assert_eq!(all, vec![Edit { at: 0, removed: 5, inserted: 6 }]);
        field.apply(Action::Undo);
        assert_eq!(field.text(), "hello world");
    }

    // ---- Mark ----

    /// A diagnostic with no severity is an error.
    #[test]
    fn a_mark_with_no_severity_is_an_error() {
        let mark = Mark { range: 0..1, diagnostic: diagnostic(0, 0, 1, None, "m"), source: 0 };
        assert_eq!(mark.severity(), DiagnosticSeverity::ERROR);
        let warning = Mark { range: 0..1, diagnostic: diagnostic(0, 0, 1, Some(DiagnosticSeverity::WARNING), "m"), source: 0 };
        assert_eq!(warning.severity(), DiagnosticSeverity::WARNING);
    }

    /// A range covers its characters but not its end; a point covers itself and the character after.
    #[test]
    fn a_mark_covers_its_range_or_its_point() {
        let range = Mark { range: 2..5, diagnostic: Diagnostic::default(), source: 0 };
        assert!(!range.covers(1));
        assert!(range.covers(2) && range.covers(4));
        assert!(!range.covers(5));
        let point = Mark { range: 3..3, diagnostic: Diagnostic::default(), source: 0 };
        assert!(!point.covers(2));
        assert!(point.covers(3) && point.covers(4));
        assert!(!point.covers(5));
    }

    // ---- Marks ----

    /// A diagnostic is placed from its line and character.
    #[test]
    fn publish_places_a_diagnostic_by_line_and_character() {
        let text = "let a\nlet foo = 1";
        let field = Field::new(text);
        let mut marks = Marks::default();
        marks.publish(&field, 0, field.revision(), text, vec![diagnostic(1, 4, 7, None, "m")]);
        assert_eq!(marks.marks().len(), 1);
        assert_eq!(marks.marks()[0].range, 10..13);
        assert_eq!(marks.revision(), field.revision());
    }

    /// Typing inside an underlined name keeps it underlined and grows it; typing before moves it.
    #[test]
    fn marks_follow_typing() {
        let text = "let foo = 1";
        let mut field = Field::new(text);
        let mut marks = Marks::default();
        marks.publish(&field, 0, field.revision(), text, vec![diagnostic(0, 4, 7, None, "m")]);
        field.apply(Action::Select(Selection::cursor(5)));
        type_text(&mut field, "xx");
        marks.follow(&field);
        assert_eq!(marks.marks()[0].range, 4..9);
        field.apply(Action::Select(Selection::cursor(0)));
        type_text(&mut field, "  ");
        marks.follow(&field);
        assert_eq!(marks.marks()[0].range, 6..11);
        assert_eq!(marks.revision(), field.revision());
    }

    /// Text typed right after an underlined name does not extend the underline.
    #[test]
    fn marks_do_not_grow_at_their_end() {
        let text = "let foo = 1";
        let mut field = at_cursor(text, 7);
        let mut marks = Marks::default();
        marks.publish(&field, 0, field.revision(), text, vec![diagnostic(0, 4, 7, None, "m")]);
        type_text(&mut field, "z");
        marks.follow(&field);
        assert_eq!(marks.marks()[0].range, 4..7);
    }

    /// A mark whose text is wholly removed is dropped.
    #[test]
    fn marks_over_removed_text_are_dropped() {
        let text = "let foo = 1";
        let mut field = Field::new(text);
        let mut marks = Marks::default();
        marks.publish(&field, 0, field.revision(), text, vec![diagnostic(0, 4, 7, None, "m")]);
        field.apply(Action::Replace { range: 3..8, text: String::new() });
        marks.follow(&field);
        assert!(marks.marks().is_empty());
    }

    /// Diagnostics given for an older revision are moved by the edits typed since.
    #[test]
    fn publish_for_an_older_revision_moves_by_later_edits() {
        let text = "let foo = 1";
        let mut field = at_cursor(text, 0);
        let old = field.revision();
        type_text(&mut field, "ab");
        let mut marks = Marks::default();
        marks.publish(&field, 0, old, text, vec![diagnostic(0, 4, 7, None, "m")]);
        assert_eq!(marks.marks()[0].range, 6..9);
        assert_eq!(marks.revision(), field.revision());
    }

    /// Diagnostics given for a revision the field no longer keeps the edits of are dropped.
    #[test]
    fn publish_for_a_forgotten_revision_is_dropped() {
        let mut field = at_cursor("", 0);
        let old = field.revision();
        for _ in 0..=MAX_EDITS {
            field.apply(Action::Insert("a".to_string()));
        }
        let mut marks = Marks::default();
        marks.publish(&field, 0, old, "", vec![diagnostic(0, 0, 0, None, "m")]);
        assert!(marks.marks().is_empty());
    }

    /// A position past the end of the text is kept at the end.
    #[test]
    fn publish_keeps_a_position_past_the_text_at_its_end() {
        let text = "ab";
        let field = Field::new(text);
        let mut marks = Marks::default();
        marks.publish(&field, 0, field.revision(), text, vec![diagnostic(5, 1, 3, None, "m")]);
        assert_eq!(marks.marks()[0].range, 2..2);
    }

    /// A second set of a source replaces its first; the other sources stay; the marks are in the order of the text.
    #[test]
    fn publish_replaces_its_own_source_only() {
        let text = "aaaa bbbb cccc";
        let field = Field::new(text);
        let mut marks = Marks::default();
        marks.publish(&field, 1, field.revision(), text, vec![diagnostic(0, 10, 14, None, "one")]);
        marks.publish(&field, 2, field.revision(), text, vec![diagnostic(0, 0, 4, None, "two")]);
        marks.publish(&field, 1, field.revision(), text, vec![diagnostic(0, 5, 9, None, "one again")]);
        let found: Vec<(u8, String)> = marks.marks().iter().map(|mark| (mark.source, mark.diagnostic.message.clone())).collect();
        assert_eq!(found, vec![(2, "two".to_string()), (1, "one again".to_string())]);
        marks.publish(&field, 1, field.revision(), text, Vec::new());
        assert_eq!(marks.marks().len(), 1);
        assert_eq!(marks.marks()[0].source, 2);
    }

    /// `clear` removes the marks of one source.
    #[test]
    fn clear_removes_one_source() {
        let text = "aaaa bbbb";
        let field = Field::new(text);
        let mut marks = Marks::default();
        marks.publish(&field, 1, field.revision(), text, vec![diagnostic(0, 0, 4, None, "a")]);
        marks.publish(&field, 2, field.revision(), text, vec![diagnostic(0, 5, 9, None, "b")]);
        marks.clear(1);
        assert_eq!(marks.marks().len(), 1);
        assert_eq!(marks.marks()[0].source, 2);
    }

    /// `at` gives the marks under a position, errors first, whatever the order they came in.
    #[test]
    fn at_gives_errors_first() {
        let text = "abcdef";
        let field = Field::new(text);
        let mut marks = Marks::default();
        marks.publish(&field, 1, field.revision(), text, vec![diagnostic(0, 0, 4, Some(DiagnosticSeverity::WARNING), "warning")]);
        marks.publish(&field, 2, field.revision(), text, vec![diagnostic(0, 1, 5, Some(DiagnosticSeverity::ERROR), "error")]);
        marks.publish(&field, 3, field.revision(), text, vec![diagnostic(0, 2, 3, Some(DiagnosticSeverity::HINT), "hint")]);
        let found: Vec<&str> = marks.at(2).iter().map(|mark| mark.diagnostic.message.as_str()).collect();
        assert_eq!(found, vec!["error", "warning", "hint"]);
        let outside: Vec<&str> = marks.at(5).iter().map(|mark| mark.diagnostic.message.as_str()).collect();
        assert!(outside.is_empty());
    }

    // ---- positions ----

    /// Positions count lines and characters from 0, in code points.
    #[test]
    fn position_of_counts_lines_and_code_points() {
        let field = Field::new("ab\ncd\né😀x");
        assert_eq!(position_of(&field, 0), pos(0, 0));
        assert_eq!(position_of(&field, 4), pos(1, 1));
        assert_eq!(position_of(&field, 6), pos(2, 0));
        assert_eq!(position_of(&field, 8), pos(2, 2));
        assert_eq!(ENCODING, crate::lsp_types::PositionEncodingKind::UTF32);
    }

    /// `offset_of` is the reverse of `position_of`, kept within the line and the text.
    #[test]
    fn offset_of_is_kept_within_the_text() {
        let field = Field::new("ab\ncd");
        assert_eq!(offset_of(&field, pos(1, 1)), 4);
        assert_eq!(offset_of(&field, pos(0, 99)), 2);
        assert_eq!(offset_of(&field, pos(9, 0)), 5);
    }

    /// `offset_in` on a text: in a line, past the end of a line, past the last line.
    #[test]
    fn offset_in_is_kept_within_line_and_text() {
        let text = "ab\ncd\n\u{e9}x";
        assert_eq!(offset_in(text, pos(0, 0)), 0);
        assert_eq!(offset_in(text, pos(1, 1)), 4);
        assert_eq!(offset_in(text, pos(2, 1)), 7);
        assert_eq!(offset_in(text, pos(0, 99)), 2);
        assert_eq!(offset_in(text, pos(1, 99)), 5);
        assert_eq!(offset_in(text, pos(9, 0)), 8);
        assert_eq!(offset_in("", pos(0, 3)), 0);
        assert_eq!(offset_in("ab\n", pos(1, 0)), 3);
        assert_eq!(offset_in("ab\n", pos(2, 0)), 3);
    }

    /// The silent service says nothing.
    #[test]
    fn silent_says_nothing() {
        let field = Field::new("abc");
        let mut service = Silent;
        assert!(service.completion(&field, pos(0, 1)).is_empty());
        assert!(service.hover(&field, pos(0, 1)).is_none());
        assert!(service.definition(&field, pos(0, 1)).is_none());
        assert!(service.diagnostics().is_empty());
    }

    // ---- Completion ----

    /// A name char is a letter, a digit or `_`.
    #[test]
    fn name_chars() {
        assert!(is_name_char('a') && is_name_char('Z') && is_name_char('_') && is_name_char('7') && is_name_char('\u{e9}'));
        assert!(!is_name_char(' ') && !is_name_char('.') && !is_name_char('-') && !is_name_char('\n'));
    }

    /// Typing the first characters of a name opens the list with the entries that start as it does, shorter first.
    #[test]
    fn typed_name_opens_the_list() {
        let field = at_cursor("let fo", 6);
        let mut completion = Completion::default();
        assert!(!completion.is_open());
        completion.open(&field, &mut fixed(&["foobar", "bar", "foo"]), false);
        assert!(completion.is_open());
        assert_eq!(labels(&completion), vec!["foo", "foobar"]);
        assert_eq!(completion.shown().1, 0);
        assert_eq!(completion.start(), Some(4));
    }

    /// Typed, the list stays closed on a digit, on no name, and when nothing starts as the name does.
    #[test]
    fn typed_open_is_refused_on_a_digit_an_empty_name_or_no_match() {
        let mut service = fixed(&["foo", "bar"]);
        let mut completion = Completion::default();
        completion.open(&at_cursor("12", 2), &mut service, false);
        assert!(!completion.is_open());
        completion.open(&at_cursor("let ", 4), &mut service, false);
        assert!(!completion.is_open());
        completion.open(&at_cursor("zz", 2), &mut service, false);
        assert!(!completion.is_open());
        assert_eq!(completion.start(), None);
        assert!(completion.shown().0.is_empty());
    }

    /// Asked by its key, the list opens on no name at all, with everything.
    #[test]
    fn asked_open_works_on_an_empty_name() {
        let mut completion = Completion::default();
        completion.open(&at_cursor("let ", 4), &mut fixed(&["foo", "bar"]), true);
        assert!(completion.is_open());
        assert_eq!(completion.start(), Some(4));
        assert_eq!(labels(&completion).len(), 2);
    }

    /// The list does not open when something is selected.
    #[test]
    fn no_list_over_a_selection() {
        let mut field = at_cursor("foo", 0);
        field.apply(Action::Select(Selection { anchor: 0, head: 2 }));
        let mut completion = Completion::default();
        completion.open(&field, &mut fixed(&["foo"]), true);
        assert!(!completion.is_open());
    }

    /// The match is made without regard to case.
    #[test]
    fn matching_ignores_case() {
        let mut completion = Completion::default();
        completion.open(&at_cursor("FO", 2), &mut fixed(&["foo", "bar"]), false);
        assert_eq!(labels(&completion), vec!["foo"]);
    }

    /// Typing narrows the list; the list closes when nothing is left.
    #[test]
    fn typing_narrows_the_list() {
        let mut field = at_cursor("f", 1);
        let mut completion = Completion::default();
        completion.open(&field, &mut fixed(&["foo", "foobar", "fig"]), false);
        assert_eq!(labels(&completion), vec!["fig", "foo", "foobar"]);
        type_text(&mut field, "o");
        completion.follow(&field);
        assert_eq!(labels(&completion), vec!["foo", "foobar"]);
        type_text(&mut field, "ob");
        completion.follow(&field);
        assert_eq!(labels(&completion), vec!["foobar"]);
        type_text(&mut field, "z");
        completion.follow(&field);
        assert!(!completion.is_open());
    }

    /// The list closes when the cursor leaves the name, by a character that ends it or by a move.
    #[test]
    fn leaving_the_name_closes_the_list() {
        let mut field = at_cursor("fo", 2);
        let mut completion = Completion::default();
        completion.open(&field, &mut fixed(&["foo"]), false);
        type_text(&mut field, " ");
        completion.follow(&field);
        assert!(!completion.is_open());

        let mut field = at_cursor("x fo", 4);
        completion.open(&field, &mut fixed(&["foo"]), false);
        assert!(completion.is_open());
        field.apply(Action::Select(Selection::cursor(0)));
        completion.follow(&field);
        assert!(!completion.is_open());
    }

    /// Erasing the whole name closes a list opened by typing, but not one that was asked for.
    #[test]
    fn erasing_the_name_closes_only_a_typed_list() {
        let mut field = at_cursor("f", 1);
        let mut completion = Completion::default();
        completion.open(&field, &mut fixed(&["foo"]), false);
        field.apply(Action::Replace { range: 0..1, text: String::new() });
        completion.follow(&field);
        assert!(!completion.is_open());

        let mut field = at_cursor("f", 1);
        completion.open(&field, &mut fixed(&["foo"]), true);
        field.apply(Action::Replace { range: 0..1, text: String::new() });
        completion.follow(&field);
        assert!(completion.is_open());
    }

    /// `close` closes it.
    #[test]
    fn close_closes() {
        let mut completion = Completion::default();
        completion.open(&at_cursor("fo", 2), &mut fixed(&["foo"]), false);
        completion.close();
        assert!(!completion.is_open());
    }

    /// The choice goes to the next entry or the one before, around the ends.
    #[test]
    fn choose_wraps_around() {
        let mut completion = Completion::default();
        completion.open(&at_cursor("f", 1), &mut fixed(&["fa", "fbb", "fccc"]), false);
        assert_eq!(completion.shown().1, 0);
        completion.choose(1);
        assert_eq!(completion.shown().1, 1);
        completion.choose(1);
        completion.choose(1);
        assert_eq!(completion.shown().1, 0);
        completion.choose(-1);
        assert_eq!(completion.shown().1, 2);
        completion.set_chosen(1);
        assert_eq!(completion.shown().1, 1);
        completion.set_chosen(7);
        assert_eq!(completion.shown().1, 1);
    }

    /// Taking writes the label in place of the name typed, the cursor after it, as one step of undo.
    #[test]
    fn take_writes_the_label_as_one_undo_step() {
        let mut field = at_cursor("let fo = 1", 6);
        let mut completion = Completion::default();
        completion.open(&field, &mut fixed(&["foo", "foobar"]), false);
        completion.choose(1);
        assert!(completion.take(&mut field));
        assert_eq!(field.text(), "let foobar = 1");
        assert_eq!(field.selection(), Selection::cursor(10));
        assert!(!completion.is_open());
        field.apply(Action::Undo);
        assert_eq!(field.text(), "let fo = 1");
    }

    /// Taking with no list open writes nothing.
    #[test]
    fn take_with_no_list_does_nothing() {
        let mut field = at_cursor("fo", 2);
        let mut completion = Completion::default();
        assert!(!completion.take(&mut field));
        assert_eq!(field.text(), "fo");
    }

    // ---- Code ----

    struct Plain;

    impl Highlighter for Plain {
        type State = ();
        fn line(&mut self, _: &(), _: &str, _: &mut Vec<Span>) {}
    }

    /// `take_published` places what the service published, for the revision it names, and brings the marks to the field.
    #[test]
    fn take_published_places_the_marks_of_the_service() {
        let mut code = Code::new(Plain);
        code.field = Field::new("let foo = 1");
        let revision = code.field.revision();
        code.field.apply(Action::Select(Selection::cursor(0)));
        type_text(&mut code.field, "  ");
        let mut service = Fixed::default();
        service.published.push(Published { source: 4, revision, text: "let foo = 1".into(), diagnostics: vec![diagnostic(0, 4, 7, None, "m")] });
        code.take_published(&mut service);
        assert_eq!(code.marks.marks().len(), 1);
        assert_eq!(code.marks.marks()[0].range, 6..9);
        assert_eq!(code.marks.marks()[0].source, 4);
        assert_eq!(code.marks.revision(), code.field.revision());
        code.take_published(&mut service);
        assert_eq!(code.marks.marks().len(), 1);
        let _ = Colours::new(Plain);
    }
}
