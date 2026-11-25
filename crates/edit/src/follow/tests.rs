//! The renames of spec 11 (decisions of the owner, 2026-11-25). Each expected text is written by hand from the rule: the
//! old name replaced by the new one in the entries of `layout` that named it, nothing else changed.

use labcraft_types::Limits;

use super::follow_renames;

const BEFORE: &str = "labcraft 1

use core

fn f(a: f64, b: f64) -> (out: f64) {
    v = core.add(a, b)
    w = core.mul(v, 2.0)
    out = core.neg(w)
}

layout {
    node v at (40, 80)
    node w at (200, 80)
    wire v -> w via (120, 80) (120, 100)
}
";

fn follow(before: &str, after: &str) -> Option<(String, Vec<(String, String)>)> {
    follow_renames(before.as_bytes(), after.as_bytes(), &Limits::default())
        .map(|followed| (String::from_utf8(followed.bytes).unwrap_or_default(), followed.renamed))
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter().map(|(from, to)| ((*from).to_owned(), (*to).to_owned())).collect()
}

/// The issue's case: an agent renames `v` to `voltage` in the body and leaves `layout` as it read it.
#[test]
fn a_rename_in_the_body_moves_the_node_line_and_the_wire_lines() {
    let after = BEFORE.replace("    v = core.add", "    voltage = core.add").replace("core.mul(v, 2.0)", "core.mul(voltage, 2.0)");
    let expected = "labcraft 1

use core

fn f(a: f64, b: f64) -> (out: f64) {
    voltage = core.add(a, b)
    w = core.mul(voltage, 2.0)
    out = core.neg(w)
}

layout {
    node voltage at (40, 80)
    node w at (200, 80)
    wire voltage -> w via (120, 80) (120, 100)
}
";
    assert_eq!(follow(BEFORE, &after), Some((expected.to_owned(), pairs(&[("v", "voltage")]))));
}

/// Typed by hand in the `Text` panel: the line first, its reader not yet. Who reads the value does not count.
#[test]
fn a_rename_of_the_line_alone_keeps_the_node() {
    let after = BEFORE.replace("    v = core.add", "    vol = core.add");
    let (text, renamed) = follow(BEFORE, &after).unwrap_or_default();
    assert_eq!(renamed, pairs(&[("v", "vol")]));
    assert!(text.contains("    node vol at (40, 80)\n") && text.contains("    wire vol -> w via (120, 80) (120, 100)\n"), "{text}");
    assert!(text.contains("core.mul(v, 2.0)"), "the body stays as written: {text}");
}

/// A rename written with a new reader, and two renames in one write.
#[test]
fn renames_written_with_other_changes_follow() {
    let after = BEFORE.replace(
        "    v = core.add(a, b)\n    w = core.mul(v, 2.0)\n    out = core.neg(w)\n",
        "    voltage = core.add(a, b)\n    doubled = core.mul(voltage, 2.0)\n    spare = core.add(voltage, 1.0)\n    out = core.neg(doubled)\n",
    );
    let expected_layout = "layout {
    node voltage at (40, 80)
    node doubled at (200, 80)
    wire voltage -> doubled via (120, 80) (120, 100)
}
";
    let (text, renamed) = follow(BEFORE, &after).unwrap_or_default();
    assert!(text.ends_with(expected_layout), "{text}");
    assert_eq!(renamed, pairs(&[("v", "voltage"), ("w", "doubled")]));
}

/// A line that changes besides its name is another node: its entries stay (spec 11, the rule of a missing node).
#[test]
fn a_line_that_changes_besides_its_name_is_not_a_rename() {
    let after = BEFORE.replace("    v = core.add(a, b)", "    voltage = core.sub(a, b)").replace("core.mul(v, 2.0)", "core.mul(voltage, 2.0)");
    assert_eq!(follow(BEFORE, &after), None);
    let after = BEFORE.replace("    v = core.add(a, b)", "    voltage = core.add(b, a)");
    assert_eq!(follow(BEFORE, &after), None);
}

/// Two lines that could be the one renamed: nothing is guessed.
#[test]
fn two_lines_that_could_be_the_one_renamed_follow_nothing() {
    let before = BEFORE.replace("    v = core.add(a, b)\n", "    v = core.add(a, b)\n    u = core.add(a, b)\n");
    let after = before.replace("    v = core.add(a, b)\n    u = core.add(a, b)\n", "    p = core.add(a, b)\n    q = core.add(a, b)\n");
    assert_eq!(follow(&before, &after), None);
}

