//! Pass 6, on small programs: what each code says, and what must stay silent.

use std::borrow::Cow;

use labcraft_types::{Interner, Limits};

use crate::{Content, Found, Libraries, LibrarySource, Origin, SourceFile, check};

struct Nothing;

impl Libraries for Nothing {
    fn find(&self, _: &str, _: Origin) -> Found {
        Found::NotFound
    }
}

/// The codes and the messages of a library of one file, `labcraft 1` and `use core` added.
fn judged(text: &str) -> Vec<(&'static str, String)> {
    let mut names = Interner::new();
    let text = format!("labcraft 1\n\nuse core\n\n{text}\n");
    let file = SourceFile { shown: "f.lcf".to_owned(), content: Content::Bytes(Cow::Owned(text.into_bytes())) };
    let source = LibrarySource { files: vec![file], ..LibrarySource::new(String::new(), String::new(), Origin::Project) };
    let mut report = check(source, &Nothing, &mut names, &Limits::default()).report;
    report.finish();
    report.diagnostics().iter().map(|diagnostic| (diagnostic.code().as_str(), diagnostic.message.clone())).collect()
}

fn codes(text: &str) -> Vec<&'static str> {
    judged(text).into_iter().map(|(code, _)| code).collect()
}

/// A function, then its sections.
fn with(function: &str, sections: &str) -> Vec<&'static str> {
    codes(&format!("{function}\n\n{sections}"))
}

const NOTHING: [&str; 0] = [];

// The panel.

const PANEL_FUNCTION: &str = "fn f(a: f64 = 1.0, b: f64 = 0.0) -> (y: f64, b: f64) {\n    y = core.add(a, 1.0)\n    b = core.add(b, 1.0)\n}";

#[test]
fn an_element_has_one_name() {
    assert_eq!(with(PANEL_FUNCTION, "panel {\n    control a: numeric\n    control a: numeric\n}"), ["LC5001"]);
    // Whatever the direction: `b` is both an input and an output, and still one element.
    assert_eq!(with(PANEL_FUNCTION, "panel {\n    control b: numeric\n    indicator b: numeric\n}"), ["LC5001"]);
}

#[test]
fn an_element_of_an_input_is_a_control_and_of_an_output_an_indicator() {
    assert_eq!(with(PANEL_FUNCTION, "panel {\n    indicator a: numeric\n}"), ["LC5002"]);
    assert_eq!(with(PANEL_FUNCTION, "panel {\n    control y: numeric\n}"), ["LC5002"]);
    // A name that is both takes either.
    assert_eq!(with(PANEL_FUNCTION, "panel {\n    control a: numeric\n    indicator b: numeric\n    indicator y: numeric\n}"), NOTHING);
    assert_eq!(with(PANEL_FUNCTION, "panel {\n    control b: numeric\n}"), NOTHING);
}

#[test]
fn a_kind_that_m0_does_not_know_works_without_a_widget() {
    assert_eq!(with(PANEL_FUNCTION, "panel {\n    indicator y: gauge(max: 10)\n}"), ["LC5003"]);
    for kind in ["numeric", "chart"] {
        assert_eq!(with(PANEL_FUNCTION, &format!("panel {{\n    indicator y: {kind}\n}}")), NOTHING, "{kind}");
    }
}

#[test]
fn the_panel_of_an_extern_is_judged_too() {
    let function = "extern rust(crate: \"c\", function: \"f\")\nfn f(a: f64) -> (err: error)";
    assert_eq!(with(function, "panel {\n    indicator a: numeric\n    indicator err: dial\n}"), ["LC5002", "LC5003"]);
}

// The layout: nodes.

const NODES: &str = "type Limits = struct { low: f64, high: f64 }\n\n\
fn f(a: f64, limits: Limits) -> (y: f64, c: bool) {\n    #add: s = core.add(a, 1.0)\n    k = 0.5\n    low = limits.low\n    \
same = s\n    y = core.mul(s, k)\n    c = match core_flag {\n        true => {\n            t = core.lt(low, same)\n            yield t\n        }\n        \
false => {\n            t = core.gt(low, same)\n            yield t\n        }\n    }\n    core_flag = core.lt(a, y)\n}";

/// The type of `NODES` goes in a file of its own.
fn layout_of_nodes(entries: &str) -> Vec<&'static str> {
    let (types, function) = NODES.split_once("\n\n").unwrap_or_default();
    let mut names = Interner::new();
    let files = [
        ("Limits.lct", format!("labcraft 1\n\n{types}\n")),
        ("f.lcf", format!("labcraft 1\n\nuse core\n\n{function}\n\nlayout {{\n{entries}\n}}\n")),
    ];
    let files = files
        .into_iter()
        .map(|(shown, text)| SourceFile { shown: shown.to_owned(), content: Content::Bytes(Cow::Owned(text.into_bytes())) })
        .collect();
    let source = LibrarySource { files, ..LibrarySource::new(String::new(), String::new(), Origin::Project) };
    let mut report = check(source, &Nothing, &mut names, &Limits::default()).report;
    report.finish();
    report.diagnostics().iter().map(|diagnostic| diagnostic.code().as_str()).collect()
}

#[test]
fn a_node_is_named_by_its_label_or_by_the_name_of_its_line() {
    for entry in ["node #add at (1, 2)", "node k at (1, 2)", "node y view standard", "node c at (-5, 1_000_000)", "node core_flag at (0, 0)"] {
        assert_eq!(layout_of_nodes(entry), NOTHING, "{entry}");
    }
    // A label takes the place of the name; an input or an output is a terminal of the frame; a second name and a field draw no node.
    for entry in
        ["node s at (1, 2)", "node a at (1, 2)", "node limits at (1, 2)", "node low at (1, 2)", "node same at (1, 2)", "node nowhere at (1, 2)"]
    {
        assert_eq!(layout_of_nodes(entry), ["LC5004"], "{entry}");
    }
}

