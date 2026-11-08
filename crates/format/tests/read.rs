//! `read`: the header, the codes of syntax errors, line endings, nesting and the shape of the model.

mod deep;

use deep::{DEEP, deep_array_type, deep_loop, in_body, nested};
use labcraft_format::{Read, STACK_SIZE, read};
use labcraft_graph::{
    Arg, Constant, Decl, File, Function, LayoutKind, Numeral, NumeralKind, Operand, OrderRef, Pattern, Rhs, Statement, StatementKind, Target,
    TypeExpr, UnitOp, UnitTerm,
};
use labcraft_types::{Code, Interner, Limits, Position, Span};

const BIDI: &str = "\u{202E}";

/// Deep nesting needs more stack than a test thread has in a debug build, so these tests take what a host
/// gives. `tests/stack.rs` measures it.
fn on_a_large_stack<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new().stack_size(STACK_SIZE).spawn(work).unwrap().join().unwrap()
}

struct Reading {
    names: Interner,
    read: Read,
}

impl Reading {
    fn of(text: &str) -> Reading {
        let mut names = Interner::new();
        let read = read(text.as_bytes(), &mut names, &Limits::default());
        Reading { names, read }
    }

    fn file(&self) -> &File {
        self.read.file.as_ref().unwrap_or_else(|| panic!("{:?}", self.read.report.diagnostics()))
    }

    fn function(&self) -> &Function {
        match &self.file().decl {
            Decl::Function(function) => function,
            other => panic!("{other:?}"),
        }
    }

    fn text(&self, name: labcraft_types::Name) -> &str {
        self.names.resolve(name)
    }
}

fn codes(text: &str) -> Vec<Code> {
    Reading::of(text).read.report.diagnostics().iter().map(|diagnostic| diagnostic.code()).collect()
}

fn statement_of(reading: &Reading) -> &Statement {
    &reading.function().body.statements[0]
}

/// The value of the one statement of a body, `x = …`.
fn value_of(reading: &Reading) -> &Rhs {
    match &statement_of(reading).kind {
        StatementKind::Binding { value, .. } => value,
        other => panic!("{other:?}"),
    }
}

/// The arguments of `x = f(…)`.
fn args_of(reading: &Reading) -> &[Arg] {
    match value_of(reading) {
        Rhs::Call(call) => &call.args,
        other => panic!("{other:?}"),
    }
}

fn assert_reads(text: &str) {
    let reading = Reading::of(text);
    assert!(reading.read.file.is_some(), "{text:?}: {:?}", reading.read.report.diagnostics());
}

#[track_caller]
fn assert_code(text: &str, code: Code) {
    let reading = Reading::of(text);
    assert_eq!(reading.read.report.diagnostics().iter().map(|d| d.code()).collect::<Vec<_>>(), [code], "{text:?}");
    assert!(reading.read.file.is_none(), "{text:?}");
}

#[track_caller]
fn assert_span(text: &str, start: (u32, u32), end: (u32, u32)) {
    let reading = Reading::of(text);
    let span = reading.read.report.diagnostics()[0].span.unwrap_or_else(|| panic!("no span for {text:?}"));
    let expected = Span { start: Position { line: start.0, column: start.1 }, end: Position { line: end.0, column: end.1 } };
    assert_eq!(span, expected, "{text:?}");
}

const MINIMAL: &str = "fn f() -> () {}\n";

// The header (spec 14.2).

#[test]
fn nothing_but_spaces_goes_before_the_header() {
    assert_reads(&format!("labcraft 1\n{MINIMAL}"));
    assert_reads(&format!("  \tlabcraft 1\n{MINIMAL}"));
    for before in ["\n", "// comment\n", "/// documentation\n", "  \n", "x\n", "$", "\"", "\u{3164}"] {
        assert_code(&format!("{before}labcraft 1\n{MINIMAL}"), Code::Lc0101);
    }
}

#[test]
fn the_header_stands_on_one_line() {
    for text in ["labcraft\n1\n", "labcraft // c\n1\n"] {
        assert_code(&format!("{text}{MINIMAL}"), Code::Lc0101);
    }
}

#[test]
fn only_a_comment_follows_the_header_on_its_line() {
    let reading = Reading::of(&format!("labcraft 1 // the format\n{MINIMAL}"));
    assert_eq!(reading.file().header_comment.as_ref().unwrap().text.as_ref(), " the format");
    assert_reads(&format!("labcraft 1//c\n{MINIMAL}"));
    for after in ["x", "1", "fn f() -> () {}", "use a", "/// documentation", "$", "\"abc", "\"\\q\"", ";", "\u{3164}"] {
        assert_code(&format!("labcraft 1 {after}\n{MINIMAL}"), Code::Lc0101);
    }
}

#[test]
fn spaces_in_the_header_are_tolerated() {
    assert_eq!(Reading::of(&format!("labcraft \t 1  \t\n{MINIMAL}")).file().version, 1);
    assert_reads(&format!("labcraft 1\r\n{MINIMAL}"));
    assert_reads(&format!("\u{FEFF}labcraft 1\n{MINIMAL}"));
}

#[test]
fn what_is_not_a_header_is_lc0101() {
    for text in [
        "",
        "   ",
        "\n",
        "labcraft",
        "labcraft\n",
        "labcraft ",
        "labcraft 1.5\n",
        "labcraft 1e3\n",
        "labcraft x\n",
        "labcraft -1\n",
        "x 1\n",
        "labcraft2 1\n",
    ] {
        assert_code(text, Code::Lc0101);
    }
    assert_code("labcraft 1x\n", Code::Lc0101);
    assert_code("labcraft 99 x\n", Code::Lc0101);
}

#[test]
fn only_the_exact_text_1_is_a_version() {
    assert_eq!(Reading::of(&format!("labcraft 1\n{MINIMAL}")).file().version, 1);
    for version in ["0", "2", "01", "001", "1_", "1_0", "0_1", "99999", "4294967296", "99999999999999999999999999"] {
        assert_code(&format!("labcraft {version}\n{MINIMAL}"), Code::Lc0102);
        assert_code(&format!("labcraft {version}"), Code::Lc0102);
    }
}

#[test]
fn a_way_of_writing_1_says_what_to_write() {
    let many_zeros = format!("{}1", "0".repeat(300));
    for version in ["01", "001", "1_", "0_1", many_zeros.as_str()] {
        let reading = Reading::of(&format!("labcraft {version}\n"));
        assert!(reading.read.report.diagnostics()[0].message.contains("write `labcraft 1`"), "{version}");
    }
    let reading = Reading::of("labcraft 99999\n");
    assert!(!reading.read.report.diagnostics()[0].message.contains("write"));
}

#[test]
fn the_header_diagnostics_have_a_place() {
    assert_span("", (1, 1), (1, 2));
    assert_span("\nlabcraft 1\n", (1, 1), (2, 1));
    assert_span("labcraft 1 x\n", (1, 12), (1, 13));
    assert_span("labcraft 99\n", (1, 10), (1, 12));
}

#[test]
fn a_file_with_only_a_header_is_truncated() {
    assert_code("labcraft 1", Code::Lc1002);
    assert_code("labcraft 1\n\n// nothing\n", Code::Lc1002);
    assert_span("labcraft 1\n", (2, 1), (2, 2));
}

// Codes that stand alone, and those that do not.

