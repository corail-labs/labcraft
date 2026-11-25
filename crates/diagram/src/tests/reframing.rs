//! Nodes dragged over another frame, judged from the drawing (notation 2.4, "Moving nodes into another frame"; #208). Each
//! verdict is worked out by hand from the rule of commands.md 6.7 (`node.reframe`, "What crosses the edges", "Refused"),
//! spec R-3, R-6 and R-17; the programs are those of the cases of `crates/edit` for `node.reframe` (the sheet of the
//! owner, 2026-11-25: A, B, C, D), and a few more for the refusals.

use super::drawing_of;
use crate::{Drawing, Reframing, reframing};

/// The node named `name`, by its index in [`Drawing::placed`].
fn node(drawing: &Drawing, name: &str) -> usize {
    drawing.placed.iter().position(|placed| placed.name == name).unwrap_or_else(|| panic!("no node {name}"))
}

/// The body of the arm `arm` of the structure `structure`, or its one body with no arm.
fn body_of(drawing: &Drawing, structure: &str, arm: &str) -> usize {
    let owner = node(drawing, structure);
    drawing.bodies.iter().position(|body| body.owner == Some(owner) && body.arm == arm).unwrap_or_else(|| panic!("no body {structure} {arm}"))
}

fn judged(main: &str, nodes: &[&str], body: impl Fn(&Drawing) -> usize) -> (Drawing, Reframing) {
    let drawing = drawing_of(main);
    let nodes: Vec<usize> = nodes.iter().map(|name| node(&drawing, name)).collect();
    let body = body(&drawing);
    let verdict = reframing(&drawing, &nodes, body);
    (drawing, verdict)
}

fn code(verdict: &Reframing) -> &'static str {
    match verdict {
        Reframing::Takes { .. } => "takes",
        Reframing::Refused(refusal) => refusal.code,
    }
}

const ARM: &str = "labcraft 1

use core

fn main(x: f64, on: bool) -> (y: f64) {
    o = core.mul(x, 2.0)
    y = match on {
        true => {
            s = core.add(x, o)
            yield s
        }
        false => yield x
    }
}
";

const READ_OUTSIDE: &str = "labcraft 1

use core

