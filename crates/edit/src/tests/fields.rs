//! The fields in place of M1 step 4b (commands.md 6.7): constants on inputs, renames, labels, templates. Each expected text
//! is written by hand from commands.md 6.7 and 9.1, and spec 5.2, 5.4 and 11.

use super::FILE;
use super::{OPEN, TWO, connect, gives, refused};
use crate::Op;

fn set(to: &str, value: &str) -> Op {
    Op::InputSetConstant { file: FILE.to_owned(), to: to.to_owned(), value: value.to_owned(), replace: false, accept_mismatch: false, arm: None }
}

fn replace(to: &str, value: &str) -> Op {
    Op::InputSetConstant { file: FILE.to_owned(), to: to.to_owned(), value: value.to_owned(), replace: true, accept_mismatch: false, arm: None }
}

fn clear(to: &str) -> Op {
    Op::InputClearConstant { file: FILE.to_owned(), to: to.to_owned(), arm: None }
}

fn remove(to: &str) -> Op {
    Op::InputRemoveValue { file: FILE.to_owned(), to: to.to_owned() }
}

fn rename(value: &str, name: &str) -> Op {
    Op::ValueRename { file: FILE.to_owned(), value: value.to_owned(), name: name.to_owned() }
}

fn label(line: u32, label: &str) -> Op {
    Op::NodeSetLabel { file: FILE.to_owned(), line, label: label.to_owned() }
}

fn template(node: &str, template: &str) -> Op {
    Op::NodeSetTemplate { file: FILE.to_owned(), node: node.to_owned(), template: template.to_owned() }
}

// input.set_constant

#[test]
fn a_constant_goes_where_a_wire_would_go() {
    gives(OPEN, set("s.a", "1.0"), "use core\n\nfn f(x: f64, z: f64) -> (y: f64) {\n    s = core.add(1.0)\n    y = x\n}\n");
    // A parameter after one that nothing feeds goes by name, as a wire goes (R-11).
    gives(OPEN, set("s.b", "2.0"), "use core\n\nfn f(x: f64, z: f64) -> (y: f64) {\n    s = core.add(b: 2.0)\n    y = x\n}\n");
}

#[test]
fn an_input_that_holds_something_is_replaced_only_when_asked() {
    let error = refused(TWO, set("a.b", "5"), "wire.occupied");
    assert!(error.message.contains("`1`"), "{error:?}");
    gives(TWO, replace("a.b", "5"), "use core\n\nfn f(x: f64) -> (y: f64) {\n    a = core.add(x, 5)\n    b = core.mul(a, 2)\n    y = b\n}\n");
    // A wire replaced by a constant goes, with its entry of `layout`.
    let wired =
        "use core\n\nfn f(x: f64, z: f64) -> (y: f64) {\n    s = core.add(x, 1)\n    y = x\n}\n\nlayout {\n    wire x -> s.a via (10, 10)\n}\n";
    gives(wired, replace("s.a", "3"), "use core\n\nfn f(x: f64, z: f64) -> (y: f64) {\n    s = core.add(3, 1)\n    y = x\n}\n");
}

#[test]
fn a_constant_of_the_wrong_type_is_refused_unless_accepted() {
    let error = refused(TWO, replace("a.b", "\"volts\""), "wire.type_mismatch");
    assert!(error.message.contains("the constant `\"volts\"` on `a.b`"), "{error:?}");
    let op = Op::InputSetConstant {
        file: FILE.to_owned(),
        to: "a.b".to_owned(),
        value: "\"volts\"".to_owned(),
        replace: true,
        accept_mismatch: true,
        arm: None,
    };
    gives(TWO, op, "use core\n\nfn f(x: f64) -> (y: f64) {\n    a = core.add(x, \"volts\")\n    b = core.mul(a, 2)\n    y = b\n}\n");
}

#[test]
fn set_constant_refuses_what_is_no_constant_or_no_place_for_one() {
    // A hole is what clearing leaves, never a constant.
    assert_eq!(refused(OPEN, set("s.a", "?"), "params.invalid").parameter, Some("value"));
    assert_eq!(refused(OPEN, set("s.a", "z"), "params.invalid").parameter, Some("value"));
    assert_eq!(refused(OPEN, set("output y", "1.0"), "params.invalid").parameter, Some("to"));
    refused(OPEN, set("s.c", "1.0"), "edit.unknown");
    let format = "use string\n\nfn f(x: f64) -> (y: f64) {\n    text = string.format(\"\")\n    y = x\n}\n";
    let error = refused(format, replace("text.template", "\"{a}\""), "params.invalid");
    assert!(error.message.contains("node.set_template"), "{error:?}");
}

