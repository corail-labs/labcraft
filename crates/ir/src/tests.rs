//! One lowering test per construct of the language and per kind of node (`docs/ir.md`, section 4). Each expected text is
//! written by hand from the program: what each line makes, its inputs in the order of section 4, the line numbers counted in
//! the source, and its position in the canonical order of spec 5.2, worked out with its algorithm (ordinary lines by name,
//! then `next`, `collect`, `until`, then the lines that only bind an output, in signature order). None of them comes from the
//! output of `print`.

use std::borrow::Cow;

use labcraft_check::{Content, Found, Libraries, LibrarySource, Origin, SourceFile, check_for_run};
use labcraft_types::{Interner, Limits};

use crate::{ConstValue, NodeKind, Program, Source, lower, print_function, verify};

struct Nothing;

impl Libraries for Nothing {
    fn find(&self, _: &str, _: Origin) -> Found {
        Found::NotFound
    }
}

/// Checks the files as one folder, lowers the function of the first one, and verifies it.
fn lowered(files: &[(&str, &str)]) -> (Program, Interner) {
    let mut root = LibrarySource::new(String::new(), String::new(), Origin::Project);
    for (shown, text) in files {
        root.files.push(SourceFile { shown: (*shown).to_owned(), content: Content::Bytes(Cow::Owned(text.as_bytes().to_vec())) });
    }
    let mut names = Interner::new();
    let checked = check_for_run(root, &Nothing, &mut names, &Limits::default());
    assert_eq!(checked.report.errors(), 0, "{:?}", checked.report.diagnostics());
    let start = checked.files.iter().position(|file| file.shown == files[0].0).unwrap();
    let program = lower(&checked, start, &mut names).unwrap();
    assert_eq!(verify(&program), Vec::new());
    (program, names)
}

fn text_of(files: &[(&str, &str)]) -> String {
    let (program, names) = lowered(files);
    print_function(&program, crate::FunctionId(0), &names)
}

/// A literal takes the type of each use (3.4): `k` gives an `f32` to `take` and an `f64` to `core.mul`, as two constants. The
/// output `y` is in `V` and its value in `mV`: the result converts with one division by 1000 (3.3). The default of `gain` is
/// made explicit as a constant of its type. Order: `k` first, which the others read, then `g` before `y`.
#[test]
fn a_constant_takes_the_type_of_each_use() {
    let source = "labcraft 1

use core

fn scale(x: f64[mV], gain: f32 = 2) -> (y: f64[V], g: f32) {
    k = 0.5
    y = core.mul(x, k)
    g = take(k, gain)
}
";
    let take = "labcraft 1

use core

fn take(v: f32, w: f32) -> (z: f32) {
    z = core.add(v, w)
}
";
    let expected = "fn scale(x: f64[mV], gain: f32 = 2.0) -> (y: f64[V], g: f32)
  region (x, gain) -> (y as f64[V] (/1000.0), g)
    0 k = constant 0.5
    1 g = call take(0.5, gain)  runs
    2 y = primitive core.mul(x, 0.5)
";
    let files = [("scale.lcf", source), ("take.lcf", take)];
    assert_eq!(text_of(&files), expected);
    let (program, _) = lowered(&files);
    let crate::Body::Region(region) = &program.functions[0].body else { panic!() };
    let constant_of = |node: usize, input: usize| match region.nodes[node].inputs[input].source {
        Source::Const(id) => &program.constants[id.0 as usize],
        _ => panic!("node {node} reads no constant"),
    };
    let (single, double) = (constant_of(1, 0), constant_of(2, 1));
    assert_ne!(single.ty, double.ty);
    assert!(matches!(program.types.get(single.ty), labcraft_types::Ty::Number { kind: labcraft_types::NumberKind::F32, .. }));
    assert!(matches!((&single.value, &double.value), (ConstValue::Float(a), ConstValue::Float(b)) if *a == 0.5 && *b == 0.5));
}

