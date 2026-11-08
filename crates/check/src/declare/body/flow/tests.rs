//! Pass 5, on small programs: what each code says, and what must stay silent.

use std::borrow::Cow;

use labcraft_types::{Diagnostic, Interner, Limits};

use crate::{Content, Found, Libraries, LibrarySource, Origin, SourceFile, check};

/// `visa`, as the shipped library declares it: `session` is a resource.
struct Shipped;

fn file(shown: &str, text: &str) -> SourceFile {
    let text = format!("labcraft 1\n\n{text}\n");
    SourceFile { shown: shown.to_owned(), content: Content::Bytes(Cow::Owned(text.into_bytes())) }
}

impl Libraries for Shipped {
    fn find(&self, name: &str, _: Origin) -> Found {
        if name != "visa" {
            return Found::NotFound;
        }
        let mut source = LibrarySource::new("visa".to_owned(), "visa".to_owned(), Origin::Shipped);
        source.manifest = Some("name = \"visa\"\nversion = \"1\"\nresources = [\"session\", \"link\"]\n".to_owned());
        source.files = vec![
            file("session.lct", "type session = opaque"),
            file(
                "query.lcf",
                "extern rust(crate: \"c\", function: \"q\")\nfn query(session: session, command: string, err: error = none) -> (value: f64, session: session, err: error)",
            ),
            file("close.lcf", "extern rust(crate: \"c\", function: \"c\")\nfn close(session: session, err: error = none) -> (err: error)"),
            file(
                "open.lcf",
                "extern rust(crate: \"c\", function: \"o\")\nfn open(resource: string, err: error = none) -> (session: session, err: error)",
            ),
            file("link.lct", "type link = opaque"),
            file("hand.lcf", "extern rust(crate: \"c\", function: \"h\")\nfn hand(s: session, err: error = none) -> (h: link, err: error)"),
            file("press.lcf", "extern rust(crate: \"c\", function: \"p\")\nfn press(h: link, err: error = none) -> (err: error)"),
            file(
                "renamed.lcf",
                "extern rust(crate: \"c\", function: \"r\")\nfn renamed(s: session, err: error = none) -> (other: session, err: error)",
            ),
        ];
        Found::Library(source)
    }
}

fn diagnostics(files: &[(&str, &str)]) -> Vec<Diagnostic> {
    let mut names = Interner::new();
    let files = files.iter().map(|(shown, text)| file(shown, text)).collect();
    let source = LibrarySource { files, ..LibrarySource::new(String::new(), String::new(), Origin::Project) };
    let mut report = check(source, &Shipped, &mut names, &Limits::default()).report;
    report.finish();
    report.diagnostics().to_vec()
}

const USES: &str = "use core\nuse errors\nuse ui\nuse visa\n";

fn judged_with(extra: &[(&str, &str)], inputs: &str, outputs: &str, body: &str) -> Vec<&'static str> {
    let text = format!("{USES}\nfn f({inputs}) -> ({outputs}) {{\n{body}\n}}");
    let mut files = vec![("f.lcf", text.as_str())];
    files.extend_from_slice(extra);
    diagnostics(&files).iter().map(|diagnostic| diagnostic.code().as_str()).collect()
}

fn judged(inputs: &str, outputs: &str, body: &str) -> Vec<&'static str> {
    judged_with(&[], inputs, outputs, body)
}

fn messages(inputs: &str, outputs: &str, body: &str) -> Vec<String> {
    let text = format!("{USES}\nfn f({inputs}) -> ({outputs}) {{\n{body}\n}}");
    diagnostics(&[("f.lcf", text.as_str())]).iter().map(|diagnostic| diagnostic.message.clone()).collect()
}

/// A body with a panel section after it.
fn judged_panel(outputs: &str, body: &str, panel: &str) -> Vec<&'static str> {
    let text = format!("{USES}\nfn f(a: f64 = 1.0) -> ({outputs}) {{\n{body}\n}}\n\npanel {{\n{panel}\n}}");
    diagnostics(&[("f.lcf", text.as_str())]).iter().map(|diagnostic| diagnostic.code().as_str()).collect()
}

const NOTHING: [&str; 0] = [];

#[test]
fn a_cycle_is_said_once_and_what_waits_for_it_is_silent() {
    let body = "a = core.add(x, b)\nb = core.mul(a, 2.0)\nc = core.add(a, 1.0)\ny = c";
    assert_eq!(judged("x: f64", "y: f64", body), ["LC4001"]);
    assert_eq!(messages("x: f64", "y: f64", body), ["cycle: `a`, `b` wait for each other: a value can be fed back only through a carry (5.2)"]);
}

#[test]
fn a_line_can_wait_for_itself() {
    assert_eq!(
        messages("x: f64", "y: f64", "y = core.add(y, x)"),
        ["cycle: `y` waits for itself: a value can be fed back only through a carry (5.2)"]
    );
}

#[test]
fn an_order_wire_counts_in_a_cycle() {
    let body = "#a: u = core.add(x, 1.0, after: #b)\n#b: v = core.add(x, 2.0, after: #a)\ny = core.add(u, v)";
    assert_eq!(judged("x: f64", "y: f64", body), ["LC4001"]);
}

#[test]
fn a_cycle_passes_through_a_structure_and_through_an_output() {
    let arm = "y = match s {\n    true => {\n        z = core.add(y, 1.0)\n        yield z\n    }\n    false => yield 0.0\n}";
    assert_eq!(judged("s: bool", "y: f64", arm), ["LC4001"]);
    assert_eq!(judged("x: f64", "y: f64", "a = core.add(y, x)\ny = core.mul(a, 2.0)"), ["LC4001"], "an output can be read back (R-2)");
}

#[test]
fn an_input_and_an_output_of_one_name_make_no_cycle() {
    assert_eq!(judged("err: error = none", "err: error", "err = err"), NOTHING, "reading it designates the input (R-2)");
}

#[test]
fn a_carry_is_how_a_value_comes_back() {
    let body = "y = loop carry p = 0.0 {\n    s = core.add(x, p)\n    done = core.ge(s, 10.0)\n    next p = s\n    until done\n}";
    assert_eq!(judged("x: f64", "y: f64", body), NOTHING);
}

#[test]
fn many_cycles_name_a_few_of_their_lines() {
    let lines: String = (0..7).map(|i| format!("a{i} = core.add(a{}, x)\n", (i + 1) % 7)).collect();
    let got = messages("x: f64", "y: f64", &format!("{lines}y = a0"));
    assert_eq!(got, ["cycle: `a0`, `a1`, `a2`, `a3`, `a4` and 2 more wait for each other: a value can be fed back only through a carry (5.2)"]);
}

