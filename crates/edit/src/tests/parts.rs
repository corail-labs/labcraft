//! The parts of a structure and the wires out of a loop, M1 step 5b (commands.md 6.7, decision of the owner 2026-11-18 on
//! `through`). Each expected text is written by hand from commands.md 6.7, spec 6.1 to 6.3 (outputs of a loop: the carries,
//! then the `collect` lines; the type of a carry on its line) and 5.2 (`next`, `collect` and `until` last in a loop body),
//! and N-14 (`w2`, and `w2_2` after a digit).

use std::collections::BTreeMap;

use labcraft_types::{Diagnostic, Limits};

use super::{FILE, Table, text};
use crate::{EditError, Edited, Files, Judge, Lookup, Op, Passage, StructureOption, Through, TypeKind, ValueType, apply};

/// The judge of the table, with the type of some values by their name, written as the check writes it.
struct Typed(BTreeMap<&'static str, (TypeKind, &'static str)>);

impl Judge for Typed {
    fn callable(&self, file: &str, call: &str) -> Lookup {
        Table.callable(file, call)
    }
    fn errors(&self, file: &str, text: &str) -> Vec<Diagnostic> {
        Table.errors(file, text)
    }
    fn value_type(&self, _: &str, _: &str, _: u32, name: &str) -> ValueType {
        match self.0.get(name) {
            Some((kind, shown)) => ValueType { kind: kind.clone(), shown: (*shown).to_owned() },
            None => ValueType::default(),
        }
    }
}

fn judge() -> Typed {
    Typed(
        [("w", (TypeKind::Other, "f64")), ("codes", (TypeKind::Array, "[i32]")), ("xs", (TypeKind::Array, "[f64]")), ("x", (TypeKind::Other, "f64"))]
            .into_iter()
            .collect(),
    )
}

fn run(before: &str, op: Op) -> Result<Edited, EditError> {
    let before = text(before);
    let mut files = Files::new();
    files.insert(FILE, &before);
    apply(&files, &op, &judge(), &Limits::default())
}

#[track_caller]
fn gives(before: &str, op: Op, after: &str) -> Edited {
    let edited = run(before, op).unwrap_or_else(|error| panic!("{error:?}"));
    assert_eq!(edited.texts.get(FILE).map(String::as_str), Some(text(after).as_str()));
    edited
}

#[track_caller]
fn refused(before: &str, op: Op, code: &str, parameter: &str) -> EditError {
    match run(before, op) {
        Ok(edited) => panic!("{code} expected, got {edited:?}"),
        Err(error) => {
            assert_eq!((error.code, error.parameter), (code, Some(parameter)), "{error:?}");
            error
        }
    }
}

fn through(from: &str, to: &str, through: Option<Through>) -> Op {
    Op::WireConnect { file: FILE.to_owned(), from: from.to_owned(), to: to.to_owned(), arm: None, replace: false, accept_mismatch: false, through }
}

fn owned(text: &str) -> String {
    text.to_owned()
}

const FOR: &str = "use core\n\nfn f(xs: [f64]) -> (y: [f64]) {\n    n = for v in xs {\n        w = core.abs(v)\n        collect w\n    }\n    s = core.add()\n    y = n\n}\n";

#[test]
fn a_wire_out_of_a_loop_needs_through() {
    let error = refused(FOR, through("w", "s.a", None), "wire.out_of_scope", "from");
    assert!(error.message.contains("through"), "{}", error.message);
}

#[test]
fn a_carry_takes_the_last_value_out_of_a_loop() {
    // The carry comes before the `collect` among the outputs, its type on its line, its initial value a hole.
    let edited = gives(
        FOR,
        through("w", "s.a", Some(Through::All(Passage::Carry))),
        "use core\n\nfn f(xs: [f64]) -> (y: [f64]) {\n    w2, n = for v in xs, carry w2: f64 = ? {\n        w = core.abs(v)\n        next w2 = w\n        collect w\n    }\n    s = core.add(w2)\n    y = n\n}\n",
    );
    assert_eq!(edited.names.get("output").map(String::as_str), Some("w2"));
}

