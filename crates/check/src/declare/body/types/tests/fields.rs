//! `string.format` against its wires (5.4, R-12). The forms of a template are tested with its reader.

use super::*;

const INPUTS: &str = "f: f64, v: f64[V], a: f64[A], n: i32, u: u8, nv: i32[mV], z: c64, s: string, b: bool, d: duration, t: timestamp, e: error, \
                      xs: [f64[V]], ss: [string], g: Verdict, r: Reading, p: Point, w: waveform<f64[V]>, h: fn(f64) -> (f64)";

fn format(args: &str) -> Vec<&'static str> {
    judged(INPUTS, "x: string", &format!("x = string.format({args})"))
}

fn field(format_text: &str, value: &str) -> Vec<&'static str> {
    format(&format!("\"{{a{format_text}}}\", a: {value}"))
}

#[test]
fn each_field_has_one_wire_and_each_wire_one_field() {
    assert_eq!(format("\"no field\""), NOTHING);
    assert_eq!(format("\"U = {a:.3 mV}, I = {b:.2}\", a: v, b: f"), NOTHING);
    assert_eq!(format("\"{a} and {a:.3}\", a: f"), NOTHING, "one terminal for two places");
    assert_eq!(format("template: \"{a}\", a: f"), NOTHING);
    assert_eq!(format("\"{a}\""), ["LC2106"], "no wire");
    assert_eq!(format("\"{a} {b} {c}\""), ["LC2106"], "one diagnostic for the node");
    assert_eq!(format("\"no field\", a: f"), ["LC2106"], "no field");
    assert_eq!(format("\"{a}\", b: f"), ["LC2106"], "both, in one diagnostic");
    assert_eq!(format("\"{a}\", f"), ["LC2106"], "without a name");
    assert_eq!(format("\"{a}\", f, s"), ["LC2106", "LC2106"]);
    assert_eq!(format("\"{a}\", a: f, a: f"), ["LC2102"]);
    assert_eq!(format("\"{a}\", a: f, s"), ["LC2104"], "not said twice");
    assert_eq!(format("\"{a}\", a: f, after: f"), NOTHING, "`after:` is no field");
}

#[test]
fn the_names_of_a_field_and_a_wire_have_one_spelling() {
    assert_eq!(format("\"{\u{03BC}}\", \u{00B5}: f"), NOTHING);
    assert_eq!(format("\"{\u{00B5}}\", \u{03BC}: f"), NOTHING);
}

#[test]
fn a_field_cannot_take_what_the_node_uses() {
    assert_eq!(format("\"{template}\""), ["LC2106"]);
    assert_eq!(format("\"{after}\""), ["LC2106"]);
    assert_eq!(format("\"{for}\""), ["LC2106"]);
}

#[test]
fn a_malformed_field_stops_the_comparison() {
    assert_eq!(format("\"{a:.}\", b: f"), ["LC2106"], "and not two");
    assert_eq!(format("\"{a:.} {\", b: f"), ["LC2106", "LC2106"], "one for each mistake");
    assert_eq!(format("\"{a: Vx}\""), ["LC2008", "LC2106"], "an unknown unit is no malformed field");
}

#[test]
fn the_template_is_written_on_the_node() {
    assert_eq!(format("s"), ["LC2106"], "a wire");
    assert_eq!(judged(INPUTS, "x: string", "k = \"{a}\"\nx = string.format(k, a: f)"), ["LC2106"], "a named constant");
    assert_eq!(format("5"), ["LC2106"]);
    assert_eq!(format("nowhere"), ["LC2001"], "said once");
    assert_eq!(format(""), ["LC2101"]);
}

#[test]
fn escapes_are_read_first() {
    assert_eq!(format("\"a\\n{b}\\t\\\"\", b: f"), NOTHING);
    assert_eq!(format("\"\\\\{b}\", b: f"), NOTHING);
}

#[test]
fn a_field_shows_what_has_a_text() {
    for value in
        ["f", "v", "n", "nv", "z", "s", "b", "d", "t", "e", "xs", "ss", "g", "r", "p", "w", "1", "1.5", "1 V", "\"text\"", "true", "none", "[1, 2]"]
    {
        assert_eq!(field("", value), NOTHING, "{value}");
        assert_eq!(field(":>8", value), NOTHING, "a width for {value}");
    }
}

