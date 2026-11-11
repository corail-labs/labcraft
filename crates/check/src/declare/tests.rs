use std::borrow::Cow;

use labcraft_types::parse_unit;

use crate::{Checked, Content, Found, LibrarySource, SourceFile, check};

use super::*;

struct Nothing;

impl Libraries for Nothing {
    fn find(&self, _: &str, _: Origin) -> Found {
        Found::NotFound
    }
}

fn source(name: &str, files: &[(&str, &str)]) -> LibrarySource {
    let files = files
        .iter()
        .map(|(shown, text)| {
            let text = format!("labcraft 1\n\n{text}\n");
            SourceFile { shown: (*shown).to_owned(), content: Content::Bytes(Cow::Owned(text.into_bytes())) }
        })
        .collect();
    LibrarySource { files, ..LibrarySource::new(name.to_owned(), name.to_owned(), Origin::Project) }
}

fn run(root: LibrarySource) -> (Checked, Interner) {
    let mut names = Interner::new();
    let checked = check(root, &Nothing, &mut names, &Limits::default());
    (checked, names)
}

fn codes_of(root: LibrarySource) -> Vec<&'static str> {
    let mut report = run(root).0.report;
    report.finish();
    report
        .diagnostics()
        .iter()
        .filter(|diagnostic| !crate::declare::body::later_pass(diagnostic.code()))
        .map(|diagnostic| diagnostic.code().as_str())
        .collect()
}

/// A folder of its own for each case, with one sub-library `lib`, which declares a type and an opaque type.
fn codes(files: &[(&str, &str)]) -> Vec<&'static str> {
    let mut root = source("", files);
    root.sub_libraries.push(source("lib", &[("lib/T.lct", "type T = f64"), ("lib/session.lct", "type session = opaque")]));
    codes_of(root)
}

fn function(signature: &str) -> Vec<&'static str> {
    codes(&[("f.lcf", &format!("fn f{signature} {{}}"))])
}

fn type_of(expression: &str) -> Vec<&'static str> {
    codes(&[("T.lct", &format!("type T = {expression}"))])
}

const NOTHING: [&str; 0] = [];

/// The diagnostics of one code, in the order of the report.
fn only(report: &Report, code: Code) -> Vec<Diagnostic> {
    report.diagnostics().iter().filter(|diagnostic| diagnostic.code() == code).cloned().collect()
}

#[test]
fn a_name_defined_twice_is_reported_once_on_the_last_file() {
    let root = source("", &[("a.lcf", "fn f() -> () {}"), ("b.lcf", "fn f() -> () {}"), ("c.lct", "type f = f64")]);
    let mut report = run(root).0.report;
    report.finish();
    let [diagnostic] = only(&report, Code::Lc2013).try_into().unwrap_or_else(|all| panic!("{all:?}"));
    assert_eq!(diagnostic.file, "c.lct");
    let related: Vec<&str> = diagnostic.related.iter().map(|related| related.file.as_str()).collect();
    assert_eq!(related, ["a.lcf", "b.lcf"]);
    assert_eq!(only(&report, Code::Lc2023).len(), 3, "no file bears the name of what it declares");
    // A function and a type of one name, each in its own file, and a copy of the function under another name.
    let root = source("", &[("f.lcf", "fn f() -> () {}"), ("a.lcf", "fn f() -> () {}"), ("f.lct", "type f = f64")]);
    let mut report = run(root).0.report;
    report.finish();
    let [twice] = only(&report, Code::Lc2013).try_into().unwrap_or_else(|all| panic!("{all:?}"));
    assert_eq!((twice.file.as_str(), twice.related.len()), ("f.lct", 1), "the copy is not one of them");
    let [copy] = only(&report, Code::Lc2023).try_into().unwrap_or_else(|all| panic!("{all:?}"));
    assert_eq!(copy.file, "a.lcf");
}

#[test]
fn a_type_cannot_bear_a_name_of_the_language() {
    for name in LANGUAGE_TYPES {
        let shown = format!("{name}.lct");
        assert_eq!(codes(&[(&shown, &format!("type {name} = i32"))]), ["LC2022"], "{name}");
    }
    assert_eq!(codes(&[("Number.lct", "type Number = i32")]), NOTHING);
}

/// The base types of spec 3.1, then the others that the language reads as types.
const LANGUAGE_TYPES: [&str; 26] = [
    "bool",
    "i8",
    "i16",
    "i32",
    "i64",
    "u8",
    "u16",
    "u32",
    "u64",
    "f32",
    "f64",
    "c64",
    "c128",
    "string",
    "bytes",
    "timestamp",
    "duration",
    "error",
    "waveform",
    "queue",
    "ref",
    "event",
    "notifier",
    "map",
    "set",
    "opaque",
];

#[test]
fn a_refused_type_says_nothing_more_wherever_it_is_named() {
    for name in LANGUAGE_TYPES {
        let files = [(format!("{name}.lct"), format!("type {name} = i32")), ("f.lcf".to_owned(), format!("fn f(a: {name}) -> () {{}}"))];
        let files: Vec<(&str, &str)> = files.iter().map(|(shown, text)| (shown.as_str(), text.as_str())).collect();
        assert_eq!(codes(&files), ["LC2022"], "{name}");
    }
    assert_eq!(codes(&[("string.lct", "type string = i32"), ("f.lcf", "fn f(string: string) -> () {}")]), ["LC2022"], "as a value too");
    let mut root = source("", &[("f.lcf", "use dep\n\nfn f(a: dep.string) -> () {}")]);
    root.sub_libraries.push(source("dep", &[("dep/string.lct", "type string = i32")]));
    assert_eq!(codes_of(root), ["LC2022"], "through a library");
}