#[test]
fn a_collect_takes_the_values_out_of_a_loop() {
    // Wired to an output of the function, the output of the loop takes its name (notation 2.4).
    gives(
        "use core\n\nfn f(xs: [f64]) -> (y: [f64], z: [f64]) {\n    n = for v in xs {\n        w = core.abs(v)\n        collect w\n    }\n    y = n\n}\n",
        through("w", "output z", Some(Through::All(Passage::Collect))),
        "use core\n\nfn f(xs: [f64]) -> (y: [f64], z: [f64]) {\n    n, z = for v in xs {\n        w = core.abs(v)\n        collect w\n        collect w\n    }\n    y = n\n}\n",
    );
}

const NESTED: &str = "use core\n\nfn f(xs: [f64]) -> (y: f64) {\n    _ = for v in xs {\n        _ = loop {\n            w = core.abs(v)\n            until ?\n        }\n    }\n    s = core.add()\n    y = s\n}\n";

#[test]
fn each_loop_that_a_wire_leaves_takes_the_value_out() {
    gives(
        NESTED,
        through("w", "s.a", Some(Through::All(Passage::Carry))),
        "use core\n\nfn f(xs: [f64]) -> (y: f64) {\n    w2_2 = for v in xs, carry w2_2: f64 = ? {\n        w2 = loop carry w2: f64 = ? {\n            w = core.abs(v)\n            next w2 = w\n            until ?\n        }\n        next w2_2 = w2\n    }\n    s = core.add(w2_2)\n    y = s\n}\n",
    );
    // One passage per loop, from the inside out: the last value of the inner loop, collected by the outer one.
    gives(
        "use core\n\nfn f(xs: [f64]) -> (y: f64, z: [f64]) {\n    _ = for v in xs {\n        _ = loop {\n            w = core.abs(v)\n            until ?\n        }\n    }\n    y = 0.0\n}\n",
        through("w", "output z", Some(Through::Each(vec![Passage::Carry, Passage::Collect]))),
        "use core\n\nfn f(xs: [f64]) -> (y: f64, z: [f64]) {\n    z = for v in xs {\n        w2 = loop carry w2: f64 = ? {\n            w = core.abs(v)\n            next w2 = w\n            until ?\n        }\n        collect w2\n    }\n    y = 0.0\n}\n",
    );
    refused(NESTED, through("w", "s.a", Some(Through::Each(vec![Passage::Carry]))), "params.invalid", "through");
}

#[test]
fn through_is_refused_where_it_cannot_be() {
    // A wire that leaves no loop.
    refused(FOR, through("xs", "s.a", Some(Through::All(Passage::Carry))), "params.invalid", "through");
    // A `for parallel` takes no carry (spec 6.2).
    let parallel = "use core\n\nfn f(xs: [f64]) -> (y: f64) {\n    _ = for parallel v in xs {\n        w = core.abs(v)\n    }\n    s = core.add()\n    y = s\n}\n";
    refused(parallel, through("w", "s.a", Some(Through::All(Passage::Carry))), "params.invalid", "through");
}

fn add_arm(patterns: &str) -> Op {
    Op::StructureAddArm { file: FILE.to_owned(), structure: owned("y"), patterns: owned(patterns) }
}

const MATCH: &str = "fn f(n: i32) -> (y: f64) {\n    y = match n {\n        1 => yield 1.0\n        _ => yield 2.0\n    }\n}\n";

#[test]
fn an_arm_goes_before_a_last_wildcard_and_yields_holes() {
    let edited = gives(
        MATCH,
        add_arm("2,3..=5"),
        "fn f(n: i32) -> (y: f64) {\n    y = match n {\n        1 => yield 1.0\n        2, 3..=5 => yield ?\n        _ => yield 2.0\n    }\n}\n",
    );
    assert_eq!(edited.names.get("arm").map(String::as_str), Some("2, 3..=5"));
    // Without a last `_`, at the end; in a structure that gives nothing, no `yield`.
    gives(
        "fn f(n: i32) -> () {\n    _ = match n {\n        1 => {\n        }\n    }\n}\n",
        Op::StructureAddArm { file: FILE.to_owned(), structure: owned("@4"), patterns: owned("2") },
        "fn f(n: i32) -> () {\n    _ = match n {\n        1 => {\n        }\n        2 => {\n        }\n    }\n}\n",
    );
}

#[test]
fn an_arm_is_one_arm_of_new_patterns() {
    refused(MATCH, add_arm("1"), "params.invalid", "patterns");
    refused(MATCH, add_arm("2 => { } 3"), "params.invalid", "patterns");
    refused(MATCH, add_arm("2\n3"), "params.invalid", "patterns");
    refused(MATCH, add_arm(""), "params.invalid", "patterns");
    refused(MATCH, Op::StructureAddArm { file: FILE.to_owned(), structure: owned("n"), patterns: owned("2") }, "edit.unknown", "structure");
}