/// A `for` over an array and a range, with a carry and a `collect`; a `loop` with a carry, a `next` that names a value and an
/// `until`, reading `total` and `limit` from outside, which its node passes in after its carry (RVSDG context).
#[test]
fn loops_pass_in_what_their_bodies_read() {
    let source = "labcraft 1

use core

fn sums(values: [f64], limit: f64) -> (total: f64, indices: [i32], count: i32) {
    total, indices = for x in values, i in 0..10, carry acc = 0.0 {
        next acc = core.add(acc, x)
        collect i
    }
    count = loop carry n = 0 {
        m = core.add(n, 1)
        big = core.ge(total, limit)
        next n = m
        until big
    }
}
";
    let expected = "fn sums(values: [f64], limit: f64) -> (total: f64, indices: [i32], count: i32)
  region (values, limit) -> (total, indices, count)
    0 total, indices = for [each, range] (carries 1, collects 1) (values, 0, 10, 0.0)
      region (x, i, acc) -> (next.acc, i)
        0 @7: next.acc = primitive core.add(acc, x)
    1 count = loop (carries 1, collects 0) (0, total, limit)
      region (n, total, limit) -> (m, big)
        0 big = primitive core.ge(total, limit)
        1 m = primitive core.add(n, 1)
";
    assert_eq!(text_of(&[("sums.lcf", source)]), expected);
}

const READING: &str = "labcraft 1

type Reading = enum { Value(f64[V]), Missing }
";

const PAIR: &str = "labcraft 1

type Pair = struct { a: f64[V], b: string }
";

/// A `match` on an enumeration binds the value of a variant; a `match` on integers has a literal, a range and `_`, which is
/// tried last wherever it is written (6.1); a `handle` has `ok`, a category, and `_(err)` that binds the error.
#[test]
fn arms_hold_their_patterns_and_bind_what_they_say() {
    let source = "labcraft 1

use errors

fn arms(r: Reading, n: i32, err: error = none) -> (v: f64[V], w: f64[V], e: error) {
    v = match r {
        Reading.Value(x) => yield x
        Reading.Missing => yield 0 V
    }
    w = match n {
        _ => yield 3 V
        1 => yield 1 V
        2..=4 => yield 2 V
    }
    e = handle err {
        ok => yield none
        io.timeout => yield none
        _(other) => yield other
    }
}
";
    let expected = "fn arms(r: Reading, n: i32, err: error = none) -> (v: f64[V], w: f64[V], e: error)
  region (r, n, err) -> (v, w, e)
    0 e = handle (err)
      arm ok () -> (none)
      arm io.timeout () -> (none)
      arm _ binds 1 (other) -> (other)
    1 v = match (r)
      arm Value binds 1 (x) -> (x)
      arm Missing () -> (0.0 V)
    2 w = match (n)
      arm 1 () -> (1.0 V)
      arm 2..=4 () -> (2.0 V)
      arm _ () -> (3.0 V)
";
    assert_eq!(text_of(&[("arms.lcf", source), ("Reading.lct", READING)]), expected);
}

