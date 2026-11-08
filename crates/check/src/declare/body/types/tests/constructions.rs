//! Step 8c: what a literal takes from where it goes (3.4), what builds a value (5.1), `waveform` (3.2), a primitive as a value (4)
//! and the default value of a parameter (R-21).

use super::*;

const PEOPLE: &[(&str, &str)] = &[
    ("Gain.lct", "type Gain = struct { a: f32, n: i32, compare: fn(f64, f64) -> (bool) }"),
    ("Seven.lct", "type Seven = struct { a: i32, b: i32, c: i32, d: i32, e: i32, f: i32, g: i32 }"),
    ("Volts.lct", "type Volts = f64[V]"),
    ("Trace.lct", "type Trace = waveform<f64[V]>"),
    ("Compare.lct", "type Compare = fn(f64, f64) -> (bool)"),
    ("List.lct", "type List = struct { items: [f32] }"),
    ("takes.lcf", "use core\n\nfn takes(g: f32, compare: fn(f64, f64) -> (bool) = core.lt) -> (r: f32) {\n    r = g\n}"),
];

fn all(inputs: &str, outputs: &str, body: &str) -> Vec<&'static str> {
    judged_with(PEOPLE, inputs, outputs, body)
}

fn only(inputs: &str, outputs: &str, body: &str) -> Diagnostic {
    let text = program(inputs, outputs, body);
    let mut files = vec![
        ("f.lcf", text.as_str()),
        ("Reading.lct", "type Reading = enum { Value(f64[V]), Pair(i32, string), Overload }"),
        ("Point.lct", "type Point = struct { x: f64[m], label: string }"),
    ];
    files.extend_from_slice(PEOPLE);
    let found = diagnostics(&files);
    let [diagnostic] = found.as_slice() else { panic!("{found:?}") };
    diagnostic.clone()
}

/// A parameter of `f` with this type and this default, and no other mistake.
fn with_default(param: &str) -> Vec<&'static str> {
    judged_with(PEOPLE, param, "", "")
}

#[test]
fn a_literal_takes_the_number_type_expected() {
    for (ty, literal) in [
        ("f32", "0.5"),
        ("f32[V]", "1.5 V"),
        ("f32[mV]", "1 V"),
        ("i32[mV]", "5 mV"),
        ("c128", "1.5"),
        ("c64", "1.5"),
        ("c128[V]", "1.5 V"),
        ("f32", "inf"),
        ("f32", "1e-50"),
    ] {
        assert_eq!(gives(ty, literal), NOTHING, "{literal} for {ty}");
    }
    for (ty, literal) in
        [("i32", "1.5"), ("i32", "1.0"), ("i32", "inf"), ("i32[mV]", "1 V"), ("i32[mV]", "5.0 mV"), ("f32[V]", "1.5"), ("f32", "1 V")]
    {
        assert_ne!(gives(ty, literal), NOTHING, "{literal} for {ty}");
    }
    assert_eq!(gives("i32", "1.5"), ["LC3003"]);
    assert_eq!(gives("i32[mV]", "1 V"), ["LC3003"], "a change of scale on an integer");
}

#[test]
fn a_literal_keeps_its_type_where_nothing_is_expected() {
    assert_eq!(judged("s: f32", "x: f32", "x = core.add(s, 0.5)"), ["LC3003"], "two number types meet");
    assert_eq!(judged("s: f32", "x: f64", "x = core.add(s, 0.5)"), NOTHING);
}

#[test]
fn a_named_constant_takes_the_type_of_each_terminal_that_it_reaches() {
    // As a constant on a terminal does, and for each terminal apart.
    assert_eq!(all("", "x: f32, y: f64", "k = 0.5\nx = k\ny = k"), NOTHING, "two outputs of two types");
    assert_eq!(all("", "x: f32", "k = 0.5\nx = takes(k)"), NOTHING, "an input");
    assert_eq!(judged("", "x: f32[V], y: f64[mV]", "k = 240 V\nx = k\ny = k"), NOTHING, "its unit converts");
    assert_eq!(judged("", "x: f32", "a = 0.5\nb = a\nx = b"), NOTHING, "through another name");
    assert_eq!(judged("s: f32", "x: f32", "k = 2\nx = core.div(s, k)"), NOTHING, "an integer beside a wire");
    assert_eq!(judged("s: f32", "x: f32", "k = 0.5\nx = core.add(s, k)"), ["LC3003"], "two number types meet, as for `core.add(s, 0.5)`");
    assert_eq!(judged("c: bool", "x: f32", "k = 0.5\nx = match c {\ntrue => yield k\nfalse => yield 1.5\n}"), NOTHING, "in an arm");
    assert_eq!(judged("", "x: [f32]", "k = 0.5\nx = [k, k]"), NOTHING, "an array of constants");
    assert_eq!(judged("", "x: [f32]", "k = 0.5\nx = [k]"), NOTHING, "an array of one");
    assert_eq!(judged("", "x: [f32], y: [f64]", "k = [0.5, 1.5]\nx = k\ny = k"), NOTHING, "an array constant");
    assert_eq!(judged("", "x: [f32], y: [f64]", "k = [0.5, 1.5]\ny = k\nx = k"), NOTHING, "in any order");
    assert_eq!(judged("", "x: [f32]", "a = 0.5\nb = [a, a]\nx = b"), NOTHING, "an array of named constants");
    assert_eq!(judged("", "x: [[f32]]", "a = [0.5]\nb = [a, a]\nx = b"), NOTHING, "an array of array constants");
    assert_eq!(judged("", "x: [f64; 2]", "k = [[0.5, 1.5], [2.5, 3.5]]\nx = k"), NOTHING, "a literal of rows");
    assert_eq!(judged("x_in: f64", "x: f64", "k = 2\nx = core.pow(x_in, k)"), NOTHING, "the exponent of `core.pow` is a constant");
}

#[test]
fn a_named_constant_is_refused_where_its_literal_is() {
    assert_eq!(judged("", "x: i32", "k = 1.5\nx = k"), ["LC3003"], "a decimal does not become an integer");
    assert_eq!(judged("", "x: f64[A]", "k = 240 V\nx = k"), ["LC3003"], "another dimension");
    assert_eq!(judged("", "x: f64[V]", "k = 240\nx = k"), ["LC3003"], "a number without a unit has no dimension");
    assert_eq!(judged("", "x: f64[mV/V]", "k = 5\nx = k"), ["LC3003"], "and no scale");
    assert_eq!(judged("", "x: f32", "k = 1e39\nx = k"), ["LC3018"], "too large for this terminal");
    assert_eq!(judged("", "x: f32, y: f64", "k = 1e39\nx = k\ny = k"), ["LC3018"], "and only for this one");
    assert_eq!(judged("", "x: string", "k = 0.5\nx = k"), ["LC3003"]);
    assert_eq!(judged("c: bool", "x: f64[mV/V]", "k = 5\nx = match c {\ntrue => yield k\nfalse => yield k\n}"), ["LC3003"], "in an arm too");
}