#[test]
fn a_name_in_two_arms_needs_a_label() {
    assert_eq!(layout_of_nodes("node t at (1, 2)"), ["LC5006"]);
}

#[test]
fn a_node_has_one_entry() {
    assert_eq!(layout_of_nodes("node k at (1, 2)\nnode k view compact"), ["LC5007"]);
    assert_eq!(layout_of_nodes("node #add at (1, 2)\nnode #add at (3, 4)"), ["LC5007"]);
}

#[test]
fn a_coordinate_is_an_integer_in_the_range() {
    for point in ["(1.5, 2)", "(1e3, 2)", "(1, 1000001)", "(-1000001, 1)"] {
        assert_eq!(layout_of_nodes(&format!("node k at {point}")), ["LC5005"], "{point}");
        assert_eq!(layout_of_nodes(&format!("wire k -> y via (1, 1) {point}")), ["LC5005"], "{point}");
    }
    // Whatever the entry names.
    assert_eq!(layout_of_nodes("node nowhere at (1.5, 2)"), ["LC5004", "LC5005"]);
}

// The layout: wires.

/// `average`, `scaled_sum` and an order wire, as on the sheet of step 9b.
const WIRES: &str = "fn f(values: [f64], k: f64, gain: f64) -> (mean: f64, total: f64, stamp: timestamp, err: error) {\n    \
sum, count = for v in values, carry s = 0.0, carry n = 0 {\n        next n = core.add(n, 1)\n        next s = core.add(s, v)\n    }\n    \
mean = core.div(sum, count)\n    \
total = for v in values, carry acc = k {\n        part = core.mul(v, k)\n        grown = core.add(acc, part)\n        next acc = grown\n    }\n    \
sq = core.mul(gain, gain)\n    #settle: e1 = time.wait(1 s, after: sq)\n    stamp = time.now(after: #settle)\n    err = e1\n}";

fn wires(entries: &str) -> Vec<&'static str> {
    codes(&format!("use time\n\n{WIRES}\n\nlayout {{\n{entries}\n}}"))
}

#[test]
fn a_wire_starts_at_the_value_it_carries() {
    for entry in ["wire sum -> mean", "wire count -> mean", "wire count -> mean.b", "wire values -> sum", "wire e1 -> err", "wire sq -> #settle"] {
        assert_eq!(wires(entry), NOTHING, "{entry}");
    }
    // Not at a node with an output: `sum` and `count` are two wires.
    assert_eq!(wires("wire sum.count -> mean"), ["LC5004"]);
}

#[test]
fn a_wire_that_enters_a_structure_has_one_entry_per_frame() {
    // `k` gives the carry `acc` its first value, and is read inside by `part`: two wires to the loop, and one inside.
    assert_eq!(wires("wire k -> total.acc\nwire k -> total.k\nwire k -> part"), NOTHING);
    assert_eq!(wires("wire k -> total"), ["LC5006"]);
    // From inside, a wire to `next` ends at the structure, at the input of its carry.
    assert_eq!(wires("wire grown -> total"), NOTHING);
    assert_eq!(wires("wire grown -> total.acc"), NOTHING);
    assert_eq!(wires("wire part -> grown.b"), NOTHING);
    // The carry is a value inside: its wire starts at its name.
    assert_eq!(wires("wire acc -> grown"), NOTHING);
}

#[test]
fn an_order_wire_is_a_wire() {
    for entry in ["wire #settle -> stamp", "wire #settle -> stamp.after", "wire sq -> #settle.after"] {
        assert_eq!(wires(entry), NOTHING, "{entry}");
    }
    // `sq` goes to `sq` twice: `a` and `b`.
    assert_eq!(wires("wire gain -> sq"), ["LC5006"]);
    assert_eq!(wires("wire gain -> sq.a\nwire gain -> sq.b"), NOTHING);
}

#[test]
fn a_wire_entry_that_names_nothing_or_joins_nothing() {
    for entry in ["wire nowhere -> mean", "wire sum -> nowhere", "wire sum -> mean.z", "wire sum -> values", "wire #nowhere -> mean"] {
        assert_eq!(wires(entry), ["LC5004"], "{entry}");
    }
    // Both exist, with no wire between them.
    for entry in ["wire sum -> sq", "wire count -> mean.a", "wire sum -> stamp.after"] {
        assert_eq!(wires(entry), ["LC5006"], "{entry}");
    }
}

#[test]
fn a_wire_has_one_entry() {
    assert_eq!(wires("wire sum -> mean\nwire sum -> mean.a via (1, 1)"), ["LC5007"]);
    assert_eq!(wires("wire e1 -> err\nwire e1 -> err"), ["LC5007"]);
}

#[test]
fn the_messages_say_what_to_write() {
    let found = judged(&format!("use time\n\n{WIRES}\n\nlayout {{\n    wire k -> total\n    wire raw.x -> mean\n}}"));
    let messages: Vec<&str> = found.iter().map(|(_, message)| message.as_str()).collect();
    assert_eq!(
        messages,
        [
            "`wire k -> total` is ambiguous: 2 wires join them: name the input, `total.acc` or `total.k` (11)",
            "`wire raw.x -> …` names no wire: a wire starts at the name of the value it carries, without a terminal (11)",
        ]
    );
}

#[test]
fn a_second_name_is_the_same_wire_wherever_it_is_written() {
    // `x = a` comes after the line that reads `x`: the order of lines has no meaning (5.2).
    let text = "fn f(a: f64) -> (y: f64) {\n    y = core.add(x, 1.0)\n    x = a\n}";
    for entry in ["wire a -> y", "wire x -> y", "wire a -> y.a"] {
        assert_eq!(with(text, &format!("layout {{\n    {entry}\n}}")), NOTHING, "{entry}");
    }
    assert_eq!(with(text, "layout {\n    wire a -> y\n    wire x -> y.a\n}"), ["LC5007"]);
    // A chain of them, each link once.
    let chain = "fn f(a: f64) -> (y: f64) {\n    y = core.add(x3, 1.0)\n    x3 = x2\n    x2 = x1\n    x1 = a\n}";
    assert_eq!(with(chain, "layout {\n    wire a -> y\n    wire x2 -> y\n}"), ["LC5007"]);
}

