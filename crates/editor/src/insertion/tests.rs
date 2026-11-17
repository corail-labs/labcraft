//! The catalog of quick insertion, on entries as `functions.list` gives them (commands.md 6.1) and the filter of notation
//! 2.6. The inputs and outputs of each entry are written by hand from its signature (spec A.1) as commands.md 6.1 says;
//! each expected list is worked out by hand from the rule of fit (spec 3.3, 3.4).

use labcraft_types::{FitKey, MAX_NESTING, NumberKind, Unit, parse_unit};
use serde_json::{Value, json};

use super::*;

fn listed(library: &str, name: &str, kind: &str, signature: &str, typed: Value) -> Listed {
    Listed {
        name: name.to_owned(),
        library: library.to_owned(),
        kind: kind.to_owned(),
        signature: signature.to_owned(),
        // A fixture that does not read fails here, never as an entry without inputs.
        typed: serde_json::from_value(typed).unwrap(),
    }
}

fn volts_fit() -> Value {
    json!({ "number": "f64", "unit": "V" })
}

fn error_input() -> Value {
    json!({ "name": "err", "type": "error", "fit": { "base": "error" } })
}

fn error_output() -> Value {
    json!({ "name": "err", "type": "error", "fit": { "base": "error" } })
}

fn numbers(variable: &str) -> Value {
    json!({ "variable": variable, "bound": "Number" })
}