#[test]
fn a_type_with_an_error_inside_is_the_error_type_by_its_name() {
    let error_of = |files: &[(&str, &str)], name: &str| {
        let (checked, mut names) = run(source("", files));
        let library = checked.libraries.first().unwrap();
        let declared = library.types.get(&names.intern(name)).unwrap();
        (declared.ty == TyId::ERROR, checked.types.has_error(library.functions.get(&names.intern("f")).unwrap().params.first().unwrap().ty))
    };
    let param = ("f.lcf", "fn f(a: U) -> () {}");
    assert_eq!(error_of(&[("U.lct", "type U = Nowhere"), param], "U"), (true, true));
    assert_eq!(error_of(&[("U.lct", "type U = [Nowhere]"), param], "U"), (true, true));
    assert_eq!(error_of(&[("U.lct", "type U = struct { a: V }"), ("V.lct", "type V = Nowhere"), param], "U"), (true, true), "through another type");
    assert_eq!(error_of(&[("U.lct", "type U = V"), ("V.lct", "type V = U"), param], "U"), (true, true), "a cycle");
    assert_eq!(error_of(&[("U.lct", "type U = [V]"), ("V.lct", "type V = f64"), param], "U"), (false, false), "and nothing else");
    for expression in ["struct { a: f64, a: i32 }", "enum { A, A }", "fn(a: f64, a: i32) -> ()", "fn(after: f64) -> ()", "[f64; 33]", "ref<f64>"] {
        let text = format!("type U = {expression}");
        assert_eq!(error_of(&[("u.lct", &text), param], "U"), (true, true), "{expression}");
    }
    assert_eq!(error_of(&[("U.lct", "type U<T> = [T]"), param], "U"), (true, true), "a generic type");
}

#[test]
fn a_variable_of_a_generic_type_is_no_mention() {
    assert_eq!(codes(&[("A.lct", "type A<B> = [B]"), ("B.lct", "type B = A<f64>")]), ["LC4006"]);
    assert_eq!(codes(&[("A.lct", "type A<A> = [A]")]), ["LC4006"]);
}

#[test]
fn a_path_from_a_refused_name_says_nothing_more() {
    assert_eq!(codes(&[("bool.lct", "type bool = f64"), ("U.lct", "type U = bool.Pass")]), ["LC2022"]);
}

#[test]
fn a_library_with_a_broken_file_says_so() {
    let (checked, _) = run(source("", &[("bad.lcf", "fn ("), ("f.lcf", "fn f() -> () {}")]));
    assert!(checked.libraries.first().unwrap().broken);
    let (checked, _) = run(source("", &[("f.lcf", "fn f() -> () {}")]));
    assert!(!checked.libraries.first().unwrap().broken);
}

#[test]
fn an_opaque_type_stands_for_itself_an_alias_is_what_it_names_and_a_struct_is_new() {
    let (checked, mut names) = run(source(
        "",
        &[
            ("s.lct", "type s = opaque"),
            ("p.lct", "type p = f64"),
            ("q.lct", "type q = struct { a: f64 }"),
            ("r.lct", "type r = q"),
            ("o.lct", "type o = s"),
        ],
    ));
    let library = checked.libraries.first().unwrap();
    let opaque = library.types.get(&names.intern("s")).unwrap();
    assert_eq!(opaque.definition, opaque.ty);
    assert!(matches!(checked.types.get(opaque.ty), Ty::Named { .. }));
    let alias = library.types.get(&names.intern("p")).unwrap();
    assert!(matches!(checked.types.get(alias.ty), Ty::Number { kind: NumberKind::F64, .. }));
    assert_eq!(alias.definition, alias.ty);
    let declared = library.types.get(&names.intern("q")).unwrap();
    assert!(matches!(checked.types.get(declared.ty), Ty::Named { .. }));
    assert!(matches!(checked.types.get(declared.definition), Ty::Struct(_)));
    assert_eq!(library.types.get(&names.intern("r")).unwrap().ty, declared.ty, "an alias of a struct is that struct");
    assert_eq!(library.types.get(&names.intern("o")).unwrap().ty, opaque.ty);
}

#[test]
fn the_first_declaration_of_a_name_in_the_order_of_the_paths_stays() {
    let (checked, mut names) = run(source(
        "",
        &[("b.lct", "type T = bool"), ("a.lct", "type T = f64"), ("d.lcf", "fn f() -> (x: bool) {}"), ("c.lcf", "fn f() -> (x: f64) {}")],
    ));
    let library = checked.libraries.first().unwrap();
    let definition = library.types.get(&names.intern("T")).unwrap().definition;
    assert!(matches!(checked.types.get(definition), Ty::Number { .. }));
    let output = library.functions.get(&names.intern("f")).unwrap().outputs.first().unwrap().ty;
    assert!(matches!(checked.types.get(output), Ty::Number { .. }));
}

#[test]
fn a_mistake_in_a_type_that_no_program_can_name_is_still_reported() {
    assert_eq!(codes(&[("string.lct", "type string = Nowhere")]), ["LC2022", "LC2003"]);
    assert_eq!(codes(&[("a.lct", "type T = f64"), ("b.lct", "type T = Nowhere")]), ["LC2023", "LC2013", "LC2023", "LC2003"]);
    assert_eq!(codes(&[("T.lct", "type T = f64"), ("b.lct", "type T = Nowhere")]), ["LC2023", "LC2003"], "a copy is no second definition");
}

#[test]
fn a_cycle_of_types_across_libraries_is_the_cycle_of_the_libraries() {
    let mut root = source("", &[("f.lcf", "fn f() -> () {}")]);
    root.sub_libraries.push(source("a", &[("a/X.lct", "use b\n\ntype X = b.Y")]));
    root.sub_libraries.push(source("b", &[("b/Y.lct", "use a\n\ntype Y = a.X")]));
    assert_eq!(codes_of(root), ["LC2021"]);
    let mut root = source("", &[("f.lcf", "fn f() -> () {}")]);
    root.sub_libraries.push(source("a", &[("a/X.lct", "use b\n\ntype X = X")]));
    root.sub_libraries.push(source("b", &[("b/Y.lct", "type Y = f64")]));
    assert_eq!(codes_of(root), ["LC2017"], "a cycle in a library that is in no cycle of libraries");
    let mut root = source("", &[("f.lcf", "fn f() -> () {}")]);
    root.sub_libraries.push(source("a", &[("a/T.lct", "type T = [T]"), ("a/U.lct", "use b\n\ntype U = f64")]));
    root.sub_libraries.push(source("b", &[("b/Y.lct", "use a\n\ntype Y = f64")]));
    let mut found = codes_of(root);
    found.sort_unstable();
    assert_eq!(found, ["LC2017", "LC2021"], "two mistakes: this cycle of types does not pass through a `use`");
}

