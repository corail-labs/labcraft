//! The operations, command by command (architecture 12.10): each expected text is written by hand, from commands.md 9.1
//! and 6.7, the spec (5.1, 5.2, 11) and visual notation 2.4, never from the output of this crate.

use std::borrow::Cow;

use labcraft_check::{Content, Found, Libraries, LibrarySource, Origin, SourceFile, check};
use labcraft_types::{Diagnostic, Interner, Limits, Severity};

use crate::{Callable, EditError, Edited, Files, Input, Judge, Kind, Lookup, Op, Point, apply, selectable};

struct Nothing;

impl Libraries for Nothing {
    fn find(&self, _: &str, _: Origin) -> Found {
        Found::NotFound
    }
}

fn input(name: &str, required: bool) -> Input {
    Input { name: name.to_owned(), required, variadic: false, named_only: false }
}

/// Signatures from a table, as the documents give them (spec A.1, 5.4), and the errors of the real check, which knows
/// `core` without a library.
struct Table;

impl Judge for Table {
    fn callable(&self, _: &str, call: &str) -> Lookup {
        let function = |library: Option<&str>, inputs: Vec<Input>, outputs: &[&str]| {
            Lookup::Found(Callable {
                kind: Kind::Function,
                library: library.map(str::to_owned),
                inputs,
                outputs: outputs.iter().map(|output| (*output).to_owned()).collect(),
            })
        };
        match call {
            "core.add" | "core.mul" => function(Some("core"), vec![input("a", true), input("b", true)], &["result"]),
            "core.abs" => function(Some("core"), vec![input("x", true)], &["result"]),
            "visa.open" => function(Some("visa"), vec![input("resource", true), input("err", false)], &["session", "err"]),
            "sound.beep" => function(Some("sound"), vec![input("err", false)], &[]),
            "time.now" => function(Some("time"), Vec::new(), &["timestamp"]),
            "string.format" => function(Some("string"), vec![input("template", true), input("values", false)], &["text"]),
            "string.concat" => function(
                Some("string"),
                vec![input("first", true), Input { name: "parts".to_owned(), required: false, variadic: true, named_only: false }],
                &["text"],
            ),
            "g" => function(None, vec![input("a", true), input("b", false)], &["r"]),
            "Point" => Lookup::Found(Callable {
                kind: Kind::Struct,
                library: None,
                inputs: vec![input("x", true), input("y", true)],
                outputs: vec!["Point".to_owned()],
            }),
            "Reading.Value" => {
                Lookup::Found(Callable { kind: Kind::Variant, library: None, inputs: vec![input("", true)], outputs: vec!["Reading".to_owned()] })
            }
            _ => Lookup::Unknown(vec!["core.add".to_owned()]),
        }
    }

    fn errors(&self, file: &str, text: &str) -> Vec<Diagnostic> {
        let files = vec![SourceFile { shown: file.to_owned(), content: Content::Bytes(Cow::Owned(text.as_bytes().to_vec())) }];
        let root = LibrarySource { files, ..LibrarySource::new(String::new(), String::new(), Origin::Project) };
        let mut names = Interner::new();
        let mut report = check(root, &Nothing, &mut names, &Limits::default()).report;
        report.finish();
        report.diagnostics().iter().filter(|diagnostic| diagnostic.severity() == Severity::Error).cloned().collect()
    }
}

const FILE: &str = "f.lcf";

/// `labcraft 1`, then the rest.
fn text(rest: &str) -> String {
    format!("labcraft 1\n\n{rest}")
}

fn run(before: &str, op: Op) -> Result<Edited, EditError> {
    let mut files = Files::new();
    files.insert(FILE, before);
    apply(&files, &op, &Table, &Limits::default())
}

#[track_caller]
fn gives(before: &str, op: Op, after: &str) -> Edited {
    let edited = run(&text(before), op).unwrap_or_else(|error| panic!("{error:?}"));
    assert_eq!(edited.texts.get(FILE).map(String::as_str), Some(text(after).as_str()));
    edited
}

#[track_caller]
fn refused(before: &str, op: Op, code: &str) -> EditError {
    match run(&text(before), op) {
        Ok(edited) => panic!("{code} expected, got {edited:?}"),
        Err(error) => {
            assert_eq!(error.code, code, "{error:?}");
            error
        }
    }
}

fn add(call: &str) -> Op {
    Op::NodeAdd { file: FILE.to_owned(), call: call.to_owned(), at: None, name: None, into: None }
}

fn connect(from: &str, to: &str) -> Op {
    Op::WireConnect { file: FILE.to_owned(), from: from.to_owned(), to: to.to_owned(), replace: false, accept_mismatch: false }
}

fn disconnect(wire: &str) -> Op {
    Op::WireDisconnect { file: FILE.to_owned(), wire: wire.to_owned() }
}

fn delete(nodes: &[&str]) -> Op {
    Op::NodeDelete { file: FILE.to_owned(), nodes: nodes.iter().map(|node| (*node).to_owned()).collect() }
}

const EMPTY: &str = "fn f(x: f64) -> (y: f64) {\n    y = x\n}\n";

// node.add

#[test]
fn a_node_is_named_from_its_function_when_its_output_says_nothing() {
    let edited = gives(EMPTY, add("core.add"), "use core\n\nfn f(x: f64) -> (y: f64) {\n    sum = core.add()\n    y = x\n}\n");
    assert_eq!(edited.names.get("node").map(String::as_str), Some("sum"));
    gives(EMPTY, add("core.abs"), "use core\n\nfn f(x: f64) -> (y: f64) {\n    abs = core.abs()\n    y = x\n}\n");
}

#[test]
fn a_node_is_named_from_its_output_and_numbered_when_taken() {
    let before = "use time\n\nfn f(x: f64) -> (y: f64) {\n    timestamp = time.now()\n    y = x\n}\n";
    gives(before, add("time.now"), "use time\n\nfn f(x: f64) -> (y: f64) {\n    timestamp = time.now()\n    timestamp2 = time.now()\n    y = x\n}\n");
    // An input and an output of the function are taken too, and so is a library that the file uses.
    gives(
        "fn f(sum: f64) -> (y: f64) {\n    y = sum\n}\n",
        add("core.add"),
        "use core\n\nfn f(sum: f64) -> (y: f64) {\n    sum2 = core.add()\n    y = sum\n}\n",
    );
}

#[test]
fn a_node_names_its_first_output_and_leaves_the_others() {
    gives(EMPTY, add("visa.open"), "use visa\n\nfn f(x: f64) -> (y: f64) {\n    session, _ = visa.open()\n    y = x\n}\n");
}

#[test]
fn a_node_without_outputs_gets_a_label() {
    let edited = gives(EMPTY, add("sound.beep"), "use sound\n\nfn f(x: f64) -> (y: f64) {\n    #beep: sound.beep()\n    y = x\n}\n");
    assert_eq!(edited.names.get("node").map(String::as_str), Some("#beep"));
}

#[test]
fn a_construction_gets_a_hole_for_each_value() {
    gives(EMPTY, add("Point"), "fn f(x: f64) -> (y: f64) {\n    point = Point { x: ?, y: ? }\n    y = x\n}\n");
    gives(EMPTY, add("Reading.Value"), "fn f(x: f64) -> (y: f64) {\n    reading = Reading.Value(?)\n    y = x\n}\n");
}

#[test]
fn string_format_is_written_with_the_empty_template() {
    gives(EMPTY, add("string.format"), "use string\n\nfn f(x: f64) -> (y: f64) {\n    text = string.format(\"\")\n    y = x\n}\n");
}

#[test]
fn a_node_given_a_place_is_fixed_there() {
    let op = Op::NodeAdd {
        file: FILE.to_owned(),
        call: "core.add".to_owned(),
        at: Some(Point { x: 40, y: -80 }),
        name: Some("total".to_owned()),
        into: None,
    };
    gives(EMPTY, op, "use core\n\nfn f(x: f64) -> (y: f64) {\n    total = core.add()\n    y = x\n}\n\nlayout {\n    node total at (40, -80)\n}\n");
}

#[test]
fn node_add_refuses_what_it_cannot_name() {
    let named = |name: &str| Op::NodeAdd { file: FILE.to_owned(), call: "core.add".to_owned(), at: None, name: Some(name.to_owned()), into: None };
    refused(EMPTY, named("x"), "name.taken");
    refused(EMPTY, named("match"), "name.invalid");
    refused(EMPTY, named("2x"), "name.invalid");
    let error = refused(EMPTY, add("core.ad"), "edit.unknown");
    assert_eq!(error.expected, ["core.add"]);
    refused(EMPTY, add("core..add"), "params.invalid");
    let far = Op::NodeAdd { file: FILE.to_owned(), call: "core.add".to_owned(), at: Some(Point { x: 1_000_001, y: 0 }), name: None, into: None };
    refused(EMPTY, far, "params.invalid");
}