fn main(x: f64, on: bool) -> (y: f64, z: f64) {
    o = core.mul(x, 2.0)
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

/// Case A: what `o` reads enters by the edge, and what reads it is in the arm. Case B: read outside too, it leaves by an
/// output of the `match`, no loop on the way. Case D: `z` in the arm would read `y`, the output of its own structure.
#[test]
fn a_frame_takes_nodes_by_what_crosses_its_edge() {
    let (_, a) = judged(ARM, &["o"], |drawing| body_of(drawing, "y", "true"));
    assert_eq!(a, Reframing::Takes { leaving: Vec::new() });
    let (_, b) = judged(READ_OUTSIDE, &["o"], |drawing| body_of(drawing, "y", "true"));
    assert_eq!(b, Reframing::Takes { leaving: Vec::new() });
    let (_, d) = judged(READ_OUTSIDE, &["z"], |drawing| body_of(drawing, "y", "true"));
    assert_eq!(code(&d), "wire.cycle", "{d:?}");
}

/// Case C: `p`, bound to the output `p` of the function, leaves the `for` it goes into: one loop, read by that output.
#[test]
fn a_value_that_would_leave_a_loop_is_named_with_its_loops() {
    let main = "labcraft 1

use core

fn main(xs: [f64], g: f64) -> (t: [f64], p: f64) {
    p = core.mul(g, 2.0)
    t = for x in xs {
        s = core.mul(x, g)
        collect s
    }
}
";
    let (drawing, verdict) = judged(main, &["p"], |drawing| body_of(drawing, "t", ""));
    let Reframing::Takes { leaving } = verdict else { panic!("{verdict:?}") };
    let [one] = leaving.as_slice() else { panic!("{leaving:?}") };
    assert_eq!(one.value, "p");
    assert_eq!(one.exits, vec![body_of(&drawing, "t", "")]);
    assert_eq!(drawing.ports.get(one.reader).map(|port| port.name.as_str()), Some("output p"));
    assert_eq!(drawing.ports.get(one.source).map(|port| port.placed), Some(Some(node(&drawing, "p"))));
}

/// Out of an arm: `b` takes out the value of the arm that it reads; when the arm yields `b` too, the `match` would read
/// what it gives.
#[test]
fn out_of_an_arm_a_node_takes_out_what_it_reads_unless_the_arm_yields_it() {
    let reads_the_arm = "labcraft 1

use core

fn main(x: f64, on: bool) -> (y: f64, z: f64) {
    y = match on {
        true => {
            a = core.mul(x, 2.0)
            b = core.add(a, 1.0)
            yield a
        }
        false => yield x
    }
    z = core.neg(x)
}
";
    let (_, verdict) = judged(reads_the_arm, &["b"], |_| 0);
    assert_eq!(verdict, Reframing::Takes { leaving: Vec::new() });
    let yields_it = reads_the_arm.replace("yield a\n", "yield b\n");
    let (_, verdict) = judged(&yields_it, &["b"], |_| 0);
    assert_eq!(code(&verdict), "wire.cycle", "{verdict:?}");
}

/// Refused: a carry of the new frame hides the `e` that `w` reads (R-3); the new frame binds `o` already; a node moved
/// would hide the outer `o` from `s`, which reads it in the new frame (R-3); the value that `z` waits for would be in an
/// arm (R-17); a structure among the nodes; a `next` out of its loop (R-6).
#[test]
fn a_frame_refuses_what_node_reframe_refuses() {
    let hidden = "labcraft 1

use core

fn main(x: f64, e: f64) -> (y: f64, w: f64) {
    w = core.add(x, e)
    y = loop carry e = 0.0 {
        n = core.add(e, 1.0)
        next e = n
        until true
    }
}
";
    let (_, verdict) = judged(hidden, &["w"], |drawing| body_of(drawing, "y", ""));
    assert!(matches!(&verdict, Reframing::Refused(refusal) if refusal.code == "wire.out_of_scope" && refusal.message.contains("R-3")), "{verdict:?}");
    let taken = "labcraft 1

use core

fn main(x: f64, on: bool) -> (y: f64, z: f64) {
    o = core.mul(x, 2.0)
    y = match on {
        true => {
            o = core.add(x, 1.0)
            yield o
        }
        false => yield x
    }
    z = core.add(o, 1.0)
}
";
    let drawing = drawing_of(taken);
    let outer = drawing.placed.iter().position(|placed| placed.line == 6).unwrap();
    assert_eq!(code(&reframing(&drawing, &[outer], body_of(&drawing, "y", "true"))), "name.taken");
    let hides = "labcraft 1

use core

fn main(x: f64, on: bool) -> (y: f64, z: f64) {
    o = core.mul(x, 2.0)
    y = match on {
        true => {
            s = core.add(x, o)
            yield s
        }
        false => yield x
    }
    z = match on {
        true => {
            o = core.add(x, 1.0)
            yield o
        }
        false => yield x
    }
}
";
    let drawing = drawing_of(hides);
    let inner = drawing.placed.iter().position(|placed| placed.line == 16).unwrap();
    let verdict = reframing(&drawing, &[inner], body_of(&drawing, "y", "true"));
    assert!(
        matches!(&verdict, Reframing::Refused(refusal) if refusal.code == "wire.out_of_scope" && refusal.message.contains("hide")),
        "{verdict:?}"
    );
    let ordered = "labcraft 1

use core

fn main(x: f64, on: bool) -> (y: f64, z: f64) {
    a = core.mul(x, 2.0)
    y = match on {
        true => yield x
        false => yield x
    }
    z = core.neg(x, after: a)
}
";
    let (_, verdict) = judged(ordered, &["a"], |drawing| body_of(drawing, "y", "true"));
    assert!(
        matches!(&verdict, Reframing::Refused(refusal) if refusal.code == "wire.out_of_scope" && refusal.message.contains("R-17")),
        "{verdict:?}"
    );
    let (_, verdict) = judged(READ_OUTSIDE, &["y"], |_| 0);
    assert_eq!(code(&verdict), "params.invalid");
    let next = hidden.replace("n = core.add(e, 1.0)\n        next e = n\n", "next e = core.add(e, 1.0)\n");
    let drawing = drawing_of(&next);
    let carried = drawing.placed.iter().position(|placed| placed.body == body_of(&drawing, "y", "")).unwrap();
    let verdict = reframing(&drawing, &[carried], 0);
    assert!(matches!(&verdict, Reframing::Refused(refusal) if refusal.code == "params.invalid" && refusal.message.contains("R-6")), "{verdict:?}");
}