#[test]
fn an_error_on_an_alias_of_an_opaque_type_says_nothing_more() {
    for alias in ["s[V]", "s<i32>"] {
        let text = format!("type a = {alias}");
        let code = if alias.contains('[') { "LC3021" } else { "LC3019" };
        assert_eq!(codes(&[("s.lct", "type s = opaque"), ("a.lct", &text), ("f.lcf", "fn f(a: a) -> () {}")]), [code], "{alias}");
    }
}

#[test]
fn what_is_not_a_library_is_not_called_one() {
    assert_eq!(codes(&[("V.lct", "type V = enum { Pass }"), ("f.lcf", "fn f(a: V.Pass) -> () {}")]), ["LC2003"]);
}

#[test]
fn a_function_at_fault_is_named_on_its_own_name_too() {
    let mut report = run(source("", &[("a.lcf", "use core\n\nfn core() -> () {}"), ("b.lcf", "use core\n\nfn core() -> () {}")])).0.report;
    report.finish();
    for diagnostic in report.diagnostics() {
        assert_eq!(diagnostic.function.as_deref(), Some("core"), "{}", diagnostic.code().as_str());
    }
}

#[test]
fn places_of_what_is_reported() {
    let line_of = |diagnostic: &Diagnostic| diagnostic.span.map(|span| span.start.line);
    let run_one = |text: &str| {
        let report = run(source("", &[("f.lcf", text)])).0.report;
        report.diagnostics().first().unwrap().clone()
    };
    let twice = run_one("fn f(\n    a: i32,\n    a: i32,\n) -> () {}");
    assert_eq!((twice.code(), line_of(&twice)), (Code::Lc2019, Some(5)));
    assert_eq!(twice.related.first().and_then(|related| related.span).map(|span| span.start.line), Some(4));
    let imported = run_one("use core\n\nfn f(core: i32) -> () {}");
    assert_eq!((imported.code(), line_of(&imported)), (Code::Lc2014, Some(5)));
    assert_eq!(imported.related.first().and_then(|related| related.span).map(|span| span.start.line), Some(3));
}

#[test]
fn a_cycle_names_the_place_of_each_of_its_members() {
    let mut report = run(source("", &[("A.lct", "type A = B"), ("B.lct", "\ntype B = A")])).0.report;
    report.finish();
    let [diagnostic] = report.diagnostics() else { panic!("{:?}", report.diagnostics()) };
    assert_eq!(diagnostic.related.first().and_then(|related| related.span).map(|span| span.start.line), Some(4));
}

#[test]
fn a_library_without_use_is_one_mistake_in_a_file() {
    let mut root = source("", &[("f.lcf", "fn f(a: lib.T, b: lib.T, c: other.T) -> () {}"), ("g.lcf", "fn g(a: lib.T) -> () {}")]);
    root.sub_libraries.push(source("lib", &[("lib/T.lct", "type T = f64")]));
    assert_eq!(codes_of(root), ["LC2005", "LC2005", "LC2005"], "`lib` and `other` in one file, `lib` in another");
}

#[test]
fn a_function_or_a_type_cannot_bear_the_name_of_an_imported_library() {
    assert_eq!(codes(&[("core.lcf", "use core\n\nfn core() -> () {}")]), ["LC2014"]);
    assert_eq!(codes(&[("core.lct", "use core\n\ntype core = i32")]), ["LC2014"]);
    assert_eq!(codes(&[("lib.lcf", "use lib\n\nfn lib() -> () {}")]), ["LC2014"]);
}

#[test]
fn the_name_of_a_library_is_refused_across_the_files_of_a_library() {
    let root = source("", &[("run.lcf", "use core\n\nfn run() -> () {}"), ("core.lct", "type core = enum { Pass, Fail }")]);
    let mut report = run(root).0.report;
    report.finish();
    let [diagnostic] = report.diagnostics() else { panic!("{:?}", report.diagnostics()) };
    assert_eq!(diagnostic.code(), Code::Lc2014);
    assert_eq!(diagnostic.file, "core.lct", "it is on the declaration");
    assert_eq!(diagnostic.related.first().map(|related| related.file.as_str()), Some("run.lcf"), "with a link to the `use`");
}

#[test]
fn a_name_of_a_library_that_nothing_imports_is_free() {
    assert_eq!(codes(&[("core.lcf", "fn core(string: i32) -> (visa: i32) {}"), ("time.lct", "type time = i32")]), NOTHING);
}

#[test]
fn a_value_cannot_bear_the_name_of_a_library_of_its_file() {
    assert_eq!(codes(&[("f.lcf", "use core\n\nfn f(core: i32) -> () {}")]), ["LC2014"]);
    assert_eq!(codes(&[("f.lcf", "use core\n\nfn f() -> (core: i32) {}")]), ["LC2014"]);
    assert_eq!(codes(&[("f.lcf", "use core\n\nfn f(core: i32) -> (core: i32) {}")]), ["LC2014"], "one name, one mistake");
    assert_eq!(codes(&[("f.lcf", "use core\n\nfn f() -> () {}"), ("g.lcf", "fn g(core: i32) -> () {}")]), NOTHING, "another file");
}

#[test]
fn the_first_segment_of_a_library_is_what_a_value_cannot_bear() {
    let library = |text: &str| {
        let mut root = source("", &[("f.lcf", text)]);
        let mut a = source("a", &[("a/x.lcf", "fn x() -> () {}")]);
        a.sub_libraries.push(source("b", &[("a/b/y.lcf", "fn y() -> () {}")]));
        root.sub_libraries.push(a);
        codes_of(root)
    };
    assert_eq!(library("use a.b\n\nfn f(a: i32) -> () {}"), ["LC2014"]);
    assert_eq!(library("use a.b\n\nfn f(b: i32) -> () {}"), NOTHING);
}

