//! The type of a panel element and the nodes of `ui` (spec 10). Each rule is said by what the diagram holds: no line comes first.

use labcraft_types::{Diagnostic, NumberKind, Ty, Unit, parse_unit};

use super::{NOTHING, codes, diagnostics, run};

const TYPES: [(&str, &str); 3] = [
    ("verdict.lct", "type Verdict = enum { Pass, Fail }"),
    ("limits.lct", "type Limits = struct { low: f64[V], high: f64[V] }"),
    ("reading.lct", "type Reading = enum { Value(f64[V]), Overload }"),
];

/// A function with these inputs and outputs, this body and these lines of `panel`.
fn program(inputs: &str, outputs: &str, body: &str, panel: &str) -> String {
    format!("use array\nuse core\nuse ui\n\nfn f({inputs}) -> ({outputs}) {{\n{body}\n}}\n\npanel {{\n{panel}\n}}")
}

fn found(inputs: &str, outputs: &str, body: &str, panel: &str) -> Vec<Diagnostic> {
    let text = program(inputs, outputs, body, panel);
    let mut files = vec![("f.lcf", text.as_str())];
    files.extend_from_slice(&TYPES);
    diagnostics(&files)
}

fn judged(inputs: &str, outputs: &str, body: &str, panel: &str) -> Vec<&'static str> {
    found(inputs, outputs, body, panel).iter().map(|diagnostic| diagnostic.code().as_str()).collect()
}

/// Every order of these lines says the same, since the order of lines has no meaning (5.2, and the drawing has none): the codes and what
/// each one says of its types. Returns the codes, sorted.
fn in_any_order(inputs: &str, outputs: &str, lines: &[&str], panel: &str) -> Vec<&'static str> {
    /// The wires of a node with no name say its line, which moves.
    fn wire_text(wire: &labcraft_types::Wire) -> (Option<String>, Option<String>) {
        (wire.value.clone(), wire.to.clone().filter(|to| !to.starts_with('@')))
    }
    type Said = (&'static str, Option<String>, Option<String>, String, Vec<(Option<String>, Option<String>)>);
    let mut seen: Option<Vec<Said>> = None;
    let mut order: Vec<usize> = (0..lines.len()).collect();
    permutations(&mut order, 0, &mut |order| {
        let body: Vec<&str> = order.iter().map(|index| lines[*index]).collect();
        let mut said: Vec<Said> = found(inputs, outputs, &body.join("\n"), panel)
            .iter()
            .map(|diagnostic| {
                let wires = diagnostic.wires.iter().map(wire_text).collect();
                (diagnostic.code().as_str(), diagnostic.expected.clone(), diagnostic.found.clone(), diagnostic.message.clone(), wires)
            })
            .collect();
        said.sort();
        match &seen {
            Some(first) => assert_eq!(*first, said, "{}", body.join(" | ")),
            None => seen = Some(said),
        }
    });
    seen.unwrap_or_default().iter().map(|said| said.0).collect()
}

fn permutations(order: &mut Vec<usize>, from: usize, each: &mut impl FnMut(&[usize])) {
    if from == order.len() {
        each(order);
        return;
    }
    for index in from..order.len() {
        order.swap(from, index);
        permutations(order, from + 1, each);
        order.swap(from, index);
    }
}

const NO_BODY: &str = "    _ = 1";

/// The type of each element, as the next layer reads it.
fn types_of(text: &str) -> Vec<Ty> {
    let checked = run(&[("f.lcf", text)]);
    let file = checked.files.first().unwrap();
    let items = file.file.panel.as_ref().map_or(&[][..], |panel| &panel.items);
    items.iter().map(|item| checked.types.get(file.resolved.ty(item.id)).clone()).collect()
}

#[test]
fn the_name_of_an_input_or_an_output_gives_the_type() {
    assert_eq!(judged("period: f64[s] = 200 ms", "x: f64[s]", "x = ui.get(period)", "control period: numeric"), NOTHING);
    assert_eq!(judged("period: f64[s]", "x: f64[V]", "x = ui.get(period)", "control period: numeric"), ["LC3003"], "the read has that type");
    assert_eq!(judged("", "count: i32", "count = 1", "indicator count: numeric"), NOTHING);
    assert_eq!(judged("name: string", "", "", "control name: text_box"), NOTHING);
}

#[test]
fn a_wire_goes_to_the_type_of_a_name_as_on_a_terminal() {
    assert_eq!(judged("m: f64[ms], period: f64[s]", "", "_ = ui.set(period, m)", "control period: numeric"), NOTHING, "a conversion");
    assert_eq!(judged("v: f64[V], period: f64[s]", "", "_ = ui.set(period, v)", "control period: numeric"), ["LC3001"]);
    assert_eq!(judged("period: f64[s]", "", "_ = ui.set(period, 5 ms)", "control period: numeric"), NOTHING);
    assert_eq!(judged("period: f64[s]", "", "_ = ui.set(period, 5)", "control period: numeric"), ["LC3001"], "write `5 s`");
    assert_eq!(
        judged("m: f64[ms], period: f64[s]", "x: f64[s]", "x = ui.set(period, m)", "control period: numeric"),
        NOTHING,
        "it gives the type of the element"
    );
}

