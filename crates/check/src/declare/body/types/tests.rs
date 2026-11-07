//! Pass 4, on small programs: the rules of 3.3, 3.4, R-7 and section 4, and what must stay silent.

use std::borrow::Cow;

use labcraft_types::{Diagnostic, Interner, Limits, NumberKind, Ty, Unit};

use crate::{Checked, Content, Found, Libraries, LibrarySource, Origin, SourceFile, check};

struct Nothing;

impl Libraries for Nothing {
    fn find(&self, _: &str, _: Origin) -> Found {
        Found::NotFound
    }
}

fn source(files: &[(&str, &str)]) -> LibrarySource {
    let files = files
        .iter()
        .map(|(shown, text)| {
            let text = format!("labcraft 1\n\n{text}\n");
            SourceFile { shown: (*shown).to_owned(), content: Content::Bytes(Cow::Owned(text.into_bytes())) }
        })
        .collect();
    LibrarySource { files, ..LibrarySource::new(String::new(), String::new(), Origin::Project) }
}

fn run(files: &[(&str, &str)]) -> Checked {
    let mut names = Interner::new();
    check(source(files), &Nothing, &mut names, &Limits::default())
}

fn diagnostics(files: &[(&str, &str)]) -> Vec<Diagnostic> {
    let mut report = run(files).report;
    report.finish();
    report.diagnostics().iter().filter(|diagnostic| !crate::declare::body::later_pass(diagnostic.code())).cloned().collect()
}

fn codes(files: &[(&str, &str)]) -> Vec<&'static str> {
    diagnostics(files).iter().map(|diagnostic| diagnostic.code().as_str()).collect()
}

const USES: &str = "use array\nuse core\nuse errors\nuse string\nuse time\n";

/// A function with these inputs and outputs and this body, beside the types `Reading`, `Point`, `Verdict` and `Grade` and the
/// functions `scale` and `measure`.
fn program(inputs: &str, outputs: &str, body: &str) -> String {
    format!("{USES}\nfn f({inputs}) -> ({outputs}) {{\n{body}\n}}")
}

fn judged_with(extra: &[(&str, &str)], inputs: &str, outputs: &str, body: &str) -> Vec<&'static str> {
    let text = program(inputs, outputs, body);
    let mut files = vec![
        ("f.lcf", text.as_str()),
        ("reading.lct", "type Reading = enum { Value(f64[V]), Pair(i32, string), Overload }"),
        ("point.lct", "type Point = struct { x: f64[m], label: string }"),
        ("verdict.lct", "type Verdict = enum { Pass, Fail }"),
        ("grade.lct", "type Grade = enum { Pass, Fail }"),
        ("scale.lcf", "fn scale(x: f64[V], gain: f64 = 1.0) -> (y: f64[V]) {\n    y = x\n}"),
        ("measure.lcf", "fn measure(x: f64[V], err: error = none) -> (y: f64[V], err: error) {\n    y = x\n    err = err\n}"),
    ];
    files.extend_from_slice(extra);
    codes(&files)
}

fn judged(inputs: &str, outputs: &str, body: &str) -> Vec<&'static str> {
    judged_with(&[], inputs, outputs, body)
}

const NOTHING: [&str; 0] = [];

/// `line`, bound to an output of this type, from inputs of many types.
fn gives(ty: &str, line: &str) -> Vec<&'static str> {
    judged(
        "n: i32, m: i64, k: i16, b: u8, w: u32, x: f64, s: f32, v: f64[V], mv: f64[mV], i: f64[A], c: c128, t: string, d: duration, stamp: timestamp, nmv: i32[mV], nv: i32[V], cel: f64[°C], kel: f64[K], ok: bool, xs: [f64], ss: [string]",
        &format!("out: {ty}"),
        &format!("out = {line}"),
    )
}

#[test]
fn an_alias_is_another_name() {
    let types =
        [("volts.lct", "type Volts = f64[V]"), ("alias.lct", "type A = Verdict"), ("again.lct", "type B = A"), ("many.lct", "type Many = [f64[V]]")];
    assert_eq!(judged_with(&types, "v: Volts", "x: f64[V]", "x = v"), NOTHING);
    assert_eq!(judged_with(&types, "v: f64[V]", "x: Volts", "x = v"), NOTHING);
    assert_eq!(judged_with(&types, "v: f64[mV]", "x: Volts", "x = v"), NOTHING, "a conversion applies through the name");
    assert_eq!(judged_with(&types, "v: f64[A]", "x: Volts", "x = v"), ["LC3003"]);
    assert_eq!(judged_with(&types, "v: Verdict", "x: B", "x = v"), NOTHING);
    assert_eq!(judged_with(&types, "", "x: Verdict", "x = B.Pass"), NOTHING, "an alias names the variants");
    assert_eq!(judged_with(&types, "m: Many", "x: [f64[V]]", "x = m"), NOTHING);
    assert_eq!(judged_with(&types, "v: Verdict", "x: Grade", "x = v"), ["LC3003"], "two enumerations of one shape are two types");
    let inline = "p: struct { x: f64[m], label: string }";
    assert_eq!(judged(inline, "q: Point", "q = p"), ["LC3003"], "a declared struct is not one written in place");
    assert_eq!(judged(inline, "q: struct { x: f64[m], label: string }", "q = p"), NOTHING, "two written in place are one");
}

#[test]
fn an_alias_of_error_is_still_not_written_as_error_on_an_external_function() {
    let external = "extern rust(crate: \"c\", function: \"e\")\nfn e() -> (err: E)";
    assert_eq!(codes(&[("e.lcf", external), ("t.lct", "type E = error")]), ["LC4005"], "R-13");
}

#[test]
fn widening_follows_the_list() {
    assert_eq!(gives("i32", "k"), NOTHING);
    assert_eq!(gives("i64", "n"), NOTHING);
    assert_eq!(gives("i16", "b"), NOTHING);
    assert_eq!(gives("i64", "w"), NOTHING);
    assert_eq!(gives("f64", "n"), NOTHING);
    assert_eq!(gives("f64", "s"), NOTHING);
    assert_eq!(gives("i32", "w"), ["LC3003"], "`u32` needs more than 32 bits with a sign");
    assert_eq!(gives("u32", "n"), ["LC3003"]);
    assert_eq!(gives("f64", "m"), ["LC3003"], "Q-4");
    assert_eq!(gives("f32", "n"), ["LC3003"]);
    assert_eq!(gives("c128", "x"), ["LC3003"], "a real number to a complex one is explicit");
}

#[test]
fn a_change_of_scale_is_a_conversion_and_an_integer_cannot_take_one() {
    assert_eq!(gives("f64[V]", "mv"), NOTHING);
    assert_eq!(gives("f64[V]", "nmv"), NOTHING);
    assert_eq!(gives("i32[mV]", "nmv"), NOTHING);
    assert_eq!(gives("i32[V]", "nmv"), ["LC3003"]);
    assert_eq!(gives("f64[K]", "cel"), NOTHING);
    assert_eq!(gives("f64[V]", "i"), ["LC3003"]);
}

#[test]
fn two_number_types_meet_at_the_smallest_that_both_widen_to() {
    assert_eq!(gives("f64", "core.add(n, 0.5)"), NOTHING);
    assert_eq!(gives("i32", "core.add(n, 0.5)"), ["LC3003"]);
    assert_eq!(gives("f64", "core.add(0.5, n)"), NOTHING);
    assert_eq!(gives("i32", "core.add(k, n)"), NOTHING);
    assert_eq!(gives("f64", "core.add(n, s)"), NOTHING, "no direct widening: both reach `f64`");
    assert_eq!(gives("i32", "core.add(k, b)"), NOTHING);
    assert_eq!(gives("f64", "core.add(x, m)"), ["LC3004"], "no type holds both");
    assert_eq!(gives("f64", "core.select(ok, n, x)"), NOTHING);
    assert_eq!(gives("f64[V]", "core.add(v, mv)"), NOTHING);
    assert_eq!(gives("f64[mV]", "core.add(mv, v)"), NOTHING, "the unit of the first");
    assert_eq!(gives("f64[V]", "core.add(mv, v)"), NOTHING, "a conversion on the output");
    assert_eq!(gives("f64[V]", "core.add(nv, nmv)"), NOTHING, "a change of scale goes through `f64`");
    assert_eq!(gives("i32[V]", "core.add(nv, nmv)"), ["LC3003"]);
    assert_eq!(gives("bool", "core.lt(n, x)"), NOTHING);
    assert_eq!(gives("bool", "core.eq(v, i)"), ["LC3002"]);
    assert_eq!(gives("bool", "core.lt(c, c)"), ["LC3001", "LC3001"], "complex numbers have no order");
}

#[test]
fn a_type_that_is_not_a_number_is_refused_where_a_number_is_expected() {
    assert_eq!(gives("f64", "core.add(t, t)"), ["LC3001", "LC3001"]);
    assert_eq!(gives("f64", "core.add(x, t)"), ["LC3001"]);
    assert_eq!(gives("f64", "core.abs(ok)"), ["LC3001"]);
    assert_eq!(gives("bool", "core.eq(t, t)"), NOTHING, "`T` has no bound");
    assert_eq!(gives("bool", "core.eq(t, x)"), ["LC3001"]);
    assert_eq!(gives("bool", "core.and(ok, n)"), ["LC3001"]);
}

#[test]
fn division_root_and_power_give_a_float_on_integers() {
    assert_eq!(gives("f64", "core.div(3, 2)"), NOTHING);
    assert_eq!(gives("i32", "core.div(3, 2)"), ["LC3003"]);
    assert_eq!(gives("f64", "core.div(n, 2)"), NOTHING);
    assert_eq!(gives("f32", "core.div(s, 2)"), NOTHING, "a literal takes the number type of the other");
    assert_eq!(gives("f32", "core.div(s, s)"), NOTHING);
    assert_eq!(gives("f64", "core.div(s, 2.0)"), NOTHING, "`2.0` is an `f64`");
    assert_eq!(gives("f64", "core.div(m, 2)"), ["LC3004"], "`i64` does not widen to `f64`");
    assert_eq!(gives("f64", "core.sqrt(n)"), NOTHING);
    assert_eq!(gives("f64", "core.sqrt(m)"), ["LC3004"]);
    assert_eq!(gives("f64", "core.pow(n, 2)"), NOTHING);
    assert_eq!(gives("i32", "core.mul(n, 2)"), NOTHING, "`mul` stays an integer");
    assert_eq!(gives("i32", "core.add(n, 2)"), NOTHING);
}

