//! Pass 3, on small programs: what each code means, and what must stay silent.

use std::borrow::Cow;

use labcraft_graph::{Arg, Decl, Operand, Rhs, StatementKind, Target as Line};
use labcraft_types::Diagnostic;

use crate::{Checked, Content, Found, Libraries, LibrarySource, Meaning, Origin, PrimitiveLibrary, SourceFile, Terminal, check};

use super::super::*;

struct Nothing;

impl Libraries for Nothing {
    fn find(&self, _: &str, _: Origin) -> Found {
        Found::NotFound
    }
}

fn source(name: &str, files: &[(&str, &str)]) -> LibrarySource {
    let files = files
        .iter()
        .map(|(shown, text)| {
            let text = format!("labcraft 1\n\n{text}\n");
            SourceFile { shown: (*shown).to_owned(), content: Content::Bytes(Cow::Owned(text.into_bytes())) }
        })
        .collect();
    LibrarySource { files, ..LibrarySource::new(name.to_owned(), name.to_owned(), Origin::Project) }
}

fn run(root: LibrarySource) -> (Checked, Interner) {
    let mut names = Interner::new();
    let checked = check(root, &Nothing, &mut names, &Limits::default());
    (checked, names)
}

fn diagnostics(files: &[(&str, &str)]) -> Vec<Diagnostic> {
    let mut report = run(source("", files)).0.report;
    report.finish();
    report.diagnostics().to_vec()
}

fn codes(files: &[(&str, &str)]) -> Vec<&'static str> {
    diagnostics(files).iter().map(|diagnostic| diagnostic.code().as_str()).collect()
}

/// `text` is the whole of the function file `f.lcf`, beside the type `Reading`, the type `Point` and the functions `g` and `takes`.
fn in_file(text: &str) -> Vec<&'static str> {
    codes(&[
        ("f.lcf", text),
        ("reading.lct", "type Reading = enum { Value(f64), Overload }"),
        ("point.lct", "type Point = struct { x: f64, y: f64 }"),
        ("g.lcf", "fn g(a: i32, b: i32 = 0, err: error = none) -> (r: i32, err: error) {\n    r = a\n    err = err\n}"),
        ("takes.lcf", "fn takes(a: [i32]) -> (r: i32) {\n    r = 1\n}"),
    ])
}

/// `lines` is the body of `fn f(p: i32, err: error = none) -> (out: i32, err: error)`, with `use core` and `use ui`.
fn in_body(lines: &str) -> Vec<&'static str> {
    in_file(&format!("use core\nuse ui\n\nfn f(p: i32, err: error = none) -> (out: i32, err: error) {{\n{lines}\nout = p\nerr = err\n}}"))
}

const NOTHING: [&str; 0] = [];

#[test]
fn a_line_is_read_wherever_it_is_written() {
    assert_eq!(in_body("x = y\ny = p"), NOTHING);
    assert_eq!(in_body("x = nowhere"), ["LC2001"]);
    assert_eq!(in_body("x = step.low"), ["LC2001"], "a path reads its first segment");
}

#[test]
fn an_output_can_be_read_back_and_a_name_that_is_both_reads_the_input() {
    assert_eq!(in_body("x = out"), NOTHING);
    let (checked, _) = run(source("", &[("f.lcf", "fn f(err: error = none) -> (err: error) {\n    x = err\n    err = x\n}")]));
    let file = checked.files.first().unwrap();
    let Decl::Function(function) = &file.file.decl else { panic!() };
    let [read, bind] = function.body.statements.as_slice() else { panic!() };
    let StatementKind::Binding { value: Rhs::Operand(Operand::Path(path)), .. } = &read.kind else { panic!() };
    let input = function.signature.params.first().unwrap().id;
    assert_eq!(file.resolved.of(path.id), Meaning::Value(input), "R-2: reading it is the input");
    let StatementKind::Binding { targets, .. } = &bind.kind else { panic!() };
    let [Line::Name(id, _)] = &**targets else { panic!() };
    let output = function.signature.outputs.first().unwrap().id;
    assert_eq!(file.resolved.of(*id), Meaning::Value(output), "R-2: binding it is the output");
}

#[test]
fn a_value_defined_twice_in_a_body() {
    assert_eq!(in_body("x = 1\nx = 2"), ["LC2012"]);
    assert_eq!(in_body("x, x = g(1)"), ["LC2012"]);
    assert_eq!(in_body("p = 1"), ["LC2012"], "an input is defined by the signature");
    assert_eq!(in_body("x, y = loop carry a = 1, carry a = 2 {\nnext a = 1\nuntil true\n}"), ["LC2012"]);
    assert_eq!(in_body("x = loop carry a = 1 {\na = 2\nnext a = 1\nuntil true\n}"), ["LC2012"]);
    assert_eq!(in_body("x = match p {\n1 => {\ny = 1\ny = 2\nyield y\n}\n_ => yield 0\n}"), ["LC2012"]);
}

#[test]
fn a_sub_body_hides_a_name_from_outside_and_keeps_it_to_itself() {
    assert_eq!(in_body("x = 1\ny = match p {\n1 => {\nx = 2\nyield x\n}\n_ => yield x\n}"), NOTHING);
    assert_eq!(in_body("y = match p {\n1 => {\nx = 2\nyield x\n}\n_ => yield x\n}"), ["LC2001"], "a line of an arm stays there");
    assert_eq!(in_body("y = match p {\n1 => {\nx = 2\nyield x\n}\n_ => {\nx = 3\nyield x\n}\n}"), NOTHING, "two arms, two bodies");
    assert_eq!(
        in_file("fn f(r: Reading) -> (x: f64) {\n    x = match r {\n        Reading.Value(v) => yield v\n        _ => yield v\n    }\n}"),
        ["LC2001"]
    );
}

#[test]
fn a_carry_and_a_clause_read_from_outside_and_bind_inside() {
    assert_eq!(in_body("x = loop carry err = err {\ny = err\nnext err = y\nuntil true\n}"), NOTHING, "R-3");
    assert_eq!(in_body("x = loop carry a = a {\nnext a = 1\nuntil true\n}"), ["LC2001"], "the carry is not visible in its clause");
    assert_eq!(in_body("x = for i in 0..3, carry a = i {\nnext a = i\n}"), ["LC2001"], "nor a clause in another clause");
    assert_eq!(in_body("x = for i in 0..p {\ncollect i\n}\ny = i"), ["LC2001"], "nothing leaves the loop");
}

#[test]
fn next_is_for_a_carry_of_this_very_loop() {
    assert_eq!(in_body("x = loop carry a = 1 {\nnext b = 1\nuntil true\n}"), ["LC2018"]);
    assert_eq!(in_body("x = for i in 0..3 {\nnext i = 1\ncollect i\n}"), ["LC2018"], "a clause is no carry");
    let nested = "x = loop carry a = 1 {\ny = loop carry b = 1 {\nnext a = 1\nnext b = 1\nuntil true\n}\nnext a = y\nuntil true\n}";
    assert_eq!(in_body(nested), ["LC2018"]);
}

