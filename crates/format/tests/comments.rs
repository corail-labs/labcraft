//! Every comment has a place (spec 5.2, `docs/architecture.md`, section 3): one test per slot, and on
//! every program of the repository, no comment is lost or kept twice, wherever it stands.

use std::borrow::Borrow;
use std::fs;
use std::path::{Path, PathBuf};

use labcraft_format::read;
use labcraft_graph::{Attached, ClauseKind, Comment, CommentKind, Decl, File, Function, LayoutKind, Rhs, Statement, StatementKind, TypeExpr};
use labcraft_types::{Interner, Limits};

fn file_of(text: &str) -> File {
    let mut names = Interner::new();
    let result = read(text.as_bytes(), &mut names, &Limits::default());
    result.file.unwrap_or_else(|| panic!("{text:?}: {:?}", result.report.diagnostics()))
}

fn function_of(file: &File) -> &Function {
    match &file.decl {
        Decl::Function(function) => function,
        other => panic!("{other:?}"),
    }
}

fn texts(comments: &[Comment]) -> Vec<&str> {
    comments.iter().map(|comment| comment.text.trim()).collect()
}

fn line_text<C: Borrow<Comment>>(comment: &Option<C>) -> Option<&str> {
    comment.as_ref().map(|comment| comment.borrow().text.trim())
}

#[track_caller]
fn assert_attached(attached: &Attached, before: &[&str], line: Option<&str>) {
    assert_eq!(texts(&attached.before), before);
    assert_eq!(line_text(&attached.line), line);
}

fn in_body(statements: &str) -> String {
    format!("labcraft 1\nfn f() -> () {{\n{statements}\n}}\n")
}

fn statements_of(file: &File) -> &[Statement] {
    &function_of(file).body.statements
}

fn value_of(statement: &Statement) -> &Rhs {
    match &statement.kind {
        StatementKind::Binding { value, .. } => value,
        other => panic!("{other:?}"),
    }
}

#[test]
fn on_the_header() {
    let file = file_of("labcraft 1 // a\nfn f() -> () {}\n");
    assert_eq!(line_text(&file.header_comment), Some("a"));
    assert_eq!(texts(&function_of(&file).comments.before), [""; 0]);
}

#[test]
fn a_comment_before_or_between_use_lines_goes_before_the_section() {
    let file = file_of("labcraft 1\n// a\nuse b // c\n// d\nuse e\n// f\nfn g() -> () {}\n");
    assert_eq!(texts(&file.use_comments), ["a", "d"]);
    assert_eq!(line_text(&file.uses[0].line), Some("c"));
    assert_eq!(line_text(&file.uses[1].line), None);
    assert_attached(&function_of(&file).comments, &["f"], None);
}

#[test]
fn with_no_use_line_a_comment_belongs_to_the_declaration() {
    let file = file_of("labcraft 1\n// a\nfn f() -> () {}\n");
    assert!(file.use_comments.is_empty());
    assert_attached(&function_of(&file).comments, &["a"], None);
}

#[test]
fn documentation_stays_in_its_order_among_the_comments_of_the_declaration() {
    let file = file_of("labcraft 1\nuse a\n// b\n/// c\n// d\n/// e\nfn f() -> () {}\n");
    let before = &function_of(&file).comments.before;
    assert_eq!(texts(before), ["b", "c", "d", "e"]);
    assert_eq!(
        before.iter().map(|comment| comment.kind).collect::<Vec<_>>(),
        [CommentKind::Line, CommentKind::Doc, CommentKind::Line, CommentKind::Doc]
    );
}

#[test]
fn documentation_goes_only_before_the_declaration() {
    for text in [
        "labcraft 1\n/// a\nuse b\nfn f() -> () {}\n",
        "labcraft 1\nuse b /// a\nfn f() -> () {}\n",
        "labcraft 1\nfn f() -> () { /// a\n}\n",
        "labcraft 1\nfn f() -> () {\n/// a\n}\n",
        "labcraft 1\nfn f() -> () {\nx = 1 /// a\n}\n",
        "labcraft 1\nfn f(\n/// a\nb: f64) -> () {}\n",
        "labcraft 1\nfn f() -> () {} /// a\n",
        "labcraft 1\nfn f() -> () {}\n/// a\n",
        "labcraft 1\nfn f() -> () {}\n/// a\npanel {}\n",
        "labcraft 1\nfn f() -> () {}\npanel {\n/// a\n}\n",
        "labcraft 1\nextern rust()\n/// a\nfn f() -> ()\n",
        "labcraft 1\ntype T = struct {\n/// a\nb: f64 }\n",
    ] {
        let mut names = Interner::new();
        let result = read(text.as_bytes(), &mut names, &Limits::default());
        assert_eq!(result.report.diagnostics().iter().map(|d| d.code().as_str()).collect::<Vec<_>>(), ["LC1001"], "{text:?}");
    }
}