#[test]
fn a_value_cannot_bear_the_name_of_a_plain_type_of_its_library() {
    assert_eq!(codes(&[("Verdict.lct", "type Verdict = enum { Pass }"), ("f.lcf", "fn f(Verdict: Verdict) -> () {}")]), ["LC2014"]);
    assert_eq!(codes(&[("V.lct", "type V = f64"), ("f.lcf", "fn f() -> (V: V) {}")]), ["LC2014"]);
}

#[test]
fn a_value_can_bear_a_base_type_or_an_opaque_type() {
    assert_eq!(function("(duration: duration, error: error, i32: i32) -> (timestamp: timestamp)"), NOTHING);
    assert_eq!(codes(&[("session.lct", "type session = opaque"), ("f.lcf", "fn f(session: session) -> () {}")]), NOTHING);
    assert_eq!(codes(&[("a.lct", "type a = b"), ("b.lct", "type b = opaque"), ("f.lcf", "fn f(a: a, b: b) -> () {}")]), NOTHING);
    assert_eq!(codes(&[("s.lct", "use lib\n\ntype s = lib.session"), ("f.lcf", "fn f(s: s) -> () {}")]), NOTHING);
}

#[test]
fn an_alias_of_what_is_unknown_says_nothing_more() {
    assert_eq!(codes(&[("s.lct", "type s = nowhere"), ("f.lcf", "fn f(s: s) -> () {}")]), ["LC2003"]);
}

/// The names that `LC2022` refuses beyond the base types, as `docs/cli.md` lists them.
#[test]
fn the_names_refused_to_types_are_those_of_the_documents() {
    const CLI: &str = include_str!("../../../../docs/cli.md");
    let row = CLI.lines().find(|line| line.starts_with("| `LC2022` |")).unwrap_or_default();
    let listed: Vec<&str> = row.split('`').skip(3).step_by(2).collect();
    for name in &listed {
        assert!(language_type(name).is_some(), "{name}");
        assert!(LANGUAGE_TYPES.contains(name), "{name}");
    }
    for name in LANGUAGE_TYPES {
        let base = matches!(language_type(name), Some(Lookup::Base(_)));
        assert!(base || listed.contains(&name), "{name} is not in the row of LC2022");
    }
}

#[test]
fn a_value_or_a_function_named_map_or_set_is_a_name() {
    let set = "fn set(map: [f64], value: f64) -> (y: [f64], err: error) {\n    y, err = array.set(map, 0, value)\n}";
    let files = [
        ("set.lcf", format!("use array\n\n{set}")),
        ("map.lcf", "fn map(a: [f64]) -> (y: [f64], err: error) {\n    y, err = set(a, 1.0)\n}".to_owned()),
    ];
    let files: Vec<(&str, &str)> = files.iter().map(|(shown, text)| (*shown, text.as_str())).collect();
    assert_eq!(codes(&files), NOTHING);
}

#[test]
fn a_type_kept_for_a_later_milestone() {
    for name in ["queue", "ref", "event", "notifier", "map", "set"] {
        assert_eq!(function(&format!("(a: {name}) -> ()")), ["LC4006"], "{name}");
    }
}

#[test]
fn a_type_that_does_not_exist() {
    assert_eq!(function("(a: Nowhere) -> ()"), ["LC2003"]);
    assert_eq!(codes(&[("g.lcf", "fn g() -> () {}"), ("f.lcf", "fn f(a: g) -> () {}")]), ["LC2003"], "a function is no type");
    assert_eq!(codes(&[("f.lcf", "use lib\n\nfn f(a: lib.Nowhere) -> () {}")]), ["LC2003"]);
    assert_eq!(codes(&[("f.lcf", "use core\n\nfn f(a: core.Nowhere) -> () {}")]), ["LC2003"], "the primitives declare no type");
    assert_eq!(function("(a: opaque) -> ()"), ["LC2003"], "only the whole of a `type` declaration");
    assert_eq!(type_of("[opaque]"), ["LC2003"]);
    assert_eq!(type_of("opaque[V]"), ["LC2003"]);
}

#[test]
fn a_type_that_may_be_in_a_broken_file_says_nothing() {
    // A file declares what its name says (R-22): only `Nowhere.lct` may declare `Nowhere`.
    assert_eq!(codes(&[("Nowhere.lct", "type Nowhere ="), ("f.lcf", "fn f(a: Nowhere) -> () {}")]), ["LC1002"]);
    assert_eq!(codes(&[("bad.lcf", "fn ("), ("f.lcf", "fn f(a: Nowhere) -> () {}")]), ["LC1001", "LC2003"], "`bad.lcf` declares `bad`");
    let mut root = source("", &[("f.lcf", "use lib\n\nfn f(a: lib.Nowhere) -> () {}")]);
    root.sub_libraries.push(source("lib", &[("lib/Nowhere.lct", "type Nowhere =")]));
    assert_eq!(codes_of(root), ["LC1002"]);
    let mut root = source("", &[("f.lcf", "use lib\n\nfn f(a: lib.Nowhere) -> () {}")]);
    root.sub_libraries.push(source("lib", &[("lib/T.lct", "type T =")]));
    assert_eq!(codes_of(root), ["LC2003", "LC1002"], "`T.lct` declares `T`; in the order of the libraries");
}

#[test]
fn a_type_of_a_library_that_does_not_exist_says_nothing_more() {
    assert_eq!(codes(&[("f.lcf", "use nowhere\n\nfn f(a: nowhere.T) -> () {}")]), ["LC2004"]);
}

fn the_fix(text: &str) -> Fix {
    let mut root = source("", &[("f.lcf", text)]);
    root.sub_libraries.push(source("lib", &[("lib/T.lct", "type T = f64")]));
    let report = run(root).0.report;
    let [diagnostic] = report.diagnostics() else { panic!("{:?}", report.diagnostics()) };
    assert_eq!(diagnostic.code(), Code::Lc2005);
    diagnostic.fixes.first().unwrap().clone()
}