#[test]
fn units_follow_their_rules() {
    assert_eq!(gives("f64[W]", "core.mul(v, i)"), NOTHING);
    assert_eq!(gives("f64[Ω]", "core.div(v, i)"), NOTHING);
    assert_eq!(gives("f64[V]", "core.mul(v, 2.0)"), NOTHING);
    assert_eq!(gives("f64[V]", "core.mul(2, v)"), NOTHING);
    assert_eq!(gives("f64[V^2]", "core.pow(v, 2)"), NOTHING);
    assert_eq!(gives("f64[V]", "core.pow(v, n)"), ["LC3016"], "the exponent is a constant");
    assert_eq!(gives("f64[V]", "core.sqrt(v)"), ["LC3016"], "an odd exponent");
    assert_eq!(gives("f64[K]", "core.sub(cel, cel)"), NOTHING, "the difference of two temperatures");
    assert_eq!(gives("f64[°C]", "core.add(cel, kel)"), NOTHING);
    assert_eq!(gives("f64", "core.unitless(v, [mV])"), NOTHING);
    assert_eq!(gives("f64", "core.unitless(v, [A])"), ["LC3005"]);
    assert_eq!(gives("f64", "core.unitless(d, [ms])"), NOTHING, "a `duration` counts as an `f64[s]`");
    assert_eq!(gives("f64[V]", "core.with_unit(x, [V])"), NOTHING);
    assert_eq!(gives("f64[V]", "core.with_unit(n, [V])"), NOTHING);
    assert_eq!(gives("f64[V]", "core.with_unit(mv, [V])"), ["LC3001"]);
}

#[test]
fn a_number_without_a_unit_has_no_dimension() {
    assert_eq!(gives("bool", "core.ge(v, 0)"), ["LC3002"]);
    assert_eq!(gives("bool", "core.ge(0, v)"), ["LC3002"]);
    assert_eq!(gives("bool", "core.ge(v, 0 V)"), NOTHING);
    assert_eq!(gives("f64[V]", "core.add(v, 1)"), ["LC3002"]);
    assert_eq!(gives("f64[V]", "0"), ["LC3003"]);
    assert_eq!(gives("f64[V]", "0 V"), NOTHING);
    assert_eq!(judged("", "x: f64[V]", "x = loop carry a: f64[V] = 0 {\nnext a = a\nuntil true\n}"), ["LC3007"]);
    assert_eq!(gives("f64", "1"), NOTHING, "a literal takes the number type");
    assert_eq!(gives("f32", "1"), NOTHING);
    assert_eq!(gives("f32", "1.5"), NOTHING, "where a type is expected, a decimal takes it");
}

#[test]
fn a_fix_writes_the_unit_of_a_literal() {
    let text = program("v: f64[V]", "x: bool", "    x = core.ge(v, 0)");
    let found = diagnostics(&[("f.lcf", &text)]);
    let [diagnostic] = found.as_slice() else { panic!("{found:?}") };
    let edit = diagnostic.fixes.first().and_then(|fix| fix.edits.first()).unwrap();
    assert_eq!(edit.text, "0 V");
}

#[test]
fn a_fix_of_a_narrowing_has_its_line_and_its_use() {
    let text = "use array\n\nfn f(values: [f64], x: f64, x2: i32) -> (item: f64, err: error) {\n    item, err = array.get(values, x)\n}";
    let found = diagnostics(&[("f.lcf", text)]);
    let [diagnostic] = found.as_slice() else { panic!("{found:?}") };
    assert_eq!(diagnostic.code().as_str(), "LC3004");
    let fix = diagnostic.fixes.first().unwrap();
    assert!(fix.title.contains("core.round_to_i32"));
    let edits: Vec<&str> = fix.edits.iter().map(|edit| edit.text.as_str()).collect();
    assert_eq!(edits, ["x3 = core.round_to_i32(x)\n    ", "x3", "use core\n"], "a name that the function does not have");
}

#[test]
fn a_literal_has_bounds_in_its_type() {
    assert_eq!(gives("i8", "300"), ["LC3018"]);
    assert_eq!(gives("u8", "-1"), ["LC3018"]);
    assert_eq!(gives("i32", "2147483647"), NOTHING);
    assert_eq!(gives("i32", "2147483648"), ["LC3018"]);
    assert_eq!(gives("i64", "2147483648"), NOTHING);
    assert_eq!(judged("", "", "x = 2147483648"), ["LC3018"], "`i32` if nothing else");
    assert_eq!(gives("i32", "core.add(k, 40000)"), ["LC3018"], "the first is an `i16`");
}

#[test]
fn the_first_arm_that_decides_gives_the_type() {
    let two = |first: &str, second: &str| format!("x = match c {{\ntrue => yield {first}\nfalse => yield {second}\n}}");
    let inputs = "c: bool, v: f64[V], n: i32, y: f64, rs: [i32]";
    assert_eq!(judged(inputs, "x: f64[V]", &two("v", "200 mV")), NOTHING);
    assert_eq!(judged(inputs, "x: f64", &two("y", "n")), NOTHING, "widened to the first");
    assert_eq!(judged(inputs, "x: f64", &two("n", "y")), ["LC3006"], "the order of branches can matter");
    assert_eq!(judged(inputs, "x: f64", &two("1", "2.5")), NOTHING, "an integer does not decide");
    assert_eq!(judged(inputs, "x: f64", &two("2.5", "1")), NOTHING);
    assert_eq!(judged(inputs, "x: i32", &two("1", "2")), NOTHING);
    assert_eq!(judged(inputs, "x: f64", &two("1", "2")), NOTHING, "what a target expects decides when no arm does");
    assert_eq!(judged(inputs, "x: [i32]", &two("[]", "rs")), NOTHING);
    assert_eq!(judged(inputs, "x: [i32]", &two("[]", "[]")), NOTHING);
    assert_eq!(judged(inputs, "", &two("[]", "[]")), ["LC3019"], "once");
    assert_eq!(judged(inputs, "x: f64[V]", &two("v", "1")), ["LC3006"]);
}

#[test]
fn the_type_of_a_carry_is_on_its_line() {
    let inputs = "n: i32, y: f64, mv: f64[mV]";
    let looped = |clauses: &str, next: &str| format!("x = loop {clauses} {{\nnext total = {next}\nuntil true\n}}");
    assert_eq!(judged(inputs, "x: f64", &looped("carry total: f64 = 0", "n")), NOTHING, "widened");
    assert_eq!(judged(inputs, "x: i32", &looped("carry total = 0", "core.add(total, 0.5)")), ["LC3007"]);
    assert_eq!(judged(inputs, "x: f64", &looped("carry total: f64 = 0", "core.add(total, 0.5)")), NOTHING);
    assert_eq!(judged(inputs, "x: f64[V]", &looped("carry total = 0 V", "mv")), NOTHING, "converted");
    assert_eq!(judged(inputs, "x: i32", &looped("carry total: i32 = 0.5", "n")), ["LC3007"], "its initial value too");
    assert_eq!(judged(inputs, "", "_ = loop carry r = [] {\nnext r = r\nuntil true\n}"), ["LC3019"]);
    assert_eq!(judged(inputs, "x: [i32]", "x = loop carry r: [i32] = [] {\nnext r = r\nuntil true\n}"), NOTHING);
    assert_eq!(judged(inputs, "x: i32, y2: f64", "x, y2 = match true {\n_ => yield (1, 2)\n}"), NOTHING);
}

#[test]
fn a_structure_gives_its_carries_then_its_collects() {
    let inputs = "xs: [f64[V]], m: [f64; 2]";
    assert_eq!(judged(inputs, "t: f64[V], ys: [f64[V]]", "t, ys = for x in xs, carry a = 0 V {\nnext a = core.add(a, x)\ncollect x\n}"), NOTHING);
    assert_eq!(judged(inputs, "t: f64[V], ys: [string]", "t, ys = for x in xs, carry a = 0 V {\nnext a = core.add(a, x)\ncollect x\n}"), ["LC3003"]);
    assert_eq!(judged(inputs, "ys: [i32]", "ys = for i in 0..3 {\ncollect i\n}"), NOTHING);
    assert_eq!(judged(inputs, "ys: [[f64]]", "ys = for row in m {\ncollect row\n}"), NOTHING, "one level of an array");
    assert_eq!(judged(inputs, "", "_ = for x in 3 {\ng = x\n}"), ["LC3020"]);
    assert_eq!(judged(inputs, "", "_ = for i in 0..1.5 {\ng = i\n}"), ["LC3020"]);
    assert_eq!(judged(inputs, "", "_ = for i in 0 V..3 {\ng = i\n}"), ["LC3020"]);
}

#[test]
fn two_dimensions_are_not_an_array_of_arrays() {
    let inputs = "m: [f64; 2], rows: [[f64]], row: [f64]";
    assert_eq!(judged(inputs, "r: [f64; 2]", "r = array.push(m, row)"), ["LC3003"]);
    assert_eq!(judged(inputs, "r: [f64; 2]", "r = for x in m {\ncollect x\n}"), ["LC3003"]);
    assert_eq!(judged(inputs, "r: [f64; 2]", "r = rows"), ["LC3003"]);
    assert_eq!(judged(inputs, "r: [[f64]]", "r = m"), ["LC3003"]);
    assert_eq!(judged(inputs, "r: [f64; 2]", "r = [row, row]"), ["LC3003"], "an array of wires has the type of its wires (3.1)");
    assert_eq!(judged(inputs, "r: [[f64]]", "r = [row, row]"), NOTHING);
    assert_eq!(judged(inputs, "r: [[f64]]", "r = array.push(m, row)"), NOTHING);
}

#[test]
fn what_is_no_array_is_refused_where_an_array_goes() {
    assert_eq!(gives("i32", "array.len(1)"), ["LC3001"]);
    assert_eq!(gives("f64", "array.sum(1)"), ["LC3001"]);
    assert_eq!(gives("[i32]", "array.push(1, 1)"), ["LC3001"]);
    assert_eq!(gives("f64", "array.sum([[1]])"), ["LC3001"], "the bound holds for a literal too");
    assert_eq!(judged("", "a: i32, e: error", "a, e = array.get(1, 0)"), ["LC3001"]);
}