#[test]
fn a_constant_adds_a_value_to_a_variadic_parameter_at_its_end() {
    let before = "use string\n\nfn f(x: f64) -> (y: f64) {\n    t = string.concat(\"a\", \"b\")\n    y = x\n}\n";
    gives(before, set("t.parts.1", "\"c\""), "use string\n\nfn f(x: f64) -> (y: f64) {\n    t = string.concat(\"a\", \"b\", \"c\")\n    y = x\n}\n");
}

// input.clear_constant

#[test]
fn a_cleared_constant_is_left_out_or_leaves_a_hole() {
    gives(TWO, clear("a.b"), "use core\n\nfn f(x: f64) -> (y: f64) {\n    a = core.add(x)\n    b = core.mul(a, 2)\n    y = b\n}\n");
    let first = "use core\n\nfn f(x: f64) -> (y: f64) {\n    a = core.add(1, x)\n    y = a\n}\n";
    gives(first, clear("a.a"), "use core\n\nfn f(x: f64) -> (y: f64) {\n    a = core.add(?, x)\n    y = a\n}\n");
    let array = "fn f(x: f64) -> (y: [f64]) {\n    k = [x, 3.0]\n    y = k\n}\n";
    gives(array, clear("k.1"), "fn f(x: f64) -> (y: [f64]) {\n    k = [x, ?]\n    y = k\n}\n");
}

#[test]
fn only_a_constant_is_cleared() {
    // A wire is unwired by `wire.disconnect`, and a hole or nothing holds no constant.
    refused(TWO, clear("a.a"), "params.invalid");
    refused(OPEN, clear("s.a"), "params.invalid");
    // A field of a value read is a wire too (spec 11); a variant names a type, and is a constant.
    refused("use core\n\nfn f(x: f64) -> (y: f64) {\n    a = core.add(x.volts, 1.0)\n    y = a\n}\n", clear("a.a"), "params.invalid");
    gives(
        "use core\n\nfn f(x: f64) -> (y: f64) {\n    a = core.add(Verdict.Pass, x)\n    y = a\n}\n",
        clear("a.a"),
        "use core\n\nfn f(x: f64) -> (y: f64) {\n    a = core.add(?, x)\n    y = a\n}\n",
    );
}

#[test]
fn a_message_shows_a_long_text_cut() {
    let long = format!("\"{}\"", "v".repeat(5_000));
    let error = refused(TWO, replace("a.b", &long), "wire.type_mismatch");
    assert!(error.message.chars().count() < 400, "{}", error.message.chars().count());
    assert!(error.message.contains('…'), "{error:?}");
}

// input.remove_value

#[test]
fn a_value_removed_moves_the_ones_after_it_up() {
    let before = "use string\n\nfn f(x: string) -> (y: string) {\n    t = string.concat(\"a\", \"b\", x)\n    y = t\n}\n";
    gives(before, remove("t.parts.0"), "use string\n\nfn f(x: string) -> (y: string) {\n    t = string.concat(\"a\", x)\n    y = t\n}\n");
    // With its wire.
    gives(before, remove("t.parts.1"), "use string\n\nfn f(x: string) -> (y: string) {\n    t = string.concat(\"a\", \"b\")\n    y = t\n}\n");
    // The parser does not read yet a `wire` entry that names a place (`k.1`, `t.parts.1`, spec 14): no entry to move up.
    let array = "fn f(x: f64) -> (y: [f64]) {\n    k = [x, 3.0, x]\n    y = k\n}\n";
    gives(array, remove("k.0"), "fn f(x: f64) -> (y: [f64]) {\n    k = [3.0, x]\n    y = k\n}\n");
}

#[test]
fn remove_value_takes_only_a_value_of_a_list() {
    refused(TWO, remove("a.b"), "params.invalid");
    let array = "fn f(x: f64) -> (y: [f64]) {\n    k = [x, 3.0]\n    y = k\n}\n";
    refused(array, remove("k.2"), "edit.unknown");
}

