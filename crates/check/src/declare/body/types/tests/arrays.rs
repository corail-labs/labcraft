//! The array nodes of appendix A, rows that become a rectangle (3.1), and a node of constants that takes its type from what its line
//! binds (3.5).

use super::*;

const INPUTS: &str = "row: [f64], rows: [[f64]], m: [f64; 2], cube: [f64; 3], x: f64, xs: [f64], v: [f64[V]], mv: [f64[mV]], n: i32, c: bool";

fn with(outputs: &str, body: &str) -> Vec<&'static str> {
    judged(INPUTS, outputs, body)
}

#[test]
fn rows_become_a_rectangle_through_a_node() {
    assert_eq!(with("a: [f64; 2], e: error", "a, e = array.from_rows(rows)"), NOTHING);
    assert_eq!(with("a: [f64; 2], e: error", "a, e = array.from_rows([row, row])"), NOTHING, "the node that an array of wires needs");
    assert_eq!(with("a: [f64; 2], e: error", "a, e = array.from_rows([[1.5], [2.5, 3.5]])"), NOTHING, "the node says it, at run time");
    assert_eq!(with("r: [[f64]]", "r = array.to_rows(m)"), NOTHING);
    assert_eq!(with("r: [f64; 2], e: error", "r, e = array.from_rows(rows, err: e0)\ne0 = none"), NOTHING);
    assert_eq!(with("r: [[f64]]", "r = array.to_rows(rows)"), ["LC3001"], "these rows are free already");
    assert_eq!(with("a: [f64; 2], e: error", "a, e = array.from_rows(m)"), ["LC3001"], "this one is rectangular already");
    assert_eq!(with("a: [f64; 2], e: error", "a, e = array.from_rows(row)"), ["LC3001"], "no rows");
    assert_eq!(with("r: [[f64]]", "r = array.to_rows(cube)"), ["LC3001"], "two dimensions only");
    assert_eq!(with("r: [[f64]]", "r = array.to_rows(x)"), ["LC3001"]);
}

#[test]
fn from_rows_holds_what_the_rows_hold() {
    let deep = "deep: [[[f64]]]";
    assert_eq!(judged(deep, "a: [[f64]; 2], e: error", "a, e = array.from_rows(deep)"), NOTHING);
    assert_eq!(judged(deep, "a: [f64; 2], e: error", "a, e = array.from_rows(deep)"), ["LC3003"]);
}

#[test]
fn a_constant_of_rows_has_one_length() {
    assert_eq!(with("r: [f64; 2]", "r = [[1.5, 2.5], [3.5, 4.5]]"), NOTHING);
    assert_eq!(with("r: [f64; 2]", "r = [[1.5, 2.5]]"), NOTHING);
    assert_eq!(with("r: [f64; 2]", "r = [[], []]"), NOTHING);
    assert_eq!(with("r: [f64; 2]", "r = [[1.5, 2.5], [3.5]]"), ["LC3003"]);
    assert_eq!(with("r: [f64; 2]", "r = [[], [3.5]]"), ["LC3003"]);
    assert_eq!(with("r: [f64; 2]", "k = [1.5, 2.5]\nr = [k, [3.5]]"), ["LC3003"], "under a name too");
    assert_eq!(with("r: [f64; 2]", "k = [[1.5, 2.5], [3.5]]\nr = k"), ["LC3003"]);
    assert_eq!(with("r: [[f64]]", "r = [[1.5, 2.5], [3.5]]"), NOTHING, "free rows take it");
    let two = [("two.lcf", "fn two(a: [f64; 2]) -> (r: i32) {\n    r = 1\n}")];
    assert_eq!(judged_with(&two, "", "r: i32", "r = two([[1.5, 2.5], [3.5]])"), ["LC3001"], "the code of where it goes");
}

#[test]
fn what_is_refused_as_a_rectangle_says_why_and_what_to_do() {
    let say = |body: &str| {
        let text = program(INPUTS, "r: [f64; 2]", body);
        let found = diagnostics(&[("f.lcf", &text)]);
        assert_eq!(found.len(), 1, "{body}");
        found.into_iter().next().unwrap()
    };
    let constant = say("r = [[1.5, 2.5], [3.5]]");
    assert!(constant.message.contains("different lengths"), "{}", constant.message);
    assert!(constant.fixes.iter().any(|fix| fix.title.contains("one length")));
    for body in ["r = [row, row]", "r = rows", "r = for y in m {\ncollect y\n}", "r = array.push(m, row)"] {
        let wired = say(body);
        assert!(!wired.message.contains("different lengths"), "{body}");
        assert!(wired.fixes.iter().any(|fix| fix.title.contains("array.from_rows")), "{body}: {:?}", wired.fixes);
    }
    let text = program(INPUTS, "r: [[f64]]", "r = m");
    let back = diagnostics(&[("f.lcf", &text)]);
    assert!(back.iter().any(|one| one.fixes.iter().any(|fix| fix.title.contains("array.to_rows"))));
}