#[test]
fn the_patterns_of_an_arm_change_and_its_nodes_stay() {
    let set = |arm: &str, patterns: &str| Op::StructureSetPatterns {
        file: FILE.to_owned(),
        structure: owned("y"),
        arm: owned(arm),
        patterns: owned(patterns),
    };
    gives(MATCH, set("1", "7..=9"), "fn f(n: i32) -> (y: f64) {\n    y = match n {\n        7..=9 => yield 1.0\n        _ => yield 2.0\n    }\n}\n");
    refused(MATCH, set("1", "_"), "params.invalid", "patterns");
    refused(MATCH, set("3", "4"), "edit.unknown", "arm");
}

fn add_carry(name: &str, initial: Option<&str>, ty: Option<&str>) -> Op {
    Op::StructureAddCarry { file: FILE.to_owned(), structure: owned("#l"), name: owned(name), initial: initial.map(owned), ty: ty.map(owned) }
}

const LOOP: &str = "fn f(x: f64) -> () {\n    #l: _ = loop {\n        until ?\n    }\n}\n";

#[test]
fn a_carry_is_added_with_its_output() {
    gives(LOOP, add_carry("acc", Some("0.0"), None), "fn f(x: f64) -> () {\n    #l: acc = loop carry acc = 0.0 {\n        until ?\n    }\n}\n");
    gives(
        LOOP,
        add_carry("acc", Some("x"), Some("f64")),
        "fn f(x: f64) -> () {\n    #l: acc = loop carry acc: f64 = x {\n        until ?\n    }\n}\n",
    );
    gives(LOOP, add_carry("acc", None, Some("i32")), "fn f(x: f64) -> () {\n    #l: acc = loop carry acc: i32 = ? {\n        until ?\n    }\n}\n");
    // After the other carries, before the `collect` lines among the outputs.
    gives(
        "fn f(x: f64) -> () {\n    #l: a, c = loop carry a = 0 {\n        collect x\n        until ?\n    }\n}\n",
        add_carry("b", Some("1"), None),
        "fn f(x: f64) -> () {\n    #l: a, b, c = loop carry a = 0, carry b = 1 {\n        collect x\n        until ?\n    }\n}\n",
    );
}

#[test]
fn a_carry_needs_a_type_a_new_name_and_a_loop_that_takes_it() {
    refused(LOOP, add_carry("acc", None, None), "params.invalid", "type");
    refused(LOOP, add_carry("x", Some("0"), None), "name.taken", "name");
    refused(LOOP, add_carry("acc", Some("nothing"), None), "edit.unknown", "initial");
    refused(LOOP, add_carry("acc", Some("0"), Some("f64 {")), "params.invalid", "type");
    let parallel = "fn f(xs: [f64]) -> () {\n    #l: _ = for parallel v in xs {}\n}\n";
    refused(parallel, add_carry("acc", Some("0"), None), "params.invalid", "structure");
}

#[test]
fn a_clause_is_named_from_its_value() {
    let add = |over: &str, name: Option<&str>| Op::StructureAddClause {
        file: FILE.to_owned(),
        structure: owned("#l"),
        over: owned(over),
        name: name.map(owned),
    };
    let before = "fn f(xs: [f64], codes: [i32], x: f64) -> () {\n    #l: a = for v in xs, carry a = 0 {}\n}\n";
    let edited = gives(
        before,
        add("codes", None),
        "fn f(xs: [f64], codes: [i32], x: f64) -> () {\n    #l: a = for v in xs, code in codes, carry a = 0 {}\n}\n",
    );
    assert_eq!(edited.names.get("clause").map(String::as_str), Some("code"));
    gives(before, add("0..3", Some("k")), "fn f(xs: [f64], codes: [i32], x: f64) -> () {\n    #l: a = for v in xs, k in 0..3, carry a = 0 {}\n}\n");
    refused(before, add("x", None), "wire.type_mismatch", "over");
    refused(before, add("codes", Some("v")), "name.taken", "name");
    refused(LOOP, add("x", None), "params.invalid", "structure");
}