// value.rename

#[test]
fn a_value_is_renamed_everywhere_in_the_function() {
    let before = "use core\n\nfn f(x: f64) -> (y: f64) {\n    a = core.add(x, 1)\n    b = core.mul(a, 2)\n    y = b\n}\n\nlayout {\n    node a at (40, 80)\n    wire a -> b via (120, 80)\n}\n";
    let edited = gives(
        before,
        rename("a", "c"),
        "use core\n\nfn f(x: f64) -> (y: f64) {\n    c = core.add(x, 1)\n    b = core.mul(c, 2)\n    y = b\n}\n\nlayout {\n    node c at (40, 80)\n    wire c -> b via (120, 80)\n}\n",
    );
    assert_eq!(edited.names.get("value").map(String::as_str), Some("c"));
}

#[test]
fn a_rename_refuses_a_name_taken_or_not_a_name() {
    assert_eq!(refused(TWO, rename("a", "b"), "name.taken").parameter, Some("name"));
    // The name of an output, which only a wire gives (notation 2.4).
    refused(TWO, rename("a", "y"), "name.taken");
    refused(TWO, rename("a", "fn"), "name.invalid");
    refused(TWO, rename("a", "2a"), "name.invalid");
    // An input or an output of the function is renamed by `signature.rename`.
    refused(TWO, rename("x", "z"), "params.invalid");
    refused(TWO, rename("y", "z"), "params.invalid");
    refused(TWO, rename("q", "z"), "edit.unknown");
}

#[test]
fn a_label_is_renamed_with_the_after_that_wait_for_it() {
    let before = "use core\nuse sound\n\nfn f(x: f64) -> (y: f64) {\n    #beep: sound.beep()\n    s = core.add(x, 1, after: #beep)\n    y = s\n}\n";
    let edited = gives(
        before,
        rename("#beep", "#bell"),
        "use core\nuse sound\n\nfn f(x: f64) -> (y: f64) {\n    #bell: sound.beep()\n    s = core.add(x, 1, after: #bell)\n    y = s\n}\n",
    );
    assert_eq!(edited.names.get("node").map(String::as_str), Some("#bell"));
    refused(before, rename("#bep", "#bell"), "edit.unknown");
    // A label is unique among labels (R-17): a value may bear its name, another label may not.
    gives(
        before,
        rename("#beep", "#s"),
        "use core\nuse sound\n\nfn f(x: f64) -> (y: f64) {\n    #s: sound.beep()\n    s = core.add(x, 1, after: #s)\n    y = s\n}\n",
    );
    let two = "use sound\n\nfn f(x: f64) -> (y: f64) {\n    #beep: sound.beep()\n    #bell: sound.beep()\n    y = x\n}\n";
    refused(two, rename("#beep", "#bell"), "name.taken");
}

// node.set_label

#[test]
fn a_node_named_by_its_line_gets_a_label_or_a_new_one() {
    // Line 1 is `labcraft 1`; `b = core.mul(a, 2)` starts on line 7.
    let edited = gives(
        TWO,
        label(7, "#scale"),
        "use core\n\nfn f(x: f64) -> (y: f64) {\n    a = core.add(x, 1)\n    #scale: b = core.mul(a, 2)\n    y = b\n}\n",
    );
    assert_eq!(edited.names.get("node").map(String::as_str), Some("#scale"));
    let labelled = "use core\n\nfn f(x: f64) -> (y: f64) {\n    a = core.add(x, 1)\n    #scale: b = core.mul(a, 2)\n    y = b\n}\n";
    gives(
        labelled,
        label(7, "gain"),
        "use core\n\nfn f(x: f64) -> (y: f64) {\n    a = core.add(x, 1)\n    #gain: b = core.mul(a, 2)\n    y = b\n}\n",
    );
}

#[test]
fn a_new_label_names_the_node_in_layout() {
    let before = "use core\n\nfn f(x: f64) -> (y: f64) {\n    a = core.add(x, 1)\n    b = core.mul(a, 2)\n    y = b\n}\n\nlayout {\n    node b at (200, 80)\n    wire a -> b via (120, 80)\n}\n";
    gives(
        before,
        label(7, "#scale"),
        "use core\n\nfn f(x: f64) -> (y: f64) {\n    a = core.add(x, 1)\n    #scale: b = core.mul(a, 2)\n    y = b\n}\n\nlayout {\n    node #scale at (200, 80)\n    wire a -> #scale via (120, 80)\n}\n",
    );
}

