//! Canonical form, rule by rule (spec 5.2): what `fmt` makes of a file, line order and layout of the text.

use labcraft_format::{Unorderable, canonical_order, format, read, write_line};
use labcraft_graph::{Decl, Function};
use labcraft_types::{Code, Interner, Limits};

struct Formatted {
    text: String,
    codes: Vec<Code>,
}

fn formatted(input: &str) -> Formatted {
    let mut names = Interner::new();
    let result = format(input.as_bytes(), &mut names, &Limits::default());
    let codes = result.report.diagnostics().iter().map(|diagnostic| diagnostic.code()).collect();
    let text = result.text.unwrap_or_else(|| panic!("{input:?}: {:?}", result.report.diagnostics()));
    Formatted { text, codes }
}

/// `input` gives `expected`, which then stays as it is.
#[track_caller]
fn assert_canonical(input: &str, expected: &str) {
    let result = formatted(input);
    assert_eq!(result.text, expected, "{input:?}");
    assert_eq!(formatted(expected).text, expected, "not a fixed point");
}

#[track_caller]
fn assert_stays(text: &str) {
    assert_canonical(text, text);
}

/// A function with this body, indented, whose signature leaves the names free.
fn function(lines: &[&str]) -> String {
    let mut text = "labcraft 1\n\nfn f() -> () {\n".to_owned();
    for line in lines {
        if !line.is_empty() {
            text.push_str("    ");
            text.push_str(line);
        }
        text.push('\n');
    }
    text.push_str("}\n");
    text
}

#[track_caller]
fn assert_body(input: &[&str], expected: &[&str]) {
    assert_canonical(&function(input), &function(expected));
}

#[track_caller]
fn assert_body_stays(lines: &[&str]) {
    assert_body(lines, lines);
}

// Order of the lines.

#[test]
fn a_default_value_is_written_like_the_statement_it_could_be() {
    assert_stays("labcraft 1\n\nfn f(a: M = M.Fast, b: R = R.Value(0 V), c: L = L { low: 1 V }, d: F = core.lt) -> () {}\n");
    assert_canonical(
        "labcraft 1\n\nfn f(a:M=M . Fast,b:R=R.Value( 0V ,),c:L=L{low:1 V,},) -> () {}\n",
        "labcraft 1\n\nfn f(a: M = M.Fast, b: R = R.Value(0 V), c: L = L { low: 1 V }) -> () {}\n",
    );
}

#[test]
fn a_default_value_stays_whole_on_the_line_of_its_parameter() {
    let long = "a_field_with_a_long_name: 1000 V, another_field_with_a_long_name: 2000 V, a_third_field_with_a_long_name: 3000 V";
    let text = format!("labcraft 1\n\nfn f(\n    limits: Limits = Limits {{ {long} }},\n    b: f64,\n) -> () {{}}\n");
    assert_stays(&text);
    let across = format!("labcraft 1\n\nfn f(limits: Limits = Limits {{\n{}\n}}, b: f64) -> () {{}}\n", long.replace(", ", ",\n"));
    assert_canonical(&across, &text);
}

#[test]
fn ordinary_lines_go_by_name_once_their_inputs_are_there() {
    assert_body(&["b = g()", "a = f()"], &["a = f()", "b = g()"]);
    assert_body(&["a = f(b)", "b = g()"], &["b = g()", "a = f(b)"]);
    assert_body(&["c = h(a)", "b = g()", "a = f()"], &["a = f()", "b = g()", "c = h(a)"]);
    // Code point order: `Z` before `a`, and `é` after both.
    assert_body(&["é = f()", "a = f()", "Z = f()"], &["Z = f()", "a = f()", "é = f()"]);
}

#[test]
fn the_name_of_a_line_is_its_first_output_that_counts_or_else_its_label() {
    assert_body(&["_, b = g()", "a = f()"], &["a = f()", "_, b = g()"]);
    assert_body(&["#z: a = f()", "#y: b = g()"], &["#z: a = f()", "#y: b = g()"]);
    // A label sorts by its name: the `#` is no part of it, as in `layout`, where it is part of the text.
    assert_body(&["#c: h()", "b = g()", "#a: f()"], &["#a: f()", "b = g()", "#c: h()"]);
    // A call without a label has the empty name, and comes first; ties keep the written order.
    assert_body(&["a = f()", "h()", "g()"], &["h()", "g()", "a = f()"]);
    assert_body(&["a = f()", "_ = h()", "_, _ = g()"], &["_ = h()", "_, _ = g()", "a = f()"]);
}

#[test]
fn a_label_counts_as_a_name_and_only_as_a_label() {
    assert_body(&["#b: f(after: #a)", "#a: g()"], &["#a: g()", "#b: f(after: #a)"]);
    assert_body(&["#b: f(after: (x, #a))", "#a: g()", "x = h()"], &["#a: g()", "x = h()", "#b: f(after: (x, #a))"]);
    // `#a` and `a` are not the same thing.
    assert_body(&["#b: f(after: a)", "#a: g()"], &["#a: g()", "#b: f(after: a)"]);
    assert_body(&["#a: f(after: #b)", "#b: g()", "b = h()"], &["#b: g()", "#a: f(after: #b)", "b = h()"]);
    assert_body_stays(&["#a: f()", "a = g()"]);
}

#[test]
fn every_kind_of_operand_reads_its_name() {
    for reader in [
        "a = f(z.x)",
        "a = f(k: z)",
        "a = f(after: z)",
        "a = f([z])",
        "a = f([z, 1])",
        "a = f([[z]])",
        "a = z",
        "a = z.y",
        "a = S { k: z }",
        "a = [z]",
    ] {
        assert_body(&[reader, "z = g()"], &["z = g()", reader]);
    }
    assert_body(&["a = match z {", "    _ => yield 1", "}", "z = g()"], &["z = g()", "a = match z {", "    _ => yield 1", "}"]);
    assert_body(&["a = match c {", "    _ => yield z", "}", "z = g()"], &["z = g()", "a = match c {", "    _ => yield z", "}"]);
    assert_body(&["a = match c {", "    [z] => yield 1", "}", "z = g()"], &["z = g()", "a = match c {", "    [z] => yield 1", "}"]);
    assert_body(&["a = for i in z {", "}", "z = g()"], &["z = g()", "a = for i in z {}"]);
    assert_body(&["a = for i in 0..z {", "}", "z = g()"], &["z = g()", "a = for i in 0..z {}"]);
    assert_body(&["a = for i in b, carry c = z {", "}", "z = g()"], &["z = g()", "a = for i in b, carry c = z {}"]);
    assert_body(&["a = loop carry c = z {", "    until c", "}", "z = g()"], &["z = g()", "a = loop carry c = z {", "    until c", "}"]);
}