#[test]
fn a_code_that_stands_alone_replaces_the_warnings_before_it() {
    for (text, code) in [(format!("// {BIDI}\nlabcraft 1\n{MINIMAL}"), Code::Lc0101), (format!("labcraft 9 // {BIDI}\n"), Code::Lc0102)] {
        let reading = Reading::of(&text);
        assert_eq!((reading.read.report.errors(), reading.read.report.warnings()), (1, 0), "{text:?}");
        assert_eq!(reading.read.report.diagnostics()[0].code(), code, "{text:?}");
    }
    on_a_large_stack(move || {
        let deep = format!("{}{}f64{}", "[".repeat(300), "", "]".repeat(300));
        let text = format!("labcraft 1\n// {BIDI}\nfn f(a: {deep}) -> () {{}}\n");
        let reading = Reading::of(&text);
        assert_eq!((reading.read.report.errors(), reading.read.report.warnings()), (1, 0));
        assert_eq!(reading.read.report.diagnostics()[0].code(), Code::Lc0007);
    });
}

#[test]
fn a_syntax_error_keeps_the_warnings_before_it() {
    let reading = Reading::of(&format!("labcraft 1\n// {BIDI}\nfn f( {{\n"));
    assert_eq!((reading.read.report.errors(), reading.read.report.warnings()), (1, 1));
    assert!(reading.read.file.is_none());
}

#[test]
fn warnings_alone_leave_a_model() {
    let reading = Reading::of(&format!("labcraft 1\n// {BIDI}\n{MINIMAL}"));
    assert_eq!((reading.read.report.errors(), reading.read.report.warnings()), (0, 1));
    assert!(reading.read.file.is_some());
}

#[test]
fn errors_of_decoding_stand_alone_and_have_no_model() {
    let mut names = Interner::new();
    let mut limits = Limits::default();
    limits.max_file_size = 4;
    let result = read(b"labcraft 1\n", &mut names, &limits);
    assert_eq!(result.report.diagnostics()[0].code(), Code::Lc0006);
    assert!(result.file.is_none());
    assert_code("labcraft 1\n\u{7}", Code::Lc0004);
    assert_eq!(read(b"\xFF", &mut names, &Limits::default()).report.diagnostics()[0].code(), Code::Lc0003);
}

// Syntax errors.

#[test]
fn a_syntax_error_names_what_was_expected() {
    let reading = Reading::of("labcraft 1\nfn f( {\n");
    let diagnostic = &reading.read.report.diagnostics()[0];
    assert_eq!(diagnostic.code(), Code::Lc1001);
    assert!(diagnostic.message.contains("expected"), "{}", diagnostic.message);
    assert!(diagnostic.message.contains("`{`"), "{}", diagnostic.message);
    assert_span("labcraft 1\nfn f( {\n", (2, 7), (2, 8));
}

#[test]
fn only_the_first_error_is_reported() {
    assert_eq!(codes("labcraft 1\nfn f( {\n$ $ $ fn loop"), [Code::Lc1001]);
}

#[test]
fn every_kind_of_truncation_is_lc1002() {
    for text in [
        "labcraft 1\nuse",
        "labcraft 1\nfn",
        "labcraft 1\nfn f",
        "labcraft 1\nfn f(",
        "labcraft 1\nfn f(a",
        "labcraft 1\nfn f(a:",
        "labcraft 1\nfn f() ->",
        "labcraft 1\nfn f() -> (",
        "labcraft 1\nfn f() -> ()",
        "labcraft 1\nfn f() -> () {",
        "labcraft 1\nfn f() -> () {\nx = ",
        "labcraft 1\nfn f() -> () {\nx = add(",
        "labcraft 1\nfn f() -> () {\nx = match y {",
        "labcraft 1\nfn f() -> () {\nx = match y {\n_ => {",
        "labcraft 1\nfn f() -> () {\nx = match y {\n_ => ",
        "labcraft 1\nfn f() -> () {\nx = loop {",
        "labcraft 1\nfn f() -> () {\nx = [1,",
        "labcraft 1\ntype T =",
        "labcraft 1\ntype T = struct {",
        "labcraft 1\nextern rust()",
        "labcraft 1\nfn f() -> () {}\npanel {",
        "labcraft 1\nfn f() -> () {}\nlayout {",
        "labcraft 1\nfn f() -> () {}\nicon 32 {",
        "labcraft 1\nfn f() -> () {}\nicon",
        "labcraft 1\n/// only documentation\n",
        "labcraft 1\nfn f() -> () {\nx = -",
        "labcraft 1\nfn f() -> () {\nx = match a {\n1..-",
        "labcraft 1\nfn f() -> () {\nx = handle a {\ncode -",
    ] {
        assert_eq!(codes(text).first(), Some(&Code::Lc1002), "{text:?}");
    }
}

#[test]
fn lc1002_points_at_where_the_file_ends() {
    assert_span("labcraft 1\nfn f(  ", (2, 8), (2, 9));
}

#[test]
fn an_unterminated_string_stays_lc1003() {
    assert_code(&in_body("x = \"abc"), Code::Lc1003);
}

#[test]
fn a_nested_call_is_lc1004_and_its_fix_names_the_value() {
    let text = in_body("x = add(mul(a, b), c)");
    assert_code(&text, Code::Lc1004);
    assert_span(&text, (3, 9), (3, 12));
    let reading = Reading::of(&text);
    let fix = &reading.read.report.diagnostics()[0].fixes[0];
    assert!(fix.title.contains("mul_value = mul(…)"), "{}", fix.title);
    for statement in [
        "x = add(visa.read(a))",
        "x = add(f<T>(a))",
        "x = add(b: f(a))",
        "x = [f(a)]",
        "x = T { a: f(b) }",
        "#a: f(after: g(x))",
        "x = match f(a) {\n_ => yield 1\n}",
        "x = for i in f(a) {\n}",
        "x = for i in 0..f(a) {\n}",
        "x = loop carry a = f(b) {\nuntil true\n}",
        "x = loop {\nuntil f(a)\n}",
        "x = for i in a {\ncollect f(i)\n}",
        "x = match a {\n_ => yield f(a)\n}",
        "x = match a {\n_ => yield (b, f(a))\n}",
    ] {
        assert_code(&in_body(statement), Code::Lc1004);
    }
}

#[test]
fn a_reserved_word_used_as_a_name_is_lc1005() {
    for text in [
        "labcraft 1\nfn loop() -> () {}\n",
        "labcraft 1\nuse fn\n",
        "labcraft 1\nuse a.for\n",
        "labcraft 1\ntype match = f64\n",
        "labcraft 1\ntype T = fn2.yield\n",
        "labcraft 1\ntype T = struct { for: f64 }\n",
        "labcraft 1\ntype T = enum { none }\n",
        "labcraft 1\nfn f(for: f64) -> () {}\n",
        "labcraft 1\nfn f() -> (true: f64) {}\n",
        "labcraft 1\nfn f(a: use) -> () {}\n",
        "labcraft 1\nfn f<fn>() -> () {}\n",
        "labcraft 1\nextern loop() fn f() -> ()\n",
    ] {
        assert_code(text, Code::Lc1005);
    }
    for statement in [
        "for = add(a)",
        "true = add(a)",
        "a, panel = add(a)",
        "x = fn",
        "x = add(icon)",
        "x = add(a.loop)",
        "x = [use]",
        "#for: x = a",
        "next = 1",
        "collect = 1",
        "until = 1",
        "yield = 1",
        "events = 1",
        "x = add(events)",
        "x = f([V/none])",
        "x = f(true: 1)",
        "x = f(none: 1)",
        "x = f(inf: 1)",
        "x = f(nan: 1)",
        "for = 1",
        "match, a = 1",
        "x = 5 V/none",
    ] {
        assert_code(&in_body(statement), Code::Lc1005);
    }
}