#[test]
fn a_label_is_unique_in_its_function_and_visible_from_its_body_inward() {
    assert_eq!(in_body("#a: x = 1\n#a: y = 2"), ["LC2015"]);
    assert_eq!(in_body("#a: x = 1\ny = match p {\n1 => {\n#a: z = 1\nyield z\n}\n_ => yield 0\n}"), ["LC2015"], "in its function, not in its body");
    assert_eq!(in_body("x, _ = g(1, after: #a)\n#a: y = 2"), NOTHING);
    assert_eq!(in_body("x, _ = g(1, after: #nowhere)"), ["LC2011"]);
    assert_eq!(in_body("x = match p {\n1 => {\ny, _ = g(1, after: #a)\nyield 0\n}\n_ => yield 0\n}\n#a: z = 1"), NOTHING, "from an outer body");
    assert_eq!(in_body("x = match p {\n1 => {\n#a: y = 1\nyield y\n}\n_ => yield 0\n}\nz, _ = g(1, after: #a)"), ["LC2011"], "not into a sub-body");
    let twice = "x = match p {\n1 => {\n#a: y = 1\nyield y\n}\n_ => {\n#a: y = 2\nz, _ = g(1, after: #a)\nyield y\n}\n}";
    assert_eq!(in_body(twice), ["LC2015"], "the second one is still beside what reads it");
}

#[test]
fn after_takes_values_and_labels() {
    assert_eq!(in_body("#a: x = 1\ny, _ = g(1, after: (x, #a))"), NOTHING);
    assert_eq!(in_body("y, _ = g(1, after: nowhere)"), ["LC2001"]);
    assert_eq!(in_body("y, _ = g(1, after: 3)"), ["LC2105"]);
    assert_eq!(in_body("y, _ = g(1, after: [p])"), ["LC2105"], "not a unit, not an array");
    assert_eq!(in_body("y, _ = g(1, after: p, after: p)"), ["LC2105"]);
    assert_eq!(in_body("y, _ = g(1, after: Reading.Overload)"), ["LC2001"]);
    assert_eq!(
        codes(&[("a.lcf", "fn a( {"), ("f.lcf", "fn f() -> () {\n    h(after: nowhere)\n}")]),
        ["LC1001", "LC2001"],
        "a value comes from this file"
    );
}

#[test]
fn a_value_cannot_bear_the_name_of_a_library_a_type_or_a_live_element() {
    assert_eq!(in_body("core = 1"), ["LC2014"]);
    assert_eq!(in_body("Reading = 1"), ["LC2014"]);
    assert_eq!(in_body("x = loop carry core = 1 {\nnext core = core\nuntil true\n}"), ["LC2014"], "R-1 comes before R-3");
    assert_eq!(in_body("x = loop carry Reading = 1 {\nnext Reading = 1\nuntil true\n}"), ["LC2014"]);
    assert_eq!(in_body("x = for core in 0..3 {\ncollect core\n}"), ["LC2014"]);
    assert_eq!(in_body("x = match p {\n1 => {\ncore = 1\nyield core\n}\n_ => yield 0\n}"), ["LC2014"]);
    assert_eq!(
        in_file(
            "use core\n\nfn f(r: Reading) -> (x: f64) {\n    x = match r {\n        Reading.Value(core) => yield 1.0\n        _ => yield 2.0\n    }\n}"
        ),
        ["LC2014"]
    );
    assert_eq!(
        in_file("fn f(p: i32) -> (x: i32) {\n    stop = 1\n    x = stop\n}\n\npanel {\n    control stop: button\n}"),
        ["LC2014"],
        "neither an input nor an output"
    );
    assert_eq!(in_file("fn f(stop: i32) -> (x: i32) {\n    x = stop\n}\n\npanel {\n    control stop: button\n}"), NOTHING);
    assert_eq!(
        codes(&[("f.lcf", "fn f() -> (x: i32) {\n    session = 1\n    x = session\n}"), ("s.lct", "type session = opaque")]),
        NOTHING,
        "an opaque type has nothing to name"
    );
}

#[test]
fn a_value_can_bear_the_name_of_a_function_and_hides_it_in_an_argument() {
    assert_eq!(in_file("fn f(g: i32) -> (x: i32, y: i32) {\n    x, _ = g(g)\n    y = g\n}"), NOTHING);
}

#[test]
fn a_function_reference_is_a_path_and_a_primitive_can_be_one() {
    assert_eq!(in_file("fn f() -> () {\n    x = g\n}"), NOTHING);
    assert_eq!(in_file("use core\n\nfn f() -> () {\n    x = core.add\n}"), NOTHING);
    assert_eq!(in_file("use core\n\nfn f() -> () {\n    x = core.nowhere\n}"), ["LC2002"]);
    assert_eq!(in_file("fn f() -> () {\n    x = core.add\n}"), ["LC2005"], "`core` is known to be a library");
    assert_eq!(in_file("fn f() -> () {\n    x = stpe.low\n}"), ["LC2001"], "a typo in a value is not told to add a `use`");
}

#[test]
fn a_type_is_not_a_value() {
    assert_eq!(in_file("fn f() -> (x: i32) {\n    x = Reading\n}"), ["LC2001"]);
}

#[test]
fn a_variant_is_written_with_its_type() {
    assert_eq!(in_file("fn f() -> (x: Reading) {\n    x = Reading.Overload\n}"), NOTHING);
    assert_eq!(in_file("fn f() -> (x: Reading) {\n    x = Reading.Nowhere\n}"), ["LC2007"]);
    assert_eq!(in_file("fn f() -> (x: f64) {\n    x = Point.x\n}"), ["LC2007"], "a struct has no variants");
    assert_eq!(in_file("fn f(v: f64) -> (x: Reading) {\n    x = Reading.Value(v)\n}"), NOTHING);
    assert_eq!(in_file("fn f(v: f64) -> (x: Reading) {\n    x = Reading.Nowhere(v)\n}"), ["LC2007"]);
    assert_eq!(
        in_file("fn f(r: Reading) -> (x: i32) {\n    x = match r {\n        Reading.Nowhere => yield 1\n        _ => yield 2\n    }\n}"),
        ["LC2007"]
    );
    assert_eq!(in_file("fn f(r: Reading) -> (x: i32) {\n    x = match r {\n        Overload => yield 1\n        _ => yield 2\n    }\n}"), ["LC2007"]);
}