#[test]
fn an_output_is_bound_once() {
    assert_eq!(judged("x: f64", "y: f64, z: f64", "y = x"), ["LC4002"]);
    assert_eq!(messages("x: f64", "y: f64, z: f64", "y = x"), ["the output `z` is never bound (R-2)"]);
    assert_eq!(judged("x: f64", "y: f64", "y = x\ny = 1.0"), ["LC4002"]);
    assert_eq!(judged("x: f64", "y: f64", "y = x\ny = 1.0\ny = 2.0"), ["LC4002"], "once per output");
}

#[test]
fn a_line_of_a_sub_body_binds_no_output() {
    let body = "a = match s {\n    true => {\n        y = 1.0\n        yield y\n    }\n    false => yield 0.0\n}\ny = a";
    assert_eq!(judged("s: bool", "y: f64", body), NOTHING, "R-3");
}

#[test]
fn a_carry_has_one_next() {
    let without = "y = loop carry p = 0.0 {\n    done = core.ge(p, 1.0)\n    until done\n}";
    assert_eq!(judged("", "y: f64", without), ["LC4003"]);
    assert_eq!(messages("", "y: f64", without), ["the carry `p` has no `next` (R-6)"]);
    let twice = "y = loop carry p = 0.0 {\n    done = core.ge(p, 1.0)\n    next p = 1.0\n    next p = 2.0\n    until done\n}";
    assert_eq!(judged("", "y: f64", twice), ["LC4003"]);
}

#[test]
fn a_next_that_names_no_carry_is_one_mistake() {
    let typo = "y = loop carry p = 0.0 {\n    done = core.ge(p, 1.0)\n    next q = 1.0\n    until done\n}";
    assert_eq!(judged("", "y: f64", typo), ["LC2018"]);
}

#[test]
fn for_parallel_takes_no_carry() {
    assert_eq!(judged("xs: [f64]", "y: f64", "y = for parallel x in xs, carry c = 0.0 {\n    next c = x\n}"), ["LC4004"]);
    assert_eq!(judged("xs: [f64]", "y: f64", "y = for parallel x in xs, carry c = 0.0 {\n}"), ["LC4004"], "once, without `LC4003` on the same carry");
    assert_eq!(judged("xs: [f64]", "y: f64", "y = for x in xs, carry c = 0.0 {\n    next c = x\n}"), NOTHING);
}

#[test]
fn an_err_output_wired_nowhere() {
    let (inputs, outputs) = ("d: visa.session", "y: f64");
    assert_eq!(judged(inputs, outputs, "y, _, _ = visa.query(d, \"a\")"), ["LC4101"], "`_` wires it nowhere");
    assert_eq!(
        messages(inputs, outputs, "y, _, _ = visa.query(d, \"a\")"),
        ["the `err` output of `visa.query` is wired nowhere: an error would be lost (7.5)"]
    );
    assert_eq!(judged(inputs, outputs, "y, _, e = visa.query(d, \"a\")"), ["LC4101"], "a name that nothing reads");
    assert_eq!(judged(inputs, outputs, "y, _, e = visa.query(d, \"a\")\ne2 = e"), ["LC4101"], "a name for it, that nothing reads");
    assert_eq!(judged(inputs, outputs, "y, _, e = visa.query(d, \"a\")\n_ = e"), ["LC4101"]);
    assert_eq!(judged(inputs, outputs, "y = 1.0\nvisa.close(d)"), ["LC4101"], "a call alone (5.1)");
}

#[test]
fn what_wires_an_err_output() {
    let (inputs, outputs) = ("d: visa.session", "y: f64, err: error");
    assert_eq!(judged(inputs, outputs, "y, _, err = visa.query(d, \"a\")"), NOTHING, "it binds an output");
    assert_eq!(judged(inputs, outputs, "y, _, e = visa.query(d, \"a\")\nerr = e"), NOTHING);
    assert_eq!(judged(inputs, outputs, "y, _, e = visa.query(d, \"a\")\ne2 = e\nerr = e2"), NOTHING, "through a name");
    assert_eq!(judged(inputs, outputs, "y, _, e = visa.query(d, \"a\")\nerr = errors.merge(e, none)"), NOTHING);
    let after = "y, _, e = visa.query(d, \"a\")\nt = core.add(y, 1.0, after: e)\nerr = none\n_ = t";
    assert_eq!(judged(inputs, outputs, after), NOTHING, "an order wire is a wire");
    let handled = "y, _, e = visa.query(d, \"a\")\nerr = handle e {\n    _ => yield none\n}";
    assert_eq!(judged(inputs, outputs, handled), NOTHING, "`handle` clears it on purpose");
}

#[test]
fn only_an_output_named_err_counts() {
    let other = [("g.lcf", "fn g() -> (status: error) {\n    status = none\n}")];
    assert_eq!(judged_with(&other, "", "y: f64", "_ = g()\ny = 1.0"), NOTHING);
}

#[test]
fn what_a_structure_gives_is_not_judged() {
    let body =
        "y, _ = loop carry v = 0.0, carry e: error = none {\n    done = core.ge(v, 1.0)\n    next v = 1.0\n    next e = none\n    until done\n}";
    assert_eq!(judged("", "y: f64", body), NOTHING);
}

const TWO_UNORDERED: &str = "a, _, e1 = visa.query(d, \"a\")\nb, _, e2 = visa.query(d, \"b\")\nerr = errors.merge(e1, e2)";

#[test]
fn two_nodes_on_a_resource_need_an_order() {
    let outputs = "a: f64, b: f64, err: error";
    assert_eq!(judged("d: visa.session", outputs, TWO_UNORDERED), ["LC4102"]);
    assert_eq!(messages("d: visa.session", outputs, TWO_UNORDERED), ["concurrent access to `d`: order by the error wire or `after:` (8.1)"]);
    let by_error = "a, _, e1 = visa.query(d, \"a\")\nb, _, err = visa.query(d, \"b\", err: e1)";
    assert_eq!(judged("d: visa.session", outputs, by_error), NOTHING, "the error wire");
    let by_after = "a, _, e1 = visa.query(d, \"a\")\nb, _, e2 = visa.query(d, \"b\", after: e1)\nerr = errors.merge(e1, e2)";
    assert_eq!(judged("d: visa.session", outputs, by_after), NOTHING, "an order wire");
    let by_label = "#one: a, _, e1 = visa.query(d, \"a\")\nb, _, e2 = visa.query(d, \"b\", after: #one)\nerr = errors.merge(e1, e2)";
    assert_eq!(judged("d: visa.session", outputs, by_label), NOTHING, "a label");
}