#[test]
fn a_value_and_its_second_name_cross_an_edge_by_one_wire() {
    let text = "fn f(v: [f64], k: f64) -> (y: [f64]) {\n    j = k\n    y = for x in v {\n        a = core.add(x, k)\n        b = core.mul(a, j)\n        collect b\n    }\n}";
    for entry in ["wire k -> y", "wire j -> y", "wire k -> y.j", "wire j -> y.k"] {
        assert_eq!(with(text, &format!("layout {{\n    {entry}\n}}")), NOTHING, "{entry}");
    }
}

#[test]
fn a_field_read_is_a_value_of_its_own() {
    let entries = "wire low -> y\nwire high -> y";
    let text = "type Limits = struct { low: f64, high: f64 }";
    let function = "fn f(limits: Limits) -> (y: bool) {\n    low = limits.low\n    high = limits.high\n    y = core.lt(low, high)\n}";
    let mut names = Interner::new();
    let files =
        [("Limits.lct", format!("labcraft 1\n\n{text}\n")), ("f.lcf", format!("labcraft 1\n\nuse core\n\n{function}\n\nlayout {{\n{entries}\n}}\n"))];
    let files = files
        .into_iter()
        .map(|(shown, text)| SourceFile { shown: shown.to_owned(), content: Content::Bytes(Cow::Owned(text.into_bytes())) })
        .collect();
    let source = LibrarySource { files, ..LibrarySource::new(String::new(), String::new(), Origin::Project) };
    let mut report = check(source, &Nothing, &mut names, &Limits::default()).report;
    report.finish();
    assert_eq!(report.diagnostics().iter().map(|diagnostic| diagnostic.code().as_str()).collect::<Vec<_>>(), NOTHING);
}

#[test]
fn an_output_bound_to_a_value_and_read_back_is_a_second_name_of_it() {
    let text = "fn f(a: f64) -> (y: f64, z: f64) {\n    y = a\n    z = core.add(y, 1.0)\n}";
    for entry in ["wire a -> z.a", "wire y -> z", "wire a -> y", "wire y -> y"] {
        assert_eq!(with(text, &format!("layout {{\n    {entry}\n}}")), NOTHING, "{entry}");
    }
    assert_eq!(with(text, "layout {\n    wire a -> z\n    wire y -> z.a\n}"), ["LC5007"]);
}

#[test]
fn a_node_goes_to_the_output_that_its_line_binds() {
    let text = "fn f(a: f64) -> (y: f64) {\n    y = core.add(a, 1.0)\n}";
    assert_eq!(with(text, "layout {\n    wire y -> y\n    wire a -> y\n}"), NOTHING);
}

#[test]
fn an_input_named_after_is_not_always_the_order() {
    let text = "fn f(v: [f64], a: f64) -> (y: f64) {\n    after = core.add(a, 1.0)\n    y = for x in v, carry s = a {\n        next s = core.add(s, after)\n    }\n}";
    assert_eq!(with(text, "layout {\n    wire after -> y.after\n}"), NOTHING);
}

#[test]
fn a_mistake_in_the_signature_holds_the_entries_back() {
    let text = "fn f(a: f64, a: f64) -> (y: f64) {\n    y = core.add(a, 1.0)\n}";
    assert_eq!(with(text, "layout {\n    wire b -> y\n}"), ["LC2019"]);
}

#[test]
fn a_panel_says_the_same_whatever_the_order_of_its_items() {
    let first = with(PANEL_FUNCTION, "panel {\n    indicator a: numeric\n    control a: gauge\n}");
    let second = with(PANEL_FUNCTION, "panel {\n    control a: gauge\n    indicator a: numeric\n}");
    let (mut first, mut second) = (first, second);
    first.sort_unstable();
    second.sort_unstable();
    assert_eq!(first, ["LC5001", "LC5002", "LC5003"]);
    assert_eq!(first, second);
}

#[test]
fn without_a_terminal_an_entry_takes_the_input_without_a_name() {
    // `stop` crosses the edge, named `stop`, and inside goes to `until`, which has no name.
    let repeat =
        "fn f(stop: bool, step: i32) -> (n: i32) {\n    n = loop carry i = 0 {\n        next i = core.add(i, step)\n        until stop\n    }\n}";
    assert_eq!(with(repeat, "layout {\n    wire stop -> n\n    wire stop -> n.stop\n}"), NOTHING);
    // The selector, which has no name, and the value that crosses the edge, named `c`.
    let choose = "fn f(c: bool, a: f64) -> (y: f64) {\n    y = match c {\n        true => {\n            off = core.not(c)\n            z = core.select(off, a, 0.0)\n            \
                  yield z\n        }\n        false => yield a\n    }\n}";
    assert_eq!(with(choose, "layout {\n    wire c -> y\n    wire c -> y.c\n    wire c -> off\n}"), NOTHING);
}

#[test]
fn what_no_name_tells_apart_keeps_an_automatic_route() {
    // The selector, yielded as it is by an arm: two inputs without a name, besides the one where `c` crosses the edge.
    let selector = "fn f(c: bool) -> (y: bool) {\n    y = match c {\n        true => yield c\n        false => yield false\n    }\n}";
    let found = judged(&format!("{selector}\n\nlayout {{\n    wire c -> y\n}}"));
    assert_eq!(found, [("LC5006", "`wire c -> y` is ambiguous: 3 wires join them (11)".to_owned())]);
    // One value yielded by two arms.
    let twice = "fn f(c: bool, a: f64) -> (y: f64) {\n    y = match c {\n        true => yield a\n        false => yield a\n    }\n}";
    assert_eq!(with(twice, "layout {\n    wire a -> y.a\n}"), NOTHING);
    assert_eq!(with(twice, "layout {\n    wire a -> y\n}"), ["LC5006"]);
}