#[test]
fn a_field_named_err_is_a_field_like_another() {
    assert_eq!(format("\"failed: {err}\", err: e"), NOTHING);
    assert_eq!(format("\"failed: {err}\", err: s"), NOTHING);
    assert_eq!(format("\"failed: {err}\""), ["LC2106"], "it has no default value");
    assert_eq!(judged("e: error", "x: string, y: error", "x, y = string.format(\"{err}\", err: e)"), ["LC2103"], "and no `err` output");
}

#[test]
fn bytes_have_a_text_and_no_number() {
    let frame = "q: bytes";
    assert_eq!(judged(frame, "x: string", "x = string.format(\"{a}\", a: q)"), NOTHING);
    assert_eq!(judged(frame, "x: string", "x = string.format(\"{a:>8}\", a: q)"), NOTHING);
    assert_eq!(judged(frame, "x: string", "x = string.format(\"{a:.3}\", a: q)"), ["LC3001"]);
    assert_eq!(judged(frame, "x: string", "x = string.format(\"{a:x}\", a: q)"), ["LC3001"]);
    assert_eq!(judged("q: [bytes]", "x: string", "x = string.format(\"{a}\", a: q)"), NOTHING);
}

#[test]
fn a_field_refuses_what_has_no_text() {
    let session = [
        ("session.lct", "type session = opaque"),
        ("Holder.lct", "type Holder = struct { s: session, x: f64 }"),
        ("Kinds.lct", "type Kinds = enum { One(session), Two }"),
    ];
    let wired = |ty: &str| judged_with(&session, &format!("q: {ty}"), "x: string", "x = string.format(\"{a}\", a: q)");
    assert_eq!(wired("session"), ["LC3001"]);
    assert_eq!(wired("Holder"), ["LC3001"], "inside a struct");
    assert_eq!(wired("Kinds"), ["LC3001"], "inside a variant");
    assert_eq!(wired("[session]"), ["LC3001"], "inside an array");
    assert_eq!(field("", "h"), ["LC3001"], "a function reference");
    assert_eq!(field("", "core.abs"), ["LC3019"], "a generic one has no type at all");
    assert_eq!(field("", "[]"), ["LC3019"], "nothing says what this holds");
}

#[test]
fn decimals_a_sign_and_zeros_go_to_numbers() {
    for format_text in [":.3", ":+", ":02", ":f", ":g", ":e", ":E", ":.3e"] {
        for value in ["f", "v", "n", "nv", "z", "d", "xs", "r", "p", "w", "1"] {
            assert_eq!(field(format_text, value), NOTHING, "{format_text} on {value}");
        }
        for value in ["s", "b", "t", "e", "ss", "g", "\"text\""] {
            assert_eq!(field(format_text, value), ["LC3001"], "{format_text} on {value}");
        }
    }
}

#[test]
fn an_integer_letter_goes_to_integers_without_a_unit() {
    for format_text in [":d", ":x", ":X", ":o", ":b", ":#x", ":08b"] {
        for value in ["n", "u", "255", "[1, 2]"] {
            assert_eq!(field(format_text, value), NOTHING, "{format_text} on {value}");
        }
        for value in ["f", "v", "nv", "z", "d", "s", "xs", "p", "1.5"] {
            assert_eq!(field(format_text, value), ["LC3001"], "{format_text} on {value}");
        }
    }
}

#[test]
fn a_unit_converts_each_number() {
    assert_eq!(field(":.3 mV", "v"), NOTHING);
    assert_eq!(field(":mV", "v"), NOTHING);
    assert_eq!(field(": mV", "v"), NOTHING);
    assert_eq!(field(":mV", "nv"), NOTHING, "an integer goes through a decimal");
    assert_eq!(field(":.3 ms", "d"), NOTHING, "a duration counts as seconds");
    assert_eq!(field(":mV", "xs"), NOTHING, "each element");
    assert_eq!(field(":mV", "r"), ["LC3005"], "a variant holds volts and an integer");
    assert_eq!(field(":mV", "w"), NOTHING, "its samples: `dt` keeps its seconds");
    assert_eq!(field(":mV", "1 V"), NOTHING);
    assert_eq!(field(":mV", "a"), ["LC3005"], "another dimension");
    assert_eq!(field(":mV", "f"), ["LC3005"], "no dimension");
    assert_eq!(field(":mV", "1"), ["LC3005"]);
    assert_eq!(field(":mV", "s"), ["LC3005"], "no number");
    assert_eq!(field(":mV", "p"), ["LC3005"], "metres");
    assert_eq!(field(": 1", "f"), NOTHING);
    assert_eq!(field(":mV/V", "f"), NOTHING, "a scale has no dimension");
}