#[test]
fn set_label_refuses_a_line_without_a_node_or_a_label_taken() {
    assert_eq!(refused(TWO, label(1, "#a1"), "edit.unknown").parameter, Some("line"));
    refused(TWO, label(7, "#fn"), "name.invalid");
    // A value of that name is no label (R-17).
    gives(TWO, label(7, "#a"), "use core\n\nfn f(x: f64) -> (y: f64) {\n    a = core.add(x, 1)\n    #a: b = core.mul(a, 2)\n    y = b\n}\n");
    let labelled = "use core\n\nfn f(x: f64) -> (y: f64) {\n    #first: a = core.add(x, 1)\n    b = core.mul(a, 2)\n    y = b\n}\n";
    refused(labelled, label(7, "#first"), "name.taken");
}

// node.set_template

const FORMAT: &str = "use string\n\nfn f(x: f64) -> (y: string) {\n    text = string.format(\"{v} {w}\", v: x, w: x)\n    y = text\n}\n";

#[test]
fn a_template_keeps_the_wires_of_the_fields_that_stay() {
    gives(
        FORMAT,
        template("text", "\"{w} and {u}\""),
        "use string\n\nfn f(x: f64) -> (y: string) {\n    text = string.format(\"{w} and {u}\", w: x)\n    y = text\n}\n",
    );
    // A new field is an input of its own, wired by name.
    let new = "use string\n\nfn f(x: f64) -> (y: string) {\n    text = string.format(\"\")\n    y = text\n}\n";
    let written = "use string\n\nfn f(x: f64) -> (y: string) {\n    text = string.format(\"{u}\")\n    y = text\n}\n";
    gives(new, template("text", "\"{u}\""), written);
    gives(written, connect("x", "text.u"), "use string\n\nfn f(x: f64) -> (y: string) {\n    text = string.format(\"{u}\", u: x)\n    y = text\n}\n");
    gives(written, set("text.u", "1.5"), "use string\n\nfn f(x: f64) -> (y: string) {\n    text = string.format(\"{u}\", u: 1.5)\n    y = text\n}\n");
}

#[test]
fn a_format_without_its_template_gets_it_by_set_template_only() {
    let bare = "use string\n\nfn f(x: f64) -> (y: string) {\n    text = string.format()\n    y = text\n}\n";
    gives(bare, template("text", "\"{u}\""), "use string\n\nfn f(x: f64) -> (y: string) {\n    text = string.format(\"{u}\")\n    y = text\n}\n");
    refused(bare, set("text.template", "\"{u}\""), "params.invalid");
}

#[test]
fn a_template_named_after_its_fields_is_written_in_its_place() {
    let named = "use string\n\nfn f(x: f64) -> (y: string) {\n    text = string.format(v: x, template: \"{v}\")\n    y = text\n}\n";
    gives(
        named,
        template("text", "\"{w}\""),
        "use string\n\nfn f(x: f64) -> (y: string) {\n    text = string.format(template: \"{w}\")\n    y = text\n}\n",
    );
    // A field that reads a field of a value takes the entry of its wire with it.
    let read = "use string\n\nfn f(x: f64) -> (y: string) {\n    text = string.format(\"{v}\", v: x.volts)\n    y = text\n}\n\nlayout {\n    wire x -> text.v via (50, 50)\n}\n";
    gives(read, template("text", "\"{w}\""), "use string\n\nfn f(x: f64) -> (y: string) {\n    text = string.format(\"{w}\")\n    y = text\n}\n");
}

#[test]
fn set_template_refuses_what_is_no_template_of_a_format() {
    let error = refused(FORMAT, template("text", "\"{v\""), "params.invalid");
    assert!(error.message.contains("nothing closes"), "the message of check: {error:?}");
    assert_eq!(refused(FORMAT, template("text", "x"), "params.invalid").parameter, Some("template"));
    assert_eq!(refused(FORMAT, template("text", "1.0"), "params.invalid").parameter, Some("template"));
    assert_eq!(refused(TWO, template("a", "\"{v}\""), "params.invalid").parameter, Some("node"));
}
