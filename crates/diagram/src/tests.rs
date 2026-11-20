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
        literal: None,
        ignored: false,
        reads: Vec::new(),
        pill: None,
        ty_text: String::new(),
        default: None,
        traced: None,
        place: None,
        expects: crate::graph::Expects::Type,
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
    let sizer = Sizer::new(false);
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
    // The band of `dev`: 12 high, inside the block, at its top: nothing above it.
    assert_eq!(raw.above, 0);
    let band = raw.shapes.iter().find(|shape| matches!(shape, Shape::Rect { paint, .. } if paint.fill_token() == Some("band.fill"))).unwrap();
    assert!(matches!(band, Shape::Rect { x: 0, y: 0, width: 4000, height: 1200, .. }), "{band:?}");
    // The two lines of the name in the middle of what the band leaves, 12 to 40: around 26.
    let middles: Vec<i64> = raw
        .shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::Text { y, runs, .. } if runs.iter().any(|run| run.token == "node.title") => Some(*y),
            _ => None,
        })
        .collect();
    assert_eq!(middles.len(), 2);
    assert!(middles[0] < 2600 && middles[1] > 2600 && middles[0] > 1200, "{middles:?}");
    // A band wider than the block, which `bands` would have cut, widens nothing.
    let wide = labcraft_types::text::ShownText::plain("dmm_driver");
    let node = graph.nodes.iter().find(|node| node.key == "dmm").unwrap();
    assert_eq!(sizer.size(node, Some(&wide), None).width, 4000);
}

/// Notation 3.2: a name on one line when it holds; else cut at an underscore when both parts hold, where the longer line
/// is shortest; else the first line as far as the last underscore that it holds; else the first word, cut, and the rest;
/// else, a single word of two lines at most, two halves, the first the longer; else as far as it holds, the rest cut by
/// `…`.
#[test]
fn a_name_holds_on_two_lines_of_the_block() {
    // 40 less a terminal of 6: 34. At 9 a cell is 1233 × 9 / 2048 = 5.42: 6 cells are 32.51, 7 are 37.93.
    assert_eq!(crate::size::name_cells(), 6);
    let cut_of = |name: &str| -> Vec<String> {
        let source = format!("labcraft 1\n\nfn main(x: f64) -> (y: f64) {{\n    y = {name}(x)\n}}\n");
        let callee = format!("labcraft 1\n\nfn {name}(x: f64) -> (y: f64) {{\n    y = x\n}}\n");
        let (checked, names) = checked(&[("main.lcf", &source), (&format!("{name}.lcf"), &callee)]);
        let graph = graph(&checked, &names, "main.lcf");
        titles(&Sizer::new(false).size(&graph.nodes[0], None, None))
    };
    assert_eq!(cut_of("open"), ["open"]);
    assert_eq!(cut_of("a_b"), ["a_b"]);
    // Both parts hold: `query` (5) and `f64` (3).
    assert_eq!(cut_of("query_f64"), ["query", "f64"]);
    // No part pair holds: the first line takes `bench`, the underscore after it is the cut, `selftest` is cut to 5 and `…`.
    assert_eq!(cut_of("bench_selftest"), ["bench", "selft\u{2026}"]);
    // `read_` holds, `read_dc_` does not: cut at the first underscore.
    assert_eq!(cut_of("read_dc_volts"), ["read", "dc_vo\u{2026}"]);
    assert_eq!(cut_of("count_to_fixed"), ["count", "to_fi\u{2026}"]);
    // A single word of two lines at most: two halves, the first the longer: 7 cells give 4 and 3, 6 give 3 and 3.
    assert_eq!(cut_of("elapsed"), ["elap", "sed"]);
    assert_eq!(cut_of("measure"), ["meas", "ure"]);
    // 6 cells hold on one line.
    assert_eq!(cut_of("settle"), ["settle"]);
    // Both parts hold: `append` (6) and `csv` (3).
    assert_eq!(cut_of("append_csv"), ["append", "csv"]);
    // The first word does not hold (`average`, 7) and no underscore is in what holds: each part on its line, `average`
    // cut to 5 and `…`.
    assert_eq!(cut_of("average_csv"), ["avera\u{2026}", "csv"]);
    // A grapheme of 2 cells: `ab一cde` is 1 + 1 + 2 + 1 + 1 + 1 = 7 cells; the shortest first line of 4 or more is `ab一`.
    assert_eq!(cut_of("ab\u{4E00}cde"), ["ab\u{4E00}", "cde"]);
    // More than two lines: 13 cells, as far as the first line holds, the rest cut.
    assert_eq!(cut_of("abcdefghijklm"), ["abcdef", "ghijk\u{2026}"]);
    // An underscore that would leave a line empty is no cut: a single word of 9 cells, halves of 5 and 4.
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
    let sizer = Sizer::new(false);
    let of = |key: &str| graph.nodes.iter().find(|node| node.key == key).unwrap();
    for key in ["a", "b"] {
        assert_eq!(of(key).inputs.iter().map(|input| input.family.base).collect::<Vec<_>>(), [Base::Number, Base::Error], "{key}");
        let sized = sizer.size(of(key), None, None);
        // Two lines, `duration` alone on the first place, 8 below the top, then the `err` pair on the last one, 8 above
        // the bottom of 40, at 32.
        assert_eq!((sized.inputs.clone(), sized.outputs.clone()), (vec![(0, 800), (0, 3200)], vec![(4000, 3200)]), "{key}");
    }
    assert!(of("c").inputs.is_empty());
}

/// Notation 3.2: lines from the top, 8 below the upper edge and `node.pitch` apart, whatever they carry; past four
/// lines, the block grows down by two places, never across.
#[test]
fn a_block_grows_by_its_lines_only() {
    let size_of = |count: usize| {
        let params: Vec<String> = (0..count).map(|index| format!("p{index}: f64")).collect();
        let callee = format!("labcraft 1\n\nfn many({}) -> (y: f64) {{\n    y = p0\n}}\n", params.join(", "));
        let source = format!("labcraft 1\n\nfn main(x: f64) -> (y: f64) {{\n    y = many({})\n}}\n", vec!["x"; count].join(", "));
        let (checked, names) = checked(&[("main.lcf", &source), ("many.lcf", &callee)]);
        let graph = graph(&checked, &names, "main.lcf");
        Sizer::new(false).size(&graph.nodes[0], None, None)
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
    // From the top: 8, then 16, 24, 32; a terminal of 6 has a radius of 3.
    assert_eq!(inputs, [(800, 300), (1600, 300), (2400, 300), (3200, 300)]);
    // Five lines do not hold in four places: two more, 40 + 16 = 56.
    let five = size_of(5);
    assert_eq!((five.width, five.height), (4000, 5600));
}

/// Notation 3.2: five lines do not hold in four places: the block grows by two places, to 56, every input on the left,
/// an optional one too, and the `err` pair on the last place, 8 above the bottom. `after:` changes nothing of it.
#[test]
fn a_block_of_five_lines_grows_by_two_places() {
    let callee =
        "labcraft 1\n\nfn wide(a: f64, b: f64, c: f64, d: f64 = 1.0, err: error = none) -> (y: f64, err: error) {\n    y = a\n    err = err\n}\n";
    let plain = "labcraft 1\n\nfn main(x: f64, e: error) -> (y: f64, err: error) {\n    y, err = wide(x, x, x, 2.0, err: e)\n}\n";
    let ordered = "labcraft 1\n\nfn main(x: f64, z: f64, e: error) -> (y: f64, err: error) {\n    y, err = wide(x, x, x, 2.0, after: z, err: e)\n}\n";
    for source in [plain, ordered] {
        let (checked, names) = checked(&[("main.lcf", source), ("wide.lcf", callee)]);
        assert_eq!(checked.report.errors(), 0, "{:?}", checked.report.diagnostics());
        let graph = graph(&checked, &names, "main.lcf");
        let sized = Sizer::new(false).size(&graph.nodes[0], None, None);
        // 8, 16, 24, 32 for `a` (with `y`), `b`, `c`, `d`; 56 - 8 = 48 for the `err` pair.
        assert_eq!((sized.width, sized.height), (4000, 5600));
        assert_eq!(sized.inputs, [(0, 800), (0, 1600), (0, 2400), (0, 3200), (0, 4800)]);
        assert_eq!(sized.outputs, [(4000, 800), (4000, 4800)]);
    }
}

/// Notation 3.2: a construction, an oblong, takes the places of a square: its five fields from the top, 8 apart, then
/// two places more, 56 high.
#[test]
fn an_oblong_takes_the_places_of_a_square() {
    let ty = "labcraft 1\n\ntype Five = struct { a: f64, b: f64, c: f64, d: f64, e: f64 }\n";
    let source = "labcraft 1\n\nfn main(x: f64) -> (y: Five) {\n    y = Five { a: x, b: x, c: x, d: x, e: x }\n}\n";
    let (checked, names) = checked(&[("main.lcf", source), ("Five.lct", ty)]);
    assert_eq!(checked.report.errors(), 0, "{:?}", checked.report.diagnostics());
    let graph = graph(&checked, &names, "main.lcf");
    let sized = Sizer::new(false).size(&graph.nodes[0], None, None);
    assert_eq!(sized.height, 5600);
    assert_eq!(sized.inputs, [(0, 800), (0, 1600), (0, 2400), (0, 3200), (0, 4000)]);
}

/// Notation 3.2: two pills less than a pill apart step outward, the lower one past the upper one, 8 from it, and the
/// constant so moved draws a wire to its terminal; a pill 16 below the first one keeps its place.
#[test]
fn neighbouring_pills_step_outward() {
    let callee = "labcraft 1\n\nfn three(a: f64, b: f64, c: f64) -> (y: f64) {\n    y = a\n}\n";
    let source = "labcraft 1\n\nfn main() -> (y: f64) {\n    y = three(1.0, 2.0, 3.0)\n}\n";
    let (checked, names) = checked(&[("main.lcf", source), ("three.lcf", callee)]);
    let graph = graph(&checked, &names, "main.lcf");
    let sized = Sizer::new(false).size(&graph.nodes[0], None, None);
    // Lines at 8, 16 and 24. A pill of 3 cells at 10, 3 × 1233 × 10 / 2048 = 18.06, and 4 on each side: 26.06. Its right
    // end 8 left of the half terminal of 6: -11; it starts at -37.06. The second, 8 lower, under 14, ends 8 left of that,
    // -45.06, and starts at -71.12. The third is 8 below the second, whose right end is 8 left of its own start: it keeps
    // its place.
    assert_eq!(sized.reaches, [Some(3706), Some(7112), Some(3706)]);
    assert_eq!(sized.left, 7112);
    let leads: Vec<&Vec<(i64, i64)>> = sized
        .shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::Poly { points, closed: false, .. } => Some(points),
            _ => None,
        })
        .collect();
    assert_eq!(leads, [&vec![(-4506, 1600), (-300, 1600)]]);
}

/// The points where the arcs of a D of `w` × `h`, 32 high or more, meet its edges (spec 11.4), worked out by hand:
/// a corner of 90° rounded by 2 meets its edges 2 from it; one of 135°, 2 / tan 67.5° = 0.83 from it, 0.59 across
/// and along on the cut.
fn d_tangents(w: i64, h: i64) -> Vec<(i64, i64)> {
    vec![
        (0, 200),
        (200, 0),
        (w - 883, 0),
        (w - 741, 59),
        (w - 59, 741),
        (w, 883),
        (w, h - 883),
        (w - 59, h - 741),
        (w - 741, h - 59),
        (w - 883, h),
        (200, h),
        (0, h - 200),
    ]
}

/// Notation 3.2: a primitive is a D of 24 × 32 with two values in, 24 × 16 with one, its right corners cut by 8 and its
/// six corners rounded by 2; one input at the middle, two on the first and last places, three on the first, the middle
/// and the last, all on the left edge and on the grid; `select` with two constants keeps its size, and its neighbouring
/// pills step outward as on a block.
#[test]
fn a_primitive_is_a_d() {
    let source = "labcraft 1\n\nuse core\n\nfn main(a: f64, b: f64, ok: bool) -> (y: f64, z: f64, w: f64) {\n    y = core.add(a, b)\n    z = core.select(ok, 1.0, 0.0)\n    w = core.neg(a)\n}\n";
    let (checked, names) = checked(&[("main.lcf", source)]);
    let graph = graph(&checked, &names, "main.lcf");
    let sizer = Sizer::new(false);
    let node = |key: &str| graph.nodes.iter().find(|node| node.key == key).unwrap();
    let add = sizer.size(node("y"), None, None);
    assert_eq!((add.width, add.height), (2400, 3200));
    assert_eq!(add.inputs, [(0, 800), (0, 2400)]);
    assert_eq!(add.outputs, [(2400, 1600)]);
    // The D of `template d`, from the file of `core.add` in `libs/core` (spec A.1, 11.4).
    let outline = add.shapes.iter().find_map(|shape| match shape {
        Shape::Outline { outline, .. } => sizer.outlines_drawn.borrow().get(*outline).cloned(),
        _ => None,
    });
    let outline = outline.unwrap();
    for point in d_tangents(2400, 3200) {
        assert!(outline.contains(&point), "{point:?}: {outline:?}");
    }
    let select = sizer.size(node("z"), None, None);
    assert_eq!((select.width, select.height), (2400, 3200));
    assert_eq!(select.inputs, [(0, 800), (0, 1600), (0, 2400)]);
    // `1.0` at 16, 3 cells at 10 and 4 on each side, 26.06, its right end 8 left of the half terminal: from -37.06. `0.0`,
    // 8 lower, steps past it: its right end 8 left of -37.06, from -71.12.
    assert_eq!(select.reaches, [None, Some(3706), Some(7112)]);
    // 16 high, one tip at (24, 8), a corner of 90° rounded by 2: its arc 2 / sin 45° - 2 = 0.83 inside the box.
    let neg = sizer.size(node("w"), None, None);
    assert_eq!((neg.width, neg.height), (2400, 1600));
    assert_eq!((neg.inputs.clone(), neg.outputs.clone()), (vec![(0, 800)], vec![(2317, 800)]));
}

/// Notation 3.2: a unit that does not hold in a D of 24 makes it wider, with 3 on each side, up to the grid; its
/// output stays on the right edge.
#[test]
fn a_long_unit_widens_the_d() {
    let source = "labcraft 1\n\nuse core\n\nfn main(a: f64) -> (y: f64[kg/(m*s)]) {\n    y = core.with_unit(a, [kg/(m*s)])\n}\n";
    let (checked, names) = checked(&[("main.lcf", source)]);
    let graph = graph(&checked, &names, "main.lcf");
    let sized = Sizer::new(false).size(&graph.nodes[0], None, None);
    // `[kg/(m*s)]`, 10 cells at 10: 10 × 1233 × 10 / 2048 = 60.21, and 3 on each side, 66.21: 72 on the grid.
    // One value in: 16 high, the output on the tip, 0.83 inside the box.
    assert_eq!((sized.width, sized.height), (7200, 1600));
    assert_eq!(sized.outputs, [(7117, 800)]);
}