#[test]
fn a_file_that_does_not_parse_is_not_edited() {
    let error = run(&text("fn f( {\n"), add("core.add")).err();
    assert_eq!(error.map(|error| error.code), Some("file.unparsed"));
    assert_eq!(run("labcraft 1\n\ntype T = f64\n", add("core.add")).err().map(|error| error.code), Some("params.invalid"));
}

#[test]
fn a_text_that_was_not_canonical_comes_out_canonical() {
    gives(
        "fn f(x: f64) -> (y: f64) {\n  y   =   x\n}\n",
        add("core.add"),
        "use core\n\nfn f(x: f64) -> (y: f64) {\n    sum = core.add()\n    y = x\n}\n",
    );
}

// constant.add

fn constant(value: &str) -> Op {
    Op::ConstantAdd { file: FILE.to_owned(), value: value.to_owned(), at: None, name: None }
}

#[test]
fn a_constant_is_a_named_line() {
    gives(EMPTY, constant("240 V"), "fn f(x: f64) -> (y: f64) {\n    constant = 240 V\n    y = x\n}\n");
    gives(EMPTY, constant("Verdict.Pass"), "fn f(x: f64) -> (y: f64) {\n    constant = Verdict.Pass\n    y = x\n}\n");
    gives(EMPTY, constant("[1, 2]"), "fn f(x: f64) -> (y: f64) {\n    constant = [1, 2]\n    y = x\n}\n");
    let named = Op::ConstantAdd { file: FILE.to_owned(), value: "\"MEAS?\"".to_owned(), at: None, name: Some("query".to_owned()) };
    gives(EMPTY, named, "fn f(x: f64) -> (y: f64) {\n    query = \"MEAS?\"\n    y = x\n}\n");
}

#[test]
fn a_constant_is_one_constant_of_the_language() {
    refused(EMPTY, constant("x"), "params.invalid");
    refused(EMPTY, constant("1 +"), "params.invalid");
    refused(EMPTY, constant("1\n    z = 2"), "params.invalid");
    refused(EMPTY, constant("?"), "params.invalid");
    refused(EMPTY, constant("Point { x: 1, y: 2 }"), "params.invalid");
}

// node.add with `into` (commands.md 6.7; decision of the owner, 2026-11-17): the value enters the first input that takes
// it, and the first output that the reader takes goes where the wire went; the lines in the order of spec 5.2.

fn into(call: &str, wire: &str) -> Op {
    Op::NodeAdd { file: FILE.to_owned(), call: call.to_owned(), at: None, name: None, into: Some(wire.to_owned()) }
}

#[test]
fn a_node_goes_into_a_wire_between_two_nodes() {
    let edited = gives(
        TWO,
        into("core.abs", "a -> b.a"),
        "use core\n\nfn f(x: f64) -> (y: f64) {\n    a = core.add(x, 1)\n    abs = core.abs(a)\n    b = core.mul(abs, 2)\n    y = b\n}\n",
    );
    assert_eq!(edited.names.get("node").map(String::as_str), Some("abs"));
    // A wire named without its input, when one wire joins the two; the name of the node numbered when taken.
    gives(
        "use core\n\nfn f(x: f64) -> (y: f64) {\n    abs = core.abs(x)\n    y = abs\n}\n",
        into("core.abs", "x -> abs"),
        "use core\n\nfn f(x: f64) -> (y: f64) {\n    abs2 = core.abs(x)\n    abs = core.abs(abs2)\n    y = abs\n}\n",
    );
}

#[test]
fn a_node_into_the_wire_of_an_output_takes_the_name_of_the_output() {
    // Wiring another node to an output bound by name folds the new one (notation 2.4).
    gives(
        TWO,
        into("core.abs", "b -> y"),
        "use core\n\nfn f(x: f64) -> (y: f64) {\n    a = core.add(x, 1)\n    b = core.mul(a, 2)\n    y = core.abs(b)\n}\n",
    );
}

#[test]
fn a_node_that_nothing_fits_goes_into_no_wire() {
    // No input takes the value, no output goes where it went: nothing is written.
    refused(TWO, into("time.now", "a -> b.a"), "params.invalid");
    refused(TWO, into("sound.beep", "a -> b.a"), "params.invalid");
    refused(TWO, into("core.abs", "a -> nothing"), "edit.unknown");
    refused(TWO, into("visa.nothing", "a -> b.a"), "edit.unknown");
}

// node.delete

const TWO: &str = "use core\n\nfn f(x: f64) -> (y: f64) {\n    a = core.add(x, 1)\n    b = core.mul(a, 2)\n    y = b\n}\n";

#[test]
fn a_positional_input_before_another_gets_a_hole() {
    gives(TWO, delete(&["a"]), "use core\n\nfn f(x: f64) -> (y: f64) {\n    b = core.mul(?, 2)\n    y = b\n}\n");
}

#[test]
fn the_last_positional_input_and_a_named_one_are_left_out() {
    let before = "use core\n\nfn f(x: f64) -> (y: f64) {\n    a = core.add(x, 1)\n    b = core.mul(2, a)\n    c = g(1, b: a)\n    y = b\n}\n";
    gives(before, delete(&["a"]), "use core\n\nfn f(x: f64) -> (y: f64) {\n    b = core.mul(2)\n    c = g(1)\n    y = b\n}\n");
}

#[test]
fn an_output_that_a_deleted_node_gave_loses_its_value() {
    gives(TWO, delete(&["b"]), "use core\n\nfn f(x: f64) -> (y: f64) {\n    a = core.add(x, 1)\n    y = ?\n}\n");
    let bound = "use core\n\nfn f(x: f64) -> (y: f64) {\n    y = core.add(x, 1)\n}\n";
    gives(bound, delete(&["y"]), "use core\n\nfn f(x: f64) -> (y: f64) {}\n");
}

#[test]
fn a_deleted_node_leaves_the_after_of_the_others() {
    let before = "use sound\nuse time\n\nfn f(x: f64) -> (t: timestamp) {\n    #beep: sound.beep()\n    a = time.now()\n    t = time.now(after: (a, #beep))\n}\n";
    gives(
        before,
        delete(&["a"]),
        "use sound\nuse time\n\nfn f(x: f64) -> (t: timestamp) {\n    #beep: sound.beep()\n    t = time.now(after: #beep)\n}\n",
    );
    gives(before, delete(&["a", "#beep"]), "use sound\nuse time\n\nfn f(x: f64) -> (t: timestamp) {\n    t = time.now()\n}\n");
}

#[test]
fn a_deleted_node_takes_its_entries_of_layout() {
    let before = "use core\n\nfn f(x: f64) -> (y: f64) {\n    a = core.add(x, 1)\n    b = core.mul(a, 2)\n    y = b\n}\n\nlayout {\n    node a at (40, 80)\n    node b at (200, 80)\n    wire a -> b via (120, 80)\n}\n";
    gives(
        before,
        delete(&["a"]),
        "use core\n\nfn f(x: f64) -> (y: f64) {\n    b = core.mul(?, 2)\n    y = b\n}\n\nlayout {\n    node b at (200, 80)\n}\n",
    );
    gives(before, delete(&["a", "b"]), "use core\n\nfn f(x: f64) -> (y: f64) {\n    y = ?\n}\n");
}

#[test]
fn a_structure_goes_with_all_it_holds() {
    let before = "use core\n\nfn f(x: f64, c: bool) -> (y: f64) {\n    m = match c {\n        true => {\n            d = core.add(x, 1)\n            yield d\n        }\n        false => yield x\n    }\n    y = core.abs(m)\n}\n";
    gives(before, delete(&["m"]), "use core\n\nfn f(x: f64, c: bool) -> (y: f64) {\n    y = core.abs()\n}\n");
    // A node inside an arm: its readers in the arm get a hole, a yield cannot leave one out.
    gives(
        before,
        delete(&["d"]),
        "use core\n\nfn f(x: f64, c: bool) -> (y: f64) {\n    m = match c {\n        true => yield ?\n        false => yield x\n    }\n    y = core.abs(m)\n}\n",
    );
}