#[test]
fn an_input_and_an_output_of_one_name() {
    assert_eq!(judged("a: f64[V]", "a: string", "a = \"x\"", "indicator a: text_box"), NOTHING, "an indicator shows the output");
    assert_eq!(judged("a: f64[V]", "a: string", "a = \"x\"", "control a: numeric"), NOTHING, "a control reads the input");
    assert_eq!(judged("a: f64[V]", "a: string", "a = \"x\"", "control a: text_box"), ["LC5011"]);
    assert_eq!(judged("a: f64[V]", "a: string", "a = \"x\"", "indicator a: numeric"), ["LC5011"]);
}

#[test]
fn a_kind_of_one_type_gives_it() {
    assert_eq!(judged("", "x: bool", "x = ui.get(stop)", "control stop: button"), NOTHING);
    assert_eq!(judged("", "x: bool", "x = ui.get(stop)", "control stop: switch"), NOTHING);
    assert_eq!(judged("", "x: string", "x = ui.get(note)", "control note: text_box"), NOTHING);
    assert_eq!(judged("", "x: i32", "x = ui.get(stop)", "control stop: button"), ["LC3003"]);
    assert_eq!(judged("v: f64", "", "_ = ui.set(stop, v)", "control stop: switch"), ["LC3001"]);
    assert_eq!(judged("", "", NO_BODY, "control stop: button"), NOTHING, "no node needs to read it");
}

#[test]
fn a_default_gives_the_type() {
    assert_eq!(judged("", "x: f64", "x = ui.get(gain)", "control gain: numeric(default: 0.5)"), NOTHING);
    assert_eq!(judged("", "x: i32", "x = ui.get(gain)", "control gain: numeric(default: 5)"), NOTHING);
    assert_eq!(judged("", "x: f64[V]", "x = ui.get(gain)", "control gain: numeric(default: 0.5 V)"), NOTHING);
    assert_eq!(judged("", "x: f64[A]", "x = ui.get(gain)", "control gain: numeric(default: 0.5 V)"), ["LC3003"]);
    assert_eq!(judged("", "x: Verdict", "x = ui.get(v)", "indicator v: led(default: Verdict.Pass)"), NOTHING);
    assert_eq!(judged("", "x: bool", "x = ui.get(v)", "indicator v: led(default: true)"), NOTHING);
    assert_eq!(judged("", "", NO_BODY, "indicator a: chart(default: [1.0, 2.0])"), NOTHING);
}

/// The developer said the type: a wire goes to it, as on a terminal, and two wires of two scales are no mistake.
#[test]
fn a_wire_goes_to_the_type_of_a_default() {
    let volts = "indicator t: numeric(default: 0.5 V)";
    assert_eq!(judged("v: f64[V]", "", "_ = ui.set(t, v)", volts), NOTHING);
    assert_eq!(judged("v: f64[mV]", "x: f64[V]", "x = ui.set(t, v)", volts), NOTHING, "converted, and it gives the type of the element");
    assert_eq!(in_any_order("v: f64[V], w: f64[mV]", "", &["_ = ui.set(t, v)", "_ = ui.set(t, w)"], volts), NOTHING);
    assert_eq!(judged("v: f64[A]", "", "_ = ui.set(t, v)", volts), ["LC3001"]);
    assert_eq!(judged("", "", "_ = ui.set(t, 1 mV)", volts), NOTHING, "a constant too");
    assert_eq!(judged("v: f32", "", "_ = ui.set(t, v)", "indicator t: numeric(default: 0.5)"), NOTHING, "widened");
}

#[test]
fn a_whole_number_and_a_wire_of_decimals() {
    let whole = "indicator t: numeric(default: -5)";
    let said = found("v: f64", "", "_ = ui.set(t, v)", whole);
    assert_eq!(said.iter().map(|d| d.code().as_str()).collect::<Vec<_>>(), ["LC3017"], "on the default");
    assert!(said[0].fixes.iter().any(|fix| fix.title.contains("`-5.0`") && !fix.edits.is_empty()), "{:?}", said[0].fixes);
    assert!(said[0].node.is_none(), "it is on the panel");
    assert_eq!(said[0].wires[0].to.as_deref(), Some("panel.t"));
    assert_eq!(said[0].expected.as_deref(), Some("f64"));
    assert_eq!(said[0].found.as_deref(), Some("i32"));
    let fixed = "indicator t: numeric(default: -5.0)";
    assert_eq!(judged("v: f64", "", "_ = ui.set(t, v)", fixed), NOTHING, "its fix is right");
    assert_eq!(judged("v: f32", "", "_ = ui.set(t, v)", fixed), NOTHING, "for an `f32` too");
    assert_eq!(judged("v: f32", "", "_ = ui.set(t, v)", whole), ["LC3017"]);
    assert_eq!(in_any_order("v: f64, w: f32", "", &["_ = ui.set(t, v)", "_ = ui.set(t, w)"], whole), ["LC3017"], "once, in any order");
    assert_eq!(judged("v: f64", "y: f64", "y = ui.set(t, v)", whole), ["LC3017"], "writing `-5.0` answers both");
    let accumulator = in_any_order("v: f64", "", &["x = ui.get(t)", "d = core.add(x, v)", "_ = ui.set(t, d)"], whole);
    assert_eq!(accumulator, ["LC3017"], "an accumulator");
    assert_eq!(judged("v: i32", "", "_ = ui.set(t, v)", whole), NOTHING, "an integer goes there");
    assert_eq!(judged("v: i64", "", "_ = ui.set(t, v)", whole), ["LC3004"], "`.0` would not say an `i64`");
    assert_eq!(judged("v: c128", "", "_ = ui.set(t, v)", whole), ["LC3004"], "nor a complex");
    assert_eq!(judged("", "", "_ = ui.set(t, 1.5)", whole), ["LC3004"], "a constant is on its own terminal");
    let volts = found("v: f64[V]", "", "_ = ui.set(t, v)", whole);
    assert!(volts[0].fixes.iter().any(|fix| fix.title.contains("`-5 V`")), "a unit says more: {:?}", volts[0].fixes);
    let both = in_any_order("v: f64, u: f64[V]", "", &["_ = ui.set(t, v)", "_ = ui.set(t, u)"], whole);
    assert_eq!(both, ["LC3017"], "the first type by its text, in any order");
}