/// An exchange of two names loses none: the entries stay with the names.
#[test]
fn an_exchange_of_two_names_moves_nothing() {
    let after = BEFORE.replace(
        "    v = core.add(a, b)\n    w = core.mul(v, 2.0)\n    out = core.neg(w)\n",
        "    w = core.add(a, b)\n    v = core.mul(w, 2.0)\n    out = core.neg(v)\n",
    );
    assert_eq!(follow(BEFORE, &after), None);
}

/// A new name that `layout` already names: those entries win, and the old ones stay.
#[test]
fn entries_of_the_new_name_win() {
    let after = BEFORE.replace("    v = core.add", "    voltage = core.add").replace("    node w at", "    node voltage at (1, 2)\n    node w at");
    assert_eq!(follow(BEFORE, &after), None);
}

/// A label is renamed as a value is: `node #sum` takes `#total`, and the wire that starts at the value `v` stays.
#[test]
fn a_label_follows() {
    let before = BEFORE.replace("    v = core.add", "    #sum: v = core.add").replace("    node v at", "    node #sum at");
    let after = before.replace("#sum: v", "#total: v");
    let (text, renamed) = follow(&before, &after).unwrap_or_default();
    assert_eq!(renamed, pairs(&[("#sum", "#total")]));
    assert!(text.contains("    node #total at (40, 80)\n") && text.contains("    wire v -> w via"), "{text}");
}

/// A carry renamed in the head of its loop: the wire that names that input of the structure follows; the line in the
/// body that reads it is a line of its own, and the loop is the same.
#[test]
fn a_carry_names_its_input_of_the_structure() {
    let before = "labcraft 1

use core

fn f(xs: [f64]) -> (out: f64) {
    k = 1.0
    total = for x in xs, carry sum = k {
        next sum = core.add(sum, x)
    }
    out = core.neg(total)
}

layout {
    node k at (40, 80)
    wire k -> total.sum via (100, 80)
}
";
    let after = before.replace("carry sum = k", "carry acc = k").replace("next sum = core.add(sum, x)", "next acc = core.add(acc, x)");
    let expected = after.replace("wire k -> total.sum via", "wire k -> total.acc via");
    assert_eq!(follow(before, &after), Some((expected, pairs(&[("sum", "acc")]))));
}

/// A value inside a loop renamed: its body is the same body, since the loop is the same.
#[test]
fn a_value_in_a_body_follows() {
    let before = "labcraft 1

use core

fn f(xs: [f64]) -> (out: f64) {
    total = for x in xs, carry sum = 0.0 {
        dx = core.mul(x, 2.0)
        next sum = core.add(sum, dx)
    }
    out = core.neg(total)
}

layout {
    node dx at (40, 80)
}
";
    let after = before.replace("dx", "step");
    let expected = after.clone();
    let after = after.replace("node step at", "node dx at");
    assert_eq!(follow(before, &after), Some((expected, pairs(&[("dx", "step")]))));
}

/// Moving a line into another body is no rename.
#[test]
fn a_line_moved_to_another_body_is_another_node() {
    let before = "labcraft 1

use core

fn f(xs: [f64]) -> (out: f64) {
    k = 2.0
    total = for x in xs, carry sum = 0.0 {
        next sum = core.add(sum, k)
    }
    out = core.neg(total)
}

layout {
    node k at (40, 80)
}
";
    let after = before
        .replace("    k = 2.0\n", "")
        .replace("carry sum = 0.0 {\n", "carry sum = 0.0 {\n        c = 2.0\n")
        .replace("core.add(sum, k)", "core.add(sum, c)");
    assert_eq!(follow(before, &after), None);
}

/// Comments, a byte order mark and CR LF are kept as written.
#[test]
fn the_rest_of_the_bytes_stays() {
    let before = format!("\u{feff}{}", BEFORE.replace("    w = core.mul", "    // doubled\n    w = core.mul").replace('\n', "\r\n"));
    let after = before.replace("    v = core.add", "    voltage = core.add").replace("// doubled", "// twice");
    let expected = after.replace("node v at", "node voltage at").replace("wire v -> w", "wire voltage -> w");
    assert_eq!(follow(&before, &after), Some((expected, pairs(&[("v", "voltage")]))));
}