#[test]
fn node_delete_names_what_it_does_not_find() {
    let error = refused(TWO, delete(&["aa"]), "edit.unknown");
    assert_eq!(error.expected.first().map(String::as_str), Some("a"));
    refused(TWO, delete(&[]), "params.invalid");
    let twice = "use core\n\nfn f(x: f64, c: bool) -> (y: f64) {\n    m = match c {\n        true => {\n            d = core.add(x, 1)\n            yield d\n        }\n        false => {\n            d = core.add(x, 2)\n            yield d\n        }\n    }\n    y = m\n}\n";
    refused(twice, delete(&["d"]), "edit.ambiguous");
}

// wire.connect

const OPEN: &str = "use core\n\nfn f(x: f64, z: f64) -> (y: f64) {\n    s = core.add()\n    y = x\n}\n";

#[test]
fn a_wire_goes_by_position_when_it_can_and_by_name_otherwise() {
    gives(OPEN, connect("x", "s.a"), "use core\n\nfn f(x: f64, z: f64) -> (y: f64) {\n    s = core.add(x)\n    y = x\n}\n");
    gives(OPEN, connect("z", "s.b"), "use core\n\nfn f(x: f64, z: f64) -> (y: f64) {\n    s = core.add(b: z)\n    y = x\n}\n");
    let half = "use core\n\nfn f(x: f64, z: f64) -> (y: f64) {\n    s = core.add(x)\n    y = x\n}\n";
    gives(half, connect("z", "s.b"), "use core\n\nfn f(x: f64, z: f64) -> (y: f64) {\n    s = core.add(x, z)\n    y = x\n}\n");
}

#[test]
fn a_wire_fills_a_hole() {
    let holed = "use core\n\nfn f(x: f64, z: f64) -> (y: f64) {\n    s = core.add(?, z)\n    y = x\n}\n";
    gives(holed, connect("x", "s.a"), "use core\n\nfn f(x: f64, z: f64) -> (y: f64) {\n    s = core.add(x, z)\n    y = x\n}\n");
    let point = "fn f(x: f64, z: f64) -> (y: f64) {\n    p = Point { x: ?, y: ? }\n    y = x\n}\n";
    gives(point, connect("z", "p.y"), "fn f(x: f64, z: f64) -> (y: f64) {\n    p = Point { x: ?, y: z }\n    y = x\n}\n");
    let variant = "fn f(x: f64, z: f64) -> (y: f64) {\n    r = Reading.Value(?)\n    y = x\n}\n";
    gives(variant, connect("z", "r.0"), "fn f(x: f64, z: f64) -> (y: f64) {\n    r = Reading.Value(z)\n    y = x\n}\n");
}

#[test]
fn an_occupied_input_is_replaced_only_when_asked() {
    let full =
        "use core\n\nfn f(x: f64, z: f64) -> (y: f64) {\n    s = core.add(x, 1)\n    y = x\n}\n\nlayout {\n    wire x -> s.a via (10, 10)\n}\n";
    let error = refused(full, connect("z", "s.a"), "wire.occupied");
    assert!(error.message.contains("`x`"), "{}", error.message);
    refused(full, connect("z", "s.b"), "wire.occupied");
    let op = Op::WireConnect { file: FILE.to_owned(), from: "z".to_owned(), to: "s.a".to_owned(), replace: true, accept_mismatch: false };
    gives(full, op, "use core\n\nfn f(x: f64, z: f64) -> (y: f64) {\n    s = core.add(z, 1)\n    y = x\n}\n");
}

#[test]
fn a_wire_of_the_wrong_type_is_refused_unless_accepted() {
    let before = "use core\n\nfn f(x: f64, t: string) -> (y: f64) {\n    s = core.abs()\n    y = x\n}\n";
    let error = refused(before, connect("t", "s"), "wire.type_mismatch");
    assert!(error.message.contains("accept_mismatch"), "{}", error.message);
    let op = Op::WireConnect { file: FILE.to_owned(), from: "t".to_owned(), to: "s".to_owned(), replace: false, accept_mismatch: true };
    gives(before, op, "use core\n\nfn f(x: f64, t: string) -> (y: f64) {\n    s = core.abs(t)\n    y = x\n}\n");
}

#[test]
fn a_wire_that_closes_a_cycle_is_refused() {
    let before = "use core\n\nfn f(x: f64) -> (y: f64) {\n    s = core.add(x)\n    t = core.mul(s, 2)\n    y = t\n}\n";
    refused(before, connect("t", "s.b"), "wire.cycle");
    refused(before, connect("s", "s.b"), "wire.cycle");
}

#[test]
fn a_wire_reaches_only_what_its_input_sees() {
    let before = "use core\n\nfn f(x: f64, c: bool) -> (y: f64) {\n    m = match c {\n        true => {\n            d = core.add(x, 1)\n            yield d\n        }\n        false => yield x\n    }\n    n = for v in [1.0, 2.0] {\n        w = core.abs(v)\n        collect w\n    }\n    s = core.add()\n    y = m\n}\n";
    refused(before, connect("w", "s.a"), "wire.out_of_scope");
    // Out of an arm, a wire adds an output to the structure: step 5.
    refused(before, connect("d", "s.a"), "params.invalid");
    refused(before, connect("nothing", "s.a"), "edit.unknown");
}

#[test]
fn an_unnamed_output_gets_a_name_when_wired() {
    let before = "use visa\n\nfn f(r: string) -> (y: f64, err: error) {\n    session, _ = visa.open(r)\n    y = 1.0\n}\n";
    let edited = gives(
        before,
        connect("session.err", "err"),
        "use visa\n\nfn f(r: string) -> (y: f64, err: error) {\n    session, err = visa.open(r)\n    y = 1.0\n}\n",
    );
    assert_eq!(edited.names.get("output").map(String::as_str), Some("err2"));
    assert_eq!(edited.renamed.first().map(|renamed| (renamed.from.as_str(), renamed.to.as_str())), Some(("err2", "err")));
}

#[test]
fn a_value_wired_to_an_output_takes_its_name() {
    let before = "use core\n\nfn f(x: f64) -> (y: f64) {\n    s = core.add(x, x)\n    t = core.abs(s)\n}\n\nlayout {\n    node s at (40, 40)\n}\n";
    // `t` reads `y` now, so `y` comes first (spec 5.2).
    let edited = gives(
        before,
        connect("s", "y"),
        "use core\n\nfn f(x: f64) -> (y: f64) {\n    y = core.add(x, x)\n    t = core.abs(y)\n}\n\nlayout {\n    node y at (40, 40)\n}\n",
    );
    assert_eq!(edited.renamed.first().map(|renamed| (renamed.from.as_str(), renamed.to.as_str())), Some(("s", "y")));
    // An input is written `y = x`.
    gives("fn f(x: f64) -> (y: f64) {}\n", connect("x", "output y"), "fn f(x: f64) -> (y: f64) {\n    y = x\n}\n");
}

#[test]
fn an_output_given_already_is_replaced_only_when_asked() {
    let before = "use core\n\nfn f(x: f64) -> (y: f64) {\n    s = core.abs(x)\n    y = core.add(x, x)\n}\n";
    // `y` is a node and an output: the bare name goes where the value goes, here the node, whose inputs are two.
    refused(before, connect("s", "y"), "edit.ambiguous");
    refused(before, connect("s", "output y"), "wire.occupied");
    let op = Op::WireConnect { file: FILE.to_owned(), from: "s".to_owned(), to: "output y".to_owned(), replace: true, accept_mismatch: false };
    gives(before, op, "use core\n\nfn f(x: f64) -> (y: f64) {\n    sum = core.add(x, x)\n    y = core.abs(x)\n}\n");
}

#[test]
fn a_variadic_value_is_positional() {
    let before = "use string\n\nfn f(a: string, b: string) -> (y: string) {\n    t = string.concat(first: a)\n    y = t\n}\n";
    gives(
        before,
        connect("b", "t.parts.0"),
        "use string\n\nfn f(a: string, b: string) -> (y: string) {\n    t = string.concat(a, b)\n    y = t\n}\n",
    );
    let empty = "use string\n\nfn f(a: string, b: string) -> (y: string) {\n    t = string.concat()\n    y = t\n}\n";
    gives(empty, connect("b", "t.parts.0"), "use string\n\nfn f(a: string, b: string) -> (y: string) {\n    t = string.concat(?, b)\n    y = t\n}\n");
}