#[test]
fn a_variant_is_positional_then_order_as_a_call_is() {
    assert_eq!(in_file("fn f(v: f64, p: i32) -> (x: Reading) {\n    x = Reading.Value(v, after: p)\n}"), NOTHING);
    assert_eq!(in_file("fn f(v: f64, p: i32) -> (x: Reading) {\n    x = Reading.Value(after: p, v)\n}"), ["LC2104"]);
    assert_eq!(in_file("fn f(v: f64) -> (x: Reading) {\n    x = Reading.Value(value: v)\n}"), ["LC2009"]);
}

#[test]
fn a_variant_through_an_alias_is_a_variant_of_what_it_names() {
    let files = [("f.lcf", "fn f() -> (x: A) {\n    x = A.One\n}"), ("a.lct", "type A = B"), ("b.lct", "type B = enum { One }")];
    assert_eq!(codes(&files), NOTHING);
    let files = [("f.lcf", "fn f() -> (x: A) {\n    x = A.Two\n}"), ("a.lct", "type A = B"), ("b.lct", "type B = enum { One }")];
    assert_eq!(codes(&files), ["LC2007"]);
}

#[test]
fn a_type_with_an_error_says_nothing_more() {
    let files = [("f.lcf", "fn f() -> (x: i32) {\n    x = T.One\n}"), ("t.lct", "type T = enum { One(nowhere) }")];
    assert_eq!(codes(&files), ["LC2003"]);
    let files = [("f.lcf", "fn f() -> (x: i32) {\n    x = T { a: 1 }\n}"), ("t.lct", "type T = nowhere")];
    assert_eq!(codes(&files), ["LC2003"]);
}

#[test]
fn a_struct_construction_names_a_type_and_its_fields() {
    assert_eq!(in_file("fn f() -> (x: Point) {\n    x = Point { x: 1.0, y: 2.0 }\n}"), NOTHING);
    assert_eq!(in_file("fn f() -> (x: Point) {\n    x = Point { x: 1.0, z: 2.0 }\n}"), ["LC2006"]);
    assert_eq!(in_file("fn f() -> (x: Point) {\n    x = Nowhere { x: 1.0 }\n}"), ["LC2003"]);
    assert_eq!(in_file("fn f() -> (x: Point) {\n    x = lib.Point { x: 1.0 }\n}"), ["LC2005"]);
    assert_eq!(in_file("fn f() -> (x: Point) {\n    x = Point { x: nowhere, y: 2.0 }\n}"), ["LC2001"]);
    assert_eq!(in_file("fn f() -> (x: Point, y: i32) {\n    x, y = Point { x: 1.0, y: 2.0 }\n}"), ["LC2103"]);
}

#[test]
fn an_unknown_function_and_a_library_without_use() {
    assert_eq!(in_body("x = nowhere(p)"), ["LC2002"]);
    assert_eq!(in_body("x = core.nowhere(p)"), ["LC2002"]);
    assert_eq!(in_file("fn f() -> (x: i32) {\n    x = visa.open(1)\n}"), ["LC2005"]);
    assert_eq!(in_body("x = p(1)"), ["LC2002"], "a value is not called");
    assert_eq!(in_body("x = p.q(1)"), ["LC2002"]);
    assert_eq!(in_body("x = Point(1)"), ["LC2002"], "a type is not called");
    assert_eq!(in_body("x = nowhere(nothing)"), ["LC2002", "LC2001"], "its arguments are still read");
    assert_eq!(in_body("x = nowhere([V/s])"), ["LC2002"], "a bracket may be a unit: nothing is known");
}

#[test]
fn a_library_without_use_has_one_diagnostic_in_a_file() {
    let text = "fn f(s: visa.session) -> (x: i32) {\n    a = visa.open(1)\n    b = visa.close(1)\n    x = visa.query(a, b, s)\n}";
    assert_eq!(in_file(text), ["LC2005"], "the signature said it, and the body says nothing more");
    let text = "fn f() -> (x: i32) {\n    a = visa.open(1)\n    x = visa.close(a)\n}";
    assert_eq!(in_file(text), ["LC2005"]);
    assert_eq!(in_file("fn f() -> (x: i32) {\n    a = visa.open(1)\n    x = lab.close(a)\n}"), ["LC2005", "LC2005"], "one per library");
}

#[test]
fn the_fix_of_a_name_of_core_qualifies_it_and_adds_its_use() {
    let found = diagnostics(&[("f.lcf", "fn f(a: i32) -> (x: i32) {\n    x = add(a, 1)\n}")]);
    let [diagnostic] = found.as_slice() else { panic!("{found:?}") };
    assert_eq!(diagnostic.code(), Code::Lc2002);
    let edits = &diagnostic.fixes.first().unwrap().edits;
    assert_eq!(edits.first().unwrap().text, "core.add");
    assert_eq!(edits.get(1).unwrap().text, "\nuse core\n");
    let found = diagnostics(&[("f.lcf", "use core\n\nfn f(a: i32) -> (x: i32) {\n    x = add(a, 1)\n}")]);
    assert_eq!(found.first().unwrap().fixes.first().unwrap().edits.len(), 1, "the file has its `use`");
    let found = diagnostics(&[("f.lcf", "fn f(a: i32) -> (x: i32) {\n    x = nowhere(a)\n}")]);
    assert!(found.first().unwrap().fixes.is_empty());
}

#[test]
fn a_diagnostic_of_a_body_names_its_function_and_its_node() {
    let found = diagnostics(&[("f.lcf", "fn f() -> (x: i32) {\n    x = nowhere\n    #a: _ = other\n}")]);
    let [first, second] = found.as_slice() else { panic!("{found:?}") };
    assert_eq!(first.function.as_deref(), Some("f"));
    let node = first.node.as_ref().unwrap();
    assert_eq!((node.name.as_deref(), node.text.as_str()), (Some("x"), "x = nowhere"));
    assert_eq!(second.node.as_ref().unwrap().name.as_deref(), Some("a"), "the name of a line with no output is its label");
    let found = diagnostics(&[(
        "f.lcf",
        "fn f() -> (x: i32) {\n    x = match a {\n        _ => {\n            y = b\n            yield c\n        }\n    }\n}",
    )]);
    let texts: Vec<&str> = found.iter().map(|diagnostic| diagnostic.node.as_ref().unwrap().text.as_str()).collect();
    assert_eq!(texts, ["x = match a {", "y = b", "x = match a {"], "the line that holds each name");
}