#[test]
fn an_array_goes_by_what_it_holds_and_not_by_its_first_element() {
    assert_eq!(judged("", "r: [[f64]]", "r = [[1, 2], [3, 4.5]]"), NOTHING);
    assert_eq!(judged("", "r: [[f64]]", "r = [[3, 4.5], [1, 2]]"), NOTHING);
    assert_eq!(judged("c: bool", "r: [f64]", "r = core.select(c, [1, 2], [3, 4.5])"), NOTHING);
    assert_eq!(judged("c: bool", "r: [f64]", "r = match c {\ntrue => yield [1, 2]\nfalse => yield [1, 2.5]\n}"), NOTHING);
    assert_eq!(judged("i: i32", "r: [f64]", "r = array.push([1, 2.5], i)"), NOTHING);
}

#[test]
fn an_arm_keeps_the_type_of_its_wire_and_a_literal_takes_the_type_of_the_target() {
    let arms = |a: &str, b: &str| format!("x = match c {{\ntrue => yield {a}\nfalse => yield {b}\n}}");
    assert_eq!(judged("c: bool", "x: [i32]", &arms("[1.5]", "[2.5]")), ["LC3001", "LC3001"], "each constant, as on a line of its own");
    assert_eq!(judged("c: bool", "x: [f32]", &arms("[0.5]", "[1.5]")), NOTHING);
    assert_eq!(judged("c: bool, d: f64", "x: [f32]", &arms("[d, d]", "[d, d]")), ["LC3003"], "one diagnostic, on the wire to the target");
    assert_eq!(judged("c: bool, d: f64[V]", "x: [f64[mV]]", &arms("[d, d]", "[d, d]")), ["LC3003"], "nothing converts in an array");
    assert_eq!(judged("c: bool, y: [f64]", "x: [f32]", &arms("[1.5]", "y")), ["LC3006"], "the literal took `[f32]`, and the wire keeps `[f64]`");
    assert_eq!(judged("c: bool, y: [f64]", "x: [f32]", &arms("y", "y")), ["LC3003"]);
    assert_eq!(judged("c: bool, row: [f64]", "x: [f64; 2]", &arms("[row, row]", "[row]")), ["LC3003"], "wires, as anywhere (3.1)");
    assert_eq!(judged("c: bool", "x: [f64; 2]", &arms("[[0.5, 1.5]]", "[[2.5, 3.5]]")), NOTHING, "constants take it");
}

#[test]
fn a_literal_of_rows_meets_an_array_of_two_dimensions() {
    let inputs = "c: bool, m: [f64; 2], r: [f64]";
    assert_eq!(judged(inputs, "x: [f64; 2]", "x = core.select(c, m, [[1.0]])"), NOTHING);
    assert_eq!(judged(inputs, "x: [f64; 2]", "x = core.select(c, m, [[1]])"), NOTHING);
    assert_eq!(judged(inputs, "x: [f64; 2]", "x = core.select(c, m, [r])"), ["LC3001"], "a wire keeps its type (3.1)");
    assert_eq!(judged(inputs, "x: bool", "x = core.eq(m, [[1.0]])"), NOTHING);
}

#[test]
fn an_empty_array_is_judged_where_it_goes() {
    assert_eq!(judged("", "o: f64", "o = []"), ["LC3003"]);
    assert_eq!(gives("bool", "core.not([])"), ["LC3001"]);
    assert_eq!(judged("", "x: f64", "x = loop carry a: f64 = [] {\nnext a = a\nuntil true\n}"), ["LC3007"]);
    assert_eq!(judged("", "o: [i32]", "o = [1, []]"), ["LC3001"]);
    assert_eq!(judged("", "", "o = []"), ["LC3019"], "nothing says its type");
}

#[test]
fn a_fixed_variable_is_an_input_like_another() {
    assert_eq!(judged("r: [f64[V]]", "o: [f64[V]]", "o = array.push(r, 1)"), ["LC3001"]);
    assert_eq!(judged("", "o: f64[V]", "o = core.add<f64[V]>(1, 2 V)"), ["LC3001"]);
}

#[test]
fn a_large_integer_against_a_unit_is_a_matter_of_unit() {
    assert_eq!(gives("f64[V]", "core.add(v, 3000000000)"), ["LC3002"]);
    assert_eq!(judged("", "o: f64[V]", "o = 3000000000"), ["LC3003"]);
    assert_eq!(judged("", "o: string", "o = 3000000000"), ["LC3003"]);
    assert_eq!(judged("", "o: string", "o = 18446744073709551615"), ["LC3003"]);
}

#[test]
fn a_duration_as_a_type_argument_meets_an_array_of_durations() {
    assert_eq!(judged("d: [duration]", "o: f64[s]", "o = array.sum<duration>(d)"), NOTHING);
}

#[test]
fn a_difference_of_integer_temperatures_needs_a_float() {
    assert_eq!(judged("a: i64[°F], b: i64[°F]", "o: i64[K]", "o = core.sub(a, b)"), ["LC3004"]);
}

#[test]
fn one_cause_gives_one_diagnostic() {
    assert_eq!(gives("bool", "core.eq([], [])"), ["LC3019"]);
    assert_eq!(gives("f64", "core.abs([[]])"), ["LC3019"]);
    assert_eq!(judged("s: string", "o: f64", "o = core.add<string>(s, s)"), ["LC3019"]);
    assert_eq!(judged("s: [string]", "o: f64", "o = array.sum<string>(s)"), ["LC3019"]);
    assert_eq!(judged("c: bool", "y: u8", "y = match c {\ntrue => yield (300, 2)\nfalse => yield (1, 2)\n}"), ["LC2103"]);
    assert_eq!(judged("r: fn(i32) -> (i32)", "o: i32", "o = core.call(r, 1, 2, function: r)"), ["LC2102", "LC2102"], "two mistakes, and no `LC2009`");
}

#[test]
fn an_integer_literal_keeps_its_value_in_a_float() {
    assert_eq!(gives("f32", "16777216"), NOTHING, "2^24, the last before a gap");
    assert_eq!(gives("f32", "16777217"), ["LC3018"]);
    assert_eq!(gives("f32", "-16777217"), ["LC3018"]);
    assert_eq!(gives("f32", "33554432"), NOTHING, "large, and exact");
    assert_eq!(gives("f64", "9007199254740992"), NOTHING, "2^53");
    assert_eq!(gives("f64", "9007199254740993"), ["LC3018"]);
    assert_eq!(gives("f64", "18446744073709551615"), ["LC3018"], "`u64::MAX`");
    assert_eq!(gives("f64", "0"), NOTHING);
    assert_eq!(gives("f64[V]", "9007199254740993 V"), ["LC3018"], "a literal with a unit is an `f64`");
    assert_eq!(gives("f64[V]", "9007199254740992 V"), NOTHING);
    assert_eq!(gives("f64", "9007199254740993.0"), NOTHING, "a decimal accepts the rounding");
    assert_eq!(gives("f64[V]", "99999999999999999999999 V"), ["LC1006"], "one diagnostic");
}

#[test]
fn the_target_decides_when_no_arm_does() {
    let two = "x = match c {\ntrue => yield 1\nfalse => yield 2\n}";
    assert_eq!(judged("c: bool", "x: u8", two), NOTHING);
    let through_a_name = "z = match c {\ntrue => yield 1\nfalse => yield 2\n}\nx = z";
    assert_eq!(judged("c: bool", "x: u8", through_a_name), ["LC3003"], "`z` is an `i32`");
    let empty = "x = match c {\ntrue => yield []\nfalse => yield []\n}";
    assert_eq!(judged("c: bool", "x: [f64]", empty), NOTHING);
    let decides = "x = match c {\ntrue => yield a\nfalse => yield 2\n}";
    assert_eq!(judged("c: bool, a: i32", "x: u8", decides), ["LC3003"], "an arm that decides keeps its type");
}

#[test]
fn until_and_selectors_have_their_types() {
    assert_eq!(judged("", "", "_ = loop {\nuntil 1\n}"), ["LC3014"]);
    assert_eq!(judged("x: f64", "y: i32", "y = match x {\n_ => yield 1\n}"), ["LC3013"], "and its arms say nothing more");
    assert_eq!(judged("x: f64", "y: i32", "y = handle x {\n_ => yield 1\n}"), ["LC3013"]);
    assert_eq!(judged("e: error", "y: i32", "y = match e {\n_ => yield 1\n}"), ["LC3013"]);
    assert_eq!(judged("n: i32[V]", "y: i32", "y = match n {\n_ => yield 1\n}"), ["LC3013"], "an integer without a unit");
    assert_eq!(judged("e: error", "y: error", "y = handle e {\nok => yield none\nio.timeout(a) => yield a\n_(b) => yield b\n}"), NOTHING);
}

#[test]
fn a_pattern_has_the_type_of_its_selector() {
    let r = "r: Reading";
    assert_eq!(judged(r, "y: f64[V]", "y = match r {\nReading.Value(v) => yield v\n_ => yield 0 V\n}"), NOTHING);
    assert_eq!(judged(r, "y: i32", "y = match r {\nReading.Pair(a, b) => yield a\n_ => yield 0\n}"), NOTHING);
    assert_eq!(judged(r, "y: string", "y = match r {\nReading.Pair(a, b) => yield a\n_ => yield \"\"\n}"), ["LC3006"], "`a` is an `i32`");
    assert_eq!(judged(r, "y: i32", "y = match r {\nReading.Value(v, w) => yield 1\n_ => yield 0\n}"), ["LC3009"]);
    assert_eq!(judged(r, "y: i32", "y = match r {\nVerdict.Pass => yield 1\n_ => yield 0\n}"), ["LC3009"]);
    assert_eq!(judged("v: Verdict", "y: i32", "y = match v {\nGrade.Pass => yield 1\n_ => yield 0\n}"), ["LC3009"], "two types of one shape");
    assert_eq!(judged("n: i32", "y: i32", "y = match n {\n\"a\" => yield 1\n_ => yield 0\n}"), ["LC3009"]);
    assert_eq!(judged("n: i32", "y: i32", "y = match n {\n1.5 => yield 1\n_ => yield 0\n}"), ["LC3009"]);
    assert_eq!(judged("t: string", "y: i32", "y = match t {\n1..=5 => yield 1\n_ => yield 0\n}"), ["LC3009"]);
    assert_eq!(judged("n: i8", "y: i32", "y = match n {\n300 => yield 1\n_ => yield 0\n}"), ["LC3018"]);
    assert_eq!(judged("n: i8", "y: i32", "y = match n {\n1..=300 => yield 1\n_ => yield 0\n}"), ["LC3018"]);
    assert_eq!(judged("t: string", "y: i32", "y = match t {\n\"a\" => yield 1\n_ => yield 0\n}"), NOTHING);
}