#[test]
fn an_order_wire_goes_to_after() {
    let before = "use sound\nuse time\n\nfn f() -> (t: timestamp) {\n    #beep: sound.beep()\n    t = time.now()\n}\n";
    gives(
        before,
        connect("#beep", "t.after"),
        "use sound\nuse time\n\nfn f() -> (t: timestamp) {\n    #beep: sound.beep()\n    t = time.now(after: #beep)\n}\n",
    );
    refused(before, connect("#beep", "output t"), "params.invalid");
}

// wire.disconnect

#[test]
fn an_input_unwired_is_left_out_or_gets_a_hole() {
    let before = "use core\n\nfn f(x: f64, z: f64) -> (y: f64) {\n    s = core.add(x, z)\n    y = s\n}\n";
    gives(before, disconnect("z -> s"), "use core\n\nfn f(x: f64, z: f64) -> (y: f64) {\n    s = core.add(x)\n    y = s\n}\n");
    gives(before, disconnect("x -> s.a"), "use core\n\nfn f(x: f64, z: f64) -> (y: f64) {\n    s = core.add(?, z)\n    y = s\n}\n");
    refused(before, disconnect("x -> s.b"), "edit.unknown");
    refused(before, disconnect("x s"), "params.invalid");
}

#[test]
fn a_value_that_reaches_a_node_twice_is_named_by_its_input() {
    let before = "use core\n\nfn f(x: f64) -> (y: f64) {\n    s = core.add(x, x)\n    y = s\n}\n";
    let error = refused(before, disconnect("x -> s"), "edit.ambiguous");
    assert_eq!(error.expected, ["x -> s.a", "x -> s.b"]);
}

#[test]
fn a_value_unwired_from_an_output_takes_back_the_name_of_its_node() {
    let before = "use core\n\nfn f(x: f64) -> (y: f64) {\n    y = core.add(x, x)\n}\n";
    let edited = gives(before, disconnect("y -> y"), "use core\n\nfn f(x: f64) -> (y: f64) {\n    sum = core.add(x, x)\n}\n");
    assert_eq!(edited.renamed.first().map(|renamed| (renamed.from.as_str(), renamed.to.as_str())), Some(("y", "sum")));
    gives("fn f(x: f64) -> (y: f64) {\n    y = x\n}\n", disconnect("x -> output y"), "fn f(x: f64) -> (y: f64) {}\n");
}

#[test]
fn wiring_back_folds_the_text_again() {
    let start = "use core\n\nfn f(x: f64) -> (y: f64) {\n    sum = core.add(x, x)\n}\n";
    let folded = "use core\n\nfn f(x: f64) -> (y: f64) {\n    y = core.add(x, x)\n}\n";
    gives(start, connect("sum", "y"), folded);
    gives(folded, disconnect("y -> y"), start);
}

// What the independent review found (2026-11-15), each with its text written by hand.

#[test]
fn deleting_two_readers_of_one_call_takes_both_out() {
    let before = "use core\n\nfn f(p: i32) -> (y: i32) {\n    x = core.abs(p)\n    z = core.abs(p)\n    r = g(x, b: z)\n    y = r\n}\n";
    gives(before, delete(&["x", "z"]), "use core\n\nfn f(p: i32) -> (y: i32) {\n    r = g()\n    y = r\n}\n");
}

#[test]
fn a_value_is_not_folded_into_an_output_that_a_body_inside_binds() {
    let before = "use core\n\nfn f(w: f64, p: [f64]) -> (y: f64) {\n    a = core.abs(w)\n    s = for y in p {\n        q = core.add(y, a)\n        collect q\n    }\n}\n";
    gives(
        before,
        connect("a", "output y"),
        "use core\n\nfn f(w: f64, p: [f64]) -> (y: f64) {\n    a = core.abs(w)\n    s = for y in p {\n        q = core.add(y, a)\n        collect q\n    }\n    y = a\n}\n",
    );
}

#[test]
fn coordinates_and_places_past_their_bounds_are_refused() {
    let far = Op::NodeAdd { file: FILE.to_owned(), call: "core.add".to_owned(), at: Some(Point { x: i64::MIN, y: 0 }), name: None, into: None };
    refused(EMPTY, far, "params.invalid");
    let before = "use string\n\nfn f(a: string, b: string) -> (y: string) {\n    t = string.concat(a, b)\n    y = t\n}\n";
    let op =
        Op::WireConnect { file: FILE.to_owned(), from: "b".to_owned(), to: format!("t.parts.{}", usize::MAX), replace: true, accept_mismatch: false };
    refused(before, op, "edit.unknown");
}

#[test]
fn an_order_wire_from_a_label_cannot_close_a_cycle_and_is_written_once() {
    let before = "use sound\n\nfn f() -> () {\n    #a: sound.beep()\n    #b: sound.beep(after: #a)\n}\n";
    refused(before, connect("#b", "#a.after"), "wire.cycle");
    refused(before, connect("#a", "#a.after"), "wire.cycle");
    let edited = run(&text(before), connect("#a", "#b.after")).unwrap_or_else(|error| panic!("{error:?}"));
    assert!(edited.texts.is_empty(), "the node waits for `#a` already: {edited:?}");
}

#[test]
fn an_element_of_an_array_is_unwired_by_its_place() {
    let before = "fn f(x: f64, z: f64) -> (y: [f64]) {\n    k = [x, z]\n    y = k\n}\n";
    gives(before, disconnect("x -> k.0"), "fn f(x: f64, z: f64) -> (y: [f64]) {\n    k = [?, z]\n    y = k\n}\n");
}

#[test]
fn a_deleted_node_bound_to_an_output_takes_its_node_entry() {
    let before = "use core\n\nfn f(x: f64) -> (y: f64) {\n    y = core.abs(x)\n}\n\nlayout {\n    node y at (40, 40)\n}\n";
    gives(before, delete(&["y"]), "use core\n\nfn f(x: f64) -> (y: f64) {}\n");
}

#[test]
fn an_edit_stays_within_the_size_of_a_file() {
    let mut limits = Limits::default();
    limits.max_file_size = 80;
    let mut files = Files::new();
    let before = text(EMPTY);
    files.insert(FILE, &before);
    let error = apply(&files, &add("core.add"), &Table, &limits).err().map(|error| error.code);
    assert_eq!(error, Some("file.too_large"));
}

// view.select: what the selection names (commands.md 9.2)

const SELECTED: &str = "use core\nuse sound\n\nfn f(x: f64, z: f64) -> (y: f64, w: f64) {\n    #beep: sound.beep()\n    s = core.add(x, z, after: #beep)\n    total = core.abs(s)\n    y = total\n    w = core.abs(x)\n}\n";

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|name| (*name).to_owned()).collect()
}

fn select(before: &str, nodes: &[&str], wires: &[&str], terminals: &[&str]) -> Result<(), EditError> {
    let before = text(before);
    let mut files = Files::new();
    files.insert(FILE, &before);
    selectable(&files, FILE, &names(nodes), &names(wires), &names(terminals), &Table, &Limits::default())
}

#[track_caller]
fn not_selectable(before: &str, nodes: &[&str], wires: &[&str], terminals: &[&str], code: &str, parameter: &str) -> EditError {
    match select(before, nodes, wires, terminals) {
        Ok(()) => panic!("{code} expected"),
        Err(error) => {
            assert_eq!((error.code, error.parameter), (code, Some(parameter)), "{error:?}");
            error
        }
    }
}

#[test]
fn a_node_is_selected_by_the_name_of_its_line_or_its_label() {
    assert_eq!(select(SELECTED, &["s", "#beep", "total", "w"], &[], &[]), Ok(()));
    let error = not_selectable(SELECTED, &["s", "totl"], &[], &[], "edit.unknown", "nodes");
    assert_eq!(error.expected, names(&["total", "s", "w"]));
    not_selectable(SELECTED, &["#s"], &[], &[], "edit.unknown", "nodes");
    not_selectable(SELECTED, &["#"], &[], &[], "params.invalid", "nodes");
    let twice = "use core\n\nfn f(x: f64, c: bool) -> (y: f64) {\n    m = match c {\n        true => {\n            d = core.add(x, 1)\n            yield d\n        }\n        false => {\n            d = core.add(x, 2)\n            yield d\n        }\n    }\n    y = m\n}\n";
    not_selectable(twice, &["d"], &[], &[], "edit.ambiguous", "nodes");
}