#[test]
fn an_array_constant_that_does_not_fit_is_refused_on_each_terminal() {
    // One diagnostic for each terminal, on that terminal, whatever the order of the lines.
    assert_eq!(judged("", "x: [i32], y: [i32]", "k = [1.5, 2.5]\nx = k\ny = k"), ["LC3003", "LC3003"]);
    assert_eq!(judged("", "x: [f32], y: [f32]", "k = [\"a\"]\nx = k\ny = k"), ["LC3003", "LC3003"]);
    assert_eq!(judged("", "x: i8, y: i8", "k = [1 V, 1 V]\nx = k\ny = k"), ["LC3003", "LC3003"]);
    assert_eq!(judged("", "x: [f32]", "k = [1e39, 1.5]\nx = k"), ["LC3003"], "too large for this terminal");
    // Its own line says only what is wrong with it, once.
    assert_eq!(judged("", "x: [f64], y: [f64]", "k = [1 V, 1 A]\nx = k\ny = k"), ["LC3001"]);
    assert_eq!(judged("", "x: i8", "k = [[]]\nx = k"), ["LC3019"]);
}

#[test]
fn what_a_constant_says_on_its_own_line_is_said_once() {
    assert_eq!(judged("", "x: i64", "k = 3000000000\nb = k\nc = b\nx = c"), ["LC3018"], "a kept difference from a constant on a terminal (3.4)");
    assert_eq!(judged("", "x: [f32]", "k = []\nx = k"), ["LC3019"], "another one");
    assert_eq!(judged("", "x: fn(f64) -> (f64)", "k = core.abs\nx = k"), ["LC3019"], "and the last");
}

#[test]
fn a_named_constant_in_a_bracket_is_a_constant() {
    let brackets = |body: &str, wanted: &str| {
        let text = format!("use array\nuse core\n\nfn f() -> (x: {wanted}) {{\n{body}\n}}");
        let helper = "use array\n\nfn takes(a: [f64[mV/V]]) -> (x: [f64[mV/V]]) {\n    x = a\n}";
        codes(&[("f.lcf", &text), ("takes.lcf", helper)])
    };
    assert_eq!(brackets("k = 2\nx = array.push([k], 1.5)", "[f64]"), NOTHING, "it decides last, as `[2]` does");
    assert_eq!(brackets("k = 0.5\nx = array.push([k], 1.5)", "[f64]"), NOTHING);
    assert_eq!(brackets("k = 5\nx = takes([k])", "[f64[mV/V]]"), ["LC3001"], "no scale, as for `[5]`");
    assert_eq!(brackets("k = 5\nx = takes([k, k])", "[f64[mV/V]]"), brackets("x = takes([5, 5])", "[f64[mV/V]]"));
    assert_eq!(brackets("k = 5 mV/V\nx = takes([k])", "[f64[mV/V]]"), NOTHING);
}

#[test]
fn a_named_constant_in_a_bracket_takes_the_type_expected() {
    let helper = "fn takes(a: [f32]) -> (x: [f32]) {\n    x = a\n}";
    let text = "fn f() -> (x: [f32]) {\n    k = 0.5\n    x = takes([k])\n}";
    assert_eq!(codes(&[("f.lcf", text), ("takes.lcf", helper)]), NOTHING);
}

#[test]
fn a_named_array_of_rows_decides_a_primitive_as_a_literal_does() {
    let inputs = "c: bool, m: [f64; 2]";
    // The wire decides first, and the literal of rows takes what it decided.
    assert_eq!(judged(inputs, "x: [f64; 2]", "x = core.select(c, m, [[0.5, 1.5], [2.5, 3.5]])"), NOTHING);
    assert_eq!(judged(inputs, "x: [f64; 2]", "k = [[0.5, 1.5], [2.5, 3.5]]\nx = core.select(c, m, k)"), NOTHING);
    // Written first, it decides, under its name or not.
    assert_eq!(judged(inputs, "x: [f64; 2]", "k = [[0.5, 1.5], [2.5, 3.5]]\nx = core.select(c, k, m)"), ["LC3001"]);
    assert_eq!(judged(inputs, "x: [f64; 2]", "x = core.select(c, [[0.5, 1.5], [2.5, 3.5]], m)"), ["LC3001"]);
}

#[test]
fn a_try_that_fails_inside_a_try_fails_it() {
    assert_eq!(judged("", "x: [[f32]]", "a = [\"s\"]\nb = [a, a]\nx = b"), ["LC3003"]);
}

#[test]
fn chains_of_array_constants_cost_what_they_hold() {
    // The work is one try for each array and each type, and not one for each path through the names.
    let mut body = String::from("a0 = [0.5, 1.5]\n");
    for level in 1..40 {
        body.push_str(&format!("a{level} = [a{0}, a{0}]\n", level - 1));
    }
    assert_eq!(judged("", &format!("x: {}f32{}", "[".repeat(25), "]".repeat(25)), &format!("{body}x = a24")), NOTHING);
}

/// `count` outputs that all read `k`, each of a type given by its position.
fn outputs_of_many_types(count: usize) -> (String, String) {
    let mut outputs = Vec::new();
    let mut lines = Vec::new();
    for kind in ["f32", "f64", "c64", "c128"] {
        for prefix in ["p", "n", "µ", "m", "", "k", "M", "G"] {
            if outputs.len() < count {
                let name = format!("x{}", outputs.len());
                outputs.push(format!("{name}: {kind}[{prefix}V]"));
                lines.push(format!("{name} = k"));
            }
        }
    }
    (outputs.join(", "), lines.join("\n"))
}

#[test]
fn the_work_on_a_number_counts_its_digits_and_its_unit() {
    let (outputs, lines) = outputs_of_many_types(32);
    assert_eq!(judged("", &outputs, &format!("k = 1 V\n{lines}")), NOTHING, "a short one goes everywhere");
    let digits = format!("{}1", "0".repeat(100_000));
    let found = judged("", &outputs, &format!("k = {digits} V\n{lines}"));
    assert!(!found.is_empty() && found.iter().all(|code| *code == "LC3003"), "{found:?}");
    let terms = "*V/V".repeat(50_000);
    let found = judged("", &outputs, &format!("k = 1 V{terms}\n{lines}"));
    assert!(!found.is_empty() && found.iter().all(|code| *code == "LC3003"), "{found:?}");
    let found = judged("", &outputs, &format!("k = 1 (V{terms})\n{lines}"));
    assert!(!found.is_empty() && found.iter().all(|code| *code == "LC3003"), "{found:?}");
}

#[test]
fn a_number_is_not_judged_again_for_the_same_type() {
    let digits = format!("{}1", "0".repeat(100_000));
    let outputs: Vec<String> = (0..50).map(|index| format!("x{index}: f32")).collect();
    let lines: Vec<String> = (0..50).map(|index| format!("x{index} = k")).collect();
    assert_eq!(judged("", &outputs.join(", "), &format!("k = {digits}\n{}", lines.join("\n"))), NOTHING);
}