#[test]
fn a_collect_goes_last_with_its_output() {
    let add = |value: &str, name: Option<&str>| Op::StructureAddCollect {
        file: FILE.to_owned(),
        structure: owned("#l"),
        value: owned(value),
        name: name.map(owned),
    };
    let before = "fn f(xs: [f64]) -> () {\n    #l: a = for v in xs, carry a = 0 {}\n}\n";
    let edited = gives(before, add("v", None), "fn f(xs: [f64]) -> () {\n    #l: a, v2 = for v in xs, carry a = 0 {\n        collect v\n    }\n}\n");
    assert_eq!(edited.names.get("output").map(String::as_str), Some("v2"));
    gives(before, add("1.5", Some("ones")), "fn f(xs: [f64]) -> () {\n    #l: a, ones = for v in xs, carry a = 0 {\n        collect 1.5\n    }\n}\n");
    refused(before, add("nothing", None), "edit.unknown", "value");
}

#[test]
fn a_last_goes_last_with_its_output() {
    let add = |value: &str, name: Option<&str>| Op::StructureAddLast {
        file: FILE.to_owned(),
        structure: owned("#l"),
        value: owned(value),
        name: name.map(owned),
    };
    let before = "fn f(xs: [f64]) -> () {\n    #l: a, b = for v in xs, carry a = 0 {\n        collect v\n    }\n}\n";
    let edited = gives(
        before,
        add("v", None),
        "fn f(xs: [f64]) -> () {\n    #l: a, b, v2 = for v in xs, carry a = 0 {\n        collect v\n        last v\n    }\n}\n",
    );
    assert_eq!(edited.names.get("output").map(String::as_str), Some("v2"));
    let after = "fn f(xs: [f64]) -> () {\n    #l: a, b, v2 = for v in xs, carry a = 0 {\n        collect v\n        last v\n    }\n}\n";
    // `remove_part` names it by its kind and its output: a `collect` is not a `last`.
    gives(after, remove("#l", "last v2"), before);
    gives(after, remove("#l", "collect b"), "fn f(xs: [f64]) -> () {\n    #l: a, v2 = for v in xs, carry a = 0 {\n        last v\n    }\n}\n");
    refused(after, remove("#l", "last b"), "edit.unknown", "part");
    refused(after, remove("#l", "collect v2"), "edit.unknown", "part");
    refused(before, add("nothing", None), "edit.unknown", "value");
}

fn remove(structure: &str, part: &str) -> Op {
    Op::StructureRemovePart { file: FILE.to_owned(), structure: owned(structure), part: owned(part) }
}

#[test]
fn a_part_goes_with_what_it_holds() {
    let before = "use core\n\nfn f(xs: [f64], codes: [i32]) -> (y: f64) {\n    a, b = for v in xs, c in codes, carry a = 0.0 {\n        s = core.add(a, v)\n        next a = s\n        collect c\n    }\n    t = core.abs(b)\n    y = a\n}\n";
    // The carry, its `next` and its output: what read them gets a hole. Canonical form puts `t` before `y`, both ready
    // after the loop, by name (spec 5.2).
    gives(
        before,
        remove("a", "carry a"),
        "use core\n\nfn f(xs: [f64], codes: [i32]) -> (y: f64) {\n    b = for v in xs, c in codes {\n        s = core.add(?, v)\n        collect c\n    }\n    t = core.abs(b)\n    y = ?\n}\n",
    );
    // A clause, and what read it.
    gives(
        before,
        remove("a", "in c"),
        "use core\n\nfn f(xs: [f64], codes: [i32]) -> (y: f64) {\n    a, b = for v in xs, carry a = 0.0 {\n        s = core.add(a, v)\n        next a = s\n        collect ?\n    }\n    t = core.abs(b)\n    y = a\n}\n",
    );
    // A `collect` and its output.
    gives(
        before,
        remove("a", "collect b"),
        "use core\n\nfn f(xs: [f64], codes: [i32]) -> (y: f64) {\n    a = for v in xs, c in codes, carry a = 0.0 {\n        s = core.add(a, v)\n        next a = s\n    }\n    t = core.abs()\n    y = a\n}\n",
    );
    refused(before, remove("a", "in v2"), "edit.unknown", "part");
    refused("fn f(xs: [f64]) -> () {\n    #l: _ = for v in xs {}\n}\n", remove("#l", "in v"), "params.invalid", "part");
}