#[test]
fn a_terminal_is_an_input_or_an_output_of_the_function() {
    assert_eq!(select(SELECTED, &[], &[], &["input x", "input z", "output y", "output w"]), Ok(()));
    let error = not_selectable(SELECTED, &[], &[], &["input q"], "edit.unknown", "terminals");
    assert_eq!(error.expected, names(&["input x", "input z", "output w"]));
    not_selectable(SELECTED, &[], &[], &["output x"], "edit.unknown", "terminals");
    not_selectable(SELECTED, &[], &[], &["x"], "params.invalid", "terminals");
    not_selectable(SELECTED, &[], &[], &["input "], "params.invalid", "terminals");
    not_selectable(SELECTED, &[], &[], &["in x"], "params.invalid", "terminals");
}

#[test]
fn a_wire_is_selected_only_when_it_exists() {
    let wires = ["x -> s", "z -> s.b", "total -> output y", "total -> y", "w -> output w", "#beep -> s.after", "s -> total", "x -> w"];
    assert_eq!(select(SELECTED, &[], &wires, &[]), Ok(()));
    // Both ends exist, but no wire joins them.
    not_selectable(SELECTED, &[], &["z -> total"], &[], "edit.unknown", "wires");
    not_selectable(SELECTED, &[], &["x -> s.b"], &[], "edit.unknown", "wires");
    not_selectable(SELECTED, &[], &["x -> output y"], &[], "edit.unknown", "wires");
    not_selectable(SELECTED, &[], &["#beep -> total.after"], &[], "edit.unknown", "wires");
    not_selectable(SELECTED, &[], &["x s"], &[], "params.invalid", "wires");
    not_selectable(SELECTED, &[], &["x -> "], &[], "params.invalid", "wires");
    // The first name that fails is the one given.
    not_selectable(SELECTED, &["s"], &["x -> s", "q -> s"], &["input x"], "edit.unknown", "wires");
}

#[test]
fn selecting_changes_nothing_and_needs_the_file() {
    let mut files = Files::new();
    let before = text(SELECTED);
    files.insert(FILE, &before);
    let error = selectable(&files, "g.lcf", &[], &[], &[], &Table, &Limits::default()).err().map(|error| error.code);
    assert_eq!(error, Some("path.not_found"));
    assert_eq!(selectable(&files, FILE, &[], &names(&["x -> s"]), &[], &Table, &Limits::default()), Ok(()));
    assert_eq!(files.get(FILE), Some(before.as_str()));
    // The same wire is still there to unwire.
    assert!(apply(&files, &disconnect("x -> s"), &Table, &Limits::default()).is_ok());
}

// node.move, node.place, node.set_view, layout.arrange: the texts written by hand from spec 11 (one entry per node, the
// order of 5.2), commands.md 6.7 and 9.2, notation 2.4, and the decisions of the owner of 2026-11-17 (a node that the
// section cannot name gets a label from the name of its line or of what it calls).

/// The places of a drawing, as `engine` would give them, the rest from [`Table`].
struct Placing(Vec<crate::Place>);

impl Judge for Placing {
    fn callable(&self, file: &str, call: &str) -> Lookup {
        Table.callable(file, call)
    }
    fn errors(&self, file: &str, text: &str) -> Vec<Diagnostic> {
        Table.errors(file, text)
    }
    fn places(&self, _: &str, _: &str) -> Vec<crate::Place> {
        self.0.clone()
    }
}

fn place_of(line: u32, name: &str, x: i64, y: i64) -> crate::Place {
    crate::Place { line, name: name.to_owned(), at: Point { x, y } }
}

fn run_with(before: &str, op: Op, places: Vec<crate::Place>) -> Result<Edited, EditError> {
    let before = text(before);
    let mut files = Files::new();
    files.insert(FILE, &before);
    apply(&files, &op, &Placing(places), &Limits::default())
}

#[track_caller]
fn moves(before: &str, op: Op, places: Vec<crate::Place>, after: &str) -> Edited {
    let edited = run_with(before, op, places).unwrap_or_else(|error| panic!("{error:?}"));
    assert_eq!(edited.texts.get(FILE).map(String::as_str), Some(text(after).as_str()));
    edited
}

fn moving(nodes: &[&str], x: i64, y: i64) -> Op {
    Op::NodeMove { file: FILE.to_owned(), nodes: nodes.iter().map(|node| (*node).to_owned()).collect(), by: Point { x, y } }
}

fn placing(node: &str, x: i64, y: i64) -> Op {
    Op::NodePlace { file: FILE.to_owned(), node: node.to_owned(), at: Point { x, y } }
}

fn viewing(nodes: &[&str], view: Option<labcraft_graph::View>) -> Op {
    Op::NodeSetView { file: FILE.to_owned(), nodes: nodes.iter().map(|node| (*node).to_owned()).collect(), view }
}

fn arranging(nodes: Option<&[&str]>, wires: &[&str]) -> Op {
    Op::LayoutArrange {
        file: FILE.to_owned(),
        nodes: nodes.map(|nodes| nodes.iter().map(|node| (*node).to_owned()).collect()),
        wires: wires.iter().map(|wire| (*wire).to_owned()).collect(),
        terminals: Vec::new(),
    }
}

/// [`TWO`] with a `layout` section.
fn placed(entries: &str) -> String {
    format!("use core\n\nfn f(x: f64) -> (y: f64) {{\n    a = core.add(x, 1)\n    b = core.mul(a, 2)\n    y = b\n}}\n\nlayout {{\n{entries}}}\n")
}

const PLACED: &str = "    node a at (40, 80) view compact\n    node b at (200, 80)\n    wire a -> b via (120, 80)\n";

#[test]
fn moving_a_fixed_node_changes_its_entry_alone() {
    let edited = moves(
        &placed(PLACED),
        moving(&["a"], 8, -16),
        vec![],
        &placed("    node a at (48, 64) view compact\n    node b at (200, 80)\n    wire a -> b via (120, 80)\n"),
    );
    assert_eq!(edited.lists.get("nodes"), Some(&vec!["a".to_owned()]));
    // Two nodes at once, each by the same; up and left of the origin, the coordinates are negative (spec 11).
    moves(
        &placed(PLACED),
        moving(&["b", "a"], -64, -96),
        vec![],
        &placed("    node a at (-24, -16) view compact\n    node b at (136, -16)\n    wire a -> b via (120, 80)\n"),
    );
}

#[test]
fn moving_a_free_node_fixes_it_from_where_it_is_drawn() {
    // `a` starts on line 6 of the text: `labcraft 1`, a blank, `use core`, a blank, the header.
    let edited = moves(TWO, moving(&["a"], 16, 8), vec![place_of(6, "a", 40, 80), place_of(7, "b", 200, 80)], &placed("    node a at (56, 88)\n"));
    assert!(edited.renamed.is_empty());
    // A node that the drawing does not place has nowhere to move from.
    let error = run_with(TWO, moving(&["a"], 16, 8), vec![]).err().map(|error| (error.code, error.parameter));
    assert_eq!(error, Some(("params.invalid", Some("nodes"))));
}

#[test]
fn a_move_by_nothing_changes_nothing() {
    let edited = run_with(&placed(PLACED), moving(&["a"], 0, 0), vec![]).unwrap_or_else(|error| panic!("{error:?}"));
    assert!(edited.texts.is_empty());
    let edited = run_with(TWO, moving(&["a"], 0, 0), vec![]).unwrap_or_else(|error| panic!("{error:?}"));
    assert!(edited.texts.is_empty());
}

/// `d` in two arms; `first` and `second` are what stands before each.
fn twice(first: &str, second: &str, layout: &str) -> String {
    format!(
        "use core\n\nfn f(x: f64, c: bool) -> (y: f64) {{\n    m = match c {{\n        true => {{\n            {first}d = core.add(x, 1)\n            yield d\n        }}\n        false => {{\n            {second}d = core.add(x, 2)\n            yield d\n        }}\n    }}\n    y = m\n}}\n{layout}"
    )
}

#[test]
fn a_node_that_the_section_cannot_name_gets_a_label() {
    // `d` of the first arm starts on line 8, that of the second on line 12: a name of two bodies, so `#d`, then `#d2`.
    let edited = moves(
        &twice("", "", ""),
        moving(&["@8"], 8, 0),
        vec![place_of(8, "@8", 40, 60), place_of(12, "@12", 40, 60)],
        &twice("#d: ", "", "\nlayout {\n    node #d at (48, 60)\n}\n"),
    );
    assert_eq!(edited.lists.get("nodes"), Some(&vec!["#d".to_owned()]));
    assert_eq!(edited.renamed, vec![crate::Renamed { file: FILE.to_owned(), from: "@8".to_owned(), to: "#d".to_owned() }]);
    let edited = moves(&twice("#d: ", "", ""), placing("@12", 0, 0), vec![], &twice("#d: ", "#d2: ", "\nlayout {\n    node #d2 at (0, 0)\n}\n"));
    assert_eq!(edited.names.get("node").map(String::as_str), Some("#d2"));
    // A call with no output is labelled from what it calls.
    moves(
        "use sound\n\nfn f(x: f64) -> (y: f64) {\n    sound.beep()\n    y = x\n}\n",
        placing("@6", 40, 40),
        vec![],
        "use sound\n\nfn f(x: f64) -> (y: f64) {\n    #beep: sound.beep()\n    y = x\n}\n\nlayout {\n    node #beep at (40, 40)\n}\n",
    );
}

