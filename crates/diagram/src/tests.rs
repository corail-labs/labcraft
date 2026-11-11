//! Tests of the graph and the sizes. Each expected number is worked out by hand from notation 2.3, 3.1 and 3.2, and the
//! decisions of step 2 of M0v; none comes from the output of the code. A cell is 1233/2048 of the size: at 12,
//! `n` cells are `n × 1233 × 1200 / 2048` hundredths, rounded to the nearest (7.22 units for one).

use std::borrow::Cow;

use labcraft_check::{Checked, Content, Found, Libraries, LibrarySource, Origin, SourceFile, check};
use labcraft_types::{Code, Interner, Limits, TypeTable};

use crate::graph::{Base, Builder, Family, Graph, Kind, Terminal, function_of, orders_of};
use crate::size::{Sizer, lines};
use crate::{DiagramLimits, Drawn, Shape, ViewState, draw, svg};

struct Nothing;

impl Libraries for Nothing {
    fn find(&self, _: &str, _: Origin) -> Found {
        Found::NotFound
    }
}

fn source(shown: &str, text: &str) -> SourceFile {
    SourceFile { shown: shown.to_owned(), content: Content::Bytes(Cow::Owned(text.as_bytes().to_vec())) }
}

/// The files as one folder; a path `dev/open.lcf` puts a file in the sub-library `dev`.
fn checked(files: &[(&str, &str)]) -> (Checked, Interner) {
    let mut names = Interner::new();
    let checked = checked_with(files, &mut names);
    (checked, names)
}

fn checked_with(files: &[(&str, &str)], names: &mut Interner) -> Checked {
    let mut root = LibrarySource::new(String::new(), String::new(), Origin::Project);
    for (path, text) in files {
        match path.split_once('/') {
            Some((library, file)) => {
                let index = match root.sub_libraries.iter().position(|sub| sub.name == library) {
                    Some(index) => index,
                    None => {
                        root.sub_libraries.push(LibrarySource::new(library.to_owned(), library.to_owned(), Origin::Project));
                        root.sub_libraries.len() - 1
                    }
                };
                root.sub_libraries[index].files.push(source(file, text));
            }
            None => root.files.push(source(path, text)),
        }
    }
    check(root, &Nothing, names, &Limits::default())
}

fn index_of(checked: &Checked, shown: &str) -> usize {
    checked.files.iter().position(|file| file.shown == shown).unwrap()
}

fn graph<'a>(checked: &'a Checked, names: &'a Interner, shown: &str) -> Graph<'a> {
    let file = &checked.files[index_of(checked, shown)];
    let function = function_of(file).unwrap();
    Builder::new(checked, file, names).graph(function, &orders_of(file, function, names), u64::MAX).unwrap()
}

const SESSION: &str = "labcraft 1\n\ntype session = opaque\n";

const OPEN: &str = "labcraft 1