#[test]
fn a_type_in_a_body_names_its_line_too() {
    let found = diagnostics(&[("g.lcf", "fn g() -> (r: i32) {\n    r = 1\n}"), ("f.lcf", "fn f() -> (x: i32) {\n    x = g<Nope>()\n}")]);
    let diagnostic = found.iter().find(|diagnostic| diagnostic.code() == Code::Lc2003).unwrap();
    assert_eq!(diagnostic.node.as_ref().unwrap().text, "x = g<Nope>()");
    let found = diagnostics(&[("f.lcf", "fn f() -> (x: i32) {\n    x = loop carry a: Nope = 1 {\n        next a = 1\n        until true\n    }\n}")]);
    let [diagnostic] = found.as_slice() else { panic!("{found:?}") };
    assert_eq!(diagnostic.node.as_ref().unwrap().name.as_deref(), Some("x"));
    let found = diagnostics(&[("f.lcf", "fn f(a: Nope) -> () {}")]);
    assert_eq!(found.first().unwrap().node, None, "a signature is no line");
}

#[test]
fn a_call_is_positional_then_named() {
    assert_eq!(in_body("x, _ = g(1)"), NOTHING);
    assert_eq!(in_body("x, _ = g(1, 2)"), NOTHING, "without a variadic parameter, a default can be positional");
    assert_eq!(in_body("x, _ = g(1, err: err)"), NOTHING);
    assert_eq!(in_body("x, _ = g(a: 1, b: 2)"), NOTHING);
    assert_eq!(in_body("x, _ = g(a: 1, 2)"), ["LC2104"]);
    assert_eq!(in_body("x, _ = g(after: p, 1)"), ["LC2104"], "`after:` is named too, and what is missing may come from that");
    assert_eq!(in_body("x, _ = g(1, 2, err, 4)"), ["LC2102"]);
    assert_eq!(in_body("x, _ = g(1, 2, err, 4, 5)"), ["LC2102"], "one diagnostic for one call");
    assert_eq!(in_body("x, _ = g(1, a: 2)"), ["LC2102"], "wired twice");
    assert_eq!(in_body("x, _ = g(a: 1, a: 2)"), ["LC2102"]);
    assert_eq!(in_body("x, _ = g(1, c: 2)"), ["LC2009"]);
    assert_eq!(in_body("x, _ = g()"), ["LC2101"]);
    assert_eq!(in_body("x, _ = g(b: 1)"), ["LC2101"]);
    assert_eq!(in_body("x, y, z = g(1)"), ["LC2103"]);
    assert_eq!(in_body("x = g(1)"), ["LC2103"]);
    assert_eq!(in_body("g(1)"), NOTHING, "a call alone binds none of them (R-16)");
}

#[test]
fn a_call_reads_its_arguments_even_when_it_has_a_mistake() {
    assert_eq!(in_body("x, _ = g(1, 2, err, nowhere)"), ["LC2001", "LC2102"], "the same place, in the order of the codes");
    assert_eq!(in_body("x, _ = g(1, c: nowhere)"), ["LC2009", "LC2001"]);
    assert_eq!(in_body("x, _ = g(a: 1, nowhere)"), ["LC2001", "LC2104"]);
}

#[test]
fn a_variadic_parameter_takes_positional_arguments_only() {
    let variadic = ("v.lcf", "fn v(parts: i32..., err: error = none) -> (r: i32) {\n    r = 1\n}");
    let call = |line: &str| codes(&[variadic, ("f.lcf", &format!("fn f(e: error) -> (r: i32) {{\n    r = {line}\n}}"))]);
    assert_eq!(call("v()"), NOTHING, "zero arguments is allowed");
    assert_eq!(call("v(1, 2, 3)"), NOTHING);
    assert_eq!(call("v(1, 2, err: e)"), NOTHING);
    assert_eq!(call("v(parts: 1)"), ["LC2009"]);
    assert_eq!(call("v(err: e, err: e)"), ["LC2102"]);
    let strings = "use errors\nuse string\n\nfn f(a: string, e: error) -> (x: string, y: error) {\n    x = string.concat(a, a, a)\n    y = errors.merge(e, e)\n}";
    assert_eq!(in_file(strings), NOTHING);
}

#[test]
fn a_function_with_a_bad_signature_says_nothing_more_in_a_call() {
    let bad = ("v.lcf", "fn v(a: i32, a: i32) -> (r: i32) {\n    r = 1\n}");
    assert_eq!(codes(&[bad, ("f.lcf", "fn f() -> (r: i32) {\n    r = v()\n}")]), ["LC2019"]);
    assert_eq!(codes(&[bad, ("f.lcf", "fn f() -> (r: i32) {\n    r = v(1, 2, 3)\n}")]), ["LC2019"]);
    assert_eq!(codes(&[bad, ("f.lcf", "fn f() -> (r: i32) {\n    r = v(c: 3)\n}")]), ["LC2019"]);
    let misplaced = ("v.lcf", "fn v(a: i32..., b: i32) -> (r: i32) {\n    r = 1\n}");
    assert_eq!(codes(&[misplaced, ("f.lcf", "fn f() -> (r: i32) {\n    r = v()\n}")]), ["LC2107"]);
}

#[test]
fn a_function_defined_twice_binds_to_the_first_one() {
    let files = [
        ("a.lcf", "fn g(a: i32) -> (r: i32) {\n    r = a\n}"),
        ("b.lcf", "fn g() -> (r: i32) {\n    r = 1\n}"),
        ("f.lcf", "fn f() -> (r: i32) {\n    r = g(1)\n}"),
    ];
    assert_eq!(codes(&files), ["LC2013"]);
    let files = [
        ("a.lcf", "fn g() -> (r: i32, s: i32) {\n    r = 1\n    s = 1\n}"),
        ("b.lcf", "fn g() -> (r: i32) {\n    r = 1\n}"),
        ("f.lcf", "fn f() -> (r: i32) {\n    r = g()\n}"),
    ];
    assert_eq!(codes(&files), ["LC2013"], "which one was meant stays unknown");
    let files = [("v.lcf", "fn v() -> (a: i32, a: i32) {\n    a = 1\n}"), ("f.lcf", "fn f() -> (r: i32) {\n    r = v()\n}")];
    assert_eq!(codes(&files), ["LC2019"]);
}

#[test]
fn a_bad_file_hides_what_it_may_declare() {
    let broken = ("a.lcf", "fn a( {");
    assert_eq!(codes(&[broken, ("f.lcf", "fn f() -> (r: i32) {\n    r = a(1)\n}")]), ["LC1001"]);
    assert_eq!(codes(&[broken, ("f.lcf", "fn f() -> (r: i32) {\n    r = a\n}")]), ["LC1001"], "it may be a function reference");
    assert_eq!(codes(&[broken, ("f.lcf", "fn f() -> (r: i32) {\n    r = T.A\n}")]), ["LC1001"]);
    assert_eq!(codes(&[broken, ("f.lcf", "fn f() -> (r: i32) {\n    r = T { a: 1 }\n}")]), ["LC1001"]);
    assert_eq!(codes(&[broken, ("f.lcf", "fn f() -> (r: i32) {\n    r = 1\n    q = T.g(1)\n}")]), ["LC1001"], "`T` may be a type of that file");
    assert_eq!(codes(&[broken, ("f.lcf", "fn f() -> (r: i32) {\n    r = nowhere(1, c)\n}")]), ["LC1001"]);
}