#[test]
fn an_arm_goes_with_its_nodes() {
    let before = "use core\n\nfn f(n: i32, x: f64) -> (y: f64) {\n    y = match n {\n        1 => {\n            d = core.abs(x)\n            yield d\n        }\n        _ => yield x\n    }\n}\n\nlayout {\n    node d at (10, 10)\n}\n";
    gives(before, remove("y", "1"), "use core\n\nfn f(n: i32, x: f64) -> (y: f64) {\n    y = match n {\n        _ => yield x\n    }\n}\n");
    refused("fn f(n: i32) -> () {\n    #m: _ = match n {\n        _ => {\n        }\n    }\n}\n", remove("#m", "_"), "params.invalid", "part");
}

#[test]
fn an_option_is_one_of_its_kind_of_loop() {
    let set = |structure: &str, option: StructureOption, on: bool| Op::StructureSetOption {
        file: FILE.to_owned(),
        structure: owned(structure),
        option,
        on,
    };
    gives(
        LOOP,
        set("#l", StructureOption::ContinueOnError, true),
        "fn f(x: f64) -> () {\n    #l: _ = loop continue_on_error {\n        until ?\n    }\n}\n",
    );
    let each = "fn f(xs: [f64]) -> () {\n    #l: _ = for v in xs {}\n}\n";
    gives(each, set("#l", StructureOption::Parallel, true), "fn f(xs: [f64]) -> () {\n    #l: _ = for parallel v in xs {}\n}\n");
    refused(each, set("#l", StructureOption::ContinueOnError, true), "params.invalid", "option");
    refused(LOOP, set("#l", StructureOption::Parallel, true), "params.invalid", "option");
    refused(
        "fn f(xs: [f64]) -> () {\n    #l: a = for v in xs, carry a = 0 {}\n}\n",
        set("#l", StructureOption::Parallel, true),
        "params.invalid",
        "on",
    );
}

#[test]
fn a_loop_keeps_its_place_when_its_first_output_changes() {
    // Spec 11: a node is named by the name of its line; a carry before the `collect` gives the line another name, and
    // its entry follows it. A wire entry from the value `n` names the value, which stays.
    let before = "use core\n\nfn f(xs: [f64]) -> (y: [f64]) {\n    n = for v in xs {\n        w = core.abs(v)\n        collect w\n    }\n    s = core.add()\n    y = n\n}\n\nlayout {\n    node n at (40, 80)\n}\n";
    gives(
        before,
        through("w", "s.a", Some(Through::All(Passage::Carry))),
        "use core\n\nfn f(xs: [f64]) -> (y: [f64]) {\n    w2, n = for v in xs, carry w2: f64 = ? {\n        w = core.abs(v)\n        next w2 = w\n        collect w\n    }\n    s = core.add(w2)\n    y = n\n}\n\nlayout {\n    node w2 at (40, 80)\n}\n",
    );
    // Removing the first output: the entry follows the next one.
    let two = "fn f(xs: [f64]) -> () {\n    a, b = for v in xs, carry a = 0 {\n        collect v\n    }\n}\n\nlayout {\n    node a at (40, 80)\n}\n";
    gives(
        two,
        remove("a", "carry a"),
        "fn f(xs: [f64]) -> () {\n    b = for v in xs {\n        collect v\n    }\n}\n\nlayout {\n    node b at (40, 80)\n}\n",
    );
}

#[test]
fn through_takes_out_the_value_that_its_wire_names() {
    // R-3: a line of the loop body hides the input `w`; without `through` the reader sees the input, with it the value
    // inside the loop is meant.
    let hidden = "use core\n\nfn f(xs: [f64], w: f64) -> (y: f64) {\n    _ = for v in xs {\n        w = core.abs(v)\n    }\n    s = core.add()\n    y = s\n}\n";
    gives(
        hidden,
        through("w", "s.a", Some(Through::All(Passage::Carry))),
        "use core\n\nfn f(xs: [f64], w: f64) -> (y: f64) {\n    w2 = for v in xs, carry w2: f64 = ? {\n        w = core.abs(v)\n        next w2 = w\n    }\n    s = core.add(w2)\n    y = s\n}\n",
    );
    // `node.output` by a label: the output gets its name where it is (N-14, `sum` for `core.add`), then the loop takes
    // it out, a carry without a type when the check does not know it.
    let labelled = "use core\n\nfn f(xs: [f64]) -> (y: f64) {\n    _ = for v in xs {\n        #a: _ = core.add(v, 1.0)\n    }\n    s = core.add()\n    y = s\n}\n";
    gives(
        labelled,
        through("#a.result", "s.a", Some(Through::All(Passage::Carry))),
        "use core\n\nfn f(xs: [f64]) -> (y: f64) {\n    sum2 = for v in xs, carry sum2 = ? {\n        #a: sum = core.add(v, 1.0)\n        next sum2 = sum\n    }\n    s = core.add(sum2)\n    y = s\n}\n",
    );
    // An order wire does not leave a structure (R-17).
    refused(FOR, through("w", "s.after", Some(Through::All(Passage::Carry))), "wire.out_of_scope", "through");
}

