//! The operations, command by command (architecture 12.10): each expected text is written by hand, from commands.md 9.1
//! and 6.7, the spec (5.1, 5.2, 11) and visual notation 2.4, never from the output of this crate.

use std::borrow::Cow;

use labcraft_check::{Content, Found, Libraries, LibrarySource, Origin, SourceFile, check};
use labcraft_types::{Diagnostic, Interner, Limits, Severity};

use crate::{Callable, EditError, Edited, Files, Input, Judge, Kind, Lookup, Op, Point, apply};

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
    Op::NodeAdd { file: FILE.to_owned(), call: call.to_owned(), at: None, name: None }
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
    let op = Op::NodeAdd { file: FILE.to_owned(), call: "core.add".to_owned(), at: Some(Point { x: 40, y: -80 }), name: Some("total".to_owned()) };
    gives(EMPTY, op, "use core\n\nfn f(x: f64) -> (y: f64) {\n    total = core.add()\n    y = x\n}\n\nlayout {\n    node total at (40, -80)\n}\n");
}

#[test]
fn node_add_refuses_what_it_cannot_name() {
    let named = |name: &str| Op::NodeAdd { file: FILE.to_owned(), call: "core.add".to_owned(), at: None, name: Some(name.to_owned()) };
    refused(EMPTY, named("x"), "name.taken");
    refused(EMPTY, named("match"), "name.invalid");
    refused(EMPTY, named("2x"), "name.invalid");
    let error = refused(EMPTY, add("core.ad"), "edit.unknown");
    assert_eq!(error.expected, ["core.add"]);
    refused(EMPTY, add("core..add"), "params.invalid");
    let far = Op::NodeAdd { file: FILE.to_owned(), call: "core.add".to_owned(), at: Some(Point { x: 1_000_001, y: 0 }), name: None };
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
    let far = Op::NodeAdd { file: FILE.to_owned(), call: "core.add".to_owned(), at: Some(Point { x: i64::MIN, y: 0 }), name: None };
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