#[test]
fn slice_goes_by_start_and_length() {
    assert_eq!(with("a: [f64], e: error", "a, e = array.slice(xs)"), NOTHING);
    assert_eq!(with("a: [f64], e: error", "a, e = array.slice(xs, 2)"), NOTHING);
    assert_eq!(with("a: [f64], e: error", "a, e = array.slice(xs, 2, n)"), NOTHING);
    assert_eq!(with("a: [f64], e: error", "a, e = array.slice(xs, length: 5)"), NOTHING);
    assert_eq!(with("a: [f64], e: error", "a, e = array.slice(xs, start: n, length: 5, err: none)"), NOTHING);
    assert_eq!(with("a: [f64], e: error", "a, e = array.slice(xs, x)"), ["LC3004"], "an index is an integer");
    assert_eq!(with("a: [f64], e: error", "a, e = array.slice(x)"), ["LC3001"]);
    assert_eq!(with("a: [f64]", "a = array.slice(xs)"), ["LC2103"], "it can fail, and says so");
}

#[test]
fn a_slice_of_a_rectangle_stays_one() {
    assert_eq!(with("a: [f64; 2], e: error", "a, e = array.slice(m, 1)"), NOTHING);
    assert_eq!(with("a: [[f64]], e: error", "a, e = array.slice(m, 1)"), ["LC3003"]);
    assert_eq!(with("a: [[f64]], e: error", "a, e = array.slice(rows, 1)"), NOTHING);
    assert_eq!(with("a: [f64; 3], e: error", "a, e = array.slice(cube, 1)"), NOTHING);
}

#[test]
fn set_takes_an_item_of_the_array() {
    assert_eq!(with("a: [f64], e: error", "a, e = array.set(xs, 0, x)"), NOTHING);
    assert_eq!(with("a: [f64], e: error", "a, e = array.set(xs, n, 1)"), NOTHING, "the array fixes it");
    assert_eq!(with("a: [f64], e: error", "a, e = array.set(xs, 0, \"a\")"), ["LC3001"]);
    assert_eq!(with("a: [f64], e: error", "a, e = array.set(xs, x, x)"), ["LC3004"]);
    assert_eq!(with("a: [[f64]], e: error", "a, e = array.set(m, 0, row)"), NOTHING, "nothing keeps the rows equal");
    assert_eq!(with("a: [f64; 2], e: error", "a, e = array.set(m, 0, row)"), ["LC3003"]);
}

#[test]
fn repeat_holds_its_item() {
    assert_eq!(with("a: [f64], e: error", "a, e = array.repeat(x, 3)"), NOTHING);
    assert_eq!(with("a: [f64], e: error", "a, e = array.repeat(x, n)"), NOTHING);
    assert_eq!(with("a: [[f64]], e: error", "a, e = array.repeat(row, 3)"), NOTHING);
    assert_eq!(with("a: [f64], e: error", "a, e = array.repeat(x, x)"), ["LC3004"]);
    assert_eq!(with("a: [f64; 2], e: error", "a, e = array.repeat(row, 3)"), ["LC3003"]);
}

#[test]
fn concat_takes_any_number_of_arrays_of_one_type() {
    assert_eq!(with("a: [f64]", "a = array.concat(xs, xs, xs)"), NOTHING);
    assert_eq!(with("a: [f64]", "a = array.concat(xs)"), NOTHING);
    assert_eq!(with("a: [f64]", "a = array.concat(xs, [1, 2])"), NOTHING, "the array fixes the type of a constant");
    assert_eq!(with("a: [f64]", "a = array.concat([1, 2], xs)"), NOTHING, "in any order");
    assert_eq!(with("a: [f64]", "a = array.concat(xs, row)"), NOTHING);
    assert_eq!(with("a: [f64[V]]", "a = array.concat(v, mv)"), ["LC3001"], "nothing converts between arrays (3.4)");
    assert_eq!(with("a: [f64]", "a = array.concat(xs, x)"), ["LC3001"]);
    assert_eq!(with("a: [[f64]]", "a = array.concat(m, m)"), NOTHING, "nothing keeps the rows equal");
    assert_eq!(with("a: [f64]", "a = array.concat(parts: xs)"), ["LC2009"], "variadic");
}