#[test]
fn a_number_without_a_unit_and_a_wire_with_one() {
    let bare = "indicator t: numeric(default: 0.5)";
    let found_here = found("v: f64[V]", "", "_ = ui.set(t, v)", bare);
    assert_eq!(found_here.iter().map(|d| d.code().as_str()).collect::<Vec<_>>(), ["LC3017"], "on the default");
    assert!(found_here[0].fixes.iter().any(|fix| fix.title.contains("0.5 V")), "{:?}", found_here[0].fixes);
    assert!(found_here[0].node.is_none(), "it is on the panel");
    assert_eq!(found_here[0].wires[0].to.as_deref(), Some("panel.t"));
    assert_eq!(judged("v: f64[V], w: f64[V]", "", "_ = ui.set(t, v)\n_ = ui.set(t, w)", bare), ["LC3017"], "once");
    assert_eq!(in_any_order("v: f64[V], w: f64[A]", "", &["_ = ui.set(t, v)", "_ = ui.set(t, w)"], bare), ["LC3017"], "the same, in any order");
    let two = found("v: f64[V], w: f64[A]", "", "_ = ui.set(t, v)\n_ = ui.set(t, w)", bare);
    assert!(two[0].fixes.iter().any(|fix| fix.title.contains("0.5 A")), "the first by its text: {:?}", two[0].fixes);
    assert_eq!(judged("", "", "_ = ui.set(t, 1 V)", bare), ["LC3001"], "a constant is on its own terminal");
    assert_eq!(judged("t2: string", "", "_ = ui.set(t, t2)", bare), ["LC3001"], "it does not say the same of a text");
}

#[test]
fn a_read_needs_an_element_that_says_its_type() {
    for kind in ["numeric", "chart", "led"] {
        assert_eq!(judged("", "x: f64", "x = ui.get(a)", &format!("control a: {kind}")), ["LC5010"], "{kind}: what it gives stays unknown");
        assert_eq!(
            judged("v: f64", "x: f64", "_ = ui.set(a, v)\nx = ui.get(a)", &format!("indicator a: {kind}")),
            ["LC5010"],
            "{kind}: even with a wire"
        );
    }
    assert_eq!(judged("", "x: f64", "x = ui.get<f64>(gain)", "control gain: numeric"), ["LC5010"], "what a call writes does not say it");
    let said = found("v: f64[V]", "", "x = ui.get(total)\nd = core.add(x, v)\n_ = ui.set(total, d)", "indicator total: numeric");
    assert_eq!(said.iter().map(|d| d.code().as_str()).collect::<Vec<_>>(), ["LC5010"], "an accumulator, once");
    assert!(said[0].fixes.iter().any(|fix| fix.title.contains("default: 0 V")), "{:?}", said[0].fixes);
    let led = found("", "x: bool", "x = ui.get(l)", "indicator l: led");
    assert!(led[0].fixes.iter().any(|fix| fix.title.contains("default: false")), "a fix that this kind accepts: {:?}", led[0].fixes);
    assert!(led[0].node.is_none());
    assert_eq!(
        in_any_order("v: f64[V]", "", &["x = ui.get(total)", "d = core.add(x, v)", "_ = ui.set(total, d)"], "indicator total: numeric(default: 0 V)"),
        NOTHING
    );
}

#[test]
fn an_element_that_is_never_read_takes_the_type_of_its_wires() {
    let chart = "indicator live: chart";
    assert_eq!(judged("v: f64[V]", "x: f64[V]", "x = ui.set(live, v)", chart), NOTHING, "it gives what it was given");
    assert_eq!(judged("v: f64[V]", "x: f64[A]", "x = ui.set(live, v)", chart), ["LC3003"]);
    assert_eq!(judged("v: bool", "", "_ = ui.set(lamp, v)", "indicator lamp: led"), NOTHING);
    let text = program("v: f64[V]", "", "_ = ui.set(a, v)", "indicator a: numeric\ncontrol s: button\ncontrol k: numeric(default: 1)");
    let volts = Ty::Number { kind: NumberKind::F64, unit: parse_unit("V", 8).unwrap() };
    assert_eq!(types_of(&text), [volts, Ty::Bool, Ty::Number { kind: NumberKind::I32, unit: Unit::ONE }], "for what reads the file next");
}