#[test]
fn a_call_to_a_library_that_is_not_there_says_nothing_more() {
    assert_eq!(in_file("use nowhere\n\nfn f() -> (x: i32) {\n    x = nowhere.g(1)\n}"), ["LC2004"]);
    assert_eq!(in_file("use nowhere\n\nfn f() -> (x: i32) {\n    x = nowhere.T.V\n}"), ["LC2004"]);
    assert_eq!(in_file("use nowhere\n\nfn f() -> (x: i32) {\n    x = nowhere.a.b.c()\n}"), ["LC2004"]);
}

#[test]
fn a_sub_library_is_named_by_its_path_and_its_types_and_variants_too() {
    let mut root = source("", &[("f.lcf", "use a.b\n\nfn f() -> (x: a.b.T, y: i32) {\n    x = a.b.T.One\n    y = a.b.g(1)\n}")]);
    let mut a = source("a", &[("a/x.lcf", "fn x() -> () {}")]);
    a.sub_libraries.push(source("b", &[("a/b/t.lct", "type T = enum { One }"), ("a/b/g.lcf", "fn g(n: i32) -> (r: i32) {\n    r = n\n}")]));
    root.sub_libraries.push(a);
    let mut report = run(root).0.report;
    report.finish();
    assert_eq!(report.diagnostics().iter().map(|d| d.code().as_str()).collect::<Vec<_>>(), NOTHING);
}

/// The folder given has `lib`, with a type `T`, and `lib/sub`, with a function `g`.
fn with_library(text: &str) -> Vec<Diagnostic> {
    let mut root = source("", &[("f.lcf", text)]);
    let mut lib = source("lib", &[("lib/t.lct", "type T = enum { A }")]);
    lib.sub_libraries.push(source("sub", &[("lib/sub/g.lcf", "fn g() -> () {}")]));
    root.sub_libraries.push(lib);
    let mut report = run(root).0.report;
    report.finish();
    report.diagnostics().to_vec()
}

#[test]
fn a_library_without_use_is_named_as_it_is() {
    let found = with_library("fn f(s: lib.T) -> () {\n    x = lib.T.B(1)\n}");
    let [diagnostic] = found.as_slice() else { panic!("{found:?}") };
    assert_eq!(diagnostic.code(), Code::Lc2005, "once for `lib`, in the signature and in the body");
    let found = with_library("fn f() -> () {\n    x = lib.T.A\n}");
    let [diagnostic] = found.as_slice() else { panic!("{found:?}") };
    assert_eq!(diagnostic.code(), Code::Lc2005);
    assert_eq!(diagnostic.fixes.first().unwrap().edits.first().unwrap().text, "\nuse lib\n");
    let found = with_library("use lib\n\nfn f() -> () {\n    lib.sub.g()\n}");
    let [diagnostic] = found.as_slice() else { panic!("{found:?}") };
    assert_eq!(diagnostic.code(), Code::Lc2005, "the sub-library exists, and the file does not import it");
    assert_eq!(diagnostic.fixes.first().unwrap().title, "Add `use lib.sub`");
}

#[test]
fn a_typo_is_not_told_to_add_a_use() {
    assert_eq!(
        in_file("fn f(r: Reading) -> (x: i32) {\n    x = match r {\n        Readin.Overload => yield 1\n        _ => yield 2\n    }\n}"),
        ["LC2007"]
    );
    assert_eq!(in_body("x = g.h()"), ["LC2002"], "`g` is a function");
}

/// A library that another file of the run imports is no more known here than one that nobody does.
#[test]
fn what_a_path_names_does_not_depend_on_another_file() {
    struct Dep;
    impl Libraries for Dep {
        fn find(&self, name: &str, _: Origin) -> Found {
            if name == "dep" { Found::Library(source("dep", &[("<dep>/g.lcf", "fn g() -> () {}")])) } else { Found::NotFound }
        }
    }
    let run_with = |files: &[(&str, &str)]| {
        let mut report = check(source("", files), &Dep, &mut Interner::new(), &Limits::default()).report;
        report.finish();
        report.diagnostics().iter().map(|diagnostic| diagnostic.code().as_str()).collect::<Vec<_>>()
    };
    let reads = ("f.lcf", "fn f() -> (x: i32) {\n    x = dep.g\n}");
    assert_eq!(run_with(&[reads]), ["LC2001"]);
    assert_eq!(run_with(&[reads, ("other.lcf", "use dep\n\nfn other() -> () {}")]), ["LC2001"]);
}

#[test]
fn a_function_of_another_library_binds_to_its_signature() {
    let run_on = |line: &str| {
        let mut root = source("", &[("f.lcf", &format!("use lib\n\nfn f() -> () {{\n    x = {line}\n}}"))]);
        root.sub_libraries.push(source("lib", &[("lib/g.lcf", "fn g(a: i32) -> (r: i32) {\n    r = a\n}"), ("lib/t.lct", "type T = enum { A }")]));
        let mut report = run(root).0.report;
        report.finish();
        report.diagnostics().iter().map(|d| d.code().as_str()).collect::<Vec<_>>()
    };
    assert_eq!(run_on("lib.g(1)"), NOTHING);
    assert_eq!(run_on("lib.g()"), ["LC2101"]);
    assert_eq!(run_on("lib.nowhere(1)"), ["LC2002"]);
    assert_eq!(run_on("lib.T(1)"), ["LC2002"]);
    assert_eq!(run_on("lib.T.A"), NOTHING);
    assert_eq!(run_on("lib.T.B"), ["LC2007"]);
    assert_eq!(run_on("lib.U.B"), ["LC2002"], "the library has no such thing");
    assert_eq!(run_on("lib.T"), ["LC2001"]);
}

#[test]
fn the_primitives_bind_as_appendix_a_says() {
    assert_eq!(in_body("x = core.add(p, 1)"), NOTHING);
    assert_eq!(in_body("x = core.add(p)"), ["LC2101"]);
    assert_eq!(in_body("x = core.add(p, 1, 2)"), ["LC2102"]);
    assert_eq!(in_body("x = core.add(a: p, b: 1)"), NOTHING);
    assert_eq!(in_body("x = core.add(p, c: 1)"), ["LC2009"], "a name mistyped is one mistake");
    assert_eq!(in_body("x, y = core.add(p, 1)"), ["LC2103"]);
}