#[test]
fn a_library_without_use_gets_a_fix_in_its_sorted_place() {
    let edit = |fix: Fix| {
        let edit = fix.edits.into_iter().next().unwrap();
        (edit.span.start.line, edit.text)
    };
    assert_eq!(edit(the_fix("fn f(a: lib.T) -> () {}")), (2, "\nuse lib\n".to_owned()));
    assert_eq!(edit(the_fix("use core\n\nfn f(a: lib.T) -> () {}")), (4, "use lib\n".to_owned()));
    assert_eq!(edit(the_fix("use core\nuse time\n\nfn f(a: lib.T) -> () {}")), (4, "use lib\n".to_owned()));
}

#[test]
fn units() {
    assert_eq!(function("(a: f64[V], b: f64[kg/(m*s)], c: i32[m/s^2], d: f64[1/s], e: f64[\u{00B0}C]) -> ()"), NOTHING);
    assert_eq!(function("(a: f64[nowhere]) -> ()"), ["LC2008"]);
    assert_eq!(function("(a: f64[m^128]) -> ()"), ["LC1006"]);
    assert_eq!(function("(a: f64[m^99999999999999999999]) -> ()"), ["LC1006"]);
    assert_eq!(function("(a: f64[m^-128]) -> ()"), ["LC1006"]);
}

#[test]
fn a_unit_is_the_one_that_parse_unit_reads() {
    let root = source("", &[("A.lct", "type A = f64[kg*m/s^2/(A*mV)]")]);
    let (checked, mut names) = run(root);
    let library = checked.libraries.first().unwrap();
    let declared = library.types.get(&names.intern("A")).unwrap();
    let expected = parse_unit("kg*m/s^2/(A*mV)", 256).unwrap();
    assert_eq!(checked.types.get(declared.definition), &Ty::Number { kind: NumberKind::F64, unit: expected });
}

#[test]
fn a_unit_on_what_is_not_a_number() {
    for ty in ["string[V]", "bool[V]", "duration[s]", "error[V]", "waveform<f64>[V]"] {
        assert_eq!(function(&format!("(a: {ty}) -> ()")), ["LC3021"], "{ty}");
    }
    assert_eq!(codes(&[("W.lct", "type W = f64"), ("f.lcf", "fn f(a: W[V]) -> () {}")]), ["LC3021"]);
    assert_eq!(function("(a: string[nowhere]) -> ()"), ["LC3021"], "one mistake");
}

#[test]
fn type_arguments() {
    assert_eq!(function("(a: waveform<f64[V]>) -> ()"), NOTHING);
    assert_eq!(function("(a: waveform) -> ()"), ["LC3019"]);
    assert_eq!(function("(a: waveform<f64, f64>) -> ()"), ["LC3019"]);
    assert_eq!(function("(a: f64<i32>) -> ()"), ["LC3019"]);
    assert_eq!(codes(&[("W.lct", "type W = f64"), ("f.lcf", "fn f(a: W<i32>) -> () {}")]), ["LC3019"]);
    assert_eq!(function("(a: waveform<Nowhere>) -> ()"), ["LC2003"]);
}

#[test]
fn dimensions_of_an_array() {
    assert_eq!(function("(a: [f64], b: [f64; 1], c: [f64; 32], d: [f64; 0_2]) -> ()"), NOTHING);
    for dimensions in ["0", "33", "4294967296", "99999999999999999999999"] {
        assert_eq!(function(&format!("(a: [f64; {dimensions}]) -> ()")), ["LC1006"], "{dimensions}");
    }
}

#[test]
fn a_type_that_contains_itself() {
    for expression in ["T", "[T]", "struct { a: f64, b: T }", "enum { A, B(T) }", "fn(T) -> ()", "fn() -> (T)", "waveform<T>"] {
        assert_eq!(type_of(expression), ["LC2017"], "{expression}");
    }
}

#[test]
fn a_cycle_is_one_error_on_its_first_file() {
    let root = source("", &[("A.lct", "type A = B"), ("B.lct", "type B = C"), ("C.lct", "type C = [A]"), ("D.lct", "type D = A")]);
    let mut report = run(root).0.report;
    report.finish();
    let [diagnostic] = report.diagnostics() else { panic!("{:?}", report.diagnostics()) };
    assert_eq!(diagnostic.code(), Code::Lc2017);
    assert_eq!(diagnostic.file, "A.lct");
    assert_eq!(diagnostic.related.len(), 2);
}

fn with_chain(length: usize, last: &str) -> Vec<&'static str> {
    let mut files = vec![("A0.lct".to_owned(), "type A0 = f64".to_owned())];
    files.extend((1..length).map(|n| (format!("A{n}.lct"), format!("type A{n} = [A{}]", n - 1))));
    let mut files: Vec<(&str, &str)> = files.iter().map(|(shown, text)| (shown.as_str(), text.as_str())).collect();
    files.push(("f.lcf", last));
    codes_of(source("", &files))
}

#[test]
fn nesting_counts_through_the_lines_of_a_chain() {
    assert_eq!(with_chain(257, "fn f(a: A256) -> () {}"), NOTHING, "256 levels");
    assert_eq!(with_chain(258, "fn f(a: A256) -> () {}"), ["LC1006"], "the 257th, and only it");
    assert_eq!(with_chain(300, "fn f(a: A299) -> () {}"), ["LC1006"], "what names it says nothing more");
    assert_eq!(with_chain(257, "fn f(a: [A256]) -> () {}"), ["LC1006"], "a signature adds to it");
}

/// The depth of a type that wraps `B`, where `B` is one level short of the limit `max`.
fn at_the_limit(wrapper: &str, max: u32) -> Vec<&'static str> {
    let mut names = Interner::new();
    let limits = Limits::default().with_max_nesting(max).unwrap();
    let root = source("", &[("A.lct", wrapper), ("B.lct", "type B = [f64]")]);
    let mut report = check(root, &Nothing, &mut names, &limits).report;
    report.finish();
    report
        .diagnostics()
        .iter()
        .filter(|diagnostic| !crate::declare::body::later_pass(diagnostic.code()))
        .map(|diagnostic| diagnostic.code().as_str())
        .collect()
}