#[test]
fn nothing_says_a_type() {
    assert_eq!(judged("", "", NO_BODY, "indicator gain: numeric"), ["LC5010"]);
    assert_eq!(judged("", "", NO_BODY, "indicator live: chart"), ["LC5010"]);
    assert_eq!(judged("", "", NO_BODY, "indicator lamp: led"), ["LC5010"]);
    let said = found("", "", NO_BODY, "indicator a: numeric");
    assert!(said[0].message.contains("no `ui.set`"), "{}", said[0].message);
    let read = found("", "x: f64", "x = ui.get(a)", "indicator a: numeric");
    assert!(read[0].message.contains("read by a `ui.get`"), "{}", read[0].message);
}

#[test]
fn wires_of_different_types_are_a_mistake_of_the_element() {
    let a = "indicator a: numeric";
    let two = ["_ = ui.set(a, v)", "_ = ui.set(a, w)"];
    assert_eq!(in_any_order("v: f64[V], w: f64[V]", "", &two, a), NOTHING);
    assert_eq!(in_any_order("v: f64[V], w: f64[A]", "", &two, a), ["LC5013"]);
    assert_eq!(in_any_order("v: f64[V], w: f64[mV]", "", &two, a), ["LC5013"], "no unit to show would be the right one");
    assert_eq!(in_any_order("v: f64, w: f32", "", &two, a), ["LC5013"], "no widening without a target");
    assert_eq!(judged("v: f64[V], w: f64[A]", "x: f64[V]", "x = ui.set(a, v)\n_ = ui.set(a, w)", a), ["LC5013"], "what it gave stays what it was");
    let arms = "y = match c {\n    true => {\n        _ = ui.set(a, v)\n        yield 1\n    }\n    false => {\n        _ = ui.set(a, w)\n        yield 2\n    }\n}";
    assert_eq!(judged("c: bool, v: f64[V], w: f64[mV]", "y: i32", arms, a), ["LC5013"], "in two arms");
    let looped = "n = loop carry i = 0 {\n    _ = ui.set(a, v)\n    next i = core.add(i, 1)\n    until true\n}";
    assert_eq!(in_any_order("v: f64[V], w: f64[mV]", "n: i32", &[looped, "_ = ui.set(a, w)"], a), ["LC5013"], "in a loop");
}

/// A type that holds what is unknown matches anything, so it would hide a difference, or not, by the order of the lines.
#[test]
fn a_value_unknown_in_part_says_nothing_of_the_others() {
    let panel = "indicator g: gauge\ncontrol b: tabs";
    assert_eq!(
        in_any_order(
            "v: f64[V], w: f64[A]",
            "",
            &["x = ui.get(b)", "_ = ui.set(g, [x])", "p = core.abs(v)", "q = core.abs(w)", "_ = ui.set(g, [p])", "_ = ui.set(g, [q])"],
            panel
        ),
        ["LC5013"]
    );
}

#[test]
fn what_a_diagnostic_of_wires_says() {
    let said = found("v: f64[V], w: f64[mV]", "", "_ = ui.set(a, v)\n_ = ui.set(a, w)", "indicator a: numeric");
    assert_eq!(said.len(), 1);
    assert_eq!(said[0].wires.len(), 2, "it names the two nodes");
    assert!(said[0].wires.iter().all(|wire| wire.from.is_some() && wire.to.is_some()), "{:?}", said[0].wires);
    assert!(said[0].node.is_none(), "it is on the element, which no node holds");

    let many: Vec<String> = (0..7).map(|index| format!("x{index} = ui.set(a, {})", if index == 0 { "v" } else { "w" })).collect();
    let said = found("v: f64[V], w: f64[A]", "", &many.join("\n"), "indicator a: numeric");
    assert_eq!(said[0].code().as_str(), "LC5013");
    assert_eq!(said[0].wires.len(), 5, "at most five");
    assert!(said[0].wires.iter().any(|wire| wire.value.as_deref() == Some("v")), "the type of few is there: {:?}", said[0].wires);
    assert!(said[0].wires.iter().all(|wire| wire.to.as_deref().is_some_and(|to| to.ends_with(".value"))), "{:?}", said[0].wires);
}

#[test]
fn a_constant_takes_the_type_of_the_wires() {
    let a = "indicator a: numeric";
    assert_eq!(in_any_order("v: f64[V]", "", &["_ = ui.set(a, v)", "_ = ui.set(a, 0 V)"], a), NOTHING);
    assert_eq!(in_any_order("v: f32", "", &["_ = ui.set(a, v)", "_ = ui.set(a, 0.5)"], a), NOTHING);
    assert_eq!(in_any_order("v: f64[mV]", "", &["_ = ui.set(a, v)", "_ = ui.set(a, 1 V)"], a), NOTHING);
    assert_eq!(in_any_order("v: f64[V]", "", &["_ = ui.set(a, 0)", "_ = ui.set(a, v)"], a), ["LC3001"], "write `0 V`");
    assert_eq!(in_any_order("v: f64[V]", "", &["k = 0 V", "_ = ui.set(a, k)", "_ = ui.set(a, v)"], a), NOTHING, "a named constant too");
}