#[test]
fn a_duration_converts_to_and_from_seconds() {
    assert_eq!(gives("error", "time.wait(100 ms)"), NOTHING);
    assert_eq!(gives("error", "time.wait(d)"), NOTHING);
    assert_eq!(gives("error", "time.wait(1)"), ["LC3001"]);
    assert_eq!(gives("error", "time.wait(v)"), ["LC3001"]);
    assert_eq!(gives("error", "time.wait(m)"), ["LC3001"]);
    assert_eq!(gives("f64[s]", "d"), NOTHING);
    assert_eq!(gives("f64[ms]", "d"), NOTHING);
    assert_eq!(gives("duration", "1 s"), NOTHING);
    assert_eq!(gives("bool", "core.gt(d, d)"), NOTHING);
    assert_eq!(gives("bool", "core.gt(d, 1 s)"), NOTHING);
    assert_eq!(gives("f64[s]", "core.add(d, d)"), NOTHING);
    assert_eq!(gives("duration", "core.select(ok, d, 1 s)"), NOTHING);
    assert_eq!(gives("duration", "time.elapsed(stamp)"), NOTHING);
    assert_eq!(gives("duration", "time.elapsed(d)"), ["LC3001"]);
}

#[test]
fn arrays_have_one_element_type_and_no_conversion() {
    assert_eq!(gives("[f64]", "[1, 2.5]"), NOTHING);
    assert_eq!(gives("[f64[V]]", "[1 V, 200 mV]"), NOTHING);
    assert_eq!(gives("[f64[V]]", "[1 V, 2 A]"), ["LC3001"]);
    assert_eq!(gives("[i32]", "[1, 2]"), NOTHING);
    assert_eq!(gives("[f64]", "[1, 2]"), NOTHING, "a literal takes the type expected");
    assert_eq!(gives("[string]", "[1]"), ["LC3001"]);
    assert_eq!(gives("[string]", "[t]"), NOTHING);
    assert_eq!(gives("[f64[mV]]", "[v]"), ["LC3003"], "an array that holds a wire has the type of its wires");
    assert_eq!(gives("[f32]", "xs"), ["LC3003"], "a wire of an array has no conversion");
    assert_eq!(gives("f64", "array.get(xs, 0)"), ["LC2103"]);
    assert_eq!(gives("i32", "array.len(xs)"), NOTHING);
    assert_eq!(gives("[f64]", "array.push(xs, 1)"), NOTHING);
    assert_eq!(gives("[f64]", "array.push(xs, n)"), NOTHING, "widened to what the array decides");
    assert_eq!(gives("[f64]", "array.push(xs, t)"), ["LC3001"]);
    assert_eq!(gives("[f64]", "array.push([], x)"), NOTHING, "`[]` takes the type of the item");
    assert_eq!(gives("i32", "array.len([])"), ["LC3019"]);
    assert_eq!(gives("i32", "array.len(x)"), ["LC3001"]);
    assert_eq!(gives("f64", "array.sum(xs)"), NOTHING);
    assert_eq!(gives("f64", "array.sum(ss)"), ["LC3001"]);
    assert_eq!(gives("string", "string.concat(t, \"a\", t)"), NOTHING);
    assert_eq!(gives("string", "string.concat(t, 1)"), ["LC3001"]);
    assert_eq!(gives("string", "string.join(ss)"), NOTHING);
    assert_eq!(gives("string", "string.join(xs)"), ["LC3001"]);
}

#[test]
fn a_type_argument_fixes_a_variable() {
    assert_eq!(gives("f64", "core.add<f64>(n, 1)"), NOTHING);
    assert_eq!(gives("i32", "core.add<i32>(n, 0.5)"), ["LC3004"]);
    assert_eq!(gives("i32", "array.len<string>([])"), NOTHING);
    assert_eq!(gives("i32", "array.len<string>(xs)"), ["LC3001"]);
}

#[test]
fn a_reference_matches_exactly() {
    let extra = [(
        "apply.lcf",
        "use core\n\nfn apply(h: fn(f64[V], err: error) -> (f64[V], error), x: f64[V], err: error = none) -> (y: f64[V], err: error) {\n    y, err = core.call(h, x, err: err)\n}",
    )];
    let run = |body: &str| judged_with(&extra, "v: f64[V]", "y: f64[V], err: error", body);
    assert_eq!(run("y, err = apply(measure, v)"), NOTHING, "the name of an argument does not count");
    assert_eq!(run("y, err = apply(scale, v)"), ["LC3008"], "a default value does not shorten a signature");
    assert_eq!(run("y, err = apply(v, v)"), ["LC3001"]);
    let exact = |ty: &str, name: &str| judged(&format!("h: {ty}"), "", &format!("_ = for i in 0..1 {{\na = core.eq(h, {name})\n}}"));
    assert_eq!(exact("fn(f64[V], f64) -> (f64[V])", "scale"), NOTHING);
    assert_eq!(exact("fn(f64[mV], f64) -> (f64[V])", "scale"), ["LC3008"], "no conversion on a reference");
    let variadic = [("v.lcf", "fn many(parts: string...) -> (n: i32) {\n    n = 1\n}")];
    assert_eq!(judged_with(&variadic, "h: fn([string]) -> (i32)", "", "a = core.eq(h, many)"), NOTHING, "a variadic parameter counts as an array");
}

#[test]
fn core_call_wires_what_the_type_says() {
    let h = "h: fn(f64[V], gain: f64) -> (f64[V], error), v: f64[V]";
    let run = |body: &str| judged(h, "y: f64[V], err: error", body);
    assert_eq!(run("y, err = core.call(h, v, 2.0)"), NOTHING);
    assert_eq!(run("y, err = core.call(h, v, gain: 2.0)"), NOTHING);
    assert_eq!(run("y, err = core.call(h, 1 V, gain: 2)"), NOTHING);
    assert_eq!(run("y, err = core.call(h, v)"), ["LC2101"], "no default value in a type");
    assert_eq!(run("y, err = core.call(h, v, 2.0, 3.0)"), ["LC2102"]);
    assert_eq!(run("y, err = core.call(h, v, 2.0, gain: 3.0)"), ["LC2102"]);
    assert_eq!(run("y, err = core.call(h, v, nowhere: 2.0)"), ["LC2009"]);
    assert_eq!(run("y, err = core.call(h, 1 A, gain: 2.0)"), ["LC3001"]);
    assert_eq!(run("y = core.call(h, v, 2.0)"), ["LC2103"]);
    assert_eq!(run("y, err = core.call(v, v, 2.0)"), ["LC3001"]);
    assert_eq!(run("y, err = core.call(nowhere, v, 2.0)"), ["LC2001"]);
}

#[test]
fn a_value_says_its_fields() {
    let p = "p: Point, n: i32, e: error";
    assert_eq!(judged(p, "x: f64[m]", "x = p.x"), NOTHING);
    assert_eq!(judged(p, "x: string", "x = p.x"), ["LC3003"]);
    assert_eq!(judged(p, "x: f64[m]", "x = p.nowhere"), ["LC2006"]);
    assert_eq!(judged(p, "x: f64[m]", "x = p.x.y"), ["LC2006"]);
    assert_eq!(judged(p, "x: i32", "x = n.y"), ["LC2006"]);
    assert_eq!(judged(p, "x: i32", "x = e.code"), ["LC2006"], "an error is read through `handle`");
    let alias = [("a.lct", "type Q = Point")];
    assert_eq!(judged_with(&alias, "q: Q", "x: f64[m]", "x = q.x"), NOTHING);
}

#[test]
fn a_line_is_typed_wherever_it_is_written() {
    assert_eq!(judged("v: f64[V]", "x: f64[V]", "x = b\nb = a\na = v"), NOTHING);
    assert_eq!(judged("v: f64[V]", "x: f64[A]", "x = b\nb = a\na = v"), ["LC3003"]);
    assert_eq!(judged("v: f64[V]", "x: f64[V]", "x = core.add(a, b)\nb = core.mul(a, 1.0)\na = core.neg(v)"), NOTHING);
}

#[test]
fn a_body_reads_what_is_around_it_in_any_order() {
    let body = "y = match c {\ntrue => {\nz = core.add(after, 1 V)\nyield z\n}\nfalse => yield 0 V\n}\nafter = core.neg(v)";
    assert_eq!(judged("c: bool, v: f64[V]", "y: f64[V]", body), NOTHING);
}

#[test]
fn a_cycle_stays_unknown_and_says_nothing_here() {
    assert_eq!(judged("", "x: f64[V]", "a = core.add(b, 1)\nb = core.add(a, 1)\nx = a"), NOTHING, "pass 5 says why");
    assert_eq!(judged("", "x: f64[V]", "x = core.add(a, 1)\na = core.neg(a)"), NOTHING);
}

#[test]
fn one_error_gives_one_diagnostic() {
    let body = "a = core.add(v, i)\nb = core.mul(a, 2)\nc = core.lt(b, a)\nd = core.and(c, a)\nx = d";
    assert_eq!(judged("v: f64[V], i: f64[A]", "x: bool", body), ["LC3002"]);
    assert_eq!(judged("v: f64[V]", "x: f64[A]", "a = nowhere\nb = core.add(a, v)\nx = b"), ["LC2001"]);
    assert_eq!(judged("v: f64[V]", "x: i32", "a = g(v)\nx = a"), ["LC2002"]);
    assert_eq!(judged("v: nowhere", "x: i32", "x = core.add(v, 1)"), ["LC2003"]);
}

#[test]
fn an_output_read_back_has_the_type_of_its_signature() {
    assert_eq!(judged("v: f64[mV]", "x: f64[V], y: f64[V]", "x = v\ny = x"), NOTHING);
    assert_eq!(judged("v: f64[mV]", "x: f64[V], y: f64[mV]", "y = x\nx = v"), NOTHING);
    assert_eq!(judged("v: i32", "x: f64, y: f64", "x = v\nz = core.select(true, x, 0.5)\ny = z"), NOTHING);
}

#[test]
fn a_variadic_parameter_is_an_array_in_its_body() {
    let text = "use array\n\nfn f(values: f64...) -> (n: i32, wrong: f64) {\n    n = array.len(values)\n    wrong = values\n}";
    assert_eq!(codes(&[("f.lcf", text)]), ["LC3003"]);
}