#[test]
fn a_warning_comes_once_per_resource() {
    let body = "a, _, e1 = visa.query(d, \"a\")\nb, _, e2 = visa.query(d, \"b\")\nc, _, e3 = visa.query(d, \"c\")\nerr = errors.merge(e1, e2, e3)";
    assert_eq!(judged("d: visa.session", "a: f64, b: f64, c: f64, err: error", body), ["LC4102"]);
}

#[test]
fn a_session_that_passes_through_a_node_is_the_same_resource() {
    let body = "a, d2, e1 = visa.query(d, \"a\")\nb, _, e2 = visa.query(d, \"b\", err: e1)\ne3 = visa.close(d2, err: e1)\nerr = errors.merge(e2, e3)";
    assert_eq!(judged("d: visa.session", "a: f64, b: f64, err: error", body), ["LC4102"], "the session can close during the second query");
    let ordered = "a, d2, e1 = visa.query(d, \"a\")\nb, d3, e2 = visa.query(d2, \"b\", err: e1)\nerr = visa.close(d3, err: e2)";
    assert_eq!(judged("d: visa.session", "a: f64, b: f64, err: error", ordered), NOTHING);
}

#[test]
fn the_name_of_the_output_says_nothing() {
    let body = "s, e1 = visa.renamed(d)\na, _, e2 = visa.query(d, \"a\", err: e1)\ne3 = visa.close(s, err: e1)\nerr = errors.merge(e2, e3)";
    assert_eq!(judged("d: visa.session", "a: f64, err: error", body), ["LC4102"]);
}

#[test]
fn a_node_that_receives_two_sessions_continues_both() {
    let both = [(
        "join.lcf",
        "use visa\n\nfn join(a: visa.session, b: visa.session) -> (s: visa.session, err: error) {\n    s = a\n    err = visa.close(b)\n}",
    )];
    let body = "s, e1 = join(a, b)\nx, _, e2 = visa.query(a, \"x\", err: e1)\ny, _, e3 = visa.query(s, \"y\", err: e1)\nerr = errors.merge(e2, e3)";
    assert_eq!(judged_with(&both, "a: visa.session, b: visa.session", "x: f64, y: f64, err: error", body), ["LC4102"]);
    let distinct = "x, _, e1 = visa.query(a, \"x\")\ny, _, e2 = visa.query(b, \"y\")\nerr = errors.merge(e1, e2)";
    assert_eq!(judged("a: visa.session, b: visa.session", "x: f64, y: f64, err: error", distinct), NOTHING, "two sessions are two resources");
}

#[test]
fn a_session_opened_here_is_a_resource_of_its_own() {
    let body =
        "d, e1 = visa.open(\"r\")\na, _, e2 = visa.query(d, \"a\", err: e1)\nb, _, e3 = visa.query(d, \"b\", err: e1)\nerr = errors.merge(e2, e3)";
    assert_eq!(judged("", "a: f64, b: f64, err: error", body), ["LC4102"]);
}

#[test]
fn a_name_for_a_session_and_a_carry_are_the_same_resource() {
    let named = "e = d\na, _, e1 = visa.query(e, \"a\")\nb, _, e2 = visa.query(d, \"b\")\nerr = errors.merge(e1, e2)";
    assert_eq!(judged("d: visa.session", "a: f64, b: f64, err: error", named), ["LC4102"]);
    let carried = "s, x, e1 = loop carry c = d, carry v = 0.0, carry e: error = none {\n    q, c2, e2 = visa.query(c, \"a\", err: e)\n    done = core.ge(q, 1.0)\n    next c = c2\n    next v = q\n    next e = e2\n    until done\n}\ny, _, e3 = visa.query(d, \"b\")\nerr = errors.merge(e1, e3)\n_ = s\nz = core.add(x, y)";
    assert_eq!(judged("d: visa.session", "z: f64, err: error", carried), ["LC4102"], "the loop and the node after it are not ordered");
}

#[test]
fn the_parts_of_a_struct_are_resources_and_the_struct_overlaps_them() {
    let bench = [
        ("Bench.lct", "use visa\n\ntype Bench = struct { dmm: visa.session, source: visa.session }"),
        ("use_all.lcf", "fn use_all(b: Bench, err: error = none) -> (err: error) {\n    err = err\n    _ = b\n}"),
    ];
    let inputs = "b: Bench";
    let parts = "x, _, e1 = visa.query(b.dmm, \"x\")\ny, _, e2 = visa.query(b.source, \"y\")\nerr = errors.merge(e1, e2)";
    assert_eq!(judged_with(&bench, inputs, "x: f64, y: f64, err: error", parts), NOTHING, "two different resources");
    let same = "x, _, e1 = visa.query(b.dmm, \"x\")\ny, _, e2 = visa.query(b.dmm, \"y\")\nerr = errors.merge(e1, e2)";
    assert_eq!(judged_with(&bench, inputs, "x: f64, y: f64, err: error", same), ["LC4102"]);
    let whole = "x, _, e1 = visa.query(b.dmm, \"x\")\ne2 = use_all(b)\nerr = errors.merge(e1, e2)";
    assert_eq!(judged_with(&bench, inputs, "x: f64, err: error", whole), ["LC4102"], "`bench` overlaps both");
    let text = format!("{USES}\nfn f(b: Bench) -> (x: f64, y: f64, err: error) {{\n{same}\n}}");
    let mut files = vec![("f.lcf", text.as_str())];
    files.extend_from_slice(&bench);
    let got: Vec<String> = diagnostics(&files).iter().map(|diagnostic| diagnostic.message.clone()).collect();
    assert_eq!(got, ["concurrent access to `b.dmm`: order by the error wire or `after:` (8.1)"]);
}