#[test]
fn constants_alone_have_their_own_type() {
    let a = "indicator a: numeric";
    assert_eq!(judged("", "x: f64[V]", "x = ui.set(a, 1.5 V)", a), NOTHING);
    assert_eq!(in_any_order("", "", &["_ = ui.set(a, 1 V)", "_ = ui.set(a, 2 V)"], a), NOTHING);
    assert_eq!(in_any_order("", "", &["_ = ui.set(a, 1)", "_ = ui.set(a, 2.5)"], a), ["LC5013"], "no order decides between them");
    assert_eq!(in_any_order("", "", &["_ = ui.set(a, \"x\")", "_ = ui.set(a, 1)"], a), ["LC3001"], "the kind refuses one");
    assert_eq!(judged("", "", "_ = ui.set(a, [])", "indicator a: chart"), ["LC3019"]);
    let said = found("", "", "x = ui.set(a, \"x\")", a);
    assert_eq!(said[0].node.as_ref().and_then(|node| node.name.as_deref()), Some("x"), "with its node");
    assert_eq!(said[0].wires[0].to.as_deref(), Some("x.value"));
}

#[test]
fn a_mistake_in_a_value_is_said_once() {
    let a = "indicator a: numeric";
    let chart = "indicator c: chart";
    assert_eq!(in_any_order("", "", &["_ = ui.set(a, 1)", "_ = ui.set(a, 99999999999)"], a), ["LC3018"]);
    assert_eq!(judged("", "", "_ = ui.set(c, [1.0])\n_ = ui.set(c, [1.0, \"x\"])", chart), ["LC3001"]);
    assert_eq!(judged("v: f64", "", "_ = ui.set(c, [v, \"x\"])", chart), ["LC3001"]);
    assert_eq!(judged("l: Limits", "", "_ = ui.set(c, l.nope)", chart), ["LC2006"]);
    assert_eq!(judged("l: Limits, v: f64[V]", "", "_ = ui.set(c, l.nope)\n_ = ui.set(c, v)", chart), ["LC2006"]);
    assert_eq!(judged("l: Limits", "", "_ = ui.set(stop, l.nope)", "control stop: button"), ["LC2006"]);
    assert_eq!(judged("", "", "_ = ui.set(l, Reading.Value)", "indicator l: led"), ["LC3015"]);
    let wires = ["_ = ui.set(a, v)", "_ = ui.set(a, w)"];
    let many: Vec<&str> = wires.iter().copied().chain(["_ = ui.set(a, 99999999999)"]).collect();
    assert_eq!(in_any_order("v: f64[V], w: f64[A]", "", &many, a), ["LC3018", "LC5013"], "beside wires that differ");
    let many: Vec<&str> = wires.iter().copied().chain(["_ = ui.set(a, [1, \"x\"])"]).collect();
    assert_eq!(in_any_order("v: f64[V], w: f64[A]", "", &many, a), ["LC3001", "LC5013"]);
}

#[test]
fn a_kind_refuses_a_type() {
    assert_eq!(judged("v: f64", "", "_ = ui.set(lamp, v)", "indicator lamp: led"), ["LC3001"], "on the wire");
    assert_eq!(judged("t: string", "", "_ = ui.set(a, t)", "indicator a: numeric"), ["LC3001"]);
    let wire = found("t: string", "", "s = ui.set(a, t)", "indicator a: numeric");
    assert_eq!(wire[0].node.as_ref().and_then(|node| node.name.as_deref()), Some("s"));
    assert_eq!(wire[0].wires[0].to.as_deref(), Some("s.value"));
    assert_eq!(
        in_any_order("t: string, v: f64", "", &["_ = ui.set(a, t)", "_ = ui.set(a, v)"], "indicator a: numeric"),
        ["LC3001"],
        "only that wire"
    );
    assert_eq!(judged("name: string", "", "", "control name: numeric"), ["LC5011"], "on the element");
    assert_eq!(judged("l: Limits", "", "", "control l: numeric"), ["LC5011"]);
    assert_eq!(judged("n: i32", "", "", "control n: text_box"), ["LC5011"]);
    assert_eq!(judged("n: i32", "", "", "control n: button"), ["LC5011"]);
    assert_eq!(judged("n: string", "", "", "control n: chart"), ["LC5011"]);
}

#[test]
fn what_each_kind_shows() {
    assert_eq!(judged("v: f64[V]", "", "", "control v: numeric"), NOTHING);
    assert_eq!(judged("v: i32", "", "", "control v: numeric"), NOTHING);
    assert_eq!(judged("v: duration", "", "", "control v: numeric"), NOTHING);
    assert_eq!(judged("v: c128", "", "", "control v: numeric"), NOTHING);
    assert_eq!(judged("v: Verdict", "", "", "control v: led"), NOTHING);
    assert_eq!(judged("v: Reading", "", "", "control v: led"), NOTHING, "a variant with values shows its name");
    assert_eq!(judged("v: bool", "", "", "control v: led"), NOTHING);
    assert_eq!(judged("v: f64[V]", "", "", "control v: chart"), NOTHING);
    assert_eq!(judged("v: [f64[V]]", "", "", "control v: chart"), NOTHING);
    assert_eq!(judged("v: [f64; 2]", "", "", "control v: chart"), NOTHING);
    assert_eq!(judged("v: [f64; 3]", "", "", "control v: chart"), ["LC5011"]);
    assert_eq!(judged("v: waveform<f64[V]>", "", "", "control v: chart"), NOTHING);
    assert_eq!(judged("v: [[f64]]", "", "", "control v: chart"), ["LC5011"]);
    assert_eq!(judged("v: [string]", "", "", "control v: chart"), ["LC5011"]);
    assert_eq!(judged("v: Limits", "", "", "control v: led"), ["LC5011"]);
}