#[test]
fn the_nodes_of_one_move_are_in_one_frame() {
    let both = twice("", "", "");
    let error = run_with(&both, moving(&["@8", "@12"], 8, 0), vec![]).err().map(|error| (error.code, error.parameter));
    assert_eq!(error, Some(("params.invalid", Some("nodes"))));
    let error = run_with(&both, moving(&["m", "@8"], 8, 0), vec![]).err().map(|error| (error.code, error.parameter));
    assert_eq!(error, Some(("params.invalid", Some("nodes"))));
    refused(&both, moving(&["d"], 8, 0), "edit.ambiguous");
    refused(&both, moving(&[], 8, 0), "params.invalid");
}

#[test]
fn placing_a_node_where_it_is_changes_nothing() {
    let edited = run_with(&placed(PLACED), placing("b", 200, 80), vec![]).unwrap_or_else(|error| panic!("{error:?}"));
    assert!(edited.texts.is_empty());
    assert_eq!(edited.names.get("node").map(String::as_str), Some("b"));
    let error = run_with(&placed(PLACED), placing("b", 2_000_000, 80), vec![]).err().map(|error| (error.code, error.parameter));
    assert_eq!(error, Some(("params.invalid", Some("at"))));
}

#[test]
fn a_view_is_written_on_the_entry_of_its_node() {
    use labcraft_graph::View;
    gives(
        &placed(PLACED),
        viewing(&["b"], Some(View::Standard)),
        &placed("    node a at (40, 80) view compact\n    node b at (200, 80) view standard\n    wire a -> b via (120, 80)\n"),
    );
    gives(&placed(PLACED), viewing(&["a"], None), &placed("    node a at (40, 80)\n    node b at (200, 80)\n    wire a -> b via (120, 80)\n"));
    // A free node gets an entry with its view alone, which `default` takes away with the section.
    let compact = placed("    node a view compact\n");
    gives(TWO, viewing(&["a"], Some(View::Compact)), &compact);
    gives(&compact, viewing(&["a"], None), TWO);
}

#[test]
fn arranging_removes_the_entries_of_what_it_arranges() {
    // The whole function: every place and every route; a view stays.
    gives(&placed(PLACED), arranging(None, &[]), &placed("    node a view compact\n"));
    // A node alone: its place, not the route of a wire to a node that is not arranged.
    gives(&placed(PLACED), arranging(Some(&["b"]), &[]), &placed("    node a at (40, 80) view compact\n    wire a -> b via (120, 80)\n"));
    // Two nodes: the wires between them too.
    gives(&placed(PLACED), arranging(Some(&["a", "b"]), &[]), &placed("    node a view compact\n"));
    // A wire: its route alone.
    gives(&placed(PLACED), arranging(None, &["a -> b"]), &placed("    node a at (40, 80) view compact\n    node b at (200, 80)\n"));
    refused(&placed(PLACED), arranging(None, &["x -> b"]), "edit.unknown");
    refused(&placed(PLACED), arranging(Some(&["c"]), &[]), "edit.unknown");
    // Nothing to arrange: nothing changes.
    let edited = run(&text(TWO), arranging(None, &[])).unwrap_or_else(|error| panic!("{error:?}"));
    assert!(edited.texts.is_empty());
}

// What the independent review found (2026-11-17), each with its text written by hand.

#[test]
fn a_structure_with_no_name_gets_a_label_that_is_no_reserved_word() {
    // `_ = match …` starts on line 4, under the header; `match` is a reserved word, so its label is `#match_node`.
    let before = "fn f(x: f64, c: bool) -> (y: f64) {\n    _ = match c {\n        true => yield x\n        false => yield x\n    }\n    y = x\n}\n";
    moves(
        before,
        placing("@4", 40, 40),
        vec![],
        "fn f(x: f64, c: bool) -> (y: f64) {\n    #match_node: _ = match c {\n        true => yield x\n        false => yield x\n    }\n    y = x\n}\n\nlayout {\n    node #match_node at (40, 40)\n}\n",
    );
}

#[test]
fn a_node_on_the_line_of_its_structure_moves_from_its_own_place() {
    // `scaled` and `s` start on line 6; the drawing places the loop at (40, 40) in the frame of the function, and `s`
    // at (16, 24) in the frame of the loop.
    let before = "use core\n\nfn f(readings: [f64]) -> (out: [f64]) {\n    scaled = for x in readings { s = core.mul(x, 2.0)\n        collect s\n    }\n    out = scaled\n}\n";
    let edited = run_with(before, moving(&["s"], 8, 0), vec![place_of(6, "scaled", 40, 40), place_of(6, "s", 16, 24)]);
    let text = edited.unwrap_or_else(|error| panic!("{error:?}")).texts.get(FILE).cloned().unwrap_or_default();
    assert!(text.ends_with("\nlayout {\n    node s at (24, 24)\n}\n"), "{text}");
}

/// A name that is an input and an output of the function (R-2): reading it reads the input, and the line `err = …` binds
/// the output, so no wire closes a cycle through that line, and `from: err` is the input. The expected texts are written
/// by hand from spec R-2 and 5.2.
#[test]
fn reading_a_name_that_is_an_input_and_an_output_reads_the_input() {
    let before = "use visa\n\nfn f(r: string, err: error = none) -> (s: visa.session, err: error) {\n    s, e1 = visa.open(r, err: err)\n    t, e2 = visa.open(r, err: e1)\n    err = e2\n}\n";
    let replace = |from: &str, to: &str| Op::WireConnect {
        file: FILE.to_owned(),
        from: from.to_owned(),
        to: to.to_owned(),
        replace: true,
        accept_mismatch: false,
    };
    // `visa.open` reads the input `err`, not the line `err = e2`, which depends on `t`: no cycle; the wire is there
    // already, and nothing changes.
    let again = run(&text(before), replace("e1", "t.err")).unwrap_or_else(|error| panic!("{error:?}"));
    assert!(again.texts.is_empty(), "{:?}", again.texts);
    gives(
        before,
        replace("err", "t.err"),
        "use visa\n\nfn f(r: string, err: error = none) -> (s: visa.session, err: error) {\n    s, e1 = visa.open(r, err: err)\n    t, e2 = visa.open(r, err: err)\n    err = e2\n}\n",
    );
    // A real cycle is still one.
    refused(before, replace("e2", "s.err"), "wire.cycle");
}

/// An output of a node named `node.output` is seen from the reader as a value is: from inside a loop to outside, it is
/// refused, and nothing is written (commands.md 6.7, `wire.connect`).
#[test]
fn an_output_named_by_its_node_is_seen_as_a_value_is() {
    let before = "use core\nuse time\n\nfn f(x: f64) -> (y: f64) {\n    t0 = time.now()\n    n = for v in [1.0, 2.0] {\n        w, _ = core.add(v, x)\n        collect w\n    }\n    y = x\n}\n";
    refused(before, connect("w.result", "t0.after"), "wire.out_of_scope");
}

/// The wires as a drawing would route them, written by hand for each text: `Table` otherwise.
struct Drawn {
    /// For a text that holds the first, these wires.
    by_text: Vec<(&'static str, Vec<crate::WireDrawn>)>,
}

impl Judge for Drawn {
    fn callable(&self, file: &str, call: &str) -> Lookup {
        Table.callable(file, call)
    }

    fn errors(&self, file: &str, text: &str) -> Vec<Diagnostic> {
        Table.errors(file, text)
    }