/// Notation 3.2, spec 11.2: only the unit of `with_unit` and `unitless` widens the D. `[V]`, 3 cells at 10, 18.06, and 3
/// on each side, 24.06: 32. `|x|`, as wide, leaves `abs` at the 24 of its section.
#[test]
fn a_unit_widens_the_d_and_a_sign_never_does() {
    let source = "labcraft 1\n\nuse core\n\nfn main(a: f64) -> (y: f64[V], z: f64) {\n    y = core.with_unit(a, [V])\n    z = core.abs(a)\n}\n";
    let (checked, names) = checked(&[("main.lcf", source)]);
    let graph = graph(&checked, &names, "main.lcf");
    let sizer = Sizer::new(false);
    let size = |key: &str| {
        let sized = sizer.size(graph.nodes.iter().find(|node| node.key == key).unwrap(), None, None);
        (sized.width, sized.height)
    };
    assert_eq!((size("y"), size("z")), ((3200, 1600), (2400, 1600)));
}

/// Decided at step 2 of M0v: 24 for the header, the first line at 40, lines 24 apart, the last 16 above the bottom, 12
/// on each side, 24 between the names, 112 at least.
#[test]
fn a_standard_block_has_the_numbers_of_step_2() {
    let (checked, names) = bench();
    let graph = graph(&checked, &names, "main.lcf");
    let sizer = Sizer::new(true);
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
    let sized = Sizer::new(false).size(node, None, None);
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
    let sized = Sizer::new(false).size(node, Some(&wide), None);
    // 40 wide; `open`, 4 cells at 9, 4 × 1233 × 9 / 2048 = 21.674, 2167 hundredths: it starts at 2000 - 2167 / 2, 917.
    let starts: Vec<i64> = sized
        .shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::Text { x, runs, .. } if runs.iter().all(|run| run.token == "node.title") => Some(*x),
            _ => None,
        })
        .collect();
    assert_eq!(starts, [917]);
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
        // The colours of the states of the viewer (decisions of the owner, 2026-11-13).
        ("\nicon 32 {\n    circle (16, 16) 8 accent select.stroke\n}\n".to_owned(), None),
        ("\nicon 32 {\n    circle (16, 16) 8 accent node.active\n}\n".to_owned(), None),
        ("\nicon 32 {\n    circle (16, 16) 8 accent wire.error\n}\n".to_owned(), Some(1)),
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

/// Extra inputs that `check` refuses are drawn by the rules of 11.2, as on any block with a section: `a` on the first
/// place, `b` on the last, the others on the free places from the top. The D grows by two places at a time, `last` and
/// `middle` counted on the grown side.
#[test]
fn a_d_holds_all_its_inputs() {
    let source = "labcraft 1\n\nuse core\n\nfn main(a: f64, b: f64) -> (y: f64) {\n    y = core.add(a, b, a, b, a)\n}\n";
    let (checked, names) = checked(&[("main.lcf", source)]);
    let graph = graph(&checked, &names, "main.lcf");
    let sized = Sizer::new(false).size(&graph.nodes[0], None, None);
    // Five inputs on three places: two more, five, 48 high; `b` on the last, 40.
    assert_eq!(sized.height, 4800);
    let ys: Vec<i64> = sized.inputs.iter().map(|&(_, y)| y).collect();
    assert_eq!(ys, [800, 4000, 1600, 2400, 3200]);
    assert_eq!(sized.outputs, [(2400, 2400)]);
}

/// Notation 3.2: targets too many for the one value of a primitive, an error that is still drawn, give it outputs on the
/// free places of its right side, by the rules of 11.2, never two on one point (a case of the fuzzing of `diagram`).
#[test]
fn the_outputs_too_many_of_a_d_take_places() {
    let size_of = |targets: &str, style: &crate::style::Style| {
        let outputs: Vec<String> = targets.split(", ").map(|name| format!("{name}: f64")).collect();
        let source =
            format!("labcraft 1\n\nuse core\n\nfn main(a: f64, b: f64) -> ({}) {{\n    {targets} = core.add(a, b)\n}}\n", outputs.join(", "));
        let (checked, names) = checked(&[("main.lcf", &source)]);
        let graph = graph(&checked, &names, "main.lcf");
        let sizer = Sizer::new(false);
        crate::style::scope(&std::sync::Arc::new(style.clone()), || sizer.size(&graph.nodes[0], None, None))
    };
    let defaults = crate::style::Style::default();
    let two = size_of("y, z", &defaults);
    assert_eq!((two.width, two.height), (2400, 3200));
    // `result` on the middle, the other on the first free place, 8, on the arc of the corner (24, 8), by hand: from its
    // tangent point (23.41, 7.41) to its middle (23.85, 8.06), both rounded to the hundredth, 8 crosses it at 23.81.
    assert_eq!(two.outputs, [(2400, 1600), (2381, 800)]);
    assert_eq!(two.inputs, [(0, 800), (0, 2400)]);
    // Four targets on three places: two more, five, 48 high; `result` on the middle, 24, the others on 8, 16 and 32.
    let four = size_of("w, x, y, z", &defaults);
    assert_eq!(four.height, 4800);
    assert_eq!(four.outputs, [(2400, 2400), (2381, 800), (2400, 1600), (2400, 3200)]);
    // A cut of 16, tried with `--style`: never two outputs on one point, whatever the outline does. The cut is at most
    // half the width, 12, so the output at 8 is on the straight part of the cut, at 12 + 8 = 20, 4 inside the box, which
    // the guard of 11.4 still allows: the D is drawn.
    let mut tried = defaults.clone();
    tried.numbers.chamfer = 1600;
    let cut = size_of("y, z", &tried);
    assert_ne!(cut.outputs.first(), cut.outputs.get(1));
    assert!(cut.shapes.iter().any(|shape| matches!(shape, Shape::Outline { .. })), "{:?}", cut.outputs);
    assert_eq!(cut.outputs, [(2400, 1600), (2000, 800)]);
    // `with_unit` and its long unit, 72 wide, with two targets: its one place on the right holds one, so it grows by two
    // places, 32 high. The cut of 16 is then whole, and the output at 8 lies on it at 72 - 16 + 8 = 64, 8 inside the
    // box: a guard of 11.4 breaks, and the block is drawn ordinary, its outputs on its right edge, none off the block.
    let source = "labcraft 1\n\nuse core\n\nfn main(a: f64) -> (y: f64[kg/(m*s)], z: f64[kg/(m*s)]) {\n    y, z = core.with_unit(a, [kg/(m*s)])\n}\n";
    let (checked, names) = checked(&[("main.lcf", source)]);
    let graph = graph(&checked, &names, "main.lcf");
    let sizer = Sizer::new(false);
    let wide = crate::style::scope(&std::sync::Arc::new(tried), || sizer.size(&graph.nodes[0], None, None));
    assert_eq!((wide.width, wide.height), (7200, 3200));
    assert!(!wide.shapes.iter().any(|shape| matches!(shape, Shape::Outline { .. })), "{:?}", wide.outputs);
    assert_eq!(wide.outputs, [(7200, 1600), (7200, 800)]);
}

/// Notation 3.2: the band of a construction is inside it, at its top, and its label in the middle of what the band
/// leaves; a variant constructor, two lines under a band, grows by two places.
#[test]
fn the_band_of_an_oblong_is_inside() {
    let ty = "labcraft 1\n\ntype StepResult = struct { name: string, ok: bool }\n";
    let reading = "labcraft 1\n\ntype Reading = enum { Value(f64), Missing }\n";
    let main = "labcraft 1\n\nuse results_log\n\nfn main(x: f64, s: string, b: bool) -> (r: results_log.StepResult, v: results_log.Reading) {\n    r = results_log.StepResult { name: s, ok: b }\n    v = results_log.Reading.Value(x)\n}\n";
    let (checked, names) = checked(&[("main.lcf", main), ("results_log/StepResult.lct", ty), ("results_log/Reading.lct", reading)]);
    assert_eq!(checked.report.errors(), 0, "{:?}", checked.report.diagnostics());
    let graph = graph(&checked, &names, "main.lcf");
    let bands = crate::bands(&graph);
    let sizer = Sizer::new(false);
    let sized = |key: &str| {
        let index = graph.nodes.iter().position(|node| node.key == key).unwrap();
        sizer.size(&graph.nodes[index], bands[index].as_ref(), None)
    };
    let construction = sized("r");
    // 12 of band, a line of 9 (2384 × 9 / 2048 = 10.48) with 8 above and below: 38.48, so the square of 40.
    assert_eq!((construction.height, construction.above), (4000, 0));
    let band = construction.shapes.iter().find(|shape| matches!(shape, Shape::Rect { paint, .. } if paint.fill_token() == Some("band.fill")));
    assert!(matches!(band, Some(Shape::Rect { x: 0, y: 0, height: 1200, .. })), "{band:?}");
    // The label in the middle of what the band leaves, 12 to 40: at 26.
    let label = construction.shapes.iter().find_map(|shape| match shape {
        Shape::Text { y, runs, .. } if runs.iter().any(|run| run.text == "StepResult") => Some(*y),
        _ => None,
    });
    let baseline = |middle: i64| match crate::size::Text::new("StepResult", 40, crate::size::label_size()).centred(0, middle, "node.title") {
        Shape::Text { y, .. } => y,
        _ => 0,
    };
    assert_eq!(label, Some(baseline(2600)));
    // A variant constructor: 12, two lines (20.95) and 16, 48.95, rounded up to the grid: 56.
    assert_eq!(sized("v").height, 5600);
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
    // In a block of 40: 40 less 5 on each side, 30; at 9 a cell is 1233 × 9 / 2048 = 5.42, 5 cells are 27.09, 6 are
    // 32.51: 5 cells. `meas…` for both; the full path is the same; in the middle, 2 cells, `…`, 2 cells.
    assert_eq!(bands, ["me\u{2026}_a", "me\u{2026}_b"]);
}

/// Notation 3.2: on a node with `after:`, the text of the band starts past the chevron on the top edge, 8 from the left
/// corner: half a terminal (3) beyond it and 2, at 13, so that it holds fewer cells; the chevron stays at (8, 0).
#[test]
fn the_band_keeps_clear_of_the_chevron_of_after() {
    let f = "labcraft 1\n\nfn f(x: f64) -> (y: f64) {\n    y = x\n}\n";
    let main = "labcraft 1\n\nuse results_log\n\nfn main(x: f64, z: f64) -> (y: f64, w: f64) {\n    y = results_log.f(x)\n    w = results_log.f(x, after: z)\n}\n";
    let (checked, names) = checked(&[("main.lcf", main), ("results_log/f.lcf", f)]);
    assert_eq!(checked.report.errors(), 0, "{:?}", checked.report.diagnostics());
    let graph = graph(&checked, &names, "main.lcf");
    let bands = crate::bands(&graph);
    let index = |key: &str| graph.nodes.iter().position(|node| node.key == key).unwrap();
    // 40 less 5 on each side, 30: 5 cells at 9 (27.09), `resu…`. With `after:`, 40 less 13 and 5, 22: 4 cells (21.67).
    assert_eq!(bands[index("y")].as_ref().unwrap().text(), "resu\u{2026}");
    assert_eq!(bands[index("w")].as_ref().unwrap().text(), "res\u{2026}");
    let sizer = Sizer::new(false);
    let sized = sizer.size(&graph.nodes[index("w")], bands[index("w")].as_ref(), None);
    assert_eq!(sized.order, Some((800, 0)));
    // `res…`, 4 cells, centred between 13 and 35: from 24 - 21.67 / 2 = 13.17 to 34.84, past the chevron, which ends at
    // 11, and short of the output terminal, which starts at 37.
    assert_eq!(band_span(&sized), Some((1317, 3484)));
}

/// Where the text of the band of a node starts and ends, in hundredths.
fn band_span(sized: &crate::size::Sized) -> Option<(i64, i64)> {
    sized.shapes.iter().find_map(|shape| match shape {
        Shape::Text { x, width, runs, .. } if runs.iter().any(|run| run.token == "band.text") => Some((*x, *x + *width)),
        _ => None,
    })
}

/// Notation 3.2 and 4.1: the brackets of an array terminal on the first line reach into the block, beside the band, and
/// its text keeps past them. Two dimensions: half a terminal (3), 2.5 per bracket and half its stroke of 1.2, 8.6, then
/// 2: 10.6 on each side.
#[test]
fn the_band_keeps_clear_of_the_brackets_of_an_array() {
    let f = "labcraft 1\n\nfn f(x: [[f64]]) -> (y: [[f64]]) {\n    y = x\n}\n";
    let main = "labcraft 1\n\nuse results_log\n\nfn main(x: [[f64]]) -> (y: [[f64]]) {\n    y = results_log.f(x)\n}\n";
    let (checked, names) = checked(&[("main.lcf", main), ("results_log/f.lcf", f)]);
    assert_eq!(checked.report.errors(), 0, "{:?}", checked.report.diagnostics());
    let graph = graph(&checked, &names, "main.lcf");
    let bands = crate::bands(&graph);
    // 40 less 21.2 is 18.8: 3 cells at 9 (16.25), `re…`.
    assert_eq!(bands[0].as_ref().unwrap().text(), "re\u{2026}");
    let sized = Sizer::new(false).size(&graph.nodes[0], bands[0].as_ref(), None);
    // Centred at 20: from 20 - 8.13 = 11.87 to 28.13, between 10.6 and 29.4.
    assert_eq!(band_span(&sized), Some((1187, 2813)));
}

/// Notation 3.2: the name of a node `always` without an icon is on one line, cut, between the band and the `▴`, which
/// it never meets: `query_f64` gives `query…`.
#[test]
fn the_name_of_an_always_node_is_on_one_line() {
    let (checked, names) = bench();
    let graph = graph(&checked, &names, "main.lcf");
    let sizer = Sizer::new(false);
    let band = labcraft_types::text::ShownText::plain("dev");
    let mut node = graph.nodes.iter().find(|node| node.key == "raw").unwrap().clone();
    node.always = true;
    let texts = |band: Option<&labcraft_types::text::ShownText>| -> Vec<(String, i64)> {
        sizer
            .size(&node, band, None)
            .shapes
            .iter()
            .filter_map(|shape| match shape {
                Shape::Text { y, runs, .. } if runs.iter().all(|run| run.token == "node.title") => {
                    Some((runs.iter().map(|run| run.text.as_str()).collect(), *y))
                }
                _ => None,
            })
            .collect()
    };
    // The baseline of a line of the name centred at `middle`.
    let baseline = |middle: i64| match crate::size::Text::new("query_f64", 6, crate::size::label_size()).centred(2000, middle, "node.title") {
        Shape::Text { y, .. } => y,
        _ => 0,
    };
    // The `▴` is centred 8 above the bottom, at 32, in a line of 2384 × 10 / 2048 = 11.64 at 10: that line starts at
    // 32 - 5.82 = 26.18. The name is centred between the band and it, (12 + 26.18) / 2 = 19.09; without a band, between
    // the top edge and it, 13.09. Its line, 10.48 high at 9, never reaches 26.18.
    for (band, middle) in [(Some(&band), 1909), (None, 1309)] {
        let texts = texts(band);
        // At most 6 cells: 5 of `query_f64`, `query`, and `…`; then the `▴`.
        assert_eq!(texts.iter().map(|(text, _)| text.as_str()).collect::<Vec<_>>(), ["query\u{2026}", "\u{25B4}"]);
        assert_eq!(texts[0].1, baseline(middle), "{texts:?}");
    }
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
    // `StepResult`, 10 cells at 9 (54.19) and 2 on each side, 58.19: 64 on the grid. 64 less 10 is 54, which holds 9
    // cells at 9 (48.77), not 10 (54.19): `results_…`, clear of the terminal of `name` beside the band.
    assert_eq!(band_of("r"), "results_\u{2026}");
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

/// S3 (notation 2.2): `k` skips the columns of `a` and `b` to reach `y`, at the height of its second input, which the
/// D of `a` and `b` (two values in, 32 high, the same top) holds. Over them is 24 + 16 from there, under them 8 + 16: the track runs
/// under them, 16 below their bottom, and nothing of `k` goes over them. Worked out from the rule, not from a drawing.
#[test]
fn a_track_takes_a_lane_under_the_nodes_when_it_is_nearer() {
    let main = "labcraft 1\n\nuse core\n\nfn main(x: f64, k: f64) -> (y: f64) {\n    a = core.mul(x, x)\n    b = core.mul(a, a)\n    y = core.add(b, k)\n}\n";
    let (checked, names) = checked(&[("main.lcf", main)]);
    let placed = placed_of(&checked, &names, "main.lcf");
    let (a, b, y) = (&placed["a"], &placed["b"], &placed["y"]);
    assert_eq!((a.y, b.y, a.height, b.height), (y.y, y.y, 3200, 3200));
    let drawing = drawing_of(main);
    let points: Vec<(i64, i64)> = branches(wire_of(&drawing, "k")).into_iter().flatten().copied().collect();
    // The horizontal that passes the columns of `a` and `b`.
    let under = points
        .windows(2)
        .find(|pair| pair[0].1 == pair[1].1 && pair[0].0.min(pair[1].0) < a.x && pair[0].0.max(pair[1].0) > b.x + b.width)
        .map(|pair| pair[0].1);
    assert_eq!(under, Some(a.y + 3200 + 1600));
    assert!(points.iter().all(|&(_, height)| height >= a.y), "{points:?}");
}

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
    // `step` is a block of 40. Its lines from the top, `x` at 8, and its error pair on the last place, 8 above the bottom:
    // at 32 from its top.
    assert_eq!((a.height, b.y, y.y), (4000, a.y, a.y));
    assert_eq!((a.column, b.column, d.column, y.column), (0, 1, 2, 3));
    let axis = a.y + 3200;
    // `c`, a D of 16 (one value in), in the column of `a`: 24 above it.
    assert_eq!(c.y, a.y - 2400 - 1600);
    // `d` is alone in its column: its bottom 40 above the axis.
    assert_eq!(d.y + 3200, axis - 4000);
}