extern rust(crate: \"labcraft-dev\", function: \"open\")
fn open(resource: string, timeout: duration = 2 s, err: error = none) -> (session: session, err: error)
";

const QUERY: &str = "labcraft 1

extern rust(crate: \"labcraft-dev\", function: \"query_f64\")
fn query_f64(session: session, command: string, err: error = none) -> (value: f64, session: session, err: error)
";

const CLOSE: &str = "labcraft 1

extern rust(crate: \"labcraft-dev\", function: \"close\")
fn close(session: session, err: error = none) -> (err: error)
";

const MAIN: &str = "labcraft 1

use core
use dev

fn main(resource: string, err: error = none) -> (value: f64, err: error) {
    dmm, e1 = dev.open(resource, err: err)
    raw, dmm2, e2 = dev.query_f64(dmm, \"MEAS:VOLT:DC?\", err: e1)
    #close: e3 = dev.close(dmm2, err: e2)
    value = core.add(raw, 1.5)
    err = e3
}
";

const BENCH: [(&str, &str); 5] =
    [("main.lcf", MAIN), ("dev/session.lct", SESSION), ("dev/open.lcf", OPEN), ("dev/query_f64.lcf", QUERY), ("dev/close.lcf", CLOSE)];

fn bench() -> (Checked, Interner) {
    checked(&BENCH)
}

fn terminal(name: &str, ty: labcraft_types::TyId, base: Base) -> Terminal {
    Terminal {
        name: name.to_owned(),
        ty,
        family: Family { base, dimensions: 0 },
        unit: None,
        constant: None,
        ignored: false,
        reads: Vec::new(),
        pill: None,
    }
}

/// Notation 3.1: `visa.open` has `resource` and `dmm`, then `timeout` alone, then the pair `err`, `e1`; `scpi.query_f64`
/// has `value` alone, the pair `session`, `command` alone, the pair `err`: four lines.
#[test]
fn the_lines_of_a_node_follow_notation_3_1() {
    let mut types = TypeTable::new(64);
    let text = types.intern(labcraft_types::Ty::String).unwrap();
    let error = types.intern(labcraft_types::Ty::ErrorValue).unwrap();
    let number = types.intern(labcraft_types::Ty::Duration).unwrap();
    let session = types.intern(labcraft_types::Ty::Queue(number)).unwrap();
    let open_in = [terminal("resource", text, Base::Text), terminal("timeout", number, Base::Number), terminal("err", error, Base::Error)];
    let open_out = [terminal("session", session, Base::Session), terminal("err", error, Base::Error)];
    assert_eq!(lines(&open_in, &open_out), [(Some(0), Some(0)), (Some(1), None), (Some(2), Some(1))]);
    let query_in = [terminal("session", session, Base::Session), terminal("command", text, Base::Text), terminal("err", error, Base::Error)];
    let query_out = [terminal("value", number, Base::Number), terminal("session", session, Base::Session), terminal("err", error, Base::Error)];
    assert_eq!(lines(&query_in, &query_out), [(None, Some(0)), (Some(0), Some(1)), (Some(1), None), (Some(2), Some(2))]);
    // `err` first in the signature still goes to the bottom line.
    let first_in = [terminal("err", error, Base::Error), terminal("x", number, Base::Number)];
    let first_out = [terminal("err", error, Base::Error), terminal("y", number, Base::Number)];
    assert_eq!(lines(&first_in, &first_out), [(Some(1), Some(1)), (Some(0), Some(0))]);
    // An error output with no error input is alone on the bottom line (`errors.merge`).
    let merge_in = [terminal("a", number, Base::Number), terminal("b", number, Base::Number)];
    let merge_out = [terminal("err", error, Base::Error)];
    assert_eq!(lines(&merge_in, &merge_out), [(Some(0), None), (Some(1), None), (None, Some(0))]);
    // Two sessions given back in the other order: the second pair would cross the first and is not kept; output `b`,
    // before the pair in its list, takes a line above it, input `b` one below.
    let swap_in = [terminal("a", session, Base::Session), terminal("b", session, Base::Session)];
    let swap_out = [terminal("b", session, Base::Session), terminal("a", session, Base::Session)];
    assert_eq!(lines(&swap_in, &swap_out), [(None, Some(0)), (Some(0), Some(1)), (Some(1), None)]);
}

/// The graph: nodes in canonical order, a new name for a wire draws nothing, the name of each node as cli.md 4.3 gives
/// it, the library in the band, `always` from the manifest only (none here), constants on their terminals.
#[test]
fn the_graph_has_a_node_per_drawn_line() {
    let (checked, names) = bench();
    let graph = graph(&checked, &names, "main.lcf");
    assert_eq!(graph.function, "main");
    assert_eq!(graph.bodies.len(), 1);
    let keys: Vec<&str> = graph.bodies[0].nodes.iter().map(|&index| graph.nodes[index].key.as_str()).collect();
    // Canonical order (spec 5.2): by dependencies, ties by name: `dmm`, `raw`, then `e3` (`#close`) before `value`.
    assert_eq!(keys, ["dmm", "raw", "e3", "value"]);
    let close = &graph.nodes[graph.bodies[0].nodes[2]];
    assert_eq!(close.label.as_deref(), Some("#close"));
    assert!(matches!(&close.kind, Kind::Call { qualified, short, library: Some(library), glyph: None, .. }
        if qualified == "dev.close" && short == "close" && library == "dev"));
    let raw = &graph.nodes[graph.bodies[0].nodes[1]];
    let inputs: Vec<(&str, Option<&str>)> = raw.inputs.iter().map(|input| (input.name.as_str(), input.constant.as_deref())).collect();
    assert_eq!(inputs, [("session", None), ("command", Some("\"MEAS:VOLT:DC?\"")), ("err", None)]);
    assert_eq!(raw.outputs.iter().map(|output| output.family.base).collect::<Vec<_>>(), [Base::Number, Base::Session, Base::Error]);
    let dmm = &graph.nodes[graph.bodies[0].nodes[0]];
    // `timeout` has a default and no argument: still a terminal, at its place, so that the node has the size of its
    // signature at every call (notation 3.2).
    let names: Vec<(&str, Option<&str>)> = dmm.inputs.iter().map(|input| (input.name.as_str(), input.constant.as_deref())).collect();
    assert_eq!(names, [("resource", None), ("timeout", None), ("err", None)]);
    let value = &graph.nodes[graph.bodies[0].nodes[3]];
    assert!(matches!(&value.kind, Kind::Call { glyph: Some(crate::graph::Glyph::Sign("+")), library: None, .. }));
}

/// The texts of a node in `node.title`: its name, its glyph.
fn titles(sized: &crate::size::Sized) -> Vec<String> {
    sized
        .shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::Text { runs, .. } if runs.iter().all(|run| run.token == "node.title") => Some(runs.iter().map(|run| run.text.as_str()).collect()),
            _ => None,
        })
        .collect()
}

/// Notation 3.2: a compact block is `node.square` wide, whatever its name or its band, and as tall as its lines need,
/// 40 at least; `query_f64` on two lines.
#[test]
fn a_compact_block_has_the_width_of_the_square() {
    let (checked, names) = bench();
    let graph = graph(&checked, &names, "main.lcf");
    let sizer = Sizer { names: &names, standard_everywhere: false };
    let band = labcraft_types::text::ShownText::plain("dev");
    let size_of = |key: &str| {
        let node = graph.nodes.iter().find(|node| node.key == key).unwrap();
        sizer.size(node, Some(&band), None)
    };
    // `open`: three lines (`resource`, `timeout` at its place, `err`), 8 + 2 × 8 + 8 = 32, under 40.
    let open = size_of("dmm");
    assert_eq!((open.width, open.height), (4000, 4000));
    // `close`: 5 cells at 10 (5 × 1233 × 10 / 2048 = 30.10) hold in 40 less a terminal, 32: nothing grows.
    assert_eq!(size_of("e3").width, 4000);
    // `query_f64`: four lines, 8 + 3 × 8 + 8 = 40; 9 cells do not hold in 5, `query` / `f64` do.
    let raw = size_of("raw");
    assert_eq!((raw.width, raw.height), (4000, 4000));
    assert_eq!(titles(&raw), ["query", "f64"]);
    // The band of `dev`: 12 high, above.
    assert_eq!(raw.above, 1200);
    // A band wider than the block, which `bands` would have cut, widens nothing.
    let wide = labcraft_types::text::ShownText::plain("dmm_driver");
    let node = graph.nodes.iter().find(|node| node.key == "dmm").unwrap();
    assert_eq!(sizer.size(node, Some(&wide), None).width, 4000);
}

/// Notation 3.2: a name on one line when it holds; else cut at an underscore when both parts hold, where the longer line
/// is shortest; else the first line as far as the last underscore that it holds, or as far as it holds, and the rest
/// cut by `…`.
#[test]
fn a_name_holds_on_two_lines_of_the_block() {
    // 40 less a terminal of 8: 32. At 10 a cell is 1233 × 10 / 2048 = 6.02: 5 cells are 30.10, 6 are 36.12.
    assert_eq!(crate::size::name_cells(), 5);
    let cut_of = |name: &str| -> Vec<String> {
        let source = format!("labcraft 1\n\nfn main(x: f64) -> (y: f64) {{\n    y = {name}(x)\n}}\n");
        let callee = format!("labcraft 1\n\nfn {name}(x: f64) -> (y: f64) {{\n    y = x\n}}\n");
        let (checked, names) = checked(&[("main.lcf", &source), (&format!("{name}.lcf"), &callee)]);
        let graph = graph(&checked, &names, "main.lcf");
        titles(&Sizer { names: &names, standard_everywhere: false }.size(&graph.nodes[0], None, None))
    };
    assert_eq!(cut_of("open"), ["open"]);
    assert_eq!(cut_of("a_b"), ["a_b"]);
    // Both parts hold: `query` (5) and `f64` (3).
    assert_eq!(cut_of("query_f64"), ["query", "f64"]);
    // No part pair holds: the first line takes `bench`, the underscore after it is the cut, `selftest` is cut to 4 and `…`.
    assert_eq!(cut_of("bench_selftest"), ["bench", "self\u{2026}"]);
    // `read_` holds: cut at its underscore.
    assert_eq!(cut_of("read_dc_volts"), ["read", "dc_v\u{2026}"]);
    assert_eq!(cut_of("count_to_fixed"), ["count", "to_f\u{2026}"]);
    // No underscore in what holds: in the middle of the word.
    assert_eq!(cut_of("elapsed"), ["elaps", "ed"]);
    assert_eq!(cut_of("abcdefghijklm"), ["abcde", "fghi\u{2026}"]);
    // An underscore that would leave a line empty is no cut.
    assert_eq!(cut_of("abcdefgh_"), ["abcde", "fgh_"]);
    assert_eq!(cut_of("abcd_e_"), ["abcd", "e_"]);
    // Two cuts whose longer line is 5: the first.
    assert_eq!(cut_of("ab_cd_ef"), ["ab", "cd_ef"]);
}

/// Notation 3.1, 3.2: an optional input of a primitive left to its default has the family of its table, so that an `err`
/// keeps the bottom line whatever the call wires; a variadic primitive given no value has no input.
#[test]
fn a_primitive_has_its_size_whatever_a_call_wires() {
    let source = "labcraft 1\n\nuse errors\nuse time\n\nfn main(e: error) -> (a: error, b: error, c: error) {\n    a = time.wait(1 s)\n    b = time.wait(1 s, err: e)\n    c = errors.merge()\n}\n";
    let (checked, names) = checked(&[("main.lcf", source)]);
    let graph = graph(&checked, &names, "main.lcf");
    let sizer = Sizer { names: &names, standard_everywhere: false };
    let of = |key: &str| graph.nodes.iter().find(|node| node.key == key).unwrap();
    for key in ["a", "b"] {
        assert_eq!(of(key).inputs.iter().map(|input| input.family.base).collect::<Vec<_>>(), [Base::Number, Base::Error], "{key}");
        let sized = sizer.size(of(key), None, None);
        // Two lines, `duration` alone, then the `err` pair: 8 above the bottom of 40, at 32, and 8 above it, at 24.
        assert_eq!((sized.inputs.clone(), sized.outputs.clone()), (vec![(0, 2400), (0, 3200)], vec![(4000, 3200)]), "{key}");
    }
    assert!(of("c").inputs.is_empty());
}

/// Notation 3.2: lines counted from the bottom, 8 above the lower edge and `node.pitch` apart, whatever they carry; past
/// four lines, the block grows down, never across.
#[test]
fn a_block_grows_by_its_lines_only() {
    let size_of = |count: usize| {
        let params: Vec<String> = (0..count).map(|index| format!("p{index}: f64")).collect();
        let callee = format!("labcraft 1\n\nfn many({}) -> (y: f64) {{\n    y = p0\n}}\n", params.join(", "));
        let source = format!("labcraft 1\n\nfn main(x: f64) -> (y: f64) {{\n    y = many({})\n}}\n", vec!["x"; count].join(", "));
        let (checked, names) = checked(&[("main.lcf", &source), ("many.lcf", &callee)]);
        let graph = graph(&checked, &names, "main.lcf");
        Sizer { names: &names, standard_everywhere: false }.size(&graph.nodes[0], None, None)
    };
    let four = size_of(4);
    // Four lines 8 apart need 8 + 24 + 8 = 40: the square.
    assert_eq!((four.width, four.height), (4000, 4000));
    let inputs: Vec<(i64, i64)> = four
        .shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::Circle { x: 0, y, radius, .. } => Some((*y, *radius)),
            _ => None,
        })
        .collect();
    // From the bottom: 40 - 8 = 32, then 24, 16, 8; a terminal of 8 has a radius of 4.
    assert_eq!(inputs, [(800, 400), (1600, 400), (2400, 400), (3200, 400)]);
    // Five lines need 8 + 32 + 8 = 48.
    let five = size_of(5);
    assert_eq!((five.width, five.height), (4000, 4800));
}