#[test]
fn a_struct_of_one_dimension_takes_a_unit() {
    let ok = [("Pair.lct", "type Pair = struct { low: f64[V], high: f64[mV], name: string }")];
    let mixed = [("Pair.lct", "type Pair = struct { low: f64[V], high: f64[A] }")];
    let shown = "x = string.format(\"{a:mV}\", a: q)";
    assert_eq!(judged_with(&ok, "q: Pair", "x: string", shown), NOTHING);
    assert_eq!(judged_with(&mixed, "q: Pair", "x: string", shown), ["LC3005"]);
}

#[test]
fn a_letter_before_a_caret_is_a_unit_with_its_dimension() {
    let area = "q: f64[m^2]";
    assert_eq!(judged(area, "x: string", "x = string.format(\"{a:m^2}\", a: q)"), NOTHING);
    assert_eq!(judged(area, "x: string", "x = string.format(\"{a:V^2}\", a: q)"), ["LC3005"]);
    assert_eq!(judged("q: string", "x: string", "x = string.format(\"{a:m^2}\", a: q)"), ["LC3005"], "and no fill");
}

#[test]
fn an_unknown_unit_has_its_own_code() {
    assert_eq!(field(": Vx", "f"), ["LC2008"]);
    assert_eq!(field(":Vx", "f"), ["LC2008"]);
    assert_eq!(field(": \u{3164}", "f"), ["LC2008"]);
}

#[test]
fn a_letter_comes_before_grams() {
    let mass = "m: f64[kg]";
    assert_eq!(judged(mass, "x: string", "x = string.format(\"{a:g}\", a: m)"), NOTHING);
    assert_eq!(judged(mass, "x: string", "x = string.format(\"{a: g}\", a: m)"), NOTHING);
    assert_eq!(judged("q: f64[V]", "x: string", "x = string.format(\"{a:g}\", a: q)"), NOTHING, "the general format");
    assert_eq!(judged("q: f64[V]", "x: string", "x = string.format(\"{a: g}\", a: q)"), ["LC3005"], "grams");
}

#[test]
fn what_is_unknown_says_nothing_more() {
    assert_eq!(field(":.3", "nowhere"), ["LC2001"]);
    assert_eq!(judged("q: Nowhere", "x: string", "x = string.format(\"{a:.3}\", a: q)"), ["LC2003"]);
    assert_eq!(field(":x", "[nowhere]"), ["LC2001"], "in an array");
    let shown = "y = g()\nx = string.format(\"{a:x}\", a: y)";
    assert_eq!(judged_with(&[("g.lcf", "fn g() -> (o: [Nowhere]) {\n    o = []\n}")], "", "x: string", shown), ["LC2003"], "from another node");
    let holder = [("S.lct", "type S = struct { q: Nowhere }"), ("g.lcf", "fn g(s: S) -> (o: S) {\n    o = s\n}")];
    assert_eq!(judged_with(&holder, "s: S", "x: string", "y = g(s)\nx = string.format(\"{a:x}\", a: y)"), ["LC2003"], "inside a struct");
}

#[test]
fn a_message_never_shows_what_shows_nothing() {
    let text = program(INPUTS, "x: string", "x = string.format(\"{\u{200B}a} {b: \u{3164}}\", b: f)");
    for diagnostic in diagnostics(&[("f.lcf", &text)]) {
        assert!(diagnostic.message.is_ascii(), "{}", diagnostic.message);
    }
}

#[test]
fn a_long_template_is_read_once_for_each_field() {
    let mut text = String::new();
    for index in 0..50_000 {
        text.push_str(&format!("{{a{index}:.3}}"));
    }
    let args: Vec<String> = (0..50_000).map(|index| format!("a{index}: f")).collect();
    assert_eq!(format(&format!("\"{text}\", {}", args.join(", "))), NOTHING);
}

#[test]
fn a_type_is_looked_at_once() {
    let fields: Vec<String> = (0..200_000).map(|index| format!("f{index}: f64[V]")).collect();
    let large = format!("type Large = struct {{ {} }}", fields.join(", "));
    let template = "{a:.3 mV}".repeat(100_000);
    let body = format!("x = string.format(\"{template}\", a: q)");
    assert_eq!(judged_with(&[("Large.lct", large.as_str())], "q: Large", "x: string", &body), NOTHING);
}