#[test]
fn two_arms_never_run_together() {
    let arms = "a, err = match s {\n    true => {\n        x, _, e1 = visa.query(d, \"x\")\n        yield (x, e1)\n    }\n    false => {\n        z, _, e2 = visa.query(d, \"z\")\n        yield (z, e2)\n    }\n}";
    assert_eq!(judged("d: visa.session, s: bool", "a: f64, err: error", arms), NOTHING);
    let inside = "a, err = match s {\n    true => {\n        x, _, e1 = visa.query(d, \"x\")\n        z, _, e2 = visa.query(d, \"z\")\n        all = errors.merge(e1, e2)\n        w = core.add(x, z)\n        yield (w, all)\n    }\n    false => yield (0.0, none)\n}";
    assert_eq!(judged("d: visa.session, s: bool", "a: f64, err: error", inside), ["LC4102"], "two nodes of one arm do");
    let outside = "b, _, e0 = visa.query(d, \"b\")\na, e1 = match s {\n    true => {\n        x, _, e2 = visa.query(d, \"x\")\n        yield (x, e2)\n    }\n    false => yield (0.0, none)\n}\nc = core.add(a, b)\nerr = errors.merge(e0, e1)";
    assert_eq!(judged("d: visa.session, s: bool", "c: f64, err: error", outside), ["LC4102"], "an arm and the outside do");
}

#[test]
fn a_structure_is_ordered_by_what_opens_it() {
    let body = "b, _, e0 = visa.query(d, \"b\")\na, err = match s {\n    true => {\n        x, _, e2 = visa.query(d, \"x\", err: e0)\n        yield (x, e2)\n    }\n    false => yield (b, e0)\n}";
    assert_eq!(judged("d: visa.session, s: bool", "a: f64, err: error", body), NOTHING);
}

#[test]
fn a_primitive_and_a_construction_do_not_reach_a_resource() {
    let picked = "e = core.select(s, a, b)\nx, _, err = visa.query(e, \"x\")";
    assert_eq!(judged("a: visa.session, b: visa.session, s: bool", "x: f64, err: error", picked), NOTHING);
    let continued = "e = core.select(s, a, b)\nx, _, e1 = visa.query(e, \"x\")\ny, _, e2 = visa.query(a, \"y\")\nerr = errors.merge(e1, e2)";
    assert_eq!(judged("a: visa.session, b: visa.session, s: bool", "x: f64, y: f64, err: error", continued), ["LC4102"], "what it moves continues");
}

#[test]
fn a_type_of_a_project_is_no_resource_before_its_manifest() {
    let own = [("h.lct", "type h = opaque"), ("g.lcf", "extern rust(crate: \"c\", function: \"g\")\nfn g(a: h, err: error = none) -> (err: error)")];
    let body = "e1 = g(a)\ne2 = g(a)\nerr = errors.merge(e1, e2)";
    assert_eq!(judged_with(&own, "a: h", "err: error", body), NOTHING);
}

#[test]
fn two_writes_of_an_element_need_an_order() {
    let body = "#one: x = ui.set(shown, a)\n#two: z = ui.set(shown, a)\ny = core.add(x, z)";
    assert_eq!(judged_panel("y: f64", body, "    indicator shown: numeric"), ["LC4103"]);
    let ordered = "#one: x = ui.set(shown, a)\n#two: z = ui.set(shown, a, after: #one)\ny = core.add(x, z)";
    assert_eq!(judged_panel("y: f64", ordered, "    indicator shown: numeric"), NOTHING);
    let arms = "y = match core.gt(a, 1.0) {\n    true => {\n        x = ui.set(shown, a)\n        yield x\n    }\n    false => {\n        z = ui.set(shown, a)\n        yield z\n    }\n}";
    assert_eq!(
        judged_panel("y: f64", &format!("c = core.gt(a, 1.0)\n{}", arms.replace("match core.gt(a, 1.0)", "match c")), "    indicator shown: numeric"),
        NOTHING
    );
    let reads = "x = ui.get(shown)\nz = ui.get(shown)\nw = ui.set(shown, a)\ny = core.add(core_x, z)";
    let reads = reads.replace("core_x", "x");
    assert_eq!(judged_panel("y: f64", &format!("{reads}\n_ = w"), "    control shown: numeric(default: 0.0)"), NOTHING, "a read is no write");
}

#[test]
fn a_body_of_for_parallel_uses_a_resource_from_outside() {
    let shared = "ys, es = for parallel c in cs {\n    y, _, e = visa.query(d, c)\n    collect y\n    collect e\n}";
    assert_eq!(judged("d: visa.session, cs: [string]", "ys: [f64], es: [error]", shared), ["LC4104"]);
    assert_eq!(
        messages("d: visa.session, cs: [string]", "ys: [f64], es: [error]", shared),
        ["this `for parallel` uses `d`, a resource from outside: its iterations have no order (8.1)"]
    );
    let twice = "ys, es = for parallel c in cs {\n    y, d2, e1 = visa.query(d, c)\n    e = visa.close(d2, err: e1)\n    collect y\n    collect e\n}";
    assert_eq!(judged("d: visa.session, cs: [string]", "ys: [f64], es: [error]", twice), ["LC4104"], "once per loop");
}

#[test]
fn a_resource_born_in_the_body_is_its_own() {
    let own = "ys, es = for parallel r in rs {\n    d, e1 = visa.open(r)\n    y, d2, e2 = visa.query(d, \"a\", err: e1)\n    e = visa.close(d2, err: e2)\n    collect y\n    collect e\n}";
    assert_eq!(judged("rs: [string]", "ys: [f64], es: [error]", own), NOTHING);
    let element = "ys, es = for parallel s in ss {\n    y, _, e = visa.query(s, \"a\")\n    collect y\n    collect e\n}";
    assert_eq!(judged("ss: [visa.session]", "ys: [f64], es: [error]", element), NOTHING, "each iteration has its element");
}

#[test]
fn a_body_of_for_parallel_writes_an_element() {
    let text = format!(
        "{USES}\nfn f(xs: [f64]) -> (ys: [f64]) {{\n    ys = for parallel x in xs {{\n        y = ui.set(shown, x)\n        collect y\n    }}\n}}\n\npanel {{\n    indicator shown: numeric\n}}"
    );
    let got: Vec<&str> = diagnostics(&[("f.lcf", text.as_str())]).iter().map(|diagnostic| diagnostic.code().as_str()).collect();
    assert_eq!(got, ["LC4104"]);
}