/// Notation 3.2: two pills less than a pill apart step outward, the lower one past the upper one, 8 from it, and the
/// constant so moved draws a wire to its terminal; a pill 16 below the first one keeps its place.
#[test]
fn neighbouring_pills_step_outward() {
    let callee = "labcraft 1\n\nfn three(a: f64, b: f64, c: f64) -> (y: f64) {\n    y = a\n}\n";
    let source = "labcraft 1\n\nfn main() -> (y: f64) {\n    y = three(1.0, 2.0, 3.0)\n}\n";
    let (checked, names) = checked(&[("main.lcf", source), ("three.lcf", callee)]);
    let graph = graph(&checked, &names, "main.lcf");
    let sized = Sizer { names: &names, standard_everywhere: false }.size(&graph.nodes[0], None, None);
    // Lines at 16, 24 and 32. A pill of 3 cells at 10, 3 × 1233 × 10 / 2048 = 18.06, and 4 on each side: 26.06. Its right
    // end 8 left of the half terminal: -12; it starts at -38.06. The second, 8 lower, under 14, ends 8 left of that,
    // -46.06, and starts at -72.12. The third is 8 below the second, whose right end is 8 left of its own start: it keeps
    // its place.
    assert_eq!(sized.reaches, [Some(3806), Some(7212), Some(3806)]);
    assert_eq!(sized.left, 7212);
    let leads: Vec<&Vec<(i64, i64)>> = sized
        .shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::Poly { points, closed: false, .. } => Some(points),
            _ => None,
        })
        .collect();
    assert_eq!(leads, [&vec![(-4606, 2400), (-400, 2400)]]);
}

/// Notation 3.2: a circle of 28; two inputs at 8 above and below the middle, on the edge; `select` with two constants
/// stretches into a capsule for them to be 24 apart, and 16 from the wired input beside them.
#[test]
fn a_primitive_is_a_circle() {
    let source = "labcraft 1\n\nuse core\n\nfn main(a: f64, b: f64, ok: bool) -> (y: f64, z: f64) {\n    y = core.add(a, b)\n    z = core.select(ok, 1.0, 0.0)\n}\n";
    let (checked, names) = checked(&[("main.lcf", source)]);
    let graph = graph(&checked, &names, "main.lcf");
    let sizer = Sizer { names: &names, standard_everywhere: false };
    let add = sizer.size(&graph.nodes[0], None, None);
    assert_eq!((add.width, add.height), (2800, 2800));
    // At 8 from the middle of a circle of radius 14: 14 - √(14² - 8²) = 14 - √132 = 14 - 11.489 = 2.511; the integer
    // root of 1,320,000 hundredths² is 1148, so 252.
    let inputs: Vec<(i64, i64)> = add
        .shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::Circle { x, y, radius: 400, .. } if *x < 1400 => Some((*x, *y)),
            _ => None,
        })
        .collect();
    assert_eq!(inputs, [(252, 600), (252, 2200)]);
    // `select`: -10, 0, 10; 16 between `ok` and the first constant, which goes to 6; 24 between the two constants, the
    // second at 30; a span of 40 and a pill of 14 make 54.
    let select = sizer.size(&graph.nodes[1], None, None);
    assert_eq!((select.width, select.height), (2800, 5400));
}

/// Decided at step 2 of M0v: 24 for the header, the first line at 40, lines 24 apart, the last 16 above the bottom, 12
/// on each side, 24 between the names, 112 at least.
#[test]
fn a_standard_block_has_the_numbers_of_step_2() {
    let (checked, names) = bench();
    let graph = graph(&checked, &names, "main.lcf");
    let sizer = Sizer { names: &names, standard_everywhere: true };
    let raw = graph.nodes.iter().find(|node| node.key == "raw").unwrap();
    let sized = sizer.size(raw, None, None);
    // Four lines: 40 + 3 × 24 + 16 = 128.
    assert_eq!(sized.height, 12_800);
    // At 10 a cell is 1233 × 10 / 2048. The widest line is `command`, alone, with its pill: 12, then 7 cells (4214), 8,
    // the pill of 15 cells (9031) and 4 on each side, then 12: 17,245. `session` and `session`: 12 + 4214 + 24 + 4214 +
    // 12 = 13,228. The header `dev.query_f64`, 13 cells (7827), with 12 on each side: 10,227. So 17,245, up to 17,600.
    assert_eq!(sized.width, 17_600);
    let close = graph.nodes.iter().find(|node| node.key == "e3").unwrap();
    // Two lines (`session` alone, then the `err` pair): 40 + 24 + 16 = 80; at least 112 wide.
    assert_eq!((sizer.size(close, None, None).height, sizer.size(close, None, None).width), (8000, 11_200));
}

/// Decided at step 2 of M0v: a line break of a constant is drawn `↵`, a bidirectional control as its code in
/// `mark.invalid`; in compact view a constant is cut at 8 cells.
#[test]
fn a_constant_shows_its_controls_by_their_signs() {
    let source = "labcraft 1\n\nuse string\n\nfn main(a: string) -> (y: string) {\n    y = string.concat(a, \"x\\ny\")\n}\n";
    let (checked, names) = checked(&[("main.lcf", source)]);
    let graph = graph(&checked, &names, "main.lcf");
    let node = &graph.nodes[0];
    assert_eq!(node.inputs[1].constant.as_deref(), Some("\"x\ny\""));
    let sized = Sizer { names: &names, standard_everywhere: false }.size(node, None, None);
    let pills: Vec<String> = sized
        .shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::Text { runs, x, .. } if *x < 0 => Some(runs.iter().map(|run| run.text.as_str()).collect()),
            _ => None,
        })
        .collect();
    // Left of the block, the pill: `"x↵y"` is 5 cells, under the 8 of the compact view.
    assert_eq!(pills, ["\"x\u{21B5}y\""]);
}

fn codes(drawn: &Drawn) -> Vec<Code> {
    match drawn {
        Drawn::NotDrawn(diagnostics) => diagnostics.iter().map(|diagnostic| diagnostic.code()).collect(),
        Drawn::Drawing(_) => Vec::new(),
    }
}

/// `LC7002` counts the nodes before anything is built (cli.md 7.6).
#[test]
fn a_function_with_too_many_nodes_is_not_drawn() {
    let (checked, names) = bench();
    let file = index_of(&checked, "main.lcf");
    assert_eq!(codes(&draw(&checked, file, &names, &ViewState::default(), &DiagramLimits { max_nodes: 3 })), [Code::Lc7002]);
    assert!(matches!(draw(&checked, file, &names, &ViewState::default(), &DiagramLimits { max_nodes: 4 }), Drawn::Drawing(_)));
    assert_eq!(crate::count_nodes(&checked, file, &names), Some(4));
    // A named constant that is a variant, or a function, is a node too: `a = Verdict.Pass` and `g = f`; `e = a` is not.
    let verdict = "labcraft 1\n\ntype Verdict = enum { Pass, Fail }\n";
    let f = "labcraft 1\n\nfn f(x: f64) -> (y: f64) {\n    y = x\n}\n";
    let main =
        "labcraft 1\n\nfn main() -> (v: Verdict, w: Verdict) {\n    a = Verdict.Pass\n    b = Verdict.Fail\n    g = f\n    v = a\n    w = b\n}\n";
    let (constants, names) = self::checked(&[("main.lcf", main), ("Verdict.lct", verdict), ("f.lcf", f)]);
    assert_eq!(crate::count_nodes(&constants, index_of(&constants, "main.lcf"), &names), Some(3));
}

/// `LC7002` counts the terminals of what it builds, a missing input included, and stops at the first one too many.
#[test]
fn a_function_with_too_many_terminals_is_not_drawn() {
    let five = "labcraft 1\n\nfn five(a: f64, b: f64, c: f64, d: f64, e: f64) -> (y: f64) {\n    y = a\n}\n";
    let main = "labcraft 1\n\nfn main() -> (y: f64) {\n    y1 = five()\n    y2 = five()\n    y = five()\n}\n";
    let (checked, names) = checked(&[("main.lcf", main), ("five.lcf", five)]);
    let file = index_of(&checked, "main.lcf");
    // Three nodes of five missing inputs and one output: 18 terminals, over 3 × 5 = 15, under 4 × 5 = 20.
    let drawn = draw(&checked, file, &names, &ViewState::default(), &DiagramLimits { max_nodes: 3 });
    assert_eq!(codes(&drawn), [Code::Lc7002]);
    let Drawn::NotDrawn(diagnostics) = drawn else { panic!() };
    assert!(diagnostics[0].message.contains("more than 15 terminals, counted on its first 3 nodes"), "{}", diagnostics[0].message);
    assert!(matches!(draw(&checked, file, &names, &ViewState::default(), &DiagramLimits { max_nodes: 4 }), Drawn::Drawing(_)));
    // The bench: `open` 3 inputs (`timeout` at its place) and 2 outputs, `query_f64` 3 and 3, `close` 2 and 1, `add` 2
    // and 1: 17.
    let (checked, names) = bench();
    let file = &checked.files[index_of(&checked, "main.lcf")];
    let function = function_of(file).unwrap();
    let orders = orders_of(file, function, &names);
    assert!(Builder::new(&checked, file, &names).graph(function, &orders, 17).is_ok());
    let Err(crate::graph::TooLarge(counts)) = Builder::new(&checked, file, &names).graph(function, &orders, 16) else { panic!() };
    assert_eq!((counts.nodes, counts.terminals), (4, 17));
}