#[test]
fn on_a_type_declaration() {
    let file = file_of("labcraft 1\n// a\ntype // b\nT = f64 // c\n");
    let Decl::Type(decl) = &file.decl else { panic!() };
    assert_attached(&decl.comments, &["a", "b"], Some("c"));
    let file = file_of("labcraft 1\ntype T = f64[V] // a\n// b\n");
    let Decl::Type(decl) = &file.decl else { panic!() };
    assert_attached(&decl.comments, &[], Some("a"));
    assert_eq!(texts(&file.end), ["b"]);
}

#[test]
fn on_an_extern_declaration() {
    let file = file_of("labcraft 1\n// a\nextern rust(// b\n) // c\nfn f(\n) -> () // d\n// e\n");
    let Decl::Extern(decl) = &file.decl else { panic!() };
    assert_attached(&decl.comments, &["a", "b", "c"], Some("d"));
    assert!(decl.signature.params_end.is_empty());
    assert_eq!(texts(&file.end), ["e"]);
}

#[test]
fn inside_a_default_value_a_comment_goes_above_its_parameter() {
    let file = file_of(
        "labcraft 1\nfn f(\n    a: L = L { // one\n        low: 1 V, // two\n    },\n    b: R = R.Value( // three\n        0 V,\n    ),\n) -> () {}\n",
    );
    let signature = &function_of(&file).signature;
    assert_attached(&signature.params[0].comments, &["one", "two"], None);
    assert_attached(&signature.params[1].comments, &["three"], None);
}

#[test]
fn before_the_parameters_a_comment_goes_before_the_declaration() {
    let file = file_of("labcraft 1\n// a\nfn // b\nf< // c\nT // d\n> // e\n(z: f64) -> () {}\n");
    let function = function_of(&file);
    assert_attached(&function.comments, &["a", "b", "c", "d", "e"], None);
    assert_attached(&function.signature.params[0].comments, &[], None);
    let file = file_of("labcraft 1\nextern rust(a: \"x\", // b\n    c: \"y\")\nfn f(z: f64) -> ()\n");
    let Decl::Extern(decl) = &file.decl else { panic!() };
    assert_attached(&decl.comments, &["b"], None);
    assert_attached(&decl.signature.params[0].comments, &[], None);
}

/// A nested list takes only what lies inside it.
#[test]
fn a_comment_stays_with_the_item_around_a_nested_struct() {
    let file = file_of("labcraft 1\ntype T = struct { // a\n    x: struct { y: f64 },\n}\n");
    let Decl::Type(decl) = &file.decl else { panic!() };
    let TypeExpr::Struct(outer) = &decl.ty else { panic!() };
    assert_attached(&outer.fields[0].comments, &["a"], None);
    let TypeExpr::Struct(inner) = &outer.fields[0].ty else { panic!() };
    assert_attached(&inner.fields[0].comments, &[], None);

    let file = file_of("labcraft 1\ntype // b\nT = struct { x: f64 }\n");
    let Decl::Type(decl) = &file.decl else { panic!() };
    assert_attached(&decl.comments, &["b"], None);
    let TypeExpr::Struct(structure) = &decl.ty else { panic!() };
    assert_attached(&structure.fields[0].comments, &[], None);

    let file = file_of("labcraft 1\nfn f(x // b\n: struct { // c\n    y: f64 }) -> () {}\n");
    let param = &function_of(&file).signature.params[0];
    assert_attached(&param.comments, &["b"], None);
    let TypeExpr::Struct(structure) = &param.ty else { panic!() };
    assert_attached(&structure.fields[0].comments, &["c"], None);

    let file = file_of("labcraft 1\ntype T = enum { // a\n    A(struct { b: f64 }) }\n");
    let Decl::Type(decl) = &file.decl else { panic!() };
    let TypeExpr::Enum(enumeration) = &decl.ty else { panic!() };
    assert_attached(&enumeration.variants[0].comments, &["a"], None);
    let TypeExpr::Struct(inner) = &enumeration.variants[0].payload[0] else { panic!() };
    assert_attached(&inner.fields[0].comments, &[], None);

    let file = file_of("labcraft 1\ntype T = a< // c\n    struct { b: f64 }>\n");
    let Decl::Type(decl) = &file.decl else { panic!() };
    assert_attached(&decl.comments, &["c"], None);
}