    fn wires(&self, _: &str, text: &str) -> Vec<crate::WireDrawn> {
        self.by_text.iter().find(|(marker, _)| text.contains(marker)).map(|(_, wires)| wires.clone()).unwrap_or_default()
    }
}

fn drawn(name: &str, path: &[(i64, i64)]) -> crate::WireDrawn {
    let value = name.split(" -> ").next().unwrap_or_default().to_owned();
    crate::WireDrawn { name: name.to_owned(), value, body: 0, path: path.iter().map(|&(x, y)| (x * 100, y * 100)).collect() }
}

fn route(wire: &str, via: &[(i64, i64)]) -> Op {
    Op::WireRoute { file: FILE.to_owned(), wire: wire.to_owned(), via: via.iter().map(|&(x, y)| Point { x, y }).collect() }
}

fn run_judged(judge: &dyn Judge, before: &str, op: Op) -> Result<Edited, EditError> {
    let before = text(before);
    let mut files = Files::new();
    files.insert(FILE, &before);
    apply(&files, &op, judge, &Limits::default())
}

const BRANCHED: &str = "use core\n\nfn f(x: f64) -> (y: f64, z: f64) {\n    a = core.abs(x)\n    y = core.abs(a)\n    z = core.abs(a)\n}\n";

/// Two readers of `a`: the trunk to `y` straight at height 40, and `z` below it.
fn two_readers() -> Vec<crate::WireDrawn> {
    vec![
        drawn("x -> a.x", &[(0, 40), (40, 40)]),
        drawn("a -> y.x", &[(100, 40), (200, 40)]),
        drawn("a -> z.x", &[(100, 40), (140, 40), (140, 120), (200, 120)]),
    ]
}

/// `wire.route` writes the `wire … via` entry of a wire, as short as the section can name it, in place of the one it
/// had (spec 11, commands.md 6.7).
#[test]
fn a_wire_routed_gets_its_entry() {
    let judge = Drawn { by_text: vec![("fn f", two_readers())] };
    let edited = run_judged(&judge, BRANCHED, route("a -> y.x", &[(120, 16)])).unwrap();
    let routed = text(&format!("{BRANCHED}\nlayout {{\n    wire a -> y via (120, 16)\n}}\n"));
    assert_eq!(edited.texts.get(FILE).map(String::as_str), Some(routed.as_str()));
    assert_eq!(edited.names.get("wire").map(String::as_str), Some("a -> y.x"));
    // Named without the input, it is the one wire from `a` to `y`; a second route takes the place of the first.
    let again = format!("{BRANCHED}\nlayout {{\n    wire a -> y via (120, 16)\n}}\n");
    let edited = run_judged(&judge, &again, route("a -> y", &[(120, 8), (160, 8)])).unwrap();
    let twice = text(&format!("{BRANCHED}\nlayout {{\n    wire a -> y via (120, 8) (160, 8)\n}}\n"));
    assert_eq!(edited.texts.get(FILE).map(String::as_str), Some(twice.as_str()));
}

#[test]
fn wire_route_refuses_what_it_cannot_route() {
    let judge = Drawn { by_text: vec![("fn f", two_readers())] };
    let refused_with = |op: Op, code: &str| {
        let error = run_judged(&judge, BRANCHED, op).unwrap_err();
        assert_eq!(error.code, code, "{error:?}");
        error
    };
    assert_eq!(refused_with(route("a -> y.x", &[]), "params.invalid").parameter, Some("via"));
    assert_eq!(refused_with(route("a -> y.x", &[(2_000_000, 0)]), "params.invalid").parameter, Some("via"));
    assert_eq!(refused_with(route("x -> y", &[(1, 1)]), "edit.unknown").parameter, Some("wire"));
    assert_eq!(refused_with(route("a y", &[(1, 1)]), "params.invalid").parameter, Some("wire"));
    // Not drawn: nothing to route.
    let error = run(&text(BRANCHED), route("a -> y.x", &[(1, 1)])).unwrap_err();
    assert_eq!((error.code, error.parameter), ("params.invalid", Some("wire")));
}

/// A junction follows its trunk (notation 2.5): its distance from the start of the segment it lies on, cut to the
/// length of that segment; when the trunk has another number of segments, the point goes and the junction is automatic.
#[test]
fn a_junction_follows_its_trunk() {
    // The branch to `z` leaves the trunk to `y` at (140, 40), 40 from the start of its only segment, which starts at
    // (100, 40).
    let before = format!("{BRANCHED}\nlayout {{\n    wire a -> z via (140, 40)\n}}\n");
    let branch = drawn("a -> z.x", &[(100, 40), (140, 40), (140, 120), (200, 120)]);
    let trunk_before = drawn("a -> y.x", &[(100, 40), (200, 40)]);
    // `a` moved down by 16: the trunk now starts at (100, 56), straight to `y` fixed at that height.
    let moved_straight = vec![drawn("a -> y.x", &[(100, 56), (200, 56)]), branch.clone()];
    let judge = Drawn { by_text: vec![("node a at (100, 56)", moved_straight), ("fn f", vec![trunk_before.clone(), branch.clone()])] };
    let move_a = Op::NodePlace { file: FILE.to_owned(), node: "a".to_owned(), at: Point { x: 100, y: 56 } };
    let edited = run_judged(&judge, &before, move_a.clone()).unwrap();
    let after = text(&format!("{BRANCHED}\nlayout {{\n    node a at (100, 56)\n    wire a -> z via (140, 56)\n}}\n"));
    assert_eq!(edited.texts.get(FILE).map(String::as_str), Some(after.as_str()));
    // A trunk that now bends: the point goes, and with it the entry, which held nothing else.
    let bent = vec![drawn("a -> y.x", &[(100, 56), (160, 56), (160, 40), (200, 40)]), branch.clone()];
    let judge = Drawn { by_text: vec![("node a at (100, 56)", bent), ("fn f", vec![trunk_before.clone(), branch.clone()])] };
    let edited = run_judged(&judge, &before, move_a.clone()).unwrap();
    let after = text(&format!("{BRANCHED}\nlayout {{\n    node a at (100, 56)\n}}\n"));
    assert_eq!(edited.texts.get(FILE).map(String::as_str), Some(after.as_str()));
    // A shorter segment cuts the distance to its length.
    let shorter = vec![drawn("a -> y.x", &[(100, 56), (120, 56)]), branch.clone()];
    let judge = Drawn { by_text: vec![("node a at (100, 56)", shorter), ("fn f", vec![trunk_before, branch])] };
    let edited = run_judged(&judge, &before, move_a).unwrap();
    let after = text(&format!("{BRANCHED}\nlayout {{\n    node a at (100, 56)\n    wire a -> z via (120, 56)\n}}\n"));
    assert_eq!(edited.texts.get(FILE).map(String::as_str), Some(after.as_str()));
}

/// A branch that the command routes is as the command leaves it.
#[test]
fn a_branch_routed_is_as_the_command_leaves_it() {
    let before = format!("{BRANCHED}\nlayout {{\n    wire a -> z via (140, 40)\n}}\n");
    let branch = drawn("a -> z.x", &[(100, 40), (140, 40), (140, 120), (200, 120)]);
    let trunk = drawn("a -> y.x", &[(100, 40), (200, 40)]);
    let judge = Drawn { by_text: vec![("fn f", vec![trunk, branch])] };
    let edited = run_judged(&judge, &before, route("a -> z.x", &[(160, 40)])).unwrap();
    let after = text(&format!("{BRANCHED}\nlayout {{\n    wire a -> z via (160, 40)\n}}\n"));
    assert_eq!(edited.texts.get(FILE).map(String::as_str), Some(after.as_str()));
}

const LABELLED: &str = "use core\n\nfn f(x: f64) -> (y: f64) {\n    a = core.abs(x)\n    #k: t = core.abs(a)\n    y = core.abs(t)\n}\n";

/// A node with a label is named by its label in `layout` and by its line in a command: both name it, and a second route
/// takes the place of the first (`LC5007` otherwise; spec 11).
#[test]
fn a_wire_to_a_node_with_a_label_is_routed_again_in_place() {
    let wires = vec![drawn("x -> a.x", &[(0, 40), (40, 40)]), drawn("a -> t.x", &[(80, 40), (120, 40)]), drawn("t -> y.x", &[(160, 40), (200, 40)])];
    let judge = Drawn { by_text: vec![("fn f", wires)] };
    let once = format!("{LABELLED}\nlayout {{\n    wire a -> #k via (40, 8)\n}}\n");
    let twice = text(&format!("{LABELLED}\nlayout {{\n    wire a -> #k via (48, 8)\n}}\n"));
    for name in ["a -> t.x", "a -> t", "a -> #k"] {
        let edited = run_judged(&judge, &once, route(name, &[(48, 8)])).unwrap_or_else(|error| panic!("{name}: {error:?}"));
        assert_eq!(edited.texts.get(FILE).map(String::as_str), Some(twice.as_str()), "{name}");
    }
    // "Arrange this wire" names it as the drawing does.
    let arranged = run_judged(
        &judge,
        &once,
        Op::LayoutArrange { file: FILE.to_owned(), nodes: None, wires: vec!["a -> t.x".to_owned()], terminals: Vec::new() },
    )
    .unwrap();
    assert_eq!(arranged.texts.get(FILE).map(String::as_str), Some(text(LABELLED).as_str()));
}

/// From inside a structure, `s -> z` is the wire to `until`, the input without a name, not the one into the loop
/// (spec 11).
#[test]
fn a_wire_to_until_is_routed() {
    let looping = "fn f(s: bool) -> (z: bool) {\n    z = loop carry a = s {\n        next a = a\n        until s\n    }\n}\n";
    let wires = vec![drawn("s -> z.a", &[(0, 40), (40, 40)]), drawn("s -> z.s", &[(0, 40), (40, 56)]), drawn("s -> z", &[(0, 16), (60, 16)])];
    let judge = Drawn { by_text: vec![("fn f", wires)] };
    let edited = run_judged(&judge, looping, route("s -> z", &[(16, 4), (40, 4)])).unwrap();
    let after = text(&format!("{looping}\nlayout {{\n    wire s -> z via (16, 4) (40, 4)\n}}\n"));
    assert_eq!(edited.texts.get(FILE).map(String::as_str), Some(after.as_str()));
}

/// A command that adds names, which the two texts then read in another order, still moves the junction with its trunk.
#[test]
fn a_junction_follows_its_trunk_whatever_names_a_command_adds() {
    let before = format!("{BRANCHED}\nlayout {{\n    wire a -> z via (140, 40)\n}}\n");
    let branch = drawn("a -> z.x", &[(100, 40), (140, 40), (140, 120), (200, 120)]);
    let trunk_before = drawn("a -> y.x", &[(100, 40), (200, 40)]);
    let trunk_after = drawn("a -> y.x", &[(100, 56), (200, 56)]);
    let judge = Drawn { by_text: vec![("aa = core.abs()", vec![trunk_after, branch.clone()]), ("fn f", vec![trunk_before, branch])] };
    let add = Op::NodeAdd { file: FILE.to_owned(), call: "core.abs".to_owned(), at: None, name: Some("aa".to_owned()), into: None };
    let edited = run_judged(&judge, &before, add).unwrap();
    let written = edited.texts.get(FILE).unwrap();
    assert!(written.contains("    wire a -> z via (140, 56)\n"), "{written}");
}

const LOOPING: &str = "fn f(s: bool) -> (z: bool) {\n    z = loop carry a = s {\n        next a = a\n        until s\n    }\n}\n";

/// `s` reaches `z` by three wires; `s -> z` is the one to `until`, which reaches an input without a name (spec 11).
fn looping_wires() -> Vec<crate::WireDrawn> {
    vec![drawn("s -> z.a", &[(0, 40), (40, 40)]), drawn("s -> z.s", &[(0, 40), (40, 56)]), drawn("s -> z", &[(0, 16), (60, 16)])]
}

/// The entry `s -> z` designates the wire to `until` alone: routing or arranging another wire of `s` to `z` keeps it.
#[test]
fn an_entry_without_its_input_is_the_route_of_one_wire() {
    let before = format!("{LOOPING}\nlayout {{\n    wire s -> z via (16, 4) (40, 4)\n}}\n");
    let judge = Drawn { by_text: vec![("fn f", looping_wires())] };
    let edited = run_judged(&judge, &before, route("s -> z.s", &[(20, 50)])).unwrap();
    let written = edited.texts.get(FILE).unwrap();
    assert!(written.contains("    wire s -> z via (16, 4) (40, 4)\n") && written.contains("    wire s -> z.s via (20, 50)\n"), "{written}");
    // Arranging `s -> z.s` takes its entry, and leaves the other.
    let both = format!("{LOOPING}\nlayout {{\n    wire s -> z via (16, 4) (40, 4)\n    wire s -> z.s via (20, 50)\n}}\n");
    let edited = run_judged(&judge, &both, arranging(None, &["s -> z.s"])).unwrap();
    let written = edited.texts.get(FILE).unwrap();
    assert!(written.contains("    wire s -> z via (16, 4) (40, 4)\n") && !written.contains("z.s via"), "{written}");
    // Arranging `s -> z` takes the entry of the wire to `until` alone.
    let edited = run_judged(&judge, &both, arranging(None, &["s -> z"])).unwrap();
    let written = edited.texts.get(FILE).unwrap();
    assert!(!written.contains("wire s -> z via") && written.contains("    wire s -> z.s via (20, 50)\n"), "{written}");
}

/// When a junction goes, the entry that it leaves with no point goes with it; an entry that the file writes with no
/// point stays.
#[test]
fn a_junction_that_goes_takes_only_its_own_entry() {
    let before = format!("{BRANCHED}\nlayout {{\n    wire a -> z via (140, 40)\n    wire a -> y\n}}\n");
    let branch = drawn("a -> z.x", &[(100, 40), (140, 40), (140, 120), (200, 120)]);
    let trunk_before = drawn("a -> y.x", &[(100, 40), (200, 40)]);
    // After, the trunk has another number of segments, and no longer goes through the junction.
    let trunk_after = drawn("a -> y.x", &[(100, 40), (120, 40), (120, 80), (200, 80)]);
    let judge = Drawn { by_text: vec![("aa = core.abs()", vec![trunk_after, branch.clone()]), ("fn f", vec![trunk_before, branch])] };
    let add = Op::NodeAdd { file: FILE.to_owned(), call: "core.abs".to_owned(), at: None, name: Some("aa".to_owned()), into: None };
    let edited = run_judged(&judge, &before, add).unwrap();
    let written = edited.texts.get(FILE).unwrap();
    assert!(!written.contains("a -> z") && written.contains("    wire a -> y\n"), "{written}");
}

/// A junction that its trunk would take off the grid of units goes: the file could not write it on the trunk.
#[test]
fn a_junction_off_the_grid_of_units_goes() {
    let before = format!("{BRANCHED}\nlayout {{\n    wire a -> z via (140, 40) (160, 120)\n}}\n");
    let branch = drawn("a -> z.x", &[(100, 40), (140, 40), (140, 120), (200, 120)]);
    let trunk_before = drawn("a -> y.x", &[(100, 40), (200, 40)]);
    let mut trunk_after = drawn("a -> y.x", &[(100, 40), (200, 40)]);
    trunk_after.path = vec![(10_000, 5_650), (20_000, 5_650)];
    let judge = Drawn { by_text: vec![("aa = core.abs()", vec![trunk_after, branch.clone()]), ("fn f", vec![trunk_before, branch])] };
    let add = Op::NodeAdd { file: FILE.to_owned(), call: "core.abs".to_owned(), at: None, name: Some("aa".to_owned()), into: None };
    let edited = run_judged(&judge, &before, add).unwrap();
    let written = edited.texts.get(FILE).unwrap();
    assert!(written.contains("    wire a -> z via (160, 120)\n"), "{written}");
}

/// The trunk of a branch is a wire without a junction of its own, when one goes through it: another branch along the
/// same trunk is drawn from its old junction until this pass moves it.
#[test]
fn a_junction_follows_the_trunk_and_not_another_branch() {
    let before = format!("{BRANCHED}\nlayout {{\n    wire a -> y via (180, 40)\n    wire a -> z via (140, 40)\n}}\n");
    // `a -> y` is a branch too, from (180, 40), along the trunk `a -> x` up to there.
    let z = drawn("a -> z.x", &[(100, 40), (140, 40), (140, 120), (200, 120)]);
    let y_before = drawn("a -> y.x", &[(100, 40), (180, 40), (180, 160), (200, 160)]);
    let y_after = drawn("a -> y.x", &[(100, 40), (180, 40), (180, 160), (200, 160)]);
    let trunk_before = drawn("a -> b.x", &[(100, 40), (240, 40)]);
    let trunk_after = drawn("a -> b.x", &[(100, 48), (240, 48)]);
    let judge = Drawn { by_text: vec![("aa = core.abs()", vec![y_after, z.clone(), trunk_after]), ("fn f", vec![y_before, z, trunk_before])] };
    let add = Op::NodeAdd { file: FILE.to_owned(), call: "core.abs".to_owned(), at: None, name: Some("aa".to_owned()), into: None };
    let edited = run_judged(&judge, &before, add).unwrap();
    let written = edited.texts.get(FILE).unwrap();
    assert!(written.contains("    wire a -> z via (140, 48)\n"), "{written}");
}