/// Spec 14.1, new words: what a later milestone may use takes a contextual word, so it stays a name today.
#[test]
fn the_words_of_later_milestones_are_names() {
    for word in ["test", "timed", "cyclic", "task", "global", "break", "event", "notifier"] {
        assert_reads(&in_body(&format!("{word} = add({word}, {word}.{word})")));
        assert_reads(&format!("labcraft 1\nfn {word}({word}: f64) -> () {{}}\n"));
    }
}

#[test]
fn a_contextual_word_is_a_name_outside_its_place() {
    for word in ["ok", "code", "in", "after", "opaque", "control", "indicator", "node", "wire", "at", "via", "view", "rect", "fill", "accent"] {
        assert_reads(&in_body(&format!("{word} = add({word}, {word}.{word})")));
        assert_reads(&format!("labcraft 1\nfn {word}({word}: {word}) -> () {{}}\n"));
    }
}

#[test]
fn a_misplaced_statement_is_a_syntax_error() {
    for statement in [
        "for i in a {\n}",
        "loop {\nuntil a\n}",
        "match a {\n_ => yield 1\n}",
        "handle a {\n_ => yield 1\n}",
        "yield a",
        "collect a",
        "until a",
        "next a = 1",
        "x, next a = f()",
        "x = loop {\nuntil a\nuntil b\n}",
        "x = loop {\n}",
        "x = for i in a {\nuntil b\n}",
        "x = match a {\n_ => {\nuntil a\nyield 1\n}\n}",
        "x = match a {\n_ => {\nnext a = 1\nyield 1\n}\n}",
        "x = match a {\n_ => {\ncollect a\nyield 1\n}\n}",
        "x = match a {\n_ => {\n}\n}",
        "x = match a {\n_ => {\nb = 1\n}\n}",
        "x = match a {\n_ => {\nyield 1\nb = 2\n}\n}",
        "x = match a {\n_ => {\nyield 1\nyield 2\n}\n}",
        "#a: yield b",
        "x = loop {\n#a: until b\n}",
        "x = for i in a {\n#a: collect b\n}",
    ] {
        assert_code(&in_body(statement), Code::Lc1001);
    }
}

#[test]
fn a_loop_takes_its_statements_directly_in_its_body() {
    assert_reads(&in_body("x = loop {\nnext a, b = f()\ncollect a\nuntil c\ncollect d\n}"));
    assert_reads(&in_body("x = for i in a {\n#l: next b = f()\ncollect a\n}"));
}

#[test]
fn a_file_is_one_declaration_and_its_sections_in_order() {
    assert_reads("labcraft 1\nfn f() -> () {}\npanel {}\nlayout {}\nicon 32 {}\n// end\n");
    assert_reads("labcraft 1\nfn f() -> () {}\nlayout {}\n");
    assert_reads("labcraft 1\nfn f() -> () {}\nicon 32 {}\n");
    for text in [
        "labcraft 1\nfn f() -> () {}\nfn g() -> () {}\n",
        "labcraft 1\nfn f() -> () {}\nlayout {}\npanel {}\n",
        "labcraft 1\nfn f() -> () {}\nicon 32 {}\nlayout {}\n",
        "labcraft 1\nfn f() -> () {}\npanel {}\npanel {}\n",
        "labcraft 1\nfn f() -> () {}\nuse a\n",
        "labcraft 1\nfn f() -> () {}\nx\n",
        "labcraft 1\npanel {}\n",
        "labcraft 1\nx\n",
        "labcraft 1\nfn f() -> () {} }\n",
        "labcraft 1\nfn f() -> () {} panel {}\n",
        "labcraft 1\nuse a use b\n",
        "labcraft 1\nuse a fn f() -> () {}\n",
        "labcraft 1\ntype T = f64 fn\n",
    ] {
        assert_code(text, Code::Lc1001);
    }
}

#[test]
fn what_only_check_can_refuse_still_reads() {
    for text in [
        "labcraft 1\ntype T = f64\npanel {}\n",
        "labcraft 1\ntype T = f64\nlayout {}\nicon 0 {}\n",
        "labcraft 1\nfn f(a: f64[V]...) -> () {}\n",
        "labcraft 1\nfn f(a: [f64; 4294967296]) -> () {}\n",
        "labcraft 1\nfn f(a: f64[m^99999999]) -> () {}\n",
        "labcraft 1\nfn f() -> () {}\nlayout {\nnode a at (1e300, -1e300)\nnode b at (5.0, 1_0)\n}\n",
    ] {
        assert_reads(text);
    }
    assert_reads(&in_body("x = 1e999\ny = 99999999999999999999999999\nz = 2s"));
}

// Line endings (14.1).

#[test]
fn a_line_ending_ends_a_statement() {
    for statement in [
        "x =\nadd(a)",
        "x\n= add(a)",
        "x,\ny = add(a)",
        "x\n, y = add(a)",
        "#a:\nx = add(a)",
        "#a\n: x = add(a)",
        "#\na: x = add(a)",
        "x = visa\n.read(a)",
        "x = visa.\nread(a)",
        "x = add\n(a)",
        "x = T\n{ a: 1 }",
        "x = 1 m/\ns",
        "add\n(a)",
        "x = f<\nT>(a)",
        "x = f<T\n>(a)",
        "x = f<T // comment\n>(a)",
        "x = f<a<\nb>>(c)",
        "x = // comment\nadd(a)",
        "x = match a {\n_\n=> yield 1\n}",
        "x = match a {\n_ =>\nyield 1\n}",
        "x = match a {\n_ =>\n{\nyield 1\n}\n}",
        "x = match a {\n_ => yield\n1\n}",
        "x = loop {\nuntil\na\n}",
        "x = for i in a {\ncollect\na\n}",
    ] {
        assert_code(&in_body(statement), Code::Lc1001);
    }
    assert_code("labcraft 1\nuse\na\nfn f() -> () {}\n", Code::Lc1001);
    assert_code("labcraft 1\nuse a\n.b\nfn f() -> () {}\n", Code::Lc1001);
}

#[test]
fn line_endings_are_ignored_inside_brackets_and_headers() {
    for statement in [
        "x = add(\na,\nb\n)",
        "x = add(a,\nb: [\n1,\n2\n])",
        "x = T {\na: 1,\nb: 2,\n}",
        "x = 1 m/(\ns\n)",
        "x = with_unit(a, [\nV\n/\ns\n])",
        "x = match\na\n{\n_ => yield 1\n}",
        "x = for\nparallel\ni\nin\na\n{\n}",
        "x = for\ni in 0\n..\n5,\ncarry a = 1\n{\n}",
        "x = loop\ncontinue_on_error\ncarry a\n:\nf64\n=\n1,\ncarry b = 2\n{\nuntil c\n}",
        "x = handle\na\n{\n_ => yield 1\n}",
        "x = match a {\nb.c(\nd,\ne\n) => yield 1\n_ => yield (\n1,\n2\n)\n}",
    ] {
        assert_reads(&in_body(statement));
    }
}

