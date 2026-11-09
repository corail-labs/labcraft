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
    let writer = Writer { env: Env { program, names }, viewer: function.library };
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