#[test]
fn a_unit_argument_is_in_brackets() {
    assert_eq!(in_body("x = core.with_unit(p, [V])"), NOTHING);
    assert_eq!(in_body("x = core.with_unit(p, unit: [V/s])"), NOTHING);
    assert_eq!(in_body("x = core.with_unit(p, [Vx])"), ["LC2008"]);
    assert_eq!(in_body("x = core.with_unit(p, V)"), ["LC3001"]);
    assert_eq!(in_body("x = core.with_unit(p, 3)"), ["LC3001"]);
    assert_eq!(in_body("x = core.with_unit(p, [m^999])"), ["LC1006"]);
}

#[test]
fn elsewhere_a_bracket_is_an_array_of_one_value() {
    assert_eq!(in_body("x = takes([p])"), NOTHING);
    assert_eq!(in_body("x = takes([1])"), NOTHING);
    assert_eq!(in_body("x = takes([nowhere])"), ["LC2001"]);
    assert_eq!(in_body("x = takes([V/s])"), ["LC3001"]);
    assert_eq!(in_body("x = core.eq([g], [g, g])"), NOTHING, "an array of one function, as `[g, g]` is an array of two");
    assert_eq!(in_body("x = takes([Reading])"), ["LC2001"]);
    assert_eq!(in_body("x = takes([p, nowhere])"), ["LC2001"], "an array of values reads them");
    assert_eq!(in_body("x = [nowhere]"), ["LC2001"], "as anything but an argument");
}

#[test]
fn an_open_function_reads_its_named_arguments_as_values_and_each_once() {
    let format = |args: &str| in_file(&format!("use string\n\nfn f(n: i32) -> (x: string) {{\n    x = string.format(\"{{a}}\", {args})\n}}"));
    assert_eq!(format("a: n"), NOTHING);
    assert_eq!(format("a: [n]"), NOTHING);
    assert_eq!(format("a: [nowhere]"), ["LC2001"]);
    assert_eq!(format("a: [m/s]"), ["LC3001"]);
    assert_eq!(format("a: n, a: n"), ["LC2102"]);
    assert_eq!(in_file("use core\n\nfn f(h: fn(a: i32) -> (i32)) -> (x: i32) {\n    x = core.call(h, a: 1, a: 2)\n}"), ["LC2102"]);
}

#[test]
fn a_category_is_open_and_is_not_a_value() {
    assert_eq!(in_file("use errors\n\nfn f() -> (err: error) {\n    err = errors.make(io.timeout)\n}"), NOTHING);
    assert_eq!(in_file("use errors\n\nfn f() -> (err: error) {\n    err = errors.make(category: anything)\n}"), NOTHING);
    assert_eq!(in_file("use errors\n\nfn f() -> (err: error) {\n    err = errors.make(\"io.timeout\")\n}"), ["LC3001"]);
    assert_eq!(
        in_file(
            "fn f(e: error) -> (x: i32) {\n    x = handle e {\n        ok => yield 1\n        io.timeout => yield 2\n        visa.*(cause) => yield 3\n        code -5 (other) => yield 4\n        _(all) => yield 5\n    }\n}"
        ),
        NOTHING
    );
}

#[test]
fn a_panel_element_is_named_by_the_section() {
    let panel = "\n\npanel {\n    control stop: button\n}";
    assert_eq!(in_file(&format!("use ui\n\nfn f() -> (x: bool) {{\n    x = ui.get(stop)\n}}{panel}")), NOTHING);
    assert_eq!(in_file(&format!("use ui\n\nfn f() -> (x: bool) {{\n    x = ui.get(go)\n}}{panel}")), ["LC2010"]);
    assert_eq!(in_file("use ui\n\nfn f() -> (x: bool) {\n    x = ui.get(stop)\n}"), ["LC2010"]);
    assert_eq!(in_file(&format!("use ui\n\nfn f() -> (x: bool) {{\n    x = ui.get(stop.on)\n}}{panel}")), ["LC2010"]);
    assert_eq!(in_file(&format!("use ui\n\nfn f() -> (x: bool) {{\n    x = ui.get(\"stop\")\n}}{panel}")), ["LC3001"]);
    assert_eq!(
        in_file("use ui\n\nfn f(stop: bool) -> (x: bool) {\n    x = ui.get(stop)\n}\n\npanel {\n    control stop: button\n}"),
        NOTHING,
        "R-1, an input of its name"
    );
}

#[test]
fn a_call_with_type_arguments() {
    assert_eq!(in_file("use array\n\nfn f(v: [f64]) -> (n: i32) {\n    n = array.len<f64>(v)\n}"), NOTHING);
    assert_eq!(in_file("use array\n\nfn f(v: [f64]) -> (n: i32) {\n    n = array.len<f64, f64>(v)\n}"), ["LC3019"]);
    assert_eq!(in_file("use array\n\nfn f(v: [f64]) -> (n: i32) {\n    n = array.len<nowhere>(v)\n}"), ["LC2003"]);
    assert_eq!(in_body("x, _ = g<i32>(1)"), ["LC3019"], "not generic");
    assert_eq!(in_file("fn f(v: f64) -> (x: Reading) {\n    x = Reading.Value<f64>(v)\n}"), ["LC3019"]);
    let generic = ("v.lcf", "fn v<T>(a: T) -> (r: T) {\n    r = a\n}");
    assert_eq!(codes(&[generic, ("f.lcf", "fn f() -> (r: i32) {\n    r = v<i32, i32>(1)\n}")]), ["LC4006"], "it has its own diagnostic");
}

#[test]
fn a_carry_can_have_a_type() {
    assert_eq!(in_body("x = loop carry a: i32 = 1 {\nnext a = 1\nuntil true\n}"), NOTHING);
    assert_eq!(in_body("x = loop carry a: nowhere = 1 {\nnext a = 1\nuntil true\n}"), ["LC2003"]);
    assert_eq!(
        in_file("fn f() -> (x: i32) {\n    x = loop carry a: visa.session = 1 {\n        next a = 1\n        until true\n    }\n}"),
        ["LC2005"]
    );
    assert_eq!(
        in_file("fn f<T>(p: T) -> (x: T) {\n    x = loop carry a: T = p {\n        next a = p\n        until true\n    }\n}"),
        ["LC4006"],
        "a generic function may name its variable"
    );
}

#[test]
fn a_structure_gives_as_many_values_as_it_has_targets() {
    assert_eq!(in_body("a, b = match p {\n1 => yield (1, 2)\n_ => yield (3, 4)\n}"), NOTHING);
    assert_eq!(in_body("a = match p {\n1 => yield (1, 2)\n_ => yield (3, 4)\n}"), ["LC2103"]);
    assert_eq!(in_body("a, b, c = handle err {\n_ => yield (3, 4)\n}"), ["LC2103"]);
    assert_eq!(
        in_body("a = match p {\n1 => yield 1\n_ => yield (3, 4)\n}"),
        ["LC3006"],
        "not `LC2103`: arms that disagree say nothing of the targets"
    );
    assert_eq!(in_body("a, b = loop carry c = 1 {\nnext c = 1\ncollect c\nuntil true\n}"), NOTHING, "the carries, then the collects");
    assert_eq!(in_body("a = loop carry c = 1 {\nnext c = 1\ncollect c\nuntil true\n}"), ["LC2103"]);
    assert_eq!(in_body("a, b = for i in 0..3 {\ncollect i\n}"), ["LC2103"]);
    assert_eq!(in_body("a, b = p"), ["LC2103"]);
}