#[test]
fn two_reads_of_a_button() {
    let two = "x = ui.get(stop)\nz = ui.get(stop)\ny = core.or(x, z)";
    assert_eq!(judged_panel("y: bool", two, "    control stop: button"), ["LC4105"]);
    assert_eq!(judged_panel("y: bool", two, "    control stop: switch"), NOTHING, "a switch keeps its state");
    let one = "y = ui.get(stop)";
    assert_eq!(judged_panel("y: bool", one, "    control stop: button"), NOTHING);
    let arms = "c = core.gt(a, 1.0)\ny = match c {\n    true => {\n        x = ui.get(stop)\n        yield x\n    }\n    false => {\n        z = ui.get(stop)\n        yield z\n    }\n}";
    assert_eq!(judged_panel("y: bool", arms, "    control stop: button"), NOTHING, "only one of them runs");
}

/// Every order of these lines says the same (5.2).
fn in_any_order(inputs: &str, outputs: &str, lines: &[&str]) -> Vec<&'static str> {
    fn orders(lines: &[String]) -> Vec<Vec<String>> {
        if lines.len() < 2 {
            return vec![lines.to_vec()];
        }
        let mut all = Vec::new();
        for index in 0..lines.len() {
            let mut rest = lines.to_vec();
            let first = rest.remove(index);
            for mut order in orders(&rest) {
                order.insert(0, first.clone());
                all.push(order);
            }
        }
        all
    }
    let lines: Vec<String> = lines.iter().map(|line| (*line).to_owned()).collect();
    let mut seen: Vec<Vec<&'static str>> = Vec::new();
    for order in orders(&lines) {
        let got = judged(inputs, outputs, &order.join("\n"));
        if !seen.contains(&got) {
            seen.push(got);
        }
    }
    assert_eq!(seen.len(), 1, "{seen:?}");
    seen.remove(0)
}

const QUERY_THEN_ARMS: [&str; 3] = [
    "b, _, e0 = visa.query(d, \"b\")",
    "a, e1 = match s {\n    true => {\n        x, _, e2 = visa.query(d, \"x\", err: e0)\n        yield (x, e2)\n    }\n    false => {\n        z, _, e3 = visa.query(d, \"z\")\n        yield (z, e3)\n    }\n}",
    "c = core.add(a, b)\nerr = errors.merge(e0, e1)",
];

#[test]
fn an_arm_that_reads_a_wire_does_not_hide_the_other_arm() {
    assert_eq!(in_any_order("d: visa.session, s: bool", "c: f64, err: error", &QUERY_THEN_ARMS), ["LC4102"]);
    let swapped = QUERY_THEN_ARMS[1].replace("true =>", "HOLD =>").replace("false =>", "true =>").replace("HOLD =>", "false =>");
    let lines = [QUERY_THEN_ARMS[0], swapped.as_str(), QUERY_THEN_ARMS[2]];
    assert_eq!(in_any_order("d: visa.session, s: bool", "c: f64, err: error", &lines), ["LC4102"], "an arm written first or second");
}

#[test]
fn arms_nested_are_judged_by_where_they_part() {
    let body = "a, err = match s {\n    true => {\n        y, e0 = match t {\n            true => {\n                x, _, e1 = visa.query(d, \"x\")\n                yield (x, e1)\n            }\n            false => {\n                z, _, e2 = visa.query(d, \"z\")\n                yield (z, e2)\n            }\n        }\n        yield (y, e0)\n    }\n    false => {\n        w, _, e3 = visa.query(d, \"w\")\n        yield (w, e3)\n    }\n}";
    assert_eq!(judged("d: visa.session, s: bool, t: bool", "a: f64, err: error", body), NOTHING, "all three part somewhere");
    let beside = "a, err = match s {\n    true => {\n        y, e0 = match t {\n            true => {\n                x, _, e1 = visa.query(d, \"x\")\n                yield (x, e1)\n            }\n            false => yield (0.0, none)\n        }\n        v, _, e3 = visa.query(d, \"v\")\n        all = errors.merge(e0, e3)\n        w = core.add(y, v)\n        yield (w, all)\n    }\n    false => yield (0.0, none)\n}";
    assert_eq!(
        judged("d: visa.session, s: bool, t: bool", "a: f64, err: error", beside),
        ["LC4102"],
        "an inner arm and what stands beside its `match`"
    );
}

#[test]
fn a_whole_struct_stays_to_be_compared_with_its_other_parts() {
    let bench = [
        ("Bench.lct", "use visa\n\ntype Bench = struct { dmm: visa.session, source: visa.session }"),
        ("use_all.lcf", "fn use_all(b: Bench, err: error = none) -> (err: error) {\n    err = err\n    _ = b\n}"),
    ];
    let parts = "e0 = use_all(b)\nx, _, e1 = visa.query(b.dmm, \"x\", err: e0)\nt1 = core.add(q, 1.0)\nt2 = core.add(t1, 1.0)\nt3 = core.add(t2, 1.0)\ny, _, e2 = visa.query(b.source, \"y\", after: t3)\nerr = errors.merge(e1, e2)\n_ = x";
    assert_eq!(judged_with(&bench, "b: Bench, q: f64", "y: f64, err: error", parts), ["LC4102"], "`use_all` and `b.source` have no order");
    let ordered = "e0 = use_all(b)\nx, _, e1 = visa.query(b.dmm, \"x\", err: e0)\ny, _, e2 = visa.query(b.source, \"y\", err: e1)\nz = core.add(x, y)\nerr = e2";
    assert_eq!(judged_with(&bench, "b: Bench", "z: f64, err: error", ordered), NOTHING);
}

#[test]
fn a_plain_field_of_a_struct_with_a_session_is_no_resource() {
    let rig = [
        ("Rig.lct", "use visa\n\ntype Rig = struct { dmm: visa.session, limit: f64 }"),
        ("within.lcf", "fn within(x: f64) -> (y: f64) {\n    y = x\n}"),
    ];
    assert_eq!(judged_with(&rig, "r: Rig", "y: f64", "a = within(r.limit)\nb = within(r.limit)\ny = core.add(a, b)"), NOTHING);
    assert_eq!(
        judged_with(&rig, "r: Rig, xs: [f64]", "ys: [f64]", "ys = for parallel x in xs {\n    y = within(r.limit)\n    collect y\n}"),
        NOTHING
    );
}

#[test]
fn core_call_runs_a_function() {
    let g = [("g.lcf", "use visa\n\nfn g(s: visa.session, err: error = none) -> (err: error) {\n    err = err\n    _ = s\n}")];
    let body = "e1 = core.call(g, d, none)\ne2 = core.call(g, d, none)\nerr = errors.merge(e1, e2)";
    assert_eq!(judged_with(&g, "d: visa.session", "err: error", body), ["LC4102"]);
}