/// Notation 3.1: a construction of constants is folded in the viewer, a variant among its constants; an exported image
/// shows its block (cli.md 7.2), which is the default view state.
#[test]
fn a_construction_of_constants_is_folded_only_in_the_viewer() {
    let verdict = "labcraft 1\n\ntype Verdict = enum { Pass, Fail }\n";
    let p = "labcraft 1\n\ntype P = struct { low: f64[V], v: Verdict }\n";
    let main = "labcraft 1\n\nfn main() -> (p: P) {\n    p = P { low: 1 V, v: Verdict.Pass }\n}\n";
    let (checked, names) = checked(&[("main.lcf", main), ("Verdict.lct", verdict), ("P.lct", p)]);
    let file = &checked.files[index_of(&checked, "main.lcf")];
    let function = function_of(file).unwrap();
    let orders = orders_of(file, function, &names);
    let exported = Builder::new(&checked, file, &names).graph(function, &orders, u64::MAX).unwrap();
    assert!(matches!(&exported.nodes[0].kind, Kind::Construction { ty, .. } if ty == "P"));
    assert_eq!(exported.nodes[0].inputs.iter().map(|input| input.name.as_str()).collect::<Vec<_>>(), ["low", "v"]);
    let viewer = Builder::new(&checked, file, &names).folding(true).graph(function, &orders, u64::MAX).unwrap();
    assert!(matches!(&viewer.nodes[0].kind, Kind::Constant { name, value } if name == "p" && value == "P {\u{2026}}"));
}

/// Notation 3.2: the name is in the centre of the block, which a band never widens.
#[test]
fn a_name_is_centred_on_its_block() {
    let (checked, names) = bench();
    let graph = graph(&checked, &names, "main.lcf");
    let node = graph.nodes.iter().find(|node| node.key == "dmm").unwrap();
    let wide = labcraft_types::text::ShownText::plain("dmm_driver");
    let sized = Sizer { names: &names, standard_everywhere: false }.size(node, Some(&wide), None);
    // 40 wide; `open`, 4 cells at 10, 4 × 1233 × 10 / 2048 = 24.08: it starts at 20 - 12.04.
    let starts: Vec<i64> = sized
        .shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::Text { x, runs, .. } if runs.iter().all(|run| run.token == "node.title") => Some(*x),
            _ => None,
        })
        .collect();
    assert_eq!(starts, [796]);
}

const ICON_OF_F: &str = "labcraft 1

fn f(x: f64) -> (y: f64) {
    y = x
}

icon 32 {
    rect (1.5, 2) (30, 3e1)
    circle (16, 16) 1_0
    path \"M1 1 L2.5 2.5 L3.5 3.5 L4.5 4.5 L5.5 5.5 L6.5 6.5 L7.5 7.5 L8.5 8.5 L9.5 9.5 L10.5 10.5 L11.5 11.5 L12.5 12.5 L13.5 13.5 L14.5 14.5 L15.5 15.5 L16.5 16.5 L17.5 17.5 L18.5 18.5 L19.5 19.5 L20.5 20.5 L21.5 21.5 L22.5 22.5 L23.5 23.5 L24.5 24.5 L25.5 25.5 L26.5 26.5 L27.5 27.5 L28.5 28.5 L29.5 29.5 L30.5 30.5 Z\"
}
";

/// An icon is in the SVG once, however many calls show it, in the numbers its file writes, without an exponent; its path
/// as written, whatever its length (spec 11.1).
#[test]
fn an_icon_is_written_once() {
    let main = "labcraft 1\n\nfn main(x: f64) -> (y: f64, z: f64) {\n    y = f(x)\n    z = f(x)\n}\n";
    let (checked, names) = checked(&[("main.lcf", main), ("f.lcf", ICON_OF_F)]);
    let file = index_of(&checked, "main.lcf");
    let Drawn::Drawing(drawing) = draw(&checked, file, &names, &ViewState::default(), &DiagramLimits::default()) else { panic!() };
    assert_eq!(drawing.icons.len(), 1);
    let text = svg(&drawing, crate::Theme::Both);
    assert_eq!(text.matches("<symbol ").count(), 1);
    assert_eq!(text.matches("<use href=\"#icon-").count(), 2);
    // From (1.5, 2) to (30, 30): 28.5 wide, 28 high.
    assert!(text.contains("<rect x=\"1.5\" y=\"2\" width=\"28.5\" height=\"28\""), "{text}");
    assert!(text.contains("<circle cx=\"16\" cy=\"16\" r=\"10\""), "{text}");
    let path = ICON_OF_F.split('"').nth(1).unwrap();
    assert!(path.chars().count() > 200);
    assert!(text.contains(&format!("d=\"{path}\"")));
}

/// A library `dev`, which `use` finds and `check` does not check, with these files; and the graph of a `main` that calls
/// each of its functions `a`, `f`, `g`, whichever it has.
struct Dev(Vec<(&'static str, String)>);

impl Libraries for Dev {
    fn find(&self, name: &str, _: Origin) -> Found {
        if name != "dev" {
            return Found::NotFound;
        }
        let mut library = LibrarySource::new("dev".to_owned(), "dev".to_owned(), Origin::Project);
        library.files.extend(self.0.iter().map(|(shown, text)| source(shown, text)));
        Found::Library(library)
    }
}

/// The icon of each call of `main`, by the name of its function, as many shapes as it draws; `None` for the label.
fn icons_through(dev: &Dev, calls: &str) -> Vec<(String, Option<usize>)> {
    let main = format!("labcraft 1\n\nuse dev\n\nfn main(x: f64) -> () {{\n{calls}}}\n");
    let mut names = Interner::new();
    let mut root = LibrarySource::new(String::new(), String::new(), Origin::Project);
    root.files.push(source("main.lcf", &main));
    let checked = check(root, dev, &mut names, &Limits::default());
    // Only the folder given is kept: the files of `dev`, which hold the icons, are not.
    assert_eq!(checked.files.len(), 1);
    let graph = graph(&checked, &names, "main.lcf");
    let mut icons: Vec<(String, Option<usize>)> = graph
        .nodes
        .iter()
        .filter_map(|node| match &node.kind {
            Kind::Call { qualified, icon, .. } => Some((qualified.clone(), icon.map(|icon| icon.items.len()))),
            _ => None,
        })
        .collect();
    icons.sort();
    icons
}

fn function_with(name: &str, icon: &str) -> String {
    format!("labcraft 1\n\nfn {name}(x: f64) -> (y: f64) {{\n    y = x\n}}\n{icon}")
}

/// Spec 11.1: each call shows the icon of its function, from whatever library: one found by `use`, which is not checked
/// and whose files are not kept, too (`visa`, a `--lib-path`). Each icon goes to the function of its own file, among
/// several, before and after it in the order of the files: `a` has none, `f` three shapes, `g` one.
#[test]
fn a_call_shows_the_icon_of_a_function_of_another_library() {
    let dev = Dev(vec![
        ("a.lcf", function_with("a", "")),
        ("f.lcf", ICON_OF_F.to_owned()),
        ("g.lcf", function_with("g", "\nicon 32 {\n    circle (16, 16) 8\n}\n")),
    ]);
    let icons = icons_through(&dev, "    p = dev.a(x)\n    q = dev.f(x)\n    r = dev.g(x)\n");
    assert_eq!(icons, [("dev.a".to_owned(), None), ("dev.f".to_owned(), Some(3)), ("dev.g".to_owned(), Some(1))]);
}

/// Spec 11.1: an icon that `check` refuses (`LC5008`), or one past its bounds (256 shapes, 64 KiB or 4 096 segments of
/// path), gives way to the label at every call, from a library that is not checked too, which nobody judged.
#[test]
fn a_call_shows_the_label_for_an_icon_refused() {
    let shapes = |count: usize| format!("\nicon 32 {{\n{}}}\n", "    circle (16, 16) 8\n".repeat(count));
    let path = |path: String| format!("\nicon 32 {{\n    path \"{path}\"\n}}\n");
    for (icon, shown) in [
        (shapes(256), Some(256)),
        (shapes(257), None),
        ("\nicon 0 {\n    circle (16, 16) 8\n}\n".to_owned(), None),
        (path("M0 0 Q".to_owned()), None),
        ("\nicon 32 {\n    circle (16, 16) nan\n}\n".to_owned(), None),
        ("\nicon 32 {\n    circle (16, 16) 0\n}\n".to_owned(), None),
        ("\nicon 32 {\n    circle (16, 16) 8 accent wire.nothing\n}\n".to_owned(), None),
        (path(format!("M0 0{}", " L1 1".repeat(4095))), Some(1)),
        (path(format!("M0 0{}", " L1 1".repeat(4096))), None),
        (path(format!("M0 0{}", " ".repeat(65_536))), None),
    ] {
        let dev = Dev(vec![("f.lcf", function_with("f", &icon))]);
        assert_eq!(icons_through(&dev, "    y = dev.f(x)\n"), [("dev.f".to_owned(), shown)], "{icon:.80}");
    }
}

/// A coordinate far off the grid is drawn as a million, without an exponent: the icon is cut to its grid anyway.
#[test]
fn an_icon_number_has_no_exponent() {
    // In hundredths, as every number of the SVG (cli.md 7.2).
    assert_eq!(crate::size::icon_number("3e1", false), 3000);
    assert_eq!(crate::size::icon_number("1_000.5", true), -100_050);
    assert_eq!(crate::size::icon_number("1e300", false), 100_000_000);
    assert_eq!(crate::size::icon_number("0.004", false), 0);
    assert_eq!(crate::size::icon_number("0.006", false), 1);
}

/// More than three inputs on a circle, an error that is still drawn, stretch it so that each input is on its edge.
#[test]
fn a_circle_holds_all_its_inputs() {
    let source = "labcraft 1\n\nuse core\n\nfn main(a: f64, b: f64) -> (y: f64) {\n    y = core.add(a, b, a, b, a)\n}\n";
    let (checked, names) = checked(&[("main.lcf", source)]);
    let graph = graph(&checked, &names, "main.lcf");
    let sized = Sizer { names: &names, standard_everywhere: false }.size(&graph.nodes[0], None, None);
    // Five inputs 8 apart: a span of 32, and a pill of 14: 46 high; their centres from 7 to 39.
    assert_eq!(sized.height, 4600);
    let ys: Vec<i64> = sized
        .shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::Circle { y, radius: 400, x, .. } if *x < 1400 => Some(*y),
            _ => None,
        })
        .collect();
    assert_eq!(ys, [700, 1500, 2300, 3100, 3900]);
}