#[test]
fn what_a_unit_names_reads_nothing_but_the_bracket_of_one_name() {
    // A unit in brackets of more than one term cannot be an array.
    assert_body(&["a = f([m/s])", "m = g()"], &["a = f([m/s])", "m = g()"]);
    assert_body(&["a = f([m^2])", "m = g()"], &["a = f([m^2])", "m = g()"]);
    // A name after a number is always a unit (R-5).
    assert_body(&["a = f(1 m)", "m = g()"], &["a = f(1 m)", "m = g()"]);
    assert_body(&["a = f([m])", "m = g()"], &["m = g()", "a = f([m])"]);
    // The name of an argument, of a callee and of a struct are no values.
    assert_body(&["a = m(1)", "m = g()"], &["a = m(1)", "m = g()"]);
    assert_body(&["a = f(m: 1)", "m = g()"], &["a = f(m: 1)", "m = g()"]);
    assert_body(&["a = m { k: 1 }", "m = g()"], &["a = m { k: 1 }", "m = g()"]);
}

#[test]
fn a_list_of_patterns_keeps_its_order_on_one_line() {
    assert_body(&["a = match c {", "    _,1 , 3..=5,8 => yield 1", "}"], &["a = match c {", "    _, 1, 3..=5, 8 => yield 1", "}"]);
    assert_body(&["a = match c {", "    2, 1 => {", "        yield 1", "    }", "}"], &["a = match c {", "    2, 1 => yield 1", "}"]);
    // Past 100 characters, it stays on its line (5.2).
    let long = format!("    {} => yield 1", ["State.LongVariantName"; 6].join(", "));
    assert_body(&["a = match c {", &long, "}"], &["a = match c {", &long, "}"]);
}

#[test]
fn a_name_that_a_sub_body_binds_is_not_read_from_outside() {
    // By a carry, even when its first value reads the name outside (R-3).
    assert_body(
        &["a = loop carry z = 1 {", "    y = g(z)", "    until y", "}", "z = g()"],
        &["a = loop carry z = 1 {", "    y = g(z)", "    until y", "}", "z = g()"],
    );
    assert_body(&["a = loop carry z = z {", "    until z", "}", "z = g()"], &["z = g()", "a = loop carry z = z {", "    until z", "}"]);
    // By a `for`.
    assert_body(&["a = for z in b {", "    collect z", "}", "z = g()"], &["a = for z in b {", "    collect z", "}", "z = g()"]);
    // By a pattern.
    assert_body(&["a = match c {", "    R.V(z) => yield z", "}", "z = g()"], &["a = match c {", "    R.V(z) => yield z", "}", "z = g()"]);
    assert_body(&["a = handle c {", "    _(z) => yield z", "}", "z = g()"], &["a = handle c {", "    _(z) => yield z", "}", "z = g()"]);
    // A list binds no name, and an array in it reads one.
    assert_body(&["a = match c {", "    [z], _ => yield 1", "}", "z = g()"], &["z = g()", "a = match c {", "    [z], _ => yield 1", "}"]);
    assert_body(&["a = match c {", "    1, [z] => yield 1", "}", "z = g()"], &["z = g()", "a = match c {", "    1, [z] => yield 1", "}"]);
    // By one of its lines, wherever it is.
    assert_body(
        &["a = match c {", "    _ => {", "        y = f(z)", "        z = g()", "        yield y", "    }", "}", "z = h()"],
        &["a = match c {", "    _ => {", "        z = g()", "        y = f(z)", "        yield y", "    }", "}", "z = h()"],
    );
}

#[test]
fn a_read_in_a_deeper_body_counts_at_the_level_that_defines_it() {
    assert_body(
        &[
            "a = match c {",
            "    _ => {",
            "        x = for i in y {",
            "            collect z",
            "        }",
            "        y = g()",
            "        yield x",
            "    }",
            "}",
            "z = h()",
        ],
        &[
            "z = h()",
            "a = match c {",
            "    _ => {",
            "        y = g()",
            "        x = for i in y {",
            "            collect z",
            "        }",
            "        yield x",
            "    }",
            "}",
        ],
    );
}

#[test]
fn next_collect_and_until_come_after_the_ordinary_lines() {
    assert_body(
        &[
            "x = loop carry b = 0, carry a = 0 {",
            "    until s",
            "    collect b",
            "    collect a",
            "    next b = n",
            "    next a = n",
            "    s = f(n)",
            "    n = g()",
            "}",
        ],
        &[
            "x = loop carry b = 0, carry a = 0 {",
            "    n = g()",
            "    s = f(n)",
            "    next a = n",
            "    next b = n",
            "    collect b",
            "    collect a",
            "    until s",
            "}",
        ],
    );
    // The first output that is not `_` decides.
    assert_body(
        &["x = loop carry z = 0 {", "    next z, b = f()", "    _, next z2, c = f()", "    a, next z3 = f()", "    until a", "}"],
        &["x = loop carry z = 0 {", "    a, next z3 = f()", "    next z, b = f()", "    _, next z2, c = f()", "    until a", "}"],
    );
    // A `next` and an `until` wait for what they read, like any line.
    assert_body(
        &["x = loop carry z = 0 {", "    until s", "    s = g(c)", "    c = 1", "}"],
        &["x = loop carry z = 0 {", "    c = 1", "    s = g(c)", "    until s", "}"],
    );
}

#[test]
fn output_lines_come_last_in_signature_order_in_the_function_body_only() {
    let text =
        "labcraft 1\n\nfn f() -> (a: i32, b: i32, err: error) {\n    err = e\n    b = y\n    a = x\n    y = h()\n    x = g()\n    e = k()\n}\n";
    let expected =
        "labcraft 1\n\nfn f() -> (a: i32, b: i32, err: error) {\n    e = k()\n    x = g()\n    y = h()\n    a = x\n    b = y\n    err = e\n}\n";
    assert_canonical(text, expected);
    // A node that produces an output is an ordinary line.
    assert_canonical(
        "labcraft 1\n\nfn f() -> (a: i32, z: i32) {\n    z = 1\n    a = g()\n}\n",
        "labcraft 1\n\nfn f() -> (a: i32, z: i32) {\n    a = g()\n    z = 1\n}\n",
    );
    // An output read back is waited for.
    assert_canonical(
        "labcraft 1\n\nfn f() -> (b: i32) {\n    a = g(b)\n    b = h()\n}\n",
        "labcraft 1\n\nfn f() -> (b: i32) {\n    b = h()\n    a = g(b)\n}\n",
    );
    // A name that is both an input and an output: reading it designates the input (R-2).
    assert_stays("labcraft 1\n\nfn f(err: error) -> (err: error) {\n    a = g(err)\n    err = a\n}\n");
    assert_stays("labcraft 1\n\nfn f(err: error) -> (err: error) {\n    a = g(err)\n    err = h(a)\n}\n");
    // In an arm, naming a value is an ordinary line.
    assert_body(
        &["x = match c {", "    _ => {", "        z = 1", "        a = g()", "        yield a", "    }", "}"],
        &["x = match c {", "    _ => {", "        a = g()", "        z = 1", "        yield a", "    }", "}"],
    );
}