#[test]
fn what_a_loop_gives_continues_what_its_next_gives_too() {
    let body = "s, e1 = loop carry c = d, carry e: error = none {\n    q, o2, e2 = visa.query(o, \"a\", err: e)\n    done = core.ge(q, 1.0)\n    next c = o2\n    next e = e2\n    until done\n}\nx, _, e3 = visa.query(s, \"x\", err: e1)\ny, _, e4 = visa.query(o, \"y\", err: e1)\nz = core.add(x, y)\nerr = errors.merge(e3, e4)";
    assert_eq!(judged("d: visa.session, o: visa.session", "z: f64, err: error", body), ["LC4102"]);
}

#[test]
fn a_line_that_reads_a_carry_does_not_depend_on_where_its_next_stands() {
    let lines = [
        "p, _, e1 = visa.query(c, \"p\", err: e)",
        "q, o2, e2 = visa.query(o, \"q\", err: e1)",
        "done = core.ge(core_p, 1.0)",
        "next c = o2",
        "next v = q",
        "next e = e2",
        "until done",
    ];
    let mut bodies = Vec::new();
    for first in 0..lines.len() {
        let mut order: Vec<String> = lines.iter().map(|line| line.replace("core_p", "p")).collect();
        order.rotate_left(first);
        bodies
            .push(format!("s, v, e0 = loop carry c = d, carry v = 0.0, carry e: error = none {{\n{}\n}}\nerr = e0\ny = v\n_ = s", order.join("\n")));
    }
    let seen: Vec<Vec<&str>> = bodies.iter().map(|body| judged("d: visa.session, o: visa.session", "y: f64, err: error", body)).collect();
    assert!(seen.windows(2).all(|pair| pair[0] == pair[1]), "{seen:?}");
}

#[test]
fn the_element_of_a_parallel_loop_is_still_what_the_array_holds() {
    let then = "ys, es = for parallel s in ss {\n    y, _, e = visa.query(s, \"a\")\n    collect y\n    collect e\n}\none, e0 = array.get(ss, 0)\nx, _, e1 = visa.query(one, \"b\", err: e0)\nz = core.add(x, 1.0)\nerr = e1\n_ = ys\n_ = es\n_ = z";
    let text = format!("use array\n{USES}\nfn f(ss: [visa.session]) -> (err: error) {{\n{then}\n}}");
    let got: Vec<&str> = diagnostics(&[("f.lcf", text.as_str())]).iter().map(|diagnostic| diagnostic.code().as_str()).collect();
    assert_eq!(got, ["LC4102"], "the loop and the node after it use one session");
    let two = "a, ea = for parallel s in ss {\n    y, _, e = visa.query(s, \"a\")\n    collect y\n    collect e\n}\nb, eb = for parallel s in ss {\n    y, _, e = visa.query(s, \"b\")\n    collect y\n    collect e\n}";
    assert_eq!(judged("ss: [visa.session]", "a: [f64], ea: [error], b: [f64], eb: [error]", two), ["LC4102"], "two loops on one array");
}

#[test]
fn a_resource_born_in_an_inner_parallel_loop_is_its_own_in_the_outer_one() {
    let extra = [("g.lcf", "use visa\n\nfn g(ss: [visa.session], err: error = none) -> (err: error) {\n    err = err\n    _ = ss\n}")];
    let body = "es = for parallel r in rs {\n    ss, oks = for parallel x in rs {\n        d, e0 = visa.open(x)\n        collect d\n        collect e0\n    }\n    e = g(ss)\n    _ = oks\n    collect e\n}";
    assert_eq!(judged_with(&extra, "rs: [string]", "es: [error]", body), NOTHING);
    let outer = "es = for parallel r in rs {\n    oks = for parallel x in rs {\n        y, _, e0 = visa.query(d, x)\n        _ = y\n        collect e0\n    }\n    _ = oks\n    _ = r\n    collect none\n}";
    assert_eq!(judged_with(&extra, "d: visa.session, rs: [string]", "es: [error]", outer).len(), 2, "both loops use it from outside");
}

#[test]
fn a_warning_waits_for_the_errors_of_its_function() {
    let body = "y, _, e = visa.query(d, \"x\")\nerr = e1";
    assert_eq!(judged("d: visa.session", "y: f64, err: error", body), ["LC2001"], "the typo is why `e` is not read");
    let twice = "y, _, e = visa.query(d, \"x\")\ny, _, e = visa.query(d, \"x\", err: e)\nerr = e";
    let got = judged("d: visa.session", "y: f64, err: error", twice);
    assert!(!got.contains(&"LC4101"), "{got:?}");
}

#[test]
fn a_cycle_is_said_on_its_first_written_line() {
    let body = "y = match s {\n    true => {\n        z = core.add(y, 1.0)\n        yield z\n    }\n    false => yield 0.0\n}";
    let text = format!("{USES}\nfn f(s: bool) -> (y: f64) {{\n{body}\n}}");
    let found = diagnostics(&[("f.lcf", text.as_str())]);
    let [diagnostic] = found.as_slice() else { panic!("{found:?}") };
    assert_eq!(diagnostic.span.as_ref().map(|span| span.start.line), Some(9), "the `match`, not the line in its arm");
}

#[test]
fn an_output_of_another_resource_type_starts_a_new_resource() {
    let body = "h, e0 = visa.hand(d)\nx, _, e1 = visa.query(d, \"x\", err: e0)\ne2 = visa.press(h, err: e0)\nerr = errors.merge(e1, e2)";
    assert_eq!(judged("d: visa.session", "x: f64, err: error", body), NOTHING);
}

#[test]
fn a_value_follows_many_sessions() {
    let inputs: Vec<String> = (0..16).map(|i| format!("s{i}: visa.session")).collect();
    let items: Vec<String> = (0..16).map(|i| format!("s{i}")).collect();
    for last in ["s15", "s0"] {
        let body = format!(
            "all = [{}]\nfor_each, e0 = array.get(all, 0)\nx, _, e1 = visa.query(for_each, \"x\", err: e0)\ny, _, e2 = visa.query({last}, \"y\", err: e0)\nz = core.add(x, y)\nerr = errors.merge(e1, e2)",
            items.join(", ")
        );
        let text = format!("use array\n{USES}\nfn f({}) -> (z: f64, err: error) {{\n{body}\n}}", inputs.join(", "));
        let got: Vec<&str> = diagnostics(&[("f.lcf", text.as_str())]).iter().map(|diagnostic| diagnostic.code().as_str()).collect();
        assert_eq!(got, ["LC4102"], "{last}");
    }
}