#[test]
fn a_call_checks_its_arguments_and_gives_its_outputs() {
    assert_eq!(judged("v: f64[V]", "y: f64[V]", "y = scale(v, 2)"), NOTHING);
    assert_eq!(judged("v: f64[A]", "y: f64[V]", "y = scale(v)"), ["LC3001"]);
    assert_eq!(judged("v: f64", "y: f64[V]", "y = scale(v)"), ["LC3001"]);
    assert_eq!(judged("v: f64[V]", "y: f64[V]", "y = scale(v, gain: \"a\")"), ["LC3001"]);
    assert_eq!(judged("v: f64[V]", "y: f64[A]", "y = scale(v)"), ["LC3003"]);
    assert_eq!(judged("v: f64[V]", "y: f64[V], z: f64[A]", "y, z = measure(v)"), ["LC3003"]);
    assert_eq!(judged("v: f64[V]", "", "#beep: scale(1)"), ["LC3001"], "a call alone too");
}

#[test]
fn a_wire_says_where_it_comes_from_and_where_it_goes() {
    let wire = |inputs: &str, outputs: &str, body: &str| {
        let text = program(inputs, outputs, body);
        let found = diagnostics(&[("f.lcf", &text), ("scale.lcf", "fn scale(x: f64[V]) -> (y: f64[V]) {\n    y = x\n}")]);
        let [diagnostic] = found.as_slice() else { panic!("{found:?}") };
        let first = diagnostic.wires.last().unwrap();
        (first.value.clone(), first.from.clone(), first.to.clone())
    };
    let some = |value: &str, from: &str, to: &str| (Some(value.to_owned()), Some(from.to_owned()), Some(to.to_owned()));
    assert_eq!(wire("a: f64", "y: f64[V]", "    y = scale(a)"), some("a", "fn.a", "y.x"));
    assert_eq!(wire("a: f64", "y: f64[V]", "    b = a\n    y = scale(b)"), some("b", "fn.a", "y.x"), "a name makes no node");
    assert_eq!(wire("a: f64", "", "    #go: scale(a)"), some("a", "fn.a", "#go.x"));
    assert_eq!(wire("a: f64", "", "    scale(a)"), some("a", "fn.a", "@10.x"));
    assert_eq!(
        wire("a: f64", "y: f64", "    y = loop carry c = a {\n    next c = c\n    b = scale(c)\n    until true\n    }"),
        some("c", "next.c", "b.x")
    );
    assert_eq!(wire("v: f64[V]", "y: f64[A]", "    b = scale(v)\n    y = b"), some("b", "b.y", "fn.y"));
}

#[test]
fn what_passes_reads_has_its_types_and_its_coercions() {
    let text = "use core\n\nfn f(a: f64[mV], n: i32) -> (y: f64[V], z: f64) {\n    y = a\n    z = core.add(n, 0.5)\n}";
    let checked = run(&[("f.lcf", text)]);
    assert!(checked.report.diagnostics().is_empty(), "{:?}", checked.report.diagnostics());
    let file = checked.files.first().unwrap();
    let coercions = file.resolved.coercions();
    assert_eq!(coercions.len(), 2, "{coercions:?}");
    let show = |ty| checked.types.get(ty).clone();
    let volts = |ty: Ty| matches!(ty, Ty::Number { kind: NumberKind::F64, unit } if unit == Unit::from_name("V").unwrap());
    assert!(coercions.iter().any(|coercion| volts(show(coercion.to))), "mV to V");
    assert!(coercions.iter().any(|coercion| matches!(show(coercion.from), Ty::Number { kind: NumberKind::I32, .. })), "i32 to f64");
    let labcraft_graph::Decl::Function(function) = &file.file.decl else { panic!() };
    let input = function.signature.params.first().unwrap().id;
    assert!(matches!(show(file.resolved.ty(input)), Ty::Number { kind: NumberKind::F64, .. }));
}

#[test]
fn a_function_with_a_faulty_signature_is_not_typed() {
    assert_eq!(codes(&[("f.lcf", "fn f(a: i32, a: i32) -> (x: string) {\n    x = a\n}")]), ["LC2019"]);
    assert_eq!(codes(&[("f.lcf", "fn f<T>(a: T) -> (x: string) {\n    x = a\n}")]), ["LC4006"]);
}

#[test]
fn another_scale_without_a_dimension_is_no_type_error_and_says_what_it_gives() {
    let ratio = "r: f64[mV/V], a: f64";
    assert_eq!(judged(ratio, "x: f64[mV/V]", "x = core.add(r, a)"), NOTHING);
    assert_eq!(judged(ratio, "x: string", "x = core.add(r, a)"), ["LC3003"], "what it gives is known");
    assert_eq!(judged(ratio, "x: string", "x = core.max(r, a)"), ["LC3003"]);
    assert_eq!(judged(ratio, "x: string", "x = core.select(true, r, a)"), ["LC3003"]);
}

#[test]
fn a_number_without_a_unit_does_not_become_a_scale() {
    for ty in ["f64[mV/V]", "f32[mV/V]", "i32[mV/V]"] {
        let output = format!("o: {ty}");
        assert_eq!(judged("", &output, "o = 5"), ["LC3003"], "{ty}");
    }
    assert_eq!(judged("", "o: f64[mV/V]", "o = 1.5"), ["LC3003"], "a decimal too");
    assert_eq!(judged("", "o: f64[mV/V]", "o = 5 mV/V"), NOTHING);
    assert_eq!(judged("a: f64", "o: f64[mV/V]", "o = a"), NOTHING, "a wire says its type where it is declared");

    let r = "r: f64[mV/V]";
    assert_eq!(judged(r, "o: f64[mV/V]", "o = core.add(r, 5)"), ["LC3001"]);
    assert_eq!(judged(r, "o: f64[mV/V]", "o = core.add(r, 1.5)"), ["LC3001"]);
    assert_eq!(judged(r, "o: f64[mV/V]", "o = core.add(r, 5 mV/V)"), NOTHING);
    assert_eq!(judged("r: f32[mV/V]", "o: f32[mV/V]", "o = core.add(r, 5)"), ["LC3001"]);
    assert_eq!(judged("r: i32[mV/V]", "o: bool", "o = core.lt(r, 5)"), ["LC3001"]);
    assert_eq!(text_of_found(r, "o: bool", "o = core.add(1.5, r)"), "f64", "the unit of the first: the number stays what it is");

    let looped = |carry: &str, next: &str| format!("x = loop carry a: f64[mV/V] = {carry} {{\nnext a = {next}\nuntil true\n}}");
    assert_eq!(judged("", "x: f64[mV/V]", &looped("5", "a")), ["LC3007"]);
    assert_eq!(judged("", "x: f64[mV/V]", &looped("5 mV/V", "5")), ["LC3007"]);

    let arms = |a: &str, b: &str| format!("x = match c {{\ntrue => yield {a}\nfalse => yield {b}\n}}");
    assert_eq!(judged("c: bool", "x: f64[mV/V]", &arms("5", "6")), ["LC3003"]);
    assert_eq!(judged("c: bool", "x: f64[mV/V]", &arms("1.5", "2.5")), ["LC3003"]);
    assert_eq!(judged("c: bool, r: f64[mV/V]", "x: f64[mV/V]", &arms("r", "5")), ["LC3006"]);
    assert_eq!(judged("c: bool", "x: f64[mV/V]", &arms("5 mV/V", "6 mV/V")), NOTHING);

    let takes = [("ratio.lcf", "fn ratio(a: [f64[mV/V]]) -> (r: i32) {\n    r = 1\n}")];
    assert_eq!(judged_with(&takes, "", "x: i32", "x = ratio([1, 2])"), ["LC3001", "LC3001"]);
    assert_eq!(judged_with(&takes, "", "x: i32", "x = ratio([1])"), ["LC3001"]);
    assert_eq!(judged_with(&takes, "", "x: i32", "x = ratio([1 mV/V])"), NOTHING);
}

#[test]
fn a_pattern_waits_for_the_lines_it_reads() {
    let (before, after) = ("z = core.add(n, 1)\n", "\nz = core.add(n, 1)");
    for arms in ["[z] => yield 1\n_ => yield 0", "1, [z], _ => yield 1"] {
        let structure = format!("y = match n {{\n{arms}\n}}");
        assert_eq!(judged("n: i32", "y: i32", &format!("{before}{structure}")), ["LC3009"], "{arms}");
        assert_eq!(judged("n: i32", "y: i32", &format!("{structure}{after}")), ["LC3009"], "{arms}: the order of the lines means nothing");
    }
}

#[test]
fn every_pattern_of_a_list_has_the_type_of_its_selector() {
    assert_eq!(judged("n: i32", "y: i32", "y = match n {\n1, 3..=5, _ => yield 1\n}"), NOTHING);
    assert_eq!(judged("n: i32", "y: i32", "y = match n {\n1, \"a\", _ => yield 1\n}"), ["LC3009"]);
    assert_eq!(judged("n: i32", "y: i32", "y = match n {\n\"a\", 1.5, _ => yield 1\n}"), ["LC3009", "LC3009"]);
    assert_eq!(judged("t: string", "y: i32", "y = match t {\n\"a\", 1..=5 => yield 1\n_ => yield 0\n}"), ["LC3009"]);
    assert_eq!(judged("v: Verdict", "y: i32", "y = match v {\nVerdict.Pass, Grade.Pass => yield 1\n_ => yield 0\n}"), ["LC3009"]);
    // A variant that holds data binds it, so it has no place in a list.
    assert_eq!(judged("r: Reading", "y: i32", "y = match r {\nReading.Value, _ => yield 1\n}"), ["LC3009"]);
}

#[test]
fn one_mistake_in_a_power_or_a_pattern_gives_one_diagnostic() {
    assert_eq!(gives("f64", "core.pow(v, 0.5)"), ["LC3004"]);
    assert_eq!(gives("f64", "core.pow(v, 3000000000)"), ["LC3018"]);
    assert_eq!(gives("f64", "core.pow(v, t)"), ["LC3001"]);
    assert_eq!(judged("r: Reading", "y: i32", "y = match r {\nVerdict.Pass(a) => yield 1\n_ => yield 0\n}"), ["LC3009"]);
}