#[test]
fn every_kind_of_type_counts_for_one_level_and_the_limit_is_the_option() {
    // `B` is an array here, which a `waveform` does not hold (3.2): the option still counts it first.
    for wrapper in ["[B]", "struct { a: B }", "enum { A(B) }", "fn(B) -> ()", "fn() -> (B)"] {
        let text = format!("type A = {wrapper}");
        assert_eq!(at_the_limit(&text, 2), NOTHING, "{wrapper}: two levels");
    }
    let text = |wrapper: &str| format!("type A = {wrapper}");
    // What the parser counts by itself is `LC0007`, so each case goes over only through `B`: one level in the file.
    for wrapper in ["[B]", "struct { a: B }", "enum { A(B) }", "fn(B) -> ()"] {
        assert_eq!(at_the_limit(&text(wrapper), 1), ["LC1006"], "{wrapper}: over the option");
    }
    assert_eq!(at_the_limit(&text("waveform<B>"), 1), ["LC1006", "LC3019"], "over the option, and an array in a waveform");
}

#[test]
fn a_signature_over_the_limit_has_the_error_type() {
    let files: Vec<(String, String)> =
        (0..257).map(|n| (format!("a{n}.lct"), if n == 0 { "type A0 = f64".to_owned() } else { format!("type A{n} = [A{}]", n - 1) })).collect();
    let mut files: Vec<(&str, &str)> = files.iter().map(|(shown, text)| (shown.as_str(), text.as_str())).collect();
    files.push(("f.lcf", "fn f(a: [A256]) -> () {}"));
    let (checked, mut names) = run(source("", &files));
    let param = checked.libraries.first().unwrap().functions.get(&names.intern("f")).unwrap().params.first().unwrap().ty;
    assert_eq!(param, TyId::ERROR);
}

struct ShippedDeep;

impl Libraries for ShippedDeep {
    fn find(&self, name: &str, _: Origin) -> Found {
        if name != "deep" {
            return Found::NotFound;
        }
        let mut library = source("deep", &[("deep/A.lct", "type A = [[f64]]"), ("deep/B.lct", "type B = [A]")]);
        library.origin = Origin::Shipped;
        Found::Library(library)
    }
}

#[test]
fn a_small_limit_does_not_apply_to_what_is_shipped() {
    let limits = Limits::default().with_max_nesting(2).unwrap();
    let root = source("", &[("f.lcf", "use deep\n\nfn f() -> () {}")]);
    let checked = check(root, &ShippedDeep, &mut Interner::new(), &limits);
    assert_eq!(checked.report.errors(), 0);
    assert!(checked.dependencies.is_empty(), "{:?}", checked.dependencies);
}

#[test]
fn the_chain_of_a_dependency_is_its_own_cycle() {
    let mut names = Interner::new();
    let root = source("", &[("f.lcf", "use dep\n\nfn f() -> () {}")]);
    let checked = check(root, &OneDependency("type T = T", false), &mut names, &Limits::default());
    assert_eq!(checked.report.errors(), 0);
    let [dependency] = checked.dependencies.as_slice() else { panic!("{:?}", checked.dependencies) };
    assert_eq!(dependency.report.diagnostics().first().map(Diagnostic::code), Some(Code::Lc2017));
}

#[test]
fn a_long_chain_of_aliases_takes_no_stack() {
    let mut files = vec![("T0.lct".to_owned(), "type T0 = opaque".to_owned())];
    files.extend((1..20_000).map(|n| (format!("T{n}.lct"), format!("type T{n} = T{}", n - 1))));
    let files: Vec<(&str, &str)> = files.iter().map(|(shown, text)| (shown.as_str(), text.as_str())).collect();
    let (checked, mut names) = run(source("", &files));
    assert_eq!(checked.report.errors(), 0);
    let library = checked.libraries.first().unwrap();
    let (first, last) = (names.intern("T0"), names.intern("T19999"));
    assert_eq!(library.types.get(&first).unwrap().ty, library.types.get(&last).unwrap().ty, "an alias of an opaque type is that type");
    assert!(library.types.get(&last).unwrap().opaque);
}

#[test]
fn an_alias_of_an_opaque_type_of_another_library_is_that_type() {
    let mut root = source("", &[("mine.lct", "use lib\n\ntype mine = lib.session"), ("plain.lct", "type plain = f64")]);
    root.sub_libraries.push(source("lib", &[("lib/session.lct", "type session = opaque")]));
    let (checked, mut names) = run(root);
    assert_eq!(checked.report.errors(), 0);
    let find = |library: &str| checked.libraries.iter().find(|declared| declared.name == library).unwrap();
    let (mine, session, plain) = (names.intern("mine"), names.intern("session"), names.intern("plain"));
    assert_eq!(find("").types.get(&mine).unwrap().ty, find("lib").types.get(&session).unwrap().ty);
    assert!(!find("").types.get(&plain).unwrap().opaque);
}

#[test]
fn two_of_one_name() {
    assert_eq!(function("(a: i32, a: i32) -> ()"), ["LC2019"]);
    assert_eq!(function("() -> (a: i32, a: i32)"), ["LC2019"]);
    assert_eq!(function("(err: error) -> (err: error)"), NOTHING, "an input and an output (R-2)");
    assert_eq!(type_of("struct { a: i32, a: i32 }"), ["LC2019"]);
    assert_eq!(type_of("enum { A, A }"), ["LC2019"]);
    assert_eq!(type_of("fn(a: i32, a: i32) -> ()"), ["LC2019"]);
    assert_eq!(type_of("fn(i32, i32) -> ()"), NOTHING, "arguments without a name");
}

#[test]
fn no_parameter_is_named_after() {
    assert_eq!(function("(after: i32) -> ()"), ["LC2105"]);
    assert_eq!(function("(a: fn(after: i32) -> ()) -> ()"), ["LC2105"]);
    assert_eq!(function("() -> (after: i32)"), NOTHING);
}

#[test]
fn the_place_of_a_variadic_parameter() {
    assert_eq!(function("(a: i32, b: string..., c: i32 = 1) -> ()"), NOTHING);
    assert_eq!(function("(a: string..., b: string...) -> ()"), ["LC2107"]);
    assert_eq!(function("(a: string..., b: i32) -> ()"), ["LC2107"]);
    assert_eq!(function("(a: i32 = 1, b: string...) -> ()"), ["LC2107"]);
}