#[test]
fn concat_without_an_input_needs_a_type() {
    assert_eq!(with("a: [f64]", "a = array.concat()"), NOTHING);
    assert_eq!(with("a: [f64]", "a = array.concat<f64>()"), NOTHING);
    assert_eq!(with("", "a = array.concat()\n_ = array.len(a)"), ["LC3019"]);
    assert_eq!(with("", "a = array.concat<f64>()\n_ = array.len(a)"), NOTHING);
}

#[test]
fn the_new_nodes_as_references() {
    let goes = |ty: &str, name: &str| judged_with(&[("h.lcf", &format!("fn h(f: {ty}) -> () {{}}"))], "", "", &format!("h({name})"));
    assert_eq!(goes("fn([f64], i32, f64, error) -> ([f64], error)", "array.set"), NOTHING);
    assert_eq!(goes("fn(f64, i32, error) -> ([f64], error)", "array.repeat"), NOTHING);
    assert_eq!(goes("fn([f64], i32, i32, error) -> ([f64], error)", "array.slice"), NOTHING);
    assert_eq!(goes("fn([[f64]]) -> ([f64])", "array.concat"), NOTHING, "a variadic parameter counts as an array");
    assert_eq!(goes("fn([f64]) -> ([f64])", "array.concat"), ["LC3008"]);
    assert_eq!(goes("fn([[f64]], error) -> ([f64; 2], error)", "array.from_rows"), NOTHING);
    assert_eq!(goes("fn([f64; 2]) -> ([[f64]])", "array.to_rows"), NOTHING);
    assert_eq!(goes("fn([[f64]]) -> ([[f64]])", "array.to_rows"), ["LC3008"]);
}

#[test]
fn a_node_of_constants_takes_its_type_from_its_line() {
    assert_eq!(with("a: [f64], e: error", "a, e = array.repeat(0, 100)"), NOTHING);
    assert_eq!(with("a: [f32], e: error", "a, e = array.repeat(0.5, 100)"), NOTHING);
    assert_eq!(with("a: [f64[V]], e: error", "a, e = array.repeat(0 V, 100)"), NOTHING);
    assert_eq!(with("a: f32", "a = core.select(c, 0.5, 1.5)"), NOTHING);
    assert_eq!(with("a: [f32]", "a = array.push([], 0.5)"), NOTHING);
    assert_eq!(with("a: [f32]", "a = array.concat([0.5], [1.5])"), NOTHING);
    assert_eq!(with("a: f32", "a = core.add(0.5, 1.5)"), NOTHING);
    assert_eq!(with("a: [f64; 2], e: error", "a, e = array.from_rows([])"), NOTHING);
    assert_eq!(with("a: [f64; 2], e: error", "a, e = array.slice([[0.5, 1.5]])"), ["LC3003"], "its array is rows, as written");
    assert_eq!(with("a: f32, e: error", "a, e = array.get([0.5, 1.5], 0)"), NOTHING);
    assert_eq!(with("a: f32", "k = 0.5\na = core.select(c, k, k)"), NOTHING, "a named constant is a constant");
}

#[test]
fn a_node_with_a_wire_keeps_the_rule_of_its_wire() {
    assert_eq!(judged("s: f32", "a: f64", "a = core.add(s, 0.5)"), NOTHING);
    assert_eq!(judged("d: f64", "a: f32", "a = core.add(d, 0.5)"), ["LC3003"], "a wire says the type");
    assert_eq!(judged("d: f64", "a: [f32], e: error", "a, e = array.repeat(d, 3)"), ["LC3003"]);
    assert_eq!(judged("xs: [f64]", "a: [f32], e: error", "a, e = array.slice(xs)"), ["LC3003"]);
}

#[test]
fn a_constant_that_does_not_fit_is_said_on_the_wire_that_leaves() {
    assert_eq!(with("a: f64", "a = core.select(c, \"x\", \"y\")"), ["LC3003"]);
    assert_eq!(with("a: i32", "a = core.select(c, 0.5, 1.5)"), ["LC3003"]);
    assert_eq!(with("a: [i32], e: error", "a, e = array.repeat(0.5, 3)"), ["LC3003"]);
    assert_eq!(with("a: f64[V]", "a = core.add(1, 2)"), ["LC3003"], "a number without a unit has no dimension");
}