#[test]
fn a_part_that_reads_what_its_loop_makes_is_a_cycle() {
    let before = "fn f(xs: [f64]) -> () {\n    #l: a, b = for v in xs, carry a = 0 {\n        collect v\n    }\n}\n";
    refused(before, Op::StructureAddClause { file: FILE.to_owned(), structure: owned("#l"), over: owned("b"), name: None }, "wire.cycle", "over");
    refused(before, add_carry_to("#l", "c", Some("a")), "wire.cycle", "initial");
    refused(before, Op::StructureAddCollect { file: FILE.to_owned(), structure: owned("#l"), value: owned("b"), name: None }, "wire.cycle", "value");
}

fn add_carry_to(structure: &str, name: &str, initial: Option<&str>) -> Op {
    Op::StructureAddCarry { file: FILE.to_owned(), structure: owned(structure), name: owned(name), initial: initial.map(owned), ty: None }
}

#[test]
fn a_handle_takes_and_loses_arms_as_a_match_does() {
    let before = "fn f(e: error) -> () {\n    #h: _ = handle e {\n        ok => {\n        }\n        _ => {\n        }\n    }\n}\n";
    let edited = gives(
        before,
        Op::StructureAddArm { file: FILE.to_owned(), structure: owned("#h"), patterns: owned("io.timeout(err)") },
        "fn f(e: error) -> () {\n    #h: _ = handle e {\n        ok => {\n        }\n        io.timeout(err) => {\n        }\n        _ => {\n        }\n    }\n}\n",
    );
    assert_eq!(edited.names.get("arm").map(String::as_str), Some("io.timeout(err)"));
    gives(before, remove("#h", "ok"), "fn f(e: error) -> () {\n    #h: _ = handle e {\n        _ => {\n        }\n    }\n}\n");
}

#[test]
fn a_carry_of_a_type_of_a_library_uses_that_library() {
    let judge = Typed([("d", (TypeKind::Other, "visa.session"))].into_iter().collect());
    let before =
        text("use core\n\nfn f(xs: [f64]) -> (y: f64) {\n    _ = for v in xs {\n        d = core.abs(v)\n    }\n    s = core.add()\n    y = s\n}\n");
    let mut files = Files::new();
    files.insert(FILE, &before);
    let op = through("d", "s.a", Some(Through::All(Passage::Carry)));
    let edited = apply(&files, &op, &judge, &Limits::default()).unwrap_or_else(|error| panic!("{error:?}"));
    let after = edited.texts.get(FILE).cloned().unwrap_or_default();
    assert!(after.contains("use core\nuse visa\n") && after.contains("carry d2: visa.session = ?"), "{after}");
}

#[test]
fn node_output_names_a_structure_or_a_constant_and_the_value_it_names() {
    // A structure gives its outputs by their names, a constant its `value` (9.2, the ports of the canvas).
    let before = "use core\n\nfn f(xs: [f64]) -> (y: f64) {\n    k = 1.0\n    n = for v in xs {\n        #r: _ = core.add(v, 1.0)\n        collect v\n    }\n    s = core.add()\n    y = s\n}\n";
    gives(
        before,
        through("k.value", "s.a", None),
        "use core\n\nfn f(xs: [f64]) -> (y: f64) {\n    k = 1.0\n    n = for v in xs {\n        #r: _ = core.add(v, 1.0)\n        collect v\n    }\n    s = core.add(k)\n    y = s\n}\n",
    );
    refused(before, through("n.nothing", "s.a", None), "edit.unknown", "from");
    // Inside the loop, without `through`: out of scope, never another value of its name.
    refused(before, through("#r.result", "s.a", None), "wire.out_of_scope", "from");
}