#[test]
fn an_external_function_has_an_err_output() {
    let external = |outputs: &str| {
        let text = format!("extern python(module: \"m\", function: \"f\")\nfn f() -> ({outputs})");
        codes(&[("f.lcf", &text)])
    };
    assert_eq!(external("err: error"), NOTHING);
    assert_eq!(external("x: i32"), ["LC4005"]);
    assert_eq!(external("err: i32"), ["LC4005"]);
    assert_eq!(external("err: Nowhere"), ["LC2003"], "what is unknown stays one mistake");
    assert_eq!(external("err: [Nowhere]"), ["LC2003"], "at any depth");
    assert_eq!(
        codes(&[("E.lct", "type E = error"), ("f.lcf", "extern python(module: \"m\", function: \"f\")\nfn f() -> (err: E)")]),
        ["LC4005"],
        "written as such"
    );
    let (checked, mut names) = run(source("", &[("f.lcf", "extern python(module: \"m\", function: \"f\")\nfn f() -> (err: error)")]));
    assert!(checked.libraries.first().unwrap().functions.get(&names.intern("f")).unwrap().external);
}

#[test]
fn what_comes_with_a_later_milestone() {
    assert_eq!(function("<T: Number>(a: T) -> (b: T)"), ["LC4006"]);
    assert_eq!(function("<T>(a: T[V]) -> ()"), ["LC4006"]);
    assert_eq!(codes(&[("Box.lct", "type Box<T> = struct { value: T }")]), ["LC4006"]);
    assert_eq!(function("(a: ref<f64>) -> ()"), ["LC4006"]);
    assert_eq!(function("(a: queue<f64>) -> ()"), ["LC4006"]);
}

#[test]
fn signatures_are_what_the_passes_above_read() {
    let (checked, mut names) = run(source("", &[("f.lcf", "fn f(a: f64[V], b: i32, c: string..., d: i32 = 1) -> (e: bool) {\n    e = true\n}")]));
    assert_eq!(checked.report.errors(), 0);
    let declared = checked.libraries.first().unwrap().functions.get(&names.intern("f")).unwrap();
    let params: Vec<(bool, bool)> = declared.params.iter().map(|param| (param.has_default, param.variadic)).collect();
    assert_eq!(params, [(false, false), (false, false), (false, true), (true, false)]);
    assert_eq!(checked.types.get(declared.outputs.first().unwrap().ty), &Ty::Bool);
    assert!(!declared.external);
}

#[test]
fn the_diagnostic_of_a_signature_names_its_function() {
    let report = run(source("", &[("f.lcf", "fn f(a: Nowhere) -> () {}")])).0.report;
    assert_eq!(report.diagnostics().first().unwrap().function.as_deref(), Some("f"));
}

/// A host with one dependency of the project, read for what it declares.
struct OneDependency<'a>(&'a str, bool);

impl Libraries for OneDependency<'_> {
    fn find(&self, name: &str, _: Origin) -> Found {
        if name != "dep" {
            return Found::NotFound;
        }
        let mut library = source("dep", &[("dep/T.lct", self.0)]);
        library.checked_elsewhere = self.1;
        Found::Library(library)
    }
}

#[test]
fn the_errors_of_a_dependency_are_its_own() {
    let mut names = Interner::new();
    let root = source("", &[("f.lcf", "use dep\n\nfn f(a: dep.T) -> () {}")]);
    let checked = check(root, &OneDependency("type T = Nowhere", false), &mut names, &Limits::default());
    assert_eq!(checked.report.errors(), 0);
    let [dependency] = checked.dependencies.as_slice() else { panic!("{:?}", checked.dependencies) };
    assert_eq!(dependency.report.diagnostics().first().map(Diagnostic::code), Some(Code::Lc2003));
}

#[test]
fn a_dependency_checked_elsewhere_says_nothing_more() {
    let mut names = Interner::new();
    let root = source("", &[("f.lcf", "use dep\n\nfn f(a: dep.T) -> () {}")]);
    let checked = check(root, &OneDependency("type T = Nowhere", true), &mut names, &Limits::default());
    assert_eq!(checked.report.errors(), 0);
    assert!(checked.dependencies.is_empty());
}

#[test]
fn a_file_bears_the_name_of_what_it_declares() {
    assert_eq!(codes(&[("scale.lcf", "fn scale() -> () {}"), ("Verdict.lct", "type Verdict = enum { Pass, Fail }")]), NOTHING);
    let root = source("", &[("gain.lcf", "fn scale() -> () {}")]);
    let mut report = run(root).0.report;
    report.finish();
    let [diagnostic] = report.diagnostics() else { panic!("{:?}", report.diagnostics()) };
    assert_eq!(diagnostic.code(), Code::Lc2023);
    assert_eq!(diagnostic.function.as_deref(), Some("scale"));
    let titles: Vec<&str> = diagnostic.fixes.iter().map(|fix| fix.title.as_str()).collect();
    assert_eq!(titles, ["rename the function to `gain`", "rename the file to `scale.lcf`"], "either name may be the one meant");
    assert_eq!(codes(&[("verdict.lct", "type Verdict = f64")]), ["LC2023"], "the case counts");
    assert_eq!(codes(&[("cafe\u{301}.lcf", "fn caf\u{E9}() -> () {}")]), NOTHING, "the spelling does not (R-20)");
    let root = source("", &[("my-scale.lcf", "fn scale() -> () {}")]);
    let report = run(root).0.report;
    let titles: Vec<&str> = report.diagnostics().iter().flat_map(|diagnostic| &diagnostic.fixes).map(|fix| fix.title.as_str()).collect();
    assert_eq!(titles, ["rename the file to `scale.lcf`"], "a file name that is no name gives one fix");
}