#[test]
fn a_very_long_constant_can_still_go_to_another_type() {
    let elements = vec!["1 V"; 500_000].join(", ");
    assert_eq!(judged("", "x: [f32[V]]", &format!("k = [{elements}]\nx = k")), NOTHING);
}

#[test]
fn the_exponent_of_a_named_constant_is_read() {
    let digits = format!("{}2", "0".repeat(100));
    assert_eq!(judged("s: f64", "x: f64", &format!("k = {digits}\nx = core.pow(s, k)")), NOTHING);
    assert_eq!(judged("s: f64", "x: f64", "k = 2.5\nx = core.pow(s, k)"), judged("s: f64", "x: f64", "x = core.pow(s, 2.5)"));
    assert_eq!(judged("s: f64, e: i32", "x: f64", "x = core.pow(s, e)"), ["LC3016"], "a wire is not a constant");
}

#[test]
fn trying_an_array_constant_changes_nothing_that_a_pass_reads() {
    use labcraft_graph::{Decl, Operand, Rhs, StatementKind};
    for lines in ["    x = k\n    y = k\n", "    y = k\n    x = k\n"] {
        let text = format!("fn f() -> (x: [f32[mV]], y: [f64[kV]]) {{\n    k = [1 V, 200 mV]\n{lines}}}");
        let checked = run(&[("f.lcf", &text)]);
        assert!(checked.report.diagnostics().is_empty(), "{:?}", checked.report.diagnostics());
        let file = checked.files.first().unwrap();
        assert_eq!(file.resolved.coercions().len(), 1, "only `200 mV` to the `V` of its own line");
        let Decl::Function(function) = &file.file.decl else { panic!() };
        let values = function.body.statements.iter().find_map(|statement| match &statement.kind {
            StatementKind::Binding { value: Rhs::Operand(Operand::Array(_, values)), .. } => Some(values),
            _ => None,
        });
        let first = values.and_then(|values| values.first()).unwrap();
        let volts = Unit::from_name("V").unwrap();
        let own = checked.types.get(file.resolved.ty(first.id()));
        assert!(matches!(own, Ty::Number { kind: NumberKind::F64, unit } if *unit == volts), "{own:?}");
    }
}

#[test]
fn the_fix_of_a_named_integer_writes_its_unit_for_an_integer() {
    let found = only("", "x: i32[V]", "k = 240\nx = k");
    let edit = found.fixes.first().and_then(|fix| fix.edits.first()).unwrap();
    assert_eq!(edit.text, "240 V");
}

#[test]
fn a_field_of_a_named_constant_is_not_that_constant() {
    assert_eq!(judged("", "x: f64", "k = 0.5\nx = k.foo"), ["LC2006"]);
}

#[test]
fn a_wire_from_a_named_constant_starts_at_its_node() {
    let text = program("", "x: string", "    k = 0.5\n    b = k\n    x = b");
    let found = diagnostics(&[("f.lcf", &text)]);
    let [diagnostic] = found.as_slice() else { panic!("{found:?}") };
    let wire = diagnostic.wires.first().unwrap();
    assert_eq!((wire.value.as_deref(), wire.from.as_deref(), wire.to.as_deref()), (Some("b"), Some("k.value"), Some("fn.x")));
}

#[test]
fn a_number_too_large_is_said_at_the_terminal_that_it_does_not_fit() {
    let text = program("", "x: f32, y: f64, z: f32", "    k = 1e39\n    x = k\n    y = k\n    z = k");
    let found = diagnostics(&[("f.lcf", &text)]);
    let lines: Vec<u32> = found.iter().map(|diagnostic| diagnostic.span.map_or(0, |span| span.start.line)).collect();
    assert_eq!(found.iter().map(|diagnostic| diagnostic.code().as_str()).collect::<Vec<_>>(), ["LC3018", "LC3018"]);
    assert_eq!(lines, [11, 13], "`k` is on line 10, and `y` can hold it");
}

#[test]
fn the_fix_of_a_named_number_goes_on_its_line() {
    let text = program("", "x: f64[V]", "    k = 240\n    x = k");
    let found = diagnostics(&[("f.lcf", &text)]);
    let [diagnostic] = found.as_slice() else { panic!("{found:?}") };
    let edit = diagnostic.fixes.first().and_then(|fix| fix.edits.first()).unwrap();
    assert_eq!(edit.text, "240 V");
    assert_eq!(edit.span.start.line, 10, "where the number is written");
    // One to convert a wire keeps its name, on a line of its own.
    let text = program("", "x: i32", "    k = 1.5\n    x = k");
    let found = diagnostics(&[("f.lcf", &text)]);
    let [diagnostic] = found.as_slice() else { panic!("{found:?}") };
    let fix = diagnostic.fixes.first().unwrap();
    assert!(fix.edits.iter().any(|edit| edit.text.contains("core.round_to_i32(k)")), "{fix:?}");
}

#[test]
fn a_chain_of_array_constants_never_goes_deep() {
    // Each new name would go down the whole chain for what is unknown around it.
    let mut body = String::from("a0 = [0.5, 1.5]\n");
    for level in 1..100_000 {
        body.push_str(&format!("a{level} = [a{}, 1.5]\n", level - 1));
    }
    body.push_str("b = [a99999, a99999]\nx = 1");
    assert!(!judged("", "x: i32", &body).is_empty(), "an array and a number disagree, and the stack of a test is enough");
}

#[test]
fn the_work_on_constants_has_a_limit() {
    // Beyond it, a constant keeps the type of its own line: what it goes to says so, and nothing slows down.
    let elements = vec!["1 V"; 20_000].join(", ");
    let mut outputs = Vec::new();
    let mut lines = Vec::new();
    for kind in ["f32", "f64", "c64", "c128"] {
        for prefix in ["p", "n", "µ", "m", "", "k", "M", "G"] {
            let name = format!("x{}", outputs.len());
            outputs.push(format!("{name}: [{kind}[{prefix}V]]"));
            lines.push(format!("{name} = k"));
        }
    }
    let body = format!("k = [{elements}]\n{}", lines.join("\n"));
    let found = judged("", &outputs.join(", "), &body);
    assert!(!found.is_empty() && found.len() < outputs.len() && found.iter().all(|code| *code == "LC3003"), "{found:?}");
}

#[test]
fn a_new_name_for_a_wire_stays_a_wire() {
    assert_eq!(judged("s: f64", "x: f32", "k = s\nx = k"), ["LC3003"], "a wire keeps its type");
    assert_eq!(judged("s: f64", "x: [f32]", "k = [s, s]\nx = k"), ["LC3003"], "an array that holds one");
    assert_eq!(judged("s: f64", "x: [f32]", "k = [s]\nx = k"), ["LC3003"], "an array of one");
    assert_eq!(judged("", "x: f64, y: f32", "x = 0.5\nk = x\ny = k"), ["LC3003"], "an output is a terminal, not a named constant");
}