#[test]
fn comments_move_with_their_statement() {
    assert_body(
        &["// b", "b = g() // b too", "// a", "a = f() // a too", "// end"],
        &["// a", "a = f() // a too", "// b", "b = g() // b too", "// end"],
    );
}

#[track_caller]
fn assert_keeps_its_order(lines: &[&str]) {
    let input = function(lines);
    let result = formatted(&input);
    assert_eq!(result.text, input);
    assert_eq!(result.codes, [Code::Lc6002], "{input}");
}

#[test]
fn a_body_that_cannot_be_ordered_keeps_its_written_order() {
    assert_keeps_its_order(&["b = f(a)", "a = g(b)"]);
    assert_keeps_its_order(&["c = f(a)", "b = f(c)", "a = g(b)"]);
    assert_keeps_its_order(&["b = f()", "a = g()", "b = h()"]);
    assert_keeps_its_order(&["b, a = f()", "a = g()"]);
    assert_keeps_its_order(&["#b: f()", "#a: g()", "#b: h()"]);
    assert_keeps_its_order(&["#b: f(after: #a)", "#a: g(after: #b)"]);
    // A line that reads itself waits for no other line: `check` reports it.
    let input = function(&["b = f(b)", "a = g()"]);
    let result = formatted(&input);
    assert_eq!(result.text, function(&["a = g()", "b = f(b)"]));
    assert_eq!(result.codes, []);
}

#[test]
fn only_the_body_that_cannot_be_ordered_keeps_its_order() {
    let lines = ["x = match c {", "    _ => {", "        b = f(a)", "        a = g(b)", "        yield a", "    }", "}", "e = g()", "d = g()"];
    let result = formatted(&function(&lines));
    assert_eq!(
        result.text,
        function(&["d = g()", "e = g()", "x = match c {", "    _ => {", "        b = f(a)", "        a = g(b)", "        yield a", "    }", "}"])
    );
    assert_eq!(result.codes, [Code::Lc6002]);
    // The arm is ordered and its parent is not.
    let lines = ["b = f(a)", "a = g(b)", "x = match c {", "    _ => {", "        z = g()", "        y = g()", "        yield y", "    }", "}"];
    let result = formatted(&function(&lines));
    assert_eq!(
        result.text,
        function(&["b = f(a)", "a = g(b)", "x = match c {", "    _ => {", "        y = g()", "        z = g()", "        yield y", "    }", "}"])
    );
    assert_eq!(result.codes, [Code::Lc6002]);
}

fn function_of(text: &str, names: &mut Interner) -> Function {
    let reading = read(text.as_bytes(), names, &Limits::default());
    match reading.file.unwrap().decl {
        Decl::Function(function) => function,
        other => panic!("{other:?}"),
    }
}

#[test]
fn canonical_order_says_what_it_finds() {
    let order = |text: &str, with_signature: bool| {
        let mut names = Interner::new();
        let model = function_of(text, &mut names);
        canonical_order(&model.body, with_signature.then_some(&model.signature), &names)
    };
    assert_eq!(order(&function(&["c = h(a)", "b = g()", "a = f()"]), true), Ok(vec![2, 1, 0]));
    assert_eq!(order(&function(&["a = f()", "b = g()"]), false), Ok(vec![0, 1]));
    assert_eq!(order(&function(&["a = f(b)", "b = g(a)"]), false), Err(Unorderable::Cycle(0)));
    // `a` waits for the cycle, and is no part of it.
    assert_eq!(order(&function(&["a = g(b)", "b = h(c)", "c = h(b)"]), false), Err(Unorderable::Cycle(1)));
    assert_eq!(order(&function(&["a = g(c)", "z = g(y)", "y = h(z)", "c = h(y)"]), false), Err(Unorderable::Cycle(1)));
    assert_eq!(order(&function(&["a = f()", "b = g()", "a = h()"]), false), Err(Unorderable::DefinedTwice(2)));
    // With the signature, an output that only names a value comes last; without it, this is an ordinary line.
    let text = "labcraft 1
fn f() -> (a: i32) {
a = 1
b = g()
}
";
    assert_eq!(order(text, false), Ok(vec![0, 1]));
    assert_eq!(order(text, true), Ok(vec![1, 0]));
}

// Sections.

#[test]
fn sections_are_separated_by_one_blank_line() {
    assert_canonical(
        "labcraft 1 // the format\nuse b\n\n\nuse a\n/// doc\nfn f() -> () {\n\n\n\n}\npanel {\n\n    control x: led\n\n}\nlayout { }\nicon 32 {\n}\n\n\n",
        "labcraft 1 // the format\n\nuse a\nuse b\n\n/// doc\nfn f() -> () {}\n\npanel {\n    control x: led\n}\n\nicon 32 {}\n",
    );
    assert_canonical("   labcraft   1   \nfn f() -> () {}", "labcraft 1\n\nfn f() -> () {}\n");
}

#[test]
fn comments_outside_a_declaration_stay_with_their_section() {
    assert_stays(
        "labcraft 1\n\n// about use\nuse a\n\n// about f\n/// doc\n// more\nfn f() -> () {}\n\n// about the panel\npanel {\n    control x: led\n}\n\n// about the end\n// and more\n",
    );
    // No comment between two `use` lines, and none on the header's next line.
    assert_canonical(
        "labcraft 1\n// a\nuse c // on c\n// b\nuse d\nfn f() -> () {}\n",
        "labcraft 1\n\n// a\n// b\nuse c // on c\nuse d\n\nfn f() -> () {}\n",
    );
}

#[test]
fn an_extern_line_and_its_fn_line_form_one_declaration() {
    assert_canonical(
        "labcraft 1\n\n/// doc\nextern   rust( crate : \"a\" ,function:\"b\" )\n\nfn f( a : i32 ) -> ( err : error ) // on it\n",
        "labcraft 1\n\n/// doc\nextern rust(crate: \"a\", function: \"b\")\nfn f(a: i32) -> (err: error) // on it\n",
    );
    let long = "a".repeat(100);
    assert_stays(&format!("labcraft 1\n\nextern rust()\nfn f(\n    {long}: i32,\n) -> ()\n"));
}

#[test]
fn use_lines_are_sorted_each_library_once() {
    assert_canonical(
        "labcraft 1\nuse b\nuse a.c\nuse a\nuse b\nuse Z\nfn f() -> () {}\n",
        "labcraft 1\n\nuse Z\nuse a\nuse a.c\nuse b\n\nfn f() -> () {}\n",
    );
    // A comment keeps a line, and only a line with a comment: nothing is lost, each library once if it can be.
    assert_canonical("labcraft 1\nuse a // one\nuse a\nfn f() -> () {}\n", "labcraft 1\n\nuse a // one\n\nfn f() -> () {}\n");
    assert_canonical("labcraft 1\nuse a\nuse a // one\nfn f() -> () {}\n", "labcraft 1\n\nuse a // one\n\nfn f() -> () {}\n");
    assert_canonical("labcraft 1\nuse a // two\nuse a // one\nfn f() -> () {}\n", "labcraft 1\n\nuse a // two\nuse a // one\n\nfn f() -> () {}\n");
}