#[test]
fn in_a_split_signature() {
    let file = file_of(concat!(
        "labcraft 1\n",
        "fn f( // a\n",
        "    // b\n",
        "    x: f64, // c\n",
        "    // d\n",
        "    y: f64 // e\n",
        "    , // f\n",
        "    // g\n",
        ") // h\n",
        "-> // i\n",
        "( // j\n",
        "    // k\n",
        "    z: f64, // l\n",
        "    // m\n",
        ") // n\n",
        "// o\n",
        "{ // p\n",
        "} // q\n",
    ));
    let function = function_of(&file);
    let signature = &function.signature;
    assert_attached(&signature.params[0].comments, &["a", "b"], Some("c"));
    assert_attached(&signature.params[1].comments, &["d"], Some("e"));
    assert_eq!(texts(&signature.params_end), ["f", "g"]);
    assert_attached(&signature.outputs[0].comments, &["h", "i", "j", "k"], Some("l"));
    assert_eq!(texts(&signature.outputs_end), ["m", "n", "o"]);
    assert_eq!(line_text(&function.open), Some("p"));
    assert_eq!(line_text(&function.comments.line), Some("q"));
}

#[test]
fn a_comment_inside_an_item_goes_before_it() {
    let file = file_of("labcraft 1\nfn f(x // a\n: // b\nf64[ // c\nV] = // d\n1 // e\n, y: f64) -> () {}\n");
    assert_attached(&function_of(&file).signature.params[0].comments, &["a", "b", "c", "d"], Some("e"));
}

#[test]
fn a_comment_after_a_comma_on_another_line_goes_to_the_next_item() {
    let file = file_of("labcraft 1\nfn f(x: f64\n, // a\ny: f64) -> () {}\n");
    let params = &function_of(&file).signature.params;
    assert_attached(&params[0].comments, &[], None);
    assert_attached(&params[1].comments, &["a"], None);
}

#[test]
fn in_a_struct_and_an_enum() {
    let file = file_of("labcraft 1\ntype T = struct { // a\n    // b\n    x: f64, // c\n    // d\n    y: f64 // e\n    // f\n} // g\n");
    let Decl::Type(decl) = &file.decl else { panic!() };
    let TypeExpr::Struct(structure) = &decl.ty else { panic!() };
    assert_attached(&structure.fields[0].comments, &["a", "b"], Some("c"));
    assert_attached(&structure.fields[1].comments, &["d"], Some("e"));
    assert_eq!(texts(&structure.end), ["f"]);
    assert_eq!(line_text(&decl.comments.line), Some("g"));

    let file = file_of("labcraft 1\ntype T = enum { // a\n    A(f64), // b\n    // c\n    B // d\n    // e\n}\n");
    let Decl::Type(decl) = &file.decl else { panic!() };
    let TypeExpr::Enum(enumeration) = &decl.ty else { panic!() };
    assert_attached(&enumeration.variants[0].comments, &["a"], Some("b"));
    assert_attached(&enumeration.variants[1].comments, &["c"], Some("d"));
    assert_eq!(texts(&enumeration.end), ["e"]);
}

#[test]
fn in_a_struct_of_a_parameter() {
    let file = file_of("labcraft 1\nfn f(x: struct { // a\n    y: f64, // b\n    // c\n}, // d\n) -> () {}\n");
    let param = &function_of(&file).signature.params[0];
    assert_attached(&param.comments, &[], Some("d"));
    let TypeExpr::Struct(structure) = &param.ty else { panic!() };
    assert_attached(&structure.fields[0].comments, &["a"], Some("b"));
    assert_eq!(texts(&structure.end), ["c"]);
}

#[test]
fn on_statements() {
    let file = file_of(concat!(
        "labcraft 1\n",
        "fn f() -> () { // a\n",
        "    // b\n",
        "    x = 1 // c\n",
        "    // d\n",
        "    y = 2\n",
        "    // e\n",
        "} // f\n"
    ));
    let function = function_of(&file);
    assert_eq!(line_text(&function.open), Some("a"));
    assert_attached(&function.body.statements[0].comments, &["b"], Some("c"));
    assert_attached(&function.body.statements[1].comments, &["d"], None);
    assert_eq!(texts(&function.body.end), ["e"]);
    assert_eq!(line_text(&function.comments.line), Some("f"));
}