#[test]
fn a_declaration_ignores_line_endings() {
    assert_reads("labcraft 1\nfn\nf\n<\nT\n>\n(\na\n:\nf64\n=\n1\n)\n->\n(\nb\n:\nf64\n)\n{\n}\n");
    assert_reads("labcraft 1\nextern\nrust\n(\n)\nfn\nf\n(\n)\n->\n(\n)\n");
    assert_reads("labcraft 1\ntype\nT\n=\nstruct\n{\na\n:\nf64\n,\n}\n");
    assert_reads("labcraft 1\nfn f() -> () {}\npanel\n{\n}\nlayout\n{\n}\nicon\n32\n{\n}\n");
}

#[test]
fn two_statements_never_share_a_line() {
    for statement in [
        "a = f() b = g()",
        "a = 1 b = 2 c",
        "f() g()",
        "x = match a {\n_ => yield 1 _ => yield 2\n}",
        "x = match a {\n_ => yield 1\n} y = 2",
        "x = loop {\nuntil a b = 1\n}",
        "x = \"s\" y = 2",
    ] {
        assert_code(&in_body(statement), Code::Lc1001);
    }
    assert_code("labcraft 1\nfn f() -> () {}\npanel {\ncontrol a: b control c: d\n}\n", Code::Lc1001);
    assert_code("labcraft 1\nfn f() -> () {}\nlayout {\nnode a view compact node b view compact\n}\n", Code::Lc1001);
    assert_code("labcraft 1\nfn f() -> () {}\nicon 32 {\nline (1, 2) (3, 4) line (1, 2) (3, 4)\n}\n", Code::Lc1001);
}

#[test]
fn a_body_can_stand_on_the_line_of_its_braces() {
    assert_reads("labcraft 1\nfn f() -> () { x = 1 }\n");
    assert_reads(&in_body("x = match a { _ => yield 1 }"));
    assert_reads(&in_body("x = match a { _ => { yield 1 } }"));
    assert_reads(&in_body("x = loop { until a }"));
    assert_reads("labcraft 1\nfn f() -> () {}\npanel { control a: b }\nlayout { node a view compact }\nicon 32 { line (1, 2) (3, 4) }\n");
}

// Final commas, as the grammar gives them.

#[test]
fn a_final_comma_only_where_the_grammar_allows_it() {
    assert_reads("labcraft 1\nfn f(a: f64,) -> (b: f64,) {}\n");
    assert_reads("labcraft 1\ntype T = struct { a: f64, }\n");
    assert_reads("labcraft 1\ntype T = enum { A, }\n");
    assert_reads(&in_body("x = f(a,)"));
    assert_reads(&in_body("x = T { a: 1, }"));
    for text in [
        "labcraft 1\nfn f(,) -> () {}\n".to_owned(),
        "labcraft 1\nfn f<T,>() -> () {}\n".to_owned(),
        "labcraft 1\nextern rust(a,) fn f() -> ()\n".to_owned(),
        "labcraft 1\ntype T = fn(a,) -> ()\n".to_owned(),
        "labcraft 1\ntype T = enum { A(f64,) }\n".to_owned(),
        "labcraft 1\ntype T = struct {}\n".to_owned(),
        "labcraft 1\ntype T = enum {}\n".to_owned(),
        "labcraft 1\nfn f() -> () {}\npanel {\ncontrol a: b(c,)\n}\n".to_owned(),
        in_body("x = [1,]"),
        in_body("x = f(,)"),
        in_body("x = T {}"),
        in_body("x = match a {\nb(c,) => yield 1\n}"),
        in_body("x = match a {\n_ => yield (1,)\n}"),
        in_body("x = loop carry a = 1, {\nuntil a\n}"),
        in_body("x = for i in a, {\n}"),
        in_body("x = f(after: (a,))"),
        in_body("x = f(after: ())"),
    ] {
        assert_code(&text, Code::Lc1001);
    }
}

// Nesting (14.1): one counter.

#[test]
fn each_kind_of_nesting_passes_at_the_limit_and_fails_above_it() {
    on_a_large_stack(|| {
        for (kind, deep) in DEEP {
            let at_limit = Reading::of(&deep(256));
            assert!(at_limit.read.file.is_some(), "{kind}: {:?}", at_limit.read.report.diagnostics());
            assert_eq!(codes(&deep(257)), [Code::Lc0007], "{kind}");
        }
    });
}

#[test]
fn a_hundred_thousand_levels_give_lc0007() {
    on_a_large_stack(|| {
        for (kind, deep) in DEEP {
            assert_eq!(codes(&deep(100_000)), [Code::Lc0007], "{kind}");
        }
    });
}

#[test]
fn the_kinds_of_nesting_share_one_counter() {
    on_a_large_stack(|| {
        let mixed = |structures: usize, parentheses: usize, brackets: usize| {
            let array = format!("{}x{}", "[".repeat(brackets), "]".repeat(brackets));
            let inner = format!("n = 1 {}V{}\nz = [{array}, 1]\n", "(".repeat(parentheses), ")".repeat(parentheses));
            in_body(&nested(
                structures,
                "v = loop {
",
                &inner,
                "until a
}
",
            ))
        };
        assert_reads(&mixed(100, 156, 155));
        assert_eq!(codes(&mixed(101, 156, 155)), [Code::Lc0007]);
        assert_eq!(codes(&mixed(100, 157, 155)), [Code::Lc0007]);
        assert_eq!(codes(&mixed(100, 156, 156)), [Code::Lc0007]);
    });
}

#[test]
fn a_structure_counts_once_with_its_arms() {
    on_a_large_stack(|| {
        let braced = |depth| {
            in_body(&nested(
                depth,
                "v = match a {
_ => {
",
                "",
                "yield 1
}
1 => {
yield 2
}
}
",
            ))
        };
        assert_reads(&braced(256));
        assert_eq!(codes(&braced(257)), [Code::Lc0007]);
    });
}

#[test]
fn siblings_do_not_add_up() {
    let sibling = "v = loop {\nw = loop {\nuntil a\n}\nuntil a\n}\n".repeat(1000);
    assert_reads(&in_body(&sibling));
    let many = format!("x = [{}1]", "[1],".repeat(10_000));
    assert_reads(&in_body(&many));
}

#[test]
fn the_limit_comes_from_the_limits() {
    let limits = Limits::default().with_max_nesting(2).unwrap();
    let codes_at = |text: &str| {
        let mut names = Interner::new();
        read(text.as_bytes(), &mut names, &limits).report.diagnostics().iter().map(|d| d.code()).collect::<Vec<_>>()
    };
    assert!(codes_at(&deep_loop(2)).is_empty());
    assert_eq!(codes_at(&deep_loop(3)), [Code::Lc0007]);
    assert!(codes_at(&deep_array_type(2)).is_empty());
    assert_eq!(codes_at(&deep_array_type(3)), [Code::Lc0007]);
}

/// One written unit has one limit: the `[` around it counts for nothing, as an argument or in a type.
#[test]
fn only_an_array_counts_for_its_bracket() {
    on_a_large_stack(|| {
        let unit = format!("{}V{}", "(".repeat(256), ")".repeat(256));
        assert_reads(&format!("labcraft 1\nfn f(a: f64[{unit}]) -> () {{}}\n"));
        assert_reads(&in_body(&format!("x = f([{unit}])")));
        assert_reads(&in_body(&format!("x = f([{}1{}, 2])", "[".repeat(255), "]".repeat(255))));
        assert_eq!(codes(&in_body(&format!("x = f([{}1{}, 2])", "[".repeat(256), "]".repeat(256)))), [Code::Lc0007]);
    });
}

