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
        ("limits.lct", format!("labcraft 1\n\n{types}\n")),
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
        [("limits.lct", format!("labcraft 1\n\n{text}\n")), ("f.lcf", format!("labcraft 1\n\nuse core\n\n{function}\n\nlayout {{\n{entries}\n}}\n"))];
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
    ] {
        assert_eq!(icon(item), ["LC5008"], "{item}");
    }
}

#[test]
fn a_shape_that_leaves_the_grid_is_a_warning() {
    for item in ["rect (-1, 0) (10, 10)", "circle (16, 16) 17", "line (0, 0) (33, 1)", "path \"M7 5 A9 9 0 0 1 25 5\"", "text \"V\" at (40, 4)"] {
        assert_eq!(icon(item), ["LC5009"], "{item}");
    }
    // A text counts by its point: the IDE chooses the font. A path that comes back where it started draws nothing.
    assert_eq!(icon("text \"a long label\" at (16, 16) size 30"), NOTHING);
    assert_eq!(icon("path \"M40 40 Z\""), NOTHING);
    assert_eq!(icon("path \"M40 40 L40 40\""), NOTHING);
    assert_eq!(icon("line (40, 40) (40, 40)"), NOTHING);
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
