//! `node.reframe` (commands.md 6.7, visual notation 2.4, #208). Each expected text is written by hand from the rule of
//! commands.md 6.7 ("What crosses the edges", "Out of an arm", `through`), N-14 for the names, notation 2.4 for a value
//! bound to an output of the function, and spec 5.2 for the order of the lines (by name among those ready; `collect` and
//! `last` after the others in a loop body). The cases are those of the sheet of the owner (2026-11-25): A, B, C, D.

use std::collections::BTreeMap;

use super::{gives, refused};
use crate::{Op, Passage, Point, Through, Within};

fn reframe(nodes: &[&str], inside: Option<&str>, arm: Option<&str>) -> Op {
    Op::NodeReframe {
        file: super::FILE.to_owned(),
        nodes: nodes.iter().map(|node| (*node).to_owned()).collect(),
        within: Within { inside: inside.map(str::to_owned), arm: arm.map(str::to_owned) },
        at: None,
        through: BTreeMap::new(),
    }
}

fn with_through(op: Op, pairs: &[(&str, Through)]) -> Op {
    match op {
        Op::NodeReframe { file, nodes, within, at, .. } => {
            Op::NodeReframe { file, nodes, within, at, through: pairs.iter().map(|(name, through)| ((*name).to_owned(), through.clone())).collect() }
        }
        other => other,
    }
}

fn with_at(op: Op, at: Point) -> Op {
    match op {
        Op::NodeReframe { file, nodes, within, through, .. } => Op::NodeReframe { file, nodes, within, at: Some(at), through },
        other => other,
    }
}

const ARM: &str = "use core

fn f(x: f64, on: bool) -> (y: f64) {
    o = core.mul(x, 2)
    y = match on {
        true => {
            s = core.add(x, o)
            yield s
        }
        false => yield x
    }
}
";

/// Case A: what the node reads enters by the edge; nothing else changes.
#[test]
fn a_node_moved_into_an_arm_reads_what_enters_it() {
    let edited = gives(
        ARM,
        reframe(&["o"], Some("y"), Some("true")),
        "use core

fn f(x: f64, on: bool) -> (y: f64) {
    y = match on {
        true => {
            o = core.mul(x, 2)
            s = core.add(x, o)
            yield s
        }
        false => yield x
    }
}
",
    );
    assert_eq!(edited.lists.get("nodes"), Some(&vec!["o".to_owned()]));
    assert_eq!(edited.lists.get("outputs"), Some(&Vec::new()));
}

/// With `at`, the node is fixed at the grid point nearest `at`, in its new frame (notation 2.4).
#[test]
fn a_node_moved_with_at_is_fixed_on_the_grid_of_its_new_frame() {
    let before = format!("{ARM}\nlayout {{\n    node o at (40, 80)\n}}\n");
    gives(
        &before,
        with_at(reframe(&["o"], Some("y"), Some("true")), Point { x: 100, y: 50 }),
        "use core

fn f(x: f64, on: bool) -> (y: f64) {
    y = match on {
        true => {
            o = core.mul(x, 2)
            s = core.add(x, o)
            yield s
        }
        false => yield x
    }
}

layout {
    node o at (104, 48)
}
",
    );
}

const READ_OUTSIDE: &str = "use core

fn f(x: f64, on: bool) -> (y: f64, z: f64) {
    o = core.mul(x, 2)
    y = match on {
        true => {
            s = core.add(x, o)
            yield s
        }
        false => yield x
    }
    z = core.add(y, o)
}
";

/// Case B: a value read outside leaves by an output of the `match`, which keeps its name; inside, a suffix; the other arm
/// yields a hole.
#[test]
fn a_value_moved_into_an_arm_and_read_outside_leaves_by_an_output() {
    let edited = gives(
        READ_OUTSIDE,
        reframe(&["o"], Some("y"), Some("true")),
        "use core

fn f(x: f64, on: bool) -> (y: f64, z: f64) {
    y, o = match on {
        true => {
            o2 = core.mul(x, 2)
            s = core.add(x, o2)
            yield (s, o2)
        }
        false => yield (x, ?)
    }
    z = core.add(y, o)
}
",
    );
    assert_eq!(edited.lists.get("outputs"), Some(&vec!["o".to_owned()]));
    assert!(edited.renamed.iter().any(|renamed| renamed.from == "o" && renamed.to == "o2"), "{:?}", edited.renamed);
}