#[test]
fn a_loop_that_gives_nothing_is_written_with_one_skip() {
    assert_eq!(in_body("_ = for i in 0..3 {\ng(i)\n}"), NOTHING);
    assert_eq!(in_body("_ = loop {\ng(1)\nuntil true\n}"), NOTHING);
    assert_eq!(in_body("x = for i in 0..3 {\ng(i)\n}"), ["LC2103"], "a name would stand for nothing");
    assert_eq!(in_body("_, _ = for i in 0..3 {\ng(i)\n}"), ["LC2103"]);
    assert_eq!(in_body("_, _ = for i in 0..3 {\ncollect i\n}"), ["LC2103"]);
    assert_eq!(in_body("_ = g(1)"), ["LC2103"], "a call alone is written alone (R-16)");
}

#[test]
fn a_number_has_bounds_and_its_unit_is_known() {
    assert_eq!(in_body("x = 1e999"), ["LC1006"]);
    assert_eq!(in_body("x = 1e308"), NOTHING);
    assert_eq!(in_body("x = 1e-999"), NOTHING, "too small to tell from zero");
    let wide = |ty: &str, number: &str| in_file(&format!("fn f() -> (x: {ty}) {{\n    x = {number}\n}}"));
    assert_eq!(wide("u64", "18_446_744_073_709_551_615"), NOTHING, "the largest `u64`");
    assert_eq!(wide("u64", "18_446_744_073_709_551_616"), ["LC1006"]);
    assert_eq!(wide("i64", "-9223372036854775808"), NOTHING, "the smallest `i64`");
    assert_eq!(wide("i64", "-9223372036854775809"), ["LC1006"]);
    assert_eq!(in_body("x = 000000000000000000000000000000000000000"), NOTHING);
    assert_eq!(in_body("x = 3 Vx"), ["LC2008"]);
    assert_eq!(in_body("x = 3 m^999"), ["LC1006"]);
    assert_eq!(in_body("x = [1e999]"), ["LC1006"]);
    assert_eq!(in_body("x = match p {\n99999999999999999999..=1 => yield 1\n_ => yield 2\n}"), ["LC1006"]);
    assert_eq!(in_body("x = handle err {\ncode 99999999999999999999 => yield 1\n_ => yield 2\n}"), ["LC1006"]);
    assert_eq!(in_body("x = match p {\n1e999 => yield 1\n_ => yield 2\n}"), ["LC1006"]);
    assert_eq!(in_file("fn f(a: f64 = 1e999, b: f64[V] = 1 Vx, c: [f64] = [1e999]) -> () {}"), ["LC1006", "LC2008", "LC1006"]);
    assert_eq!(in_file("extern rust(crate: \"c\", function: \"f\")\nfn f(a: f64 = 1e999) -> (err: error)"), ["LC1006"]);
}

#[test]
fn a_function_that_calls_itself_is_a_warning_on_its_first_file() {
    let found = diagnostics(&[("f.lcf", "fn f(n: i32) -> (x: i32) {\n    x = f(n)\n}")]);
    let [diagnostic] = found.as_slice() else { panic!("{found:?}") };
    assert_eq!(diagnostic.code(), Code::Lc2016);
    assert_eq!(diagnostic.function.as_deref(), Some("f"));
    assert_eq!(diagnostic.span.unwrap().start.line, 4);
    let found = diagnostics(&[
        ("g.lcf", "fn g() -> () {\n    h()\n}"),
        ("h.lcf", "fn h() -> () {\n    f()\n}"),
        ("f.lcf", "fn f() -> () {\n    g()\n    g()\n}"),
    ]);
    let [diagnostic] = found.as_slice() else { panic!("{found:?}") };
    assert_eq!((diagnostic.code(), diagnostic.file.as_str()), (Code::Lc2016, "f.lcf"));
    assert_eq!(diagnostic.span.unwrap().start.line, 4, "its first call in the cycle");
    let related: Vec<&str> = diagnostic.related.iter().map(|related| related.file.as_str()).collect();
    assert_eq!(related, ["g.lcf", "h.lcf"]);
}

/// Its calls are kept by name, in no order.
#[test]
fn the_place_of_a_cycle_is_the_first_call_whatever_the_order_of_a_map() {
    for _ in 0..20 {
        let found = diagnostics(&[
            ("f.lcf", "fn f() -> () {\n    h()\n    g()\n}"),
            ("g.lcf", "fn g() -> () {\n    f()\n}"),
            ("h.lcf", "fn h() -> () {\n    f()\n}"),
        ]);
        let [diagnostic] = found.as_slice() else { panic!("{found:?}") };
        assert_eq!(diagnostic.span.unwrap().start.line, 4);
    }
}

#[test]
fn an_extern_defined_twice_is_what_a_call_reaches() {
    let external = "extern rust(crate: \"c\", function: \"f\")\nfn dup() -> (err: error)";
    let files = [("a.lcf", external), ("z.lcf", "fn dup() -> (err: error) {\n    err = dup()\n}")];
    assert_eq!(codes(&files), ["LC2013"], "no cycle: the call binds to the first one");
}

#[test]
fn only_a_cycle_is_recursion() {
    assert_eq!(
        codes(&[("f.lcf", "fn f() -> () {\n    g()\n    g()\n}"), ("g.lcf", "fn g() -> () {\n    h()\n}"), ("h.lcf", "fn h() -> () {}")]),
        NOTHING
    );
    assert_eq!(codes(&[("f.lcf", "fn f() -> () {\n    x = f\n}")]), NOTHING, "a reference is no call");
}

#[test]
fn a_dependency_is_read_for_what_it_declares() {
    struct One;
    impl Libraries for One {
        fn find(&self, name: &str, _: Origin) -> Found {
            if name == "lib" {
                Found::Library(source("lib", &[("<lib>/g.lcf", "fn g() -> (x: i32) {\n    x = nowhere\n}")]))
            } else {
                Found::NotFound
            }
        }
    }
    let mut names = Interner::new();
    let root = source("", &[("f.lcf", "use lib\n\nfn f() -> (x: i32) {\n    x = lib.g()\n}")]);
    let checked = check(root, &One, &mut names, &Limits::default());
    assert_eq!(checked.report.errors(), 0);
    assert!(checked.dependencies.is_empty(), "its bodies are checked when it is itself the library being checked");
}