// Spaces.

#[test]
fn spaces_follow_the_rules() {
    assert_body(
        &["x,y=f ( a,b , k : 1 , after : ( c , #d ) , )", "#l : z = a . b", "w = -1", "v=g<T,U>(1 s , 2  m / s ^ - 2)"],
        &["v = g<T, U>(1 s, 2 m/s^-2)", "w = -1", "x, y = f(a, b, k: 1, after: (c, #d))", "#l: z = a.b"],
    );
    assert_body(&["x = S{a:1,b:2,}"], &["x = S { a: 1, b: 2 }"]);
    assert_body(&["x = [ 1 ,2 ]", "y = [ ]"], &["x = [1, 2]", "y = []"]);
    assert_body(&["x = f( [ m / s ] , [ V ] , [ 1 ] )"], &["x = f([m/s], [V], [1])"]);
    assert_body(&["x = 2s", "y = 9.81m/s^2", "z = 1_000.5_0 kΩ"], &["x = 2 s", "y = 9.81 m/s^2", "z = 1_000.5_0 kΩ"]);
    assert_body(&["x = 1 (kg·m)/s"], &["x = 1 (kg·m)/s"]);
}

#[test]
fn signatures_and_types_follow_the_rules() {
    assert_canonical(
        "labcraft 1\nfn f<T:Number,U>( a : [ f64 ; 2 ] = [ 1 ] , b:string ... , c : T<U,V>[ V ] , d : fn( x : a.b , c ) -> ( d ) )->( e : struct{x:f64,y:enum{A,B(f64,g),},},){}\n",
        "labcraft 1\n\nfn f<T: Number, U>(\n    a: [f64; 2] = [1],\n    b: string...,\n    c: T<U, V>[V],\n    d: fn(x: a.b, c) -> (d),\n) -> (e: struct { x: f64, y: enum { A, B(f64, g) } }) {}\n",
    );
    assert_canonical("labcraft 1\ntype T<A:B> = struct{a:f64,}\n", "labcraft 1\n\ntype T<A: B> = struct { a: f64 }\n");
    assert_canonical("labcraft 1\ntype T=fn()->()\n", "labcraft 1\n\ntype T = fn() -> ()\n");
    assert_canonical("labcraft 1\ntype session = opaque\n", "labcraft 1\n\ntype session = opaque\n");
}

#[test]
fn literals_keep_their_spelling() {
    assert_body_stays(&["a = 1e3", "b = 1E3", "c = 1_000", "d = 007", "e = 1.50", "f = 1e-3", "g = inf", "h = -inf", "i = nan", "j = \"\\n\\\"\""]);
    assert_body_stays(&["a = f(1 m^02)", "b = g<[f64; 02]>()"]);
}

// Structures.

#[test]
fn structures_follow_the_rules() {
    assert_body(
        &["x=match a{", "1=>{ yield 1 }", "2..3=>yield(2)", "-5..=5=>yield(a,b)", "R.V ( v , w )=>{ y=f()", "yield y }", "_=>yield 1 }"],
        &[
            "x = match a {",
            "    1 => yield 1",
            "    2..3 => yield 2",
            "    -5..=5 => yield (a, b)",
            "    R.V(v, w) => {",
            "        y = f()",
            "        yield y",
            "    }",
            "    _ => yield 1",
            "}",
        ],
    );
    assert_body(
        &[
            "x=handle e{",
            "ok=>yield 1",
            "io.timeout ( e2 )=>yield 2",
            "visa.* =>yield 3",
            "code -1 ( e3 )=>yield 4",
            "code 5=>yield 5",
            "_ ( e4 )=>yield 6",
            "}",
        ],
        &[
            "x = handle e {",
            "    ok => yield 1",
            "    io.timeout(e2) => yield 2",
            "    visa.* => yield 3",
            "    code -1 (e3) => yield 4",
            "    code 5 => yield 5",
            "    _(e4) => yield 6",
            "}",
        ],
    );
    assert_body(&["x=for parallel i in a,j in 0..n,carry c:i32=1{", "}"], &["x = for parallel i in a, j in 0..n, carry c: i32 = 1 {}"]);
    assert_body(&["x=loop continue_on_error carry c=1{ until c }"], &["x = loop continue_on_error carry c = 1 {", "    until c", "}"]);
    assert_body(&["x=loop{ until c }"], &["x = loop {", "    until c", "}"]);
}

#[test]
fn empty_bodies_are_written_with_two_braces() {
    assert_body(&["x = for i in a {", "", "}"], &["x = for i in a {}"]);
    // A `match` or a `handle` is never on one line, and what is split ends with a `{` that ends its line.
    assert_body(&["y = match a {}", "z = handle a {}"], &["y = match a {", "}", "z = handle a {", "}"]);
    assert_body_stays(&["x = for", "    // above", "    i in a", "{", "}"]);
    assert_stays("labcraft 1\n\nfn f() -> (\n    // above\n    a: i32,\n) {\n}\n");
    assert_stays("labcraft 1\n\nfn f(\n    // above\n    a: i32,\n) -> () {}\n");
    assert_stays("labcraft 1\n\nfn f() -> () {} // after\n");
    assert_stays("labcraft 1\n\nfn f() -> () { // open\n}\n");
    assert_stays("labcraft 1\n\nfn f() -> () {\n    // inside\n}\n");
    assert_body_stays(&["x = for i in a { // open", "}", "y = match a { // open", "}", "z = match a {", "    // inside", "} // after"]);
}

#[test]
fn parentheses_around_a_single_value_are_dropped() {
    assert_body(&["x = f(after: (e))", "y = g(after: (#z))"], &["x = f(after: e)", "y = g(after: #z)"]);
    assert_body(&["x = match a {", "    _ => yield (b)", "}"], &["x = match a {", "    _ => yield b", "}"]);
    // The array of one value is written as an argument reads it, and they are the same thing for `check` (R-5).
    assert_body(&["x = f(after: ([e]))"], &["x = f(after: [e])"]);
    assert_body(&["x = f(after: ([1]))"], &["x = f(after: [1])"]);
    assert_body(&["x = f(after: ([e, g]))"], &["x = f(after: [e, g])"]);
}

#[test]
fn an_arm_that_is_only_a_yield_goes_on_one_line_with_its_comments_before_it() {
    assert_body(
        &[
            "x = match a {",
            "    // before",
            "    _ => { // open",
            "        // above",
            "        yield 1 // on it",
            "        // below",
            "    } // after",
            "}",
        ],
        &["x = match a {", "    // before", "    // open", "    // above", "    // on it", "    // below", "    _ => yield 1 // after", "}"],
    );
}

#[test]
fn an_arm_with_a_body_keeps_its_comments_in_their_places() {
    assert_body_stays(&[
        "x = match a {",
        "    // before",
        "    _ => { // open",
        "        // above y",
        "        y = f() // on y",
        "        // above the yield",
        "        yield y // on the yield",
        "        // below",
        "    } // after",
        "    // at the end",
        "}",
    ]);
}