#[test]
fn a_node_whose_output_has_no_name_starts_at_its_label() {
    let text = "fn f(v: [f64]) -> (n: i32) {\n    n = for x in v, carry k = 0 {\n        #inc: next k = core.add(k, 1)\n    }\n}";
    assert_eq!(with(text, "layout {\n    wire #inc -> n.k\n    wire #inc -> n\n}"), ["LC5007"]);
    assert_eq!(with(text, "layout {\n    wire #inc -> n.k\n    wire k -> #inc\n}"), NOTHING);
}

#[test]
fn a_value_read_in_two_arms_crosses_the_edge_once() {
    let text = "fn f(a: f64, c: bool) -> (y: f64) {\n    y = match c {\n        true => {\n            p = core.add(a, 1.0)\n            yield p\n        }\n        \
                false => {\n            q = core.mul(a, 2.0)\n            yield q\n        }\n    }\n}";
    assert_eq!(with(text, "layout {\n    wire a -> y\n    wire a -> p\n    wire a -> q\n    wire p -> y\n}"), NOTHING);
    // The selector has no name.
    assert_eq!(with(text, "layout {\n    wire c -> y\n}"), NOTHING);
}

#[test]
fn entries_wait_for_the_errors_of_the_function() {
    // `nowhere` has its `LC2001`, and the entry, which no wire could match, says nothing.
    let text = "fn f(a: f64) -> (y: f64) {\n    y = core.add(a, nowhere)\n}";
    assert_eq!(with(text, "layout {\n    wire a -> y\n    wire b -> y\n}"), ["LC2001"]);
    // Coordinates do not wait.
    assert_eq!(with(text, "layout {\n    node y at (1.5, 0)\n}"), ["LC2001", "LC5005"]);
}

#[test]
fn an_extern_has_no_diagram() {
    let function = "extern rust(crate: \"c\", function: \"f\")\nfn f(a: f64) -> (err: error)";
    assert_eq!(with(function, "layout {\n    node a at (1, 2)\n    wire a -> err\n}"), ["LC5004", "LC5004"]);
}

// The icon.

const ICON_FUNCTION: &str = "fn f() -> (y: f64) {\n    y = 1.0\n}";

fn icon(items: &str) -> Vec<&'static str> {
    with(ICON_FUNCTION, &format!("icon 32 {{\n{items}\n}}"))
}

#[test]
fn the_icon_of_check_voltage_is_valid() {
    let items = "rect (2, 4) (30, 28) radius 4\npath \"M7 21 A9 9 0 0 1 25 21\"\nline (16, 21) (21, 13)\ntext \"V\" at (16, 27) size 6";
    assert_eq!(icon(items), NOTHING);
    assert_eq!(icon("circle (16, 16) 16 fill accent wire.error\nrect (0, 0) (32, 32) radius 40\nline (0.5, 0) (31.5, 32)"), NOTHING);
}

#[test]
fn the_grid_is_written_32() {
    for grid in ["0", "64", "032", "3_2"] {
        assert_eq!(with(ICON_FUNCTION, &format!("icon {grid} {{}}")), ["LC5008"], "{grid}");
    }
}

#[test]
fn an_invalid_shape_is_an_error() {
    for item in [
        "circle (16, 16) 0",
        "circle (16, 16) -2",
        "rect (1, 1) (2, 2) radius 0",
        "text \"V\" at (16, 16) size 0",
        "line (1e999, 0) (1, 1)",
        "path \"L 1 1\"",
        "path \"M1 1 A 1 1 0 2 0 3 3\"",
        "line (1, 1) (2, 2) accent wire.nothing",
        "line (1, 1) (2, 2) accent canvas",
        // The colour of a selection in the viewer (decision of the owner, 2026-11-13).
        "line (1, 1) (2, 2) accent select.stroke",
        "line (1, 1) (2, 2) accent node.active",
    ] {
        assert_eq!(icon(item), ["LC5008"], "{item}");
    }
}

#[test]
fn a_shape_that_leaves_the_grid_is_a_warning() {
    for item in ["rect (-1, 0) (10, 10)", "circle (16, 16) 17", "line (0, 0) (33, 1)", "path \"M7 5 A9 9 0 0 1 25 5\"", "text \"V\" at (40, 4)"] {
        assert_eq!(icon(item), ["LC5009"], "{item}");
    }
    // A path that comes back where it started draws nothing, nor does an empty text.
    assert_eq!(icon("path \"M40 40 Z\""), NOTHING);
    assert_eq!(icon("path \"M40 40 L40 40\""), NOTHING);
    assert_eq!(icon("line (40, 40) (40, 40)"), NOTHING);
    assert_eq!(icon("text \"\" at (40, 40)"), NOTHING);
}

/// Spec 11.1: a text by its box, as wide as its cells times 1233/2048 of its size, centered on `at`, from 1493/2048 of its size
/// above the baseline to 483/2048 below it. Worked out by hand: at size 10 a cell is 6.0205 wide, the capitals 7.2900 high and
/// the descent 2.3584; at 14 a cell is 8.4287 wide.
#[test]
fn a_text_leaves_the_grid_by_its_box() {
    for item in [
        "text \"a long label\" at (16, 16) size 30",     // 12 cells: 216.7 wide
        "text \"V\" at (1, 16)",                         // from x -2.01
        "text \"V\" at (31, 16)",                        // to x 34.01
        "text \"V\" at (16, 7.28)",                      // from y -0.0100
        "text \"V\" at (16, 29.65)",                     // to y 32.0084
        "text \"\u{4E2D}\u{4E2D}\" at (16, 20) size 14", // 4 cells: 33.71 wide
    ] {
        assert_eq!(icon(item), ["LC5009"], "{item}");
    }
    for item in [
        "text \"V\" at (16, 27) size 6",
        "text \"V\" at (3.02, 16)",                              // from x 0.0098
        "text \"V\" at (16, 7.3)",                               // from y 0.0100
        "text \"V\" at (16, 29.64)",                             // to y 31.9984
        "text \"abc\" at (16, 20) size 14",                      // 3 cells: 25.29 wide
        "text \"e\u{301}e\u{301}e\u{301}\" at (16, 20) size 14", // 3 graphemes of 2 code points: 3 cells
    ] {
        assert_eq!(icon(item), NOTHING, "{item}");
    }
}

