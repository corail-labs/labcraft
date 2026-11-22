//! The forms of #300: a wire out of a loop beside the reader, an output bound by the node of its name, the reads of an
//! input of the name of an output (R-2).

use super::{FILE, connect, gives, refused, text};
use crate::{Files, Op, Passage, Through, apply};
use labcraft_types::Limits;

fn replace(from: &str, to: &str) -> Op {
    Op::WireConnect {
        file: FILE.to_owned(),
        from: from.to_owned(),
        to: to.to_owned(),
        replace: true,
        accept_mismatch: false,
        arm: None,
        through: None,
    }
}

/// Commands.md 6.7: a value inside a loop beside the structure of the reader is inside a loop (`through` takes it out),
/// not "of another arm"; taken out, it feeds the `match` that the loop reads, a cycle.
#[test]
fn a_value_of_a_loop_beside_the_reader_is_inside_a_loop() {
    let before = "use core\n\nfn f(c: bool, x: f64) -> (y: [f64]) {\n    n = match c {\n        true => yield x\n        false => {\n            m = core.abs(x)\n            yield m\n        }\n    }\n    y = for v in [1.0, 2.0] {\n        w = core.add(v, n)\n        collect w\n    }\n}\n";
    let error = refused(before, connect("w", "m.x"), "wire.out_of_scope");
    assert!(error.message.contains("inside a loop"), "{error:?}");
    let through = Op::WireConnect {
        file: FILE.to_owned(),
        from: "w".to_owned(),
        to: "m.x".to_owned(),
        replace: true,
        accept_mismatch: false,
        arm: None,
        through: Some(Through::All(Passage::Collect)),
    };
    refused(before, through, "wire.cycle");
}

/// Commands.md 6.7, `wire.occupied`: the node that binds an output gives it already; only `replace` passes, and then the
/// wire is there and nothing changes.
#[test]
fn an_output_given_by_its_node_is_occupied() {
    let before = "use visa\n\nfn f(r: string, err: error = none) -> (s: visa.session, err: error) {\n    s, err = visa.open(r, err: err)\n}\n";
    refused(before, connect("s.err", "output err"), "wire.occupied");
    let before = text(before);
    let mut files = Files::new();
    files.insert(FILE, &before);
    let again = apply(&files, &replace("s.err", "output err"), &super::Table, &Limits::default()).unwrap_or_else(|error| panic!("{error:?}"));
    assert!(again.texts.is_empty(), "{:?}", again.texts);
}

/// R-2: reading `err` reads the input. Another value wired to the output `err`, which a node binds, gives that node back
/// the name that inserting it would have given; the reads of the input `err` stay as they are.
#[test]
fn a_node_that_gives_the_output_back_leaves_the_reads_of_the_input() {
    let before = "use visa\n\nfn f(r: string, err: error = none) -> (s: visa.session, t: visa.session, err: error) {\n    s, e1 = visa.open(r, err: err)\n    t, err = visa.open(r, err: e1)\n}\n";
    let edited = gives(
        before,
        replace("e1", "output err"),
        "use visa\n\nfn f(r: string, err: error = none) -> (s: visa.session, t: visa.session, err: error) {\n    s, e1 = visa.open(r, err: err)\n    t, err2 = visa.open(r, err: e1)\n    err = e1\n}\n",
    );
    assert!(edited.renamed.iter().any(|renamed| renamed.from == "err" && renamed.to == "err2"), "{:?}", edited.renamed);
}