#[test]
fn an_array_of_one_wire_is_judged_as_an_array_of_two() {
    let takes = [("takes.lcf", "fn takes(a: [f64]) -> (r: i32) {\n    r = 1\n}")];
    assert_eq!(judged_with(&takes, "q: f64", "x: i32", "x = takes([q, q])"), NOTHING);
    assert_eq!(judged_with(&takes, "q: f64", "x: i32", "x = takes([q])"), NOTHING);
    assert_eq!(judged_with(&takes, "q: i32", "x: i32", "x = takes([q, q])"), ["LC3001"], "nothing converts in an array");
    assert_eq!(judged_with(&takes, "q: i32", "x: i32", "x = takes([q])"), ["LC3001"]);
    assert_eq!(judged_with(&takes, "q: string", "x: i32", "x = takes([q])"), ["LC3001"]);
    assert_eq!(gives("[f64]", "array.push([1], x)"), NOTHING, "as `[1, 2]`");
}

#[test]
fn an_empty_array_waits_for_what_decides() {
    let two = |first: &str, second: &str| format!("x = match c {{\ntrue => yield {first}\nfalse => yield {second}\n}}");
    assert_eq!(judged("c: bool", "", &two("[]", "[1, 2]")), NOTHING);
    assert_eq!(judged("c: bool", "", &two("[1, 2]", "[]")), NOTHING);
    assert_eq!(gives("bool", "core.eq([], [1, 2])"), NOTHING);
    assert_eq!(gives("[i32]", "array.push([], 1)"), NOTHING);
}

#[test]
fn a_conversion_is_recorded_once() {
    let coercions = |line: &str| {
        let text = format!("use array\nuse core\n\nfn f(n: i32, xs: [f64], d: duration) -> () {{\n    r = {line}\n}}");
        let checked = run(&[("f.lcf", text.as_str())]);
        assert!(checked.report.diagnostics().is_empty(), "{:?}", checked.report.diagnostics());
        checked.files.first().unwrap().resolved.coercions().len()
    };
    assert_eq!(coercions("core.add<f64>(n, n)"), 2);
    assert_eq!(coercions("array.push(xs, n)"), 1);
    assert_eq!(coercions("core.add(n, 0.5)"), 1);
    assert_eq!(coercions("core.add(0.5, n)"), 1);
    assert_eq!(coercions("core.add(n, n)"), 0);
    assert_eq!(coercions("core.add(d, 1 s)"), 1, "a `duration` to the `f64[s]` it counts as");
}

#[test]
fn a_type_argument_keeps_to_its_bound() {
    assert_eq!(gives("bool", "core.lt<c128>(1, 2)"), ["LC3019"]);
    assert_eq!(gives("f64", "array.sum<string>([])"), ["LC3019"]);
    assert_eq!(gives("f64", "core.add<string>(x, x)"), ["LC3019"]);
    assert_eq!(gives("f64", "array.sum<f64>([])"), NOTHING);
}

#[test]
fn a_named_argument_keeps_its_place_in_the_signature() {
    assert_eq!(gives("f64[mV]", "core.add(b: v, a: mv)"), NOTHING, "the unit of `a`, as written or not");
    assert_eq!(judged("v: f64[V], mv: f64[mV]", "x: bool, y: f64[A]", "x = core.lt(b: v, a: mv)\ny = core.add(b: v, a: mv)"), ["LC3003"]);
}

#[test]
fn a_temperature_difference_on_integers_goes_through_a_float() {
    let inputs = "a: i32[°F], b: i32[°F]";
    assert_eq!(judged(inputs, "x: f64[K]", "x = core.sub(a, b)"), NOTHING);
    assert_eq!(judged(inputs, "x: i32[K]", "x = core.sub(a, b)"), ["LC3003"]);
}

#[test]
fn a_fix_with_a_unit_gives_what_is_expected() {
    let only_fix = |inputs: &str, outputs: &str, body: &str| {
        let text = program(inputs, outputs, body);
        let found = diagnostics(&[("f.lcf", &text)]);
        let [diagnostic] = found.as_slice() else { panic!("{found:?}") };
        diagnostic.fixes.iter().map(|fix| fix.title.clone()).collect::<Vec<_>>()
    };
    assert!(only_fix("", "x: i32[mV]", "    x = 0").first().unwrap().contains("0 mV"), "`0 mV` takes the type expected");
    assert!(only_fix("", "x: f64[mV]", "    x = 0").first().unwrap().contains("0 mV"));
    assert!(only_fix("", "x: f32[V]", "    x = 1.5").first().unwrap().contains("1.5 V"));
    assert_eq!(only_fix("", "x: i32[mV]", "    x = 0.5"), Vec::<String>::new(), "a decimal does not become an integer");
    assert_eq!(only_fix("y: f64", "x: i32[mV]", "    x = y"), Vec::<String>::new(), "`core.with_unit` gives an `f64`");
    assert_eq!(only_fix("y: f64[V]", "x: i32", "    x = y"), Vec::<String>::new(), "`core.unitless` gives an `f64`");
}

#[test]
fn fixes_on_many_lines_take_new_names_in_order() {
    let text = "use core\n\nfn f(x: f64, x2: i32) -> () {\n    a = core.to_i32(1)\n    b = g(x)\n    c = g(x)\n}";
    let found = diagnostics(&[("f.lcf", text), ("g.lcf", "fn g(a: i32) -> (r: i32) {\n    r = a\n}")]);
    let names: Vec<&str> = found.iter().filter_map(|d| d.fixes.first()).filter_map(|fix| fix.edits.get(1)).map(|edit| edit.text.as_str()).collect();
    assert_eq!(names, ["x3", "x4"]);
}

#[test]
fn a_literal_array_keeps_to_the_bound_and_to_what_is_known() {
    assert_eq!(gives("[i32]", "core.abs([1, 2])"), ["LC3001"]);
    assert_eq!(gives("[i32]", "core.add([1], [2])"), ["LC3001"], "what failed says nothing more of the same variable");
    assert_eq!(gives("bool", "core.lt([1], [2])"), ["LC3001"]);
    assert_eq!(gives("f64", "core.sqrt([1, 2])"), ["LC3001"]);
    assert_eq!(gives("[string]", "array.push([1], t)"), ["LC3001"]);
    assert_eq!(gives("i32", "array.len<string>([1])"), ["LC3001"]);
    assert_eq!(gives("i32", "array.len<string>([1, 2])"), ["LC3001", "LC3001"], "one for each element");
    assert_eq!(gives("i32", "array.len<f64>([1])"), NOTHING);
}

#[test]
fn a_difference_of_temperatures_is_one_rule_with_a_type_argument_too() {
    let inputs = "a: f64[°C], b: f64[°C]";
    assert_eq!(judged(inputs, "x: f64[K]", "x = core.sub<f64[°C]>(a, b)\ny = core.neg(x)"), NOTHING);
    assert_eq!(judged(inputs, "x: bool", "x = core.sub<f64[°C]>(a, b)"), ["LC3003"]);
    let kept = text_of_found(inputs, "x: bool", "x = core.sub<f64[°C]>(a, b)");
    assert_eq!(kept, "f64[K]", "a scale of 283.15 K would read a difference as an absolute");
}

fn text_of_found(inputs: &str, outputs: &str, body: &str) -> String {
    let text = program(inputs, outputs, body);
    let found = diagnostics(&[("f.lcf", &text)]);
    let [diagnostic] = found.as_slice() else { panic!("{found:?}") };
    diagnostic.found.clone().unwrap()
}

#[test]
fn a_duration_counts_as_seconds_wherever_it_comes_from() {
    let d = "a: duration, ds: [duration]";
    assert_eq!(text_of_found(d, "x: bool", "x = core.mul<f64[s], f64>(a, 2.0)"), "f64[s]");
    assert_eq!(text_of_found(d, "x: bool", "x = core.mul<duration, f64>(a, 2.0)"), "f64[s]");
    assert_eq!(text_of_found(d, "x: bool", "x = core.add<duration>(a, a)"), "f64[s]");
    assert_eq!(text_of_found(d, "x: bool", "x = array.sum(ds)"), "f64[s]");
    assert_eq!(judged(d, "x: f64[V]", "x = core.sqrt<duration>(a)"), ["LC3016"], "an odd exponent, and not silence");
}

#[test]
fn rows_take_the_type_expected_and_a_wire_keeps_its_own() {
    let two = [("two.lcf", "fn two(a: [f64; 2]) -> (r: i32) {\n    r = 1\n}")];
    assert_eq!(judged_with(&two, "row: [f64]", "x: i32", "x = two([row, row])"), ["LC3001"]);
    assert_eq!(judged_with(&two, "row: [f64]", "x: i32", "x = two([row])"), ["LC3001"]);
    assert_eq!(judged_with(&two, "", "x: i32", "x = two([[1.5, 2.5], [3.5, 4.5]])"), NOTHING);
    let ratio = [("ratio.lcf", "fn ratio(a: [f64[mV/V]]) -> (r: i32) {\n    r = 1\n}")];
    assert_eq!(judged_with(&ratio, "a: f64", "x: i32", "x = ratio([a, a])"), ["LC3001"]);
    assert_eq!(judged_with(&ratio, "a: f64", "x: i32", "x = ratio([a])"), ["LC3001"]);
}

#[test]
fn what_is_not_wired_gives_one_diagnostic() {
    assert_eq!(gives("f64", "core.pow(x)"), ["LC2101"]);
    assert_eq!(judged("h: fn(i32) -> (i32)", "x: i32", "x = core.call(function: h, 1)"), ["LC2104"]);
}

#[test]
fn a_long_chain_of_lines_takes_no_stack() {
    use std::fmt::Write;
    let mut body = String::from("    a0 = v\n");
    for index in 1..100_000 {
        let _ = writeln!(body, "    a{index} = a{}", index - 1);
    }
    let text = format!("fn f(v: f64[V]) -> (x: f64[A]) {{\n{body}    x = a99999\n}}");
    assert_eq!(codes(&[("f.lcf", &text)]), ["LC3003"]);
}

#[test]
fn a_type_nested_line_by_line_has_a_limit() {
    use std::fmt::Write;
    let mut body = String::from("    a0 = 1\n");
    for index in 1..2_000 {
        let _ = writeln!(body, "    a{index} = [a{}]", index - 1);
    }
    let text = format!("fn f() -> () {{\n{body}}}");
    let found = codes(&[("f.lcf", &text)]);
    assert_eq!(found, ["LC1006"], "and what follows says nothing more");
}

fn matching(selector: &str, arms: &str) -> Vec<&'static str> {
    judged(&format!("s: {selector}"), "y: i32", &format!("y = match s {{\n{arms}\n}}"))
}

fn handling(arms: &str) -> Vec<&'static str> {
    judged("e: error", "y: i32", &format!("y = handle e {{\n{arms}\n}}"))
}

