//! `terminal.move` and the `input x at y` / `output x at y` entries of M1 (commands.md 6.7, spec 5.2 and 11). Each expected
//! text is written by hand from the canonical order of spec 5.2: `frame size`, the structures, the `input` entries then the
//! `output` entries in signature order, `node`, then `wire`.

use super::{FILE, gives, refused};
use crate::{Op, Terminal};

const HEAD: &str = "use core\n\nfn f(a: f64, err: error, b: f64) -> (y: f64, err: error) {\n    y = core.add(a, b)\n}\n";

fn with_layout(entries: &str) -> String {
    format!("{HEAD}\nlayout {{\n{entries}}}\n")
}

fn input(name: &str) -> Terminal {
    Terminal::Input(name.to_owned())
}

fn output(name: &str) -> Terminal {
    Terminal::Output(name.to_owned())
}

fn move_to(terminal: Terminal, y: i64) -> Op {
    Op::TerminalMove { file: FILE.to_owned(), terminal, y }
}

#[test]
fn a_move_writes_the_entry_of_the_terminal() {
    let edited = gives(HEAD, move_to(input("b"), 120), &with_layout("    input b at 120\n"));
    assert_eq!(edited.names.get("terminal").map(String::as_str), Some("input b"));
    gives(HEAD, move_to(output("y"), -16), &with_layout("    output y at -16\n"));
}

#[test]
fn a_move_replaces_the_entry_that_the_terminal_has_and_keeps_its_comment() {
    gives(&with_layout("    // by hand\n    input a at 8\n"), move_to(input("a"), 64), &with_layout("    // by hand\n    input a at 64\n"));
}

#[test]
fn an_input_and_an_output_of_one_name_have_an_entry_each() {
    // R-2: `input err` and `output err` are two terminals; the entries go inputs first, in signature order (spec 5.2).
    gives(&with_layout("    output err at 40\n"), move_to(input("err"), 40), &with_layout("    input err at 40\n    output err at 40\n"));
    gives(
        &with_layout("    input b at 8\n    output err at 40\n"),
        move_to(input("a"), 200),
        &with_layout("    input a at 200\n    input b at 8\n    output err at 40\n"),
    );
}

#[test]
fn the_height_is_written_as_given_off_the_grid_and_close_to_another() {
    // The grid of 8 and the 24 between terminals are rules of the gesture, not of the command (commands.md 6.7).
    gives(&with_layout("    input a at 8\n"), move_to(input("b"), 13), &with_layout("    input a at 8\n    input b at 13\n"));
}

#[test]
fn a_move_names_a_terminal_of_the_function_in_the_range() {
    refused(HEAD, move_to(input("y"), 8), "edit.unknown");
    refused(HEAD, move_to(output("a"), 8), "edit.unknown");
    let error = refused(HEAD, move_to(input("a"), 1_000_001), "params.invalid");
    assert_eq!(error.parameter, Some("y"));
    gives(HEAD, move_to(input("a"), -1_000_000), &with_layout("    input a at -1000000\n"));
}

#[test]
fn arrange_removes_the_entries_of_the_terminals_it_names_or_of_all_for_the_whole_function() {
    let before = with_layout("    input a at 8\n    output y at 16\n");
    let arrange = |terminals: &[&str]| Op::LayoutArrange {
        file: FILE.to_owned(),
        nodes: None,
        wires: Vec::new(),
        terminals: terminals.iter().map(|terminal| (*terminal).to_owned()).collect(),
        within: crate::Within::default(),
    };
    gives(&before, arrange(&["output y"]), &with_layout("    input a at 8\n"));
    gives(&before, arrange(&[]), HEAD);
}

#[test]
fn the_entry_follows_a_rename_and_goes_with_a_remove() {
    let rename = |terminal: Terminal, name: &str| Op::SignatureRename { file: FILE.to_owned(), terminal, name: name.to_owned() };
    gives(
        &with_layout("    input a at 8\n    output y at 16\n"),
        rename(input("a"), "c"),
        "use core\n\nfn f(c: f64, err: error, b: f64) -> (y: f64, err: error) {\n    y = core.add(c, b)\n}\n\nlayout {\n    input c at 8\n    output y at 16\n}\n",
    );
    gives(
        &with_layout("    input a at 8\n    output y at 16\n"),
        rename(output("y"), "sum"),
        "use core\n\nfn f(a: f64, err: error, b: f64) -> (sum: f64, err: error) {\n    sum = core.add(a, b)\n}\n\nlayout {\n    input a at 8\n    output sum at 16\n}\n",
    );
    // The input `err` goes; the output `err` and its entry stay.
    gives(
        &with_layout("    input err at 8\n    output err at 16\n"),
        Op::SignatureRemove { file: FILE.to_owned(), terminal: input("err") },
        "use core\n\nfn f(a: f64, b: f64) -> (y: f64, err: error) {\n    y = core.add(a, b)\n}\n\nlayout {\n    output err at 16\n}\n",
    );
}

#[test]
fn a_terminal_named_with_more_spaces_is_the_same_terminal() {
    // `select::terminal` reads `input  a` as `input a`: arranging it removes its entry.
    let arrange = Op::LayoutArrange {
        file: FILE.to_owned(),
        nodes: None,
        wires: Vec::new(),
        terminals: vec!["input  a".to_owned()],
        within: crate::Within::default(),
    };
    gives(&with_layout("    input a at 8\n    output y at 16\n"), arrange, &with_layout("    output y at 16\n"));
}

#[test]
fn a_move_leaves_one_entry_and_a_remove_takes_it_away_with_its_comment() {
    // Two entries of one terminal are `LC5007`: the move writes one.
    gives(&with_layout("    input a at 400\n    input a at 800\n"), move_to(input("a"), 64), &with_layout("    input a at 64\n"));
    // A comment keeps an entry that names nothing from `fmt`; `signature.remove` takes it with the terminal. What read
    // `b` is left out, its last argument (commands.md 9.1).
    gives(
        &with_layout("    // by hand\n    input b at 8\n    output y at 16\n"),
        Op::SignatureRemove { file: FILE.to_owned(), terminal: input("b") },
        "use core\n\nfn f(a: f64, err: error) -> (y: f64, err: error) {\n    y = core.add(a)\n}\n\nlayout {\n    output y at 16\n}\n",
    );
}