/// A bidirectional control is drawn as its code, `⟨U+202E⟩`, 8 cells: 33.7 wide at 7, where the character alone would be 4.2.
#[test]
fn a_text_counts_the_cells_it_is_drawn_with() {
    assert_eq!(icon("text \"\u{202E}\" at (16, 20) size 7"), ["LC5009", "LC0005"]);
    assert_eq!(icon("text \"\u{202E}\" at (16, 20) size 6"), ["LC0005"]);
    // The text as it runs, its escapes read: `a"b` and `\"` are 3 cells, 25.29 wide at 14; written, they would be 4 or more.
    assert_eq!(icon("text \"a\\\"b\" at (16, 20) size 14"), NOTHING);
    assert_eq!(icon("text \"\\\\\\\\\\\\\" at (16, 20) size 14"), NOTHING);
    assert_eq!(icon("text \"a\\\\\\\\\\\\b\" at (16, 20) size 14"), ["LC5009"]);
    // A finite size can give a box past every number: it leaves the grid all the same.
    assert_eq!(icon("text \"VVVV\" at (16, 16) size 1e308"), ["LC5009"]);
}

/// 11.1: at most 256 shapes, and 4 096 segments for all the paths of an icon; one more, and its calls show the label.
/// A path `M0 0` followed by `k` times `L1 1` has 1 + k segments: two of 2 048 make 4 096.
#[test]
fn an_icon_has_at_most_256_shapes_and_4096_segments_of_path() {
    let circles = |count: usize| vec!["circle (16, 16) 8"; count].join("\n");
    assert_eq!(icon(&circles(256)), NOTHING);
    assert_eq!(icon(&circles(257)), ["LC5008"]);
    let path = |lines: usize| format!("path \"M0 0{}\"", " L1 1".repeat(lines));
    assert_eq!(icon(&format!("{}\n{}", path(2047), path(2047))), NOTHING);
    assert_eq!(icon(&format!("{}\n{}", path(2047), path(2048))), ["LC5008"]);
}

/// 11.1: at most 64 KiB for the `path` strings of an icon together, whatever their segments: `M0 0` and spaces, a path of
/// one segment, at 65 536 bytes and at one more. An arc that ends where it starts counts as a segment too: `M0 0`, then
/// 4 095 such arcs, make 4 096, one more is past the bound.
#[test]
fn the_paths_of_an_icon_have_at_most_64_kib_and_every_arc_counts() {
    let spaces = |bytes: usize| format!("path \"M0 0{}\"", " ".repeat(bytes - 4));
    assert_eq!(icon(&spaces(65_536)), NOTHING);
    assert_eq!(icon(&spaces(65_537)), ["LC5008"]);
    let arcs = |count: usize| format!("path \"M0 0{}\"", " A1 1 0 0 0 0 0".repeat(count));
    assert_eq!(icon(&arcs(4095)), NOTHING);
    assert_eq!(icon(&arcs(4096)), ["LC5008"]);
}

#[test]
fn the_icon_of_an_extern_is_judged_too() {
    let function = "extern rust(crate: \"c\", function: \"f\")\nfn f(a: f64) -> (err: error)";
    assert_eq!(with(function, "icon 16 {}"), ["LC5008"]);
}

// The limits: past them, nothing more is said of the wires.

/// Deep in nested loops, on the stack of a host: the test thread is too small for 200 levels in debug.
fn on_a_large_stack<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new().stack_size(labcraft_format::STACK_SIZE).spawn(work).unwrap().join().unwrap()
}

/// Each of `count` values is read under `depth` loops and has an entry, and a last entry names a value that does not go to
/// its node.
fn values_under_loops(count: usize, depth: usize) -> String {
    let values: String = (0..count).map(|index| format!("    v{index} = 1\n")).collect();
    let open: String = (0..depth).map(|level| format!("    _ = for i{level} in [1] {{\n")).collect();
    let reads: String = (0..count).map(|index| format!("    b{index} = core.add(v{index}, 1)\n")).collect();
    let close = "    }\n".repeat(depth);
    let entries: String = (0..count).map(|index| format!("    wire v{index} -> b{index}\n")).collect();
    format!("fn f() -> () {{\n{values}{open}{reads}{close}}}\n\nlayout {{\n{entries}    wire v0 -> b1\n}}")
}

#[test]
fn past_the_limit_of_wires_nothing_is_said_of_them() {
    let pass_6 = |text: String| codes(&text).into_iter().filter(|code| code.starts_with("LC5")).collect::<Vec<_>>();
    // Each value crosses each loop on its way: about 201 wires for one read, far more than the file has places, so the limit is
    // 2^18 wires, between 1,298 and 1,306 values.
    assert_eq!(on_a_large_stack(move || pass_6(values_under_loops(1_290, 200))), ["LC5006"]);
    assert_eq!(on_a_large_stack(move || pass_6(values_under_loops(1_310, 200))), NOTHING);
}

/// `x` and `y` in each of `arms` arms, an entry from `x` to `y`, then one that names a value that does not go to its node.
fn names_in_many_arms(arms: usize) -> String {
    let arms: String = (0..arms)
        .map(|arm| format!("        {arm} => {{\n            x = 1\n            y = core.add(x, 1)\n            yield y\n        }}\n"))
        .collect();
    format!(
        "fn f(n: i32) -> (r: i32, z: i32) {{\n    w = 2\n    z = core.add(n, 1)\n    r = match n {{\n{arms}        _ => yield 0\n    }}\n}}\n\nlayout {{\n    wire x -> y\n    wire w -> z\n}}"
    )
}