fn catalog() -> Catalog {
    Catalog::new(&[
        listed(
            "core",
            "core.add",
            "function",
            "fn core.add<N: Number>(a: N, b: N) -> (result: N)",
            json!({
                "inputs": [
                    { "name": "a", "type": "N", "fit": numbers("N"), "required": true },
                    { "name": "b", "type": "N", "fit": numbers("N"), "required": true },
                ],
                "outputs": [{ "name": "result", "type": "N", "fit": numbers("N") }],
            }),
        ),
        listed(
            "core",
            "core.lt",
            "function",
            "fn core.lt<N: Number>(a: N, b: N) -> (result: bool)",
            json!({
                "inputs": [
                    { "name": "a", "type": "N", "fit": numbers("N"), "required": true },
                    { "name": "b", "type": "N", "fit": numbers("N"), "required": true },
                ],
                "outputs": [{ "name": "result", "type": "bool", "fit": { "base": "bool" } }],
            }),
        ),
        listed(
            "core",
            "core.mul",
            "function",
            "fn core.mul<A: Number, B: Number>(a: A, b: B) -> (result: A·B)",
            json!({
                "inputs": [
                    { "name": "a", "type": "A", "fit": numbers("A"), "required": true },
                    { "name": "b", "type": "B", "fit": numbers("B"), "required": true },
                ],
                "outputs": [{ "name": "result", "type": "A·B", "fit": { "numbers": ["A", "B"] } }],
            }),
        ),
        listed(
            "core",
            "core.select",
            "function",
            "fn core.select<T>(condition: bool, if_true: T, if_false: T) -> (result: T)",
            json!({
                "inputs": [
                    { "name": "condition", "type": "bool", "fit": { "base": "bool" }, "required": true },
                    { "name": "if_true", "type": "T", "fit": { "variable": "T" }, "required": true },
                    { "name": "if_false", "type": "T", "fit": { "variable": "T" }, "required": true },
                ],
                "outputs": [{ "name": "result", "type": "T", "fit": { "variable": "T" } }],
            }),
        ),
        listed(
            "core",
            "core.with_unit",
            "function",
            "fn core.with_unit(x: f64, unit: kind unit) -> (result: f64[unit])",
            json!({
                "inputs": [
                    { "name": "x", "type": "f64", "fit": { "number": "f64" }, "required": true },
                    { "name": "unit", "type": "kind unit", "required": true, "literal": true },
                ],
                "outputs": [{ "name": "result", "type": "f64[unit]", "fit": { "numbers": ["unit"] } }],
            }),
        ),
        listed(
            "string",
            "string.format",
            "function",
            "fn string.format(template: string, …) -> (text: string)",
            json!({
                "inputs": [{ "name": "template", "type": "string", "required": true, "literal": true }],
                "outputs": [{ "name": "text", "type": "string", "fit": { "base": "string" } }],
            }),
        ),
        listed(
            "time",
            "time.now",
            "function",
            "fn time.now() -> (timestamp: timestamp)",
            json!({ "outputs": [{ "name": "timestamp", "type": "timestamp", "fit": { "base": "timestamp" } }] }),
        ),
        listed(
            "time",
            "time.wait",
            "function",
            "fn time.wait(duration: duration, err: error = none) -> (err: error)",
            json!({
                "inputs": [{ "name": "duration", "type": "duration", "fit": { "base": "duration" }, "required": true }, error_input()],
                "outputs": [error_output()],
            }),
        ),
        listed(
            "visa",
            "visa.open",
            "function",
            "fn visa.open(resource: string, timeout: duration = 2 s, err: error = none) -> (session: visa.session, err: error)",
            json!({
                "inputs": [
                    { "name": "resource", "type": "string", "fit": { "base": "string" }, "required": true },
                    { "name": "timeout", "type": "duration", "fit": { "base": "duration" } },
                    error_input(),
                ],
                "outputs": [{ "name": "session", "type": "visa.session", "fit": { "shape": "visa.session" } }, error_output()],
            }),
        ),
        listed("visa", "visa.session", "type", "type visa.session = opaque", json!({})),
        listed(
            "psu",
            "psu.session",
            "type",
            "type psu.session = visa.session",
            json!({ "is": { "type": "visa.session", "fit": { "shape": "visa.session" } } }),
        ),
        // The engine follows the alias: the fit of a `psu.session` is that of a `visa.session`.
        listed(
            "psu",
            "psu.set_voltage",
            "function",
            "fn psu.set_voltage(source: psu.session, voltage: f64[V], err: error = none) -> (source: psu.session, err: error)",
            json!({
                "inputs": [
                    { "name": "source", "type": "psu.session", "fit": { "shape": "visa.session" }, "required": true },
                    { "name": "voltage", "type": "f64[V]", "fit": volts_fit(), "required": true },
                    error_input(),
                ],
                "outputs": [{ "name": "source", "type": "psu.session", "fit": { "shape": "visa.session" } }, error_output()],
            }),
        ),
        listed(
            "",
            "Step",
            "type",
            "type Step = struct { name: string, route: string, voltage: f64[V], low: f64[V], high: f64[V] }",
            json!({
                "fields": [
                    { "name": "name", "type": "string", "fit": { "base": "string" } },
                    { "name": "route", "type": "string", "fit": { "base": "string" } },
                    { "name": "voltage", "type": "f64[V]", "fit": volts_fit() },
                    { "name": "low", "type": "f64[V]", "fit": volts_fit() },
                    { "name": "high", "type": "f64[V]", "fit": volts_fit() },
                ],
            }),
        ),
        listed(
            "",
            "Reading",
            "type",
            "type Reading = enum { Value(f64[V]), Missing }",
            json!({ "variants": [{ "name": "Value", "values": [{ "type": "f64[V]", "fit": volts_fit() }] }, { "name": "Missing" }] }),
        ),
        listed(
            "",
            "settle",
            "function",
            "fn settle(dmm: visa.session, target: f64[V], tolerance: f64[V], period: duration, timeout: duration, err: error = none) -> (value: f64[V], err: error)",
            json!({
                "inputs": [
                    { "name": "dmm", "type": "visa.session", "fit": { "shape": "visa.session" }, "required": true },
                    { "name": "target", "type": "f64[V]", "fit": volts_fit(), "required": true },
                    { "name": "tolerance", "type": "f64[V]", "fit": volts_fit(), "required": true },
                    { "name": "period", "type": "duration", "fit": { "base": "duration" }, "required": true },
                    { "name": "timeout", "type": "duration", "fit": { "base": "duration" }, "required": true },
                    error_input(),
                ],
                "outputs": [{ "name": "value", "type": "f64[V]", "fit": volts_fit() }, error_output()],
            }),
        ),
    ])
}

fn volts() -> FitKey {
    FitKey::Number(NumberKind::F64, parse_unit("V", MAX_NESTING).unwrap_or(Unit::ONE))
}

fn calls(catalog: &Catalog, rows: &[Row]) -> Vec<String> {
    rows.iter()
        .map(|row| match row {
            Row::Typed(text) => format!("constant {text}"),
            Row::Entry(index) => catalog.entries.get(*index).map(|entry| entry.call.clone()).unwrap_or_default(),
        })
        .collect()
}