#[test]
fn a_comment_after_an_open_brace_with_a_statement_on_the_next_line_stays_on_the_brace() {
    let file = file_of(&in_body("// a\nx = 1"));
    assert_attached(&statements_of(&file)[0].comments, &["a"], None);
    assert_eq!(line_text(&function_of(&file).open), None);
}

#[test]
fn inside_a_statement_written_over_several_lines() {
    let file = file_of(&in_body("// a\nx = add( // b\n    // c\n    y, // d\n    z // e\n) // f"));
    assert_attached(&statements_of(&file)[0].comments, &["a", "b", "c", "d", "e"], Some("f"));

    let file = file_of(&in_body("x = T { // a\n    b: 1, // c\n}"));
    assert_attached(&statements_of(&file)[0].comments, &["a", "c"], None);

    let file = file_of(&in_body("#l: f( // a\n    b,\n) // c"));
    assert_attached(&statements_of(&file)[0].comments, &["a"], Some("c"));
}

#[test]
fn a_struct_in_the_type_arguments_of_a_call_keeps_no_comment() {
    let file = file_of(&in_body("x = f<struct { // a\n    b: f64, // c\n    // d\n}>(y) // e"));
    let statement = &statements_of(&file)[0];
    assert_attached(&statement.comments, &["a", "c", "d"], Some("e"));
    let Rhs::Call(call) = value_of(statement) else { panic!() };
    let TypeExpr::Struct(structure) = &call.type_args[0] else { panic!() };
    assert_attached(&structure.fields[0].comments, &[], None);
    assert!(structure.end.is_empty());
}

#[test]
fn on_a_match() {
    let file = file_of(&in_body(concat!(
        "// a\n",
        "x = match // b\n",
        "    y // c\n",
        "{ // d\n",
        "    // e\n",
        "    1 => yield 1 // f\n",
        "    // g\n",
        "    _ => {\n",
        "        z = 1 // h\n",
        "        yield z\n",
        "    } // i\n",
        "    // j\n",
        "} // k",
    )));
    let statement = &statements_of(&file)[0];
    assert_attached(&statement.comments, &["a", "b", "c"], Some("k"));
    let Rhs::Match(branching) = value_of(statement) else { panic!() };
    assert_eq!(line_text(&branching.open), Some("d"));
    assert_attached(&branching.arms[0].comments, &["e"], Some("f"));
    assert_attached(&branching.arms[1].comments, &["g"], Some("i"));
    assert_eq!(texts(&branching.end), ["j"]);
}

#[test]
fn on_a_handle() {
    let file = file_of(&in_body("x = handle y { // a\n    // b\n    _(e) => yield 1 // c\n    // d\n} // e"));
    let statement = &statements_of(&file)[0];
    assert_eq!(line_text(&statement.comments.line), Some("e"));
    let Rhs::Handle(branching) = value_of(statement) else { panic!() };
    assert_eq!(line_text(&branching.open), Some("a"));
    assert_attached(&branching.arms[0].comments, &["b"], Some("c"));
    assert_eq!(texts(&branching.end), ["d"]);
}

#[test]
fn in_an_arm_with_a_body() {
    let file = file_of(&in_body(concat!(
        "x = match y {\n",
        "    _ => { // a\n",
        "        // b\n",
        "        z = 1 // c\n",
        "        // d\n",
        "        yield z // e\n",
        "        // f\n",
        "    }\n",
        "}",
    )));
    let Rhs::Match(branching) = value_of(&statements_of(&file)[0]) else { panic!() };
    let arm = &branching.arms[0];
    assert_eq!(line_text(&arm.open), Some("a"));
    assert_attached(&arm.body.statements[0].comments, &["b"], Some("c"));
    assert_attached(&arm.yields.comments, &["d"], Some("e"));
    assert_eq!(texts(&arm.body.end), ["f"]);
}

/// 5.2: such an arm goes on one line, so everything in it goes on the line before.
#[test]
fn in_an_arm_that_is_only_a_yield() {
    let file = file_of(&in_body(concat!(
        "x = match y {\n",
        "    // a\n",
        "    _ => { // b\n",
        "        // c\n",
        "        yield ( // d\n",
        "            1, // e\n",
        "            2\n",
        "        ) // f\n",
        "        // g\n",
        "    } // h\n",
        "}",
    )));
    let Rhs::Match(branching) = value_of(&statements_of(&file)[0]) else { panic!() };
    let arm = &branching.arms[0];
    assert_attached(&arm.comments, &["a", "b", "c", "d", "e", "f", "g"], Some("h"));
    assert_eq!(line_text(&arm.open), None);
    assert_attached(&arm.yields.comments, &[], None);
    assert!(arm.body.end.is_empty());
    assert!(arm.body.statements.is_empty());
}