/// A node that the `layout` section places is there, from the top-left corner of the frame of the function; a node
/// that would cover it moves up out of its way, 24 above it.
#[test]
fn a_placed_node_keeps_its_place() {
    let main = "labcraft 1\n\nuse core\n\nfn main(x: f64) -> (y: f64) {\n    a = core.neg(x)\n    y = core.abs(x)\n}\n\nlayout {\n    node y at (66, 72)\n}\n";
    let (checked, names) = checked(&[("main.lcf", main)]);
    let placed = placed_of(&checked, &names, "main.lcf");
    let (a, y) = (&placed["a"], &placed["y"]);
    // `a` stands in the first column: after the name `x` (one cell at 10, 6.02), 12 before it and 8 after it, the
    // narrowest channel, 16, its wire being straight, and the half terminal of 3: 45.02 from the corner of the frame,
    // its block, 24 wide, reaching 69.02. It reaches over `y`, at 66, so it goes up, 24 above it: at 72 - 24 - 16 = 32
    // from the corner, higher than the header of 24 and its margin of 16 (40). The frame grows up by 8 so that it holds
    // it: its top is at the margin, 16, its corner 8 lower, at 24 (notation 5).
    // Across, the margin of 16 is around everything: the terminal of `x` reaches 3 left of the frame, at 19.
    assert_eq!((y.x, y.y), (1900 + 6600, 2400 + 7200));
    assert_eq!((a.x, a.y), (1900 + 1200 + 602 + 800 + 1600 + 300, 2400 + 3200));
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
    drawing_with(&[("main.lcf", main)])
}

/// The drawing of `main.lcf` among `files`, which must check without a diagnostic.
fn drawing_with(files: &[(&str, &str)]) -> crate::Drawing {
    let (checked, names) = checked(files);
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
/// keeps its distance as a line with a constant does. A change of scale shows the unit it arrives in; a widening that
/// keeps the value, the arrow alone; the whole conversion goes to the tooltip of the input.
#[test]
fn a_conversion_is_a_pill_on_its_wire() {
    let f0 = "labcraft 1\n\nfn f0(v: f64[V]) -> (w: f64[V]) {\n    w = v\n}\n";
    let main = "labcraft 1\n\nfn main(x: f64[mV]) -> (y: f64[V]) {\n    y = f0(x)\n}\n";
    let (checked, names) = checked(&[("main.lcf", main), ("f0.lcf", f0)]);
    assert!(checked.report.diagnostics().is_empty(), "{:?}", checked.report.diagnostics());
    let graph = graph(&checked, &names, "main.lcf");
    let pills: Vec<(&str, Option<&str>)> = graph.pills.values().map(|pill| (pill.text.as_str(), pill.conversion.as_deref())).collect();
    assert_eq!(pills, [("\u{2192}V", Some("f64[mV]\u{2192}f64[V]"))]);
    assert_eq!(graph.nodes[0].inputs[0].pill.as_deref(), Some("\u{2192}V"));
    // `f32` into `f64`: the value is kept.
    let f1 = "labcraft 1\n\nfn f1(v: f64) -> (w: f64) {\n    w = v\n}\n";
    let main = "labcraft 1\n\nfn main(x: f32) -> (y: f64) {\n    y = f1(x)\n}\n";
    let (widened, names) = self::checked(&[("main.lcf", main), ("f1.lcf", f1)]);
    assert!(widened.report.diagnostics().is_empty(), "{:?}", widened.report.diagnostics());
    let built = self::graph(&widened, &names, "main.lcf");
    let pills: Vec<(&str, Option<&str>)> = built.pills.values().map(|pill| (pill.text.as_str(), pill.conversion.as_deref())).collect();
    assert_eq!(pills, [("\u{2192}", Some("f32\u{2192}f64"))]);
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
    let (frame, ..) = drawing.bodies[0].frame;
    let found = branches(wire_of(&drawing, "x"));
    assert!(found[0].iter().any(|&(x, _)| x == frame + 10_000), "{found:?}");
    // The wire is marked as the file's; the body is not, since no node of it is placed.
    assert!(wire_of(&drawing, "x").wire.as_ref().unwrap().via);
    assert!(!drawing.bodies[0].from_file);
}

/// The paths of the readers of a value, in the order of its wires.
fn reader_paths(drawing: &crate::Drawing, value: &str) -> Vec<Vec<(i64, i64)>> {
    wire_of(drawing, value).wire.as_ref().unwrap().readers.iter().map(|(_, path)| path.clone()).collect()
}

/// A wire whose first waypoint lies on a wire of its value is a branch of it: it starts at that point, its junction, and
/// the trunk is drawn once (notation 2.4).
#[test]
fn a_branch_starts_at_its_junction() {
    let plain = "labcraft 1\n\nuse core\n\nfn main(x: f64) -> (y: f64, z: f64) {\n    a = core.neg(x)\n    b = core.abs(a)\n    y = core.neg(b)\n    z = core.abs(a)\n}\n";
    let before = drawing_of(plain);
    let (fx, fy, ..) = before.bodies[0].frame;
    // A point of the trunk of `a` on a horizontal segment, on the grid of units, from its first reader's path.
    let trunk = reader_paths(&before, "a").into_iter().next().unwrap();
    let (start, end) = trunk.windows(2).map(|pair| (pair[0], pair[1])).find(|(p, q)| p.1 == q.1 && (q.0 - p.0).abs() >= 300).unwrap();
    let junction = (((start.0.min(end.0) - fx) / 100 + 1) * 100 + fx, start.1);
    assert!(junction.0 < start.0.max(end.0));
    let (jx, jy) = ((junction.0 - fx) / 100, (junction.1 - fy) / 100);
    let routed = format!("{plain}\nlayout {{\n    wire a -> z via ({jx}, {jy})\n}}\n");
    let drawing = drawing_of(&routed);
    let group = wire_of(&drawing, "a");
    let wire = group.wire.as_ref().unwrap();
    assert_eq!(wire.vias, [false, true]);
    assert_eq!(wire.entries, [Some("a -> b.x".to_owned()), Some("a -> z.x".to_owned())]);
    let paths = reader_paths(&drawing, "a");
    // The branch runs along the trunk to its junction, then leaves it.
    assert!(crate::route::prefix_to(&paths[0], junction).is_some(), "{paths:?}");
    let to_junction = paths[1].iter().position(|&point| point == junction).unwrap();
    assert_eq!(paths[1][..to_junction], crate::route::prefix_to(&paths[0], junction).unwrap()[..to_junction]);
    assert_ne!(paths[1].get(to_junction + 1).map(|point| point.1), Some(junction.1), "it leaves the trunk downward: {paths:?}");
    let circles: Vec<(i64, i64)> = group
        .shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::Circle { x, y, .. } => Some((*x, *y)),
            _ => None,
        })
        .collect();
    assert!(circles.contains(&junction), "{circles:?}");
}

/// From inside a structure, `wire stop -> n` is the part from the notch to `until`, and it keeps its waypoints (spec 11).
#[test]
fn a_via_inside_a_structure_is_kept() {
    let plain = "labcraft 1\n\nfn main(s: bool) -> (z: bool) {\n    z = loop carry a = s {\n        next a = a\n        until s\n    }\n}\n";
    let drawing = drawing_of(plain);
    let inner = drawing.groups.iter().find_map(|group| group.wire.as_ref().filter(|wire| wire.value == "s" && wire.body != 0)).unwrap();
    assert_eq!(inner.entries, [Some("s -> z".to_owned())]);
    assert_eq!(inner.vias, [false]);
    let body = inner.body;
    let (fx, fy, ..) = drawing.bodies[body].frame;
    let (ox, oy) = drawing.bodies[body].origin;
    let routed = format!("{plain}\nlayout {{\n    wire s -> z via (16, 4) (40, 4)\n}}\n");
    let drawing = drawing_of(&routed);
    let inner = drawing.groups.iter().find_map(|group| group.wire.as_ref().filter(|wire| wire.value == "s" && wire.body != 0)).unwrap();
    assert_eq!(inner.vias, [true]);
    let path = &inner.readers[0].1;
    let (ox2, oy2) = drawing.bodies[inner.body].origin;
    assert!(
        path.contains(&(ox2 + 1_600, oy2 + 400)) && path.contains(&(ox2 + 4_000, oy2 + 400)),
        "{path:?}, frame {:?} origin {:?}",
        (fx, fy),
        (ox, oy)
    );
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
            // The terminals of the frame of an arm, over its wires: in that arm.
            (None, Some(placed)) if group.arm.is_none() && group.node.is_empty() => {
                assert!(group.within.is_some_and(|arm| placed.holds.contains(&frame.holds[arm])), "{:?}", group.within);
                continue;
            }
            (None, Some(placed)) if group.arm.is_none() => placed.body,
            (None, Some(_)) => continue,
            // The terminals of the function, over its wires.
            (None, None) if group.node.is_empty() => 0,
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
    // The title alone, then the terminals of the function over its wires (notation 5).
    let edges = drawing.groups.iter().filter(|group| group.wire.is_none() && group.placed.is_none()).flat_map(|group| group.shapes.iter());
    let details: Vec<(String, crate::Detail)> = drawing
        .loose
        .iter()
        .chain(edges)
        .filter_map(|shape| match shape {
            Shape::Text { runs, detail, .. } => Some((runs.iter().map(|run| run.text.as_str()).collect(), *detail)),
            _ => None,
        })
        .collect();
    assert_eq!(
        details,
        [
            ("fn main".to_owned(), crate::Detail::Title),
            ("x".to_owned(), crate::Detail::Name),
            ("n".to_owned(), crate::Detail::Name),
            ("y".to_owned(), crate::Detail::Name)
        ]
    );
    assert!(crate::Detail::Name.shown_at(0.75) && !crate::Detail::Name.shown_at(0.74));
    assert!(crate::Detail::Label.shown_at(0.4) && !crate::Detail::Label.shown_at(0.39) && crate::Detail::Title.shown_at(0.01));
}

/// `LC7005` (notation 2.1): two nodes that the file places one over the other, in one body. `core.add` is a D of
/// 24 across (its file of `libs/core`, `top 2`), so its block spans x to x + 24: at 100 and 123 the blocks share one unit
/// across and all their height; at 100 and 124 they only touch.
#[test]
fn two_nodes_that_the_file_places_one_over_the_other_get_lc7005() {
    let main = |second: &str| {
        format!(
            "labcraft 1\n\nuse core\n\nfn main(a: f64, b: f64, c: f64) -> (y: f64) {{\n    ab = core.add(a, b)\n    y = core.add(ab, c)\n}}\n\n\
             layout {{\n    node ab at (100, 40)\n    node y at ({second}, 40)\n}}\n"
        )
    };
    let warnings = drawing_of(&main("123")).warnings;
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    let warning = &warnings[0];
    assert_eq!(warning.code(), labcraft_types::Code::Lc7005);
    assert!(warning.message.starts_with("node `y` overlaps node `ab`"), "{}", warning.message);
    // The entry of `y`: line 12 of the file (1 `labcraft`, 3 `use`, 5 `fn`, 10 `layout {`, 11 `node ab`, 12 `node y`).
    assert_eq!(warning.span.map(|span| span.start.line), Some(12));
    assert_eq!(warning.file, "main.lcf");
    assert!(drawing_of(&main("124")).warnings.is_empty());
    // A node that the file does not place moves out of the way and is never named.
    let unplaced = "labcraft 1\n\nuse core\n\nfn main(a: f64, b: f64, c: f64) -> (y: f64) {\n    ab = core.add(a, b)\n    y = core.add(ab, c)\n}\n\n\
                    layout {\n    node ab at (100, 40)\n}\n";
    assert!(drawing_of(unplaced).warnings.is_empty());
    // Three in a diagonal, `y` at (100, 100), `w` at (105, 105), `a` at (110, 110), each 24 across, 32 down: from the
    // left `y`, `w`, `a`. `w` overlaps `y`; `a` overlaps both, and names the nearest above it, `w` (105 against 100).
    let diagonal = "labcraft 1\n\nuse core\n\nfn main(p: f64, q: f64, r: f64, s: f64) -> (y: f64) {\n    a = core.add(p, q)\n    \
                    w = core.add(a, r)\n    y = core.add(w, s)\n}\n\nlayout {\n    node a at (110, 110)\n    node w at (105, 105)\n    \
                    node y at (100, 100)\n}\n";
    let messages: Vec<String> = drawing_of(diagonal).warnings.iter().map(|warning| warning.message.clone()).collect();
    assert_eq!(messages.len(), 2, "{messages:?}");
    assert!(messages[0].starts_with("node `w` overlaps node `y`"), "{messages:?}");
    assert!(messages[1].starts_with("node `a` overlaps node `w`"), "{messages:?}");
}