#[test]
fn array_constants_built_on_each_other_are_not_worked_out_again() {
    // Without the memory of what each one makes, each line would double the work.
    let mut body = String::from("a0 = [0.5, 1.5]\n");
    for level in 1..40 {
        body.push_str(&format!("a{level} = [a{}, a{}]\n", level - 1, level - 1));
    }
    body.push_str("x = 1");
    assert_eq!(judged("", "x: i32", &body), NOTHING);
}

#[test]
fn a_literal_takes_what_every_place_expects() {
    assert_eq!(all("", "x: f32", "x = takes(0.5)"), NOTHING, "an input");
    assert_eq!(all("", "x: f32", "x = loop carry a: f32 = 0.5 {\nnext a = 1.5\nuntil true\n}"), NOTHING, "a carry, and `next`");
    assert_eq!(all("a: [f32]", "x: [f32]", "x = array.push(a, 0.5)"), NOTHING, "what an array fixes");
    assert_eq!(all("s: f32", "x: f32", "x = core.add<f32>(s, 0.5)"), NOTHING, "what a type argument fixes");
    assert_eq!(all("s: f32", "x: [f32]", "y = [s, 0.5]\nx = y"), NOTHING, "the element after one that decides");
    assert_eq!(all("s: f32", "x: [f32]", "y = [0.5, s]\nx = y"), ["LC3003"], "the first element decides");
    assert_eq!(all("", "x: [f32]", "x = [0.5, 1.5]"), NOTHING);
    assert_eq!(all("", "x: [f32]", "x = []"), NOTHING);
}

#[test]
fn an_array_that_holds_a_wire_has_the_type_of_its_wires() {
    let inputs = "d: f64, v: f64[V], mv: f64[mV], n: i32, s: f32, row: [f64], m: [f64; 2], b: bool";
    assert_eq!(judged(inputs, "x: [f32]", "x = [d, d]"), ["LC3003"], "one diagnostic, on the wire to the output");
    assert_eq!(judged(inputs, "x: [f64[mV]]", "x = [v, v]"), ["LC3003"]);
    assert_eq!(judged(inputs, "x: [f64]", "x = [n]"), ["LC3003"], "an array of one wire");
    assert_eq!(judged(inputs, "x: [f64]", "x = [n, 1]"), ["LC3003"], "a constant beside a wire does not change that");
    assert_eq!(judged(inputs, "x: [[f64]]", "x = [[n, 1]]"), ["LC3003"], "at any depth");
    assert_eq!(judged(inputs, "x: [f64[V]]", "x = [v, mv]"), NOTHING, "the elements still go by the first that decides");
    assert_eq!(judged(inputs, "x: [f32]", "x = [0.5, 1.5]"), NOTHING, "only constants");
    assert_eq!(judged(inputs, "x: [f32]", "x = [s, 0.5]"), NOTHING, "the constant takes the type of the wire");
    assert_eq!(judged(inputs, "x: [f32]", "x = [0.5, s]"), ["LC3003"], "the first element decides");
    assert_eq!(judged(inputs, "x: [f64; 2]", "x = [row, row]"), ["LC3003"], "rows");
    assert_eq!(judged(inputs, "x: [f64; 2]", "x = [row]"), ["LC3003"], "a row");
    assert_eq!(judged(inputs, "x: bool", "x = core.eq(m, [row])"), ["LC3001"]);
    assert_eq!(all(inputs, "l: List", "l = List { items: [d, d] }"), ["LC3015"], "a field");
    assert_eq!(all(inputs, "x: [f64]", "y = [n]\nx = y"), ["LC3003"], "a name changes nothing");
    assert_eq!(judged_with(&[("h.lcf", "fn h(a: [f64]) -> () {}")], "n: i32", "", "h([n])"), ["LC3001"], "an input");
    assert_eq!(judged_with(&[("h.lcf", "fn h(a: [f64]) -> () {}")], "n: i32", "", "h([n, 1])"), ["LC3001"], "an input");
}

#[test]
fn a_constant_in_a_collect_takes_the_type_of_the_target_of_its_loop() {
    let looped = |value: &str| format!("x = for i in 0..3 {{\ncollect {value}\n}}");
    let inputs = "d: f64, s: f32";
    assert_eq!(judged(inputs, "x: [f32]", &looped("0.5")), NOTHING);
    assert_eq!(judged(inputs, "x: [f32[V]]", &looped("1.5 V")), NOTHING);
    assert_eq!(judged(inputs, "x: [f32]", &looped("s")), NOTHING);
    assert_eq!(judged(inputs, "x: [f32]", &looped("d")), ["LC3003"], "a wire keeps its type");
    assert_eq!(judged(inputs, "x: [i32]", &looped("1.5")), ["LC3003"], "on the wire to the target, as for an arm");
    assert_eq!(judged(inputs, "x: [f32]", &looped("1e39")), ["LC3018"]);
    assert_eq!(judged(inputs, "x: [[f32]]", &looped("[0.5]")), NOTHING);
    assert_eq!(judged(inputs, "x: [[f32]]", &looped("[]")), NOTHING, "`[]` knows its type");
    assert_eq!(judged(inputs, "x: [f32]", "y = for i in 0..3 {\ncollect 0.5\n}\nx = y"), ["LC3003"], "a name has no type written");
    assert_eq!(judged(inputs, "", &looped("[]")), ["LC3019"]);
    let two = "a, x = for i in 0..3 {\ncollect 1.5\ncollect 0.5\n}";
    assert_eq!(judged(inputs, "x: [f32]", two), NOTHING, "by position");
    let carry = "c, x = for i in 0..3, carry n = 1 {\nnext n = n\ncollect 0.5\n}";
    assert_eq!(judged(inputs, "c: i32, x: [f32]", carry), NOTHING, "after the carries");
    let bad_targets = "a, b, x = for i in 0..3 {\ncollect 0.5\n}";
    assert_eq!(judged(inputs, "x: [f32]", bad_targets), ["LC2103"]);
    let nested = "x = for i in 0..3 {\ncollect 0.5\ny = for j in 0..3 {\ncollect 1.5\n}\n_ = y\n}";
    assert_eq!(judged(inputs, "x: [f32]", nested), NOTHING);
    let unknown = "a = loop carry w = nowhere {\nnext w = for i in 0..3 {\ncollect []\n}\nuntil true\n}";
    assert_eq!(judged(inputs, "", unknown), ["LC2001"], "what is unknown says nothing more");
}