#[test]
fn lc0007_points_at_the_level_too_deep() {
    let text = format!("labcraft 1\nfn f(a: {}f64) -> () {{}}\n", "[".repeat(257));
    on_a_large_stack(move || assert_span(&text, (2, 265), (2, 266)));
}

// The model.

fn numeral_text(reading: &Reading, numeral: Numeral) -> String {
    match numeral.kind {
        NumeralKind::Int(text) | NumeralKind::Float(text) => reading.text(text).to_owned(),
        NumeralKind::Inf => "inf".to_owned(),
        NumeralKind::Nan => "nan".to_owned(),
    }
}

#[test]
fn numbers_keep_their_text() {
    for (written, negative, text, float) in [
        ("1_000", false, "1_000", false),
        ("007", false, "007", false),
        ("1e3", false, "1e3", true),
        ("-1.5e-3", true, "1.5e-3", true),
        ("1e999", false, "1e999", true),
    ] {
        let reading = Reading::of(&in_body(&format!("x = {written}")));
        let Rhs::Operand(Operand::Number(_, numeral, None)) = value_of(&reading) else {
            panic!("{written}");
        };
        assert_eq!(numeral.negative, negative);
        assert_eq!(numeral_text(&reading, *numeral), text);
        assert_eq!(matches!(numeral.kind, NumeralKind::Float(_)), float, "{written}");
    }
    let reading = Reading::of(&in_body("x = -inf"));
    let Rhs::Operand(Operand::Number(_, numeral, None)) = value_of(&reading) else { panic!() };
    assert_eq!((numeral.negative, numeral.kind), (true, NumeralKind::Inf));
    let reading = Reading::of(&in_body("x = nan"));
    let Rhs::Operand(Operand::Number(_, numeral, None)) = value_of(&reading) else { panic!() };
    assert_eq!(numeral.kind, NumeralKind::Nan);
}

#[test]
fn a_minus_takes_no_space_and_nan_no_sign() {
    for statement in [
        "x = - 1",
        "x = -\n1",
        "x = -nan",
        "x = -",
        "x = f(- 1)",
        "x = match a {\n- 1 => yield 1\n}",
        "x = match a {\n1..- 2 => yield 1\n}",
        "x = -a",
        "x = f(-\n1)",
        "x = f(- // c\n1)",
        "x = [-\n1]",
    ] {
        assert_code(&in_body(statement), Code::Lc1001);
    }
    assert_reads(&in_body("x = f(-1)"));
}

#[test]
fn a_name_right_after_a_number_is_a_unit() {
    let reading = Reading::of(&in_body("x = 9.81 m/s^-2"));
    let Rhs::Operand(Operand::Number(_, _, Some(unit))) = value_of(&reading) else { panic!() };
    let UnitTerm::Named { name, exponent: None } = unit.first else { panic!() };
    assert_eq!(reading.text(name), "m");
    let (operator, UnitTerm::Named { name, exponent: Some(exponent) }) = &unit.rest[0] else { panic!() };
    assert_eq!((*operator, reading.text(*name), exponent.negative, reading.text(exponent.digits)), (UnitOp::Slash, "s", true, "2"));
    assert_reads(&in_body("x = 2s"));
    assert_reads(&in_body("x = 20 \u{00B0}C"));
    assert_reads(&in_body("x = 1 1/s"));
    assert_reads(&in_body("x = 1 kg\u{00B7}m"));
    assert_reads(&in_body("x = 1 kg*m/(s^2*A)"));
}

#[test]
fn a_group_in_a_unit_takes_no_exponent() {
    assert_code("labcraft 1\nfn f(a: f64[(m/s)^2]) -> () {}\n", Code::Lc1001);
}

#[test]
fn a_type_keeps_its_written_form() {
    let reading = Reading::of("labcraft 1\ntype T<U: Number, V> = f64[kg/(m*s)]\n");
    let Decl::Type(decl) = &reading.file().decl else { panic!() };
    assert_eq!(decl.generics.len(), 2);
    assert_eq!(decl.generics[0].bound.as_ref().map(|path| reading.text(path.first)), Some("Number"));
    let TypeExpr::Named(named) = &decl.ty else { panic!() };
    let unit = named.unit.as_ref().unwrap();
    assert_eq!(unit.rest.len(), 1);
    assert!(matches!(unit.rest[0].1, UnitTerm::Group(_)));
}

#[test]
fn types_of_every_form_read() {
    for ty in [
        "f64",
        "visa.session",
        "f64[V]",
        "[f64]",
        "[f64; 2]",
        "queue<T>",
        "map<a.b, [c]>",
        "struct { a: f64, b: string }",
        "enum { A, B(f64), C(f64, [string]) }",
        "fn(visa.session) -> (f64[V], error)",
        "fn(a: f64, f64[V]) -> ()",
        "fn() -> ()",
        "opaque",
    ] {
        assert_reads(&format!("labcraft 1\ntype T = {ty}\n"));
    }
    for bad in ["", "1", "(f64)", "[f64;]", "[f64; x]", "[f64; 1.5]", "f64[]", "struct", "fn", "fn()", "fn() ->", "T<>"] {
        assert!(!codes(&format!("labcraft 1\ntype T = {bad}\n")).is_empty(), "{bad}");
    }
}

#[test]
fn a_parameter_has_a_constant_for_a_default_and_may_be_variadic() {
    let reading = Reading::of("labcraft 1\nfn f(a: f64[V] = 1 mV, b: string..., c: [i32] = []) -> () {}\n");
    let params = &reading.function().signature.params;
    assert!(matches!(params[0].default, Some(Constant::Operand(Operand::Number(_, _, Some(_))))));
    assert!(params[1].variadic && params[1].default.is_none());
    assert!(matches!(&params[2].default, Some(Constant::Operand(Operand::Array(_, values))) if values.is_empty()));
    for bad in ["a: f64 = 1...", "a: f64 = b...", "a: f64... = 1", "a: f64 =", "a: E = E.A x", "a: L = L {}", "a: L = L { b }"] {
        assert_code(&format!("labcraft 1\nfn f({bad}) -> () {{}}\n"), Code::Lc1001);
    }
}

#[test]
fn a_default_reads_a_path_a_call_or_a_struct_as_a_statement_does() {
    let reading = Reading::of(
        "labcraft 1\nfn f(a: M = M.Fast, b: R = R.Value(0 V), c: L = L { low: 1 V, high: 2 V }, d: F = core.lt, e: R = g<f64>(1, after: x)) -> () {}\n",
    );
    let params = &reading.function().signature.params;
    assert!(matches!(&params[0].default, Some(Constant::Operand(Operand::Path(path))) if path.rest.len() == 1));
    assert!(matches!(&params[1].default, Some(Constant::Call(call)) if call.args.len() == 1));
    assert!(matches!(&params[2].default, Some(Constant::Struct(literal)) if literal.fields.len() == 2));
    assert!(matches!(&params[3].default, Some(Constant::Operand(Operand::Path(_)))));
    assert!(matches!(&params[4].default, Some(Constant::Call(call)) if call.type_args.len() == 1 && call.args.len() == 2), "for `check` to refuse");
    for reserved in ["a: f64 = match", "a: f64 = use", "a: R = fn.x"] {
        assert_code(&format!("labcraft 1\nfn f({reserved}) -> () {{}}\n"), Code::Lc1005);
    }
    for nested in ["a: L = L { b: g(1) }", "a: R = R.Value(g(1))", "a: R = R.Value(g<f64>(1))", "a: [R] = [g(1)]"] {
        let text = format!("labcraft 1\nfn f({nested}) -> () {{}}\n");
        assert_code(&text, Code::Lc1004);
        let mut names = Interner::new();
        let result = read(text.as_bytes(), &mut names, &Limits::default());
        assert!(result.report.diagnostics().iter().all(|diagnostic| diagnostic.fixes.is_empty()), "{nested}: no line before it to move to");
    }
}