/// Notation 4: a name is never written twice side by side. `period` at `text.medium` 10 is 6 cells of 6, 36 units, and
/// needs 4 more on each side: 44. Its halfway repeat (N-3) needs a horizontal past the first name three times that, 132,
/// which neither branch has. In hundredths:
/// - a first run of 200, a drop, a run of 40: the name goes on the first run, unless the wire leaves a terminal that
///   shows it;
/// - a first run of 10, too short, a drop, a last run of 80: the name goes on the last run, unless the branch ends at a
///   terminal that shows it.
#[test]
fn a_wire_has_no_name_beside_a_terminal_that_shows_it() {
    let count = |branch: Vec<(i64, i64)>, start: bool, end: bool| {
        let mut obstacles = crate::names::Obstacles::default();
        crate::wire_names("period", &[branch], (start, &[end]), &mut obstacles).len()
    };
    let long_first = || vec![(0, 0), (20000, 0), (20000, 4000), (24000, 4000)];
    assert_eq!(count(long_first(), false, false), 1);
    assert_eq!(count(long_first(), true, false), 0);
    let long_last = || vec![(0, 0), (1000, 0), (1000, 4000), (9000, 4000)];
    assert_eq!(count(long_last(), false, false), 1);
    assert_eq!(count(long_last(), false, true), 0);
    // Two branches sharing their trunk: a run of 5, too short, a drop, a run of 295 that holds the first name, then the
    // second branch goes on down. The run of 295 is the longest past the first segment of the second branch, but it is
    // the trunk that holds the first name: no second name there.
    let mut obstacles = crate::names::Obstacles::default();
    let first = vec![(0, 0), (500, 0), (500, 1000), (30000, 1000)];
    let second = vec![(0, 0), (500, 0), (500, 1000), (30000, 1000), (30000, 3000)];
    assert_eq!(crate::wire_names("period", &[first, second], (false, &[false, false]), &mut obstacles).len(), 1);
}

/// Notation 5, 7: the function has a frame without fill; a wire leaves the centre of a terminal of a frame, under its
/// name, which lies on a ground of the fill of its frame, so that the wire shows when the name does not.
#[test]
fn the_frame_of_the_function_has_no_fill_and_its_wires_leave_the_centre_of_its_terminals() {
    let main = "labcraft 1\n\nuse core\n\nfn main(x: f64) -> (y: f64) {\n    y = core.neg(x)\n}\n";
    let drawing = drawing_of(main);
    // Its outline, with no fill, then its title.
    assert!(
        matches!(drawing.loose.first(), Some(Shape::Rect { paint, .. }) if paint.fill.is_none() && paint.stroke_token() == Some("frame.stroke")),
        "{:?}",
        drawing.loose
    );
    assert!(matches!(drawing.loose.get(1), Some(Shape::Text { .. })), "{:?}", drawing.loose);
    let (left, _, width, _) = drawing.bodies[0].frame;
    assert_eq!(branches(wire_of(&drawing, "x"))[0].first().map(|point| point.0), Some(left));
    assert_eq!(branches(wire_of(&drawing, "y"))[0].last().map(|point| point.0), Some(left + width));
    let ground_of = |name: &str| {
        drawing
            .groups
            .iter()
            .flat_map(|group| group.shapes.iter())
            .find_map(|shape| match shape {
                Shape::Text { runs, ground, .. } if runs.iter().map(|run| run.text.as_str()).collect::<String>() == name => *ground,
                _ => None,
            })
            .unwrap()
    };
    // `x`: from the half terminal of 6 and an outline of 1, 4 inside, so that it never covers the terminal, to where
    // its route starts: 12 before the name, one cell at 10 (6.02) and 4 after it, 22.02 from the edge; a line at 10
    // high, 11.64.
    let ground = ground_of("x");
    assert_eq!((ground.token, ground.x, ground.width, ground.height), ("canvas.fill", left + 400, 2202 - 400, 1164));
    // `y` on the right, from where its route ends to the half terminal and its outline.
    let ground = ground_of("y");
    assert_eq!((ground.token, ground.x + ground.width), ("canvas.fill", left + width - 400));
    // The terminals of the arms of a `match`, two levels down, on the fill of their frames.
    let arms = drawn_with(&ViewState::default());
    let tokens: std::collections::BTreeSet<&str> = arms
        .groups
        .iter()
        .filter(|group| group.wire.is_none() && group.node.is_empty() && group.placed.is_some())
        .flat_map(|group| group.shapes.iter())
        .filter_map(|shape| match shape {
            Shape::Text { ground: Some(ground), .. } => Some(ground.token),
            _ => None,
        })
        .collect();
    assert_eq!(tokens.into_iter().collect::<Vec<_>>(), ["frame.fill.2"]);
}

/// Notation 5.2: the number of arms never sets the width of the frame. Nine arms that each yield a constant: their row of
/// tabs would be wider than the frame, so a selector stands for it, its parts each shown with its arm.
#[test]
fn tabs_that_do_not_hold_give_way_to_a_selector() {
    let ty = "labcraft 1\n\ntype Range = enum { Millivolts, Volts, TenVolts, HundredVolts, Kilovolts, Milliamps, Amps, Ohms, Kiloohms }\n";
    let arms: String = ["Millivolts", "Volts", "TenVolts", "HundredVolts", "Kilovolts", "Milliamps", "Amps", "Ohms", "Kiloohms"]
        .iter()
        .map(|variant| format!("        Range.{variant} => yield 1.0\n"))
        .collect();
    let main = format!("labcraft 1\n\nfn main(range: Range) -> (scale: f64) {{\n    scale = match range {{\n{arms}    }}\n}}\n");
    let (checked, names) = checked(&[("main.lcf", &main), ("Range.lct", ty)]);
    assert!(checked.report.diagnostics().is_empty(), "{:?}", checked.report.diagnostics());
    let Drawn::Drawing(drawing) =
        draw(&checked, index_of(&checked, "main.lcf"), &names, &ViewState { tabs: true, ..ViewState::default() }, &DiagramLimits::default())
    else {
        panic!()
    };
    let structure = drawing.placed.iter().position(|placed| !placed.holds.is_empty()).unwrap();
    let frame = &drawing.placed[structure];
    // The tabs alone, at 10 a cell and 4 on each side, 4 apart: `Range.Millivolts` is 16 cells, 96.33 + 8; the nine far
    // past the frame, whose width they no longer set.
    assert!(drawing.tabs.iter().all(|tab| tab.kind != crate::TabKind::Tab));
    let names_of: Vec<&str> = drawing.tabs.iter().filter(|tab| tab.kind == crate::TabKind::Name).map(|tab| tab.caption.as_str()).collect();
    assert_eq!(names_of.len(), 9);
    // A value of an enumeration shows its variant alone (notation 3.1).
    assert_eq!(names_of[1], "Volts");
    // Each title shows with its arm, between its two steps, round: from the first, `‹` goes to the last.
    for tab in &drawing.tabs {
        assert!(tab.when.is_some());
        assert!(tab.x >= frame.x && tab.x + tab.width <= frame.x + frame.width, "{tab:?} {frame:?}");
    }
    let first = drawing.tabs.iter().find(|tab| tab.kind == crate::TabKind::Name).unwrap();
    let steps: Vec<usize> = drawing.tabs.iter().filter(|tab| tab.kind == crate::TabKind::Step && tab.when == first.when).map(|tab| tab.arm).collect();
    let rank = |arm: usize| drawing.arms[arm].arm;
    assert_eq!(steps.iter().map(|&arm| rank(arm)).collect::<Vec<_>>(), [8, 1]);
}

/// Notation 3.2, 4.1: the tooltip of a terminal says its name and its exact type, and the default of an input that no
/// argument wires; that of a wire, its name and its type.
#[test]
fn a_terminal_tells_its_type_and_its_default() {
    let f = "labcraft 1\n\nfn f(x: f64[V], k: f64 = 2.0, err: error = none) -> (y: f64[V], err: error) {\n    y = x\n    err = err\n}\n";
    let main = "labcraft 1\n\nuse core\nuse time\n\nfn main(x: f64[V]) -> (y: f64[V], e: error, w: error, s: f64[V]) {\n    y, e1 = f(x)\n    e = time.wait(1 s, err: e1)\n    w = time.wait(2 s)\n    s = core.add(x, y)\n}\n";
    let (checked, names) = checked(&[("main.lcf", main), ("f.lcf", f)]);
    assert!(checked.report.diagnostics().is_empty(), "{:?}", checked.report.diagnostics());
    let Drawn::Drawing(drawing) = draw(&checked, index_of(&checked, "main.lcf"), &names, &ViewState::default(), &DiagramLimits::default()) else {
        panic!()
    };
    let texts: Vec<&str> = drawing.hints.iter().map(|hint| hint.text.as_str()).collect();
    // A parameter of a primitive whose type in the table has no variable says it, wired or not: `duration`, not the
    // `f64[s]` that the constant settles. One with a variable says what the call settles: `a: f64[V]` for `core.add`.
    for expected in ["x: f64[V]", "k: f64\ndefault: 2.0", "y: f64[V]", "duration: duration", "a: f64[V]", "b: f64[V]"] {
        assert!(texts.contains(&expected), "{expected:?} in {texts:?}");
    }
    // The `err` of `f`, and that of the second `time.wait`, which the table of the primitive gives (spec appendix A).
    assert_eq!(texts.iter().filter(|text| **text == "err: error\ndefault: none").count(), 2, "{texts:?}");
    assert_eq!(wire_of(&drawing, "x").wire.as_ref().unwrap().ty, "f64[V]");
    assert_eq!(wire_of(&drawing, "e1").wire.as_ref().unwrap().ty, "error");
}

/// `docs/cli.md` 7.2: an error that names a node is outlined around that node, though the groups of the terminals of the
/// frames come between the groups of the nodes; and the SVG has no node without a name.
#[test]
fn a_mark_outlines_its_own_node_and_the_svg_has_no_unnamed_node() {
    let main = "labcraft 1\n\nuse core\n\nfn main(x: f64[V], n: i32) -> (y: f64[V], z: f64) {\n    a = core.neg(x)\n    y = match n {\n        0 => yield a\n        _ => yield x\n    }\n    z = core.neg(2.0)\n}\n";
    let mut drawing = drawing_of(main);
    let mut diagnostic = labcraft_types::Diagnostic::new(labcraft_types::Code::Lc2102, "m", "main.lcf");
    diagnostic.node = Some(labcraft_types::Node { name: Some("a".to_owned()), text: "a = core.neg(x)".to_owned() });
    crate::mark(&mut drawing, &[diagnostic]);
    let placed = drawing.placed.iter().zip(drawing.groups.iter().filter(|group| group.is_node())).find(|(_, group)| group.node == "a").unwrap().0;
    let Some(Shape::Rect { x, y, .. }) = drawing.marks[0].shapes.first() else { panic!("{:?}", drawing.marks) };
    // 3 around its block (notation 4.2).
    assert_eq!((*x, *y), (placed.x - 300, placed.y - 300));
    let svg = crate::svg(&drawing, crate::Theme::Light);
    assert!(!svg.contains("data-node=\"\""), "{svg}");
    // The terminals of the arms are in their `data-arm`: each arm has its frame and its terminals.
    assert_eq!(svg.matches("data-arm=\"0\"").count(), 2);
}

/// Notation 3.3, 5.2: a value of an enumeration shows its variant alone, in the title of an arm, on the pill of a
/// constant, in a default and in an array; the terminal gives the type. A variant with data keeps what it binds. The
/// `data-arm` of the SVG keeps the pattern as the text writes it (`docs/cli.md`, 7.2).
#[test]
fn a_value_of_an_enumeration_shows_its_variant_alone() {
    let state = "labcraft 1\n\ntype State = enum { Init, RunStep(i32) }\n";
    let f0 = "labcraft 1\n\nfn f0(v: [State]) -> (w: [State]) {\n    w = v\n}\n";
    let main = "labcraft 1\n\nfn main(s: State, d: State = State.RunStep(3)) -> (t: State, c: i32, a: [State]) {\n    t, c = match s {\n        State.Init => yield (State.Init, 0)\n        State.RunStep(code) => yield (State.Init, code)\n    }\n    a = f0([State.Init, State.Init])\n}\n";
    let (checked, names) = checked(&[("main.lcf", main), ("State.lct", state), ("f0.lcf", f0)]);
    assert!(checked.report.diagnostics().is_empty(), "{:?}", checked.report.diagnostics());
    let graph = graph(&checked, &names, "main.lcf");
    let arms: Vec<(&str, &str)> =
        graph.bodies.iter().filter(|body| body.arm.is_some()).map(|body| (body.caption.as_str(), body.pattern.as_str())).collect();
    assert_eq!(arms, [("Init", "State.Init"), ("RunStep(code)", "State.RunStep(code)")]);
    let yielded: Vec<Option<&str>> = graph
        .bodies
        .iter()
        .filter(|body| body.arm.is_some())
        .map(|body| body.right.first().and_then(|terminal| terminal.terminal.constant.as_deref()))
        .collect();
    assert_eq!(yielded, [Some("Init"), Some("Init")]);
    assert_eq!(graph.bodies[0].left[1].terminal.constant.as_deref(), Some("RunStep(\u{2026})"));
    let call = graph.nodes.iter().find(|node| node.inputs.first().is_some_and(|input| input.name == "v")).unwrap();
    assert_eq!(call.inputs[0].constant.as_deref(), Some("[Init, Init]"));
}

/// Notation 5.1: `x in values` enters by `]` and the shape of the element's family, `collect` leaves by that shape in
/// brackets, a range by a box that writes each constant bound in its place and `·` for one that a wire gives; the start
/// of a range is not a pill on a wire. The tooltip of the box covers no more than a register's would.
#[test]
fn the_terminals_of_a_for_show_what_crosses_the_edge() {
    let main = "labcraft 1\n\nuse core\n\nfn main(values: [f64], n: i32) -> (out: [f64]) {\n    out = for x in values, i in 0..n {\n        y = core.mul(x, 2.0)\n        collect y\n    }\n}\n";
    let (checked, names) = checked(&[("main.lcf", main)]);
    assert!(checked.report.diagnostics().is_empty(), "{:?}", checked.report.diagnostics());
    let graph = graph(&checked, &names, "main.lcf");
    let body = graph.bodies.iter().find(|body| body.left.iter().any(|terminal| terminal.edge == crate::graph::Edge::In)).unwrap();
    let left: Vec<(crate::graph::Edge, Option<&str>)> = body.left.iter().map(|terminal| (terminal.edge, terminal.start.as_deref())).collect();
    assert_eq!(left, [(crate::graph::Edge::In, None), (crate::graph::Edge::Range, Some("0..\u{B7}"))]);
    let collect = body.right.iter().find(|terminal| terminal.edge == crate::graph::Edge::Collect).unwrap();
    // `]` at (0, 0): 2.5 to 5 outside, 12 high, its stroke 1.5; then the filled circle of a number, of 6 (3 across its
    // centre), 3 inside.
    let shapes = crate::edge_terminal(&body.left[0], 0, 0);
    assert!(matches!(&shapes[0], Shape::Poly { points, closed: false, .. } if *points == [(-500, -600), (-250, -600), (-250, 600), (-500, 600)]));
    assert!(matches!(&shapes[1], Shape::Circle { x: 300, y: 0, radius: 300, .. }), "{shapes:?}");
    // `[•]`: two brackets 2.5 to 5 on each side, the circle of a number between them.
    let shapes = crate::edge_terminal(collect, 0, 0);
    assert_eq!(shapes.len(), 3);
    assert!(matches!(&shapes[2], Shape::Circle { x: 0, y: 0, radius: 300, .. }), "{shapes:?}");
    // `•]`, a `last`: the circle 3 inside, one bracket 2.5 to 5 outside, as `]▸` mirrored (notation 5.1).
    let mut last = collect.clone();
    last.edge = crate::graph::Edge::Last;
    let shapes = crate::edge_terminal(&last, 0, 0);
    assert!(matches!(&shapes[0], Shape::Poly { points, closed: false, .. } if *points == [(250, -600), (500, -600), (500, 600), (250, 600)]));
    assert!(matches!(&shapes[1], Shape::Circle { x: -300, y: 0, radius: 300, .. }), "{shapes:?}");
    // The box of the range reaches 7 inside, as a register's, and grows outside.
    let shapes = crate::edge_terminal(&body.left[1], 0, 0);
    assert!(matches!(&shapes[0], Shape::Rect { x, width, radius: 300, .. } if x + width == 700 && *x < -700));
    // The other forms of a range: both bounds constant, the start wired.
    for (range, inputs, written) in [("0..10", "n: i32", "0..10"), ("a..10", "a: i32", "\u{B7}..10")] {
        let main = format!(
            "labcraft 1\n\nuse core\n\nfn main({inputs}) -> (out: [i32]) {{\n    out = for i in {range} {{\n        j = core.neg(i)\n        collect j\n    }}\n}}\n"
        );
        let (other, names) = self::checked(&[("main.lcf", main.as_str())]);
        let built = self::graph(&other, &names, "main.lcf");
        let starts: Vec<Option<&str>> = built.bodies.iter().flat_map(|body| body.left.iter()).map(|terminal| terminal.start.as_deref()).collect();
        assert!(starts.contains(&Some(written)), "{range}: {starts:?}");
    }
    let drawing = drawing_of(main);
    let texts: Vec<String> = drawing
        .groups
        .iter()
        .flat_map(|group| group.shapes.iter())
        .filter_map(|shape| match shape {
            Shape::Text { runs, .. } => Some(runs.iter().map(|run| run.text.as_str()).collect::<String>()),
            _ => None,
        })
        .collect();
    assert!(texts.iter().any(|text| text == "0..\u{B7}"), "{texts:?}");
    assert!(!texts.iter().any(|text| text == "0"), "{texts:?}");
    // The tooltip of `i`: 7 on each side of the edge, as a register's, never the width of the box outside.
    // Two: the input of the structure (8, the least) and the terminal of its frame (14).
    let sizes: Vec<(i64, i64)> = drawing.hints.iter().filter(|hint| hint.text.starts_with("i: ")).map(|hint| (hint.width, hint.height)).collect();
    assert_eq!(sizes.len(), 2);
    assert!(sizes.contains(&(1400, 1400)), "{sizes:?}");
    assert!(sizes.iter().all(|&(width, height)| width <= 1400 && height <= 1400), "{sizes:?}");
}

