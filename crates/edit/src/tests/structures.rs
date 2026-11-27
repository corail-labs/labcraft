//! Structures of M1 step 5a (commands.md 6.7 and 9.6, visual notation 2.6). Each expected text is written by hand from
//! commands.md 9.6 and 9.2, spec 6 and 5.2 (the order of lines: by name among those ready, `collect` and `until` last in a
//! loop body), and notation 2.4 (a value bound to an output takes the name of its node when it leaves it).

use std::collections::BTreeMap;

use labcraft_types::{Diagnostic, Limits};

use super::{FILE, Table, text};
use crate::{EditError, Edited, Files, Judge, Lookup, Op, Point, StructureKind, TypeKind, ValueType, VariantShape, Within, apply};

/// The judge of the table, with the types of some values by their name.
struct Typed(BTreeMap<&'static str, TypeKind>);

impl Judge for Typed {
    fn callable(&self, file: &str, call: &str) -> Lookup {
        Table.callable(file, call)
    }
    fn errors(&self, file: &str, text: &str) -> Vec<Diagnostic> {
        Table.errors(file, text)
    }
    fn value_type(&self, _: &str, _: &str, _: u32, name: &str) -> ValueType {
        let kind = self.0.get(name).cloned().unwrap_or_default();
        let shown = match &kind {
            TypeKind::Other => "f64".to_owned(),
            TypeKind::Error => "error".to_owned(),
            TypeKind::Bool => "bool".to_owned(),
            _ => String::new(),
        };
        ValueType { kind, shown }
    }
}

fn typed(pairs: &[(&'static str, TypeKind)]) -> Typed {
    Typed(pairs.iter().cloned().collect())
}

fn run_with(judge: &dyn Judge, before: &str, op: Op) -> Result<Edited, EditError> {
    let before = text(before);
    let mut files = Files::new();
    files.insert(FILE, &before);
    apply(&files, &op, judge, &Limits::default())
}

#[track_caller]
fn gives_with(judge: &dyn Judge, before: &str, op: Op, after: &str) -> Edited {
    let edited = run_with(judge, before, op).unwrap_or_else(|error| panic!("{error:?}"));
    assert_eq!(edited.texts.get(FILE).map(String::as_str), Some(text(after).as_str()));
    edited
}

#[track_caller]
fn refused_with(judge: &dyn Judge, before: &str, op: Op, code: &str, parameter: &str) -> EditError {
    match run_with(judge, before, op) {
        Ok(edited) => panic!("{code} expected, got {edited:?}"),
        Err(error) => {
            assert_eq!((error.code, error.parameter), (code, Some(parameter)), "{error:?}");
            error
        }
    }
}

fn surround(kind: StructureKind, nodes: &[&str]) -> Op {
    Op::StructureSurround {
        file: FILE.to_owned(),
        kind,
        nodes: nodes.iter().map(|node| (*node).to_owned()).collect(),
        selector: None,
        over: None,
        arm: None,
        within: Within::default(),
        at: None,
    }
}

fn with_selector(op: Op, value: &str) -> Op {
    match op {
        Op::StructureSurround { file, kind, nodes, over, arm, within, at, .. } => {
            Op::StructureSurround { file, kind, nodes, selector: Some(value.to_owned()), over, arm, within, at }
        }
        other => other,
    }
}

fn with_over(op: Op, value: &str) -> Op {
    match op {
        Op::StructureSurround { file, kind, nodes, selector, arm, within, at, .. } => {
            Op::StructureSurround { file, kind, nodes, selector, over: Some(value.to_owned()), arm, within, at }
        }
        other => other,
    }
}

fn renamed(edited: &Edited) -> Vec<(String, String)> {
    edited.renamed.iter().map(|renamed| (renamed.from.clone(), renamed.to.clone())).collect()
}

/// `b` is read outside, by `y`; `a` comes from outside.
const CHAIN: &str = "use core\n\nfn f(x: f64, ok: bool) -> (y: f64) {\n    a = core.add(x, 1)\n    b = core.mul(a, 2)\n    y = core.abs(b)\n}\n";

#[test]
fn a_match_on_a_boolean_has_both_arms_and_its_output_keeps_the_name() {
    let judge = typed(&[("ok", TypeKind::Bool)]);
    let edited = gives_with(
        &judge,
        CHAIN,
        with_selector(surround(StructureKind::Match, &["b"]), "ok"),
        "use core\n\nfn f(x: f64, ok: bool) -> (y: f64) {\n    a = core.add(x, 1)\n    b = match ok {\n        true => {\n            b2 = core.mul(a, 2)\n            yield b2\n        }\n        false => yield ?\n    }\n    y = core.abs(b)\n}\n",
    );
    assert_eq!(renamed(&edited), [("b".to_owned(), "b2".to_owned())]);
    assert_eq!(edited.names.get("structure").map(String::as_str), Some("b"));
    assert_eq!(edited.lists.get("outputs"), Some(&vec!["b".to_owned()]));
    // The nodes go into the arm named.
    let mut op = with_selector(surround(StructureKind::Match, &["b"]), "ok");
    if let Op::StructureSurround { arm, .. } = &mut op {
        *arm = Some("false".to_owned());
    }
    gives_with(
        &judge,
        CHAIN,
        op,
        "use core\n\nfn f(x: f64, ok: bool) -> (y: f64) {\n    a = core.add(x, 1)\n    b = match ok {\n        true => yield ?\n        false => {\n            b2 = core.mul(a, 2)\n            yield b2\n        }\n    }\n    y = core.abs(b)\n}\n",
    );
}

#[test]
fn a_match_on_an_enumeration_has_an_arm_per_variant_and_names_what_they_hold() {
    let state = TypeKind::Enum {
        library: None,
        variants: vec![
            VariantShape { path: "Reading.Value".to_owned(), values: vec!["value".to_owned()] },
            VariantShape { path: "Reading.Missing".to_owned(), values: Vec::new() },
        ],
    };
    let judge = typed(&[("r", state)]);
    let before = "use core\n\nfn f(x: f64, r: Reading) -> () {\n    #beep: sound.beep()\n}\n";
    gives_with(
        &judge,
        before,
        with_selector(surround(StructureKind::Match, &["#beep"]), "r"),
        "use core\n\nfn f(x: f64, r: Reading) -> () {\n    #match_node: _ = match r {\n        Reading.Value(value) => {\n            #beep: sound.beep()\n        }\n        Reading.Missing => {\n        }\n    }\n}\n",
    );
}

#[test]
fn a_match_on_an_enumeration_of_a_library_renames_a_value_named_after_it() {
    // #317: the arms name the variants by their library, so the command adds `use visa`; the value `visa` takes a
    // suffix (N-14). The line of the `match`, named by its label, comes before `visa2` (spec 5.2, by name).
    let state = TypeKind::Enum {
        library: Some("visa".to_owned()),
        variants: vec![
            VariantShape { path: "visa.State.Open".to_owned(), values: Vec::new() },
            VariantShape { path: "visa.State.Closed".to_owned(), values: Vec::new() },
        ],
    };
    let judge = typed(&[("r", state)]);
    let before = "use core\n\nfn f(x: f64, r: visa.State) -> () {\n    visa = core.abs(x)\n    #beep: sound.beep()\n}\n";
    let edited = gives_with(
        &judge,
        before,
        with_selector(surround(StructureKind::Match, &["#beep"]), "r"),
        "use core\nuse visa\n\nfn f(x: f64, r: visa.State) -> () {\n    #match_node: _ = match r {\n        visa.State.Open => {\n            #beep: sound.beep()\n        }\n        visa.State.Closed => {\n        }\n    }\n    visa2 = core.abs(x)\n}\n",
    );
    assert_eq!(renamed(&edited), [("visa".to_owned(), "visa2".to_owned())]);
}

#[test]
fn a_selector_of_another_type_is_refused() {
    let judge = typed(&[("x", TypeKind::Other)]);
    let error = refused_with(&judge, CHAIN, with_selector(surround(StructureKind::Match, &["b"]), "x"), "wire.type_mismatch", "selector");
    assert!(error.message.contains("f64"), "{}", error.message);
    refused_with(&judge, CHAIN, with_selector(surround(StructureKind::Handle, &["b"]), "x"), "wire.type_mismatch", "selector");
    // What a kind needs, and what it does not take.
    refused_with(&judge, CHAIN, surround(StructureKind::Match, &["b"]), "params.invalid", "selector");
    refused_with(&judge, CHAIN, surround(StructureKind::For, &["b"]), "params.invalid", "over");
    refused_with(&judge, CHAIN, with_selector(surround(StructureKind::Loop, &["b"]), "ok"), "params.invalid", "selector");
    // A selector made by the nodes surrounded: the structure would read its own output.
    refused_with(&typed(&[]), CHAIN, with_selector(surround(StructureKind::Match, &["a", "b"]), "a"), "wire.cycle", "selector");
}

#[test]
fn the_nodes_of_one_frame_are_surrounded() {
    let before = "use core\n\nfn f(x: f64, ok: bool) -> (y: f64) {\n    m = match ok {\n        true => {\n            d = core.add(x, 1)\n            yield d\n        }\n        false => yield x\n    }\n    y = core.abs(m)\n}\n";
    refused_with(&typed(&[]), before, surround(StructureKind::Loop, &["d", "y"]), "params.invalid", "nodes");
    // Inside an arm, the structure goes in that arm.
    gives_with(
        &typed(&[]),
        before,
        surround(StructureKind::Loop, &["d"]),
        "use core\n\nfn f(x: f64, ok: bool) -> (y: f64) {\n    m = match ok {\n        true => {\n            d = loop {\n                d2 = core.add(x, 1)\n                collect d2\n                until ?\n            }\n            yield d\n        }\n        false => yield x\n    }\n    y = core.abs(m)\n}\n",
    );
}

#[test]
fn a_value_bound_to_an_output_takes_the_name_of_its_node_inside() {
    // In a loop, the value read outside becomes a `collect`: an array, which the output does not take (`LC3001`).
    let edited = gives_with(
        &typed(&[]),
        CHAIN,
        surround(StructureKind::Loop, &["y"]),
        "use core\n\nfn f(x: f64, ok: bool) -> (y: f64) {\n    a = core.add(x, 1)\n    b = core.mul(a, 2)\n    y = loop {\n        abs = core.abs(b)\n        collect abs\n        until ?\n    }\n}\n",
    );
    assert_eq!(renamed(&edited), [("y".to_owned(), "abs".to_owned())]);
}

/// One error enters `#close`, one leaves it.
const CLOSING: &str =
    "use visa\n\nfn f(r: string, ok: bool) -> (err: error) {\n    s, e1 = visa.open(r)\n    #close: e2 = visa.close(s, err: e1)\n    err = e2\n}\n";

fn errors() -> Typed {
    typed(&[("e1", TypeKind::Error), ("e2", TypeKind::Error), ("ok", TypeKind::Bool)])
}

#[test]
fn the_error_chain_goes_through_the_other_arms_and_through_a_carry() {
    gives_with(
        &errors(),
        CLOSING,
        with_selector(surround(StructureKind::Match, &["#close"]), "ok"),
        "use visa\n\nfn f(r: string, ok: bool) -> (err: error) {\n    s, e1 = visa.open(r)\n    e2 = match ok {\n        true => {\n            #close: e2_2 = visa.close(s, err: e1)\n            yield e2_2\n        }\n        false => yield e1\n    }\n    err = e2\n}\n",
    );
    // In a loop, a carry `err`, numbered since the output has the name: the node reads the carry, its `next` is the
    // error that leaves.
    gives_with(
        &errors(),
        CLOSING,
        surround(StructureKind::Loop, &["#close"]),
        "use visa\n\nfn f(r: string, ok: bool) -> (err: error) {\n    s, e1 = visa.open(r)\n    e2 = loop carry err2 = e1 {\n        #close: e2_2 = visa.close(s, err: err2)\n        next err2 = e2_2\n        until ?\n    }\n    err = e2\n}\n",
    );
}

#[test]
fn an_empty_structure_goes_where_it_is_asked_and_gives_nothing() {
    let before = "fn f(items: [f64], ok: bool) -> () {\n}\n";
    let mut op = with_over(surround(StructureKind::For, &[]), "items");
    if let Op::StructureSurround { at, .. } = &mut op {
        *at = Some(Point { x: 40, y: 80 });
    }
    let edited = gives_with(
        &typed(&[]),
        before,
        op,
        "fn f(items: [f64], ok: bool) -> () {\n    #for_node: _ = for item in items {}\n}\n\nlayout {\n    node #for_node at (40, 80)\n}\n",
    );
    assert_eq!(edited.names.get("structure").map(String::as_str), Some("#for_node"));
    // A range gives `i`; a value whose name does not end in `s` gives `item`.
    gives_with(
        &typed(&[]),
        before,
        with_over(surround(StructureKind::For, &[]), "0..10"),
        "fn f(items: [f64], ok: bool) -> () {\n    #for_node: _ = for i in 0..10 {}\n}\n",
    );
    // Based integers are bounds too, as written (spec 3.4, #228); `0x_1` is none (14.1), and names no value.
    refused_with(&typed(&[]), before, with_over(surround(StructureKind::For, &[]), "0x_1..10"), "params.invalid", "over");
    gives_with(
        &typed(&[]),
        before,
        with_over(surround(StructureKind::For, &[]), "0x10..-0b1"),
        "fn f(items: [f64], ok: bool) -> () {\n    #for_node: _ = for i in 0x10..-0b1 {}\n}\n",
    );
    gives_with(
        &typed(&[("ok", TypeKind::Bool)]),
        before,
        with_selector(surround(StructureKind::Match, &[]), "ok"),
        "fn f(items: [f64], ok: bool) -> () {\n    #match_node: _ = match ok {\n        true => {\n        }\n        false => {\n        }\n    }\n}\n",
    );
    gives_with(
        &typed(&[]),
        before,
        surround(StructureKind::Loop, &[]),
        "fn f(items: [f64], ok: bool) -> () {\n    #loop_node: _ = loop {\n        until ?\n    }\n}\n",
    );
    // In an arm of another structure, by `inside` and `inside_arm`.
    let nested =
        "fn f(items: [f64], ok: bool) -> () {\n    #m: _ = match ok {\n        true => {\n        }\n        false => {\n        }\n    }\n}\n";
    let mut op = surround(StructureKind::Loop, &[]);
    if let Op::StructureSurround { within, .. } = &mut op {
        *within = Within { inside: Some("#m".to_owned()), arm: Some("false".to_owned()) };
    }
    gives_with(
        &typed(&[]),
        nested,
        op,
        "fn f(items: [f64], ok: bool) -> () {\n    #m: _ = match ok {\n        true => {\n        }\n        false => {\n            #loop_node: _ = loop {\n                until ?\n            }\n        }\n    }\n}\n",
    );
    // With nodes, the frame and the place are theirs.
    let mut op = surround(StructureKind::Loop, &["b"]);
    if let Op::StructureSurround { at, .. } = &mut op {
        *at = Some(Point { x: 0, y: 0 });
    }
    refused_with(&typed(&[]), CHAIN, op, "params.invalid", "at");
}

fn connect_arm(from: &str, to: &str, arm: Option<&str>) -> Op {
    Op::WireConnect {
        file: FILE.to_owned(),
        from: from.to_owned(),
        to: to.to_owned(),
        arm: arm.map(str::to_owned),
        replace: false,
        accept_mismatch: false,
        through: None,
    }
}

/// A loop with a carry, a `collect` and its `until` holes, and a match whose second arm yields a hole.
const PARTS: &str = "use core\n\nfn f(x: f64, ok: bool) -> (y: f64) {\n    m = match ok {\n        true => yield x\n        false => yield ?\n    }\n    t, c = loop carry n = 0.0 {\n        z = core.abs(n)\n        collect ?\n        until ?\n    }\n    y = m\n}\n";

#[test]
fn a_wire_reaches_the_yield_the_collect_the_until_and_the_next_of_a_structure() {
    gives_with(
        &typed(&[]),
        PARTS,
        connect_arm("x", "m", Some("false")),
        "use core\n\nfn f(x: f64, ok: bool) -> (y: f64) {\n    m = match ok {\n        true => yield x\n        false => yield x\n    }\n    t, c = loop carry n = 0.0 {\n        z = core.abs(n)\n        collect ?\n        until ?\n    }\n    y = m\n}\n",
    );
    gives_with(
        &typed(&[]),
        PARTS,
        connect_arm("z", "c", None),
        "use core\n\nfn f(x: f64, ok: bool) -> (y: f64) {\n    m = match ok {\n        true => yield x\n        false => yield ?\n    }\n    t, c = loop carry n = 0.0 {\n        z = core.abs(n)\n        collect z\n        until ?\n    }\n    y = m\n}\n",
    );
    gives_with(
        &typed(&[]),
        PARTS,
        connect_arm("ok", "t", None),
        "use core\n\nfn f(x: f64, ok: bool) -> (y: f64) {\n    m = match ok {\n        true => yield x\n        false => yield ?\n    }\n    t, c = loop carry n = 0.0 {\n        z = core.abs(n)\n        collect ?\n        until ok\n    }\n    y = m\n}\n",
    );
    // `next n` has no line yet: one comes.
    gives_with(
        &typed(&[]),
        PARTS,
        connect_arm("z", "t.next.n", None),
        "use core\n\nfn f(x: f64, ok: bool) -> (y: f64) {\n    m = match ok {\n        true => yield x\n        false => yield ?\n    }\n    t, c = loop carry n = 0.0 {\n        z = core.abs(n)\n        next n = z\n        collect ?\n        until ?\n    }\n    y = m\n}\n",
    );
    refused_with(&typed(&[]), PARTS, connect_arm("x", "m", Some("maybe")), "edit.unknown", "arm");
    refused_with(&typed(&[]), PARTS, connect_arm("x", "t.next.q", None), "edit.unknown", "to");
}

#[test]
fn a_node_a_constant_and_a_paste_go_into_the_arm_named() {
    let before = "fn f(x: f64, ok: bool) -> () {\n    #m: _ = match ok {\n        true => {\n        }\n        false => {\n        }\n    }\n}\n";
    let within = Within { inside: Some("#m".to_owned()), arm: Some("false".to_owned()) };
    let op = Op::NodeAdd { file: FILE.to_owned(), call: "core.abs".to_owned(), at: None, name: None, into: None, within: within.clone() };
    gives_with(
        &typed(&[]),
        before,
        op,
        "use core\n\nfn f(x: f64, ok: bool) -> () {\n    #m: _ = match ok {\n        true => {\n        }\n        false => {\n            abs = core.abs()\n        }\n    }\n}\n",
    );
    let op = Op::ConstantAdd { file: FILE.to_owned(), value: "1 V".to_owned(), at: None, name: None, within: within.clone() };
    gives_with(
        &typed(&[]),
        before,
        op,
        "fn f(x: f64, ok: bool) -> () {\n    #m: _ = match ok {\n        true => {\n        }\n        false => {\n            constant = 1 V\n        }\n    }\n}\n",
    );
    let fragment = "labcraft 1\n\nfn clipboard() -> () {\n    k = 2.0\n}\n";
    let op = Op::FragmentPaste { file: FILE.to_owned(), text: fragment.to_owned(), at: Point { x: 0, y: 0 }, within: within.clone() };
    gives_with(
        &typed(&[]),
        before,
        op,
        "fn f(x: f64, ok: bool) -> () {\n    #m: _ = match ok {\n        true => {\n        }\n        false => {\n            k = 2.0\n        }\n    }\n}\n",
    );
    let unknown = Within { inside: Some("#m".to_owned()), arm: Some("1".to_owned()) };
    let op = Op::ConstantAdd { file: FILE.to_owned(), value: "1 V".to_owned(), at: None, name: None, within: unknown };
    let error = refused_with(&typed(&[]), before, op, "edit.unknown", "arm");
    assert_eq!(error.expected, ["true", "false"]);
    let lone = Within { inside: None, arm: Some("true".to_owned()) };
    let op = Op::ConstantAdd { file: FILE.to_owned(), value: "1 V".to_owned(), at: None, name: None, within: lone };
    refused_with(&typed(&[]), before, op, "params.invalid", "arm");
}

fn disconnect_arm(wire: &str, arm: Option<&str>) -> Op {
    Op::WireDisconnect { file: FILE.to_owned(), wire: wire.to_owned(), arm: arm.map(str::to_owned) }
}

/// Every part of `PARTS` wired.
const WIRED: &str = "use core\n\nfn f(x: f64, ok: bool) -> (y: f64) {\n    m = match ok {\n        true => yield x\n        false => yield x\n    }\n    t, c = loop carry n = 0.0 {\n        z = core.abs(n)\n        next n = z\n        collect z\n        until ok\n    }\n    y = m\n}\n";

#[test]
fn a_wire_to_a_part_of_a_structure_is_disconnected() {
    // A `yield` gets a hole, as a `collect` and an `until` do (9.1); the line of a `next` goes.
    gives_with(
        &typed(&[]),
        WIRED,
        disconnect_arm("x -> m", Some("false")),
        "use core\n\nfn f(x: f64, ok: bool) -> (y: f64) {\n    m = match ok {\n        true => yield x\n        false => yield ?\n    }\n    t, c = loop carry n = 0.0 {\n        z = core.abs(n)\n        next n = z\n        collect z\n        until ok\n    }\n    y = m\n}\n",
    );
    gives_with(
        &typed(&[]),
        WIRED,
        disconnect_arm("z -> c", None),
        "use core\n\nfn f(x: f64, ok: bool) -> (y: f64) {\n    m = match ok {\n        true => yield x\n        false => yield x\n    }\n    t, c = loop carry n = 0.0 {\n        z = core.abs(n)\n        next n = z\n        collect ?\n        until ok\n    }\n    y = m\n}\n",
    );
    gives_with(
        &typed(&[]),
        WIRED,
        disconnect_arm("ok -> t", None),
        "use core\n\nfn f(x: f64, ok: bool) -> (y: f64) {\n    m = match ok {\n        true => yield x\n        false => yield x\n    }\n    t, c = loop carry n = 0.0 {\n        z = core.abs(n)\n        next n = z\n        collect z\n        until ?\n    }\n    y = m\n}\n",
    );
    gives_with(
        &typed(&[]),
        WIRED,
        disconnect_arm("z -> t.next.n", None),
        "use core\n\nfn f(x: f64, ok: bool) -> (y: f64) {\n    m = match ok {\n        true => yield x\n        false => yield x\n    }\n    t, c = loop carry n = 0.0 {\n        z = core.abs(n)\n        collect z\n        until ok\n    }\n    y = m\n}\n",
    );
    refused_with(&typed(&[]), WIRED, disconnect_arm("z -> m", Some("true")), "edit.unknown", "wire");
}

#[test]
fn a_yield_wire_is_found_without_its_arm_when_one_arm_yields_the_value() {
    let before = "use core\n\nfn f(x: f64, ok: bool) -> (y: f64) {\n    m = match ok {\n        true => {\n            d = core.abs(x)\n            yield d\n        }\n        false => yield x\n    }\n    y = m\n}\n";
    gives_with(
        &typed(&[]),
        before,
        disconnect_arm("d -> m", None),
        "use core\n\nfn f(x: f64, ok: bool) -> (y: f64) {\n    m = match ok {\n        true => {\n            d = core.abs(x)\n            yield ?\n        }\n        false => yield x\n    }\n    y = m\n}\n",
    );
    // `x` is yielded by one arm, and is no selector: that arm.
    gives_with(
        &typed(&[]),
        before,
        disconnect_arm("x -> m", None),
        "use core\n\nfn f(x: f64, ok: bool) -> (y: f64) {\n    m = match ok {\n        true => {\n            d = core.abs(x)\n            yield d\n        }\n        false => yield ?\n    }\n    y = m\n}\n",
    );
}

/// Commands.md 9.6: a wire between two nodes surrounded keeps its route, counted from the corner of the nodes, and names
/// the values by the names they take inside; the nodes are fixed in the new frame, from that corner. Without a drawing,
/// the structure stands at the corner. Spec 5.2: the `node` entries by name, then the `wire` entries.
#[test]
fn a_route_between_two_nodes_surrounded_is_kept_with_their_names_inside() {
    let before = "use core\n\nfn f(x: f64, ok: bool) -> (y: f64, z: f64) {\n    a = core.abs(x)\n    b = core.abs(a)\n    y = core.abs(b)\n    z = core.abs(a)\n}\n\nlayout {\n    node a at (40, 80)\n    node b at (200, 120)\n    wire a -> b via (120, 80) (120, 136)\n}\n";
    gives_with(
        &typed(&[("ok", TypeKind::Bool)]),
        before,
        with_selector(surround(StructureKind::Match, &["a", "b"]), "ok"),
        "use core\n\nfn f(x: f64, ok: bool) -> (y: f64, z: f64) {\n    a, b = match ok {\n        true => {\n            a2 = core.abs(x)\n            b2 = core.abs(a2)\n            yield (a2, b2)\n        }\n        false => yield (?, ?)\n    }\n    y = core.abs(b)\n    z = core.abs(a)\n}\n\nlayout {\n    node a at (40, 80)\n    node a2 at (0, 0)\n    node b2 at (160, 40)\n    wire a2 -> b2 via (80, 0) (80, 56)\n}\n",
    );
}

/// Commands.md 6.7: a wire out of an arm into an arm of a structure beside it gives the value as an output of its own
/// structure, which the other reads as an outer value (spec 6).
#[test]
fn a_wire_from_an_arm_reaches_the_arm_of_a_structure_beside_it() {
    let before = "use core\n\nfn f(x: f64, ok: bool) -> (y: f64) {\n    m = match ok {\n        true => {\n            d = core.abs(x)\n            yield d\n        }\n        false => yield x\n    }\n    n = match ok {\n        true => {\n            s = core.add()\n            yield s\n        }\n        false => yield x\n    }\n    y = core.add(m, n)\n}\n";
    gives_with(
        &typed(&[]),
        before,
        connect_arm("d", "s.a", None),
        "use core\n\nfn f(x: f64, ok: bool) -> (y: f64) {\n    m, d2 = match ok {\n        true => {\n            d = core.abs(x)\n            yield (d, d)\n        }\n        false => yield (x, ?)\n    }\n    n = match ok {\n        true => {\n            s = core.add(d2)\n            yield s\n        }\n        false => yield x\n    }\n    y = core.add(m, n)\n}\n",
    );
}

/// Spec 5.2: `b` reads `a` and is read by `c`; surrounding `a` and `c` without `b` makes a structure that reads `b`,
/// which reads its output `a`.
#[test]
fn nodes_around_one_left_outside_that_links_them_are_a_cycle() {
    let before = "use core\n\nfn f(x: f64, ok: bool) -> (y: f64) {\n    a = core.abs(x)\n    b = core.abs(a)\n    c = core.add(b, 1)\n    y = core.abs(c)\n}\n";
    refused_with(&typed(&[("ok", TypeKind::Bool)]), before, with_selector(surround(StructureKind::Match, &["a", "c"]), "ok"), "wire.cycle", "nodes");
}

/// Commands.md 9.2: `n` alone names the `until` of `n = loop …`, also when `n` is the output of its `collect`.
#[test]
fn the_name_of_a_loop_alone_names_its_until() {
    let before = "use core\n\nfn f(x: f64, ok: bool) -> (y: f64) {\n    n = loop {\n        w = core.abs(x)\n        collect w\n        until ?\n    }\n    y = core.abs(x)\n}\n";
    gives_with(
        &typed(&[]),
        before,
        connect_arm("ok", "n", None),
        "use core\n\nfn f(x: f64, ok: bool) -> (y: f64) {\n    n = loop {\n        w = core.abs(x)\n        collect w\n        until ok\n    }\n    y = core.abs(x)\n}\n",
    );
}

/// Commands.md 6.7: `wire.disconnect` removes the wire and its entry; spec 11 names a wire to `next n` inside after the
/// loop and its carry.
#[test]
fn a_wire_to_a_next_goes_with_its_entry() {
    let before = format!("{WIRED}\nlayout {{\n    node m at (0, 0)\n    wire z -> t.n via (10, 10)\n}}\n");
    let after = "use core\n\nfn f(x: f64, ok: bool) -> (y: f64) {\n    m = match ok {\n        true => yield x\n        false => yield x\n    }\n    t, c = loop carry n = 0.0 {\n        z = core.abs(n)\n        collect z\n        until ok\n    }\n    y = m\n}\n\nlayout {\n    node m at (0, 0)\n}\n";
    gives_with(&typed(&[]), &before, disconnect_arm("z -> t.next.n", None), after);
}

/// Commands.md 9.6: `outputs` lists the outputs of the structure in order; in a loop the error that leaves is its first
/// (the carry comes first, spec 6.2).
#[test]
fn the_outputs_are_listed_in_the_order_of_the_structure() {
    let before = "use core\nuse visa\n\nfn f(x: f64, r: string) -> (y: f64, err: error) {\n    s, e1 = visa.open(r)\n    a = core.abs(x)\n    #close: e2 = visa.close(s, err: e1)\n    y = core.abs(a)\n    err = e2\n}\n";
    let edited = run_with(&errors(), before, surround(StructureKind::Loop, &["a", "#close"])).unwrap_or_else(|error| panic!("{error:?}"));
    assert_eq!(edited.lists.get("outputs"), Some(&vec!["e2".to_owned(), "a".to_owned()]));
}

/// Commands.md 6.7, "Into a structure that gives nothing" (#379): a wire from inside an arm to the `yield` of that arm of a
/// structure that gives nothing adds an output, named after the value (N-14) as a wire out of an arm names it: that arm
/// yields the value, the other arm a hole. Written by hand from commands.md 6.7 and spec 5.2 (an arm that holds only its
/// `yield` is written on one line).
#[test]
fn a_wire_to_the_yield_of_a_structure_that_gives_nothing_adds_an_output() {
    let before = "use core\n\nfn f(x: f64, ok: bool) -> (y: f64) {\n    #m: _ = match ok {\n        true => {\n            d = core.abs(x)\n        }\n        false => {\n        }\n    }\n    y = x\n}\n";
    let edited = gives_with(
        &typed(&[]),
        before,
        connect_arm("d", "#m", Some("true")),
        "use core\n\nfn f(x: f64, ok: bool) -> (y: f64) {\n    #m: d2 = match ok {\n        true => {\n            d = core.abs(x)\n            yield d\n        }\n        false => yield ?\n    }\n    y = x\n}\n",
    );
    assert_eq!(edited.names.get("output").map(String::as_str), Some("d2"));
    // By its line, the same; an outer value too, which the arm sees.
    gives_with(
        &typed(&[]),
        before,
        connect_arm("x", "@6", Some("false")),
        "use core\n\nfn f(x: f64, ok: bool) -> (y: f64) {\n    #m: x2 = match ok {\n        true => {\n            d = core.abs(x)\n            yield ?\n        }\n        false => yield x\n    }\n    y = x\n}\n",
    );
    // A value of the other arm: the structure would read its own output.
    refused_with(&typed(&[]), before, connect_arm("d", "#m", Some("false")), "wire.cycle", "to");
    // A constant names no output to add.
    let constant = Op::InputSetConstant {
        file: FILE.to_owned(),
        to: "#m".to_owned(),
        arm: Some("true".to_owned()),
        value: "1.0".to_owned(),
        replace: false,
        accept_mismatch: false,
    };
    refused_with(&typed(&[]), before, constant, "params.invalid", "to");
}