/// Two libraries whose bands would read the same show their full path, and cut in the middle when they still do
/// (notation 3.2).
#[test]
fn bands_that_read_the_same_show_more() {
    let f = "labcraft 1\n\nfn f(x: f64) -> (y: f64) {\n    y = x\n}\n";
    let main = "labcraft 1\n\nuse measurement_a\nuse measurement_b\n\nfn main(x: f64) -> (y: f64, z: f64) {\n    y = measurement_a.f(x)\n    z = measurement_b.f(x)\n}\n";
    let (checked, names) = checked(&[("main.lcf", main), ("measurement_a/f.lcf", f), ("measurement_b/f.lcf", f)]);
    let graph = graph(&checked, &names, "main.lcf");
    let bands: Vec<String> = crate::bands(&graph).into_iter().map(|band| band.unwrap().text()).collect();
    // Above a block of 40: 40 less 4 on each side, 32; at 9 a cell is 1233 × 9 / 2048 = 5.42, 5 cells are 27.09, 6 are
    // 32.51: 5 cells. `meas…` for both; the full path is the same; in the middle, 2 cells, `…`, 2 cells.
    assert_eq!(bands, ["me\u{2026}_a", "me\u{2026}_b"]);
}

/// Notation 3.2: a band is cut to the width of its block; an oblong is as wide as its label, so its band shows more.
#[test]
fn a_band_is_cut_to_the_width_of_its_block() {
    let ty = "labcraft 1\n\ntype StepResult = struct { name: string, ok: bool }\n";
    let f = "labcraft 1\n\nfn f(x: f64) -> (y: f64) {\n    y = x\n}\n";
    let main = "labcraft 1\n\nuse results_log\n\nfn main(x: f64, s: string, b: bool) -> (y: f64, r: results_log.StepResult) {\n    y = results_log.f(x)\n    r = results_log.StepResult { name: s, ok: b }\n}\n";
    let (checked, names) = checked(&[("main.lcf", main), ("results_log/StepResult.lct", ty), ("results_log/f.lcf", f)]);
    assert_eq!(checked.report.errors(), 0, "{:?}", checked.report.diagnostics());
    let graph = graph(&checked, &names, "main.lcf");
    let bands = crate::bands(&graph);
    let band_of = |key: &str| {
        let index = graph.nodes.iter().position(|node| node.key == key).unwrap();
        bands[index].as_ref().unwrap().text()
    };
    // A block of 40 holds 5 cells at 9 (above): `resu…`.
    assert_eq!(band_of("y"), "resu\u{2026}");
    // `StepResult`, 10 cells at 10 (60.21) and 2 on each side, 64.21: 72 on the grid. 72 less 8 is 64, which holds 11
    // cells at 9 (59.60), not 12 (65.02): `results_log` whole.
    assert_eq!(band_of("r"), "results_log");
}

/// A terminal shows the type of its parameter, not that of its argument: `timeout: duration` given `5 s` has no unit
/// badge, and `x: f64[V]` given `5 mV` shows `V` (the conversion is on the wire, notation 4.2).
#[test]
fn a_terminal_has_the_type_of_its_parameter() {
    let take = "labcraft 1\n\nfn take(x: f64[V], timeout: duration) -> (y: f64[V]) {\n    y = x\n}\n";
    let main = "labcraft 1\n\nfn main() -> (y: f64[V]) {\n    y = take(5 mV, 5 s)\n}\n";
    let (checked, names) = checked(&[("main.lcf", main), ("take.lcf", take)]);
    let graph = graph(&checked, &names, "main.lcf");
    let units: Vec<Option<&str>> = graph.nodes[0].inputs.iter().map(|input| input.unit.as_deref()).collect();
    assert_eq!(units, [Some("V"), None]);
}

/// The SVG names the function, its schema and each node, and its font, with the theme as CSS variables.
#[test]
fn the_svg_names_what_it_draws() {
    let (checked, names) = bench();
    let file = index_of(&checked, "main.lcf");
    let Drawn::Drawing(drawing) = draw(&checked, file, &names, &ViewState::default(), &DiagramLimits::default()) else { panic!() };
    let text = svg(&drawing, crate::Theme::Both);
    assert!(text.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""));
    assert!(text.contains(" font-family=\"Hack, DejaVu Sans Mono, Menlo, monospace\" data-schema=\"labcraft-diagram/1\" data-function=\"main\">"));
    assert!(text.contains("<g data-node=\"e3\"><title>dev.close</title>"));
    assert!(text.contains("@media (prefers-color-scheme: dark)"));
    assert!(text.contains("--wire-error: #AD3A0B;"));
    assert!(!svg(&drawing, crate::Theme::Dark).contains("@media"));
    // Nothing comes from the order in which names were read: another check, with names interned in another order first,
    // gives the same bytes.
    let mut other_names = Interner::new();
    for name in ["zz", "close", "e3", "dmm2", "value"] {
        other_names.intern(name);
    }
    let other = checked_with(&BENCH, &mut other_names);
    let Drawn::Drawing(again) = draw(&other, index_of(&other, "main.lcf"), &other_names, &ViewState::default(), &DiagramLimits::default()) else {
        panic!()
    };
    assert_eq!(text, svg(&again, crate::Theme::Both));
}

/// A missing input keeps the place of its parameter: `three(b: x)` has `a`, `b`, `c`, the first and last missing.
#[test]
fn a_missing_input_keeps_its_place() {
    let three = "labcraft 1\n\nfn three(a: f64, b: f64, c: f64) -> (y: f64) {\n    y = a\n}\n";
    let main = "labcraft 1\n\nfn main(x: f64) -> (y: f64) {\n    y = three(b: x)\n}\n";
    let (checked, names) = checked(&[("main.lcf", main), ("three.lcf", three)]);
    let graph = graph(&checked, &names, "main.lcf");
    assert_eq!(graph.nodes[0].inputs.iter().map(|input| input.name.as_str()).collect::<Vec<_>>(), ["a", "b", "c"]);
}

/// A function called alone has the outputs of its signature, and `core.call` written alone those of the function it calls
/// (`Resolved::outputs_of`).
#[test]
fn a_call_written_alone_has_the_outputs_of_its_callee() {
    let f0 = "labcraft 1\n\nfn f0(x: f64) -> (y: f64) {\n    y = x\n}\n";
    let main = "labcraft 1\n\nuse core\n\nfn main(x: f64) -> (y: f64) {\n    core.call(f0, x)\n    f0(x)\n    y = x\n}\n";
    let (checked, names) = checked(&[("main.lcf", main), ("f0.lcf", f0)]);
    assert!(checked.report.diagnostics().is_empty());
    let graph = graph(&checked, &names, "main.lcf");
    let outputs: Vec<Vec<Base>> = graph.nodes.iter().map(|node| node.outputs.iter().map(|output| output.family.base).collect()).collect();
    assert_eq!(outputs, [vec![Base::Number], vec![Base::Number]]);
}