#[test]
fn inside_the_pattern_of_an_arm() {
    let file = file_of(&in_body("x = match y {\n    A( // a\n        b // c\n    ) => { // e\n        z = 1\n        yield z\n    }\n}"));
    let Rhs::Match(branching) = value_of(&statements_of(&file)[0]) else { panic!() };
    assert_attached(&branching.arms[0].comments, &["a", "c"], None);
    assert_eq!(line_text(&branching.arms[0].open), Some("e"));
}

#[test]
fn in_the_clauses_of_a_loop() {
    let file = file_of(&in_body(concat!(
        "// a\n",
        "x = loop // b\n",
        "    // c\n",
        "    carry y = 1, // d\n",
        "    // e\n",
        "    carry z = 2 // f\n",
        "    // g\n",
        "{ // h\n",
        "    // i\n",
        "    until w // j\n",
        "    // k\n",
        "} // l",
    )));
    let statement = &statements_of(&file)[0];
    assert_attached(&statement.comments, &["a"], Some("l"));
    let Rhs::Loop(structure) = value_of(statement) else { panic!() };
    assert_attached(&structure.clauses[0].comments, &["b", "c"], Some("d"));
    assert_attached(&structure.clauses[1].comments, &["e"], Some("f"));
    assert_eq!(texts(&structure.clauses_end), ["g"]);
    assert_eq!(line_text(&structure.open), Some("h"));
    assert_attached(&structure.body.statements[0].comments, &["i"], Some("j"));
    assert_eq!(texts(&structure.body.end), ["k"]);
}

#[test]
fn in_the_clauses_of_a_for() {
    let file = file_of(&in_body(concat!(
        "x = for parallel // a\n",
        "    i in y, // b\n",
        "    // c\n",
        "    j in 0..5 // d\n",
        "    , carry z = 1 // e\n",
        "{ // f\n",
        "    collect i // g\n",
        "}",
    )));
    let Rhs::For(structure) = value_of(&statements_of(&file)[0]) else { panic!() };
    assert_attached(&structure.clauses[0].comments, &["a"], Some("b"));
    assert_attached(&structure.clauses[1].comments, &["c"], Some("d"));
    assert_attached(&structure.clauses[2].comments, &[], Some("e"));
    assert!(matches!(structure.clauses[2].kind, ClauseKind::Carry(_)));
    assert_eq!(line_text(&structure.open), Some("f"));
    assert_attached(&structure.body.statements[0].comments, &[], Some("g"));
}

#[test]
fn a_loop_without_clauses_has_its_comments_before_the_statement() {
    let file = file_of(&in_body("x = loop // a\n// b\n{ // c\n    until y\n}"));
    let statement = &statements_of(&file)[0];
    assert_attached(&statement.comments, &["a", "b"], None);
    let Rhs::Loop(structure) = value_of(statement) else { panic!() };
    assert!(structure.clauses_end.is_empty());
    assert_eq!(line_text(&structure.open), Some("c"));
}

#[test]
fn in_a_panel() {
    let file = file_of(concat!(
        "labcraft 1\n",
        "fn f() -> () {}\n",
        "// a\n",
        "panel // b\n",
        "{ // c\n",
        "    // d\n",
        "    control x: y // e\n",
        "    // f\n",
        "    indicator z: w( // g\n",
        "        a: 1, // h\n",
        "        b: 2\n",
        "    ) // i\n",
        "    // j\n",
        "} // k\n",
    ));
    let panel = file.panel.as_ref().unwrap();
    assert_attached(&panel.comments, &["a"], Some("k"));
    assert_attached(&panel.items[0].comments, &["b", "c", "d"], Some("e"));
    assert_attached(&panel.items[1].comments, &["f", "g", "h"], Some("i"));
    assert_eq!(texts(&panel.end), ["j"]);
}