#[test]
fn the_entries_read_with_their_types() {
    let catalog = catalog();
    let open = catalog.entry("visa.open").map(|entry| entry.inputs.clone()).unwrap_or_default();
    assert_eq!(
        open.iter().map(|input| (input.name.as_str(), input.required)).collect::<Vec<_>>(),
        [("resource", true), ("timeout", false), ("err", false)]
    );
    assert_eq!(open.first().map(|input| &input.ty), Some(&SlotTy::Key(FitKey::String)));
    // The template of `string.format` and a `kind` take no wire (spec 5.4, A.1).
    let format = catalog.entry("string.format").map(|entry| entry.inputs.clone()).unwrap_or_default();
    assert_eq!(format.iter().map(|input| &input.ty).collect::<Vec<_>>(), [&SlotTy::Literal]);
    let with_unit = catalog.entry("core.with_unit").map(|entry| entry.inputs.clone()).unwrap_or_default();
    assert_eq!(with_unit.get(1).map(|input| &input.ty), Some(&SlotTy::Literal));
    // An alias is followed: a `psu.session` is a `visa.session`.
    let source = catalog.entry("psu.set_voltage").and_then(|entry| entry.inputs.first().cloned()).map(|input| input.ty);
    assert_eq!(source, Some(SlotTy::Named("visa.session".to_owned())));
    // A struct is a construction, a variant with values one too, a variant without values a constant.
    assert_eq!(catalog.entry("Step").map(|entry| (entry.kind, entry.inputs.len())), Some((EntryKind::Struct, 5)));
    assert_eq!(catalog.entry("Reading.Value").map(|entry| entry.kind), Some(EntryKind::Variant));
    assert_eq!(catalog.entry("Reading.Missing").map(|entry| entry.kind), Some(EntryKind::Constant));
}

#[test]
fn a_wire_of_volts_from_an_output_lists_what_takes_it() {
    let catalog = catalog();
    let wanted = Wanted { from_output: true, key: Some(volts()), text: "f64[V]".to_owned(), family: "Number/0".to_owned() };
    let rows = catalog.rows("", Some(&wanted), &[]);
    // First the entries whose first required input takes it, then the others, each in the alphabet.
    assert_eq!(calls(&catalog, &rows), ["Reading.Value", "core.add", "core.lt", "core.mul", "Step", "core.select", "psu.set_voltage", "settle"]);
}

#[test]
fn a_wire_drawn_back_from_an_input_lists_what_gives_it() {
    let catalog = catalog();
    let wanted = Wanted { from_output: false, key: Some(FitKey::Bool), text: "bool".to_owned(), family: "Bool/0".to_owned() };
    assert_eq!(calls(&catalog, &catalog.rows("", Some(&wanted), &[])), ["core.lt", "core.select"]);
    // Text that reads as a constant comes first when the input takes it.
    assert_eq!(calls(&catalog, &catalog.rows("true", Some(&wanted), &[])).first().map(String::as_str), Some("constant true"));
    assert!(catalog.rows("240 V", Some(&wanted), &[]).iter().all(|row| !matches!(row, Row::Typed(_))));
    let volts = Wanted { from_output: false, key: Some(volts()), text: "f64[V]".to_owned(), family: "Number/0".to_owned() };
    assert_eq!(catalog.rows("240 V", Some(&volts), &[]).first(), Some(&Row::Typed("240 V".to_owned())));
}

#[test]
fn a_session_goes_to_the_session_of_another_name() {
    let catalog = catalog();
    let wanted = Wanted { from_output: true, key: Some(FitKey::Shape(7)), text: "visa.session".to_owned(), family: "Session/0".to_owned() };
    assert_eq!(calls(&catalog, &catalog.rows("", Some(&wanted), &[])), ["psu.set_voltage", "settle", "core.select"]);
}

#[test]
fn the_search_then_what_was_inserted_last_then_the_alphabet() {
    let catalog = catalog();
    let recent = vec!["time.wait".to_owned()];
    let all = calls(&catalog, &catalog.rows("", None, &recent));
    assert_eq!(all.first().map(String::as_str), Some("time.wait"));
    assert_eq!(all.get(1).map(String::as_str), Some("Reading.Missing"));
    // A query keeps what matches, the best match first.
    assert_eq!(calls(&catalog, &catalog.rows("now", None, &recent)), ["time.now"]);
    assert_eq!(calls(&catalog, &catalog.rows("240 V", None, &[])).first().map(String::as_str), Some("constant 240 V"));
}