/// Case D: inside the arm, `z` would read `y`, the output of its own structure.
#[test]
fn a_node_moved_into_the_structure_whose_output_it_reads_is_a_cycle() {
    let error = refused(READ_OUTSIDE, reframe(&["z"], Some("y"), Some("true")), "wire.cycle");
    assert!(error.message.contains("its own output"), "{}", error.message);
}

const LOOP: &str = "use core

fn f(xs: [f64], g: f64) -> (t: [f64], p: f64) {
    p = core.mul(g, 2)
    t = for x in xs {
        s = core.mul(x, g)
        collect s
    }
}
";

/// Case C: a value bound to an output of the function leaves the loop as `through` says, a `last`; inside, it takes the
/// name of its node (`product` for `core.mul`, N-14 and notation 2.4).
#[test]
fn a_value_moved_into_a_loop_and_read_outside_leaves_as_through_says() {
    gives(
        LOOP,
        with_through(reframe(&["p"], Some("t"), None), &[("p", Through::All(Passage::Last))]),
        "use core

fn f(xs: [f64], g: f64) -> (t: [f64], p: f64) {
    t, p = for x in xs {
        product = core.mul(g, 2)
        s = core.mul(x, g)
        collect s
        last product
    }
}
",
    );
}

/// Without `through`, a value that would leave a loop is refused, as a wire out of a loop is.
#[test]
fn a_value_that_would_leave_a_loop_needs_through() {
    refused(LOOP, reframe(&["p"], Some("t"), None), "wire.out_of_scope");
}

/// Out of an arm, a node that reads a value of the arm: that value leaves by an output, the arm yielding it.
#[test]
fn a_node_moved_out_of_an_arm_takes_out_what_it_reads_there() {
    gives(
        "use core

fn f(x: f64, on: bool) -> (y: f64) {
    y = match on {
        true => {
            a = core.mul(x, 2)
            b = core.add(a, 1)
            yield a
        }
        false => yield x
    }
}
",
        reframe(&["b"], None, None),
        "use core

fn f(x: f64, on: bool) -> (y: f64) {
    y, a = match on {
        true => {
            a2 = core.mul(x, 2)
            yield (a2, a2)
        }
        false => yield (x, ?)
    }
    b = core.add(a, 1)
}
",
    );
}

/// Out of an arm, a node whose value the arm yields and that reads a value of the arm: the structure would read its own
/// output.
#[test]
fn a_node_moved_out_of_an_arm_that_yields_it_is_a_cycle_when_it_reads_the_arm() {
    refused(
        "use core

fn f(x: f64, on: bool) -> (y: f64) {
    y = match on {
        true => {
            a = core.mul(x, 2)
            b = core.add(a, 1)
            yield b
        }
        false => yield x
    }
}
",
        reframe(&["b"], None, None),
        "wire.cycle",
    );
}

/// A name of the new frame that hides what the node reads (R-3).
#[test]
fn a_node_moved_where_a_carry_hides_what_it_reads_is_refused() {
    let error = refused(
        "use core

fn f(x: f64, e: f64) -> (y: f64, w: f64) {
    w = core.add(x, e)
    y = loop carry e = 0.0 {
        n = core.add(e, 1)
        next e = n
        until ?
    }
}
",
        with_through(reframe(&["w"], Some("y"), None), &[("w", Through::All(Passage::Last))]),
        "wire.out_of_scope",
    );
    assert!(error.message.contains("R-3"), "{}", error.message);
}

/// A value of the same name in the new frame.
#[test]
fn a_node_moved_where_its_name_is_taken_is_refused() {
    refused(
        "use core

fn f(x: f64, on: bool) -> (y: f64, z: f64) {
    o = core.mul(x, 2)
    y = match on {
        true => {
            o = core.add(x, 1)
            yield o
        }
        false => yield x
    }
    z = core.add(o, 1)
}
",
        // The outer `o`, by its line: `labcraft 1`, a blank line, `use core`, a blank line, `fn f`, then it.
        reframe(&["@6"], Some("y"), Some("true")),
        "name.taken",
    );
}