/// Notation 4.2: the whole conversion is in the tooltip of what it reaches, an output of the function as an input of a
/// node, and in the `data-conversion` of its wire (`docs/cli.md`, 7.2).
#[test]
fn the_whole_conversion_is_in_the_tooltip_and_the_svg() {
    let main = "labcraft 1\n\nfn main(x: f32) -> (y: f64) {\n    y = x\n}\n";
    let drawing = drawing_of(main);
    let hint = drawing.hints.iter().find(|hint| hint.text.starts_with("y: ")).unwrap();
    assert_eq!(hint.text, "y: f64\nconverted: f32\u{2192}f64");
    let svg = crate::svg(&drawing, crate::Theme::Light);
    assert!(svg.contains("data-conversion=\"f32\u{2192}f64\""), "{svg}");
    let f0 = "labcraft 1\n\nfn f0(v: f64[V]) -> (w: f64[V]) {\n    w = v\n}\n";
    let main = "labcraft 1\n\nfn main(x: f64[mV]) -> (y: f64[V]) {\n    y = f0(x)\n}\n";
    let (checked, names) = checked(&[("main.lcf", main), ("f0.lcf", f0)]);
    let Drawn::Drawing(drawing) = draw(&checked, index_of(&checked, "main.lcf"), &names, &ViewState::default(), &DiagramLimits::default()) else {
        panic!()
    };
    let hint = drawing.hints.iter().find(|hint| hint.text.starts_with("v: ")).unwrap();
    assert_eq!(hint.text, "v: f64[V]\nconverted: f64[mV]\u{2192}f64[V]");
}

/// Notation 4, B1: the wire of a constant node has no name at its start, since the node shows it.
#[test]
fn the_wire_of_a_constant_node_is_not_named_at_its_start() {
    let main = "labcraft 1\n\nuse core\n\nfn main() -> (y: f64) {\n    gain = 2.0\n    y = core.neg(gain)\n}\n";
    let drawing = drawing_of(main);
    let names: Vec<String> = wire_of(&drawing, "gain")
        .shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::Text { runs, .. } => Some(runs.iter().map(|run| run.text.as_str()).collect::<String>()),
            _ => None,
        })
        .collect();
    assert!(!names.iter().any(|name| name == "gain"), "{names:?}");
}

/// The end of the first branch of the wire that leaves `from` (`fn.a`).
fn end_of(drawing: &crate::Drawing, from: &str) -> (i64, i64) {
    let group = drawing.groups.iter().find(|group| group.wire.as_ref().is_some_and(|wire| wire.from.as_deref() == Some(from))).unwrap();
    *branches(group).first().unwrap().last().unwrap()
}

/// S1 (notation 5): the notches of a loop are in the order of the height of what feeds them outside, not in the order in
/// which the body reads them: `b` is read first, but `a`, above it on the frame of the function, has the upper notch.
#[test]
fn the_notches_follow_what_feeds_them() {
    let main = "labcraft 1\n\nuse core\n\nfn main(a: f64, b: f64) -> (y: f64) {\n    y = loop carry v = 0.0 {\n        s = core.sub(b, a)\n        next v = s\n        until true\n    }\n}\n";
    let drawing = drawing_of(main);
    let (a, b) = (end_of(&drawing, "fn.a"), end_of(&drawing, "fn.b"));
    assert!(a.1 < b.1, "{a:?} {b:?}");
}

/// Notation 5.1: a range whose two bounds are wires has a point for each, the start a place above the end.
#[test]
fn the_two_wired_bounds_of_a_range_enter_at_two_points() {
    let main = "labcraft 1\n\nuse core\n\nfn main(first: i32, n: i32) -> (total: i32) {\n    total = for i in first..n, carry acc = 0 {\n        next acc = core.add(acc, i)\n    }\n}\n";
    let drawing = drawing_of(main);
    let (first, n) = (end_of(&drawing, "fn.first"), end_of(&drawing, "fn.n"));
    assert_eq!(n.1 - first.1, crate::style::compact_pitch(), "{first:?} {n:?}");
    // One wired bound: one point, as before.
    let main = "labcraft 1\n\nuse core\n\nfn main(n: i32) -> (total: i32) {\n    total = for i in 0..n, carry acc = 0 {\n        next acc = core.add(acc, i)\n    }\n}\n";
    let drawing = drawing_of(main);
    let svg = crate::svg(&drawing, crate::Theme::Light);
    assert!(svg.contains("0..\u{B7}"), "{svg}");
    // The wire of `n` ends where `i` leaves inside: at the height of the box, not a place below.
    let start_of_i = *branches(wire_of(&drawing, "i")).first().unwrap().first().unwrap();
    assert_eq!(end_of(&drawing, "fn.n").1, start_of_i.1);
}

/// S1 in nested frames: the notches of an inner loop follow what feeds them at the end, through the notches of the outer
/// one, so that `a` stays above `b` at both levels and their wires do not cross between the frames.
#[test]
fn nested_notches_follow_one_order() {
    let main = "labcraft 1\n\nuse core\n\nfn main(a: f64, b: f64) -> (y: f64) {\n    y = loop carry v = 0.0 {\n        w = loop carry u = 0.0 {\n            s = core.sub(b, a)\n            next u = s\n            until true\n        }\n        next v = w\n        until true\n    }\n}\n";
    let drawing = drawing_of(main);
    // The wires into the inner loop, in the body of the outer one: each from its notch to the notch inside.
    let end_in = |value: &str| -> i64 {
        let group = drawing.groups.iter().find(|group| group.wire.as_ref().is_some_and(|wire| wire.value == value && wire.body == 1)).unwrap();
        branches(group).first().unwrap().last().unwrap().1
    };
    assert!(end_in("a") < end_in("b"));
    let (a, b) = (end_of(&drawing, "fn.a"), end_of(&drawing, "fn.b"));
    assert!(a.1 < b.1, "{a:?} {b:?}");
}

/// Where the title of a frame starts, the frame's own `x`, and the open lines drawn in its header before the title with
/// their color: the mark of its family (notation 5).
fn header_of(shapes: &[Shape]) -> (i64, i64, Vec<(usize, &'static str)>) {
    let Some(Shape::Rect { x: frame, y: top, .. }) = shapes.first() else { panic!("{shapes:?}") };
    let title = shapes
        .iter()
        .find_map(|shape| match shape {
            Shape::Text { x, runs, .. } if runs.iter().all(|run| run.token == "node.title") => Some(*x),
            _ => None,
        })
        .unwrap();
    let marks = shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::Poly { points, closed: false, paint }
                if points.iter().all(|&(x, y)| x > frame + 900 && x < title && y > top - 1000 && y < top + 2400) =>
            {
                Some((points.len(), paint.stroke_token().unwrap()))
            }
            _ => None,
        })
        .collect();
    (*frame, title, marks)
}

/// Each structure shows the mark of its family before its title, in the color of its frame, and the title follows it:
/// 12 from the edge (`place.label_start`), a mark of 12 (`frame.mark`) and a gap of 4 (`node.padding`), 28 in all. A
/// return arc of 16 points and its head of 3; a stem of 2 and a bracket of 4; two arrows of a line and a head each. The
/// frame of the function has no mark, its title at 12 (notation 5, 10).
#[test]
fn a_structure_shows_the_mark_of_its_family_before_its_title() {
    let main = "labcraft 1\n\nuse core\n\nfn main(xs: [f64], s: bool, err: error = none) -> (ys: [f64], z: f64, w: bool, v: f64, err: error) {\n    ys = for parallel x in xs {\n        y = core.neg(x)\n        collect y\n    }\n    z = match s {\n        true => yield 1.0\n        false => yield 2.0\n    }\n    w = loop carry a = s {\n        next a = a\n        until a\n    }\n    v, e2 = core.div(1.0, 2.0, err)\n    k = handle e2 {\n        ok => yield 0.0\n        _ => yield 1.0\n    }\n    err = none\n}\n";
    let (checked, names) = checked(&[("main.lcf", main)]);
    let Drawn::Drawing(drawing) = draw(&checked, index_of(&checked, "main.lcf"), &names, &ViewState::default(), &DiagramLimits::default()) else {
        panic!("{:?}", checked.report.diagnostics())
    };
    for (node, marks) in [
        ("ys", vec![(2, "frame.stroke"), (3, "frame.stroke"), (2, "frame.stroke"), (3, "frame.stroke")]),
        ("z", vec![(2, "frame.stroke"), (4, "frame.stroke")]),
        ("w", vec![(16, "frame.stroke"), (3, "frame.stroke")]),
        ("k", vec![(2, "wire.error"), (4, "wire.error")]),
    ] {
        let group = drawing.groups.iter().find(|group| group.node == node && group.arm.is_none()).unwrap();
        let (frame, title, found) = header_of(&group.shapes);
        assert_eq!(title - frame, 2800, "{node}");
        assert_eq!(found, marks, "{node}");
    }
    let (frame, title, found) = header_of(&drawing.loose);
    assert_eq!((title - frame, found), (1200, Vec::new()));
}

/// The mark widens the header that sets the width of a frame: `loop` (4 cells at 10, 24.08) and its badge
/// `continue_on_error` (8, then 17 cells at 10, 102.35, and 4 on each side), after 12, the mark and its gap (16), and
/// before 12: 182.43, 184 on the grid of 8 (without the mark, 166.43 and 168).
#[test]
fn the_mark_widens_the_header_of_a_frame() {
    let main = "labcraft 1\n\nfn main(s: bool) -> (z: bool) {\n    z = loop continue_on_error carry a = s {\n        next a = a\n        until a\n    }\n}\n";
    let (checked, names) = checked(&[("main.lcf", main)]);
    let Drawn::Drawing(drawing) = draw(&checked, index_of(&checked, "main.lcf"), &names, &ViewState::default(), &DiagramLimits::default()) else {
        panic!()
    };
    let group = drawing.groups.iter().find(|group| group.node == "z").unwrap();
    let Some(Shape::Rect { width, .. }) = group.shapes.first() else { panic!() };
    assert_eq!(*width, 18_400);
}

/// The corners of a polyline, worked by hand from notation 4.1: radius 6 (600 hundredths) at most, half of a short side at
/// most, and the ends stay as they are.
#[test]
fn corners_round_by_the_radius_or_half_the_shorter_side() {
    use crate::shape::{Corner, corners};
    // Sides of 30 units: the radius is 6, so the curve starts 6 before the corner and ends 6 after it.
    let long = [(0, 0), (3000, 0), (3000, 3000)];
    assert_eq!(
        corners(&long),
        [Corner::Point((0, 0)), Corner::Round { from: (2400, 0), corner: (3000, 0), to: (3000, 600) }, Corner::Point((3000, 3000))]
    );
    // Sides of 10 units: half of a side is 5, less than 6.
    let short = [(0, 0), (1000, 0), (1000, 1000)];
    assert_eq!(
        corners(&short),
        [Corner::Point((0, 0)), Corner::Round { from: (500, 0), corner: (1000, 0), to: (1000, 500) }, Corner::Point((1000, 1000))]
    );
    // Going back up and left: the signs of the sides give the sides of the curve.
    let back = [(3000, 3000), (3000, 0), (0, 0)];
    assert_eq!(corners(&back)[1], Corner::Round { from: (3000, 600), corner: (3000, 0), to: (2400, 0) });
    assert!(corners(&[]).is_empty());
    assert_eq!(corners(&[(7, 9)]), [Corner::Point((7, 9))]);
}

/// The verdict of the drawing on a wire from the port named `from` to the input named `to` (notation 2.5).
fn verdict_of(drawing: &crate::Drawing, from: &str, to: &str) -> crate::Verdict {
    let start = drawing.ports.iter().position(|port| !port.input && port.name == from).unwrap_or_else(|| panic!("no output {from}"));
    let end = drawing.ports.iter().position(|port| port.input && port.name == to).unwrap_or_else(|| panic!("no input {to}"));
    crate::wiring(drawing, start).into_iter().nth(end).flatten().unwrap_or_else(|| panic!("{from} -> {to} is no wire"))
}

fn code_of(verdict: &crate::Verdict) -> &'static str {
    match verdict {
        crate::Verdict::Takes { .. } => "takes",
        crate::Verdict::Refused { refusal, .. } => refusal.code,
    }
}

/// The ports name their ends as `wire.connect` does (commands.md 9.2), and the drawing judges a wire as the command
/// would, from the documents: a carry hides the outer value of its name inside its loop (spec R-3); `array.len` takes an
/// array, whatever its elements (spec A.2); a value from inside a loop reaches outside it through the loop, which takes
/// it out (commands.md 6.7).
#[test]
fn the_drawing_judges_a_wire_by_scope_and_by_family() {
    let main = "labcraft 1\n\nuse array\nuse core\nuse time\n\nfn main(xs: [f64], k: i32, e: error = none) -> (n: i32, out: error) {\n    n = array.len(xs)\n    out = loop carry e = e {\n        d = core.add(1.0, 2.0)\n        f = time.wait(1 s, err: e)\n        next e = f\n        until true\n    }\n}\n";
    let drawing = drawing_of(main);
    // `array.len(array: [T])`, spec A.2.
    let len = "n.array";
    assert_eq!(code_of(&verdict_of(&drawing, "xs", len)), "takes");
    assert_eq!(code_of(&verdict_of(&drawing, "k", len)), "wire.type_mismatch");
    // Inside the loop, `e` is the carry: the input `e` of the function is hidden there.
    let hidden = verdict_of(&drawing, "e", "f.err");
    assert!(matches!(&hidden, crate::Verdict::Refused { refusal, .. } if refusal.code == "wire.out_of_scope"), "{hidden:?}");
    // Out of the loop, the loop takes the value out (commands.md 6.7, `through`): its body is the one exit.
    let out = verdict_of(&drawing, "d", len);
    assert!(matches!(&out, crate::Verdict::Takes { exits, .. } if exits.len() == 1), "{out:?}");
}