/// A `match` unfolded is a frame with its header, cut at 40 cells, and a box per arm titled by its patterns, `_` written
/// `other` (notation 5.2).
#[test]
fn a_match_is_a_frame_with_its_arms() {
    let main = "labcraft 1\n\nfn main(c: i32) -> (v: f64) {\n    v = match c {\n        1 => yield 1.0\n        _ => yield 2.0\n    }\n}\n";
    let long =
        main.replace("(c: i32)", "(a_very_long_name_for_a_channel_number: i32)").replace("match c", "match a_very_long_name_for_a_channel_number");
    let (checked, names) = checked(&[("main.lcf", &long)]);
    let file = index_of(&checked, "main.lcf");
    let Drawn::Drawing(drawing) = draw(&checked, file, &names, &ViewState::default(), &DiagramLimits::default()) else { panic!() };
    let group = drawing.groups.iter().find(|group| group.node == "v").unwrap();
    let texts: Vec<String> = group
        .shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::Text { runs, .. } => Some(runs.iter().map(|run| run.text.as_str()).collect()),
            _ => None,
        })
        .collect();
    // `match a_very_long_name_for_a_channel_number` is 44 cells: 39 of them and `…`.
    assert_eq!(texts[0], "match a_very_long_name_for_a_channel_nu\u{2026}");
    // Each arm a group of its own, `data-arm`, written as its tab (cli.md 7.2).
    let arms: Vec<&str> = drawing.groups.iter().filter_map(|group| group.arm.as_deref()).collect();
    assert_eq!(arms, ["1", "other"]);
    let svg = svg(&drawing, crate::Theme::Light);
    assert!(svg.contains("<g data-arm=\"1\">") && svg.contains("<g data-arm=\"other\">"));
}

/// The grid of an icon is a number of the SVG like the others, in its shortest form; a grid not written `32` (`3_2`),
/// which `check` refuses (`LC5008`), gives way to the label (11.1).
#[test]
fn the_grid_of_an_icon_is_written_in_its_shortest_form() {
    let main = "labcraft 1\n\nfn main(x: f64) -> (y: f64) {\n    y = f(x)\n}\n";
    for (grid, symbols) in [("icon 32", 1), ("icon 3_2", 0)] {
        let f = ICON_OF_F.replace("icon 32", grid);
        let (checked, names) = checked(&[("main.lcf", main), ("f.lcf", &f)]);
        let Drawn::Drawing(drawing) = draw(&checked, index_of(&checked, "main.lcf"), &names, &ViewState::default(), &DiagramLimits::default()) else {
            panic!()
        };
        let text = svg(&drawing, crate::Theme::Both);
        assert_eq!(text.matches("viewBox=\"0 0 32 32\" overflow=\"hidden\">").count(), symbols, "{grid}");
    }
}

/// A value too many for a variant, which `check` refuses, is still drawn, by its position, as an argument too many for a
/// function (decided at step 2 of M0v).
#[test]
fn a_value_too_many_for_a_variant_is_drawn() {
    let v = "labcraft 1\n\ntype V = enum { A, B(f64) }\n";
    let main = "labcraft 1\n\nfn main(x: f64, z: f64) -> (v: V) {\n    v = V.B(x, z)\n}\n";
    let (checked, names) = checked(&[("main.lcf", main), ("V.lct", v)]);
    assert!(!checked.report.diagnostics().is_empty());
    let graph = graph(&checked, &names, "main.lcf");
    assert_eq!(graph.nodes[0].inputs.iter().map(|input| input.name.as_str()).collect::<Vec<_>>(), ["0", "1"]);
}

// Placement (notation 2.1, and the numbers decided at step 3 of M0v: 24 between stacked nodes, 40 above the axis for a
// column with no node of the chain, a frame at the margin of 16).

fn placed_of(checked: &Checked, names: &Interner, shown: &str) -> std::collections::HashMap<String, crate::Placed> {
    let Drawn::Drawing(drawing) = draw(checked, index_of(checked, shown), names, &ViewState::default(), &DiagramLimits::default()) else { panic!() };
    drawing.placed.into_iter().map(|placed| (placed.key.clone(), placed)).collect()
}

const STEP: &str = "labcraft 1