#[test]
fn every_value_has_an_arm() {
    assert_eq!(matching("bool", "true => yield 1\nfalse => yield 2"), NOTHING);
    assert_eq!(matching("bool", "true, false => yield 1"), NOTHING);
    assert_eq!(matching("bool", "true => yield 1\n_ => yield 2"), NOTHING);
    assert_eq!(matching("bool", "true => yield 1"), ["LC3010"]);
    assert_eq!(matching("bool", "false => yield 1"), ["LC3010"]);
    assert_eq!(matching("bool", ""), ["LC3010"]);
    assert_eq!(matching("Verdict", "Verdict.Pass => yield 1\nVerdict.Fail => yield 2"), NOTHING);
    assert_eq!(matching("Verdict", "Verdict.Pass, Verdict.Fail => yield 1"), NOTHING);
    assert_eq!(matching("Verdict", "Verdict.Pass => yield 1\n_ => yield 2"), NOTHING);
    assert_eq!(matching("Verdict", "Verdict.Pass => yield 1"), ["LC3010"]);
    assert_eq!(matching("Reading", "Reading.Value(v) => yield 1\nReading.Overload => yield 2"), ["LC3010"]);
    assert_eq!(matching("i32", "1 => yield 1\n_ => yield 2"), NOTHING);
    assert_eq!(matching("i32", "1, _ => yield 1"), NOTHING, "wherever it is written");
    assert_eq!(matching("i32", "1 => yield 1"), ["LC3010"]);
    assert_eq!(matching("u8", "0..=255 => yield 1"), ["LC3010"], "even when ranges cover the whole type");
    assert_eq!(matching("u8", "0..=255 => yield 1\n_ => yield 2"), NOTHING, "`_` is required here, so it is not useless");
    assert_eq!(matching("string", "\"a\" => yield 1\n_ => yield 2"), NOTHING);
    assert_eq!(matching("string", "\"a\" => yield 1"), ["LC3010"]);
    assert_eq!(matching("string", ""), ["LC3010"]);
}

#[test]
fn the_message_names_a_few_missing_variants() {
    let many = ("many.lct", "type Many = enum { A, B, C, D, E, F, G, H }");
    let text = program("s: Many", "y: i32", "y = match s {\nMany.A => yield 1\n}");
    let found = diagnostics(&[("f.lcf", text.as_str()), many]);
    assert_eq!(found.len(), 1);
    let message = found.first().map_or("", |diagnostic| diagnostic.message.as_str());
    assert!(message.contains("`B`, `C`, `D`, `E`, `F` and 2 more"), "{message}");
}

#[test]
fn an_alias_covers_what_its_type_covers() {
    let extra = [
        ("channel.lct", "type Channel = i32"),
        ("flag.lct", "type Flag = bool"),
        ("other.lct", "type Other = Verdict"),
        ("t.lct", "type T = string"),
    ];
    let on = |selector: &str, arms: &str| judged_with(&extra, &format!("s: {selector}"), "y: i32", &format!("y = match s {{\n{arms}\n}}"));
    assert_eq!(on("Channel", "1..=3 => yield 1\n_ => yield 2"), NOTHING);
    assert_eq!(on("Channel", "1..=3 => yield 1"), ["LC3010"]);
    assert_eq!(on("Channel", "1..=3, 2 => yield 1\n_ => yield 2"), ["LC3011"]);
    assert_eq!(on("Channel", "3..=1 => yield 1\n_ => yield 2"), ["LC3009"]);
    assert_eq!(on("Channel", "1 => yield 1\n1 => yield 2\n_ => yield 3"), ["LC3011"]);
    assert_eq!(on("Flag", "true => yield 1"), ["LC3010"]);
    assert_eq!(on("T", "\"a\" => yield 1"), ["LC3010"]);
    assert_eq!(on("Other", "Other.Pass => yield 1"), ["LC3010"]);
    assert_eq!(on("Other", "Other.Pass => yield 1\nVerdict.Fail => yield 2"), NOTHING);
    assert_eq!(on("Verdict", "Other.Pass => yield 1\nVerdict.Pass => yield 2\nVerdict.Fail => yield 3"), ["LC3022"], "one variant, by two names");
}

#[test]
fn a_range_holds_a_value() {
    assert_eq!(matching("i32", "5..=1 => yield 1\n_ => yield 2"), ["LC3009"]);
    assert_eq!(matching("i32", "0..0 => yield 1\n_ => yield 2"), ["LC3009"]);
    assert_eq!(matching("i32", "5..5 => yield 1\n_ => yield 2"), ["LC3009"]);
    assert_eq!(matching("i32", "5..=5 => yield 1\n_ => yield 2"), NOTHING);
    assert_eq!(matching("i32", "5..6 => yield 1\n_ => yield 2"), NOTHING);
    assert_eq!(matching("i32", "-5..-4 => yield 1\n_ => yield 2"), NOTHING);
    assert_eq!(matching("u8", "300..=200 => yield 1\n_ => yield 2"), ["LC3018"], "and not empty as well");
    assert_eq!(matching("u8", "400..=300 => yield 1\n_ => yield 2"), ["LC3018", "LC3018"]);
}

#[test]
fn every_number_of_a_pattern_fits_its_selector() {
    assert_eq!(matching("u8", "0..=255 => yield 1\n_ => yield 2"), NOTHING);
    assert_eq!(matching("u8", "0..256 => yield 1\n_ => yield 2"), ["LC3018"], "an excluded bound too");
    assert_eq!(matching("i8", "-128..128 => yield 1\n_ => yield 2"), ["LC3018"]);
    assert_eq!(matching("i8", "-129..=0 => yield 1\n_ => yield 2"), ["LC3018"]);
    let said = |arms: &str| {
        let text = program("s: u8", "y: i32", &format!("y = match s {{\n{arms}\n_ => yield 2\n}}"));
        diagnostics(&[("f.lcf", text.as_str())]).first().map_or(String::new(), |diagnostic| diagnostic.message.clone())
    };
    assert!(said("0..256 => yield 1").contains("write `..=255`"), "{}", said("0..256 => yield 1"));
    assert!(!said("0..=256 => yield 1").contains("write"), "an included bound has no such way out");
    assert!(!said("0..300 => yield 1").contains("write"), "and `..=299` would not fit either");
    assert!(!said("256..300 => yield 1").contains("write"), "the first bound does not fit, whatever the last one");
}

#[test]
fn patterns_do_not_overlap() {
    assert_eq!(matching("i32", "1 => yield 1\n2 => yield 2\n_ => yield 3"), NOTHING);
    assert_eq!(matching("i32", "1 => yield 1\n1 => yield 2\n_ => yield 3"), ["LC3011"]);
    assert_eq!(matching("i32", "1, 1 => yield 1\n_ => yield 2"), ["LC3011"], "in one arm");
    assert_eq!(matching("i32", "1_000 => yield 1\n1000 => yield 2\n_ => yield 3"), ["LC3011"], "by value");
    assert_eq!(matching("i32", "-0 => yield 1\n0 => yield 2\n_ => yield 3"), ["LC3011"]);
    assert_eq!(matching("i32", "1..=3 => yield 1\n3..=5 => yield 2\n_ => yield 3"), ["LC3011"]);
    assert_eq!(matching("i32", "1..3 => yield 1\n3..=5 => yield 2\n_ => yield 3"), NOTHING, "the end is excluded");
    assert_eq!(matching("i32", "1..=3 => yield 1\n1..=3 => yield 2\n_ => yield 3"), ["LC3011"]);
    assert_eq!(matching("i32", "5 => yield 1\n1..=10 => yield 2\n_ => yield 3"), ["LC3011"], "whatever the order");
    assert_eq!(matching("i32", "1..=10 => yield 1\n2 => yield 2\n9 => yield 3\n_ => yield 4"), ["LC3011", "LC3011"], "a long range stays in force");
    assert_eq!(matching("u64", "18446744073709551615 => yield 1\n0..=18446744073709551615 => yield 2\n_ => yield 3"), ["LC3011"]);
    assert_eq!(matching("bool", "true, true, false => yield 1"), ["LC3011"]);
    assert_eq!(matching("string", "\"a\" => yield 1\n\"a\" => yield 2\n_ => yield 3"), ["LC3011"]);
    assert_eq!(matching("string", "\"a\\n\" => yield 1\n\"a\\\\n\" => yield 2\n_ => yield 3"), NOTHING, "after their escapes");
    assert_eq!(matching("string", "\"\\\"\" => yield 1\n\"\\\"\" => yield 2\n_ => yield 3"), ["LC3011"]);
    assert_eq!(matching("string", "\"\u{e9}\" => yield 1\n\"e\u{301}\" => yield 2\n_ => yield 3"), NOTHING, "R-20 leaves strings alone");
    assert_eq!(matching("u8", "300 => yield 1\n300 => yield 2\n_ => yield 3"), ["LC3018", "LC3018"], "what does not fit has its diagnostic");
}

#[test]
fn a_pattern_is_written_once() {
    assert_eq!(matching("i32", "_ => yield 1\n_ => yield 2"), ["LC3022"]);
    assert_eq!(matching("i32", "1, _, _ => yield 1"), ["LC3022"]);
    assert_eq!(matching("i32", "1, _ => yield 1\n_ => yield 2"), ["LC3022"]);
    assert_eq!(matching("Verdict", "Verdict.Pass => yield 1\nVerdict.Pass => yield 2\nVerdict.Fail => yield 3"), ["LC3022"]);
    assert_eq!(matching("Verdict", "Verdict.Pass, Verdict.Pass, Verdict.Fail => yield 1"), ["LC3022"]);
    assert_eq!(matching("Reading", "Reading.Value(v) => yield 1\nReading.Value(w) => yield 2\n_ => yield 3"), ["LC3022"]);
    assert_eq!(handling("_ => yield 1\n_ => yield 2"), ["LC3022"]);
    assert_eq!(handling("_(a) => yield 1\n_(b) => yield 2"), ["LC3022"]);
    assert_eq!(handling("ok => yield 1\nok => yield 2\n_ => yield 3"), ["LC3022"]);
    assert_eq!(handling("code 5 => yield 1\ncode 5 => yield 2\n_ => yield 3"), ["LC3022"]);
    assert_eq!(handling("code 5 => yield 1\ncode 0_5 => yield 2\n_ => yield 3"), ["LC3022"], "by value");
    assert_eq!(handling("code 5 => yield 1\ncode -5 => yield 2\n_ => yield 3"), NOTHING);
    assert_eq!(handling("a.b => yield 1\na.b(x) => yield 2\n_ => yield 3"), ["LC3022"]);
    assert_eq!(handling("a.* => yield 1\na.* => yield 2\n_ => yield 3"), ["LC3022"]);
}