/// A name of the signature is no rename here (`signature.rename`), nor a text that does not parse, nor a write without
/// `layout`.
#[test]
fn what_is_not_followed() {
    let after = BEFORE.replace("(out: f64)", "(result: f64)").replace("    out = core.neg(w)", "    result = core.neg(w)");
    assert_eq!(follow(BEFORE, &after), None);
    assert_eq!(follow(BEFORE, &BEFORE.replace("    v = core.add(a, b)", "    voltage = core.add(a, b")), None);
    let bare = BEFORE.split("\nlayout").next().unwrap_or_default().to_owned() + "\n";
    assert_eq!(follow(&bare, &bare.replace("    v = core.add", "    voltage = core.add")), None);
    assert_eq!(follow(BEFORE, BEFORE), None);
}

/// A line told apart only by the line before it: each pass finds the next one (`wI` reads `wI-1`).
#[test]
fn a_chain_of_lines_alike_follows_line_by_line() {
    let lines = |name: &str| -> String {
        (1..=40).map(|i| format!("    {name}{i} = core.neg({})\n", if i == 1 { "a".to_owned() } else { format!("{name}{}", i - 1) })).collect()
    };
    let text = |name: &str, layout: &str| {
        format!(
            "labcraft 1\n\nuse core\n\nfn f(a: f64) -> (out: f64) {{\n{}    out = core.neg({name}40)\n}}\n\nlayout {{\n    node {layout}30 at (40, 80)\n}}\n",
            lines(name)
        )
    };
    let followed = follow(&text("v", "v"), &text("w", "v"));
    assert_eq!(followed.map(|(text, _)| text), Some(text("w", "w")));
}

/// Two structures alike side by side: a line of the first is not the line of the second (the same body, by its place).
#[test]
fn a_line_of_another_structure_alike_is_another_node() {
    let before = "labcraft 1

use core

fn f(xs: [f64]) -> (out: f64) {
    p = for x in xs, carry s = 0.0 {
        a = core.mul(x, 2.0)
        next s = core.add(s, a)
    }
    q = for x in xs, carry s = 0.0 {
        next s = core.add(s, x)
    }
    out = core.add(p, q)
}

layout {
    node a at (10, 10)
}
";
    let after = before.replace("        a = core.mul(x, 2.0)\n        next s = core.add(s, a)\n", "        next s = core.add(s, x)\n").replace(
        "    q = for x in xs, carry s = 0.0 {\n        next s = core.add(s, x)\n",
        "    q = for x in xs, carry s = 0.0 {\n        b = core.mul(x, 2.0)\n        next s = core.add(s, b)\n",
    );
    assert_eq!(follow(before, &after), None);
}

/// A name that a wire gives to an input of a call (`k.b`) is no entry of a node: a rename to it follows.
#[test]
fn a_terminal_of_a_call_does_not_hold_its_name() {
    let before = BEFORE
        .replace("fn f(a: f64, b: f64)", "fn f(p: f64, q: f64)")
        .replace("core.add(a, b)", "core.add(p, q)")
        .replace("    wire v -> w via (120, 80) (120, 100)\n", "    wire v -> w.b via (120, 80) (120, 100)\n");
    let after = before.replace("    v = core.add", "    b = core.add").replace("core.mul(v, 2.0)", "core.mul(b, 2.0)");
    let (text, renamed) = follow(&before, &after).unwrap_or_default();
    assert_eq!(renamed, pairs(&[("v", "b")]));
    assert!(text.contains("    node b at (40, 80)\n") && text.contains("    wire b -> w.b via"), "{text}");
}

/// Names passed from line to line (`v` becomes `w`, `w` becomes `u`): `w` stays bound and keeps its entries, and no line
/// of `v` is the line of `u`.
#[test]
fn a_shift_of_names_moves_nothing() {
    let after = BEFORE.replace(
        "    v = core.add(a, b)\n    w = core.mul(v, 2.0)\n    out = core.neg(w)\n",
        "    w = core.add(a, b)\n    u = core.mul(w, 2.0)\n    out = core.neg(u)\n",
    );
    assert_eq!(follow(BEFORE, &after), None);
}
