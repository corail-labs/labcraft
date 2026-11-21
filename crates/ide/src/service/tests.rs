//! The language service of the view `Text`, on texts written for each rule: each expected value worked out by hand from
//! the text and the rule (spec 14; ide.md 7; decisions of the owner, 2026-11-21).

use kit_text::lsp_types::{GotoDefinitionResponse, HoverContents};
use kit_text::{Field, LanguageService, position_of};
use labcraft_engine::view::{Listed, ListedType};
use labcraft_types::listing::{Typed, VariantSlots};

use super::*;

const TEXT: &str = r"labcraft 1

/// Scales.
fn scale(x: f64[V], gain: f64 = 2.0) -> (y: f64[V]) {
    k = core.mul(x, gain) // twice
    y = core.add(k, x)
    total = for v in readings, carry acc = 0 V {
        next acc = core.add(acc, v)
    }
}

layout {
    node k at (8, 8)
}
";

fn callable(name: &str, kind: &str, signature: &str) -> Callable {
    let library = name.rsplit_once('.').map(|(library, _)| library.to_owned()).unwrap_or_default();
    Callable { name: name.to_owned(), library, kind: kind.to_owned(), signature: signature.to_owned(), doc: None, typed: Typed::default() }
}

fn callables() -> Vec<Callable> {
    let mut state = callable("State", "type", "type State = enum { Init, Done }");
    state.typed.variants =
        vec![VariantSlots { name: "Init".to_owned(), values: Vec::new() }, VariantSlots { name: "Done".to_owned(), values: Vec::new() }];
    vec![
        callable("core.mul", "function", "fn core.mul(a: T, b: T) -> (product: T)"),
        callable("core.add", "function", "fn core.add(a: T, b: T) -> (sum: T)"),
        callable("visa.open", "function", "fn visa.open(resource: string) -> (session: visa.session, err: error)"),
        callable("double", "function", "fn double(x: f64) -> (y: f64)"),
        state,
    ]
}

/// The service on `text`, asked with the cursor at the end of the first `marker` in it (or at its start with `before`).
struct Asked {
    callables: Vec<Callable>,
    functions: Vec<Listed>,
    types: Vec<ListedType>,
    published: Vec<Published>,
    scanned: Scanned,
    field: Field,
}

impl Asked {
    fn new(text: &str) -> Asked {
        let listed = |function: &str, file: &str| Listed {
            function: function.to_owned(),
            file: file.to_owned(),
            drawable: true,
            errors: 0,
            warnings: 0,
            needs: Vec::new(),
        };
        Asked {
            callables: callables(),
            functions: vec![listed("scale", "scale.lcf"), listed("double", "double.lcf")],
            types: vec![ListedType { name: "State".to_owned(), file: "State.lct".to_owned() }],
            published: Vec::new(),
            scanned: Scanned::default(),
            field: Field::new(text),
        }
    }