#[test]
fn a_handle_has_its_wildcard() {
    assert_eq!(handling("_ => yield 1"), NOTHING);
    assert_eq!(handling("ok => yield 1\na.b => yield 2\na.* => yield 3\ncode 1 => yield 4\n_(x) => yield 5"), NOTHING, "the most specific arm wins");
    assert_eq!(handling(""), ["LC3012"]);
    assert_eq!(handling("ok => yield 1"), ["LC3012"]);
    assert_eq!(handling("a.* => yield 1\nok => yield 2"), ["LC3012"]);
    assert_eq!(judged("x: i32", "y: i32", "y = handle x {\nok => yield 1\n}"), ["LC3012", "LC3013"], "whatever its selector");
    assert_eq!(judged("x: i32", "y: i32", "y = handle x {\n_ => yield 1\n_ => yield 2\n}"), ["LC3013", "LC3022"]);
}

#[test]
fn a_wildcard_that_catches_nothing_has_a_warning() {
    assert_eq!(matching("bool", "true, false, _ => yield 1"), ["LC4106"]);
    assert_eq!(matching("bool", "true => yield 1\nfalse => yield 2\n_ => yield 3"), ["LC4106"]);
    assert_eq!(matching("Verdict", "Verdict.Pass => yield 1\nVerdict.Fail => yield 2\n_ => yield 3"), ["LC4106"]);
    assert_eq!(matching("Verdict", "Verdict.Pass => yield 1\n_, Verdict.Fail => yield 2"), ["LC4106"]);
    assert_eq!(matching("bool", "_ => yield 1"), NOTHING);
}

#[test]
fn the_fix_of_a_useless_wildcard_removes_it() {
    let removed = |arms: &str| {
        let text = format!("labcraft 1\n\n{}", program("s: bool", "y: i32", &format!("y = match s {{\n{arms}\n}}")));
        let found = diagnostics(&[("f.lcf", text.trim_start_matches("labcraft 1\n\n"))]);
        let fix = found.first().and_then(|diagnostic| diagnostic.fixes.first()).cloned();
        let Some(fix) = fix else { return (String::new(), None) };
        let [edit] = fix.edits.as_slice() else { return (fix.title, None) };
        assert_eq!(edit.span.start.line, edit.span.end.line, "a list stays on the line of its `=>`");
        let line = text.lines().nth(edit.span.start.line as usize - 1).unwrap_or("");
        let cut = |column: u32| line.char_indices().nth(column as usize - 1).map_or(line.len(), |(place, _)| place);
        (fix.title, Some(format!("{}{}", line.get(..cut(edit.span.start.column)).unwrap_or(""), line.get(cut(edit.span.end.column)..).unwrap_or(""))))
    };
    let kept = Some("true, false => yield 1".to_owned());
    assert_eq!(removed("true, false, _ => yield 1"), ("Remove `_`".to_owned(), kept.clone()));
    assert_eq!(removed("_, true, false => yield 1"), ("Remove `_`".to_owned(), kept.clone()));
    assert_eq!(removed("true, _, false => yield 1"), ("Remove `_`".to_owned(), kept));
    let (title, edit) = removed("_ => yield 1\ntrue => yield 2\nfalse => yield 3");
    assert!(title.starts_with("Remove this arm"), "{title}");
    assert_eq!(edit, None, "it can decide the type of the others (R-7), so the fix has no edit");
}

/// The code, the line and the related lines of each diagnostic of a function with these arms.
fn places(selector: &str, arms: &str, kind: &str) -> Vec<(&'static str, u32, Vec<u32>)> {
    let text = format!("use core\n\nfn f(s: {selector}) -> (y: i32) {{\ny = {kind} s {{\n{arms}\n}}\n}}");
    let files = [("f.lcf", text.as_str()), ("verdict.lct", "type Verdict = enum { Pass, Fail }")];
    diagnostics(&files)
        .iter()
        .map(|diagnostic| {
            let line_of = |span: &Option<labcraft_types::Span>| span.map_or(0, |span| span.start.line);
            (diagnostic.code().as_str(), line_of(&diagnostic.span), diagnostic.related.iter().map(|related| line_of(&related.span)).collect())
        })
        .collect()
}

#[test]
fn a_repeated_pattern_is_reported_where_it_is_repeated() {
    // The arms start on line 7, after the header that `source` adds.
    let two = "1 => yield 1\n1 => yield 2\n_ => yield 3";
    assert_eq!(places("i32", two, "match"), [("LC3011", 8, vec![7])]);
    assert_eq!(places("i32", "1..=9 => yield 1\n_ => yield 2\n5 => yield 3", "match"), [("LC3011", 9, vec![7])]);
    assert_eq!(
        places("i32", "5 => yield 1\n_ => yield 2\n1..=9 => yield 3", "match"),
        [("LC3011", 7, vec![9])],
        "the one that starts inside the other"
    );
    assert_eq!(places("bool", "true => yield 1\ntrue => yield 2\n_ => yield 3", "match"), [("LC3011", 8, vec![7])]);
    assert_eq!(places("string", "\"a\" => yield 1\n\"a\" => yield 2\n_ => yield 3", "match"), [("LC3011", 8, vec![7])]);
    assert_eq!(places("i32", "_ => yield 1\n_ => yield 2", "match"), [("LC3022", 8, vec![7])]);
    assert_eq!(places("error", "ok => yield 1\nok => yield 2\n_ => yield 3", "handle"), [("LC3022", 8, vec![7])]);
    let variants = "Verdict.Pass => yield 1\nVerdict.Pass => yield 2\nVerdict.Fail => yield 3";
    assert_eq!(places("Verdict", variants, "match"), [("LC3022", 8, vec![7])]);
}

#[test]
fn what_covers_nothing_is_reported_on_what_is_at_fault() {
    assert_eq!(places("bool", "true => yield 1\nfalse => yield 2\n_ => yield 3", "match"), [("LC4106", 9, vec![])], "on the `_`");
    assert_eq!(places("bool", "true => yield 1", "match"), [("LC3010", 6, vec![])], "on the `match`");
    assert_eq!(places("error", "ok => yield 1", "handle"), [("LC3012", 6, vec![])], "on the `handle`");
    assert_eq!(places("i32", "3..=1 => yield 1\n_ => yield 2", "match"), [("LC3009", 7, vec![])]);
}

#[test]
fn escapes_and_categories_compare_as_the_spec_says() {
    assert_eq!(matching("string", "\"\\t\" => yield 1\n\"\t\" => yield 2\n_ => yield 3"), ["LC3011"], "an escape and the character it writes");
    assert_eq!(matching("string", "\"\\n\" => yield 1\n\"\\r\" => yield 2\n\"\\\\\" => yield 3\n_ => yield 4"), NOTHING);
    assert_eq!(handling("a.b => yield 1\na.b.* => yield 2\n_ => yield 3"), NOTHING, "two arms: the most specific one wins");
    assert_eq!(handling("a.b => yield 1\na.c => yield 2\n_ => yield 3"), NOTHING);
}

#[test]
fn a_mistaken_pattern_hides_what_depends_on_it() {
    assert_eq!(matching("bool", "true => yield 1\n5 => yield 2"), ["LC3009"], "no `LC3010`: it may have meant `false`");
    assert_eq!(matching("bool", "true, false, 5, _ => yield 1"), ["LC3009"], "no `LC4106`");
    assert_eq!(matching("Verdict", "Verdict.Pass => yield 1\nGrade.Fail => yield 2"), ["LC3009"]);
    assert_eq!(matching("Verdict", "Verdict.Pass => yield 1\nVerdict.Nope => yield 2"), ["LC2007"]);
    assert_eq!(matching("Verdict", "Verdict.Pass => yield 1\nVerdict.Fail(x) => yield 2\n_ => yield 3"), ["LC3009"]);
    assert_eq!(matching("bool", "true => yield 1\ntrue => yield 2"), ["LC3011"], "a repeat may be the one that is missing");
    assert_eq!(matching("bool", "true, true => yield 1"), ["LC3011"]);
    assert_eq!(matching("bool", "true, true, false, _ => yield 1"), ["LC3011"], "no `LC4106`");
    assert_eq!(matching("bool", "true, false, _, _ => yield 1"), ["LC3022"], "no `LC4106`");
    assert_eq!(matching("Verdict", "Verdict.Pass => yield 1\nVerdict.Pass => yield 2"), ["LC3022"]);
    assert_eq!(matching("Verdict", "Verdict.Pass, Verdict.Pass => yield 1"), ["LC3022"]);
    assert_eq!(matching("Verdict", "Verdict.Pass, Verdict.Pass, Verdict.Fail, _ => yield 1"), ["LC3022"], "no `LC4106`");
    assert_eq!(matching("i32", "1, 1 => yield 1"), ["LC3010", "LC3011"], "a missing `_` depends on no other pattern");
    assert_eq!(matching("string", "\"a\", \"a\" => yield 1"), ["LC3010", "LC3011"]);
    assert_eq!(matching("i32", "\"a\" => yield 1"), ["LC3010", "LC3009"], "an integer has `_` whatever its patterns");
    assert_eq!(matching("string", "1 => yield 1"), ["LC3010", "LC3009"]);
    assert_eq!(matching("f64", "1 => yield 1\n1 => yield 2"), ["LC3013"], "an unknown selector says nothing of its arms");
    assert_eq!(matching("f64", "_ => yield 1\n_ => yield 2"), ["LC3013", "LC3022"], "but `_` is `_`");
    assert_eq!(matching("Nope", "1 => yield 1"), ["LC2003"]);
}

#[test]
fn nested_arms_are_judged_with_their_own_selector() {
    let body = "y = match a {\ntrue => {\nz = match b {\ntrue => yield 1\n}\nyield z\n}\nfalse => yield 2\n}";
    assert_eq!(judged("a: bool, b: bool", "y: i32", body), ["LC3010"]);
}

#[test]
fn a_long_list_of_patterns_is_judged() {
    use std::fmt::Write;
    let mut arms = String::new();
    for value in 0..200_000 {
        let _ = write!(arms, "{value}, ");
    }
    arms.push_str("_ => yield 1");
    assert_eq!(matching("i32", &arms), NOTHING);
}

mod arrays;
mod constructions;
mod fields;
mod panel;