#[test]
fn an_alias_says_what_it_names() {
    let of = |inputs: &str, panel: &str| {
        let text = program(inputs, "", "", panel);
        codes(&[
            ("f.lcf", text.as_str()),
            ("volts.lct", "type Volts = f64[V]"),
            ("a.lct", "type A = enum { X, Y }"),
            ("b.lct", "type B = A"),
            ("m.lct", "type M = [Volts]"),
        ])
    };
    assert_eq!(of("v: Volts", "control v: numeric"), NOTHING);
    assert_eq!(of("v: B", "control v: led"), NOTHING);
    assert_eq!(of("v: M", "control v: chart"), NOTHING);
    assert_eq!(of("v: B", "control v: numeric"), ["LC5011"]);
}

#[test]
fn a_kind_that_m0_does_not_know_accepts_everything() {
    assert_eq!(judged("l: Limits", "", "", "control l: group"), NOTHING);
    assert_eq!(judged("", "", NO_BODY, "control pages: tabs"), NOTHING, "nothing gives it a type, and it is no mistake");
    assert_eq!(judged("", "x: i32", "x = ui.get(pages)", "control pages: tabs"), NOTHING, "its type stays unknown");
    assert_eq!(
        judged("", "", "x = ui.get(pages)\n_ = ui.set(a, x)", "control pages: tabs\nindicator a: numeric"),
        NOTHING,
        "what it gives says nothing"
    );
    assert_eq!(judged("", "", NO_BODY, "indicator a: gauge(default: \"x\")"), NOTHING, "M0 does not know what it shows");
    assert_eq!(judged("v: f64, w: string", "", "_ = ui.set(g, v)\n_ = ui.set(g, w)", "indicator g: gauge"), ["LC5013"], "an element has one type");
}

#[test]
fn a_default_goes_with_the_kind() {
    assert_eq!(judged("", "", NO_BODY, "control stop: button(default: true)"), NOTHING);
    assert_eq!(judged("", "", NO_BODY, "control stop: button(default: 1)"), ["LC3017"]);
    assert_eq!(judged("", "", NO_BODY, "control note: text_box(default: \"x\")"), NOTHING);
    assert_eq!(judged("", "", NO_BODY, "control note: text_box(default: 1)"), ["LC3017"]);
    assert_eq!(judged("", "", NO_BODY, "indicator lamp: led(default: 5)"), ["LC3017"]);
    assert_eq!(judged("", "", NO_BODY, "indicator a: numeric(default: \"x\")"), ["LC3017"]);
    let said = found("", "", NO_BODY, "indicator t: led(default: 1)");
    assert_eq!(said[0].wires[0].to.as_deref(), Some("panel.t"));
    assert!(said[0].node.is_none());
}

#[test]
fn a_default_has_no_fix_that_adds_a_node() {
    for (inputs, panel) in [("v: f64", "indicator t: button(default: 1)"), ("v: i32", "indicator t: numeric(default: \"x\")")] {
        let said = found(inputs, "", NO_BODY, panel);
        assert_eq!(said[0].code().as_str(), "LC3017");
        assert!(said[0].fixes.iter().all(|fix| !fix.title.contains("Convert")), "{:?}", said[0].fixes);
    }
}

#[test]
fn a_default_is_a_constant_that_an_argument_can_hold() {
    assert_eq!(judged("v: f64", "", "", "indicator a: numeric(default: v)"), ["LC3017"], "it reads nothing of the diagram");
    assert_eq!(judged("", "", NO_BODY, "indicator a: led(default: Verdict.Nope)"), ["LC2007"]);
    assert_eq!(judged("", "", NO_BODY, "indicator a: numeric(format: \"0\")"), ["LC5010"], "only `default:` says something");
    assert_eq!(judged("", "", NO_BODY, "indicator a: gauge(default: none)"), ["LC3017"]);
    assert_eq!(judged("", "", NO_BODY, "indicator a: gauge(default: [none])"), ["LC3017"]);
    assert_eq!(judged("", "", NO_BODY, "indicator a: gauge(default: [1])"), NOTHING);
    let text = "use ui\n\nfn f() -> () {\n    _ = 1\n}\n\npanel {\n    indicator a: gauge(default: g)\n}";
    assert_eq!(codes(&[("f.lcf", text), ("g.lcf", "fn g() -> () {}")]), ["LC3017"], "a function");
}

#[test]
fn an_input_or_an_output_has_no_default_here() {
    assert_eq!(judged("period: f64[s] = 200 ms", "", "", "control period: numeric(default: 500 ms)"), ["LC5012"]);
    assert_eq!(judged("period: f64[s]", "", "", "control period: numeric(default: 500 ms)"), ["LC5012"], "even with none in the signature");
    assert_eq!(judged("", "count: i32", "count = 1", "indicator count: numeric(default: 0)"), ["LC5012"]);
    assert_eq!(judged("", "", NO_BODY, "indicator a: numeric(default: 1, default: 2)"), ["LC5012"]);
    assert_eq!(judged("v: f64", "", "", "control v: numeric(default: nope)"), ["LC5012"], "what it holds is not read");
    let output = found("a: f64", "a: string", "a = \"x\"", "indicator a: text_box(default: \"y\")");
    assert!(output[0].message.contains("an output"), "{}", output[0].message);
    let input = found("a: f64", "a: string", "a = \"x\"", "control a: numeric(default: 1.0)");
    assert!(input[0].message.contains("an input"), "{}", input[0].message);
}