#[test]
fn a_decimal_too_large_for_its_type_is_refused() {
    for (ty, literal) in [("f32", "1e39"), ("f32", "-1e39"), ("c64", "1e39"), ("f32[V]", "1e39 V")] {
        assert_eq!(gives(ty, literal), ["LC3018"], "{literal} for {ty}");
    }
    assert_eq!(gives("i8[mV]", "300 mV"), ["LC3018"]);
    assert_eq!(gives("f32[V]", "16777217 V"), ["LC3018"]);
    assert_eq!(gives("f64", "1e39"), NOTHING);
    assert_eq!(gives("c128", "1e39"), NOTHING);
    assert_eq!(gives("f64", "1e999"), ["LC1006"], "beyond every type");
    assert_eq!(gives("f32[V]", "1e999 V"), ["LC1006"], "one diagnostic");
    assert_eq!(gives("f32", "1_0e38"), ["LC3018"], "its digits, without their separators");
    assert_eq!(gives("f32", "3_000.5"), NOTHING);
}

#[test]
fn a_literal_is_judged_by_its_value_in_the_unit_that_it_goes_to() {
    assert_eq!(gives("f32[mV]", "1e36 V"), ["LC3018"], "1e39 mV");
    assert_eq!(gives("f32[V]", "1e39 mV"), NOTHING, "1e36 V");
    assert_eq!(gives("f64[mV]", "1e308 V"), ["LC3018"], "1e311 mV");
    assert_eq!(gives("duration", "1e308 ks"), ["LC3018"], "1e311 s");
    assert_eq!(gives("duration", "5 ms"), NOTHING);
    assert_eq!(gives("f32[V]", "0 kV"), NOTHING);
}

#[test]
fn a_literal_of_another_dimension_keeps_its_type() {
    let diagnostic = only("", "x: i32", "    x = 1 V");
    assert_eq!(diagnostic.found.as_deref(), Some("f64[V]"));
}

#[test]
fn a_literal_in_an_arm_takes_the_type_of_the_target() {
    let arms = |a: &str, b: &str| format!("x = match c {{\ntrue => yield {a}\nfalse => yield {b}\n}}");
    let inputs = "c: bool, s: f32, d: f64";
    assert_eq!(judged(inputs, "x: f32", &arms("0.5", "1.5")), NOTHING);
    assert_eq!(judged(inputs, "x: f32[V]", &arms("0.5 V", "200 mV")), NOTHING);
    assert_eq!(judged(inputs, "x: f32", &arms("s", "0.5")), NOTHING);
    assert_eq!(judged(inputs, "x: f32", &arms("0.5", "s")), NOTHING);
    assert_eq!(judged(inputs, "x: f32", &arms("0.5", "d")), ["LC3006"], "a wire keeps its type");
    assert_eq!(judged(inputs, "x: f32", &arms("d", "d")), ["LC3003"]);
    assert_eq!(judged(inputs, "x: i32", &arms("1.5", "2.5")), ["LC3003"], "a decimal does not become an integer");
    assert_eq!(judged(inputs, "x: i32[mV]", &arms("1 V", "2 V")), ["LC3003"]);
    assert_eq!(judged(inputs, "x: f32", &format!("{}\n", arms("1e39", "0.5"))), ["LC3018"]);
}

#[test]
fn without_a_target_the_first_arm_that_decides_gives_the_type() {
    let arms = |a: &str, b: &str| format!("y = match c {{\ntrue => yield {a}\nfalse => yield {b}\n}}\nx = y");
    let inputs = "c: bool, s: f32";
    assert_eq!(judged(inputs, "x: f32", &arms("s", "0.5")), NOTHING, "the literal takes the type of the arm that decides");
    assert_eq!(judged(inputs, "x: f32", &arms("0.5", "s")), ["LC3003"], "the order of arms counts (R-7)");
    assert_eq!(judged(inputs, "x: f64", &arms("0.5", "s")), NOTHING);
}

#[test]
fn a_struct_is_built_with_every_field_once() {
    let built = |fields: &str| judged("", "p: Point", &format!("p = Point {{ {fields} }}"));
    assert_eq!(built("x: 1 m, label: \"a\""), NOTHING);
    assert_eq!(built("label: \"a\", x: 1 m"), NOTHING, "in any order");
    assert_eq!(built("x: 1000 mm, label: \"a\""), NOTHING, "a wire converts");
    assert_eq!(built("x: 1 m"), ["LC3015"]);
    assert_eq!(built("x: 1 V, label: \"a\""), ["LC3015"]);
    assert_eq!(built("x: 1 m, label: 1"), ["LC3015"]);
    assert_eq!(built("x: 1 m, z: 1"), ["LC2006"], "what seems missing may be the cause");
    assert_eq!(built("x: 1 m, x: 2 m, label: \"a\""), ["LC3015"]);
    assert_eq!(built("x: 1 m, x: 2 m"), ["LC3015"], "what seems missing may be the cause");
    assert_eq!(built("x: 1 m, label: []"), ["LC3015"]);
    assert_eq!(built("x: 1 m, z: [[]]"), ["LC2006"], "what is unknown says nothing more");
    assert_eq!(built("x: 1 m, z: [core.abs]"), ["LC2006"]);
    assert_eq!(built("x: 1 m, x: [core.abs], label: \"a\""), ["LC3015"]);
}

#[test]
fn an_element_in_a_field_has_the_code_of_a_construction() {
    assert_eq!(all("t: string", "l: List", "l = List { items: [0.5, t] }"), ["LC3015"]);
}

#[test]
fn what_holds_a_value_ends_with_it() {
    let lines = "p = Point { x: 1 m, label: \"a\" }\nx = core.add";
    assert_eq!(judged("", "x: fn(f64) -> (f64)", lines), ["LC3008"], "after a struct");
    let lines = "r = Reading.Value(1 V)\nx = core.add";
    assert_eq!(judged("", "x: fn(f64) -> (f64)", lines), ["LC3008"], "after a variant");
    assert_eq!(judged("a: f32 = 0.5", "x: Reading", "x = Reading.Value"), ["LC3015"], "after a default");
    assert_eq!(judged("a: f32 = 0.5", "x: [string]", "x = [1, \"a\"]"), ["LC3001"], "after a default");
}

#[test]
fn a_field_given_twice_points_at_the_first() {
    let diagnostic = only("", "p: Point", "    p = Point { x: 1 m, x: 2 m, label: \"a\" }");
    assert_eq!(diagnostic.code().as_str(), "LC3015");
    assert_eq!(diagnostic.related.len(), 1);
    assert!(diagnostic.span.unwrap().start.column > diagnostic.related.first().unwrap().span.unwrap().start.column);
}

#[test]
fn a_wrong_wire_on_a_field_has_the_code_of_a_construction() {
    let built = |fields: &str| all("d: f64, y: i64", "g: Gain", &format!("g = Gain {{ {fields} }}"));
    let good = "a: 0.5, n: 1, compare: core.lt";
    assert_eq!(built(good), NOTHING);
    assert_eq!(built("a: d, n: 1, compare: core.lt"), ["LC3015"], "a narrowing");
    assert_eq!(built("a: 0.5, n: y, compare: core.lt"), ["LC3015"], "a narrowing");
    assert_eq!(built("a: 0.5, n: 1, compare: core.add"), ["LC3015"], "a function of the wrong signature");
    assert_eq!(built("a: 0.5, n: 1, compare: takes"), ["LC3015"]);
    let diagnostic = only("d: f64", "p: Point", "    p = Point { x: d, label: \"a\" }");
    let wire = diagnostic.wires.first().unwrap();
    assert_eq!(wire.to.as_deref(), Some("p.x"));
    assert_eq!(wire.from.as_deref(), Some("fn.d"));
}

