//! `block.set_terminal`, `block.set_size`, `block.set_band` and `block.reset` (commands.md, "The block of a function and the
//! mask of a library", spec 11.2 and 5.2). Each expected text is written by hand from the canonical order of spec 5.2: `top`,
//! `left`, the lines in signature order, `band`; `input` or `output` only where the name alone would mean the other terminal.

use super::{FILE, gives, refused};
use crate::{BlockAt, Op, Terminal};
use labcraft_graph::BandPlace;

const HEAD: &str = "use core\n\nfn f(a: f64, err: error, b: f64) -> (y: f64, err: error) {\n    y = core.add(a, b)\n}\n";

fn with(lines: &str) -> String {
    format!("{HEAD}\nterminals {{\n{lines}}}\n")
}

fn input(name: &str) -> Terminal {
    Terminal::Input(name.to_owned())
}

fn output(name: &str) -> Terminal {
    Terminal::Output(name.to_owned())
}

fn set(terminal: Terminal, side: &str, place: &str) -> Op {
    Op::BlockSetTerminal { file: FILE.to_owned(), terminal, at: Some(BlockAt::parse(side, place).expect("a place")) }
}

fn unset(terminal: Terminal) -> Op {
    Op::BlockSetTerminal { file: FILE.to_owned(), terminal, at: None }
}

fn size(top: Option<Option<u32>>, left: Option<Option<u32>>) -> Op {
    Op::BlockSetSize { file: FILE.to_owned(), top, left }
}

#[test]
fn a_terminal_is_written_where_the_name_alone_says_what_it_is() {
    // `a` is an input only: on any side the name alone means it. `y` is an output only: likewise.
    gives(HEAD, set(input("a"), "left", "3"), &with("    left 3 a\n"));
    gives(HEAD, set(input("a"), "right", "2"), &with("    right 2 a\n"));
    gives(HEAD, set(output("y"), "left", "2"), &with("    left 2 y\n"));
    gives(HEAD, set(input("b"), "top", "last"), &with("    top last b\n"));
    gives(HEAD, set(input("b"), "left", "middle"), &with("    left middle b\n"));
}

#[test]
fn input_and_output_stay_where_the_name_alone_would_mean_the_other_terminal() {
    // `err` is both: alone it means the input on the left or the top, the output on the right (11.2).
    gives(HEAD, set(input("err"), "left", "2"), &with("    left 2 err\n"));
    gives(HEAD, set(output("err"), "right", "2"), &with("    right 2 err\n"));
    gives(HEAD, set(output("err"), "left", "2"), &with("    left 2 output err\n"));
    gives(HEAD, set(input("err"), "right", "2"), &with("    right 2 input err\n"));
    gives(HEAD, set(output("err"), "top", "3"), &with("    top 3 output err\n"));
}

#[test]
fn lines_go_in_signature_order_and_a_second_command_replaces_the_line() {
    // Inputs in signature order (a, err, b), then the outputs: whatever the order of the commands.
    let before = with("    left 3 b\n");
    gives(&before, set(input("a"), "left", "2"), &with("    left 2 a\n    left 3 b\n"));
    gives(&before, set(input("b"), "left", "2"), &with("    left 2 b\n"));
    gives(&before, set(output("y"), "right", "1"), &with("    left 3 b\n    right 1 y\n"));
}

#[test]
fn a_null_place_removes_the_line_and_the_section_when_it_is_empty() {
    gives(&with("    left 3 a\n"), unset(input("a")), HEAD);
    gives(&with("    left 3 a\n    left 2 b\n"), unset(input("a")), &with("    left 2 b\n"));
}

#[test]
fn a_place_is_judged_against_the_side() {
    // Top: first place kept for `after:`; 4 places without `top N`, so 5 is past the last.
    for place in ["1", "5"] {
        assert_eq!(refused(HEAD, set(input("a"), "top", place), "params.invalid").parameter, Some("place"));
    }
    // A place of a given size: `top 5` has 5 places, so 6 is past it, and `left 3`: 4.
    assert_eq!(refused(&with("    top 5\n"), set(input("a"), "top", "6"), "params.invalid").parameter, Some("place"));
    assert_eq!(refused(&with("    left 3\n"), set(input("a"), "left", "4"), "params.invalid").parameter, Some("place"));
    assert_eq!(refused(&with("    left 3\n"), set(input("a"), "right", "4"), "params.invalid").parameter, Some("place"));
    gives(&with("    top 5\n"), set(input("a"), "top", "5"), &with("    top 5\n    top 5 a\n"));
    // `left 4` with a `middle` has 5 places: place 5 is there.
    gives(&with("    left 4\n    left middle a\n"), set(input("b"), "left", "5"), &with("    left 4\n    left middle a\n    left 5 b\n"));
    // Place 0 is no place at all: the parameter is refused as it is read.
    assert_eq!(BlockAt::parse("left", "0").expect_err("no place 0").parameter, Some("place"));
}