#[test]
fn an_element_is_one_name() {
    let text = "control l: group";
    assert_eq!(judged("l: Limits", "x: Limits", "x = ui.get(l)", text), NOTHING, "the struct as a whole");
    assert_eq!(judged("l: Limits", "", "_ = ui.set(l, l)", text), NOTHING);
    assert_eq!(judged("l: Limits, v: f64[V]", "", "_ = ui.set(l.high, v)", text), ["LC2010"], "not a field");
    assert_eq!(judged("l: Limits", "x: f64[V]", "x = ui.get(l.high)", text), ["LC2010"]);
    assert_eq!(judged("l: Limits", "x: f64[V]", "y = ui.get(l)\nx = y.high", text), NOTHING, "a field is read after it, as on any wire");
}

#[test]
fn an_unknown_element_says_nothing_more() {
    assert_eq!(judged("v: f64", "x: i32", "x = ui.set(nope, v)", "control stop: button"), ["LC2010"]);
    assert_eq!(judged("", "x: i32", "x = ui.get(nope)", "control stop: button"), ["LC2010"]);
    assert_eq!(judged("v: f64", "", "ui.set(stop)", "control stop: button"), ["LC2101"]);
    assert_eq!(judged("", "", "ui.set(a)", "indicator a: numeric"), ["LC2101"], "a `ui.set` without its value is no read, and it writes");
}

#[test]
fn a_written_type_on_an_element_that_says_its_own() {
    assert_eq!(judged("", "x: bool", "x = ui.get<bool>(stop)", "control stop: button"), NOTHING);
    assert_eq!(judged("", "x: i32", "x = ui.get<i32>(stop)", "control stop: button"), ["LC3001"], "one mistake");
    assert_eq!(judged("v: f64", "", "_ = ui.set<f64>(stop, v)", "control stop: button"), ["LC3001"], "one mistake");
}

/// On an element that its wires give a type to, a written type is the type of that value, as on any generic primitive (3.5).
#[test]
fn a_written_type_on_a_wire() {
    let a = "indicator a: numeric";
    assert_eq!(judged("", "x: f64", "x = ui.set<f64>(a, 1)", a), NOTHING);
    assert_eq!(judged("t: string", "", "_ = ui.set<f64>(a, t)", a), ["LC3001"]);
    assert_eq!(in_any_order("v: f32", "", &["_ = ui.set<f64>(a, v)", "_ = ui.set(a, v)"], a), ["LC5013"]);
}

#[test]
fn a_cycle_of_the_diagram_is_not_a_mistake_of_the_element() {
    let a = "indicator a: numeric";
    assert_eq!(in_any_order("", "", &["p = core.abs(q)", "q = core.abs(p)", "_ = ui.set(a, p)"], a), NOTHING, "pass 5 says why");
    assert_eq!(in_any_order("", "", &["x = ui.set(a, y)", "y = core.abs(x)"], a), NOTHING);
}

#[test]
fn a_panel_leaves_the_diagram_as_it_was() {
    let point = ("point.lct", "type Point = struct { x: f64[m], label: string }");
    let bad = "p = Point { x: 1 m, x: 2 m, label: \"a\" }\nq = p";
    for panel in [
        "control s: button(default: true)",
        "indicator t: numeric(default: 0.5)",
        "indicator t: numeric(default: 0.5)\ncontrol s: button(default: 1)",
    ] {
        let text = program("", "", bad, panel);
        let codes_here = codes(&[("f.lcf", text.as_str()), point]);
        assert_eq!(codes_here.iter().filter(|code| **code == "LC3015").count(), 1, "{panel}: {codes_here:?}");
    }
    let text = program("v: f64[V]", "", &format!("{bad}\n_ = ui.set(t, v)"), "indicator t: numeric(default: 1 V)");
    assert_eq!(codes(&[("f.lcf", text.as_str()), point]), ["LC3015"]);
    let text = program("v: f64[V]", "", &format!("_ = ui.set(t, v)\n{bad}"), "indicator t: numeric(default: 0.5)");
    let mut both = codes(&[("f.lcf", text.as_str()), point]);
    both.sort_unstable();
    assert_eq!(both, ["LC3015", "LC3017"], "after a wire that blames the default");
}

/// It has `LC5001` (pass 6). Which item counts would depend on their order, which `fmt` changes, so neither says anything here.
#[test]
fn a_name_given_twice_says_nothing_of_its_type() {
    assert_eq!(judged("v: f64", "", "_ = ui.set(a, v)", "indicator a: numeric\nindicator a: led"), NOTHING);
    assert_eq!(judged("", "", NO_BODY, "indicator a: numeric(default: 1)\nindicator a: led(default: 1)"), NOTHING);
    assert_eq!(judged("v: i32", "", "", "control v: numeric\ncontrol v: button(default: 1)"), NOTHING);
    assert_eq!(judged("", "", NO_BODY, "control v: numeric\ncontrol v: numeric(default: none)"), NOTHING);
    for panel in ["indicator a: chart\ncontrol a: button", "control a: button\nindicator a: chart"] {
        assert_eq!(judged("", "x: i32", "x = ui.get(a)", panel), NOTHING, "{panel}");
    }
}