#[test]
fn what_a_call_binds_to_is_there_for_the_layers_above() {
    let text = "use core\n\nfn f(a: f64) -> (x: f64) {\n    x = core.with_unit(a, [V], after: a)\n}";
    let (checked, _) = run(source("", &[("f.lcf", text)]));
    let file = checked.files.first().unwrap();
    let Decl::Function(function) = &file.file.decl else { panic!() };
    let StatementKind::Binding { value: Rhs::Call(call), .. } = &function.body.statements.first().unwrap().kind else { panic!() };
    let resolved = file.resolved.call(call.id).unwrap();
    assert!(matches!(resolved.callee, Meaning::Primitive { library: PrimitiveLibrary::Core, .. }));
    assert_eq!(&*resolved.terminals, [Terminal::Param(0), Terminal::Param(1), Terminal::Order]);
    let Arg::Value { value: Operand::Bracket(id, _), .. } = call.args.get(1).unwrap() else { panic!() };
    assert_eq!(file.resolved.of(*id), Meaning::Unit);
}

/// What each kind of item comes to mean, counted, so that a meaning that is no longer recorded fails here.
#[test]
fn each_kind_of_item_has_its_meaning() {
    let text = "use array\nuse errors\nuse ui\n\nfn f(p: [f64], h: i32, n: i32, r: Reading, err: error = none) -> (x: Point, err: error) {\n    #a: l = array.len(array: [h])\n    m = array.len([1])\n    o = array.len([g])\n    e = errors.make(io.timeout)\n    s = ui.get(stop, after: #a)\n    c = loop carry t: i32 = n {\n        next t = n\n        until true\n    }\n    v = Reading.Overload\n    w = match r {\n        Reading.Overload => yield 1\n        _ => yield 2\n    }\n    y = handle e {\n        io.timeout => yield 1\n        _ => yield 2\n    }\n    x = Point { x: 1.0, y: 2.0 }\n    err = err\n}\n\npanel {\n    control stop: button\n}";
    let (checked, _) = run(source(
        "",
        &[
            ("f.lcf", text),
            ("reading.lct", "type Reading = enum { Value(f64), Overload }"),
            ("point.lct", "type Point = struct { x: f64, y: f64 }"),
            ("g.lcf", "fn g() -> () {}"),
        ],
    ));
    assert_eq!(checked.report.errors(), 0, "{:?}", checked.report.diagnostics());
    let file = checked.files.iter().find(|file| file.shown == "f.lcf").unwrap();
    let count = |kind: fn(&Meaning) -> bool| file.resolved.all().iter().filter(|meaning| kind(meaning)).count();
    assert_eq!(count(|meaning| matches!(meaning, Meaning::Label(_))), 1);
    assert_eq!(count(|meaning| matches!(meaning, Meaning::Element(_))), 1);
    assert_eq!(count(|meaning| matches!(meaning, Meaning::Category)), 2, "in an argument, and in a pattern");
    assert_eq!(count(|meaning| matches!(meaning, Meaning::ArrayOfValue(_))), 1);
    assert_eq!(count(|meaning| matches!(meaning, Meaning::ArrayOfNumber)), 1);
    assert_eq!(count(|meaning| matches!(meaning, Meaning::ArrayOfFunction { .. })), 1);
    assert_eq!(count(|meaning| matches!(meaning, Meaning::Type(_))), 2, "a carry, and a struct construction");
    assert_eq!(count(|meaning| matches!(meaning, Meaning::Variant { .. })), 2, "an operand, and a pattern");
    let Decl::Function(function) = &file.file.decl else { panic!() };
    let StatementKind::Binding { value: Rhs::Call(call), .. } = &function.body.statements.first().unwrap().kind else { panic!() };
    assert_eq!(&*file.resolved.call(call.id).unwrap().terminals, [Terminal::Param(0)], "by name too");
}

/// What a hostile file can repeat costs what it holds, not its square: a call does not walk the signature it binds to, nor a
/// variant the enumeration it names, nor a label or a value the bodies around it. No time is asserted, which would depend on
/// the machine: a quadratic cost shows as a test that takes minutes instead of a second.
#[test]
fn nothing_here_is_quadratic() {
    let count = 40_000;
    let params: Vec<String> = (0..count).map(|index| format!("p{index}: i32")).collect();
    let many = format!("fn many({}) -> () {{}}", params.join(", "));
    let calls = "many()\n".repeat(count);
    let variants: Vec<String> = (0..count).map(|index| format!("V{index}")).collect();
    let big = format!("type Big = enum {{ {} }}", variants.join(", "));
    let reads = format!("_ = Big.V{}\n", count - 1).repeat(count);
    let found = codes(&[
        ("many.lcf", &many),
        ("big.lct", &big),
        ("f.lcf", &format!("fn f() -> () {{\n{calls}}}")),
        ("g.lcf", &format!("fn g() -> () {{\n{reads}}}")),
    ]);
    assert_eq!(found.len(), count.min(1000), "each call has one `LC2101`, and the report keeps a thousand");
}

/// Each mistake of a line names that line and its function, and a call its missing input. Copied whole before the report cuts
/// them, as `nothing_here_is_quadratic` says, these would take minutes: 200,000 times 400 KB, or 1 MiB.
#[test]
fn what_a_diagnostic_repeats_is_cut_before_it_is_copied() {
    let args = vec!["q"; 200_000].join(", ");
    let found = diagnostics(&[("f.lcf", &format!("fn f() -> () {{\n    f({args})\n}}"))]);
    let node = found.first().unwrap().node.as_ref().unwrap();
    assert_eq!(node.text.chars().count(), labcraft_types::MAX_TEXT);

    let long = "n".repeat(1 << 20);
    let calls = "z()\n".repeat(200_000);
    let found = diagnostics(&[("f.lcf", &format!("fn {long}() -> () {{\n{calls}}}"))]);
    assert_eq!(found.first().unwrap().function.as_ref().unwrap().chars().count(), labcraft_types::MAX_TEXT);

    let params: Vec<String> = (0..200_000).map(|index| format!("p{index}: Nope")).collect();
    let found = diagnostics(&[("f.lcf", &format!("fn {long}({}) -> () {{}}", params.join(", ")))]);
    assert_eq!(found.first().unwrap().function.as_ref().unwrap().chars().count(), labcraft_types::MAX_TEXT, "pass 2 too");

    let calls = "m()\n".repeat(200_000);
    let found = diagnostics(&[("m.lcf", &format!("fn m({long}: i32) -> () {{}}")), ("f.lcf", &format!("fn f() -> () {{\n{calls}}}"))]);
    assert_eq!(found.len(), 1000);
    assert!(found.first().unwrap().message.chars().count() <= labcraft_types::MAX_TEXT);
}