#[test]
fn a_name_without_a_type_gives_nothing() {
    assert_eq!(with("a: [f64]", "r, e = array.repeat(0, 3)\n_ = e\na = r"), ["LC3003"], "the constant keeps `i32`");
}

#[test]
fn what_is_ordered_has_no_complex_type_from_its_line() {
    assert_eq!(with("a: c64", "a = core.min(0.5, 1.5)"), ["LC3003"]);
    assert_eq!(with("a: c64", "a = core.add(0.5, 1.5)"), NOTHING);
}

#[test]
fn a_constant_of_three_dimensions_has_one_length_at_each_level() {
    assert_eq!(with("r: [f64; 3]", "r = [[[1.5, 2.5]], [[3.5, 4.5]]]"), NOTHING);
    assert_eq!(with("r: [f64; 3]", "r = [[[1.5, 2.5]], [[3.5]]]"), ["LC3003"], "across its parts");
    assert_eq!(with("r: [f64; 3]", "r = [[[1.5]], [[2.5], [3.5]]]"), ["LC3003"]);
    assert_eq!(with("r: [f64; 3]", "k = [[1.5], [2.5, 3.5]]\nr = [k, k]"), ["LC3003"], "under a name");
    assert_eq!(
        with("r: [f64; 2], q: [f64; 3]", "k = [[1.5], [2.5, 3.5]]\nr = k\nq = [k, k]"),
        ["LC3003", "LC3003"],
        "what a name takes is not kept from around it"
    );
}

#[test]
fn a_deep_chain_of_names_is_looked_at_once() {
    let mut lines = String::from("a0 = [1.5, 2.5]\n");
    for level in 1..40 {
        lines.push_str(&format!("a{level} = [a{}, a{}]\n", level - 1, level - 1));
    }
    assert_eq!(with("r: [[f64]]", &format!("{lines}_ = a39\nr = [a0, a0]")), NOTHING);
}

#[test]
fn the_fix_of_rows_is_the_one_that_solves_it() {
    let fixes = |inputs: &str, outputs: &str, body: &str| -> Vec<String> {
        let text = program(inputs, outputs, body);
        diagnostics(&[("f.lcf", &text)]).into_iter().flat_map(|one| one.fixes).map(|fix| fix.title).collect()
    };
    assert!(fixes("ints: [[i32]]", "r: [f64; 2]", "r = ints").iter().all(|title| !title.contains("from_rows")), "the elements differ too");
    assert!(fixes("rows: [[f64]]", "r: [f64[V]; 2]", "r = rows").iter().all(|title| !title.contains("from_rows")));
    assert!(fixes("m: [i32; 2]", "r: [[f64]]", "r = m").iter().all(|title| !title.contains("to_rows")));
    let carried = |next: &str| {
        fixes("m: [f64; 2], rows: [[f64]]", "x: [f64; 2]", &format!("x = loop carry a: [f64; 2] = m {{\nnext a = {next}\nuntil true\n}}"))
    };
    assert!(carried("rows").iter().any(|title| title.contains("array.from_rows")), "{:?}", carried("rows"));
    assert!(carried("[[1.5], [2.5, 3.5]]").iter().any(|title| title.contains("one length")));
}

#[test]
fn to_rows_takes_a_constant_of_rows() {
    assert_eq!(with("", "r = array.to_rows([[1.5, 2.5], [3.5, 4.5]])\n_ = array.len(r)"), NOTHING);
    assert_eq!(with("", "k = [[1.5, 2.5]]\nr = array.to_rows(k)\n_ = array.len(r)"), NOTHING);
    assert_eq!(with("", "r = array.to_rows([[1, 2], [3, 4]])\n_ = array.len(r)"), NOTHING);
    assert_eq!(with("r: [[f64]]", "r = array.to_rows([[1.5, 2.5]])"), NOTHING);
    assert_eq!(with("", "r = array.to_rows([[1.5], [2.5, 3.5]])\n_ = array.len(r)"), ["LC3001"]);
    assert_eq!(with("", "r = array.to_rows([row, row])\n_ = array.len(r)"), ["LC3001"], "wires");
}