#[test]
fn past_the_budget_of_comparisons_nothing_more_is_said() {
    let pass_6 = |text: String| codes(&text).into_iter().filter(|code| code.starts_with("LC5")).collect::<Vec<_>>();
    // `wire x -> y` is ambiguous, and `w` does not go to `z`.
    assert_eq!(pass_6(names_in_many_arms(3)), ["LC5006", "LC5006"]);
    // Each `x` is compared with each `y`: 4,096 squared is the budget, 2^24, which the first entry spends, so the second says
    // nothing either.
    assert_eq!(pass_6(names_in_many_arms(4_090)), ["LC5006", "LC5006"]);
    assert_eq!(pass_6(names_in_many_arms(4_100)), NOTHING);
}

/// Whether `check` says that a budget or a limit made it say less, which the fuzzing of `check` reads.
fn cut_short(text: &str) -> bool {
    let text = format!("labcraft 1\n\nuse core\n\n{text}\n");
    let file = SourceFile { shown: "f.lcf".to_owned(), content: Content::Bytes(Cow::Owned(text.into_bytes())) };
    let source = LibrarySource { files: vec![file], ..LibrarySource::new(String::new(), String::new(), Origin::Project) };
    check(source, &Nothing, &mut Interner::new(), &Limits::default()).cut_short
}

#[test]
fn a_limit_reached_is_known() {
    assert!(!cut_short(&names_in_many_arms(4_090)));
    assert!(cut_short(&names_in_many_arms(4_100)));
    assert!(!on_a_large_stack(|| cut_short(&values_under_loops(1_290, 200))));
    assert!(on_a_large_stack(|| cut_short(&values_under_loops(1_310, 200))));
}

// The layout: the size of a frame (`frame size`, `frame NAME size`).

#[test]
fn a_frame_size_names_a_structure_or_the_function() {
    // `total` and `sum` are loops of `WIRES`; no diagnostic.
    for entry in ["frame size (600, 300)", "frame total size (320, 200)", "frame sum size (1, 2)", "frame total size (1, 2)\nframe size (3, 4)"] {
        assert_eq!(wires(entry), NOTHING, "{entry}");
    }
}

#[test]
fn a_frame_size_on_a_node_that_is_no_structure_names_nothing() {
    // `mean` is a call.
    assert_eq!(wires("frame mean size (320, 200)"), ["LC5004"]);
}

#[test]
fn a_frame_has_one_size() {
    assert_eq!(wires("frame total size (1, 2)\nframe total size (3, 4)"), ["LC5007"]);
    assert_eq!(wires("frame size (1, 2)\nframe size (3, 4)"), ["LC5007"]);
}

#[test]
fn the_coordinates_of_a_frame_size_are_integers_in_the_range() {
    assert_eq!(wires("frame size (5.0, 1)"), ["LC5005"]);
    assert_eq!(wires("frame total size (1, 1000001)"), ["LC5005"]);
}

// The layout: the height of a terminal of the frame (`input x at y`, `output x at y`).

#[test]
fn a_terminal_entry_names_an_input_or_an_output_of_its_kind() {
    // Spec 11: `values`, `k`, `gain` are inputs of `WIRES`; `mean`, `err` outputs. No diagnostic.
    for entry in ["input values at 40", "input gain at -16", "output mean at 120", "output err at 0", "input k at 8\noutput mean at 8"] {
        assert_eq!(wires(entry), NOTHING, "{entry}");
    }
    // An input that is no output, an output that is no input, and a name of neither.
    for entry in ["output values at 40", "input mean at 40", "input sq at 40", "output nothing at 40"] {
        assert_eq!(wires(entry), ["LC5004"], "{entry}");
    }
}

#[test]
fn a_terminal_has_one_entry() {
    assert_eq!(wires("input k at 8\ninput k at 64"), ["LC5007"]);
    assert_eq!(wires("output err at 8\noutput err at 64"), ["LC5007"]);
}

#[test]
fn the_height_of_a_terminal_is_an_integer_in_the_range() {
    assert_eq!(wires("input k at 5.0"), ["LC5005"]);
    assert_eq!(wires("output mean at 1000001"), ["LC5005"]);
    assert_eq!(wires("output mean at -1000000"), NOTHING);
}

// The terminals of the block (spec 11.2): `LC5014` for a line that cannot apply (a warning), `LC5015` for a section that
// contradicts itself (an error).

const BLOCK_FUNCTION: &str = "use time\n\nfn f(a: f64, err: error, b: f64) -> (y: f64, err: error) {\n    y = core.add(a, b)\n    e1 = time.wait(1 s, after: y)\n    err = e1\n}";

fn block(lines: &str) -> Vec<&'static str> {
    with(BLOCK_FUNCTION, &format!("terminals {{\n{lines}\n}}"))
}

#[test]
fn a_clean_terminals_section_says_nothing() {
    assert_eq!(block("top 5\nleft 4\ntop 3 a\nleft middle b\nright 4 y"), NOTHING);
    // `left 4` with a `middle` has 5 places, `top 4` likewise: place 5 is there.
    assert_eq!(block("left 4\nleft middle a\nleft 5 b"), NOTHING);
    assert_eq!(block("top 4\ntop middle a\ntop 5 b"), NOTHING);
    // Both terminals of the pair named: each keeps its own place, nothing is taken twice.
    assert_eq!(block("left 2 err\nright 2 err"), NOTHING);
    assert_eq!(block("left 2 err\nright 3 output err"), NOTHING);
    // The default top side has 4 places: 2 to 4 are free; the left side without `left N` grows to hold what it is given.
    assert_eq!(block("top 4 a\nleft 40 b\nband above"), NOTHING);
}