/// R-5: only an argument can be a unit, and `[x]` can be both.
#[test]
fn a_bracket_stays_open_only_where_it_can_be_a_unit() {
    let bracket = |written: &str| {
        let reading = Reading::of(&in_body(&format!("x = f({written})")));
        let Arg::Value { value, .. } = &args_of(&reading)[0] else { panic!() };
        matches!(value, Operand::Bracket(..))
    };
    for written in ["[x]", "[V]", "[1]", "[m/s]", "[V^2]", "[1/s]", "[1^2]", "[kg/(m*s)]", "[(V)]", "[\u{00B0}C]", "unit: [V]"] {
        assert!(bracket(written), "{written}");
    }
    for written in
        ["[]", "[x, y]", "[x.y]", "[2]", "[1, 2]", "[1 V]", "[\"a\"]", "[true]", "[none]", "[-1]", "[[x]]", "[x,\ny]", "[1, x]", "values: [x, y]"]
    {
        assert!(!bracket(written), "{written}");
    }
    for elsewhere in ["x = [y]", "x = [1]"] {
        let reading = Reading::of(&in_body(elsewhere));
        assert!(matches!(value_of(&reading), Rhs::Operand(Operand::Array(..))), "{elsewhere}");
    }
    for bad in ["[x y]", "[x/]", "[x", "[1 2 3]", "[m/s, 2]", "[(m), 2]"] {
        assert!(!codes(&in_body(&format!("x = f({bad})"))).is_empty(), "{bad}");
    }
}

#[test]
fn the_values_of_a_first_element_read_in_an_array_are_kept() {
    let reading = Reading::of(&in_body("x = f([1 V, a.b, 2])"));
    let Arg::Value { value: Operand::Array(_, values), .. } = &args_of(&reading)[0] else { panic!() };
    assert_eq!(values.len(), 3);
    assert!(matches!(values[0], Operand::Number(_, _, Some(_))));
    assert!(matches!(values[1], Operand::Path(_)));
    let reading = Reading::of(&in_body("x = f([a.b, c])"));
    let Arg::Value { value: Operand::Array(_, values), .. } = &args_of(&reading)[0] else { panic!() };
    assert_eq!(values.len(), 2);
}

#[test]
fn after_is_read_where_the_syntax_expects_it() {
    let reading = Reading::of(&in_body("x = f(a, b: c, after: (#l, e, [y]))"));
    let args = args_of(&reading);
    assert_eq!(args.len(), 3);
    let Arg::After { refs, .. } = &args[2] else { panic!("{:?}", args[2]) };
    assert!(matches!(refs[0], OrderRef::Label(_)));
    assert!(matches!(refs[1], OrderRef::Value(Operand::Path(_))));
    assert!(matches!(refs[2], OrderRef::Value(Operand::Array(..))), "only an argument of its own can be a unit in brackets");
    assert_code(&in_body("x = f(after: (m/s))"), Code::Lc1001);
    assert_code(&in_body("x = f(after: ([m/s]))"), Code::Lc1001);
    for single in ["after: e", "after: #l", "after: (e)", "after: [m/s]"] {
        let reading = Reading::of(&in_body(&format!("x = f({single})")));
        let Arg::After { refs, .. } = &args_of(&reading)[0] else { panic!("{single}") };
        assert_eq!(refs.len(), 1, "{single}");
    }
}

#[test]
fn every_form_of_a_statement_reads() {
    let reading = Reading::of(&in_body("#l: a, _ = f<T, [U]>(x)"));
    let statement = statement_of(&reading);
    assert_eq!(statement.label.map(|label| reading.text(label.name)), Some("l"));
    let StatementKind::Binding { targets, value: Rhs::Call(call) } = &statement.kind else { panic!() };
    assert!(matches!(targets[0], Target::Name(..)));
    assert!(matches!(targets[1], Target::Skip(_)));
    assert_eq!(call.type_args.len(), 2);

    let reading = Reading::of(&in_body("#beep: sound.beep()"));
    assert!(matches!(statement_of(&reading).kind, StatementKind::Call(_)));

    let reading = Reading::of(&in_body("x = T { a: 1, b: c.d }"));
    assert!(matches!(value_of(&reading), Rhs::Struct(literal) if literal.fields.len() == 2));

    let reading = Reading::of(&in_body("x = Reading.Value(v)"));
    assert!(matches!(value_of(&reading), Rhs::Call(_)));

    for statement in ["x = a", "x = a.b", "x = \"s\"", "x = true", "x = false", "x = none", "x = [a, 1]", "err = e3"] {
        let reading = Reading::of(&in_body(statement));
        assert!(matches!(value_of(&reading), Rhs::Operand(_)), "{statement}");
    }
    for bad in ["x", "a.b", "a.b = 1", "x y = 1", "x = ", "= 1", "1 = x", "(x) = 1", "#a x = 1", "x = 1, 2", "x = (1)", "\"s\"", "x = a b"] {
        assert!(!codes(&in_body(bad)).is_empty(), "{bad}");
    }
}

fn arms_of(reading: &Reading) -> &[labcraft_graph::Arm] {
    match value_of(reading) {
        Rhs::Match(branching) | Rhs::Handle(branching) => &branching.arms,
        other => panic!("{other:?}"),
    }
}

#[test]
fn every_pattern_of_a_match_reads() {
    let text = in_body(
        "x = match a {\n1 => yield 1\n-1 V => yield 1\n1.5 => yield 1\n\"s\" => yield 1\ntrue => yield 1\nnone => yield 1\n[a, 1] => yield 1\n\
         0..5 => yield 1\n-5..=-1 => yield 1\nState.Init => yield 1\nReading.Value(v, w) => yield 1\nb => yield 1\n_ => yield 1\n}",
    );
    let reading = Reading::of(&text);
    let arms = arms_of(&reading);
    assert_eq!(arms.len(), 13);
    for (index, arm) in arms.iter().enumerate().take(7) {
        assert!(matches!(arm.pattern, Pattern::Literal(_)), "{index}");
    }
    let Pattern::Range { low, high, inclusive: false, .. } = arms[7].pattern else { panic!() };
    assert_eq!((low.negative, reading.text(low.digits), high.negative, reading.text(high.digits)), (false, "0", false, "5"));
    let Pattern::Range { low, high, inclusive: true, .. } = arms[8].pattern else { panic!() };
    assert_eq!((low.negative, reading.text(low.digits), high.negative, reading.text(high.digits)), (true, "5", true, "1"));
    assert!(matches!(&arms[9].pattern, Pattern::Variant { bindings, .. } if bindings.is_empty()));
    assert!(matches!(&arms[10].pattern, Pattern::Variant { bindings, .. } if bindings.len() == 2));
    assert!(matches!(arms[12].pattern, Pattern::Wildcard { binding: None, .. }));
    for bad in ["1.5..2 =>", "1..2.5 =>", "1 V..2 =>", "a..b =>", "_(e) =>", "b() =>", "b(_) =>", "b(c.d) =>", "ok(1) =>", "=>"] {
        assert!(!codes(&in_body(&format!("x = match a {{\n{bad} yield 1\n}}"))).is_empty(), "{bad}");
    }
}