fn said(args: &str) -> Vec<Diagnostic> {
    let text = program("f: f64, v: f64[V]", "x: string", &format!("x = string.format({args})"));
    diagnostics(&[("f.lcf", &text)])
}

#[test]
fn what_differs_gives_a_few_names_and_what_to_do() {
    let found = said("\"{a} {b} {c} {d1} {e1} {f1} {g1}\", z1: f");
    assert_eq!(found.len(), 1);
    let one = &found[0];
    assert!(one.message.contains("`a`") && one.message.contains("`e1`"), "{}", one.message);
    assert!(!one.message.contains("`f1`"), "{}", one.message);
    assert!(one.message.contains("2 more"), "{}", one.message);
    assert!(one.message.contains("`z1`"), "{}", one.message);
    assert!(one.fixes.iter().any(|fix| fix.title.starts_with("Wire")), "{:?}", one.fixes);
    assert!(one.fixes.iter().any(|fix| fix.title.starts_with("Add a field")), "{:?}", one.fixes);
}

#[test]
fn a_field_says_which_wire() {
    let found = said("\"{a:x}\", a: f");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].wires.first().and_then(|wire| wire.to.as_deref()), Some("x.a"));
    assert_eq!(found[0].wires.first().and_then(|wire| wire.value.as_deref()), Some("f"));
    assert_eq!(found[0].found.as_deref(), Some("f64"));
}

#[test]
fn a_cause_is_said_once_for_a_name_however_it_is_written() {
    assert_eq!(field(":x} {a:d", "f"), ["LC3001"]);
    assert_eq!(field(":x} {a:X} {a:#o", "f"), ["LC3001"]);
    assert_eq!(field(":.3} {a:.4", "s"), ["LC3001"]);
    assert_eq!(field(":A} {a:Hz", "v"), ["LC3005"], "the first that does not convert");
    assert_eq!(field(":A} {a:x", "v"), ["LC3001", "LC3005"], "two causes");
    assert_eq!(format("\"{a:x} {b:x}\", a: f, b: f"), ["LC3001", "LC3001"], "two wires");
}

#[test]
fn a_fill_is_seen() {
    assert_eq!(field(":\u{3164}<8", "f"), ["LC2106"], "a letter that shows nothing");
    assert_eq!(field(":\u{00E9}<8", "f"), NOTHING);
}

#[test]
fn a_message_says_the_unit_as_it_is_written() {
    let found = said("\"{a: 1}\", a: v");
    assert_eq!(found.len(), 1);
    assert!(found[0].message.contains("in `1`"), "{}", found[0].message);
    let found = said("\"{a:mV/V}\", a: v");
    assert!(found[0].message.contains("in `mV/V`"), "{}", found[0].message);
}

#[test]
fn a_very_long_name_is_cut_in_what_is_said() {
    let long = "a".repeat(200_000);
    for found in [said(&format!("\"{{{long}:x}}\", {long}: f")), said(&format!("\"{{{long}}}\""))] {
        assert_eq!(found.len(), 1);
        assert!(found[0].message.len() < 10_000, "{}", found[0].message.len());
        assert!(found[0].fixes.iter().all(|fix| fix.title.len() < 10_000), "the fixes too");
    }
    let found = said(&format!("\"{{a:{long}}}\", a: f"));
    assert_eq!(found.len(), 1);
    assert!(found[0].message.len() < 10_000, "a unit that is not one: {}", found[0].message.len());
}

#[test]
fn escapes_are_read_before_the_format() {
    assert_eq!(field(":\\\"<8", "f"), NOTHING, "a quote as the fill");
}

#[test]
fn a_positional_value_after_a_named_one_is_said_once() {
    assert_eq!(format("\"{a}\", after: f, f"), ["LC2104"]);
}

#[test]
fn a_correct_field_does_not_pay_for_the_text_of_its_type() {
    // At each level the text of the type grows, so that writing it for each field is what takes the time.
    let deep = format!("{}f64{}", "[".repeat(200), "]".repeat(200));
    let template = "{a}".repeat(200_000);
    assert_eq!(judged(&format!("q: {deep}"), "x: string", &format!("x = string.format(\"{template}\", a: q)")), NOTHING);
}