/// Commands.md 6.7 and spec 5.2: a wire out of an arm gives the value as an output of its structure; to a node that
/// feeds the selector of that structure, it closes a cycle, as `wire.connect` says after taking the value out.
#[test]
fn a_wire_out_of_an_arm_to_what_feeds_its_structure_is_a_cycle() {
    let main = "labcraft 1\n\nuse core\n\nfn main(x: f64) -> (y: f64) {\n    s = core.add(x, 1.0)\n    c = core.gt(s, 0.0)\n    y = match c {\n        true => {\n            d = core.abs(x)\n            yield d\n        }\n        false => yield x\n    }\n}\n";
    let drawing = drawing_of(main);
    assert_eq!(code_of(&verdict_of(&drawing, "d", "s.b")), "wire.cycle");
}

// The size of a frame (`frame size`, `frame NAME size`, spec 11): a frame is drawn as large as its entry says, never
// under what its content needs. Expected sizes are those of the same text without the entry, and the entry itself.

const SIZED_FOR: &str = "labcraft 1\n\nfn main(v: [f64]) -> (ys: [f64]) {\n    ys = for x in v {\n        collect x\n    }\n}\n";

fn drawing_with_layout(text: &str, layout: &str) -> crate::Drawing {
    let text = if layout.is_empty() { text.to_owned() } else { format!("{text}\nlayout {{\n    {layout}\n}}\n") };
    let (checked, names) = checked(&[("main.lcf", text.as_str())]);
    assert!(checked.report.diagnostics().is_empty(), "{:?}", checked.report.diagnostics());
    let Drawn::Drawing(drawing) = draw(&checked, index_of(&checked, "main.lcf"), &names, &ViewState::default(), &DiagramLimits::default()) else {
        panic!()
    };
    drawing
}

fn placed_named<'a>(drawing: &'a crate::Drawing, name: &str) -> &'a crate::Placed {
    drawing.placed.iter().find(|placed| placed.name == name).unwrap()
}

#[test]
fn a_structure_larger_than_its_content_is_drawn_as_its_entry_says() {
    let automatic = drawing_with_layout(SIZED_FOR, "");
    let auto = placed_named(&automatic, "ys");
    let auto_size = (auto.width, auto.height - auto.over);
    // Larger by a whole unit each way, whatever the automatic size rounds to.
    let (w, h) = (auto_size.0 / 100 + 100, auto_size.1 / 100 + 100);
    let sized = drawing_with_layout(SIZED_FOR, &format!("frame ys size ({w}, {h})"));
    let placed = placed_named(&sized, "ys");
    assert_eq!((placed.width, placed.height - placed.over), (w * 100, h * 100));
    // The least is the automatic size, with or without the entry.
    assert_eq!(auto.least, auto_size);
    assert_eq!(placed.least, auto_size);
}

#[test]
fn a_structure_smaller_than_its_content_is_drawn_as_without_the_entry() {
    let automatic = drawing_with_layout(SIZED_FOR, "");
    let small = drawing_with_layout(SIZED_FOR, "frame ys size (1, 1)");
    let (a, s) = (placed_named(&automatic, "ys"), placed_named(&small, "ys"));
    assert_eq!((a.width, a.height, a.over, a.least), (s.width, s.height, s.over, s.least));
    assert_eq!((automatic.width, automatic.height), (small.width, small.height));
}

#[test]
fn the_frame_of_the_function_is_as_large_as_its_entry_says() {
    let size = |drawing: &crate::Drawing| {
        let body = &drawing.bodies[0];
        // From the origin of `layout`: the frame starts there and its top is the origin.
        (body.frame.0 + body.frame.2 - body.origin.0, body.frame.1 + body.frame.3 - body.origin.1)
    };
    let automatic = drawing_with_layout(SIZED_FOR, "");
    let auto = size(&automatic);
    assert_eq!(automatic.bodies[0].least, Some(auto));
    let (w, h) = (auto.0 / 100 + 100, auto.1 / 100 + 100);
    let sized = drawing_with_layout(SIZED_FOR, &format!("frame size ({w}, {h})"));
    assert_eq!(size(&sized), (w * 100, h * 100));
    assert_eq!(sized.bodies[0].least, Some(auto));
    // Smaller: nothing changes.
    let small = drawing_with_layout(SIZED_FOR, "frame size (1, 1)");
    assert_eq!(size(&small), auto);
    assert_eq!(small.bodies[0].least, Some(auto));
}

#[test]
fn a_stacked_match_taller_has_its_last_arm_taller() {
    let automatic = drawing_with_layout(ARMS, "");
    let auto = placed_named(&automatic, "y");
    let auto_height = auto.height - auto.over;
    let h = auto_height / 100 + 100;
    let sized = drawing_with_layout(ARMS, &format!("frame y size ({}, {h})", auto.width / 100 + 100));
    let placed = placed_named(&sized, "y");
    assert_eq!(placed.height - placed.over, h * 100);
    let extra = h * 100 - auto_height;
    let arm_height = |drawing: &crate::Drawing, placed: &crate::Placed, arm: usize| drawing.bodies[placed.holds[arm]].frame.3;
    let last = placed.holds.len() - 1;
    assert_eq!(arm_height(&sized, placed, last), arm_height(&automatic, auto, last) + extra);
    assert_eq!(arm_height(&sized, placed, 0), arm_height(&automatic, auto, 0));
}

// The height of a terminal of the frame (`input x at y`, `output x at y`, spec 11): a terminal that its entry places is at
// that height, the others keep their order and stay a place (24) apart, and the frame holds them all. Expected heights are
// those of the entries, and the rule of spec 11 for the others.

const TERMINALS_FOR: &str =
    "labcraft 1\n\nuse core\n\nfn main(a: f64, b: f64, c: f64) -> (y: f64, z: f64) {\n    y = core.add(a, b)\n    z = core.mul(b, c)\n}\n";

/// The height of each terminal of the frame of the function, from its origin, and whether it is fixed: inputs, then outputs.
fn frame_terminals(drawing: &crate::Drawing) -> Vec<(String, bool, i64, bool)> {
    let origin = drawing.bodies[0].origin.1;
    drawing
        .ports
        .iter()
        .filter(|port| port.frame && port.body == 0)
        .map(|port| (port.caption.clone(), port.input, port.y - origin, port.fixed))
        .collect()
}

fn height_of(drawing: &crate::Drawing, name: &str, output: bool) -> i64 {
    frame_terminals(drawing).into_iter().find(|(caption, input, _, _)| caption == name && *input == output).unwrap().2
}

#[test]
fn a_terminal_that_its_entry_places_is_at_that_height() {
    let drawing = drawing_with_layout(TERMINALS_FOR, "input b at 400");
    assert_eq!(height_of(&drawing, "b", false), 40_000);
    let fixed: Vec<String> = frame_terminals(&drawing).into_iter().filter(|(_, _, _, fixed)| *fixed).map(|(name, ..)| name).collect();
    assert_eq!(fixed, ["b"]);
    let drawing = drawing_with_layout(TERMINALS_FOR, "output z at 16");
    assert_eq!(height_of(&drawing, "z", true), 1_600);
}

#[test]
fn the_other_terminals_keep_their_order_a_place_apart_around_a_fixed_one() {
    let automatic = drawing_with_layout(TERMINALS_FOR, "");
    let (a, c) = (height_of(&automatic, "a", false), height_of(&automatic, "c", false));
    // `b` fixed where `a` was: `a` and `c` stay in their order, each at least 24 from `b` and from each other.
    let drawing = drawing_with_layout(TERMINALS_FOR, &format!("input b at {}", a / 100));
    let (a2, b2, c2) = (height_of(&drawing, "a", false), height_of(&drawing, "b", false), height_of(&drawing, "c", false));
    assert_eq!(b2, a);
    assert!(a2 < c2, "{a2} {c2}");
    for (one, other) in [(a2, b2), (b2, c2), (a2, c2)] {
        assert!((one - other).abs() >= 2_400, "{one} {other}");
    }
    // A terminal far from the fixed one stays where it was.
    let far = drawing_with_layout(TERMINALS_FOR, "input b at 2000");
    assert_eq!((height_of(&far, "a", false), height_of(&far, "c", false)), (a, c));
}

#[test]
fn the_frame_grows_to_hold_a_terminal_placed_above_or_below_it() {
    let frame = |drawing: &crate::Drawing| {
        let body = &drawing.bodies[0];
        (body.frame.1 - body.origin.1, body.frame.1 + body.frame.3 - body.origin.1)
    };
    let below = drawing_with_layout(TERMINALS_FOR, "input a at 2000");
    assert!(frame(&below).1 >= 200_000 + 1_200, "{:?}", frame(&below));
    let above = drawing_with_layout(TERMINALS_FOR, "output y at -200");
    assert!(frame(&above).0 <= -20_000 - 1_200, "{:?}", frame(&above));
    assert_eq!(height_of(&above, "y", true), -20_000);
}

#[test]
fn the_first_of_two_entries_for_one_terminal_counts() {
    // `check` says `LC5007` of the second; the drawing takes the first, as for a node.
    let text = format!("{TERMINALS_FOR}\nlayout {{\n    input a at 400\n    input a at 800\n}}\n");
    let (checked, names) = checked(&[("main.lcf", text.as_str())]);
    let Drawn::Drawing(drawing) = draw(&checked, index_of(&checked, "main.lcf"), &names, &ViewState::default(), &DiagramLimits::default()) else {
        panic!()
    };
    assert_eq!(height_of(&drawing, "a", false), 40_000);
}

// The terminals of a block (spec 11.2). The places are 8 apart (800 hundredths) and counted from 1: the terminal of place
// `p` on the left or the right is at height `p × 800`, on the top at `p × 800` from the left. A block is `(top + 1) × 8`
// wide and `(left + 1) × 8` high; without `top` the top side has 4 places (width 40), and without `left` the left side has
// 4 places, 40 high, for a function so small.

const G: &str = "fn g(x: f64) -> (n: i64) {\n    n = 1\n}\n";
const H: &str = "fn h(a: f64, err: error = none) -> (n: i64, err: error) {\n    n = 1\n    err = err\n}\n";

/// The size of the only call of `main` to `callee` (a function of `main.lcf`'s folder), with `terminals` added to it.
fn block_sized(callee: &str, call: &str, terminals: &str) -> crate::size::Sized {
    call_sized(callee, call, terminals, None, true)
}

/// The same, under `band` when given, and whatever `check` says of the section unless `clean`.
fn call_sized(callee: &str, call: &str, terminals: &str, band: Option<&str>, clean: bool) -> crate::size::Sized {
    let callee = format!("labcraft 1\n\n{callee}{terminals}");
    let main = format!("labcraft 1\n\nfn main(v: f64, e: error) -> (r: i64, err: error) {{\n    {call}\n    err = e\n}}\n");
    // The file of the function, by its name.
    let name = callee.split("fn ").nth(1).and_then(|rest| rest.split(['(', '<']).next()).unwrap();
    let file = format!("{name}.lcf");
    let (checked, names) = checked(&[("main.lcf", &main), (&file, &callee)]);
    if clean {
        assert_eq!(checked.report.errors(), 0, "{:?}", checked.report.diagnostics());
    }
    let graph = graph(&checked, &names, "main.lcf");
    let node = graph.nodes.iter().find(|node| matches!(node.kind, Kind::Call { .. })).unwrap();
    let band = band.map(labcraft_types::text::ShownText::plain);
    Sizer::new(false).size(node, band.as_ref(), None)
}

fn g_sized(terminals: &str) -> crate::size::Sized {
    block_sized(G, "r = g(v)", terminals)
}

fn h_sized(terminals: &str) -> crate::size::Sized {
    block_sized(H, "r, e2 = h(v, err: e)", terminals)
}

#[test]
fn a_block_without_a_section_is_unchanged() {
    let sized = g_sized("");
    assert_eq!((sized.width, sized.height), (4000, 4000));
    assert_eq!((sized.inputs, sized.outputs), (vec![(0, 800)], vec![(4000, 800)]));
    let sized = h_sized("");
    assert_eq!((sized.width, sized.height), (4000, 4000));
    assert!(!sized.top_side);
}

#[test]
fn top_5_gives_a_block_48_wide() {
    // (5 + 1) × 8 = 48; the left side keeps its 4 places by default: (4 + 1) × 8 = 40.
    let sized = g_sized("\nterminals {\n    top 5\n}\n");
    assert_eq!((sized.width, sized.height), (4800, 4000));
    // The terminals stay on their default places: the right edge moved with the width.
    assert_eq!((sized.inputs, sized.outputs), (vec![(0, 800)], vec![(4800, 800)]));
}

#[test]
fn left_3_gives_a_block_32_high_and_left_4_with_a_middle_five_places() {
    // `left 3`: (3 + 1) × 8 = 32.
    assert_eq!(g_sized("\nterminals {\n    left 3\n}\n").height, 3200);
    // `left 4` with `left middle x`: 5 places, (5 + 1) × 8 = 48 high; the middle is place 3, 3 × 8 = 24 from the top.
    let sized = g_sized("\nterminals {\n    left 4\n    left middle x\n}\n");
    assert_eq!((sized.width, sized.height), (4000, 4800));
    assert_eq!(sized.inputs, [(0, 2400)]);
    assert_eq!(sized.outputs, [(4000, 800)]);
    // Without `left`, the side has the 4 places of a small block, and the middle makes it 5: the same.
    let sized = g_sized("\nterminals {\n    left middle x\n}\n");
    assert_eq!((sized.width, sized.height), (4000, 4800));
    assert_eq!(sized.inputs, [(0, 2400)]);
}

#[test]
fn a_terminal_moved_to_the_right_or_the_top_is_drawn_there() {
    // `right 2 x`: the input on the right edge, place 2, 16 from the top; `n` takes the first place that no line took.
    let sized = g_sized("\nterminals {\n    right 2 x\n}\n");
    assert_eq!((sized.inputs, sized.outputs), (vec![(4000, 1600)], vec![(4000, 800)]));
    assert!(!sized.top_side);
    // `top 3 x`: 3 × 8 = 24 from the left, on the top edge.
    let sized = g_sized("\nterminals {\n    top 3 x\n}\n");
    assert_eq!((sized.inputs, sized.outputs), (vec![(2400, 0)], vec![(4000, 800)]));
    assert!(sized.top_side);
    // `top last x`: the top side has 4 places by default, so the last is 4 × 8 = 32.
    assert_eq!(g_sized("\nterminals {\n    top last x\n}\n").inputs, [(3200, 0)]);
    // An output on the left: place 2.
    assert_eq!(g_sized("\nterminals {\n    left 2 n\n}\n").outputs, [(0, 1600)]);
}