#[test]
fn a_place_that_another_line_holds_is_refused() {
    let before = with("    left 2 a\n");
    assert_eq!(refused(&before, set(input("b"), "left", "2"), "params.invalid").parameter, Some("place"));
    // Another side is free.
    gives(&before, set(input("b"), "right", "2"), &with("    left 2 a\n    right 2 b\n"));
}

#[test]
fn a_size_is_between_1_or_0_and_64_and_holds_the_places_that_the_lines_name() {
    gives(HEAD, size(Some(Some(5)), Some(Some(3))), &with("    top 5\n    left 3\n"));
    for places in [0, 65] {
        assert_eq!(refused(HEAD, size(Some(Some(places)), None), "params.invalid").parameter, Some("top"));
    }
    assert_eq!(refused(HEAD, size(None, Some(Some(65))), "params.invalid").parameter, Some("left"));
    gives(HEAD, size(Some(Some(1)), None), &with("    top 1\n"));
    gives(HEAD, size(None, Some(Some(0))), &with("    left 0\n"));
    gives(HEAD, size(Some(Some(2)), Some(Some(1))), &with("    top 2\n    left 1\n"));
    gives(HEAD, size(Some(Some(3)), None), &with("    top 3\n"));
    gives(HEAD, size(Some(Some(64)), None), &with("    top 64\n"));
    // A line names place 7 on the top: the smallest size is 7.
    let before = with("    top 8\n    top 7 a\n");
    let error = refused(&before, size(Some(Some(5)), None), "params.invalid");
    assert_eq!(error.parameter, Some("top"));
    assert!(error.message.contains('7'), "{}", error.message);
    gives(&before, size(Some(Some(7)), None), &with("    top 7\n    top 7 a\n"));
    // The left size holds the places of the right side too.
    let before = with("    left 5\n    right 5 y\n");
    assert_eq!(refused(&before, size(None, Some(Some(4))), "params.invalid").parameter, Some("left"));
    // `Some(None)` removes a size, and an empty section goes.
    gives(&with("    top 5\n"), size(Some(None), None), HEAD);
}

#[test]
fn the_band_is_one_line_and_null_removes_it() {
    let band = |band| Op::BlockSetBand { file: FILE.to_owned(), band };
    gives(HEAD, band(Some(BandPlace::Above)), &with("    band above\n"));
    gives(&with("    band above\n"), band(Some(BandPlace::Hidden)), &with("    band hidden\n"));
    gives(&with("    left 3 a\n    band above\n"), band(None), &with("    left 3 a\n"));
    gives(&with("    band above\n"), band(None), HEAD);
}

#[test]
fn a_reset_removes_the_section() {
    let reset = Op::BlockReset { file: FILE.to_owned() };
    gives(&with("    top 5\n    left 3\n    left middle a\n    band above\n"), reset, HEAD);
}

#[test]
fn a_place_that_a_pair_takes_on_the_facing_side_is_refused() {
    // `left 2 err` names the pair alone: its output takes the right place 2 too (11.2), so `y` cannot go there.
    refused(
        &with(
            "    left 2 err
",
        ),
        set(output("y"), "right", "2"),
        "params.invalid",
    );
}

#[test]
fn a_size_counts_the_middle_that_makes_its_side_odd() {
    // `top middle b` makes 4 places 5 (11.2), which hold `top 5 a`: `top 4` is written.
    let before = with(
        "    top 5 a
    top middle b
",
    );
    gives(
        &before,
        size(Some(Some(4)), None),
        &with(
            "    top 4
    top 5 a
    top middle b
",
        ),
    );
}

