//! The bodies on what lowering delivers: a program goes through `check` and `ir`, and a small walk runs the nodes of its
//! body in their order, as the interpreter will, through [`run`] and [`outputs_on_error`] with the `Skip` that lowering
//! built. The expected values are worked out by hand from the program and appendix A.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use labcraft_check::{Content, Found, Libraries, LibrarySource, Origin, SourceFile, check_for_run};
use labcraft_ir::{Body, NodeKind, Program, Skip, Source, Use, lower};
use labcraft_types::{Interner, Limits};

use super::tests::FakeHost;
use super::{Call, Outcome, outputs_on_error, run};
use crate::convert::{Env, constant, read};
use crate::text::{Text, Writer};
use crate::value::Value;

struct Nothing;

impl Libraries for Nothing {
    fn find(&self, _: &str, _: Origin) -> Found {
        Found::NotFound
    }
}

fn lowered(source: &str) -> (Program, Interner) {
    let mut root = LibrarySource::new(String::new(), String::new(), Origin::Project);
    root.files.push(SourceFile { shown: "g.lcf".to_owned(), content: Content::Bytes(Cow::Owned(source.as_bytes().to_vec())) });
    let mut names = Interner::new();
    let checked = check_for_run(root, &Nothing, &mut names, &Limits::default());
    assert_eq!(checked.report.errors(), 0, "{:?}", checked.report.diagnostics());
    let program = lower(&checked, 0, &mut names).unwrap();
    (program, names)
}

/// Runs the body of the first function on its arguments: its nodes are primitives and constants only.
fn run_body(program: &Program, names: &Interner, args: Vec<Value>) -> Vec<Value> {
    let env = Env { program, names };
    let function = &program.functions[0];
    let Body::Region(region) = &function.body else { panic!("a native body") };
    let mut outputs: HashMap<(u32, u32), Value> = HashMap::new();
    let value_of = |used: &Use, outputs: &HashMap<(u32, u32), Value>| -> Value {
        fn source(used: &Use, args: &[Value], outputs: &HashMap<(u32, u32), Value>, env: Env<'_>) -> Value {
            let raw = match &used.source {
                Source::Arg(index) => args[*index as usize].clone(),
                Source::Out { node, output } => outputs[&(*node, *output)].clone(),
                Source::Const(id) => constant(env, *id).unwrap(),
                Source::Array(items) => Value::array(items.iter().map(|item| source(item, args, outputs, env)).collect()),
            };
            match &used.source {
                Source::Array(_) => raw,
                _ => read(&raw, used).unwrap(),
            }
        }
        source(used, &args, outputs, env)
    };
    for (index, node) in region.nodes.iter().enumerate() {
        let inputs: Vec<Value> = node.inputs.iter().map(|used| value_of(used, &outputs)).collect();
        let results = match &node.kind {
            NodeKind::Constant => inputs,
            NodeKind::Primitive { library, name, statics, variadic } => {
                let skips = matches!(&node.skip, Skip::OnError { err, always: false, .. } if inputs[*err as usize].as_error().is_some());
                if skips {
                    let types: Vec<_> = node.outputs.iter().map(|output| output.ty).collect();
                    outputs_on_error(env, &node.skip, &inputs, &types, None, &FakeHost::new()).unwrap()
                } else {
                    let mut call = Call {
                        env,
                        viewer: function.library,
                        input_types: node.inputs.iter().map(|used| used.ty).collect(),
                        output_types: node.outputs.iter().map(|output| output.ty).collect(),
                        inputs,
                        statics,
                        variadic: *variadic,
                    };
                    match run(*library, names.resolve(*name), &mut call, &mut FakeHost::new()).unwrap() {
                        Outcome::Done(values) => values,
                        Outcome::Fail(error) => {
                            outputs_on_error(env, &node.skip, &call.inputs, &call.output_types, Some(error), &FakeHost::new()).unwrap()
                        }
                        other => panic!("{other:?}"),
                    }
                }
            }
            other => panic!("not in this walk: {other:?}"),
        };
        assert_eq!(results.len(), node.outputs.len(), "node {index}");
        for (output, value) in results.into_iter().enumerate() {
            outputs.insert((index as u32, output as u32), value);
        }
    }
    region.results.iter().map(|used| value_of(used, &outputs)).collect()
}