#[test]
fn a_wire_comes_before_a_constant_on_the_arrays_of_a_node() {
    assert_eq!(with("a: [f64[V]]", "a = array.concat(v, [1 mV])"), NOTHING);
    assert_eq!(with("a: [f64[V]]", "a = array.concat([1 mV], v)"), NOTHING);
    assert_eq!(judged("v: f64[V]", "a: [f64[V]]", "a = array.push([1 mV], v)"), ["LC3003"], "an array decides, even a constant one");
}

#[test]
fn a_constant_on_a_temperature_scale_keeps_its_scale() {
    assert_eq!(with("a: f64[K]", "a = core.add(20 \u{00B0}C, 10 \u{00B0}C)"), NOTHING, "by the wire that leaves, after adding in \u{00B0}C");
    let converted = |body: &str| {
        let text = program(INPUTS, "a: f64[K]", body);
        let mut run = run(&[("f.lcf", &text)]);
        assert!(run.report.diagnostics().is_empty());
        run.files.remove(0)
    };
    // Only the wire that leaves converts: each constant that follows would have a conversion of its own, and the sum another value.
    let conversions = |output: &str, body: &str| {
        let text = program(INPUTS, output, body);
        let mut run = run(&[("f.lcf", &text)]);
        assert!(run.report.diagnostics().is_empty(), "{body}");
        run.files.remove(0).resolved.coercions().len()
    };
    assert_eq!(converted("a = core.add(20 \u{00B0}C, 10 \u{00B0}C)").resolved.coercions().len(), 1);
    assert_eq!(conversions("a: f64[K]", "a = array.sum([20 \u{00B0}C, 10 \u{00B0}C])"), 1, "in an array too");
    assert_eq!(conversions("a: f64[K]", "k = [20 \u{00B0}C, 10 \u{00B0}C]\na = array.sum(k)"), 1, "under a name too");
    assert_eq!(conversions("a: f64[\u{00B0}C]", "a = core.add(20 K, 10 K)"), 1, "toward a scale too");
    assert_eq!(conversions("a: f64[\u{00B0}F]", "a = core.add(20 \u{00B0}C, 10 \u{00B0}C)"), 1, "between two scales");
    assert_eq!(conversions("a: f64[K]", "k = 20 \u{00B0}C\na = array.sum([k])"), 1, "and what was tried leaves nothing behind");
    assert_eq!(conversions("a: f64[K]", "a = core.add(20 K, 10 K)"), 0, "the same scale has nothing to convert");
    assert_eq!(conversions("a: f64[mV]", "a = core.add(1 V, 2 V)"), 2, "a scale without an offset follows");
    assert_eq!(conversions("a: f32", "a = core.add(1.5, 2147483648)"), 0, "what a constant cannot be on its own does not stop it");
}

#[test]
fn what_cannot_follow_is_said_with_the_type_of_the_constants() {
    let text = program(INPUTS, "a: [f32], e: error", "a, e = array.slice([1.5, \"a\"])");
    let found = diagnostics(&[("f.lcf", &text)]);
    assert_eq!(found.iter().map(|one| one.code().as_str()).collect::<Vec<_>>(), ["LC3001"]);
    assert!(found[0].message.contains("`f64`"), "the line gave nothing: {}", found[0].message);
}

#[test]
fn each_level_of_rectangles_looks_for_itself() {
    let cubes = "r: [[f64; 2]; 2]";
    assert_eq!(with(cubes, "r = [[[[1.5, 2.5], [3.5, 4.5]]]]"), NOTHING);
    assert_eq!(with(cubes, "r = [[[[1.5, 2.5], [3.5]]]]"), ["LC3001"], "written there");
    assert_eq!(with(cubes, "k = [[1.5, 2.5], [3.5]]\nr = [[k]]"), ["LC3001"], "under a name");
    assert_eq!(with("r: [f64; 3]", "r = [[[1.5], [2.5]], [[3.5], [4.5, 5.5]]]"), ["LC3003"], "the rows of the last level too");
    assert_eq!(with("r: [f64; 2]", "k = 1.5\nr = [[k], [k, k]]"), ["LC3003"], "`[k]` is a row of one");
    assert_eq!(with("r: [f64; 2]", "k = 1.5\nr = [[k], [k]]"), NOTHING);
    assert_eq!(with("r: [f64; 2]", "k = 1.5\nb = [k]\nr = [b, [k, k]]"), ["LC3003"], "a row of one under its name");
    assert_eq!(with("r: [f64; 2]", "k = 1.5\nb = [k]\nr = [b, [k]]"), NOTHING);
}