    /// Asks the service, with the cursor at a character of the text.
    fn ask<R>(&mut self, at: usize, ask: impl FnOnce(&mut Service<'_>, &Field, kit_text::lsp_types::Position) -> R) -> R {
        let known = Known { callables: &self.callables, functions: &self.functions, types: &self.types, drawing: None };
        let mut service = Service { file: "scale.lcf", known: &known, published: &mut self.published, scanned: &mut self.scanned };
        ask(&mut service, &self.field, position_of(&self.field, at))
    }

    /// The labels completed at a character of the text.
    fn completed(&mut self, at: usize) -> Vec<String> {
        self.ask(at, |service, field, at| service.completion(field, at)).into_iter().map(|item| item.label).collect()
    }

    fn hovered(&mut self, at: usize) -> Option<String> {
        let hover = self.ask(at, |service, field, at| service.hover(field, at))?;
        match hover.contents {
            HoverContents::Markup(markup) => Some(markup.value),
            _ => None,
        }
    }

    fn defined(&mut self, at: usize) -> Option<(String, u32, u32)> {
        match self.ask(at, |service, field, at| service.definition(field, at))? {
            GotoDefinitionResponse::Scalar(location) => Some((path_of(&location.uri)?, location.range.start.line, location.range.start.character)),
            _ => None,
        }
    }
}

/// The character just after the first `marker` of `text` (the text is ASCII: bytes are characters).
fn after(text: &str, marker: &str) -> usize {
    text.find(marker).unwrap() + marker.len()
}

/// The inputs and outputs are seen in the whole file; a line of the body binds in the body; a `for` clause and a carry
/// in the body of the loop (R-3).
#[test]
fn each_binding_is_seen_in_its_braces() {
    let scan = Scan::new(TEXT);
    let names = |at: usize| {
        let mut seen: Vec<String> = scan.seen_at(at).into_iter().map(|binding| binding.name.clone()).collect();
        seen.sort();
        seen.dedup();
        seen
    };
    // In the line `y = core.add(k, x)`.
    assert_eq!(names(after(TEXT, "core.add(")), ["gain", "k", "total", "x", "y"]);
    // In the body of the loop: its clause and its carry too.
    assert_eq!(names(after(TEXT, "next acc = core.add(")), ["acc", "gain", "k", "total", "v", "x", "y"]);
    // In `layout`, the signature only.
    assert_eq!(names(after(TEXT, "node ")), ["gain", "x", "y"]);
}

/// Where each part of the text is.
#[test]
fn each_place_is_known() {
    let scan = Scan::new(TEXT);
    assert_eq!(scan.place_at(after(TEXT, "fn scale(x")), Place::Signature);
    assert_eq!(scan.place_at(after(TEXT, "k = co")), Place::Body);
    assert_eq!(scan.place_at(after(TEXT, "node k")), Place::Layout);
    assert_eq!(scan.place_at(0), Place::Top);
    assert_eq!(scan.place_at(TEXT.len()), Place::Top);
}

/// In a body: the values seen there, the functions of the project and the libraries, the words of a body.
#[test]
fn a_body_completes_values_functions_libraries_and_its_words() {
    let mut asked = Asked::new(TEXT);
    let found = asked.completed(after(TEXT, "y = core.add(k, x)\n    "));
    for label in ["k", "x", "gain", "double", "State", "core", "visa", "match", "loop", "yield"] {
        assert!(found.iter().any(|found| found == label), "{label} in {found:?}");
    }
    // The values of the loop are not seen outside it, and no word of the top is offered.
    assert!(!found.iter().any(|found| found == "v" || found == "acc"), "{found:?}");
    assert!(!found.iter().any(|found| found == "fn" || found == "use"), "{found:?}");
}

/// After `core.`, the functions of `core`, by the rest of their name; after a type, its variants.
#[test]
fn after_a_dot_come_the_members() {
    let text = "labcraft 1\n\nfn f() -> () {\n    a = core.\n    s = State.\n}\n";
    let mut asked = Asked::new(text);
    let mut found = asked.completed(after(text, "core."));
    found.sort();
    assert_eq!(found, ["add", "mul"]);
    let mut found = asked.completed(after(text, "State."));
    found.sort();
    assert_eq!(found, ["Done", "Init"]);
}

/// After `:` in a signature, the types: the base types, the types of the project, the libraries.
#[test]
fn a_signature_completes_types_after_a_colon() {
    let text = "labcraft 1\n\nfn f(x: ) -> () {}\n";
    let mut asked = Asked::new(text);
    let found = asked.completed(after(text, "x: "));
    for label in ["f64", "duration", "waveform", "State", "core"] {
        assert!(found.iter().any(|found| found == label), "{label} in {found:?}");
    }
    assert!(!found.iter().any(|found| found == "double"), "no function where a type goes: {found:?}");
}

/// Outside every declaration, the words that start one; after `use`, the libraries.
#[test]
fn the_top_completes_declarations_and_use_libraries() {
    let text = "labcraft 1\nuse \n\n";
    let mut asked = Asked::new(text);
    let mut found = asked.completed(after(text, "use "));
    found.sort();
    assert_eq!(found, ["core", "visa"]);
    let found = asked.completed(text.len());
    assert_eq!(found, ["use", "fn", "extern", "type", "panel", "layout", "terminals", "outline", "icon"]);
}

/// Nothing is completed in a comment or a string.
#[test]
fn no_completion_in_a_comment_or_a_string() {
    let mut asked = Asked::new(TEXT);
    assert!(asked.completed(after(TEXT, "// twi")).is_empty());
    let text = "labcraft 1\n\nfn f() -> () {\n    s = \"co\"\n}\n";
    let mut asked = Asked::new(text);
    assert!(asked.completed(after(text, "\"co")).is_empty());
}

/// A hover over a function gives its signature; over a value, its name (with no drawing, no type); over a library, it
/// says so; over a word or a number, nothing.
#[test]
fn a_hover_says_what_a_name_is() {
    let mut asked = Asked::new(TEXT);
    assert_eq!(asked.hovered(after(TEXT, "k = core.m")).as_deref(), Some("fn core.mul(a: T, b: T) -> (product: T)"));
    assert_eq!(asked.hovered(after(TEXT, "core.add(") - 2).as_deref(), Some("fn core.add(a: T, b: T) -> (sum: T)"));
    assert_eq!(asked.hovered(after(TEXT, "core.add(")).as_deref(), Some("k"));
    assert_eq!(asked.hovered(after(TEXT, "k = c")).as_deref(), Some("library core"));
    assert_eq!(asked.hovered(after(TEXT, "for ") - 2), None);
    assert_eq!(asked.hovered(after(TEXT, "= 2")), None);
}

/// A value goes to the line that binds it, the innermost first; a function of the project and a type to their file.
#[test]
fn a_definition_goes_to_the_binding_or_the_file() {
    let text = "labcraft 1\n\nfn f(x: f64) -> (y: f64) {\n    k = double(x)\n    y = core.add(k, State.Init)\n}\n";
    let mut asked = Asked::new(text);
    // `k` in the line of `y`: line 3, character 4.
    assert_eq!(asked.defined(after(text, "core.add(")), Some(("scale.lcf".to_owned(), 3, 4)));
    // `x` read by `double`: the input, line 2, character 5.
    assert_eq!(asked.defined(after(text, "double(")), Some(("scale.lcf".to_owned(), 2, 5)));
    assert_eq!(asked.defined(after(text, "k = do")), Some(("double.lcf".to_owned(), 0, 0)));
    assert_eq!(asked.defined(after(text, "Stat")), Some(("State.lct".to_owned(), 0, 0)));
    // A primitive has no file of the project.
    assert_eq!(asked.defined(after(text, "core.ad")), None);
}

/// A carry hides an outer value of its name in the loop (R-3): its definition is the carry there.
#[test]
fn an_inner_binding_hides_an_outer_one() {
    let text = "labcraft 1\n\nfn f(err: error) -> (err: error) {\n    n = loop carry err = err {\n        next err = core.id(err)\n        until true\n    }\n}\n";
    let mut asked = Asked::new(text);
    // `err` read in the loop: the carry, line 3, character 19.
    assert_eq!(asked.defined(after(text, "core.id(")), Some(("scale.lcf".to_owned(), 3, 19)));
}

/// The error of `text.save` is a diagnostic of LSP: lines and columns from 1 become lines and characters from 0.
#[test]
fn the_error_of_a_text_is_a_diagnostic() {
    let error = serde_json::json!({
        "code": "LC1001",
        "message": "expected `)`",
        "span": { "start": { "line": 3, "column": 5 }, "end": { "line": 3, "column": 7 } },
    });
    let diagnostic = diagnostic_of_error(&error).unwrap();
    assert_eq!((diagnostic.range.start.line, diagnostic.range.start.character), (2, 4));
    assert_eq!((diagnostic.range.end.line, diagnostic.range.end.character), (2, 6));
    assert_eq!(diagnostic.code, Some(NumberOrString::String("LC1001".to_owned())));
    assert_eq!(diagnostic.severity, Some(DiagnosticSeverity::ERROR));
}

/// A path of the project, spaces and letters beyond ASCII included, comes back from its URI.
#[test]
fn a_path_goes_through_its_uri() {
    for path in ["main.lcf", "lib/sub dir/é ✓.lcf", "a%b.lcf"] {
        let uri = uri_of(path).unwrap();
        assert_eq!(path_of(&uri).as_deref(), Some(path), "{}", uri.as_str());
    }
}

/// An external function has no body (spec 4.1): a section after it is that section, not a body (code review).
#[test]
fn an_external_function_opens_no_body() {
    let text = "labcraft 1\n\nextern python(\"m\") fn f(x: f64) -> (y: f64, err: error)\n\npanel {\n    \n}\n";
    let scan = Scan::new(text);
    assert_eq!(scan.place_at(after(text, "panel {\n  ")), Place::Panel);
}

/// In `layout`, a node names a value of the whole function: its definition is the line that binds it (code review).
#[test]
fn a_name_in_layout_goes_to_its_binding() {
    let mut asked = Asked::new(TEXT);
    assert_eq!(asked.defined(after(TEXT, "node k")), Some(("scale.lcf".to_owned(), 4, 4)));
    assert_eq!(asked.hovered(after(TEXT, "node k")).as_deref(), Some("k"));
}

/// A file that starts with a byte order mark, which `check` does not read (spec 2), has its values, and the diagnostics
/// of its first line fall one character later (independent review).
#[test]
fn a_byte_order_mark_is_not_read() {
    let text = "\u{FEFF}labcraft 1\n\nfn f(x: f64) -> () {\n    k = x\n}\n";
    let mut asked = Asked::new(text);
    let start = text.chars().count() - "}\n".len();
    assert!(asked.completed(start).iter().any(|label| label == "k"), "the body is read");
    // `x` read by `k`: the input, line 2, character 5.
    let at = text.chars().count() - "x\n}\n".len();
    assert_eq!(asked.defined(at), Some(("scale.lcf".to_owned(), 2, 5)));
    let error =
        serde_json::json!({ "code": "LC0101", "message": "m", "span": { "start": { "line": 1, "column": 1 }, "end": { "line": 1, "column": 3 } } });
    let shifted = after_bom(text, vec![diagnostic_of_error(&error).unwrap()]);
    assert_eq!((shifted[0].range.start.character, shifted[0].range.end.character), (1, 3));
}

/// A path longer than any name of the language is read up to a bound: a hover on it answers at once (independent
/// review: a path of 160,000 segments took seconds).
#[test]
fn a_very_long_path_is_bounded() {
    let path = vec!["a"; 100_000].join(".");
    let text = format!("labcraft 1\n\nfn f(a: f64) -> () {{\n    y = core.id({path})\n}}\n");
    let mut asked = Asked::new(&text);
    let at = text.find(")\n}").unwrap() - 1;
    assert_eq!(asked.hovered(at), None);
}

/// Completion lists each value once, however many lines bind it.
#[test]
fn each_value_is_listed_once() {
    let text = "labcraft 1\n\nfn f() -> () {\n    v = 1\n    v = 2\n    w = 3\n    \n}\n";
    let mut asked = Asked::new(text);
    let found = asked.completed(after(text, "w = 3\n    "));
    assert_eq!(found.iter().filter(|label| *label == "v").count(), 1, "{found:?}");
}