#[test]
fn an_arm_of_a_match_lists_patterns() {
    let text = in_body("x = match a {\n1, _ => yield 1\n3..=5, 8, \"s\", State.Init, b => {\ny = 1\nyield y\n}\n2 => yield 1\n}");
    let reading = Reading::of(&text);
    let arms = arms_of(&reading);
    assert_eq!(arms.len(), 3);
    assert!(matches!(arms[0].pattern, Pattern::Literal(_)));
    assert!(matches!(arms[0].others[..], [Pattern::Wildcard { binding: None, .. }]));
    assert!(matches!(arms[1].pattern, Pattern::Range { .. }));
    assert!(matches!(arms[1].others[..], [Pattern::Literal(_), Pattern::Literal(_), Pattern::Variant { .. }, Pattern::Variant { .. }]));
    assert!(arms[2].others.is_empty());
    let written: Vec<_> = arms[0].patterns().map(|pattern| matches!(pattern, Pattern::Wildcard { .. })).collect();
    assert_eq!(written, [false, true], "in written order");
    assert_eq!(arms[1].patterns().count(), 5);
    assert_eq!(arms[2].patterns().count(), 1);
    for bad in ["1, =>", ", 1 =>", "1,, 2 =>", "1 2 =>", "1,\n2 =>", "1\n, 2 =>"] {
        assert!(!codes(&in_body(&format!("x = match a {{\n{bad} yield 1\n}}"))).is_empty(), "{bad}");
    }
}

#[test]
fn a_pattern_that_binds_names_stands_alone() {
    assert_code(&in_body("x = match a {\nR.V(v), 1 => yield 1\n}"), Code::Lc1001);
    assert_span(&in_body("x = match a {\nR.V(v), 1 => yield 1\n}"), (4, 1), (4, 7));
    assert_code(&in_body("x = match a {\n1, R.V(v) => yield 1\n}"), Code::Lc1001);
    assert_span(&in_body("x = match a {\n1, R.V(v) => yield 1\n}"), (4, 4), (4, 10));
    assert_code(&in_body("x = match a {\n_, R.V(v, w) => yield 1\n}"), Code::Lc1001);
}

#[test]
fn an_arm_of_a_handle_keeps_one_pattern() {
    for bad in ["ok, _ =>", "io.timeout, visa.* =>", "_, _ =>", "code 5, _ =>"] {
        assert_code(&in_body(&format!("x = handle a {{\n{bad} yield 1\n}}")), Code::Lc1001);
    }
}

#[test]
fn a_bound_that_is_no_integer_is_reported_where_it_stands() {
    assert_code(&in_body("x = match a {\n1..2.5 => yield 1\n}"), Code::Lc1001);
    assert_span(&in_body("x = match a {\n1..2.5 => yield 1\n}"), (4, 4), (4, 7));
    assert_span(&in_body("x = handle a {\ncode -1.5 => yield 1\n}"), (4, 6), (4, 10));
}

#[test]
fn every_pattern_of_a_handle_reads() {
    let text = in_body(
        "x = handle a {\nok => yield 1\nio.timeout => yield 1\nio.timeout(e) => yield 1\nvisa.* => yield 1\nvisa.a.*(e) => yield 1\n\
         code -1073807339 => yield 1\ncode 5 (e) => yield 1\n_ => yield 1\n_(e) => yield 1\ncode => yield 1\nok.a => yield 1\nok(e) => yield 1\n}",
    );
    let reading = Reading::of(&text);
    let arms = arms_of(&reading);
    assert_eq!(arms.len(), 12);
    assert!(matches!(arms[0].pattern, Pattern::Ok { .. }));
    assert!(matches!(&arms[1].pattern, Pattern::Category { wildcard: false, binding: None, path, .. } if path.rest.len() == 1));
    assert!(matches!(arms[2].pattern, Pattern::Category { binding: Some(_), .. }));
    assert!(matches!(&arms[3].pattern, Pattern::Category { wildcard: true, path, .. } if path.rest.is_empty()));
    assert!(matches!(arms[4].pattern, Pattern::Category { wildcard: true, binding: Some(_), .. }));
    let Pattern::Code { code, binding: None, .. } = arms[5].pattern else { panic!() };
    assert_eq!((code.negative, reading.text(code.digits)), (true, "1073807339"));
    assert!(matches!(arms[6].pattern, Pattern::Code { binding: Some(_), .. }));
    assert!(matches!(arms[7].pattern, Pattern::Wildcard { binding: None, .. }));
    assert!(matches!(arms[8].pattern, Pattern::Wildcard { binding: Some(_), .. }));
    assert!(matches!(arms[9].pattern, Pattern::Category { .. }), "`code` alone is a category");
    assert!(matches!(arms[10].pattern, Pattern::Category { .. }));
    assert!(matches!(arms[11].pattern, Pattern::Category { binding: Some(_), .. }));
    for bad in ["1 =>", "a.*.b =>", "code 1.5 =>", "a(b, c) =>", "a() =>", "\"s\" =>"] {
        assert!(!codes(&in_body(&format!("x = handle a {{\n{bad} yield 1\n}}"))).is_empty(), "{bad}");
    }
}

#[test]
fn a_yield_gives_one_value_or_several() {
    let reading = Reading::of(&in_body("x = match a {\n1 => yield b\n2 => yield (b)\n3 => yield (b, c)\n}"));
    let arms = arms_of(&reading);
    assert_eq!(arms.iter().map(|arm| arm.yields.values.len()).collect::<Vec<_>>(), [1, 1, 2]);
}

#[test]
fn the_clauses_of_a_for_and_of_a_loop() {
    let reading = Reading::of(&in_body("x = for parallel i in a, j in 0..n, carry c: f64[V] = 1 V, carry d = e {\n}"));
    let Rhs::For(structure) = value_of(&reading) else { panic!() };
    assert!(structure.parallel);
    assert_eq!(structure.clauses.len(), 4);
    let reading = Reading::of(&in_body("x = loop continue_on_error carry c = 1 {\nuntil a\n}"));
    let Rhs::Loop(structure) = value_of(&reading) else { panic!() };
    assert!(structure.continue_on_error);
    assert_eq!(structure.clauses.len(), 1);
    assert_reads(&in_body("x = loop {\nuntil a\n}"));
    for bad in [
        "x = for {\n}",
        "x = for i {\n}",
        "x = for i in {\n}",
        "x = for carry a = 1 {\n}",
        "x = for carry a = 1, i in a {\n}",
        "x = for i in a, carry b = 1, j in a {\n}",
        "x = loop i in a {\nuntil b\n}",
        "x = loop carry a {\nuntil b\n}",
        "x = loop carry a = 1 carry b = 2 {\nuntil c\n}",
    ] {
        assert!(!codes(&in_body(bad)).is_empty(), "{bad}");
    }
}