#[test]
fn the_err_pair_takes_the_last_place_of_its_sides() {
    // `top 5`: 48 wide, 4 places on the left: the last is 4 × 8 = 32; `a` and `n` take the first, 8.
    let sized = h_sized("\nterminals {\n    top 5\n}\n");
    assert_eq!((sized.width, sized.height), (4800, 4000));
    assert_eq!(sized.inputs, [(0, 800), (0, 3200)]);
    assert_eq!(sized.outputs, [(4800, 800), (4800, 3200)]);
    // A line that moves `a` leaves the pair where it is.
    let sized = h_sized("\nterminals {\n    top 5\n    top 3 a\n}\n");
    assert_eq!(sized.inputs, [(2400, 0), (0, 3200)]);
}

/// The size of the call to `g` with `terminals` added to it, under a band, whatever `check` says of the section.
fn g_banded(terminals: &str) -> crate::size::Sized {
    call_sized(G, "r = g(v)", terminals, Some("dev"), false)
}

/// The rectangle of the band of a block, if it has one: its top and its height.
fn band_rect(sized: &crate::size::Sized) -> Option<(i64, i64)> {
    sized.shapes.iter().find_map(|shape| match shape {
        Shape::Rect { y, height, paint, .. } if paint.fill_token() == Some("band.fill") => Some((*y, *height)),
        _ => None,
    })
}

#[test]
fn a_side_of_one_place_gives_a_block_of_16() {
    // Spec 11.2: `top 1` and `left 1`, (1 + 1) × 8 = 16 each way; the input and the output on the one place, 8 down.
    let sized = g_sized("\nterminals {\n    top 1\n    left 1\n}\n");
    assert_eq!((sized.width, sized.height), (1600, 1600));
    assert_eq!((sized.inputs, sized.outputs), (vec![(0, 800)], vec![(1600, 800)]));
    // `top 2`, `left 2`: 24 × 24; `a` on the first place, the `err` pair on the last (notation 3.2).
    let sized = h_sized("\nterminals {\n    top 2\n    left 2\n}\n");
    assert_eq!((sized.width, sized.height), (2400, 2400));
    assert_eq!(sized.inputs, [(0, 800), (0, 1600)]);
    assert_eq!(sized.outputs, [(2400, 800), (2400, 1600)]);
    // Two lines on `left 1` grow it by two places, as any side that does not hold its terminals: 3 places, 32 high.
    assert_eq!(h_sized("\nterminals {\n    left 1\n}\n").height, 3200);
}

#[test]
fn a_left_side_of_no_place_holds_a_block_of_top_terminals() {
    // Spec 11.2: `top 3`, `left 0`, both terminals on the top: (3 + 1) × 8 = 32 wide, (0 + 1) × 8 = 8 high, `x` on place
    // 2 (16) and `n` on place 3 (24), on the top edge.
    let sized = g_sized("\nterminals {\n    top 3\n    left 0\n    top 2 x\n    top 3 n\n}\n");
    assert_eq!((sized.width, sized.height), (3200, 800));
    assert_eq!((sized.inputs, sized.outputs), (vec![(1600, 0)], vec![(2400, 0)]));
    // Without the lines, its one line of terminals has no place: it grows by two places, 24 high, `x` and `n` on place 1.
    let sized = g_sized("\nterminals {\n    left 0\n}\n");
    assert_eq!(sized.height, 2400);
    assert_eq!((sized.inputs, sized.outputs), (vec![(0, 800)], vec![(4000, 800)]));
}

#[test]
fn a_size_outside_1_to_64_is_drawn_at_the_nearest_bound() {
    // `top 0` is drawn as `top 1`, `left 99` as `left 64`: 16 wide, (64 + 1) × 8 = 520 high.
    let sized = g_banded("\nterminals {\n    top 0\n    left 99\n}\n");
    assert_eq!((sized.width, sized.height), (1600, 52000));
}

#[test]
fn a_narrow_or_low_block_cuts_its_band_and_its_name_to_it() {
    let texts = |sized: &crate::size::Sized, token: &str| -> Vec<i64> {
        sized
            .shapes
            .iter()
            .filter_map(|shape| match shape {
                Shape::Text { width, runs, .. } if runs.iter().any(|run| run.token == token) => Some(*width),
                _ => None,
            })
            .collect()
    };
    // A band of 11 letters, cut to 40 by the graph, is cut again to a block of 16: what it shows is no wider than it.
    let sized = call_sized(G, "r = g(v)", "\nterminals {\n    top 1\n    left 1\n}\n", Some("development"), true);
    let band = texts(&sized, "band.text");
    assert!(band.len() == 1 && band.iter().all(|width| *width <= 1600), "{band:?}");
    // 8 high holds no line of the name, 10.48 high (notation 3.2): it is left out; 16 holds one.
    assert!(texts(&g_sized("\nterminals {\n    top 3\n    left 0\n    top 2 x\n    top 3 n\n}\n"), "node.title").is_empty());
    assert_eq!(texts(&g_sized("\nterminals {\n    top 3\n    left 1\n}\n"), "node.title").len(), 1);
    // `gh_ij` would take two lines, `gh` and `ij`; 24 × 16 holds one, so it is cut on one.
    let callee = "fn gh_ij(x: f64) -> (n: i64) {\n    n = 1\n}\n";
    let sized = call_sized(callee, "r = gh_ij(v)", "\nterminals {\n    top 2\n    left 1\n}\n", None, true);
    let lines: Vec<String> = sized
        .shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::Text { runs, .. } if runs.iter().all(|run| run.token == "node.title") => Some(runs.iter().map(|run| run.text.as_str()).collect()),
            _ => None,
        })
        .collect();
    assert_eq!(lines.len(), 1, "{lines:?}");
}

#[test]
fn a_block_too_low_for_its_band_draws_it_above() {
    // Notation 3.2: 16 high is lower than twice the band of 12, which goes above, 12 high from -12.
    assert_eq!(band_rect(&g_banded("\nterminals {\n    left 1\n}\n")).map(|rect| rect.0), Some(-1200));
    // 24 high keeps it inside, at the top.
    assert_eq!(band_rect(&g_banded("\nterminals {\n    left 2\n}\n")), Some((0, 1200)));
    // `band hidden` stays hidden; a top side keeps it inside.
    assert_eq!(band_rect(&g_banded("\nterminals {\n    left 1\n    band hidden\n}\n")), None);
    assert_eq!(band_rect(&g_banded("\nterminals {\n    top 3\n    left 1\n    top 2 x\n}\n")).map(|rect| rect.0), Some(0));
}

#[test]
fn the_label_of_a_block_with_a_top_side_goes_below_it() {
    let labelled = |terminals: &str| block_sized(G, "#lab: r = g(v)", terminals);
    // Notation 3.2: the label is above a block, and below it when a terminal is on its top side.
    let plain = labelled("");
    assert!(plain.above > 0 && plain.below == 0, "{} {}", plain.above, plain.below);
    let top = labelled("\nterminals {\n    top 3 x\n}\n");
    // Above it, only the lane of the wire of `x`, its one terminal off its default side (spec 11.2): one wire gap.
    assert!(top.below > 0 && top.above == crate::style::wire_gap(), "{} {}", top.above, top.below);
}

#[test]
fn each_terminal_off_its_side_has_its_own_lane_above_the_block() {
    // `a` on the top, the output `n` on the left 1, the input `err` on the right 2, whose pair takes the facing place, left 2
    // (11.2). The outputs take the lanes first (n, err), then the inputs (a, err), 8 apart, and the node reserves them above
    // its block (notation 2.2: two wires never share a segment).
    let sized = block_sized(
        H,
        "r, e2 = h(v, e)",
        "
terminals {
    top 3 a
    left 1 n
    right 2 input err
}
",
    );
    let gap = crate::style::wire_gap();
    use labcraft_graph::BlockSide::{Left, Right, Top};
    // Beside its side, each wire one gap further out than the one before it on that side: n then err on the left.
    assert_eq!(sized.lanes.1, [Some((Left, 0, gap)), Some((Left, gap, 2 * gap))]);
    assert_eq!(sized.lanes.0, [Some((Top, 2 * gap, 0)), Some((Right, 3 * gap, gap))]);
    assert!(sized.above >= 4 * gap, "{}", sized.above);
}

// The outline of a block (spec 11.4). `g(x) -> (n)`: 40 × 40, `x` on the left at 8, `n` on the right at 8. Worked out by
// hand from 11.4: a piece moves where a terminal starts and how far the block reaches.

/// The size of the call of `main` to `g`, with `sections` added to `g`, and the outlines that the sizer drew.
fn g_outlined(sections: &str) -> (crate::size::Sized, Vec<Vec<(i64, i64)>>) {
    let callee = format!("labcraft 1\n\n{G}{sections}");
    let main = "labcraft 1\n\nfn main(v: f64, e: error) -> (r: i64, err: error) {\n    r = g(v)\n    err = e\n}\n";
    let (checked, names) = checked(&[("main.lcf", main), ("g.lcf", &callee)]);
    assert_eq!(checked.report.errors(), 0, "{:?}", checked.report.diagnostics());
    let graph = graph(&checked, &names, "main.lcf");
    let node = graph.nodes.iter().find(|node| matches!(node.kind, Kind::Call { .. })).unwrap();
    let sizer = Sizer::new(false);
    let sized = sizer.size(node, None, None);
    (sized, sizer.take_outlines())
}

/// The outlines that a block draws, as their points.
fn outline_polygons((sized, outlines): &(crate::size::Sized, Vec<Vec<(i64, i64)>>)) -> Vec<Vec<(i64, i64)>> {
    sized
        .shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::Outline { x, y, outline, .. } => outlines.get(*outline).map(|points| points.iter().map(|(px, py)| (px + x, py + y)).collect()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_block_with_an_outline_draws_it_and_its_terminals_start_on_it() {
    // Without `terminals`, the block is placed as 11.2 places it, 40 × 40; a pin of 4 at `n`, a notch of 4 at `x`.
    let drawn = g_outlined(
        "\noutline {\n    path (0, 0) (w, 0) (w, h) (0, h)\n    left terminal { path (0, -4) (-4, 0) (0, 4) }\n    right terminal { path (0, -2) (4, -2) (4, 2) (0, 2) }\n}\n",
    );
    assert_eq!((drawn.0.width, drawn.0.height), (4000, 4000));
    assert_eq!(drawn.0.inputs, [(400, 800)], "the notch: 4 inside");
    assert_eq!(drawn.0.outputs, [(4400, 800)], "the pin: 4 outside");
    assert_eq!(drawn.0.reach, (0, 400, 0));
    let expected =
        vec![(0, 0), (4000, 0), (4000, 600), (4400, 600), (4400, 1000), (4000, 1000), (4000, 4000), (0, 4000), (0, 1200), (400, 800), (0, 400)];
    assert!(outline_polygons(&drawn).contains(&expected), "{:?}", outline_polygons(&drawn));
    // `template d`: the D of the primitives, its six corners rounded.
    let drawn = g_outlined("\noutline {\n    template d\n}\n");
    let polygons = outline_polygons(&drawn);
    assert_eq!(polygons.len(), 1, "{polygons:?}");
    for point in d_tangents(4000, 4000) {
        assert!(polygons[0].contains(&point), "{point:?}: {polygons:?}");
    }
}

#[test]
fn an_outline_that_breaks_a_guard_is_drawn_as_the_ordinary_block() {
    let drawn = g_outlined("\noutline {\n    path (8, 0) (w, 0) (w, h) (8, h)\n}\n");
    assert!(outline_polygons(&drawn).is_empty());
    assert_eq!((drawn.0.inputs, drawn.0.outputs), (vec![(0, 800)], vec![(4000, 800)]));
}

#[test]
fn the_after_terminal_starts_where_the_outline_crosses_its_place() {
    // A body whose top-left corner is cut from (0, 6) to (12, 0), clear of `v` at 8: the place of `after:`, 8 from the
    // left, crosses the cut at 6 - 6 × 8 / 12 = 2 below the top edge (11.4).
    let sized = block_sized(G, "r = g(v, after: e)", "\noutline {\n    path (12, 0) (w, 0) (w, h) (0, h) (0, 6)\n}\n");
    assert_eq!(sized.order, Some((800, 200)));
    // Without an outline, on the top edge.
    assert_eq!(block_sized(G, "r = g(v, after: e)", "").order, Some((800, 0)));
}

/// Every primitive of `core` with a glyph has its file in `libs/core`, which gives its D (spec A.1): without one, it
/// would lose its glyph and be drawn as an ordinary block.
#[test]
fn every_primitive_with_a_glyph_has_its_block() {
    use labcraft_check::primitives::{PRIMITIVES, PrimitiveLibrary};
    for primitive in PRIMITIVES.iter().filter(|primitive| primitive.library == PrimitiveLibrary::Core) {
        let glyph = crate::graph::sign_of(primitive.name).is_some();
        assert_eq!(glyph, labcraft_check::core_blocks::block(primitive.name).is_some(), "{}", primitive.name);
    }
}

/// The fills of the shapes of a block, in order.
fn fills(sized: &crate::size::Sized) -> Vec<crate::shape::Colour> {
    sized
        .shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::Rect { paint, .. } | Shape::Outline { paint, .. } | Shape::Poly { paint, .. } => paint.fill,
            _ => None,
        })
        .collect()
}

#[test]
fn fill_and_stroke_are_drawn_as_written_and_the_label_takes_the_ink_with_the_more_contrast() {
    use crate::shape::Colour;
    let sized = g_sized("\nterminals {\n    fill \"#64748B\"\n    stroke \"#102030\"\n}\n");
    assert_eq!(fills(&sized), [Colour::Written(0x64_748B, 0x64_748B)]);
    let stroked =
        sized.shapes.iter().any(|shape| matches!(shape, Shape::Rect { paint, .. } if paint.stroke == Some(Colour::Written(0x10_2030, 0x10_2030))));
    assert!(stroked);
    // White has 4.76:1 on `#64748B`, `#1F2328` 3.32:1 (notation 8).
    let label = sized.shapes.iter().find_map(|shape| match shape {
        Shape::Text { runs, .. } => runs.first().map(|run| run.token),
        _ => None,
    });
    assert_eq!(label, Some("ink.light"));
}

#[test]
fn color_gives_the_band_as_written_and_tints_the_body() {
    use crate::shape::Colour;
    let sized = g_banded("\nterminals {\n    color \"#2F6F9F\"\n}\n");
    // The body: `node.fill` mixed with 16 % of the colour, worked out in `colour::tests` for the light theme
    // (`#DEE8F0`); in the dark theme `#20242A` with 16 %: 32 + 15 × 0.16 = 34.4 → 34, 36 + 75 × 0.16 = 48, 42 + 117 × 0.16
    // = 60.72 → 61.
    assert_eq!(fills(&sized).first(), Some(&Colour::Written(0xDE_E8F0, 0x22_303D)));
    assert!(fills(&sized).contains(&Colour::Written(0x2F_6F9F, 0x2F_6F9F)), "{:?}", fills(&sized));
}

// The mask of a library (spec 11.3). `lib.g(x) -> (n)` is called by `main`; the manifest of `lib` is `manifest`, and `sections`
// follows the function. The block is 40 × 40 (11.2), the band 12 high (notation 3.2).

type Masked = (crate::size::Sized, Vec<Vec<(i64, i64)>>, Option<String>);