#[test]
fn in_a_layout() {
    let file = file_of(concat!(
        "labcraft 1\n",
        "fn f() -> () {}\n",
        "// a\n",
        "layout { // b\n",
        "    node x view compact // c\n",
        "    // d\n",
        "    wire x -> y via ( // e\n",
        "        1, 2) // f\n",
        "    // g\n",
        "} // h\n",
    ));
    let layout = file.layout.as_ref().unwrap();
    assert_attached(&layout.comments, &["a"], Some("h"));
    assert_attached(&layout.items[0].comments, &["b"], Some("c"));
    assert_attached(&layout.items[1].comments, &["d", "e"], Some("f"));
    assert!(matches!(layout.items[1].kind, LayoutKind::Wire { .. }));
    assert_eq!(texts(&layout.end), ["g"]);
}

#[test]
fn in_an_icon() {
    let file = file_of(concat!(
        "labcraft 1\n",
        "fn f() -> () {}\n",
        "// a\n",
        "icon // b\n",
        "32 // c\n",
        "{ // d\n",
        "    line (1, 2) (3, 4) // e\n",
        "    // f\n",
        "    path \"M0 0\" // g\n",
        "    // h\n",
        "} // i\n",
    ));
    let icon = file.icon.as_ref().unwrap();
    assert_attached(&icon.comments, &["a"], Some("i"));
    assert_attached(&icon.items[0].comments, &["b", "c", "d"], Some("e"));
    assert_attached(&icon.items[1].comments, &["f"], Some("g"));
    assert_eq!(texts(&icon.end), ["h"]);
}

#[test]
fn a_comment_outside_a_declaration_belongs_to_the_section_after_it_or_to_the_end() {
    let file = file_of("labcraft 1\nfn f() -> () {} // a\n// b\npanel {} // c\n// d\nlayout {}\n// e\nicon 32 {}\n// f\n// g\n");
    assert_attached(&function_of(&file).comments, &[], Some("a"));
    assert_attached(&file.panel.as_ref().unwrap().comments, &["b"], Some("c"));
    assert_attached(&file.layout.as_ref().unwrap().comments, &["d"], None);
    assert_attached(&file.icon.as_ref().unwrap().comments, &["e"], None);
    assert_eq!(texts(&file.end), ["f", "g"]);
}

#[test]
fn comments_keep_their_text_in_their_one_spelling() {
    let file = file_of("labcraft 1\n//no space\n//    four spaces  \n//\n//   \n///   doc  \n//\ttab\t\nfn f() -> () {}\n");
    let before = &function_of(&file).comments.before;
    assert_eq!(before.iter().map(|comment| comment.text.as_ref()).collect::<Vec<_>>(), [" no space", "    four spaces", "", "", "   doc", " \ttab"]);
}

fn programs() -> Vec<PathBuf> {
    fn walk(folder: &Path, files: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, files);
            } else if path.extension().is_some_and(|extension| extension == "lcf" || extension == "lct") {
                files.push(path);
            }
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for folder in ["examples", "libs", "docs/rfc", "tests/acceptance/type-errors"] {
        walk(&root.join(folder), &mut files);
    }
    assert!(files.len() > 20);
    files
}

/// `Debug` shows every comment of the model. A marker is a word no program holds.
fn assert_each_marker_is_there_once(text: &str, markers: usize, origin: &Path) {
    let mut names = Interner::new();
    let result = read(text.as_bytes(), &mut names, &Limits::default());
    let file = result.file.unwrap_or_else(|| panic!("{}: {:?}\n{text}", origin.display(), result.report.diagnostics()));
    let model = format!("{file:?}");
    assert!(markers > 0);
    for marker in 0..markers {
        let needle = format!("marker{marker}.");
        assert_eq!(model.matches(&needle).count(), 1, "{needle} in {}\n{text}", origin.display());
    }
}

/// `5.2: no comment is lost`, for a comment at the end of every line and on a line of its own before every
/// line: the two places where a comment can stand, on real programs.
#[test]
fn no_comment_is_lost_or_kept_twice_on_any_program() {
    for path in programs() {
        let source = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = source.lines().collect();

        let mut markers = 0;
        let mut at_line_ends = String::new();
        for line in &lines {
            at_line_ends.push_str(line);
            if !line.contains("//") {
                at_line_ends.push_str(&format!(" // marker{markers}."));
                markers += 1;
            }
            at_line_ends.push('\n');
        }
        assert_each_marker_is_there_once(&at_line_ends, markers, &path);

        let mut markers = 0;
        let mut on_their_own = String::new();
        for (index, line) in lines.iter().enumerate() {
            if index > 0 {
                on_their_own.push_str(&format!("// marker{markers}.\n"));
                markers += 1;
            }
            on_their_own.push_str(line);
            on_their_own.push('\n');
        }
        assert_each_marker_is_there_once(&on_their_own, markers, &path);
    }
}