// Width.

#[test]
fn a_signature_goes_on_one_line_up_to_100_code_points() {
    // With a body, the line ends with ` {`.
    let name = "a".repeat(100 - "fn f(: i32) -> () {".chars().count());
    assert_stays(&format!("labcraft 1\n\nfn f({name}: i32) -> () {{\n    x = 1\n}}\n"));
    let longer = format!("{name}a");
    assert_canonical(
        &format!("labcraft 1\n\nfn f({longer}: i32) -> () {{\n    x = 1\n}}\n"),
        &format!("labcraft 1\n\nfn f(\n    {longer}: i32,\n) -> () {{\n    x = 1\n}}\n"),
    );
    // Without one, with ` {}`, which is one more.
    let name = "a".repeat(100 - "fn f(: i32) -> () {}".chars().count());
    assert_stays(&format!("labcraft 1\n\nfn f({name}: i32) -> () {{}}\n"));
    let longer = format!("{name}a");
    assert_canonical(&format!("labcraft 1\n\nfn f({longer}: i32) -> () {{}}\n"), &format!("labcraft 1\n\nfn f(\n    {longer}: i32,\n) -> () {{}}\n"));
    // An `extern` has neither.
    let name = "a".repeat(100 - "fn f(: i32) -> ()".chars().count());
    assert_stays(&format!("labcraft 1\n\nextern rust()\nfn f({name}: i32) -> ()\n"));
    // Code points, not bytes.
    let name = "\u{00E9}".repeat(100 - "fn f(: i32) -> () {}".chars().count());
    assert_stays(&format!("labcraft 1\n\nfn f({name}: i32) -> () {{}}\n"));
    // The comment is not counted.
    let name = "a".repeat(100 - "fn f(: i32) -> () {".chars().count());
    assert_stays(&format!("labcraft 1\n\nfn f({name}: i32) -> () {{ // {}\n}}\n", "c".repeat(100)));
}

#[test]
fn a_signature_splits_its_parameters_then_its_outputs() {
    let p = "p".repeat(60);
    let o = "o".repeat(60);
    let q = "q".repeat(120);
    assert_canonical(
        &format!("labcraft 1\nfn f({p}: i32, b: i32) -> ({o}: i32) {{}}\n"),
        &format!("labcraft 1\n\nfn f(\n    {p}: i32,\n    b: i32,\n) -> ({o}: i32) {{}}\n"),
    );
    assert_canonical(
        &format!("labcraft 1\nfn f(a: i32) -> ({q}: i32) {{}}\n"),
        &format!("labcraft 1\n\nfn f(\n    a: i32,\n) -> (\n    {q}: i32,\n) {{\n}}\n"),
    );
    // No parameter: the first line keeps `fn name() -> (`.
    assert_canonical(&format!("labcraft 1\nfn f() -> ({q}: i32) {{}}\n"), &format!("labcraft 1\n\nfn f() -> (\n    {q}: i32,\n) {{\n}}\n"));
    // Nothing to split.
    let name = "n".repeat(120);
    assert_stays(&format!("labcraft 1\n\nfn {name}() -> () {{}}\n"));
    assert_stays("labcraft 1\n\nfn f( // c\n) -> () {}\n".replace("f( // c\n", "f(\n    // c\n").as_str());
}

#[test]
fn a_comment_in_a_signature_splits_it() {
    assert_stays("labcraft 1\n\nfn f(\n    a: i32, // on a\n) -> (b: i32) {}\n");
    assert_stays("labcraft 1\n\nfn f(\n    // above\n    a: i32,\n    // at the end\n) -> (b: i32) {}\n");
    assert_stays("labcraft 1\n\nfn f(\n    a: i32,\n) -> (\n    b: i32, // on b\n) {\n}\n");
    assert_stays("labcraft 1\n\nfn f() -> (\n    // above\n    b: i32,\n) {\n}\n");
    assert_stays("labcraft 1\n\nfn f() -> (\n    b: i32,\n    // at the end\n) {\n}\n");
    // In a struct of a parameter too, and only that struct is split.
    assert_stays("labcraft 1\n\nfn f(\n    a: struct {\n        // above\n        b: i32,\n    },\n    c: struct { d: i32 },\n) -> () {}\n");
    assert_canonical(
        "labcraft 1\n// before\nfn // b\n f // c\n( a: i32 ) -> () {}\n",
        "labcraft 1\n\n// before\n// b\n// c\nfn f(a: i32) -> () {}\n",
    );
}

#[test]
fn a_struct_or_an_enum_goes_on_one_line_if_the_line_fits() {
    let fields = |width: usize| "a".repeat(width);
    let wide = fields(100 - "type T = struct { : f64 }".chars().count());
    assert_stays(&format!("labcraft 1\n\ntype T = struct {{ {wide}: f64 }}\n"));
    let wider = fields(101 - "type T = struct { : f64 }".chars().count());
    assert_canonical(
        &format!("labcraft 1\n\ntype T = struct {{ {wider}: f64 }}\n"),
        &format!("labcraft 1\n\ntype T = struct {{\n    {wider}: f64,\n}}\n"),
    );
    assert_stays("labcraft 1\n\ntype T = enum { A, B(f64) } // on it\n");
    assert_stays("labcraft 1\n\ntype T = enum {\n    A, // on A\n    B(f64),\n    // at the end\n}\n");
    // A nested struct is judged on its own line, with its comma.
    let inner = fields(100 - "    a: struct { : f64 },".chars().count());
    assert_stays(&format!("labcraft 1\n\ntype T = struct {{\n    a: struct {{ {inner}: f64 }},\n    b: f64,\n}}\n"));
    let inner = fields(101 - "    a: struct { : f64 },".chars().count());
    assert_stays(&format!("labcraft 1\n\ntype T = struct {{\n    a: struct {{\n        {inner}: f64,\n    }},\n    b: f64,\n}}\n"));
    // Inside an array, a function type, the arguments of a type: the same.
    assert_stays("labcraft 1\n\ntype T = [struct {\n    // above\n    a: f64,\n}]\n");
    // The line is judged as a whole: the second one splits too, though it would fit (5.2).
    assert_stays("labcraft 1\n\ntype T = fn(struct {\n    // above\n    a: f64,\n}, struct {\n    b: f64,\n}) -> ()\n");
    assert_stays("labcraft 1\n\ntype T = enum {\n    A(struct {\n        // above\n        a: f64,\n    }),\n}\n");
}