fn text(program: &Program, names: &Interner, value: &Value, index: usize) -> String {
    let function = &program.functions[0];
    let mut out = Text::new(1 << 20);
    let writer = Writer { env: Env { program, names }, viewer: function.library, cut: false };
    writer.write(&mut out, value, function.outputs[index].ty, None).unwrap();
    out.finish()
}

fn grid(rows: &[&[f64]]) -> Value {
    crate::convert::grid(rows.iter().map(|row| Value::array(row.iter().map(|x| Value::F64(*x)).collect())).collect(), 2).unwrap()
}

#[test]
fn the_bodies_take_what_lowering_gives_them() {
    let source = "labcraft 1

use array
use string

fn g(m: [f64; 2], k: [[f64]], xs: [i32], v: f64[V]) -> (row: [f64], set: [[f64]], pushed: [[f64]], joined: [[f64]], cut: [i32], whole: [i32], e: error, text: string) {
    row, e1 = array.get(m, 1)
    set, e2 = array.set(m, 0, [9.0], err: e1)
    pushed = array.push(m, [5.0, 6.0])
    joined = array.concat(m, k)
    cut, e3 = array.slice(xs, 1, err: e2)
    whole, e = array.slice(xs, 9, err: e3)
    text = string.format(\"{v} and {v:.1 mV}\", v: v)
}
";
    let (program, names) = lowered(source);
    let args = vec![
        grid(&[&[1.0, 2.0], &[3.0, 4.0]]),
        Value::array(vec![Value::array(vec![Value::F64(7.0)])]),
        Value::array(vec![Value::I32(10), Value::I32(20), Value::I32(30)]),
        Value::F64(1.5),
    ];
    let results = run_body(&program, &names, args);
    let shown: Vec<String> = (0..results.len()).map(|index| text(&program, &names, &results[index], index)).collect();
    // By hand, from appendix A.2: the row 1 of m; m with row 0 set, as rows; a row pushed; the rows of m then of k; from index
    // 1 to the end; a start past the end, which fails and passes `xs` through (decision of the owner, 7.2), with its error.
    assert_eq!(
        shown,
        [
            "[3.0, 4.0]",
            "[[9.0], [3.0, 4.0]]",
            "[[1.0, 2.0], [3.0, 4.0], [5.0, 6.0]]",
            "[[1.0, 2.0], [3.0, 4.0], [7.0]]",
            "[20, 30]",
            "[10, 20, 30]",
            "array.index_out_of_range: start 9 is past the end of the array (size 3)",
            "1.5 V and 1500.0 mV",
        ]
    );
    let _ = Arc::new(());
}

#[test]
fn a_node_after_an_error_skips_with_the_skip_that_lowering_built() {
    let source = "labcraft 1

use array

fn g(xs: [i32]) -> (first: i32, second: [i32], e: error) {
    first, e1 = array.get(xs, 5)
    second, e = array.set(xs, 0, 1, err: e1)
}
";
    let (program, names) = lowered(source);
    let results = run_body(&program, &names, vec![Value::array(vec![Value::I32(10), Value::I32(20)])]);
    let shown: Vec<String> = (0..results.len()).map(|index| text(&program, &names, &results[index], index)).collect();
    // By hand: `get` fails, its `item` takes the default of `i32`; `set` skips, its `array` passes `xs` through and its `err`
    // the error it received.
    assert_eq!(shown, ["0", "[10, 20]", "array.index_out_of_range: index 5 outside the array (size 2)"]);
}

#[test]
fn a_set_of_a_grid_that_fails_gives_its_rows() {
    let source = "labcraft 1

use array

fn g(m: [f64; 2], e0: error) -> (failed: [[f64]], skipped: [[f64]], e: error) {
    failed, e1 = array.set(m, 5, [1.0])
    skipped, e = array.set(m, 0, [1.0], err: e0)
}
";
    let (program, names) = lowered(source);
    let e0 = Value::error(crate::value::ErrorData {
        category: "io.timeout".into(),
        code: 0,
        message: "".into(),
        origin: None,
        others: std::sync::Arc::default(),
    });
    let results = run_body(&program, &names, vec![grid(&[&[1.0, 2.0], &[3.0, 4.0]]), e0]);
    let shown: Vec<String> = (0..results.len()).map(|index| text(&program, &names, &results[index], index)).collect();
    // By hand (decision of the owner, 2026-11-09): an error of `array.set` gives the array unchanged, as its rows, whether
    // it fails (index 5 of 2 rows) or skips (an error on `err`).
    assert_eq!(shown, ["[[1.0, 2.0], [3.0, 4.0]]", "[[1.0, 2.0], [3.0, 4.0]]", "io.timeout"]);
}

fn bytes(data: &[u8]) -> Value {
    Value::Bytes(Arc::from(data))
}

/// Runs the body of `g` in `source` on `args`, and shows each output as `run` writes it.
fn shown(source: &str, args: Vec<Value>) -> Vec<String> {
    let (program, names) = lowered(source);
    let results = run_body(&program, &names, args);
    (0..results.len()).map(|index| text(&program, &names, &results[index], index)).collect()
}

#[test]
fn bytes_from_and_to_text_arrays_and_parts() {
    let source = "labcraft 1

use bytes

fn g(ab: bytes, one_two: bytes, three: bytes, ff: bytes, data: bytes) -> (a: bytes, la: i32, e_acute: bytes, le: i32, t: string, bad: string, eb: error, arr: bytes, back: [u8], joined: bytes, nothing: bytes, s1: bytes, s2: bytes, s3: bytes, es: error) {
    a = bytes.from_string(\"AB\")
    la = bytes.len(a)
    e_acute = bytes.from_string(\"\u{e9}\")
    le = bytes.len(e_acute)
    t, _ = bytes.to_string(ab)
    bad, eb = bytes.to_string(ff)
    arr = bytes.from_array([1, 2, 3])
    back = bytes.to_array(arr)
    joined = bytes.concat(one_two, three)
    nothing = bytes.concat()
    s1, _ = bytes.slice(data, 1, 2)
    s2, _ = bytes.slice(data, 1)
    s3, es = bytes.slice(data, 2, 5)
}
";
    let shown = shown(source, vec![bytes(&[0x41, 0x42]), bytes(&[1, 2]), bytes(&[3]), bytes(&[0xff]), bytes(&[1, 2, 3])]);
    // By hand: "A" and "B" are 0x41 and 0x42; "é" (U+00E9) is c3 a9 in UTF-8; 0xff starts no UTF-8 character, so `""` and
    // `bytes.invalid_text`; [1, 2, 3] is 010203 and back; 0102 then 03; nothing at all is empty; from 1 for 2 of 010203 is
    // 0203, from 1 to the end the same; 5 from 2 goes past the end of 3: it fails, and passes `data` through.
    assert_eq!(
        shown,
        [
            "4142",
            "2",
            "c3a9",
            "2",
            "AB",
            "",
            "bytes.invalid_text: not UTF-8: no character at position 0",
            "010203",
            "[1, 2, 3]",
            "010203",
            "",
            "0203",
            "0203",
            "010203",
            "bytes.out_of_range: 5 bytes from position 2 go past the end of the bytes (length 3)",
        ]
    );
}

#[test]
fn bytes_read_integers_and_f32_at_a_position() {
    let source = "labcraft 1

use bytes

fn g(one_two: bytes, fffe: bytes, x100: bytes, one: bytes, pi: bytes, quiet: bytes) -> (be: u16, le: u16, minus_two: i16, n256: u32, second: u8, f: f32, p: f32, q: f32, past: u16, ep: error, negative: u8, en: error, li: i32) {
    be, _ = bytes.read_u16(one_two, 0)
    le, _ = bytes.read_u16(one_two, 0, little_endian: true)
    minus_two, _ = bytes.read_i16(fffe, 0)
    n256, _ = bytes.read_u32(x100, 0)
    second, _ = bytes.read_u8(one_two, 1)
    f, _ = bytes.read_f32(one, 0)
    p, _ = bytes.read_f32(pi, 0)
    q, _ = bytes.read_f32(quiet, 0)
    past, ep = bytes.read_u16(one_two, 1)
    negative, en = bytes.read_u8(one_two, -1)
    li, _ = bytes.read_i32(fffe, 1)
}
";
    let (program, names) = lowered(source);
    let args = vec![
        bytes(&[1, 2]),
        bytes(&[0xff, 0xfe]),
        bytes(&[0, 0, 1, 0]),
        bytes(&[0x3f, 0x80, 0, 0]),
        bytes(&[0x40, 0x49, 0x0f, 0xdb]),
        bytes(&[0x7f, 0xc0, 0, 0]),
    ];
    let results = run_body(&program, &names, args);
    let shown: Vec<String> = (0..results.len()).map(|index| text(&program, &names, &results[index], index)).collect();
    // By hand: 0x0102 is 258, 0x0201 is 513; 0xfffe in two's complement is −2; 0x00000100 is 256; the byte at 1 of 0102 is 2;
    // 0x3f800000 is 1.0 (sign 0, exponent 127, mantissa 0). Two bytes at 1 of a length of 2, and a position −1, fail and give 0.
    // Four bytes at 1 of fffe go past the end too, and give 0, not the position.
    assert_eq!(shown[..6], ["258", "513", "-2", "256", "2", "1.0"]);
    // 0x40490fdb is the f32 nearest π (exponent 128, mantissa 0x490fdb: 1.5707963705062866 × 2), as `f32::consts::PI` is.
    assert!(matches!(results[6], Value::F32(p) if p.to_bits() == std::f32::consts::PI.to_bits()), "{:?}", results[6]);
    // 0x7fc00000 has an exponent of all ones and a mantissa that is not 0: a `nan`.
    assert!(matches!(results[7], Value::F32(q) if q.is_nan()), "{:?}", results[7]);
    assert_eq!(
        shown[8..],
        [
            "0",
            "bytes.out_of_range: 2 bytes at position 1 go past the end of the bytes (length 2)",
            "0",
            "bytes.out_of_range: position -1 is negative",
            "0",
        ]
    );
}

#[test]
fn bytes_from_integers_and_f32() {
    let source = "labcraft 1

use bytes

fn g(n: i32, small: u8) -> (be: bytes, le: bytes, b255: bytes, b256: bytes, e256: error, neg: bytes, eneg: error, big: bytes, ebig: error, m2: bytes, max: bytes, one: bytes, infinite: bytes, wide: bytes, from_u8_wire: bytes, i32_le: bytes) {
    be, _ = bytes.from_u16(258)
    le, _ = bytes.from_u16(258, little_endian: true)
    b255, _ = bytes.from_u8(255)
    b256, e256 = bytes.from_u8(256)
    neg, eneg = bytes.from_u16(-1)
    big, ebig = bytes.from_u32(4294967296)
    m2, _ = bytes.from_i16(-2)
    max, _ = bytes.from_u32(4294967295)
    one = bytes.from_f32(1.0)
    infinite = bytes.from_f32(1e39)
    wide, _ = bytes.from_u16(n)
    from_u8_wire, _ = bytes.from_u8(small)
    i32_le, _ = bytes.from_i32(-2, little_endian: true)
}
";
    let shown = shown(source, vec![Value::I32(258), Value::U8(7)]);
    // By hand: 258 is 0x0102, turned around 0201; 255 is ff; 256 is no u8, −1 no u16 and 2^32 no u32: empty bytes and the
    // error; −2 in an i16 is fffe; 2^32 − 1 is ffffffff; 1.0 is 3f800000; 1e39 is beyond the largest f32 (about 3.4e38), so
    // an infinity, 7f800000; an i32 wire widens to the i64 input (3.4); a u8 wire too; −2 in an i32 is fffffffe, turned
    // around feffffff.
    assert_eq!(
        shown,
        [
            "0102",
            "0201",
            "ff",
            "",
            "bytes.out_of_range: 256 is outside the range of u8",
            "",
            "bytes.out_of_range: -1 is outside the range of u16",
            "",
            "bytes.out_of_range: 4294967296 is outside the range of u32",
            "fffe",
            "ffffffff",
            "3f800000",
            "7f800000",
            "0102",
            "07",
            "feffffff",
        ]
    );
}

#[test]
fn bytes_in_little_endian_and_text_cut_short() {
    let source = "labcraft 1

use bytes

fn g(x: bytes, feff: bytes, one_le: bytes, cut: bytes, bad: bytes) -> (u: u32, i: i16, f: f32, back: bytes, t: string, e1: error, t2: string, e2: error) {
    u, _ = bytes.read_u32(x, 0, little_endian: true)
    i, _ = bytes.read_i16(feff, 0, little_endian: true)
    f, _ = bytes.read_f32(one_le, 0, little_endian: true)
    back, _ = bytes.from_u32(16909060, little_endian: true)
    t, e1 = bytes.to_string(cut)
    t2, e2 = bytes.to_string(bad)
}
";
    let shown =
        shown(source, vec![bytes(&[1, 2, 3, 4]), bytes(&[0xfe, 0xff]), bytes(&[0, 0, 0x80, 0x3f]), bytes(&[0x41, 0xc3]), bytes(&[0xe2, 0x28, 0xa1])]);
    // By hand: 01020304 read from the low end is 0x04030201, 67305985; feff turned around is fffe, −2; 0000803f turned around
    // is 3f800000, 1.0; 16909060 is 0x01020304, written from the low end 04030201. `A` then c3, which starts a character of
    // two bytes that the end cuts short; e2 starts one of three, and 28 cannot continue it.
    assert_eq!(
        shown,
        [
            "67305985",
            "-2",
            "1.0",
            "04030201",
            "",
            "bytes.invalid_text: not UTF-8: the character at position 1 is cut short by the end",
            "",
            "bytes.invalid_text: not UTF-8: no character at position 0",
        ]
    );
}

#[test]
fn a_node_of_bytes_after_an_error_skips() {
    let source = "labcraft 1

use bytes

fn g(d: bytes, e0: error) -> (s: bytes, v: u16, t: string, f: bytes, e: error) {
    s, e1 = bytes.slice(d, 1, err: e0)
    v, e2 = bytes.read_u16(d, 0, err: e1)
    t, e3 = bytes.to_string(d, err: e2)
    f, e = bytes.from_u16(1, err: e3)
}
";
    let e0 =
        Value::error(crate::value::ErrorData { category: "io.timeout".into(), code: 0, message: "".into(), origin: None, others: Arc::default() });
    let shown = shown(source, vec![bytes(&[1, 2]), e0]);
    // By hand (spec 7.2): each node skips; `data` of `slice` passes `d` through, the others take their default (0, "",
    // empty bytes), and `err` passes on the error received.
    assert_eq!(shown, ["0102", "0", "", "", "io.timeout"]);
}

#[test]
fn bytes_write_any_nan_as_one_pattern() {
    let source = "labcraft 1

use bytes

fn g(negative: f64, signalling: f64) -> (a: bytes, b: bytes, c: bytes) {
    a = bytes.from_f32(negative)
    b = bytes.from_f32(signalling)
    c = bytes.from_f32(nan, little_endian: true)
}
";
    // A negative quiet nan (what 0/0 gives on x86_64) and a signalling one, by their IEEE 754 bits; and the literal.
    let shown = shown(source, vec![Value::F64(f64::from_bits(0xfff8_0000_0000_0000)), Value::F64(f64::from_bits(0x7ff0_0000_0000_0001))]);
    // By hand (spec A.2, decision of the owner, 2026-11-19): every `nan` is 7fc00000, turned around 0000c07f.
    assert_eq!(shown, ["7fc00000", "7fc00000", "0000c07f"]);
}