#[test]
fn every_item_of_the_sections_reads() {
    let reading = Reading::of(
        "labcraft 1\nfn f() -> () {}\npanel {\ncontrol a: text_box\nindicator b: numeric(format: \"0\", default: 1)\ncontrol c: d()\n}\n\
         layout {\nnode a at (1, -2)\nnode #b at (1_0, 1e3) view standard\nnode c view compact\nwire a -> #b\nwire a.x -> b.y via (1, 2) (3, 4)\n}\n\
         icon 32 {\nrect (2, 4) (30, 28) radius 4\ncircle (1, 2) 3 fill\nline (1, 2) (3, 4) accent wire.error\npath \"M0 0\" fill accent a\ntext \"V\" at (1, 2) size 6\n}\n",
    );
    let file = reading.file();
    let panel = file.panel.as_ref().unwrap();
    assert_eq!(panel.items.len(), 3);
    assert!(panel.items[0].args.is_none());
    assert_eq!(panel.items[1].args.as_ref().map(|args| args.len()), Some(2));
    assert_eq!(panel.items[2].args.as_ref().map(|args| args.len()), Some(0));
    let layout = file.layout.as_ref().unwrap();
    assert_eq!(layout.items.len(), 5);
    let LayoutKind::Node { node, at: Some(point), view: Some(_) } = layout.items[1].kind else { panic!() };
    assert!(node.is_label);
    assert_eq!(numeral_text(&reading, point.y), "1e3");
    let LayoutKind::Wire { from, via, .. } = &layout.items[4].kind else { panic!() };
    assert!(from.terminal.is_some());
    assert_eq!(via.len(), 2);
    let icon = file.icon.as_ref().unwrap();
    assert_eq!(reading.text(icon.grid), "32");
    assert_eq!(icon.items.len(), 5);
    assert!(icon.items[1].fill && icon.items[3].fill);
    assert!(icon.items[2].accent.is_some() && icon.items[3].accent.is_some());
}

#[test]
fn a_bad_item_of_a_section_is_a_syntax_error() {
    for section in [
        "panel {\na: b\n}",
        "panel {\ncontrol a\n}",
        "panel {\ncontrol a: 1\n}",
        "panel {\ncontrol a: b.c\n}",
        "layout {\nnode a\n}",
        "layout {\nnode a at (1)\n}",
        "layout {\nnode a at (1, x)\n}",
        "layout {\nnode a at (1 V, 2)\n}",
        "layout {\nnode a view other\n}",
        "layout {\nnode a view compact at (1, 2)\n}",
        "layout {\nwire a\n}",
        "layout {\nwire a -> b via\n}",
        "layout {\nx\n}",
        "icon {\n}",
        "icon x {\n}",
        "icon 32 {\nsquare (1, 2)\n}",
        "icon 32 {\nrect (1, 2)\n}",
        "icon 32 {\ncircle (1, 2)\n}",
        "icon 32 {\npath x\n}",
        "icon 32 {\ntext \"a\" (1, 2)\n}",
        "icon 32 {\nline (1, 2) (3, 4) accent fill\nline\n}",
        "icon 32 {\nline (1, 2) (3, 4) accent a fill\n}",
    ] {
        assert!(!codes(&format!("labcraft 1\nfn f() -> () {{}}\n{section}\n")).is_empty(), "{section}");
    }
}

#[test]
fn an_external_function_has_no_body() {
    let reading = Reading::of("labcraft 1\nextern rust(crate: \"a\", function: \"b\")\nfn f(a: f64) -> (err: error)\n");
    let Decl::Extern(function) = &reading.file().decl else { panic!() };
    assert_eq!(reading.text(function.language), "rust");
    assert_eq!(function.args.len(), 2);
    assert_eq!(reading.text(function.name), "f");
    assert_code("labcraft 1\nextern rust()\nfn f() -> () {}\n", Code::Lc1001);
    assert_code("labcraft 1\nextern rust() type T = f64\n", Code::Lc1001);
}

// Ids and spans.

#[test]
fn ids_give_the_place_of_what_they_name() {
    let reading = Reading::of("labcraft 1\nfn f() -> () {\n    x, _ = add(a, b: 1 V)\n}\n");
    let map = &reading.read.source_map;
    let at = |line, start, end| Some(Span { start: Position { line, column: start }, end: Position { line, column: end } });
    let statement = statement_of(&reading);
    assert_eq!(map.span(statement.id), at(3, 5, 26));
    let StatementKind::Binding { targets, value: Rhs::Call(call) } = &statement.kind else { panic!() };
    let Target::Name(first, _) = targets[0] else { panic!() };
    let Target::Skip(second) = targets[1] else { panic!() };
    assert_eq!((map.span(first), map.span(second)), (at(3, 5, 6), at(3, 8, 9)));
    assert_eq!(map.span(call.id), at(3, 12, 26));
    assert_eq!(map.span(call.callee.id), at(3, 12, 15));
    let Arg::Value { id, value: Operand::Path(path), .. } = &call.args[0] else { panic!() };
    assert_eq!((map.span(*id), map.span(path.id)), (at(3, 16, 17), at(3, 16, 17)));
    let Arg::Value { id, value: Operand::Number(number, _, Some(unit)), .. } = &call.args[1] else { panic!() };
    assert_eq!((map.span(*id), map.span(*number), map.span(unit.id)), (at(3, 19, 25), at(3, 22, 25), at(3, 24, 25)));
    assert_eq!(map.span(reading.function().id), Some(Span { start: Position { line: 2, column: 1 }, end: Position { line: 4, column: 2 } }));
}

#[test]
fn a_yield_is_placed_whole_and_a_structure_up_to_its_brace() {
    let reading = Reading::of(&in_body("x = match a {\n    1 => yield (b, c)\n}"));
    let map = &reading.read.source_map;
    let at = |line, start, end| Some(Span { start: Position { line, column: start }, end: Position { line, column: end } });
    let arm = &arms_of(&reading)[0];
    assert_eq!(map.span(arm.yields.id), at(4, 10, 22));
    assert_eq!(map.span(arm.id), at(4, 5, 22));
    let Rhs::Match(branching) = value_of(&reading) else { panic!() };
    assert_eq!(map.span(branching.id), Some(Span { start: Position { line: 3, column: 5 }, end: Position { line: 5, column: 2 } }));
}

/// Ids are all different, and a parsed file is one value: two readings of one text are equal.
#[test]
fn the_same_text_gives_the_same_model() {
    let text = "labcraft 1\n/// a\nfn f(a: f64[V] = 1 V) -> (b: f64) {\n    b = add(a, 1)\n}\n";
    let mut names = Interner::new();
    let (first, second) = (read(text.as_bytes(), &mut names, &Limits::default()), read(text.as_bytes(), &mut names, &Limits::default()));
    assert_eq!(first.file, second.file);
    assert_eq!(first.source_map, second.source_map);
    let other = read(text.replace("add", "sub").as_bytes(), &mut names, &Limits::default());
    assert_ne!(first.file, other.file);
}

#[test]
fn names_are_read_in_their_one_spelling() {
    let reading = Reading::of(&in_body("e\u{0301} = \u{00E9}"));
    let StatementKind::Binding { targets, value: Rhs::Operand(Operand::Path(path)) } = &statement_of(&reading).kind else { panic!() };
    let Target::Name(_, target) = targets[0] else { panic!() };
    assert_eq!(target, path.first);
}

#[test]
fn a_message_shows_neither_a_string_nor_an_invisible_character() {
    let reading = Reading::of(&in_body(&format!("x = \"{BIDI}secret\" \"second\"")));
    let message = &reading.read.report.diagnostics().iter().find(|d| d.code() == Code::Lc1001).unwrap().message;
    assert!(!message.contains("secret") && !message.contains("second") && !message.contains(BIDI), "{message}");
}

#[test]
fn a_huge_name_in_a_message_is_cut() {
    let name = "a".repeat(100_000);
    let reading = Reading::of(&format!("labcraft 1\n{name}\n"));
    assert!(reading.read.report.diagnostics()[0].message.chars().count() <= labcraft_types::MAX_TEXT + 1);
}