#[test]
fn clauses_go_on_the_line_of_the_keyword_if_they_fit() {
    let prefix = "x = for i in ".len() + " {".len();
    let a = "a".repeat(100 - prefix - 4);
    assert_body_stays(&[format!("x = for i in {a} {{").as_str(), "    collect i", "}"]);
    let a = "a".repeat(100 - prefix - 3);
    assert_body(
        &[format!("x = for i in {a} {{").as_str(), "    collect i", "}"],
        &["x = for", format!("    i in {a}").as_str(), "{", "    collect i", "}"],
    );
    assert_body_stays(&[
        "x = loop continue_on_error",
        "    carry a = 1, // on a",
        "    // above b",
        "    carry b = 2 // on b",
        "    // at the end",
        "{ // open",
        "    until a",
        "} // after",
    ]);
    assert_body_stays(&["x = for parallel", "    // above", "    i in a", "{", "}"]);
    // An empty body takes one column more on the line of the keyword.
    let a = "a".repeat(100 - "    x = for i in  {}".chars().count());
    assert_body_stays(&[format!("x = for i in {a} {{}}").as_str()]);
    let a = format!("{a}a");
    assert_body(&[format!("x = for i in {a} {{}}").as_str()], &["x = for", format!("    i in {a}").as_str(), "{", "}"]);
    // A struct in a carry can be split.
    assert_body_stays(&["x = loop", "    carry a: struct {", "        // above", "        b: f64,", "    } = 1", "{", "    until a", "}"]);
    // A comment in the header goes before the statement.
    assert_body(&["x = loop // a", "    carry b = 1 {", "    until b", "}"], &["x = loop", "    // a", "    carry b = 1", "{", "    until b", "}"]);
    assert_body(&["x = match // a", "b {", "    _ => yield 1", "}"], &["// a", "x = match b {", "    _ => yield 1", "}"]);
    // Past the width of a line, every header is split, with what it can.
    let long = "n".repeat(100);
    assert_body_stays(&[format!("{long} = loop {{").as_str(), "    until a", "}"]);
}

#[test]
fn any_other_statement_stays_on_one_line_whatever_its_length() {
    let long = "a".repeat(150);
    assert_body_stays(&[format!("x = f({long}, {long})").as_str()]);
    assert_body_stays(&[format!("x = S {{ a: {long}, b: {long} }}").as_str()]);
    assert_body_stays(&[format!("x = f<struct {{ a: {long}, b: {long} }}>()").as_str()]);
    assert_body(&["x = f( // a", "b, // c", "d", ") // e"], &["// a", "// c", "x = f(b, d) // e"]);
}

// Comments.

#[test]
fn a_comment_has_at_least_one_space_and_none_at_the_end_of_the_line() {
    assert_body(
        &["//a", "//  b", "//", "//   ", "// c   ", "x = 1 //d", "y = 1 //   e  "],
        &["// a", "//  b", "//", "//", "// c", "x = 1 // d", "y = 1 //   e"],
    );
    assert_canonical("labcraft 1\n///a\n///  b\n///\n///   \nfn f() -> () {}\n", "labcraft 1\n\n/// a\n///  b\n///\n///\nfn f() -> () {}\n");
    assert_canonical("labcraft 1 //a\nfn f() -> () {}\n", "labcraft 1 // a\n\nfn f() -> () {}\n");
    // A tab is text, with a space before it, unless it ends the line.
    assert_body(&["//\ta", "// b\t", "// c\t  ", "//\t"], &["// \ta", "// b", "// c", "//"]);
    assert_canonical("labcraft 1\n////a\nfn f() -> () {}\n", "labcraft 1\n\n/// /a\nfn f() -> () {}\n");
    // A string is never touched.
    assert_body_stays(&["x = \"  a  \\t \"", "y = \"//\""]);
}

#[test]
fn names_and_units_have_one_spelling() {
    // é composed or not, and the mu of Greek as the micro sign (R-20).
    assert_body(&["e\u{0301} = 1 \u{03BC}V", "x = 1 \u{2126}"], &["x = 1 \u{03A9}", "\u{00E9} = 1 \u{00B5}V"]);
    // Strings and comments stay as they are.
    assert_body_stays(&["// e\u{0301}", "x = \"e\u{0301}\""]);
}

// Panel, layout and icon.

#[test]
fn items_of_a_panel_go_by_direction_then_signature_then_name() {
    assert_canonical(
        "labcraft 1\nfn f(a: i32, b: i32) -> (x: i32, y: i32) {}\npanel {\nindicator z: led\nindicator y: led\nindicator x: led\ncontrol z: led\ncontrol c: led\ncontrol b: led\ncontrol a: numeric( format : \"0\" ) // on a\n// above\ncontrol d: led()\n// at the end\n}\n",
        "labcraft 1\n\nfn f(a: i32, b: i32) -> (x: i32, y: i32) {}\n\npanel {\n    control a: numeric(format: \"0\") // on a\n    control b: led\n    control c: led\n    // above\n    control d: led()\n    control z: led\n    indicator x: led\n    indicator y: led\n    indicator z: led\n    // at the end\n}\n",
    );
    // An input as an indicator is no terminal of that direction; ties keep the written order.
    assert_canonical(
        "labcraft 1\nfn f(a: i32) -> () {}\npanel {\nindicator b: led\nindicator a: led\ncontrol b: led\ncontrol b: chart\n}\n",
        "labcraft 1\n\nfn f(a: i32) -> () {}\n\npanel {\n    control b: led\n    control b: chart\n    indicator a: led\n    indicator b: led\n}\n",
    );
}

fn with_nodes(section: &str) -> String {
    format!("labcraft 1\n\nfn f() -> () {{\n    #l: a = g()\n    b = g()\n    c, d = g()\n    _, e = g()\n    k = a\n    #m: h()\n}}\n\n{section}")
}

#[test]
fn items_of_a_layout_go_by_kind_then_by_the_text_of_their_references() {
    assert_canonical(
        &with_nodes(
            "layout {\nwire b.x -> #l.y\nwire b -> c\nwire b -> #m\nnode e at (1,2)\nnode c view compact\nnode b at ( 1 , 2 ) view standard\nnode #m at (3,4)\nnode #l at (3,4)\n}\n",
        ),
        &with_nodes(
            "layout {\n    node #l at (3, 4)\n    node #m at (3, 4)\n    node b at (1, 2) view standard\n    node c view compact\n    node e at (1, 2)\n    wire b -> #m\n    wire b -> c\n    wire b.x -> #l.y\n}\n",
        ),
    );
    // Ties keep the written order, and so do the points after `via`.
    assert_stays(&with_nodes("layout {\n    node b at (2, 2)\n    node b at (1, 1)\n    wire b -> c via (9, 9) (1, 1) (5, 5)\n}\n"));
}

#[test]
fn a_coordinate_in_the_range_is_written_in_plain_decimal() {
    let section = |point: &str| with_nodes(&format!("layout {{\n    node b at {point}\n}}\n"));
    assert_canonical(&section("(1_000, -0)"), &section("(1000, 0)"));
    assert_canonical(&section("(007, 1__0)"), &section("(7, 10)"));
    assert_canonical(&section("(1_000_000, -1_000_000)"), &section("(1000000, -1000000)"));
    assert_canonical(&section("(0_0, -00)"), &section("(0, 0)"));
    // Anything else keeps its spelling, for `check` to refuse.
    assert_stays(&section("(1_000_001, 1e3)"));
    assert_stays(&section("(5.0, -1_000_001)"));
    assert_stays(&section("(inf, nan)"));
    assert_stays(&section("(99_999_999_999_999_999_999_999, -0.0)"));
    // In `via` too; an icon keeps its spelling, since the rule is for `layout` (5.2).
    assert_canonical(
        &with_nodes("layout {\n    wire b -> c via (1_0, 2) (3, -0)\n}\n"),
        &with_nodes("layout {\n    wire b -> c via (10, 2) (3, 0)\n}\n"),
    );
    assert_stays(&with_nodes("icon 32 {\n    rect (1_0, 2) (3, -0) radius 01\n}\n"));
}