#[test]
fn an_array_of_one_row_goes_to_a_rectangle() {
    assert_eq!(with("q: [[f64]]", "k = [1.5, 2.5]\nq = array.to_rows([k])"), NOTHING);
    assert_eq!(with("", "k = [1.5, 2.5]\nq = array.to_rows([k])\n_ = array.len(q)"), NOTHING, "nothing on the line");
    assert_eq!(with("", "k = [1.5, 2.5]\nj = [k]\nq = array.to_rows(j)\n_ = array.len(q)"), NOTHING, "under a name");
    assert_eq!(with("q: [[f64]]", "q = array.to_rows([row])"), ["LC3001"], "a wire");
}

#[test]
fn a_constant_given_to_to_rows_is_a_rectangle() {
    use labcraft_graph::{Arg, Decl, Rhs, StatementKind};
    // With nothing on its line, so that only its parameter says what it is.
    let checked = run(&[("f.lcf", &program("", "", "q = array.to_rows([[1.5, 2.5]])\n_ = array.len(q)"))]);
    assert!(checked.report.diagnostics().is_empty(), "{:?}", checked.report.diagnostics());
    let file = checked.files.first().unwrap();
    let Decl::Function(function) = &file.file.decl else { panic!() };
    let value = function.body.statements.iter().find_map(|statement| match &statement.kind {
        StatementKind::Binding { value: Rhs::Call(call), .. } => call.args.first(),
        _ => None,
    });
    let Some(Arg::Value { value, .. }) = value else { panic!() };
    let own = checked.types.get(file.resolved.ty(value.id()));
    assert!(matches!(own, Ty::Array { dimensions: 2, .. }), "{own:?}");
}

#[test]
fn a_carry_says_why_its_rows_are_refused() {
    let text = program("", "x: i32", "r = loop carry m: [f64; 2] = [[1.5]] {\nnext m = [[1.5], [2.5, 3.5]]\nuntil true\n}\nx = array.len(r.m)");
    let found = diagnostics(&[("f.lcf", &text)]);
    assert!(found.iter().any(|one| one.message.contains("different lengths")), "{found:?}");
}

#[test]
fn a_line_that_uses_a_large_constant_many_times_pays_for_it_once() {
    let row = "[1.5], ";
    let lines: String = (0..20_000).map(|i| format!("q{i} = array.to_rows(k)\n_ = array.len(q{i})\n")).collect();
    let body = format!("k = [{}[1.5]]\n{lines}", row.repeat(20_000));
    assert_eq!(with("", &body), NOTHING);
    let unequal = format!("k = [{}[1.5, 2.5]]", row.repeat(20_000));
    let outputs: Vec<String> = (0..2_000).map(|i| format!("o{i}: [f64; 2]")).collect();
    let lines: String = (0..2_000).map(|i| format!("o{i} = k\n")).collect();
    assert!(judged("", &outputs.join(", "), &format!("{unequal}\n{lines}")).iter().all(|code| *code == "LC3003"));
}

#[test]
fn a_slice_follows_its_line() {
    assert_eq!(with("a: [f32], e: error", "a, e = array.slice([0.5, 1.5])"), NOTHING);
}

#[test]
fn what_a_line_says_of_a_concat_stays_silent_when_it_is_unknown() {
    let body = "x = loop carry r: Nowhere = 1 {\nnext r = array.concat()\nuntil true\n}";
    assert_eq!(with("x: i32", body), ["LC2003"]);
    assert_eq!(with("a: [f64], b: [f64]", "a, b = array.concat()"), ["LC2103"], "not said twice");
}

#[test]
fn a_reference_to_slice_keeps_a_rectangle_and_one_to_concat_takes_rows() {
    let goes = |ty: &str, name: &str| judged_with(&[("h.lcf", &format!("fn h(f: {ty}) -> () {{}}"))], "", "", &format!("h({name})"));
    assert_eq!(goes("fn([f64; 2], i32, i32, error) -> ([f64; 2], error)", "array.slice"), NOTHING);
    assert_eq!(goes("fn([f64; 2], i32, i32, error) -> ([[f64]], error)", "array.slice"), ["LC3008"]);
    assert_eq!(goes("fn([f64; 2]) -> ([f64])", "array.concat"), ["LC3008"], "a variadic parameter is an array of what it takes");
    assert_eq!(goes("fn([f64; 2]) -> ([[f64]])", "array.concat"), ["LC3008"], "and not a rectangle of its rows");
}