#[test]
fn a_construction_names_at_most_five_missing_fields() {
    let diagnostic = only("", "s: Seven", "    s = Seven { a: 1 }");
    for shown in ["`b`", "`c`", "`d`", "`e`", "`f`"] {
        assert!(diagnostic.message.contains(shown), "{}", diagnostic.message);
    }
    assert!(!diagnostic.message.contains("`g`"), "{}", diagnostic.message);
    assert!(diagnostic.message.contains("1 more"), "{}", diagnostic.message);
}

#[test]
fn only_a_struct_or_a_waveform_is_built_with_braces() {
    assert_eq!(all("", "x: Verdict", "x = Verdict { a: 1 }"), ["LC3015"]);
    assert_eq!(all("", "x: f64", "x = f64 { a: 1 }"), ["LC3015"]);
    assert_eq!(all("", "x: f64", "x = f64 { a: [[]] }"), ["LC3015"], "what is unknown says nothing more");
    assert_eq!(all("", "x: f64[V]", "x = Volts { a: 1 }"), ["LC3015"], "through an alias");
    assert_eq!(all("", "", "x = queue { a: nowhere }"), ["LC3015", "LC2001"], "the mistake inside is another one");
}

#[test]
fn a_variant_has_its_values() {
    let gives_reading = |line: &str| judged("t: string", "x: Reading", &format!("x = {line}"));
    assert_eq!(gives_reading("Reading.Overload"), NOTHING);
    assert_eq!(gives_reading("Reading.Overload()"), NOTHING);
    assert_eq!(gives_reading("Reading.Value(1 V)"), NOTHING);
    assert_eq!(gives_reading("Reading.Value(200 mV)"), NOTHING, "a wire converts");
    assert_eq!(gives_reading("Reading.Pair(1, t)"), NOTHING);
    assert_eq!(gives_reading("Reading.Value"), ["LC3015"], "no default value, ever");
    assert_eq!(gives_reading("Reading.Value()"), ["LC3015"]);
    assert_eq!(gives_reading("Reading.Pair(1)"), ["LC3015"]);
    assert_eq!(gives_reading("Reading.Pair(1, t, [])"), ["LC3015"], "too many, and one diagnostic");
    assert_eq!(gives_reading("Reading.Overload(1)"), ["LC3015"]);
    assert_eq!(gives_reading("Reading.Overload([[]])"), ["LC3015"], "what is in excess says nothing more");
    assert_eq!(gives_reading("Reading.Value(t)"), ["LC3015"]);
    assert_eq!(gives_reading("Reading.Value(1)"), ["LC3015"], "a number without a unit");
    assert_eq!(judged("c: bool", "x: [Reading]", "x = [Reading.Value]"), ["LC3015"]);
    assert_eq!(judged("c: bool", "x: Reading", "x = core.select(c, Reading.Value, Reading.Overload)"), ["LC3015"]);
    assert_eq!(judged("", "x: Reading", "x = Reading.Value(value: 1 V)"), ["LC2009"]);
}

#[test]
fn a_waveform_is_built_like_a_struct() {
    let inputs = "t: timestamp, s: [f64[V]], mv: [f64[mV]], d: duration, x: f64, u: string, xs: [string], m: [f64; 2], rows: [[f64]], ds: [duration]";
    let built = |output: &str, fields: &str| judged(inputs, &format!("w: {output}"), &format!("w = waveform {{ {fields} }}"));
    let v = "waveform<f64[V]>";
    assert_eq!(built(v, "t0: t, dt: 1 ms, samples: s"), NOTHING);
    assert_eq!(built(v, "t0: t, dt: d, samples: s"), NOTHING);
    assert_eq!(built(v, "samples: s, t0: t, dt: 1 ms"), NOTHING, "in any order");
    assert_eq!(built(v, "t0: t, dt: 1 ms"), ["LC3015"]);
    assert_eq!(built(v, "t0: t, dt: 1 ms, samples: s, samples: s"), ["LC3015"]);
    assert_eq!(built(v, "t0: 1, dt: 1 ms, samples: s"), ["LC3015"]);
    assert_eq!(built(v, "t0: t, dt: 5, samples: s"), ["LC3015"]);
    assert_eq!(built(v, "t0: t, dt: 1 V, samples: s"), ["LC3015"]);
    assert_eq!(built(v, "t0: t, dt: 1 ms, samples: x"), ["LC3015"], "not an array");
    assert_eq!(built(v, "t0: t, dt: 1 ms, samples: s, z: 1"), ["LC2006"]);
    assert_eq!(built(v, "t0: t, z: 1"), ["LC2006"], "what seems missing may be the cause");
    assert_eq!(built(v, "t0: t, dt: 1 ms, samples: mv"), ["LC3003"], "no conversion between two waveforms");
    assert_eq!(built("waveform<f64[mV]>", "t0: t, dt: 1 ms, samples: s"), ["LC3003"]);
    assert_eq!(built("waveform<f64[mV]>", "t0: t, dt: 1 ms, samples: [1 V]"), NOTHING, "a literal takes the type expected");
    assert_eq!(built("waveform<f32>", "t0: t, dt: 1 ms, samples: [0.5]"), NOTHING);
    assert_eq!(built("waveform<f64>", "t0: t, dt: 1 ms, samples: []"), NOTHING, "`T` of the output");
    for samples in ["xs", "rows", "m", "ds"] {
        assert_eq!(built("f64", &format!("t0: t, dt: 1 ms, samples: {samples}")), ["LC3019"], "{samples}: `T` is a number");
    }
    assert_eq!(judged(inputs, "", "w = waveform { t0: t, dt: 1 ms, samples: [] }"), ["LC3019"], "nothing says `T`");
    assert_eq!(judged(inputs, "w: f64", "w = waveform { t0: t, dt: 1 ms, samples: s }"), ["LC3003"]);
    assert_eq!(judged("t: timestamp, u: [Nowhere]", "", "w = waveform { t0: t, dt: 1 ms, samples: u }"), ["LC2003"], "one mistake");
    let unknown = "a = loop carry w = nowhere {\nnext w = waveform { t0: t, dt: 1 ms, samples: [] }\nuntil true\n}";
    assert_eq!(judged("t: timestamp", "", unknown), ["LC2001"], "what it goes to is unknown, and has its diagnostic");
}