#[test]
fn what_a_warning_names_does_not_depend_on_the_lines() {
    let bench = [
        ("Bench.lct", "use visa\n\ntype Bench = struct { dmm: visa.session, source: visa.session }"),
        ("use_b.lcf", "fn use_b(b: Bench, err: error = none) -> (err: error) {\n    err = err\n    _ = b\n}"),
    ];
    let lines = [
        "x, _, e0 = visa.query(b.dmm, \"x\")",
        "t1 = core.add(q, 1.0)",
        "e1 = use_b(b, err: e0)",
        "e2 = use_b(b, after: t1)",
        "err = errors.merge(e1, e2)",
        "y = x",
    ];
    let said = |body: String| -> Vec<String> {
        let text = format!("{USES}\nfn f(b: Bench, q: f64) -> (y: f64, err: error) {{\n{body}\n}}");
        let mut files = vec![("f.lcf", text.as_str())];
        files.extend_from_slice(&bench);
        let mut got: Vec<String> = diagnostics(&files).iter().map(|diagnostic| diagnostic.message.clone()).collect();
        got.sort();
        got
    };
    let first = said(lines.join("\n"));
    assert_eq!(first.len(), 2, "{first:?}");
    let mut rotated: Vec<&str> = lines.to_vec();
    for _ in 0..lines.len() {
        rotated.rotate_left(1);
        assert_eq!(said(rotated.join("\n")), first);
        let mut reversed = rotated.clone();
        reversed.reverse();
        assert_eq!(said(reversed.join("\n")), first);
    }
}

#[test]
fn a_parallel_loop_around_another_shares_what_the_array_holds() {
    let nested = "es = for parallel r in rs {\n    oks = for parallel t in ss {\n        y, _, e0 = visa.query(t, r)\n        _ = y\n        collect e0\n    }\n    _ = oks\n    collect none\n}";
    let got = messages("ss: [visa.session], rs: [string]", "es: [error]", nested);
    assert_eq!(got, ["this `for parallel` uses `ss`, a resource from outside: its iterations have no order (8.1)"], "the outer loop");
    let rows = "es = for parallel row in m {\n    oks = for parallel t in row {\n        y, _, e0 = visa.query(t, \"a\")\n        _ = y\n        collect e0\n    }\n    _ = oks\n    collect none\n}";
    assert_eq!(judged("m: [[visa.session]]", "es: [error]", rows), NOTHING, "each row is its loop's own, and each element too");
    let left = "es = for parallel r in rs {\n    got, oks = for parallel t in ss {\n        collect t\n        collect none\n    }\n    one, e0 = array.get(got, 0)\n    y, _, e1 = visa.query(one, r, err: e0)\n    _ = y\n    _ = oks\n    collect e1\n}";
    let text = format!("use array\n{USES}\nfn f(ss: [visa.session], rs: [string]) -> (es: [error]) {{\n{left}\n}}");
    let found: Vec<&str> = diagnostics(&[("f.lcf", text.as_str())]).iter().map(|diagnostic| diagnostic.code().as_str()).collect();
    assert_eq!(found, ["LC4104"], "an element that left its loop, used in the one around it");
}

#[test]
fn a_parallel_loop_names_one_resource_whatever_the_lines() {
    let lines = ["x, _, e1 = visa.query(c, \"x\")", "y, _, e2 = visa.query(a, \"y\")", "z = core.add(x, y)", "collect z", "collect errors.merge"];
    let body = |order: &[&str]| {
        let text =
            order.iter().map(|line| line.replace("collect errors.merge", "e = errors.merge(e1, e2)\ncollect e")).collect::<Vec<_>>().join("\n");
        format!("ys, es = for parallel q in qs {{\n_ = q\n{text}\n}}")
    };
    let mut other = lines.to_vec();
    other.swap(0, 1);
    for order in [lines.to_vec(), other] {
        let got = messages("a: visa.session, c: visa.session, qs: [string]", "ys: [f64], es: [error]", &body(&order));
        assert_eq!(got, ["this `for parallel` uses `a`, a resource from outside: its iterations have no order (8.1)"]);
    }
}

#[test]
fn a_signature_with_an_error_waits_too() {
    let got = judged("d: visa.session, d: visa.session", "y: f64, er: error", "y, _, err = visa.query(d, \"a\")");
    assert_eq!(got, ["LC2019"], "the typo in `er` is why the `err` output reads nothing");
}

#[test]
fn a_chain_on_a_part_is_searched_once() {
    let bench = [
        ("Bench.lct", "use visa\n\ntype Bench = struct { dmm: visa.session, source: visa.session }"),
        ("use_b.lcf", "fn use_b(b: Bench, err: error = none) -> (err: error) {\n    err = err\n    _ = b\n}"),
    ];
    let mut lines = vec!["e0 = use_b(b)".to_owned()];
    for i in 0..8000 {
        lines.push(format!("x{i}, _, e{} = visa.query(b.dmm, \"x\", err: e{i})\n_ = x{i}", i + 1));
    }
    lines.push("a, _, ea = visa.query(o, \"a\")\nz, _, ez = visa.query(o, \"z\")\n_ = a\n_ = z\nerr = errors.merge(e8000, ea, ez)".to_owned());
    assert_eq!(
        judged_with(&bench, "b: Bench, o: visa.session", "err: error", &lines.join("\n")),
        ["LC4102"],
        "the work allowed reaches the session `o`"
    );
}

#[test]
fn a_part_of_a_part_overlaps_what_holds_it() {
    let deep = [
        ("Inner.lct", "use visa\n\ntype Inner = struct { a: visa.session, c: visa.session }"),
        ("Outer.lct", "use visa\n\ntype Outer = struct { sub: Inner, other: visa.session }"),
        ("use_i.lcf", "fn use_i(i: Inner, err: error = none) -> (err: error) {\n    err = err\n    _ = i\n}"),
    ];
    let conflict = "x, _, e0 = visa.query(o.sub.a, \"x\")\ne1 = use_i(o.sub)\ny = x\nerr = errors.merge(e0, e1)";
    assert_eq!(judged_with(&deep, "o: Outer", "y: f64, err: error", conflict), ["LC4102"]);
    let apart = "x, _, e0 = visa.query(o.sub.a, \"x\")\nz, _, e1 = visa.query(o.other, \"z\")\ny = core.add(x, z)\nerr = errors.merge(e0, e1)";
    assert_eq!(judged_with(&deep, "o: Outer", "y: f64, err: error", apart), NOTHING);
}