/// The titles of the fixes of the one `LC2023` of these files.
fn fixes_of_the_misnamed(files: &[(&str, &str)]) -> Vec<String> {
    let report = run(source("", files)).0.report;
    let misnamed: Vec<&Diagnostic> = report.diagnostics().iter().filter(|diagnostic| diagnostic.code() == Code::Lc2023).collect();
    let [diagnostic] = misnamed.as_slice() else { panic!("{:?}", report.diagnostics()) };
    diagnostic.fixes.iter().map(|fix| fix.title.clone()).collect()
}

#[test]
fn a_fix_never_names_what_is_taken() {
    let copy = [("scale.lcf", "fn scale() -> () {}"), ("scale - Copy.lcf", "fn scale() -> () {}")];
    assert_eq!(fixes_of_the_misnamed(&copy), ["give this function and its file a name of their own, or delete the file if it is a copy"]);
    let other_case = [("scale.lcf", "fn scale() -> () {}"), ("a.lcf", "fn Scale() -> () {}")];
    assert_eq!(fixes_of_the_misnamed(&other_case), ["rename the function to `a`"], "`Scale.lcf` is `scale.lcf` on Windows");
    assert_eq!(fixes_of_the_misnamed(&[("con.lcf", "fn scale() -> () {}")]), ["rename the file to `scale.lcf`"], "never to `LC2025`");
    let type_x = [("x.lcf", "fn scale() -> () {}"), ("x.lct", "type x = f64")];
    assert_eq!(fixes_of_the_misnamed(&type_x), ["rename the file to `scale.lcf`"], "never to `LC2013`");
    assert_eq!(fixes_of_the_misnamed(&[("verdict.lct", "type Verdict = f64")]), ["rename the type to `verdict`", "rename the file to `Verdict.lct`"]);
    assert_eq!(fixes_of_the_misnamed(&[("f64.lct", "type Foo = f64")]), ["rename the file to `Foo.lct`"], "never to `LC2022`");
    assert_eq!(fixes_of_the_misnamed(&[("S.lct", "type event = f64")]), ["rename the type to `S`"], "nor keeping it");
    let imported = [("time.lcf", "fn scale() -> () {}"), ("g.lcf", "use time\n\nfn g() -> () {}")];
    assert_eq!(fixes_of_the_misnamed(&imported), ["rename the file to `scale.lcf`"], "never to `LC2014`");
    let kinds = [("scale.lct", "type scale = f64"), ("x.lcf", "fn scale() -> () {}")];
    assert_eq!(fixes_of_the_misnamed(&kinds), ["rename the function to `x`"], "the file name would keep its `LC2013`");
    let other_kind = [("x.lcf", "fn scale() -> () {}"), ("X.lct", "type X = f64")];
    assert_eq!(fixes_of_the_misnamed(&other_kind), ["rename the function to `x`", "rename the file to `scale.lcf`"], "`fn x` beside `type X`");
}

#[test]
fn the_message_names_the_file_as_it_is() {
    let report = run(source("", &[("g.lct", "fn f() -> () {}")])).0.report;
    let messages: Vec<&str> = report.diagnostics().iter().map(|diagnostic| diagnostic.message.as_str()).collect();
    assert!(messages.iter().any(|message| message.starts_with("the file `g.lct` declares the function `f`")), "{messages:?}");
    assert_eq!(fixes_of_the_misnamed(&[("g.lct", "fn f() -> () {}")]), ["rename the function to `g`", "rename the file to `f.lcf`"]);
}

#[test]
fn a_function_and_a_type_of_one_name_are_no_copies_of_each_other() {
    let files = [("scale.lct", "type scale = f64"), ("x.lcf", "fn scale() -> () {}"), ("f.lcf", "fn f() -> () {\n    scale()\n}")];
    let mut found = codes(&files);
    found.sort_unstable();
    assert_eq!(found, ["LC2013", "LC2023"]);
}

#[test]
fn a_copy_under_another_name_leaves_the_original_as_it_is() {
    let files = [("scale.lcf", "fn scale(x: f64) -> (y: f64) {\n    y = x\n}"), ("gain.lcf", "fn scale() -> () {}")];
    let files = [files[0], files[1], ("f.lcf", "fn f() -> (y: f64) {\n    y = scale(1.0)\n}")];
    let mut report = run(source("", &files)).0.report;
    report.finish();
    let found: Vec<(&str, &str)> = report.diagnostics().iter().map(|diagnostic| (diagnostic.code().as_str(), diagnostic.file.as_str())).collect();
    assert_eq!(found, [("LC2023", "gain.lcf")], "no `LC2013`, and the call binds to `scale.lcf`");
}

#[test]
fn two_names_of_one_kind_differ_by_more_than_their_case() {
    let root = source("", &[("Scale.lcf", "fn Scale() -> () {}"), ("scale.lcf", "fn scale() -> () {}"), ("SCALE.lcf", "fn SCALE() -> () {}")]);
    let mut report = run(root).0.report;
    report.finish();
    let [diagnostic] = report.diagnostics() else { panic!("{:?}", report.diagnostics()) };
    assert_eq!((diagnostic.code(), diagnostic.file.as_str()), (Code::Lc2024, "scale.lcf"), "once, on the last file");
    let related: Vec<&str> = diagnostic.related.iter().map(|related| related.file.as_str()).collect();
    assert_eq!(related, ["SCALE.lcf", "Scale.lcf"]);
    assert_eq!(codes(&[("Step.lct", "type Step = f64"), ("STEP.lct", "type STEP = f64")]), ["LC2024"], "types too");
    assert_eq!(codes(&[("step.lcf", "fn step() -> () {}"), ("Step.lct", "type Step = f64")]), NOTHING, "a function and a type do not meet");
}

#[test]
fn windows_keeps_some_names_for_its_devices() {
    assert_eq!(codes(&[("con.lcf", "fn con() -> () {}")]), ["LC2025"]);
    assert_eq!(codes(&[("Nul.lct", "type Nul = f64")]), ["LC2025"]);
    assert_eq!(codes(&[("f.lcf", "fn lpt1() -> () {}")]), ["LC2025"], "and no `LC2023`, whose fix would name such a file");
    assert_eq!(codes(&[("console.lcf", "fn console() -> () {}"), ("com10.lcf", "fn com10() -> () {}")]), NOTHING);
}