#[test]
fn two_sizes_for_one_side_two_bands_two_lines_for_one_terminal_or_a_size_outside_its_range_are_an_error() {
    for lines in [
        "top 4\ntop 5",
        "left 3\nleft 4",
        "band above\nband hidden",
        "left 3 a\nleft 4 a",
        "left 2 err\ntop 3 err",
        "top 0",
        "top 65",
        "left 65",
        "left 0_65",
        "left 99999999999",
    ] {
        assert_eq!(block(lines), ["LC5015"], "{lines}");
    }
    // 1 and 64 are in the range, and 2 too; the left side may have no place.
    assert_eq!(block("top 1\nleft 64"), NOTHING);
    assert_eq!(block("top 5\nleft 0\ntop 3 a"), NOTHING);
    assert_eq!(block("top 64\nleft 1"), NOTHING);
    assert_eq!(block("top 2\nleft 2"), NOTHING);
}

#[test]
fn a_colour_is_a_string_hash_rrggbb_in_either_case_once_for_each_line() {
    assert_eq!(
        block(
            "color \"#2F6F9F\"
fill \"#64748b\"
stroke \"#000000\""
        ),
        NOTHING
    );
    for lines in ["color \"#2F6F9\"", "fill \"64748B\"", "stroke \"#GGGGGG\"", "color \"red\"", "fill \"#64748B \"", "stroke \"\""] {
        assert_eq!(block(lines), ["LC5018"], "{lines}");
    }
    for lines in [
        "color \"#2F6F9F\"
color \"#2F6F9F\"",
        "fill \"#000000\"
fill \"#FFFFFF\"",
        "stroke \"#000000\"
stroke \"x\"",
    ] {
        assert_eq!(block(lines), ["LC5015"], "{lines}");
    }
}

#[test]
fn a_width_outside_its_bounds_is_drawn_at_the_nearest_and_one_that_is_no_number_refuses_the_icon() {
    for item in ["line (4, 4) (28, 28) width 0.25", "line (4, 4) (28, 28) width 8", "circle (16, 16) 4 width 1.5 fill"] {
        assert_eq!(icon(item), NOTHING, "{item}");
    }
    for item in ["line (4, 4) (28, 28) width 0.2", "line (4, 4) (28, 28) width 9", "line (4, 4) (28, 28) width 0", "line (4, 4) (28, 28) width -1"] {
        assert_eq!(icon(item), ["LC5019"], "{item}");
    }
    assert_eq!(icon("line (4, 4) (28, 28) width inf"), ["LC5008"]);
}

#[test]
fn a_size_outside_the_range_counts_at_the_nearest_bound() {
    // `top 0` counts as `top 1`, whose only place is that of `after:`: a terminal put on place 2 is past the side.
    assert_eq!(block("top 0\ntop 2 a"), ["LC5015", "LC5014"]);
    // `top 99` counts as `top 64`, which holds place 64; `left 70` as `left 64`, which does not hold place 65.
    assert_eq!(block("top 99\ntop 64 a"), ["LC5015"]);
    assert_eq!(block("left 70\nleft 65 a"), ["LC5015", "LC5014"]);
    // Digits are read whole, as `fmt` reads them: 300 zeros then 2 is `left 2`, which holds place 2.
    let zeros = "0".repeat(300);
    assert_eq!(block(&format!("left {zeros}2\nleft {zeros}2 a")), NOTHING);
}

#[test]
fn a_line_that_cannot_apply_is_a_warning() {
    for lines in [
        // Names no terminal, or no terminal of that kind.
        "left 2 zz",
        "left 2 input y",
        "left 2 output a",
        "left 2 output b",
        // Place 0, past the last place of the side, the first place of the top side.
        "left 0 a",
        "top 5 a",
        "top 3\ntop 4 a",
        "left 3\nleft 4 a",
        "left 3\nright 4 y",
        "top 1 a",
        // `middle` makes the count odd, 4 becomes 5, so 6 is still past.
        "top 4\ntop middle a\ntop 6 b",
        // Another line took the place, on the same side.
        "left 2 a\nleft 2 b",
        "top 3 a\ntop 3 b",
        // A pair named alone takes the same place on the facing side: `err` on the right 2 takes the left 2 too.
        "right 2 err\nleft 2 a",
        "left 2 err\nright 2 y",
    ] {
        assert_eq!(block(lines), ["LC5014"], "{lines}");
    }
}

#[test]
fn of_two_lines_on_one_place_the_first_in_signature_order_applies() {
    // `b` comes after `a` in the signature, so its line is the one that does not apply, whatever the written order (11.2).
    for lines in [
        "left 2 b
left 2 a",
        "left 2 a
left 2 b",
    ] {
        let found = judged(&format!(
            "{BLOCK_FUNCTION}

terminals {{
{lines}
}}"
        ));
        assert_eq!(found.len(), 1, "{lines}: {found:?}");
        assert!(found.iter().all(|(code, message)| *code == "LC5014" && message.contains("`b`")), "{lines}: {found:?}");
    }
}

/// Worked out by hand from spec 11.2 and notation 3.1: `f` has three lines of terminals, `a` beside `y`, `b`, then the
/// `err` pair, so a side that grows holds 4 places, 5 with a `middle`, whose middle is place 3 (#250).
#[test]
fn a_middle_or_a_last_on_a_side_that_grows_takes_its_place_before_a_number() {
    for (lines, refused) in [
        // 5 places: `middle` is 3, which `a` takes first, in signature order.
        ("left middle a\nleft 3 b", "`b`"),
        // `left 4 b` makes the side 4 places, whose last is 4.
        ("left last a\nleft 4 b", "`b`"),
        // A pair named alone on the middle takes the same place of the facing side, before an output.
        ("left middle err\nright 3 y", "`y`"),
    ] {
        let found = judged(&format!("{BLOCK_FUNCTION}\n\nterminals {{\n{lines}\n}}"));
        assert_eq!(found.len(), 1, "{lines}: {found:?}");
        assert!(found.iter().all(|(code, message)| *code == "LC5014" && message.contains(refused)), "{lines}: {found:?}");
    }
    // On a place that the middle leaves free, both apply.
    assert_eq!(block("left middle a\nleft 4 b"), NOTHING);
    assert_eq!(block("left middle a\nright 3 y"), NOTHING);
}