#[test]
fn a_waveform_holds_numbers() {
    assert_eq!(judged("a: waveform<f64[V]>", "", ""), NOTHING);
    assert_eq!(all("a: waveform<Volts>", "", ""), NOTHING, "an alias of a number");
    for ty in ["string", "duration", "Point", "[f64]", "Verdict", "waveform<f64>", "fn() -> ()"] {
        assert_eq!(judged(&format!("a: waveform<{ty}>"), "", ""), ["LC3019"], "{ty}");
    }
    assert_eq!(judged("a: waveform<Nowhere>", "", ""), ["LC2003"], "one mistake");
}

#[test]
fn a_waveform_has_three_fields_to_read() {
    let inputs = "w: waveform<f64[V]>";
    assert_eq!(judged(inputs, "x: timestamp, y: duration, z: [f64[V]]", "x = w.t0\ny = w.dt\nz = w.samples"), NOTHING);
    assert_eq!(judged(inputs, "x: f64", "x = w.nope"), ["LC2006"]);
    assert_eq!(judged(inputs, "x: [f64[mV]]", "x = w.samples"), ["LC3003"], "nothing converts inside an array");
    assert_eq!(all("w: Trace", "z: [f64[V]]", "z = w.samples"), NOTHING, "through an alias");
}

#[test]
fn an_alias_of_a_waveform_knows_its_element() {
    let inputs = "t: timestamp, s: [f64[V]], mv: [f64[mV]], ss: [string]";
    let built = |fields: &str| all(inputs, "w: Trace", &format!("w = Trace {{ {fields} }}"));
    assert_eq!(built("t0: t, dt: 1 ms, samples: s"), NOTHING);
    assert_eq!(built("t0: t, dt: 1 ms, samples: []"), NOTHING);
    assert_eq!(built("t0: t, dt: 1 ms, samples: [1 V, 200 mV]"), NOTHING);
    assert_eq!(built("t0: t, dt: 1 ms, samples: mv"), ["LC3015"], "no conversion in an array");
    assert_eq!(built("t0: t, dt: 1 ms, samples: ss"), ["LC3015"]);
    assert_eq!(built("t0: t, dt: 1 ms, samples: 1"), ["LC3015"]);
    assert_eq!(built("dt: 1 ms, samples: s"), ["LC3015"]);
    assert_eq!(built("t0: t, dt: 1 ms, samples: s, z: 1"), ["LC2006"], "as for a `waveform` written out");
    assert_eq!(all(inputs, "", "w = Trace { t0: t, dt: 1 ms, samples: [] }"), NOTHING, "the alias says `T`");
}

#[test]
fn a_primitive_takes_its_variables_from_the_function_type() {
    let references = [
        ("fn(f64[V]) -> (f64[V])", "core.abs"),
        ("fn(f64, f64) -> (bool)", "core.lt"),
        ("fn(string, string) -> (bool)", "core.eq"),
        ("fn(f64[V], f64[A]) -> (f64[W])", "core.mul"),
        ("fn(i32, i32) -> (i32)", "core.mul"),
        ("fn(i32, f64) -> (f64)", "core.mul"),
        ("fn(f64[V], f64[A]) -> (f64[Ω])", "core.div"),
        ("fn(i32, i32) -> (f64)", "core.div"),
        ("fn(f64[V^2]) -> (f64[V])", "core.sqrt"),
        ("fn(i32) -> (f64)", "core.sqrt"),
        ("fn(f64, i32) -> (f64)", "core.pow"),
        ("fn(f64[°C], f64[°C]) -> (f64[K])", "core.sub"),
        ("fn(i32[°C], i32[°C]) -> (f64[K])", "core.sub"),
        ("fn(duration, duration) -> (bool)", "core.lt"),
        ("fn(duration, duration) -> (f64[s])", "core.add"),
        ("fn([duration]) -> (f64[s])", "array.sum"),
        ("fn(f64[V], f64[V]) -> (f64[V])", "core.sub"),
        ("fn(bool, f64, f64) -> (f64)", "core.select"),
        ("fn([f64]) -> (i32)", "array.len"),
        ("fn([f64], i32, error) -> (f64, error)", "array.get"),
        ("fn([f64], f64) -> ([f64])", "array.push"),
        ("fn([f64[V]]) -> (f64[V])", "array.sum"),
    ];
    for (ty, primitive) in references {
        assert_eq!(judged("", &format!("x: {ty}"), &format!("x = {primitive}")), NOTHING, "{primitive} as {ty}");
    }
    let refused = [
        ("fn(i32, f64) -> (f64)", "core.add"),
        ("fn(f64, i32) -> (f64)", "core.add"),
        ("fn(i32[°C], i32[°C]) -> (i32[K])", "core.sub"),
        ("fn(f64) -> (f64[V])", "core.abs"),
        ("fn(f64, f64) -> (f64)", "core.abs"),
        ("fn(f64[V], f64[A]) -> (f64[V])", "core.mul"),
        ("fn(i64, f64) -> (f64)", "core.mul"),
        ("fn(f64[°C], f64[°C]) -> (f64[°C])", "core.sub"),
        ("fn(f64[V], i32) -> (f64[V])", "core.pow"),
        ("fn(c128, c128) -> (bool)", "core.lt"),
        ("fn(string) -> (string)", "core.abs"),
        ("fn(f64[V]) -> (f64[V])", "core.sqrt"),
        ("fn(f64, f64, f64) -> (f64)", "core.select"),
        ("fn(duration) -> (duration)", "core.abs"),
        ("fn([f64], i32) -> (f64, error)", "array.get"),
    ];
    for (ty, primitive) in refused {
        assert_eq!(judged("", &format!("x: {ty}"), &format!("x = {primitive}")), ["LC3008"], "{primitive} as {ty}");
    }
}

#[test]
fn a_generic_primitive_needs_a_function_type() {
    assert_eq!(judged("", "", "x = core.abs"), ["LC3019"]);
    assert_eq!(judged("", "r: f64", "r = core.call(core.abs, 1.5)"), ["LC3019"]);
    assert_eq!(gives("f64", "core.abs"), ["LC3008"], "what it goes to is no function type");
    assert_eq!(judged("", "", "x = core.select(true, core.abs, core.neg)"), ["LC3019"], "one diagnostic");
    assert_eq!(judged("", "", "x = [core.abs, core.neg]"), ["LC3019"], "one diagnostic");
    assert_eq!(all("", "x: Compare", "x = core.lt"), NOTHING, "through an alias");
    assert_eq!(all("", "x: [Compare]", "x = [core.lt, core.gt]"), NOTHING, "an array");
    assert_eq!(all("", "x: f32", "x = takes(0.5, core.gt)"), NOTHING, "an input");
    assert_eq!(all("c: bool", "x: Compare", "x = match c {\ntrue => yield core.lt\nfalse => yield core.gt\n}"), NOTHING, "an arm");
    assert_eq!(all("c: bool, f: Compare", "x: Compare", "x = core.select(c, f, core.lt)"), NOTHING, "the wire beside it");
    assert_eq!(all("c: bool, f: Compare", "x: Compare", "x = core.select(c, core.lt, f)"), NOTHING, "in any order");
    assert_eq!(all("f: Compare", "", "y = [core.lt, f]"), NOTHING, "in an array too");
    assert_eq!(all("c: bool, f: Compare", "", "y = match c {\ntrue => yield core.lt\nfalse => yield f\n}"), NOTHING, "in an arm too");
    assert_eq!(all("", "x: f32", "x = takes(0.5, core.add)"), ["LC3008"]);
}