#[test]
fn a_call_that_cannot_be_bound_is_no_absent_set() {
    let live = "indicator live: chart";
    assert_eq!(judged("v: f64", "", "_ = ui.sett(live, v)", live), ["LC2002", "LC2001"], "an element is no value of an unknown function");
    assert_eq!(judged("v: f64", "", "_ = ui.set(lve, v)", live), ["LC2010"]);
    assert_eq!(judged("v: f64", "", "_ = ui.set(\"live\", v)", live), ["LC3001"]);
    let text = "fn f(v: f64) -> () {\n    _ = ui.set(live, v)\n}\n\npanel {\n    indicator live: chart\n}";
    assert_eq!(codes(&[("f.lcf", text)]), ["LC2005", "LC2001"], "without `use ui`");
}

#[test]
fn a_mistake_in_a_default_is_said_once() {
    for panel in [
        "indicator a: numeric(default: none)",
        "control a: button(default: none)",
        "indicator a: chart(default: [none])",
        "indicator a: numeric(default: core.abs)",
        "control a: button(default: Reading.Value)",
        "indicator a: numeric(default: Reading.Value)",
    ] {
        assert_eq!(judged("", "", NO_BODY, panel), ["LC3017"], "{panel}");
    }
    assert_eq!(
        judged("v: f64[V]", "", "_ = ui.set(a, v)", "indicator a: numeric(default: 99999999999)"),
        ["LC3018"],
        "and a wire has nothing to add"
    );
    assert_eq!(judged("v: f64[V]", "", "_ = ui.set(a, v)", "indicator a: led(default: 0.5)"), ["LC3017"]);
}

#[test]
fn a_refused_default_gives_no_type() {
    assert_eq!(judged("", "x: string", "x = ui.get(a)", "indicator a: numeric(default: \"y\")"), ["LC3017"], "its reads say nothing more");
    assert_eq!(judged("", "x: i32", "x = ui.get(a)", "indicator a: numeric(default: none)"), ["LC3017"]);
}

#[test]
fn the_wrong_default_answers_for_what_the_node_gives() {
    let bare = "indicator t: numeric(default: 0.5)";
    assert_eq!(judged("v: f64[V]", "y: f64[V]", "y = ui.set(t, v)", bare), ["LC3017"], "writing `0.5 V` answers both");
    let said = found("d: duration", "", "_ = ui.set(t, d)", bare);
    assert_eq!(said.iter().map(|d| d.code().as_str()).collect::<Vec<_>>(), ["LC3017"], "a `duration` counts as `f64[s]`");
    assert!(said[0].fixes.iter().any(|fix| fix.title.contains("0.5 s")), "{:?}", said[0].fixes);
}

#[test]
fn a_written_type_says_one_thing() {
    let a = "indicator a: numeric";
    assert_eq!(judged("", "", "_ = ui.set<string>(a, 1)", a), ["LC3001"], "not the kind as well");
    assert_eq!(
        in_any_order("", "", &["_ = ui.set<f32>(a, 1)", "_ = ui.set(a, 2.5)"], a),
        NOTHING,
        "it counts as a wire, and a constant takes its type"
    );
}

#[test]
fn the_types_of_a_difference_go_by_their_text() {
    let said = found("v: f64[V], w: f64[A]", "", "_ = ui.set(a, v)\n_ = ui.set(a, w)", "indicator a: numeric");
    assert_eq!(said[0].expected.as_deref(), Some("f64[A]"));
    assert_eq!(said[0].found.as_deref(), Some("f64[V]"));
    let three = ["x1 = ui.set(a, v)", "x2 = ui.set(a, v)", "x3 = ui.set(a, w)", "x4 = ui.set(a, z)"];
    assert_eq!(in_any_order("v: f64[V], w: f64[V], z: f64[mV]", "", &three, "indicator a: numeric"), ["LC5013"], "the wires named too");
}

#[test]
fn a_faulty_or_generic_signature_says_nothing() {
    assert_eq!(judged("a: i32, a: i32", "", "", "indicator gain: numeric"), ["LC2019"]);
    assert_eq!(judged("a: nowhere", "", "", "control a: numeric"), ["LC2003"]);
    let text = "use ui\n\nfn f<T>(a: T) -> () {}\n\npanel {\n    indicator gain: numeric\n    control a: numeric\n}";
    assert_eq!(codes(&[("f.lcf", text)]), ["LC4006"]);
}

#[test]
fn an_extern_has_no_body_to_read() {
    let of = |panel: &str| {
        let text = format!("extern rust(\"k\") fn f(a: i32) -> (err: error)\n\npanel {{\n    {panel}\n}}");
        codes(&[("f.lcf", text.as_str())])
    };
    assert_eq!(of("indicator gain: numeric"), NOTHING, "no `ui.set` could give it a type");
    assert_eq!(of("control a: text_box"), ["LC5011"], "what the signature says is known");
    assert_eq!(of("control a: numeric(default: 1)"), ["LC5012"]);
    assert_eq!(of("indicator g: numeric(default: nope)"), ["LC3017"], "a constant");
    assert_eq!(of("indicator g: led(default: 1)"), ["LC3017"]);
}