#[test]
fn what_holds_a_resource() {
    let held = [
        ("Rig.lct", "use visa\n\ntype Rig = struct { d: visa.session }"),
        ("Held.lct", "use visa\n\ntype Held = enum { One(visa.session), Nothing }"),
        ("use_rigs.lcf", "fn use_rigs(r: [Rig], err: error = none) -> (err: error) {\n    err = err\n    _ = r\n}"),
        ("use_held.lcf", "fn use_held(h: Held, err: error = none) -> (err: error) {\n    err = err\n    _ = h\n}"),
    ];
    let two = |call: &str| format!("e1 = {call}\ne2 = {call}\nerr = errors.merge(e1, e2)");
    assert_eq!(judged_with(&held, "r: [Rig]", "err: error", &two("use_rigs(r)")), ["LC4102"], "an array of structs");
    assert_eq!(judged_with(&held, "h: Held", "err: error", &two("use_held(h)")), ["LC4102"], "an enumeration with a variant that holds one");
}

#[test]
fn a_value_bound_out_of_a_variant_is_what_the_variant_holds() {
    let held = [
        ("Held.lct", "use visa\n\ntype Held = enum { One(visa.session), Nothing }"),
        ("use_held.lcf", "fn use_held(h: Held, err: error = none) -> (err: error) {\n    err = err\n    _ = h\n}"),
    ];
    let body = "y, e1 = match h {\n    Held.One(s) => {\n        x, _, e0 = visa.query(s, \"x\")\n        yield (x, e0)\n    }\n    Held.Nothing => yield (0.0, none)\n}\ne2 = use_held(h)\nerr = errors.merge(e1, e2)";
    assert_eq!(judged_with(&held, "h: Held", "y: f64, err: error", body), ["LC4102"]);
}

#[test]
fn what_a_structure_gives_continues_what_it_held() {
    let by_match = "e, e0 = match s {\n    true => yield (a, none)\n    false => yield (a, none)\n}\nx, _, e1 = visa.query(e, \"x\", err: e0)\nz, _, e2 = visa.query(a, \"z\", err: e0)\ny = core.add(x, z)\nerr = errors.merge(e1, e2)";
    assert_eq!(judged("a: visa.session, s: bool", "y: f64, err: error", by_match), ["LC4102"], "what the arms yield");
    let by_for = "got = for t in ts {\n    _ = t\n    collect a\n}\nfirst, e0 = array.get(got, 0)\nx, _, e1 = visa.query(first, \"x\", err: e0)\nz, _, e2 = visa.query(a, \"z\", err: e0)\ny = core.add(x, z)\nerr = errors.merge(e1, e2)";
    let text = format!("use array\n{USES}\nfn f(a: visa.session, ts: [string]) -> (y: f64, err: error) {{\n{by_for}\n}}");
    let found: Vec<&str> = diagnostics(&[("f.lcf", text.as_str())]).iter().map(|diagnostic| diagnostic.code().as_str()).collect();
    assert_eq!(found, ["LC4102"], "what a loop collects");
}

#[test]
fn a_clause_of_a_for_orders_a_node_and_wires_an_error() {
    let body = "a, _, e1 = visa.query(d, \"a\")\nbs = for x in xs, carry e: error = e1 {\n    b, _, e2 = visa.query(d, x, err: e)\n    next e = e2\n    collect b\n}";
    assert_eq!(judged("d: visa.session, xs: [string]", "a: f64, bs: [f64]", body).len(), 1, "LC2103: a target is missing");
    let right = "a, _, e1 = visa.query(d, \"a\")\nerr, bs = for x in xs, carry e: error = e1 {\n    b, _, e2 = visa.query(d, x, err: e)\n    next e = e2\n    collect b\n}";
    assert_eq!(
        judged("d: visa.session, xs: [string]", "a: f64, err: error, bs: [f64]", right),
        NOTHING,
        "the carry reads `e1`, and the body waits for the clause"
    );
}

#[test]
fn an_err_output_of_a_primitive_counts() {
    let text = format!("use array\n{USES}\nfn f(xs: [f64]) -> (y: f64) {{\n    y, _ = array.get(xs, 0)\n}}");
    let found: Vec<&str> = diagnostics(&[("f.lcf", text.as_str())]).iter().map(|diagnostic| diagnostic.code().as_str()).collect();
    assert_eq!(found, ["LC4101"]);
}

#[test]
fn an_output_that_holds_a_session_can_be_read_back() {
    let body = "s = d\nx, _, e1 = visa.query(s, \"x\")\nz, _, e2 = visa.query(d, \"z\")\ny = core.add(x, z)\nerr = errors.merge(e1, e2)";
    assert_eq!(judged("d: visa.session", "s: visa.session, y: f64, err: error", body), ["LC4102"], "`s` binds an output, and reading it back is `d`");
}

#[test]
fn the_message_of_two_writes_names_the_element() {
    let text = format!(
        "{USES}\nfn f(a: f64 = 1.0) -> (y: f64) {{\n    x = ui.set(shown, a)\n    z = ui.set(shown, a)\n    y = core.add(x, z)\n}}\n\npanel {{\n    indicator shown: numeric\n}}"
    );
    let found: Vec<String> = diagnostics(&[("f.lcf", text.as_str())]).iter().map(|diagnostic| diagnostic.message.clone()).collect();
    assert_eq!(found, ["the panel element `shown` is written by two nodes with no order between them: order them by a wire or `after:` (8.1)"]);
}

#[test]
fn the_order_of_the_lines_changes_no_verdict() {
    let lines = ["a, _, e1 = visa.query(d, \"a\")", "b, _, e2 = visa.query(d, \"b\")", "err = errors.merge(e1, e2)"];
    for (first, second, third) in [(0, 1, 2), (0, 2, 1), (1, 0, 2), (1, 2, 0), (2, 0, 1), (2, 1, 0)] {
        let body = format!("{}\n{}\n{}", lines[first], lines[second], lines[third]);
        assert_eq!(judged("d: visa.session", "a: f64, b: f64, err: error", &body), ["LC4102"], "{body}");
        assert_eq!(
            messages("d: visa.session", "a: f64, b: f64, err: error", &body),
            ["concurrent access to `d`: order by the error wire or `after:` (8.1)"]
        );
    }
}