#[test]
fn a_primitive_without_variables_has_its_own_type() {
    assert_eq!(judged("", "r: f32", "x = core.to_f32\nr = core.call(x, 1.5)"), NOTHING);
    assert_eq!(judged("", "x: fn(f64) -> (f32)", "x = core.to_f32"), NOTHING);
    assert_eq!(judged("", "x: fn([string]) -> (string)", "x = string.concat"), NOTHING, "a variadic parameter counts as an array");
    assert_eq!(judged("", "x: fn(string, error) -> (f64, error)", "x = string.to_f64"), NOTHING, "a default does not shorten a signature");
    assert_eq!(judged("", "x: fn(string) -> (f64, error)", "x = string.to_f64"), ["LC3003"]);
    assert_eq!(judged("", "x: fn() -> (timestamp)", "x = time.now"), NOTHING);
}

#[test]
fn some_primitives_have_no_function_type() {
    for primitive in ["core.call", "string.format", "core.with_unit", "core.unitless", "errors.make", "errors.raise_if"] {
        assert_eq!(judged("", "", &format!("x = {primitive}")), ["LC3008"], "{primitive}");
        assert_eq!(judged("", "x: fn(f64) -> (f64)", &format!("x = {primitive}")), ["LC3008"], "{primitive}");
    }
}

#[test]
fn a_default_value_has_the_type_of_its_parameter() {
    for accepted in [
        "gain: f32 = 0.5",
        "v: f32[V] = 1.5 V",
        "n: i32[mV] = 5 mV",
        "z: c128 = 1.5",
        "v: f64[V] = 200 mV",
        "t: duration = 5 s",
        "b: i32 = 5",
        "b: f64 = 5",
        "s: string = \"a\"",
        "e: error = none",
        "r: Reading = Reading.Value(1 V)",
        "r: Reading = Reading.Pair(1, \"a\")",
        "p: Point = Point { x: 1 m, label: \"a\" }",
        "f: fn(f64, f64) -> (bool) = core.lt",
        "f: fn(f64[V], f64) -> (f64[V]) = scale",
        "a: [f32] = [0.5, 1.5]",
        "a: [i32] = [1]",
        "a: [i32] = []",
    ] {
        assert_eq!(with_default(accepted), NOTHING, "{accepted}");
    }
    for refused in [
        "n: i32 = 1.5",
        "n: i32[mV] = 1 V",
        "s: string = 1",
        "e: error = 1",
        "v: f64[V] = 1",
        "x: f64 = []",
        "r: Reading = Verdict.Pass",
        "r: Reading = Reading.Value",
        "r: Reading = Reading.Value()",
        "r: Reading = Reading.Value(\"a\")",
        "r: Reading = Reading.Overload(1)",
        "r: Reading = Reading.Pair(1)",
        "g: Grade = Verdict.Pass",
        "p: Point = Point { x: 1 m }",
        "p: Point = Point { x: 1 m, x: 2 m, label: \"a\" }",
        "p: Point = Point { x: 1 V, label: \"a\" }",
        "v: Verdict = Verdict { a: 1 }",
        "x: f64 = f64 { a: 1 }",
        "x: i32 = Point { x: 1 m, label: \"a\" }",
        "f: fn(f64, f64) -> (bool) = core.add",
        "f: fn(f64, f64) -> (f64) = scale",
        "f: fn(f64) -> (f64) = core.call",
        "a: [f64] = [1, \"a\"]",
        "w: waveform<f64> = waveform { t0: 0, dt: 1 ms, samples: [] }",
        "w: waveform<f64> = waveform { dt: 1 ms, samples: [] }",
        "x: f64 = [[]]",
        "x: f64 = [core.abs]",
        "p: Point = Point { x: [[]], label: \"a\" }",
        "r: Reading = Reading.Value([[]])",
    ] {
        let codes = with_default(refused);
        assert_eq!(codes, ["LC3017"], "{refused}");
    }
}

#[test]
fn a_default_keeps_the_codes_of_what_it_names_and_of_its_numbers() {
    assert_eq!(with_default("gain: f32 = 1e39"), ["LC3018"]);
    assert_eq!(with_default("n: i8 = 300"), ["LC3018"]);
    assert_eq!(with_default("p: Point = Point { x: 1 m, z: 1 }"), ["LC2006"]);
    assert_eq!(with_default("r: Reading = Reading.Nowhere"), ["LC2007"]);
}

#[test]
fn a_default_has_its_terminal_and_no_conversion_node() {
    let diagnostic = only("v: i32 = 1.5", "", "");
    assert_eq!(diagnostic.code().as_str(), "LC3017");
    assert_eq!(diagnostic.wires.first().unwrap().to.as_deref(), Some("fn.v"));
    assert_eq!(diagnostic.fixes.len(), 0, "a default reads nothing of the diagram (R-21)");
    let diagnostic = only("v: f32[V] = 1.5", "", "");
    assert!(diagnostic.fixes.first().unwrap().title.contains("1.5 V"));
    let diagnostic = only("p: Point = Point { x: 1 V, label: \"a\" }", "", "");
    assert_eq!(diagnostic.wires.first().unwrap().to.as_deref(), Some("fn.p"));
    let diagnostic = only("v: f64[V] = Point { x: 1 m, label: \"a\" }", "", "");
    assert_eq!(diagnostic.wires.first().unwrap().to.as_deref(), Some("fn.v"));
}

#[test]
fn an_extern_has_its_defaults_judged() {
    let declared = |param: &str| {
        codes(&[
            ("h.lcf", format!("extern rust(crate: \"a\", function: \"h\")\nfn h({param}) -> (err: error)").as_str()),
            ("Volts.lct", "type Volts = f64[V]"),
        ])
    };
    assert_eq!(declared("gain: f32 = 0.5"), NOTHING);
    assert_eq!(declared("gain: i32 = 0.5"), ["LC3017"]);
    assert_eq!(declared("v: Volts = 1"), ["LC3017"]);
}

#[test]
fn a_signature_with_an_error_says_nothing_more_of_its_defaults() {
    assert_eq!(judged("a: i32 = \"x\", a: i32", "", ""), ["LC2019"]);
    assert_eq!(codes(&[("g.lcf", "fn g<T: Number>(a: i32 = \"x\") -> () {}")]), ["LC4006"]);
    assert_eq!(judged("a: Nowhere = 1", "", ""), ["LC2003"]);
    assert_eq!(judged("a: Nowhere, b: i32 = \"x\"", "", ""), ["LC2003", "LC3017"], "another parameter, and another mistake");
}