/// The block of the call to `lib.g`, its outlines and the text of its band, when `lib` has `manifest` and `g` `sections`.
fn masked(manifest: Option<&str>, sections: &str) -> Masked {
    let callee = format!("labcraft 1\n\n{G}{sections}");
    let main = "labcraft 1\n\nuse lib\n\nfn main(v: f64, e: error) -> (r: i64, err: error) {\n    r = lib.g(v)\n    err = e\n}\n";
    let mut root = LibrarySource::new(String::new(), String::new(), Origin::Project);
    root.files.push(source("main.lcf", main));
    let mut lib = LibrarySource::new("lib".to_owned(), "lib".to_owned(), Origin::Project);
    lib.files.push(source("g.lcf", &callee));
    lib.manifest = manifest.map(|text| Ok(text.to_owned()));
    root.sub_libraries.push(lib);
    let mut names = Interner::new();
    let checked = check(root, &Nothing, &mut names, &Limits::default());
    assert_eq!(checked.report.errors(), 0, "{:?}", checked.report.diagnostics());
    let graph = graph(&checked, &names, "main.lcf");
    let bands = crate::bands(&graph);
    let index = graph.nodes.iter().position(|node| matches!(node.kind, Kind::Call { .. })).unwrap();
    let sizer = Sizer::new(false);
    let sized = sizer.size(&graph.nodes[index], bands[index].as_ref(), None);
    let text = bands[index].as_ref().map(|band| band.text());
    (sized, sizer.take_outlines(), text)
}

#[test]
fn the_band_text_of_a_mask_replaces_the_name_of_the_library_and_is_cut_as_a_name() {
    assert_eq!(masked(None, "").2.as_deref(), Some("lib"));
    assert_eq!(masked(Some("[mask]\nband = \"DMM\"\n"), "").2.as_deref(), Some("DMM"));
    // 11 letters in a block of 40: 40 less 5 on each side, 30, 5 cells at 9: `meas…`, as for the name `measurement` of a
    // library (`bands_that_read_the_same_show_more` gives the cells).
    assert_eq!(masked(Some("[mask]\nband = \"measurement\"\n"), "").2.as_deref(), Some("meas\u{2026}"));
    // A refused band (empty) keeps the name of the library.
    assert_eq!(masked(Some("[mask]\nband = \"\"\n"), "").2.as_deref(), Some("lib"));
}

#[test]
fn the_band_text_of_a_mask_names_the_library_on_the_band_of_its_types() {
    let ty = "labcraft 1\n\ntype StepResult = struct { name: string, ok: bool }\n";
    let main = "labcraft 1\n\nuse lib\n\nfn main(s: string, b: bool) -> (r: lib.StepResult) {\n    r = lib.StepResult { name: s, ok: b }\n}\n";
    let mut root = LibrarySource::new(String::new(), String::new(), Origin::Project);
    root.files.push(source("main.lcf", main));
    let mut lib = LibrarySource::new("lib".to_owned(), "lib".to_owned(), Origin::Project);
    lib.files.push(source("StepResult.lct", ty));
    lib.manifest = Some(Ok("[mask]\nband = \"DMM\"\n".to_owned()));
    root.sub_libraries.push(lib);
    let mut names = Interner::new();
    let checked = check(root, &Nothing, &mut names, &Limits::default());
    assert_eq!(checked.report.errors(), 0, "{:?}", checked.report.diagnostics());
    let graph = graph(&checked, &names, "main.lcf");
    let bands: Vec<String> = crate::bands(&graph).into_iter().flatten().map(|band| band.text()).collect();
    assert_eq!(bands, ["DMM"]);
}

#[test]
fn the_colour_of_a_mask_gives_the_band_and_tints_the_body_as_a_color_line_does() {
    use crate::shape::Colour;
    let from_mask = masked(Some("[mask]\ncolor = \"#2F6F9F\"\n"), "").0;
    let from_line = masked(None, "\nterminals {\n    color \"#2F6F9F\"\n}\n").0;
    // The values are those worked out for the `color` line (`color_gives_the_band_as_written_and_tints_the_body`).
    assert_eq!(fills(&from_mask).first(), Some(&Colour::Written(0xDE_E8F0, 0x22_303D)));
    assert!(fills(&from_mask).contains(&Colour::Written(0x2F_6F9F, 0x2F_6F9F)), "{:?}", fills(&from_mask));
    assert_eq!(fills(&from_mask), fills(&from_line));
}

#[test]
fn the_fill_and_the_stroke_of_a_mask_are_drawn_as_written() {
    use crate::shape::Colour;
    let sized = masked(Some("[mask]\nfill = \"#64748B\"\nstroke = \"#102030\"\n"), "").0;
    assert_eq!(fills(&sized).first(), Some(&Colour::Written(0x64_748B, 0x64_748B)));
    let stroked =
        sized.shapes.iter().any(|shape| matches!(shape, Shape::Rect { paint, .. } if paint.stroke == Some(Colour::Written(0x10_2030, 0x10_2030))));
    assert!(stroked);
}

#[test]
fn the_band_place_of_a_mask_is_followed_and_a_band_line_of_the_function_wins() {
    // Above a block of 40: 12 high from -12 (notation 3.2); inside: at the top, 12 high from 0.
    // (Its height is 12 and the 2 of its rounded corner that it overlaps with the block: only its top is checked.)
    assert_eq!(band_rect(&masked(Some("[mask]\nband_place = \"above\"\n"), "").0).map(|rect| rect.0), Some(-1200));
    assert_eq!(band_rect(&masked(Some("[mask]\nband_place = \"hidden\"\n"), "").0), None);
    assert_eq!(band_rect(&masked(Some("[mask]\nband_place = \"above\"\n"), "\nterminals {\n    band inside\n}\n").0), Some((0, 1200)));
    assert_eq!(band_rect(&masked(Some("[mask]\nband_place = \"inside\"\n"), "\nterminals {\n    band hidden\n}\n").0), None);
}

#[test]
fn the_outline_of_a_mask_is_drawn_and_a_function_with_its_own_keeps_it() {
    let items =
        "path (0, 0) (w, 0) (w, h) (0, h)\nleft terminal { path (0, -4) (-4, 0) (0, 4) }\nright terminal { path (0, -2) (4, -2) (4, 2) (0, 2) }";
    let manifest = format!("[mask]\noutline = '''\n{items}\n'''\n");
    let (sized, outlines, _) = masked(Some(&manifest), "");
    assert_eq!((sized.width, sized.height), (4000, 4000));
    assert_eq!((sized.inputs.clone(), sized.outputs.clone()), (vec![(400, 800)], vec![(4400, 800)]));
    // The polygon worked out by hand in `a_block_with_an_outline_draws_it_and_its_terminals_start_on_it`.
    let expected =
        vec![(0, 0), (4000, 0), (4000, 600), (4400, 600), (4400, 1000), (4000, 1000), (4000, 4000), (0, 4000), (0, 1200), (400, 800), (0, 400)];
    assert!(outline_polygons(&(sized, outlines)).contains(&expected));
    // The own `outline` section wins: `template block` is the ordinary block.
    let own = masked(Some(&manifest), "\noutline {\n    template block\n}\n");
    assert!(outline_polygons(&(own.0.clone(), own.1)).is_empty());
    assert_eq!((own.0.inputs, own.0.outputs), (vec![(0, 800)], vec![(4000, 800)]));
    // The D of the primitives, by the template of the mask.
    let d = masked(Some("[mask]\noutline = \"template d\"\n"), "");
    let polygons = outline_polygons(&(d.0, d.1));
    // With a band, the outline is drawn twice, the fill under the band and the stroke over it (`shaped`): the same points.
    assert_eq!(polygons.len(), 2, "{polygons:?}");
    assert_eq!(polygons[0], polygons[1]);
    for point in d_tangents(4000, 4000) {
        assert!(polygons[0].contains(&point), "{point:?}");
    }
}

#[test]
fn each_field_of_the_function_wins_alone_and_the_others_come_from_the_mask() {
    use crate::shape::Colour;
    // Mask `color`, function `fill`: the body is the fill of the function, the band the colour of the mask.
    let sized = masked(Some("[mask]\ncolor = \"#2F6F9F\"\n"), "\nterminals {\n    fill \"#64748B\"\n}\n").0;
    assert_eq!(fills(&sized).first(), Some(&Colour::Written(0x64_748B, 0x64_748B)));
    assert!(fills(&sized).contains(&Colour::Written(0x2F_6F9F, 0x2F_6F9F)), "{:?}", fills(&sized));
    // Mask `fill`, function `color`: the reverse.
    let sized = masked(Some("[mask]\nfill = \"#64748B\"\n"), "\nterminals {\n    color \"#2F6F9F\"\n}\n").0;
    assert_eq!(fills(&sized).first(), Some(&Colour::Written(0x64_748B, 0x64_748B)));
    assert!(fills(&sized).contains(&Colour::Written(0x2F_6F9F, 0x2F_6F9F)), "{:?}", fills(&sized));
    // The same field: the function wins.
    let sized = masked(Some("[mask]\nfill = \"#000000\"\n"), "\nterminals {\n    fill \"#64748B\"\n}\n").0;
    assert_eq!(fills(&sized).first(), Some(&Colour::Written(0x64_748B, 0x64_748B)));
}

#[test]
fn a_mask_with_only_the_default_band_place_draws_the_same_shapes_as_no_mask() {
    // Spec 11.4: a block with no `terminals` section has the places of an empty one; `inside` is the default (11.3).
    let plain = masked(None, "");
    let same = masked(Some("[mask]\nband_place = \"inside\"\n"), "");
    // `lanes` (the wires off their side) holds `None` for each terminal when a section exists, empty without one: not a shape.
    assert_eq!(plain.0.shapes, same.0.shapes);
    assert_eq!((plain.0.width, plain.0.height, &plain.0.inputs, &plain.0.outputs), (same.0.width, same.0.height, &same.0.inputs, &same.0.outputs));
    assert_eq!(plain.1, same.1);
}

// The block of the function drawn, alone, for the view `Block` (ide.md 3). Values worked out from spec 11.2: a block of
// `top 5` is (5 + 1) × 8 = 48 wide; without `left`, a small block keeps the 4 places of notation 3.2, 40 high.

/// The block of the function of `shown` among `files`.
fn own_of(files: &[(&str, &str)], manifest: Option<&str>, shown: &str) -> crate::OwnBlock {
    let mut names = Interner::new();
    let mut root = LibrarySource::new(String::new(), String::new(), Origin::Project);
    let mut lib = LibrarySource::new("lib".to_owned(), "lib".to_owned(), Origin::Project);
    for (path, text) in files {
        match path.strip_prefix("lib/") {
            Some(file) => lib.files.push(source(file, text)),
            None => root.files.push(source(path, text)),
        }
    }
    lib.manifest = manifest.map(|text| Ok(text.to_owned()));
    root.sub_libraries.push(lib);
    let checked = check(root, &Nothing, &mut names, &Limits::default());
    let index = index_of(&checked, shown);
    // An export never has it: only a session asks for it.
    let Drawn::Drawing(drawing) = draw(&checked, index, &names, &ViewState::default(), &DiagramLimits::default()) else { panic!("not drawn") };
    assert!(drawing.block.is_none());
    crate::own_block(&checked, index, &names, &DiagramLimits::default()).expect("the block of the function")
}

#[test]
fn the_block_of_a_function_is_drawn_alone_with_what_its_section_writes() {
    let g = "labcraft 1\n\nfn g(x: f64, y: f64) -> (n: i64) {\n    n = 1\n}\n\nterminals {\n    top 5\n    top 3 y\n    color \"#2f6f9f\"\n}\n";
    let own = own_of(&[("g.lcf", g)], None, "g.lcf");
    // One node, its box 48 × 40, inside the drawing, with a margin around what it draws.
    assert_eq!(own.drawing.placed.len(), 1);
    let placed = &own.drawing.placed[0];
    assert_eq!((placed.width, placed.height), (4800, 4000));
    assert_eq!(placed.callee, "g.lcf");
    assert!(own.drawing.loose.is_empty(), "no frame");
    for shape in own.drawing.groups.iter().flat_map(|group| group.shapes.iter()) {
        let (left, top, right, bottom) = crate::extent(shape);
        assert!(left >= 0 && top >= 0 && right <= own.drawing.width && bottom <= own.drawing.height, "{shape:?}");
    }
    let captions: Vec<&str> = own.drawing.ports.iter().filter(|port| !port.order).map(|port| port.caption.as_str()).collect();
    assert_eq!(captions, ["x", "y", "n"]);
    // The sizes in force; `top` written, `left` not. The line `top 3 y` needs 3 places on the top side; on the left and
    // the right, `x` and `n` need a place each, and a side grows by two places from 0 to hold them: 2.
    assert_eq!((own.top, own.left), (5, 4));
    assert_eq!((own.written.top, own.written.left), (Some(5), None));
    assert_eq!(own.least, (3, 2));
    assert_eq!(own.written.color.as_deref(), Some("#2F6F9F"));
    assert_eq!((own.written.fill.as_deref(), own.written.band.as_deref(), own.written.outline.as_ref()), (None, None, None));
    // The root library has no band, and the commands can change it.
    assert_eq!(own.library, "");
    assert_eq!(own.manifest, "labcraft.toml");
    assert_eq!(own.fixed, "");
    assert_eq!(own.mask, crate::BlockFields::default());
}

#[test]
fn the_block_of_a_function_says_what_the_mask_of_its_library_gives() {
    let g = format!("labcraft 1\n\n{G}\nterminals {{\n    band hidden\n}}\n\noutline {{\n    template block\n}}\n");
    let manifest = "[mask]\nband_place = \"above\"\ncolor = \"#B45309\"\nstroke = \"#78350f\"\noutline = '''\ntemplate d\n'''\n";
    let own = own_of(&[("lib/g.lcf", &g)], Some(manifest), "g.lcf");
    assert_eq!(own.library, "lib");
    assert_eq!(own.mask.band.as_deref(), Some("above"));
    assert_eq!((own.mask.color.as_deref(), own.mask.fill.as_deref(), own.mask.stroke.as_deref()), (Some("#B45309"), None, Some("#78350F")));
    assert_eq!(own.mask.outline, Some(crate::OutlineChoice::Template("d".to_owned())));
    assert_eq!((own.mask.top, own.mask.left), (None, None));
    // Its own lines: the band, and the ordinary block in place of the D of the mask.
    assert_eq!(own.written.band.as_deref(), Some("hidden"));
    assert_eq!(own.written.outline, Some(crate::OutlineChoice::Template("block".to_owned())));
    assert_eq!(own.written.color, None);
}

#[test]
fn the_block_of_an_extern_or_of_a_drawn_outline_says_so() {
    let f = "labcraft 1\n\nextern rust(crate: \"some\", function: \"f\")\nfn f(a: f64) -> (b: f64)\n\noutline {\n    path (0, 0) (w, 0) (w, h) (0, h)\n}\n";
    let own = own_of(&[("f.lcf", f)], None, "f.lcf");
    assert!(own.fixed.contains("`extern`"), "{}", own.fixed);
    assert_eq!(own.written.outline, Some(crate::OutlineChoice::Path));
}

#[test]
fn the_least_left_side_holds_the_terminals_that_no_line_places() {
    // No section: six inputs on the left, grown by two places from 0 (spec 11.2): 6. No line asks for the top side: 1.
    let f = "labcraft 1

fn f(a: f64, b: f64, c: f64, d: f64, e: f64, g: f64) -> (n: i64) {
    n = 1
}
";
    let own = own_of(&[("f.lcf", f)], None, "f.lcf");
    assert_eq!(own.least, (1, 6));
    assert_eq!((own.written.top, own.written.left), (None, None));
}