extern rust(crate: \"labcraft-dev\", function: \"step\")
fn step(v: f64, err: error = none) -> (w: f64, err: error)
";

/// A node stands in the column after the last node that feeds it; one fed only by the inputs of the function in the
/// first; a node with no input just before its leftmost reader, or in the last column when nothing reads it.
#[test]
fn a_node_stands_after_what_feeds_it() {
    let main = "labcraft 1\n\nuse core\n\nfn main(x: f64) -> (y: f64, z: f64) {\n    a = core.neg(x)\n    k = 2.0\n    b = core.mul(a, k)\n    c = core.abs(x)\n    y = core.add(b, c)\n    z = core.neg(c)\n    u = 5.0\n}\n";
    let (checked, names) = checked(&[("main.lcf", main)]);
    let placed = placed_of(&checked, &names, "main.lcf");
    let columns: Vec<(&str, u32)> = ["a", "k", "b", "c", "y", "z", "u"].iter().map(|key| (*key, placed[*key].column)).collect();
    assert_eq!(columns, [("a", 0), ("k", 0), ("b", 1), ("c", 0), ("y", 2), ("z", 1), ("u", 2)]);
}

/// A node with no input whose reader is in the first column opens a column before it.
#[test]
fn a_constant_before_the_first_column_opens_one() {
    let main = "labcraft 1\n\nuse core\n\nfn main(x: f64) -> (y: f64) {\n    m = 3.0\n    y = core.add(m, x)\n}\n";
    let (checked, names) = checked(&[("main.lcf", main)]);
    let placed = placed_of(&checked, &names, "main.lcf");
    assert_eq!((placed["m"].column, placed["y"].column), (0, 1));
}

/// The error chain on one line: its nodes have their error pair on the axis; the others stand above, 24 apart, and a
/// column with no node of the chain starts 40 above the axis.
#[test]
fn the_error_chain_is_one_straight_line() {
    let main = "labcraft 1\n\nuse core\n\nfn main(x: f64, err: error = none) -> (y: f64, err: error) {\n    a, e1 = step(x, err: err)\n    b, e2 = step(a, err: e1)\n    c = core.neg(x)\n    d = core.add(b, c)\n    y, e3 = step(d, err: e2)\n    err = e3\n}\n";
    let (checked, names) = checked(&[("main.lcf", main), ("step.lcf", STEP)]);
    assert_eq!(checked.report.errors(), 0, "{:?}", checked.report.diagnostics());
    let placed = placed_of(&checked, &names, "main.lcf");
    let (a, b, y, c, d) = (&placed["a"], &placed["b"], &placed["y"], &placed["c"], &placed["d"]);
    // `step` is a block of 40. Its lines are 8 apart, the last 8 from the bottom, at 24 and 32: its error pair at 32 from
    // its top.
    assert_eq!((a.height, b.y, y.y), (4000, a.y, a.y));
    assert_eq!((a.column, b.column, d.column, y.column), (0, 1, 2, 3));
    let axis = a.y + 3200;
    // `c`, a circle of 28, in the column of `a`: 24 above it.
    assert_eq!(c.y, a.y - 2400 - 2800);
    // `d` is alone in its column: its bottom 40 above the axis.
    assert_eq!(d.y + 2800, axis - 4000);
}

/// A node that the `layout` section places is there, from the top-left corner of the frame of the function; a node
/// that would cover it moves up out of its way, 24 above it.
#[test]
fn a_placed_node_keeps_its_place() {
    let main = "labcraft 1\n\nuse core\n\nfn main(x: f64) -> (y: f64) {\n    a = core.neg(x)\n    y = core.abs(x)\n}\n\nlayout {\n    node y at (80, 80)\n}\n";
    let (checked, names) = checked(&[("main.lcf", main)]);
    let placed = placed_of(&checked, &names, "main.lcf");
    let (a, y) = (&placed["a"], &placed["y"]);
    // `a` stands in the first column: after the name `x` (one cell at 10, 6.02), 12 before it and 8 after it, the
    // narrowest channel, 16, its wire being straight, and the half terminal of 4: 62.02 from the corner of the frame.
    // It reaches over `y`, at 80, so it
    // goes up, 24 above it: at 80 - 24 - 28 = 28 from the corner, higher than the header and its margin (48). The frame
    // grows up by 20, 24 on the grid, so that it holds it: its top is at the margin, 16, its corner 24 lower, at 40.
    // Across, the margin of 16 is around everything: the terminal of `x` reaches 4 left of the frame, at 20.
    assert_eq!((y.x, y.y), (2000 + 8000, 4000 + 8000));
    assert_eq!((a.x, a.y), (2000 + 1200 + 602 + 800 + 1600 + 400, 4000 + 2800));
}

/// The terminals of a frame, from the top: each where it prefers, 24 below the one before, the pinned one on the axis.
#[test]
fn the_terminals_of_a_frame_keep_their_order() {
    let heights = crate::place::resolve(&[Some(-5000), Some(-4900), None, Some(0)], &[false, false, false, true], &[None; 4]);
    // Down: -5000, then -4900 is pushed to -2600, the third goes 24 after it to -200, the last stays on the axis. Up:
    // -200 rises to -2400, -2600 to -4800, -5000 to -7200.
    assert_eq!(heights, [-7200, -4800, -2400, 0]);
}

/// The same file gives the same placement, whatever the order in which names were interned.
#[test]
fn placement_does_not_depend_on_the_interner() {
    let main = "labcraft 1\n\nuse core\n\nfn main(x: f64, err: error = none) -> (y: f64, err: error) {\n    a, e1 = step(x, err: err)\n    b, e2 = step(a, err: e1)\n    c = core.neg(x)\n    d = core.add(b, c)\n    y, e3 = step(d, err: e2)\n    err = e3\n}\n";
    let files = [("main.lcf", main), ("step.lcf", STEP)];
    let (checked, names) = checked(&files);
    let mut other = Interner::new();
    for name in ["zz", "e3", "d", "c", "b", "a", "y", "x"] {
        other.intern(name);
    }
    let checked_other = checked_with(&files, &mut other);
    let one = draw(&checked, index_of(&checked, "main.lcf"), &names, &ViewState::default(), &DiagramLimits::default());
    let two = draw(&checked_other, index_of(&checked_other, "main.lcf"), &other, &ViewState::default(), &DiagramLimits::default());
    assert_eq!(one, two);
}

/// The chain goes through a loop on one straight line: the error register of the loop is on the axis, and the chain
/// inside goes on along it (notation 2.1, 5.1).
#[test]
fn the_chain_goes_straight_through_a_loop() {
    let main = "labcraft 1\n\nfn main(x: f64, err: error = none) -> (y: f64, err: error) {\n    a, e1 = step(x, err: err)\n    w, e3 = loop\n        carry v = a,\n        carry e = e1\n    {\n        b, e2 = step(v, err: e)\n        next v = b\n        next e = e2\n        until true\n    }\n    y, e4 = step(w, err: e3)\n    err = e4\n}\n";
    let (checked, names) = checked(&[("main.lcf", main), ("step.lcf", STEP)]);
    assert_eq!(checked.report.errors(), 0, "{:?}", checked.report.diagnostics());
    let placed = placed_of(&checked, &names, "main.lcf");
    // Three blocks of `step`, 48 high, their error pair 40 from their top: one line, one top.
    assert_eq!((placed["w/b"].y, placed["y"].y), (placed["a"].y, placed["a"].y));
}

/// The last arm of a `match` is on the axis: the error enters and leaves the `match` on that line, even when only an
/// arm above reads it, so that the chain around stays straight (notation 5.2).
#[test]
fn the_chain_goes_straight_through_a_match() {
    let main = "labcraft 1\n\nfn main(x: f64, c: i32, err: error = none) -> (z: f64, err: error) {\n    a, e0 = step(x, err: err)\n    y, e = match c {\n        1 => {\n            v, e1 = step(a, err: e0)\n            yield (v, e1)\n        }\n        _ => yield (a, none)\n    }\n    z, e9 = step(y, err: e)\n    err = e9\n}\n";
    let (checked, names) = checked(&[("main.lcf", main), ("step.lcf", STEP)]);
    assert_eq!(checked.report.errors(), 0, "{:?}", checked.report.diagnostics());
    let placed = placed_of(&checked, &names, "main.lcf");
    assert_eq!(placed["z"].y, placed["a"].y);
}

/// Two arms can each have a line of one name: the arm is part of what follows a node (notation 2.1).
#[test]
fn a_node_is_followed_by_its_arm() {
    let main = "labcraft 1\n\nuse core\n\nfn main(x: f64, c: i32) -> (y: f64) {\n    y = match c {\n        1 => {\n            v = core.neg(x)\n            yield v\n        }\n        _ => {\n            v = core.abs(x)\n            yield v\n        }\n    }\n}\n";
    let (checked, names) = checked(&[("main.lcf", main)]);
    let placed = placed_of(&checked, &names, "main.lcf");
    assert!(placed.contains_key("y:1/v") && placed.contains_key("y:2/v"), "{:?}", placed.keys().collect::<Vec<_>>());
}

/// A value that is also the selector enters by the selector, which serves as its notch: one wire outside (notation 5).
#[test]
fn the_selector_is_the_notch_of_its_value() {
    let main = "labcraft 1\n\nfn main(c: i32) -> (y: i32) {\n    y = match c {\n        1 => yield c\n        _ => yield 0\n    }\n}\n";
    let (checked, names) = checked(&[("main.lcf", main)]);
    let graph = graph(&checked, &names, "main.lcf");
    assert_eq!(graph.nodes[0].inputs.len(), 1);
    // Outside, one wire: from the input `c` of the function to the selector.
    assert_eq!(graph.bodies[0].wires.len(), 2, "{:?}", graph.bodies[0].wires);
}

/// `LC7002` counts the notches: an input of the structure for each outer value, and a notch in each arm that reads it.
#[test]
fn the_notches_are_terminals() {
    let main = "labcraft 1\n\nfn main(a: f64, b: f64, c: i32) -> (y: f64) {\n    y = match c {\n        1 => yield a\n        2 => yield b\n        _ => yield a\n    }\n}\n";
    let (checked, names) = checked(&[("main.lcf", main)]);
    let file = &checked.files[index_of(&checked, "main.lcf")];
    let function = function_of(file).unwrap();
    let orders = orders_of(file, function, &names);
    // The `match`: its selector and its output, 2; `a` and `b` enter it, 2 inputs; `a` has a notch in two arms, `b` in
    // one, 3: 7 in all.
    assert!(Builder::new(&checked, file, &names).graph(function, &orders, 6).is_err());
    assert!(Builder::new(&checked, file, &names).graph(function, &orders, 7).is_ok());
}

/// The drawing of `main.lcf`, which must check without a diagnostic.
fn drawing_of(main: &str) -> crate::Drawing {
    let (checked, names) = checked(&[("main.lcf", main)]);
    assert!(checked.report.diagnostics().is_empty(), "{:?}", checked.report.diagnostics());
    let Drawn::Drawing(drawing) = draw(&checked, index_of(&checked, "main.lcf"), &names, &ViewState::default(), &DiagramLimits::default()) else {
        panic!()
    };
    drawing
}

/// The wire of a value: its group.
fn wire_of<'a>(drawing: &'a crate::Drawing, value: &str) -> &'a crate::Group {
    drawing.groups.iter().find(|group| group.wire.as_ref().is_some_and(|wire| wire.value == value)).unwrap()
}

fn branches(group: &crate::Group) -> Vec<&Vec<(i64, i64)>> {
    group
        .shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::Wire { points, .. } => Some(points),
            _ => None,
        })
        .collect()
}

/// An array of wires is a node with an input per element, each reached by its wire, as `ir` lowers it (decision of the
/// owner, M0v step 4); an array of constants stays a constant.
#[test]
fn an_array_of_wires_is_a_node_with_an_input_per_element() {
    let main = "labcraft 1\n\nfn main(a: f64, b: f64) -> (y: [f64], z: [f64]) {\n    y = [a, b]\n    z = [1.0, 2.0]\n}\n";
    let (checked, names) = checked(&[("main.lcf", main)]);
    assert!(checked.report.diagnostics().is_empty(), "{:?}", checked.report.diagnostics());
    let graph = graph(&checked, &names, "main.lcf");
    let kinds: Vec<(&str, usize)> = graph
        .nodes
        .iter()
        .map(|node| match &node.kind {
            Kind::Construction { ty, .. } => (ty.as_str(), node.inputs.len()),
            Kind::Constant { .. } => ("constant", node.inputs.len()),
            _ => ("other", node.inputs.len()),
        })
        .collect();
    assert_eq!(kinds, [("[ ]", 2), ("constant", 0)]);
    let into: Vec<&str> = graph.bodies[0]
        .wires
        .iter()
        .filter(|wire| matches!(wire.to, crate::graph::End::Node { node: 0, .. }))
        .map(|wire| wire.name.as_str())
        .collect();
    assert_eq!(into, ["a", "b"]);
}