#[test]
fn a_node_goes_into_a_wire_when_it_takes_and_gives_its_value() {
    let catalog = catalog();
    let wanted = Wanted { from_output: true, key: Some(volts()), text: "f64[V]".to_owned(), family: "Number/0".to_owned() };
    let into = |call: &str| catalog.entry(call).is_some_and(|entry| catalog.goes_into(entry, &wanted));
    assert!(into("core.add") && into("settle") && into("core.select"));
    assert!(!into("core.lt") && !into("time.now") && !into("Reading.Missing"));
}

#[test]
fn recent_insertions_are_kept_once_the_last_first() {
    let mut recent = Vec::new();
    for call in ["a", "b", "a"] {
        remember(&mut recent, call);
    }
    assert_eq!(recent, ["a", "b"]);
    for index in 0..40 {
        remember(&mut recent, &index.to_string());
    }
    assert_eq!(recent.len(), RECENT);
}

#[test]
fn a_function_input_takes_no_wire_of_a_value() {
    let catalog = Catalog::new(&[listed(
        "",
        "sweep",
        "function",
        "fn sweep(measure: fn(f64) -> (f64), limit: f64[V], unit: string = \")>\") -> (ok: bool)",
        json!({
            "inputs": [
                { "name": "measure", "type": "fn(f64) -> (f64)", "fit": { "shape": "fn(f64) -> (f64)" }, "required": true },
                { "name": "limit", "type": "f64[V]", "fit": volts_fit(), "required": true },
                { "name": "unit", "type": "string", "fit": { "base": "string" } },
            ],
            "outputs": [{ "name": "ok", "type": "bool", "fit": { "base": "bool" } }],
        }),
    )]);
    let inputs = catalog.entry("sweep").map(|entry| entry.inputs.clone()).unwrap_or_default();
    assert_eq!(inputs.iter().map(|input| input.name.as_str()).collect::<Vec<_>>(), ["measure", "limit", "unit"]);
    assert_eq!(inputs.get(1).map(|input| &input.ty), Some(&SlotTy::Key(volts())));
    // A function takes no wire of volts: `sweep` is listed for its `limit`, the second input, not for `measure`.
    let wanted = Wanted { from_output: true, key: Some(volts()), text: "f64[V]".to_owned(), family: "Number/0".to_owned() };
    assert_eq!(catalog.entry("sweep").and_then(|entry| catalog.fits(entry, &wanted)), Some(1));
}

/// An input typed by an alias of a number has the fit of that number, as the engine follows it (commands.md 6.1): a wire
/// of `f64[V]` goes to an input of `Volts = f64[mV]` by a conversion that the wire shows (spec 3.3), so the list offers it,
/// as `wire.connect` takes it; a wire of `f64[A]`, of another dimension, does not.
#[test]
fn an_input_of_an_alias_of_a_number_takes_a_wire_of_that_dimension() {
    let catalog = Catalog::new(&[
        listed("", "Volts", "type", "type Volts = f64[mV]", json!({ "is": { "type": "f64[mV]", "fit": { "number": "f64", "unit": "mV" } } })),
        listed(
            "",
            "trim",
            "function",
            "fn trim(level: Volts) -> (ok: bool)",
            json!({
                "inputs": [{ "name": "level", "type": "Volts", "fit": { "number": "f64", "unit": "mV" }, "required": true }],
                "outputs": [{ "name": "ok", "type": "bool", "fit": { "base": "bool" } }],
            }),
        ),
    ]);
    let volts = Wanted { from_output: true, key: Some(volts()), text: "f64[V]".to_owned(), family: "Number/0".to_owned() };
    assert_eq!(calls(&catalog, &catalog.rows("", Some(&volts), &[])), ["trim"]);
    let amperes = FitKey::Number(NumberKind::F64, parse_unit("A", MAX_NESTING).unwrap_or(Unit::ONE));
    let amperes = Wanted { from_output: true, key: Some(amperes), text: "f64[A]".to_owned(), family: "Number/0".to_owned() };
    assert!(catalog.rows("", Some(&amperes), &[]).is_empty());
}