#[test]
fn a_middle_counts_only_on_a_line_that_names_a_terminal() {
    // `top middle zz` names no terminal (LC5014): it does not make the top side odd, so the default 4 places stay 4 and
    // `top 5 a` is past the last, as the drawing counts them (11.2).
    assert_eq!(
        block(
            "top middle zz
top 5 a"
        ),
        ["LC5014", "LC5014"]
    );
}

// The `outline` section (11.4). `f` has 3 lines of terminals: its block is 40 × 40, `a` on the left at 8, `b` at 16, the
// `err` pair on the last place, 32, and `y` on the right at 8.

fn outline(items: &str) -> Vec<&'static str> {
    with(BLOCK_FUNCTION, &format!("outline {{\n{items}\n}}"))
}

#[test]
fn a_clean_outline_says_nothing() {
    assert_eq!(outline("template d"), NOTHING);
    assert_eq!(outline("template block"), NOTHING);
    assert_eq!(outline("path (0, 0) (w, 0) round 4 (w, h) (0, h)\nright terminal { path (0, -2) (4, 0) (0, 2) }"), NOTHING);
    // A coordinate anchored on a terminal of the left side, as a height.
    assert_eq!(outline("path (4, 0) (w, 0) (w, h) (4, h) (0, b + 2) (0, b - 2)"), NOTHING);
}

#[test]
fn an_outline_that_cannot_be_read_as_one_is_an_error() {
    for items in ["template d\ntemplate block", "template d\npath (0, 0) (w, 0) (w, h)", "template e", "path (0, 0) (w, 0) round 17 (w, h)"] {
        assert_eq!(outline(items), ["LC5016"], "{items}");
    }
}

#[test]
fn an_outline_that_breaks_a_guard_at_the_size_of_its_section_is_a_warning() {
    for items in [
        "path (0, 0) (w, 0) (w, 41) (0, h)",
        "path (0, 0) (w, zz) (w, h) (0, h)",
        "path (0, 0) (w, h) (w, 0) (0, h)",
        "path (8, 0) (w, 0) (w, h) (8, h)",
    ] {
        assert_eq!(outline(items), ["LC5017"], "{items}");
    }
    // The size of the section counts: `left 5` makes the block 48 high.
    assert_eq!(with(BLOCK_FUNCTION, "terminals {\n    left 5\n}\n\noutline {\n    path (0, 0) (w, 0) (w, 48) (0, h)\n}"), NOTHING);
    assert_eq!(with(BLOCK_FUNCTION, "terminals {\n    left 5\n}\n\noutline {\n    path (0, 0) (w, 0) (w, 49) (0, h)\n}"), ["LC5017"]);
}

/// The diagnostics of a library of several files, each with its own name.
fn judged_files(files: &[(&str, &str)]) -> Vec<(String, &'static str)> {
    let mut names = Interner::new();
    let files = files
        .iter()
        .map(|(shown, text)| SourceFile {
            shown: (*shown).to_owned(),
            content: Content::Bytes(Cow::Owned(format!("labcraft 1\n\nuse core\n\n{text}\n").into_bytes())),
        })
        .collect();
    let source = LibrarySource { files, ..LibrarySource::new(String::new(), String::new(), Origin::Project) };
    let mut report = check(source, &Nothing, &mut names, &Limits::default()).report;
    report.finish();
    report.diagnostics().iter().map(|diagnostic| (diagnostic.file.clone(), diagnostic.code().as_str())).collect()
}

#[test]
fn an_outline_is_judged_at_the_size_of_each_call_once_for_its_item() {
    // `g` has 2 lines of terminals with one value of `rest`: 40 high, a rectangle with its corner (0, 40) on (0, h). A call
    // with 5 values has 6 lines: 56 high (4 places grow by two to 6), and the last value of `rest`, on the left at 48,
    // meets the outline at 20 from the left edge: past 4 (11.4). Two such calls give one warning, in the file of `g`.
    let g = "fn g(first: f64, rest: f64...) -> (y: f64) {\n    y = first\n}\n\noutline {\n    path (0, 0) (w, 0) (w, h) (0, 40)\n}";
    let small = "fn small(x: f64) -> (y: f64) {\n    y = g(x, 1.0)\n}";
    assert_eq!(judged_files(&[("g.lcf", g), ("small.lcf", small)]), []);
    let large = "fn large(x: f64) -> (y: f64, z: f64) {\n    y = g(x, 1.0, 2.0, 3.0, 4.0, 5.0)\n    z = g(x, 5.0, 4.0, 3.0, 2.0, 1.0)\n}";
    assert_eq!(judged_files(&[("g.lcf", g), ("large.lcf", large)]), [("g.lcf".to_owned(), "LC5017")]);
}

#[test]
fn a_line_on_a_side_that_grows_is_judged_at_the_size_of_each_call() {
    // `g(a, b, rest...)` with `left middle a` and `left 5 b` (#250). Worked out by hand from spec 11.2:
    // - one value of `rest`: 3 lines, at least 5 places for `left 5`, so 6, odd with the middle: 7; the middle is 4, and
    //   `b` keeps 5: nothing to say;
    // - five values: 7 lines, so 8, odd: 9; the middle is 5, which `a` takes first, in signature order: `b` keeps its
    //   default place, one LC5014 in the file of `g`, once for two such calls.
    let g = "fn g(a: f64, b: f64, rest: f64...) -> (y: f64) {\n    y = a\n}\n\nterminals {\n    left middle a\n    left 5 b\n}";
    let one = "fn one(x: f64) -> (y: f64) {\n    y = g(x, x, x)\n}";
    assert_eq!(judged_files(&[("g.lcf", g), ("one.lcf", one)]), []);
    let five = "fn five(x: f64) -> (y: f64, z: f64) {\n    y = g(x, x, x, x, x, x, x)\n    z = g(x, x, 1.0, 2.0, 3.0, 4.0, 5.0)\n}";
    assert_eq!(judged_files(&[("g.lcf", g), ("five.lcf", five)]), [("g.lcf".to_owned(), "LC5014")]);
}