/// A wire into a node carries the conversion that `check` puts on what it reads, in a pill (notation 4.2), and its line
/// keeps its distance as a line with a constant does.
#[test]
fn a_conversion_is_a_pill_on_its_wire() {
    let f0 = "labcraft 1\n\nfn f0(v: f64[V]) -> (w: f64[V]) {\n    w = v\n}\n";
    let main = "labcraft 1\n\nfn main(x: f64[mV]) -> (y: f64[V]) {\n    y = f0(x)\n}\n";
    let (checked, names) = checked(&[("main.lcf", main), ("f0.lcf", f0)]);
    assert!(checked.report.diagnostics().is_empty(), "{:?}", checked.report.diagnostics());
    let graph = graph(&checked, &names, "main.lcf");
    let pills: Vec<(&str, bool)> = graph.pills.values().map(|pill| (pill.text.as_str(), pill.conversion)).collect();
    assert_eq!(pills, [("mV\u{2192}V", true)]);
    assert_eq!(graph.nodes[0].inputs[0].pill.as_deref(), Some("mV\u{2192}V"));
}

/// A value read by two nodes is one wire: a trunk, a junction point where a branch leaves it, the terminals it joins in
/// its attributes (cli.md 7.2).
#[test]
fn a_value_read_twice_is_one_wire_with_a_junction() {
    let main = "labcraft 1\n\nuse core\n\nfn main(x: f64) -> (a: f64, b: f64) {\n    a = core.neg(x)\n    b = core.abs(x)\n}\n";
    let drawing = drawing_of(main);
    let group = wire_of(&drawing, "x");
    let wire = group.wire.as_ref().unwrap();
    assert_eq!((wire.from.as_deref(), wire.to.len()), (Some("fn.x"), 2));
    assert_eq!(branches(group).len(), 2);
    assert_eq!(group.shapes.iter().filter(|shape| matches!(shape, Shape::Circle { .. })).count(), 1);
    // The first branch starts past the name of the input; the second is drawn from where it leaves the first, a
    // junction point, so that the trunk is drawn once.
    let found = branches(group);
    let junction = group.shapes.iter().find_map(|shape| match shape {
        Shape::Circle { x, y, .. } => Some((*x, *y)),
        _ => None,
    });
    assert_eq!(Some(found[1][0]), junction);
    let svg = crate::svg(&drawing, crate::Theme::Light);
    assert!(svg.contains("<g data-wire=\"x\" data-from=\"fn.x\" data-to=\""), "{svg}");
}

/// The waypoints of a `via` are kept, in the frame of the wire (spec 11).
#[test]
fn a_via_is_kept() {
    let main = "labcraft 1\n\nuse core\n\nfn main(x: f64) -> (y: f64) {\n    y = core.neg(x)\n}\n\nlayout {\n    wire x -> y via (100, 40)\n}\n";
    let drawing = drawing_of(main);
    let Some(Shape::Rect { x: frame, .. }) = drawing.loose.first() else { panic!() };
    let found = branches(wire_of(&drawing, "x"));
    assert!(found[0].iter().any(|&(x, _)| x == frame + 10_000), "{found:?}");
    // The wire is marked as the file's; the body is not, since no node of it is placed.
    assert!(wire_of(&drawing, "x").wire.as_ref().unwrap().via);
    assert!(!drawing.bodies[0].from_file);
}

// The view state of the viewer (notation 5.2, commands.md 6.5): arms in tabs, a structure unfolded, a node in standard
// view.

const ARMS: &str = "labcraft 1

use core

fn main(x: f64, n: i64) -> (y: f64) {
    y = match n {
        0 => yield x
        1 => {
            a = core.add(x, 1.0)
            b = core.mul(a, 2.0)
            yield b
        }
        _ => yield 0.0
    }
}
";

fn drawn_with(view: &ViewState) -> crate::Drawing {
    let (checked, names) = checked(&[("main.lcf", ARMS)]);
    assert!(checked.report.diagnostics().is_empty(), "{:?}", checked.report.diagnostics());
    let Drawn::Drawing(drawing) = draw(&checked, index_of(&checked, "main.lcf"), &names, view, &DiagramLimits::default()) else { panic!() };
    drawing
}

#[test]
fn an_export_unfolds_every_arm() {
    let drawing = drawn_with(&ViewState::default());
    assert!(drawing.arms.is_empty() && drawing.tabs.is_empty());
    assert!(drawing.groups.iter().all(|group| group.within.is_none()));
}

#[test]
fn in_tabs_the_arms_lie_on_one_another_in_a_frame_as_large_as_the_largest() {
    let unfolded = drawn_with(&ViewState::default());
    let tabs = drawn_with(&ViewState { tabs: true, ..ViewState::default() });
    let structure = tabs.placed.iter().position(|placed| !placed.holds.is_empty()).unwrap();
    let key = tabs.placed[structure].key.clone();
    assert_eq!(
        tabs.arms.iter().map(|arm| (arm.structure.as_str(), arm.arm, arm.within)).collect::<Vec<_>>(),
        [(key.as_str(), 0, None), (key.as_str(), 1, None), (key.as_str(), 2, None)]
    );
    assert_eq!(tabs.tabs.iter().map(|tab| tab.arm).collect::<Vec<_>>(), [0, 1, 2]);
    // The tabs side by side in one row, inside the frame.
    let frame = &tabs.placed[structure];
    assert!(tabs.tabs.windows(2).all(|pair| pair[0].y == pair[1].y && pair[0].x + pair[0].width < pair[1].x));
    assert!(tabs.tabs.iter().all(|tab| tab.x > frame.x && tab.x + tab.width < frame.x + frame.width));
    // Each arm its box, all from one corner, and the frame holds the largest; the nodes of the second arm are within it.
    let boxes: Vec<(i64, i64, i64, i64)> = tabs
        .groups
        .iter()
        .filter(|group| group.arm.is_some())
        .map(|group| match group.shapes.first() {
            Some(Shape::Rect { x, y, width, height, .. }) => (*x, *y, *width, *height),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(boxes.len(), 3);
    assert!(boxes.iter().all(|&(x, y, width, _)| (x, y, width) == (boxes[0].0, boxes[0].1, boxes[0].2)));
    let deepest = boxes.iter().map(|&(_, y, _, height)| y + height).max().unwrap();
    assert!(deepest < frame.y + frame.height);
    assert!(boxes[1].3 > boxes[0].3);
    let inner: Vec<&crate::Placed> = tabs.placed.iter().filter(|placed| placed.body == frame.holds[1]).collect();
    assert_eq!(inner.len(), 2);
    assert!(inner.iter().all(|placed| placed.y > boxes[1].1 && placed.y + placed.height < boxes[1].1 + boxes[1].3));
    // Lower than unfolded, where the three arms stand one above the other.
    assert!(frame.height < unfolded.placed[structure].height);
    // Every group inside an arm says which; the others none.
    for group in &tabs.groups {
        let body = match (&group.wire, group.placed.map(|placed| &tabs.placed[placed])) {
            (Some(wire), _) => wire.body,
            (None, Some(placed)) if group.arm.is_none() => placed.body,
            (None, Some(_)) => continue,
            (None, None) => panic!("a group with neither a node nor a wire"),
        };
        let arm = frame.holds.iter().position(|&held| held == body);
        assert_eq!(group.within, arm, "{}", group.node);
    }
}

#[test]
fn a_structure_unfolded_or_a_node_in_standard_view_is_a_state_of_the_view() {
    let tabs = drawn_with(&ViewState { tabs: true, ..ViewState::default() });
    let key = tabs.arms[0].structure.clone();
    let unfolded = drawn_with(&ViewState { tabs: true, unfolded: [key].into(), ..ViewState::default() });
    assert_eq!(unfolded, drawn_with(&ViewState::default()));
    let add = tabs.placed.iter().find(|placed| placed.holds.is_empty() && placed.body != 0).unwrap();
    let standard = drawn_with(&ViewState { tabs: true, standard: [add.key.clone()].into(), ..ViewState::default() });
    let wider = standard.placed.iter().find(|placed| placed.key == add.key).unwrap();
    assert!(wider.width > add.width, "{} {}", wider.width, add.width);
}

#[test]
fn a_name_of_a_terminal_shows_from_75_percent_and_a_title_always() {
    let drawing = drawn_with(&ViewState::default());
    let details: Vec<(String, crate::Detail)> = drawing
        .loose
        .iter()
        .filter_map(|shape| match shape {
            Shape::Text { runs, detail, .. } => Some((runs.iter().map(|run| run.text.as_str()).collect(), *detail)),
            _ => None,
        })
        .collect();
    assert_eq!(
        details,
        [
            ("main".to_owned(), crate::Detail::Title),
            ("x".to_owned(), crate::Detail::Name),
            ("n".to_owned(), crate::Detail::Name),
            ("y".to_owned(), crate::Detail::Name)
        ]
    );
    assert!(crate::Detail::Name.shown_at(0.75) && !crate::Detail::Name.shown_at(0.74));
    assert!(crate::Detail::Label.shown_at(0.4) && !crate::Detail::Label.shown_at(0.39) && crate::Detail::Title.shown_at(0.01));
}