/// The parameters: nodes of one frame, no structure among them, another frame than theirs, `through` for a loop left.
#[test]
fn node_reframe_refuses_what_it_does_not_do() {
    let parameter = |op: Op| refused(ARM, op, "params.invalid").parameter;
    assert_eq!(parameter(reframe(&[], Some("y"), None)), Some("nodes"), "no node");
    assert_eq!(parameter(reframe(&["o", "s"], Some("y"), None)), Some("nodes"), "two frames");
    assert_eq!(parameter(reframe(&["y"], None, None)), Some("nodes"), "a structure");
    assert_eq!(parameter(reframe(&["s"], Some("y"), Some("true"))), Some("inside"), "its own frame");
    let through = with_through(reframe(&["o"], Some("y"), Some("true")), &[("o", Through::All(Passage::Last))]);
    assert_eq!(parameter(through), Some("through"), "no loop left");
    assert_eq!(parameter(reframe(&["o"], Some("o"), None)), Some("inside"), "not a structure");
    refused(ARM, reframe(&["o"], Some("y"), Some("maybe")), "edit.unknown");
}

/// An order wire from a value moved into an arm, to a node that stays outside it, would cross the edge (R-17): refused,
/// and the value is not taken out instead.
#[test]
fn a_node_moved_away_from_what_waits_for_it_is_refused() {
    let error = refused(
        "use core

fn f(x: f64, on: bool) -> (y: f64, z: f64) {
    a = core.mul(x, 2)
    y = match on {
        true => yield x
        false => yield x
    }
    z = core.neg(x, after: a)
}
",
        reframe(&["a"], Some("y"), Some("true")),
        "wire.out_of_scope",
    );
    assert!(error.message.contains("R-17"), "{}", error.message);
}

/// Places far apart that the file gives (a hostile file): the second node would go past the bounds of a coordinate
/// (spec 11), which is refused as for `at`, and nothing overflows.
#[test]
fn nodes_moved_with_at_stay_within_the_bounds_of_a_coordinate() {
    let before = "use core

fn f(x: f64, on: bool) -> (y: f64) {
    o = core.mul(x, 2)
    p = core.add(x, 1)
    y = match on {
        true => {
            s = core.add(o, p)
            yield s
        }
        false => yield x
    }
}

layout {
    node o at (9223372036854775807, 0)
    node p at (-9223372036854775807, 0)
}
";
    let error = refused(before, with_at(reframe(&["o", "p"], Some("y"), Some("true")), Point { x: 0, y: 0 }), "params.invalid");
    assert_eq!(error.parameter, Some("at"), "{error:?}");
}

/// The view a node is shown in stays (spec 11, "View of a node"): without `at`, its entry keeps only the view.
#[test]
fn a_node_moved_keeps_its_view() {
    let before = format!("{ARM}\nlayout {{\n    node o at (40, 80) view standard\n}}\n");
    gives(
        &before,
        reframe(&["o"], Some("y"), Some("true")),
        "use core

fn f(x: f64, on: bool) -> (y: f64) {
    y = match on {
        true => {
            o = core.mul(x, 2)
            s = core.add(x, o)
            yield s
        }
        false => yield x
    }
}

layout {
    node o view standard
}
",
    );
}

/// In the body of the function, a line named as an output gives that output (R-2): a value of that name moved there would
/// wire the output, which only a wire does (notation 2.4), so the move is refused, as `value.rename` refuses that name.
#[test]
fn a_value_named_as_an_output_is_not_moved_into_the_body_of_the_function() {
    refused(
        "use core

fn f(x: f64, on: bool) -> (y: f64, z: f64) {
    y = match on {
        true => {
            z = core.mul(x, 2)
            s = core.add(z, 1)
            yield s
        }
        false => yield x
    }
}
",
        reframe(&["z"], None, None),
        "name.taken",
    );
}