#[test]
fn the_line_of_a_terminal_follows_its_rename_and_goes_with_it() {
    let rename = |terminal: Terminal, name: &str| Op::SignatureRename { file: FILE.to_owned(), terminal, name: name.to_owned() };
    let remove = |terminal: Terminal| Op::SignatureRemove { file: FILE.to_owned(), terminal };
    // The line names `a` by the name alone: it names `alpha` once `a` is renamed (commands.md, `signature.rename`).
    let renamed = "use core

fn f(alpha: f64, err: error, b: f64) -> (y: f64, err: error) {
    y = core.add(alpha, b)
}
";
    gives(
        &with(
            "    right 2 a
",
        ),
        rename(input("a"), "alpha"),
        &format!(
            "{renamed}
terminals {{
    right 2 alpha
}}
"
        ),
    );
    // An input that nothing reads goes with its line, and the section with it when nothing is left (`signature.remove`).
    let head = "fn g(a: f64, c: f64) -> (y: f64) {
    y = a
}
";
    gives(
        &format!(
            "{head}
terminals {{
    left 3 c
}}
"
        ),
        remove(input("c")),
        "fn g(a: f64) -> (y: f64) {
    y = a
}
",
    );
}

#[test]
fn a_move_that_pushes_out_another_line_is_refused() {
    // `top middle a` makes the default 4 top places 5, which `top 5 b` needs: moving `a` off the top leaves 4 (11.2).
    refused(
        &with(
            "    top middle a
    top 5 b
",
        ),
        set(input("a"), "left", "2"),
        "params.invalid",
    );
    // Without `output err`'s own line, the pair `left 2 err` would take the right 2, which `y` holds.
    refused(
        &with(
            "    left 2 err
    right 3 output err
    right 2 y
",
        ),
        unset(output("err")),
        "params.invalid",
    );
}

#[test]
fn a_line_keeps_naming_its_terminal_when_a_rename_makes_its_name_shared() {
    // `x` becomes `y`, the name of the output: on the right, `y` alone would mean the output, so the line of the input says
    // `input`; on the left, `y` alone would mean the input, so the line of the output says `output` (11.2).
    let rename = Op::SignatureRename { file: FILE.to_owned(), terminal: input("x"), name: "y".to_owned() };
    let head = "fn g(x: f64, z: f64) -> (y: f64) {
    y = z
}
";
    let after = "fn g(y: f64, z: f64) -> (y: f64) {
    y = z
}
";
    gives(
        &format!(
            "{head}
terminals {{
    right 2 x
    left 3 y
}}
"
        ),
        rename,
        &format!(
            "{after}
terminals {{
    right 2 input y
    left 3 output y
}}
"
        ),
    );
}

fn outline(items: Option<&str>) -> Op {
    Op::BlockSetOutline { file: FILE.to_owned(), outline: items.map(str::to_owned) }
}

fn with_outline(items: &str) -> String {
    format!("{HEAD}\noutline {{\n{items}}}\n")
}

#[test]
fn an_outline_is_written_in_canonical_form_and_null_removes_it() {
    // Spec 5.2: the body first, then the pieces of the top, left and right sides; plain decimal; one line each.
    gives(HEAD, outline(Some("template d")), &with_outline("    template d\n"));
    gives(
        HEAD,
        outline(Some("right terminal {path (0,-2) (8,0) (0,2)}\npath (0,0) (w - 0,0) round 0_4 (w,h) (0,h)")),
        &with_outline("    path (0, 0) (w, 0) round 4 (w, h) (0, h)\n    right terminal { path (0, -2) (8, 0) (0, 2) }\n"),
    );
    gives(&with_outline("    template d\n"), outline(Some("template block")), &with_outline("    template block\n"));
    gives(&with_outline("    template d\n"), outline(None), HEAD);
    // After `terminals`, which stays.
    gives(&with("    top 5\n"), outline(Some("template d")), &format!("{}\noutline {{\n    template d\n}}\n", with("    top 5\n")));
}

#[test]
fn an_outline_that_does_not_read_or_that_check_refuses_is_refused_and_one_that_breaks_a_guard_is_written() {
    for items in
        ["path (0, 0) (w, 0)", "template e", "template d\ntemplate block", "path (0, 0) (w, 0) round 17 (w, h)", "path (0, 0) (2000, 0) (w, h)"]
    {
        assert_eq!(refused(HEAD, outline(Some(items)), "params.invalid").parameter, Some("outline"), "{items}");
    }
    // A corner outside the box is a warning of `check`, at the size of each block: written.
    gives(HEAD, outline(Some("path (0, 0) (w + 4, 0) (w, h) (0, h)")), &with_outline("    path (0, 0) (w + 4, 0) (w, h) (0, h)\n"));
}

#[test]
fn a_reset_removes_the_outline_too() {
    let reset = Op::BlockReset { file: FILE.to_owned() };
    gives(&format!("{}\noutline {{\n    template d\n}}\n", with("    top 5\n")), reset, HEAD);
}