#[test]
fn a_layout_entry_that_names_no_node_is_removed() {
    let entry = |text: &str| with_nodes(&format!("layout {{\n    {text}\n}}\n"));
    // What each line can be called (spec 11).
    for known in ["node #l", "node b", "node c", "node e", "node #m"] {
        let text = entry(&format!("{known} at (1, 2)"));
        assert_eq!(formatted(&text).text, text, "{known}");
        assert_eq!(formatted(&text).codes, [], "{known}");
    }
    for stale in [
        "node z at (1, 2)",
        // A label takes the place of the name of the line.
        "node a at (1, 2)",
        "node #z at (1, 2)",
        // Only the first output names a line.
        "node d at (1, 2)",
        // A label is not a name, and a name is not a label.
        "node l at (1, 2)",
        "node #b at (1, 2)",
        // Naming a value creates no node.
        "node k at (1, 2)",
        "wire b -> z",
        "wire z -> b",
        "wire z.x -> b.y via (1, 1)",
    ] {
        let result = formatted(&entry(stale));
        assert_eq!(result.text, with_nodes("").trim_end_matches('\n').to_owned() + "\n", "{stale}");
        assert_eq!(result.codes, [Code::Lc5004], "{stale}");
    }
}

#[test]
fn a_layout_entry_can_name_a_constant() {
    // A name given to a constant is a node that the user places (spec 5.1, 11).
    for value in ["0.5", "240 V", "\"text\"", "true", "none", "[1, 2]", "[1]", "Verdict.Pass", "core.abs"] {
        let text = format!("labcraft 1\n\nfn f() -> () {{\n    k = {value}\n}}\n\nlayout {{\n    node k at (1, 2)\n}}\n");
        assert_stays(&text);
        assert_eq!(formatted(&text).codes, [], "{value}");
    }
    // In a nested body too, and at the end of a wire.
    let text = "labcraft 1\n\nfn f() -> () {\n    a = match c {\n        _ => {\n            k = 1\n            yield k\n        }\n    }\n    b = g(a)\n}\n\nlayout {\n    node k at (1, 2)\n    wire k -> b\n}\n";
    assert_stays(text);
    assert_eq!(formatted(text).codes, []);
    // By its label, if it has one.
    assert_stays("labcraft 1\n\nfn f() -> () {\n    #l: k = 1\n}\n\nlayout {\n    node #l at (1, 2)\n}\n");
}

#[test]
fn a_constant_on_a_terminal_designates_no_node() {
    // R-4: an output, a carry and nothing at all are not nodes, whatever their value and their label.
    for line in ["#l: x = 1", "#l: _ = 1", "#l: k = s"] {
        let result = formatted(&format!("labcraft 1\n\nfn f(s: i32) -> (x: i32) {{\n    {line}\n}}\n\nlayout {{\n    node #l at (1, 2)\n}}\n"));
        assert_eq!(result.codes, [Code::Lc5004], "{line}");
    }
    for line in ["#l: next v = 1", "#l: next v = s"] {
        let result = formatted(&format!(
            "labcraft 1\n\nfn f(s: i32) -> (x: i32) {{\n    x = loop carry v = 0 {{\n        {line}\n        until true\n    }}\n}}\n\nlayout {{\n    node #l at (1, 2)\n}}\n"
        ));
        assert_eq!(result.codes, [Code::Lc5004], "{line}");
    }
}

#[test]
fn a_layout_entry_that_names_an_input_or_an_output_stays_for_check() {
    let function = "labcraft 1\n\nfn f(a: i32) -> (err: i32) {\n    b = g(a)\n    err = b\n}\n\n";
    let text = format!(
        "{function}layout {{\n    node a at (1, 2)\n    node err at (1, 2)\n    wire a -> b via (1, 1)\n    wire a -> err\n    wire a.a -> b.x\n    wire b -> a\n    wire b -> err\n    wire err -> b\n}}\n"
    );
    assert_eq!(formatted(&text).text, text);
    assert_eq!(formatted(&text).codes, []);
    // An unknown name at the other end still makes it stale, and an input is not a label.
    for stale in ["wire a -> z", "wire z -> err", "node #a at (1, 2)", "wire #a -> b"] {
        let result = formatted(&format!("{function}layout {{\n    {stale}\n}}\n"));
        assert_eq!(result.text, function.trim_end_matches('\n').to_owned() + "\n", "{stale}");
        assert_eq!(result.codes, [Code::Lc5004], "{stale}");
    }
}

#[test]
fn a_node_in_a_nested_body_can_be_named() {
    let text = "labcraft 1\n\nfn f() -> () {\n    a = match c {\n        _ => {\n            b = g()\n            yield b\n        }\n    }\n    x = loop {\n        y = g()\n        until y\n    }\n}\n\nlayout {\n    node a at (1, 2)\n    node b at (1, 2)\n    node x at (1, 2)\n    node y at (1, 2)\n}\n";
    assert_stays(text);
    assert_eq!(formatted(text).codes, []);
}

#[test]
fn a_stale_entry_with_a_comment_stays_and_a_section_with_one_too() {
    let keep = with_nodes("layout {\n    node y at (1, 2) // why\n    // why\n    node z at (1, 2)\n}\n");
    assert_eq!(formatted(&keep).text, keep);
    assert_eq!(formatted(&keep).codes, []);
    // The other entries go, and the comment at the end keeps the section.
    let result = formatted(&with_nodes("layout {\n    node z at (1, 2)\n    // why\n}\n"));
    assert_eq!(result.text, with_nodes("layout {\n    // why\n}\n"));
    assert_eq!(result.codes, [Code::Lc5004]);
    let result = formatted(&with_nodes("// why\nlayout {\n    node z at (1, 2)\n}\n"));
    assert_eq!(result.text, with_nodes("// why\nlayout {}\n"));
    let result = formatted(&with_nodes("layout {\n    node z at (1, 2)\n} // why\n"));
    assert_eq!(result.text, with_nodes("layout {} // why\n"));
}

#[test]
fn a_function_that_cannot_be_ordered_keeps_its_layout_entries() {
    let text = "labcraft 1\n\nfn f() -> () {\n    a = g(b)\n    b = g(a)\n}\n\nlayout {\n    node z at (1, 2)\n}\n";
    let result = formatted(text);
    assert_eq!(result.text, text);
    assert_eq!(result.codes, [Code::Lc6002]);
}