/// A struct is built with its fields in declaration order, whatever order the line writes them in; a variant with its values; a
/// field read and a new name make no node (R-4), but keep their position (`a` is 0); `[a, b]` on a line is a node, which only
/// binds an output and so comes last, and as an argument a use of uses; `string.format`
/// takes its fields in the order of its template, which is data of the node.
#[test]
fn constructions_and_arrays_are_nodes_and_names_are_not() {
    let source = "labcraft 1

use string

fn build(p: Pair, name: string) -> (q: Pair, r: Reading, both: [string], joined: string, shown: string) {
    a = p.a
    q = Pair { b: name, a: a }
    r = Reading.Value(a)
    both = [name, p.b]
    joined = string.join([name, p.b])
    shown = string.format(\"{y}-{x}\", x: name, y: p.b)
}
";
    let expected = "fn build(p: Pair, name: string) -> (q: Pair, r: Reading, both: [string], joined: string, shown: string)
  region (p, name) -> (q, r, both, joined, shown)
    1 joined = primitive string.join([name, p.b])
    2 q = construct Pair {p.a, name}
    3 r = construct Reading.Value(p.a)
    4 shown = primitive string.format(p.b, name, template \"{y}-{x}\")
    5 both = array [name, p.b]
";
    assert_eq!(text_of(&[("build.lcf", source), ("Pair.lct", PAIR), ("Reading.lct", READING)]), expected);
}

/// What the runtime needs of spec 7.2: a primitive with an `err` input skips, its outputs take their defaults and `array` passes
/// its input through (5.3); a call of a LabCraft function runs; a variadic parameter takes consecutive inputs; the `length` of
/// `array.slice` with no wire is the constant that reaches the end (decision of the owner, 2026-11-09).
#[test]
fn errors_on_the_wire_are_decided_at_lowering() {
    let source = "labcraft 1

use array
use errors

fn wired(items: [f64], e1: error, e2: error, err: error = none) -> (out: [f64], part: [f64], merged: error, e: error) {
    out, e3 = array.set(items, 0, 1.5, err: err)
    part, e4 = array.slice(items, start: 1, err: e3)
    merged = errors.merge(e1, e2)
    e = e4
}
";
    let expected = "fn wired(items: [f64], e1: error, e2: error, err: error = none) -> (out: [f64], part: [f64], merged: error, e: error)
  region (items, e1, e2, err) -> (out, part, merged, e4)
    0 merged = primitive errors.merge(e1, e2)
    1 out, e3 = primitive array.set(items, 0, 1.5, err)  skip(err: 3)
    2 part, e4 = primitive array.slice(items, 1, …, e3)  skip(err: 3)
";
    let (program, names) = lowered(&[("wired.lcf", source)]);
    assert_eq!(print_function(&program, crate::FunctionId(0), &names), expected);
    let crate::Body::Region(region) = &program.functions[0].body else { panic!() };
    let crate::Skip::OnError { through, defaults, always, .. } = &region.nodes[1].skip else { panic!() };
    // `array` is named like an input, the first; `err` like the fourth. Neither is `always`: only a manifest says so.
    assert_eq!((through.as_slice(), defaults.len(), *always), (&[Some(0), Some(3)][..], 2, false));
    let NodeKind::Primitive { variadic, .. } = &region.nodes[0].kind else { panic!() };
    assert_eq!(*variadic, Some(crate::Variadic { start: 0, count: 2 }));
}

/// An order on a node of the function, written inside a loop, goes to the loop: a region reads and waits for nothing outside it
/// (`docs/ir.md`, section 3). A call of a LabCraft function names it by its id, and its default comes from its signature.
#[test]
fn an_order_from_inside_goes_to_the_structure() {
    let source = "labcraft 1

use time

fn ordered(period: duration) -> (n: i32, err: error) {
    #first: err = time.wait(period)
    n = loop carry c = 0 {
        e = time.wait(period, after: #first)
        d = helper(c)
        next c = d
        until true
    }
}
";
    let helper = "labcraft 1

use core

fn helper(x: i32, step: i32 = 2) -> (y: i32) {
    y = core.add(x, step)
}
";
    // By hand: the loop waits for the node of `#first`, named by its first output, `err`; in the loop, `d` comes before `e`.
    let expected = "fn ordered(period: duration) -> (n: i32, err: error)
  region (period) -> (n, err)
    0 err = primitive time.wait(period, none)  skip(err: 1)
    1 n = loop (carries 1, collects 0) (0, period) after(err)
      region (c, period) -> (d, true)
        0 d = call helper(c, 2)  runs
        1 e = primitive time.wait(period, none)  skip(err: 1)
";
    let (program, names) = lowered(&[("ordered.lcf", source), ("helper.lcf", helper)]);
    assert_eq!(print_function(&program, crate::FunctionId(0), &names), expected);
    assert_eq!(program.functions.len(), 2);
}

/// A parallel `for` has no carry; `core.call` takes the function, then its arguments in the order of its type, and skips as
/// the function it runs would, whatever names its type gives (decision of the owner, 2026-11-09); a function written in the folder with `extern rust` has a native body, and its call is a native node
/// whose `err` skips it; a panel element is data of `ui.get` and `ui.set`, never a wire (R-1); `[r]` is an array of one wire
/// (R-5). Order: `doubled` and `r`, which read no line, by name; then `n`, `out`, `shown`, `stored`, which all read `r`; then
/// `one`, which only binds an output. `core.call` wires `x` before `err`, as its type says, whatever the call writes.
#[test]
fn natives_references_and_panels_lower_as_data() {
    let source = "labcraft 1

use array
use core
use ui

fn more(values: [f64], f: fn(x: f64, err: error) -> (f64, error)) -> (doubled: [f64], r: f64, shown: f64, stored: f64, one: [f64], out: f64) {
    doubled = for parallel x in values {
        d = core.mul(x, 2.0)
        collect d
    }
    r, e = core.call(f, err: none, x: 1.0)
    shown = ui.get(gain)
    stored = ui.set(level, value: r)
    one = [r]
    n = array.len([r])
    out, e2 = twice(r, err: e)
}

panel {
    control gain: numeric(default: 1.5)
    indicator level: numeric
}
";
    let twice = "labcraft 1

extern rust(crate: \"labcraft-twice\", function: \"twice_of\")
fn twice(x: f64, err: error = none) -> (y: f64, err: error)
";
    let expected =
        "fn more(values: [f64], f: fn(x: f64, err: error) -> (f64, error)) -> (doubled: [f64], r: f64, shown: f64, stored: f64, one: [f64], out: f64)
  element gain: numeric f64 = 1.5
  element level: numeric f64 = 0.0
  region (values, f) -> (doubled, r, shown, stored, one, out)
    0 doubled = for parallel [each] (carries 0, collects 1) (values)
      region (x) -> (d)
        0 d = primitive core.mul(x, 2.0)
    1 r, e = primitive core.call(f, 1.0, none)  skip(by callee)
    2 n = primitive array.len([r])
    3 out, e2 = native twice(r, e)  skip(err: 1)
    4 shown = primitive ui.get(element gain)
    5 stored = primitive ui.set(r, element level)
    6 one = array [r]
";
    let (program, names) = lowered(&[("more.lcf", source), ("twice.lcf", twice)]);
    assert_eq!(print_function(&program, crate::FunctionId(0), &names), expected);
    let native = program.functions.iter().find(|function| &*function.shown == "twice").unwrap();
    let crate::Body::Native { function, runs, .. } = &native.body else { panic!("twice has no native body") };
    assert_eq!((names.resolve(*function), *runs), ("twice_of", true));
}

/// The arms of a `handle` in the order they are tried, the most specific first (7.3): `ok`, a code, an exact category,
/// `a.b.*` before `a.*`, then `_`, whatever order they are written in. A loop that gives nothing has no output (6.2).
#[test]
fn the_most_specific_arm_of_a_handle_is_tried_first() {
    let source = "labcraft 1

use core

fn picked(err: error, xs: [f64]) -> (n: i32) {
    n = handle err {
        _ => yield 9
        io.* => yield 4
        io.timeout => yield 2
        io.serial.* => yield 3
        code -5 => yield 1
        ok => yield 0
    }
    _ = for x in xs {
        y = core.neg(x)
    }
}
";
    // By hand: `_ = for` is on line 14 and has the empty name (no output but `_`, no label), which comes before `n` (5.2).
    let expected = "fn picked(err: error, xs: [f64]) -> (n: i32)
  region (err, xs) -> (n)
    0 @14: for [each] (carries 0, collects 0) (xs)
      region (x) -> ()
        0 y = primitive core.neg(x)
    1 n = handle (err)
      arm ok () -> (0)
      arm code -5 () -> (1)
      arm io.timeout () -> (2)
      arm io.serial.* () -> (3)
      arm io.* () -> (4)
      arm _ () -> (9)
";
    assert_eq!(text_of(&[("picked.lcf", source)]), expected);
}

/// The verifier sees a use that reads another type than what it names: a result read through a field it does not have, and
/// one read as another unit with no conversion.
#[test]
fn the_verifier_sees_a_use_of_the_wrong_type() {
    let source = "labcraft 1

use core

fn pair(x: f64[mV]) -> (y: f64[mV]) {
    y = core.neg(x)
}
";
    let (mut program, _) = lowered(&[("pair.lcf", source)]);
    let volts = program
        .types
        .intern(labcraft_types::Ty::Number { kind: labcraft_types::NumberKind::F64, unit: labcraft_types::parse_unit("V", 8).unwrap() })
        .unwrap();
    let crate::Body::Region(region) = &mut program.functions[0].body else { panic!() };
    region.nodes[0].inputs[0].ty = volts;
    region.results[0].fields = vec![7];
    let messages: Vec<String> = verify(&program).into_iter().map(|violation| violation.message).collect();
    assert!(messages.iter().any(|message| message.contains("is not what it reads")), "{messages:?}");
    assert!(messages.iter().any(|message| message.contains("field 7")), "{messages:?}");
}

/// The text form of a constant is cut, so that it stays within a constant factor of its file: a long text, written in full
/// once on its line, is written wherever it is used as its first 78 characters between its two quotes, 80 bytes, then `…`.
#[test]
fn a_long_constant_is_cut_where_it_is_printed() {
    let long = "A".repeat(10_000);
    let source = format!(
        "labcraft 1

use string

fn f() -> (x: string) {{
    t = \"{long}\"
    x = string.concat(t, t)
}}
"
    );
    let text = text_of(&[("f.lcf", &source)]);
    let cut = format!("\"{}\"…", "A".repeat(78));
    assert!(text.contains(&format!("primitive string.concat({cut}, {cut})")), "{}", text.chars().take(600).collect::<String>());
    assert!(text.len() < 1_000, "{}", text.len());
}

/// The 80 bytes of a constant count the names of its types and variants and its brackets too. `[T.` and 77 of the 200 `V` of
/// the variant are 80 bytes; an array nested 200 deep is 80 `[`. Each line stays short however long the name or deep the array.
#[test]
fn names_and_brackets_count_in_what_a_constant_writes() {
    let variant = "V".repeat(200);
    let source = format!(
        "labcraft 1

use array

fn f() -> (y: [T], n: i32) {{
    k = [T.{variant}]
    y = array.concat(k, k)
    n = array.len(d)
    d = {open}1{close}
}}
",
        open = "[".repeat(200),
        close = "]".repeat(200),
    );
    let enumeration = format!("type T = enum {{ {variant}, B }}");
    let ty = format!("labcraft 1\n\n{enumeration}\n");
    let text = text_of(&[("f.lcf", &source), ("T.lct", &ty)]);
    let cut = format!("[T.{}…", "V".repeat(77));
    assert!(text.contains(&format!("primitive array.concat({cut}, {cut})")), "{text}");
    assert!(text.contains(&format!("primitive array.len({}…)", "[".repeat(80))), "{text}");
    assert!(text.lines().all(|line| line.len() < 300), "{text}");
}

/// What a `core.call` applies comes with the function it runs (7.2): `string.to_f64` as a value skips on its input 1, `err`,
/// its outputs taking `0.0` and `none`, and its `err` passing the one it got; `twice`, an `extern` function, the same by its
/// own ports. Worked out from their signatures: `(text, err) -> (value, err)` and `(x, err) -> (y, err)`.
#[test]
fn a_function_called_by_reference_brings_its_skip() {
    let source = "labcraft 1

use core
use string

fn g(t: string) -> (v: f64, e: error, w: f64, e2: error) {
    v, e = core.call(string.to_f64, t, none)
    w, e2 = core.call(twice, v, none)
}
";
    let twice = "labcraft 1

extern rust(crate: \"labcraft-twice\", function: \"twice_of\")
fn twice(x: f64, err: error = none) -> (y: f64, err: error)
";
    let (program, _) = lowered(&[("g.lcf", source), ("twice.lcf", twice)]);
    let value = |skip: &crate::Skip| match skip {
        crate::Skip::OnError { err, always, defaults, through } => {
            let defaults: Vec<&ConstValue> = defaults.iter().map(|id| &program.constants[id.0 as usize].value).collect();
            Some((*err, *always, format!("{defaults:?}"), through.clone()))
        }
        _ => None,
    };
    let expected = Some((1, false, "[Float(0.0), NoError]".to_owned(), vec![None, Some(1)]));
    let primitive = program.constants.iter().find_map(|constant| match &constant.value {
        ConstValue::Primitive { skip, .. } => Some(skip),
        _ => None,
    });
    assert_eq!(primitive.map(value), Some(expected.clone()));
    let native = program.functions.iter().find(|function| &*function.shown == "twice").unwrap();
    let crate::Body::Native { skip, .. } = &native.body else { panic!("twice has no native body") };
    assert_eq!(value(skip), expected);
}

/// `ByCallee` belongs to a call of a reference alone: on `core.neg`, whose input is no function, or on a native body, the
/// verifier says so.
#[test]
fn the_verifier_keeps_a_skip_by_the_callee_to_a_call_of_a_reference() {
    let source = "labcraft 1

use core

fn neg(x: f64) -> (y: f64) {
    y = core.neg(x)
}
";
    let (mut program, _) = lowered(&[("neg.lcf", source)]);
    let crate::Body::Region(region) = &mut program.functions[0].body else { panic!() };
    region.nodes[0].skip = crate::Skip::ByCallee;
    let messages: Vec<String> = verify(&program).into_iter().map(|violation| violation.message).collect();
    assert!(messages.iter().any(|message| message.contains("calls no reference")), "{messages:?}");
}

/// A text is written as the language writes it: `it's` keeps its apostrophe, which has no escape (`LC1003`), and a quote and a
/// tab take theirs. A text that starts where no character of it fits is not written at all: 73 `a` between quotes and `, ` are
/// 1 + 75 + 2 = 78 bytes after `[`, and `"` and one more character with its closing quote would pass 80.
#[test]
fn a_text_is_written_as_the_language_writes_it() {
    let filler = "a".repeat(73);
    let source = format!(
        "labcraft 1

use array
use string

fn f() -> (x: string, n: i32) {{
    t = \"it's \\\"so\\\"\\t\"
    x = string.concat(t, t)
    n = array.len(l)
    l = [\"{filler}\", \"bc\"]
}}
"
    );
    let text = text_of(&[("f.lcf", &source)]);
    assert!(text.contains(r#"primitive string.concat("it's \"so\"\t", "it's \"so\"\t")"#), "{text}");
    assert!(text.contains(&format!("array.len([\"{filler}\", …)")), "{text}");
}

/// What `check` counts against `LC4007` is what lowering builds (spec 6). Counted by hand: `k` is `step` and `d` is `a` (a new
/// name and a field read are followed), `lim` is a constant, `v` a carry of the loop. The loop passes in `step` and `a`, on
/// its node and in its region: 2 × 2. The `match` passes in the same two, on its node and in its two arms: 2 × 3. In all 10.
#[test]
fn what_check_counts_passed_in_is_what_lowering_builds() {
    let source = "labcraft 1

use core

fn f(c: bool, step: Step, a: f64) -> (y: f64) {
    k = step.low
    lim = 2.0
    d = a
    y = match c {
        true => {
            r = loop carry v = 0.0 {
                w = core.add(v, k)
                q = core.add(w, d)
                s = core.mul(q, lim)
                done = core.gt(s, a)
                next v = s
                until done
            }
            yield r
        }
        false => yield d
    }
}
";
    let step = "labcraft 1

type Step = struct { low: f64 }
";
    assert_eq!(passed_in_both(&[("f.lcf", source), ("Step.lct", step)]), (10, 10));
}

/// Counted by hand: in the `match`, `x` comes from the `for` and `b` from the function, on its node and in its two arms:
/// 2 × 3. The `for` passes in `b` and the selector `c`, on its node and in its region: 2 × 2; `xs` and `n` are its clauses,
/// not values passed in. In all 10.
#[test]
fn a_value_passes_into_each_structure_between_its_line_and_where_it_is_read() {
    let source = "labcraft 1

use core

fn g(xs: [f64], n: i32, b: f64, c: i32) -> (ys: [f64]) {
    ys = for x in xs, i in 0..n {
        z = match c {
            1 => {
                u = core.add(x, b)
                yield u
            }
            _ => yield x
        }
        collect z
    }
}
";
    assert_eq!(passed_in_both(&[("g.lcf", source)]), (10, 10));
}

/// The values passed in of the function of the first file, as `check` counts them and as lowering built them.
fn passed_in_both(files: &[(&str, &str)]) -> (u64, u64) {
    let mut root = LibrarySource::new(String::new(), String::new(), Origin::Project);
    for (shown, text) in files {
        root.files.push(SourceFile { shown: (*shown).to_owned(), content: Content::Bytes(Cow::Owned(text.as_bytes().to_vec())) });
    }
    let mut names = Interner::new();
    let checked = check_for_run(root, &Nothing, &mut names, &Limits::default());
    assert_eq!(checked.report.errors(), 0, "{:?}", checked.report.diagnostics());
    let start = checked.files.iter().position(|file| file.shown == files[0].0).unwrap();
    let file = &checked.files[start];
    let labcraft_graph::Decl::Function(function) = &file.file.decl else { panic!("not a function") };
    let counted = labcraft_check::passed_in(function, &file.resolved, u64::MAX).count;
    let program = lower(&checked, start, &mut names).unwrap();
    let crate::Body::Region(region) = &program.functions[0].body else { panic!("no region") };
    (counted, region.passed_in())
}

/// Counted by hand, each rule of `check::passed_in` a value of its own, so that leaving a rule out changes the count. The loop
/// passes in `h` (its body) and `stop` (its `until`): 2 × 2. The arm passes in `a` (the first value of the carry), `h` and
/// `stop`: the `match` has one arm, 3 × 2. In all 10.
#[test]
fn the_first_value_of_a_carry_and_until_are_read_where_they_are_written() {
    let source = "labcraft 1

use core

fn f(c: i32, a: f64, h: f64, stop: bool) -> (y: f64) {
    y = match c {
        _ => {
            r = loop carry v = a {
                w = core.add(v, h)
                next v = w
                until stop
            }
            yield r
        }
    }
}
";
    assert_eq!(passed_in_both(&[("f.lcf", source)]), (10, 10));
}

/// Counted by hand: `next v = z` reads the output `z`, which `z = d` binds, so it is `d`. The loop passes in `d` and `s`: 2 × 2;
/// the `match`, `a`, `d` and `s`: 3 × 2. In all 10. Reading `v` itself is the carry, never passed in.
#[test]
fn next_and_an_output_read_back_are_followed_to_what_they_name() {
    let source = "labcraft 1

fn g(c: i32, a: f64, d: f64, s: bool) -> (y: f64, z: f64) {
    z = d
    y = match c {
        _ => {
            r = loop carry v = a {
                next v = z
                until s
            }
            yield r
        }
    }
}
";
    assert_eq!(passed_in_both(&[("g.lcf", source)]), (10, 10));
}

/// Counted by hand: the `for` passes in `b` and `q` (the fields of the struct), `g` (through `[g]`, an array of one value for
/// a parameter that takes a value, R-5) and `e` (its second `collect`): 4 × 2. The arm passes in `xs` and `n` (the clauses, the
/// end of the range included), `b`, `q`, `g` and `e`: 6 × 2. In all 20.
#[test]
fn clauses_fields_brackets_and_collect_read_what_they_name() {
    let source = "labcraft 1

fn h(c: i32, xs: [f64], n: i32, b: f64, q: f64, g: f64, e: f64) -> (ps: [P], m: [f64]) {
    ps, m = match c {
        _ => {
            r, es, ms = for x in xs, i in 0..n {
                p = P { u: b, w: [q] }
                t = first([g])
                collect p
                collect e
                collect t
            }
            yield (r, ms)
        }
    }
}
";
    let first = "labcraft 1

fn first(xs: [f64]) -> (y: f64) {
    y = 1.0
}
";
    let p = "labcraft 1

type P = struct { u: f64, w: [f64] }
";
    assert_eq!(passed_in_both(&[("h.lcf", source), ("P.lct", p), ("first.lcf", first)]), (20, 20));
}

/// Counted by hand: `ee` is bound by the pattern of its arm, and the loop inside passes it in, with `s`: 2 × 2. The `handle`
/// passes in `a` and `s`, on its node and in its two arms: 2 × 3. In all 10.
#[test]
fn what_a_pattern_binds_is_defined_in_its_arm() {
    let source = "labcraft 1

use errors

fn k(a: f64, s: bool, err: error) -> (y: f64) {
    y = handle err {
        ok => yield a
        _(ee) => {
            r = loop carry v = a {
                e2 = errors.merge(ee, ee)
                next v = v
                until s
            }
            yield r
        }
    }
}
";
    assert_eq!(passed_in_both(&[("k.lcf", source)]), (10, 10));
}

/// A call written alone has the outputs of what it calls, which nothing reads (architecture 5): `f0(x)` one `f64`, as `f0`
/// declares it, and `core.call(f0, x)` the same, from the function type of `f0`. `lowered` verifies them.
#[test]
fn a_call_written_alone_has_the_outputs_of_what_it_calls() {
    let main = "labcraft 1

use core

fn main(x: f64) -> (z: f64) {
    core.call(f0, x)
    f0(x)
    z = x
}
";
    let f0 = "labcraft 1

fn f0(x: f64) -> (y: f64) {
    y = x
}
";
    let (program, _) = lowered(&[("main.lcf", main), ("f0.lcf", f0)]);
    let crate::Body::Region(region) = &program.functions[0].body else { panic!("no region") };
    let f64_type = region.args[0].ty;
    let outputs: Vec<Vec<labcraft_types::TyId>> = region.nodes.iter().map(|node| node.outputs.iter().map(|value| value.ty).collect()).collect();
    assert_eq!(outputs, [vec![f64_type], vec![f64_type]]);
}

/// Every kind of call written alone, with what it gives worked out from the declarations: `core.call` of a parameter by
/// named arguments, the two outputs of its function type (`f64`, `string`); `two`, its own two (`f64`, `string`), twice; the
/// `extern` function `q`, its `f64` and its `error`; the variant `Shape.Circle(x)`, one `Shape`. The two calls of `two` share
/// one list of outputs, as `check` shares their types.
#[test]
fn every_call_written_alone_has_what_its_callee_gives() {
    let main = "labcraft 1

use core

fn main(x: f64, s: string, g: fn(a: f64, b: string) -> (f64, string)) -> (z: f64) {
    core.call(g, b: s, a: x)
    two(x)
    two(x)
    q(x)
    Shape.Circle(x)
    z = x
}
";
    let two = "labcraft 1

fn two(x: f64) -> (y: f64, t: string) {
    y = x
    t = \"\"
}
";
    let q = "labcraft 1

extern rust(crate: \"c\", function: \"q\")
fn q(x: f64, err: error = none) -> (y: f64, err: error)
";
    let shape = "labcraft 1

type Shape = enum { Circle(f64), Empty }
";
    let files = [("main.lcf", main), ("two.lcf", two), ("q.lcf", q), ("Shape.lct", shape)];
    let (program, names) = lowered(&files);
    let crate::Body::Region(region) = &program.functions[0].body else { panic!("no region") };
    let shown = |node: &crate::Node| -> Vec<String> {
        node.outputs.iter().map(|value| program.types.display(value.ty, program.functions[0].library, &names).to_string()).collect()
    };
    let by_line: Vec<(u32, Vec<String>)> = region.nodes.iter().map(|node| (node.position, shown(node))).collect();
    let mut sorted = by_line.clone();
    sorted.sort();
    assert_eq!(by_line.len(), 5);
    let all: Vec<Vec<String>> = sorted.into_iter().map(|(_, types)| types).collect();
    assert!(all.contains(&vec!["f64".to_owned(), "string".to_owned()]), "{all:?}");
    assert!(all.contains(&vec!["f64".to_owned(), "error".to_owned()]), "{all:?}");
    assert!(all.contains(&vec!["Shape".to_owned()]), "{all:?}");
    assert_eq!(all.iter().filter(|types| **types == ["f64", "string"]).count(), 3, "{all:?}");
    let twos: Vec<&crate::Node> = region.nodes.iter().filter(|node| matches!(node.kind, NodeKind::Call { .. })).collect();
    assert_eq!(twos.len(), 2);
    assert!(std::sync::Arc::ptr_eq(&twos[0].outputs, &twos[1].outputs), "one list for the calls of one callee");
}

/// What `check` counts against `LC4008` is what lowering builds for its calls. Counted by hand: `core.call(g, b: s, a: x)` the
/// function, its two inputs and the two outputs of its type, 5; `two(x)` its two parameters, `k` by its default, and its two
/// outputs, 4; `q(x)` its two parameters, `err` by its default, and its two outputs, 4; `string.format` its one field and its
/// text, 2; `core.add` 2 and 1, 3; `errors.merge` its two errors, one variadic parameter, and its error, 3. In all 21.
#[test]
fn what_check_counts_as_terminals_is_what_lowering_builds() {
    let main = "labcraft 1

use core
use errors
use string

fn main(x: f64, s: string, g: fn(a: f64, b: string) -> (f64, string)) -> (z: f64, m: string, e: error) {
    core.call(g, b: s, a: x)
    two(x)
    q(x)
    m = string.format(\"{s}\", s: s)
    z = core.add(x, x)
    e = errors.merge(none, none)
}
";
    let two = "labcraft 1

fn two(x: f64, k: f64 = 2.0) -> (y: f64, t: string) {
    y = k
    t = \"\"
}
";
    let q = "labcraft 1

extern rust(crate: \"c\", function: \"q\")
fn q(x: f64, err: error = none) -> (y: f64, err: error)
";
    let files = [("main.lcf", main), ("two.lcf", two), ("q.lcf", q)];
    let mut root = LibrarySource::new(String::new(), String::new(), Origin::Project);
    for (shown, text) in files {
        root.files.push(SourceFile { shown: shown.to_owned(), content: Content::Bytes(Cow::Owned(text.as_bytes().to_vec())) });
    }
    let mut names = Interner::new();
    let checked = check_for_run(root, &Nothing, &mut names, &Limits::default());
    assert_eq!(checked.report.errors(), 0, "{:?}", checked.report.diagnostics());
    let start = checked.files.iter().position(|file| file.shown == "main.lcf").unwrap();
    let file = &checked.files[start];
    let labcraft_graph::Decl::Function(function) = &file.file.decl else { panic!("not a function") };
    let counted = labcraft_check::terminals(function, &file.resolved, &checked.libraries, &names, u64::MAX).count;
    let program = lower(&checked, start, &mut names).unwrap();
    let crate::Body::Region(region) = &program.functions[0].body else { panic!("no region") };
    assert_eq!((counted, region.terminals()), (21, 21));
}