#[test]
fn outside_a_function_entries_name_no_node_or_are_for_check() {
    let result = formatted("labcraft 1\n\nextern rust()\nfn f() -> ()\n\nlayout {\n    node a at (1, 2)\n}\n");
    assert_eq!(result.text, "labcraft 1\n\nextern rust()\nfn f() -> ()\n");
    assert_eq!(result.codes, [Code::Lc5004]);
    // With no node, a parameter does not keep it either (`docs/cli.md`, section 2).
    let result = formatted("labcraft 1\n\nextern rust()\nfn f(a: i32) -> ()\n\nlayout {\n    node a at (1, 2)\n}\n");
    assert_eq!(result.text, "labcraft 1\n\nextern rust()\nfn f(a: i32) -> ()\n");
    assert_eq!(result.codes, [Code::Lc5004]);
    // `LC0104` is for `check`: the section stays for it to see.
    let text = "labcraft 1\n\ntype T = f64\n\npanel {\n    control a: led\n}\n\nlayout {\n    node a at (1, 2)\n}\n\nicon 32 {}\n";
    assert_stays(text);
    assert_eq!(formatted(text).codes, []);
}

#[test]
fn an_empty_panel_or_layout_is_removed_and_an_icon_is_not() {
    assert_canonical("labcraft 1\nfn f() -> () {}\npanel {}\nlayout {\n}\nicon 32 {}\n", "labcraft 1\n\nfn f() -> () {}\n\nicon 32 {}\n");
    for section in [
        "panel {\n    // why\n}\n",
        "panel {} // why\n",
        "// why\npanel {}\n",
        "layout {\n    // why\n}\n",
        "layout {} // why\n",
        "// why\nlayout {}\n",
    ] {
        let text = format!("labcraft 1\n\nfn f() -> () {{}}\n\n{section}");
        assert_stays(&text);
    }
}

#[test]
fn items_of_an_icon_keep_their_written_order() {
    assert_canonical(
        "labcraft 1\nfn f() -> () {}\nicon 32 {\n// a\ncircle (9,9) 1 fill accent wire.error // b\nrect(1,1)(2,2)radius 3\nline (1,1) (2,2) fill\ntext \"V\" at(1,2)size 6 accent x\npath \"M0 0\"\n// c\n} // d\n",
        "labcraft 1\n\nfn f() -> () {}\n\nicon 32 {\n    // a\n    circle (9, 9) 1 fill accent wire.error // b\n    rect (1, 1) (2, 2) radius 3\n    line (1, 1) (2, 2) fill\n    text \"V\" at (1, 2) size 6 accent x\n    path \"M0 0\"\n    // c\n} // d\n",
    );
}

// The whole of it.

#[test]
fn a_text_over_the_size_limit_gets_lc6003_and_nothing_is_written() {
    let limits = |size: u64| {
        let mut limits = Limits::default();
        limits.max_file_size = size;
        limits
    };
    let input = "labcraft 1\nfn f()->(){x=1\n}\n";
    let canonical = "labcraft 1\n\nfn f() -> () {\n    x = 1\n}\n";
    assert!(input.len() < canonical.len());
    let mut names = Interner::new();
    let result = format(input.as_bytes(), &mut names, &limits(canonical.len() as u64));
    assert_eq!(result.text.as_deref(), Some(canonical));
    let result = format(input.as_bytes(), &mut names, &limits(canonical.len() as u64 - 1));
    assert_eq!(result.text, None);
    let codes: Vec<Code> = result.report.diagnostics().iter().map(|diagnostic| diagnostic.code()).collect();
    assert_eq!(codes, [Code::Lc6003]);
    assert_eq!(result.report.errors(), 1);
    // A warning met on the way stays.
    let input = "labcraft 1\nfn f()->(){x=\"\u{202E}\"\n}\n";
    let result = format(input.as_bytes(), &mut names, &limits(input.len() as u64));
    let codes: Vec<Code> = result.report.diagnostics().iter().map(|diagnostic| diagnostic.code()).collect();
    assert_eq!(result.text, None);
    assert_eq!(codes, [Code::Lc0005, Code::Lc6003]);
}

/// A file of short lines deep down, whose canonical form is hundreds of times its size.
#[test]
fn the_indentation_of_a_deep_file_cannot_make_a_huge_text() {
    std::thread::Builder::new()
        .stack_size(labcraft_format::STACK_SIZE)
        .spawn(|| {
            let depth = 256;
            let open = "a=match b{\n_=>{\n".repeat(depth - 1);
            let close = "yield a\n}\n}\n".repeat(depth - 1);
            let lines = "c=1\n".repeat(100_000);
            let input = format!("labcraft 1\nfn f()->(){{\n{open}a=match b{{\n_=>{{\n{lines}yield a\n}}\n}}\n{close}}}\n");
            assert!(input.len() < 16 * 1024 * 1024);
            let mut names = Interner::new();
            let result = format(input.as_bytes(), &mut names, &Limits::default());
            assert_eq!(result.text, None);
            let codes: Vec<Code> = result.report.diagnostics().iter().map(|diagnostic| diagnostic.code()).collect();
            // What `canonicalize` says about the text counts only if there is one: here, a value defined twice.
            assert_eq!(codes, [Code::Lc6003]);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn a_file_that_does_not_read_gets_no_text() {
    let mut names = Interner::new();
    let result = format(b"labcraft 1\nfn f(\n", &mut names, &Limits::default());
    assert_eq!(result.text, None);
    assert_eq!(result.report.errors(), 1);
}

#[test]
fn write_line_gives_the_line_of_a_statement() {
    let text = function(&[
        "#l: a, _ = f(b, k: [V], after: (c, #d))",
        "e = S { x: 1 }",
        "g = 1 V",
        "h = match i {",
        "    _ => yield 1",
        "}",
        "j = handle k {",
        "    _ => yield 1",
        "}",
        "m = for parallel n in o, carry p: struct { q: f64 } = 1 {}",
        "r = loop continue_on_error carry s = 1 {",
        "    next s = 2",
        "    collect s",
        "    until s",
        "}",
        "t()",
    ]);
    let mut names = Interner::new();
    let model = function_of(&text, &mut names);
    let lines: Vec<String> = model.body.statements.iter().map(|statement| write_line(statement, &names)).collect();
    assert_eq!(
        lines,
        [
            "#l: a, _ = f(b, k: [V], after: (c, #d))",
            "e = S { x: 1 }",
            "g = 1 V",
            "h = match i {",
            "j = handle k {",
            "m = for parallel n in o, carry p: struct { q: f64 } = 1 {",
            "r = loop continue_on_error carry s = 1 {",
            "t()",
        ]
    );
    let inner = match &model.body.statements[6].kind {
        labcraft_graph::StatementKind::Binding { value: labcraft_graph::Rhs::Loop(structure), .. } => &structure.body.statements,
        other => panic!("{other:?}"),
    };
    let lines: Vec<String> = inner.iter().map(|statement| write_line(statement, &names)).collect();
    assert_eq!(lines, ["next s = 2", "collect s", "until s"]);
}
