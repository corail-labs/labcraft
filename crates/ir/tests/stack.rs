//! Lowering at the ceiling of nesting (`docs/architecture.md`, sections 4 and 10.2): programs that `check` accepts, nested as
//! deep as `MAX_NESTING` lets a file nest, lowered, verified and printed on the stack of a host. Bodies are walked with
//! explicit work lists; constants and types recurse, as `check` does, within that stack.

use std::borrow::Cow;

use labcraft_check::{Content, Found, Libraries, LibrarySource, Origin, SourceFile, check_for_run};
use labcraft_format::STACK_SIZE;
use labcraft_types::{Interner, Limits, MAX_NESTING};

struct Nothing;

impl Libraries for Nothing {
    fn find(&self, _: &str, _: Origin) -> Found {
        Found::NotFound
    }
}

fn nested(depth: usize, open: &str, innermost: &str, close: &str) -> String {
    format!("{}{innermost}{}", open.repeat(depth), close.repeat(depth))
}

/// Loops inside loops, each with a carry, each reading `a` from the function: every level passes it in.
fn deep_loops(depth: usize) -> String {
    let body = nested(depth - 1, "v = loop carry c = 0 {\n", "v = loop carry c = 0 {\nnext c = 1\nuntil a\n}\n", "next c = v\nuntil a\n}\n");
    format!("labcraft 1\nfn f(a: bool) -> (v: i32) {{\n{body}}}\n")
}

/// A `match` in the arm of a `match`.
fn deep_matches(depth: usize) -> String {
    let body = nested(depth - 1, "v = match a {\n_ => {\n", "v = match a {\n_ => yield 1\n}\n", "yield v\n}\n}\n");
    format!("labcraft 1\nfn f(a: bool) -> (v: i32) {{\n{body}}}\n")
}

/// An array constant of that many levels, bound to an output of that type.
fn deep_constant(depth: usize) -> String {
    let ty = format!("{}i32{}", "[".repeat(depth), "]".repeat(depth));
    let value = format!("{}1{}", "[".repeat(depth), "]".repeat(depth));
    format!("labcraft 1\nfn f() -> (v: {ty}) {{\nv = {value}\n}}\n")
}

fn lower_at_the_ceiling(text: String) {
    let mut limits = Limits::default().with_max_nesting(MAX_NESTING).unwrap();
    limits.max_file_size = u64::MAX;
    let mut names = Interner::new();
    let file = SourceFile { shown: "f.lcf".to_owned(), content: Content::Bytes(Cow::Owned(text.into_bytes())) };
    let root = LibrarySource { files: vec![file], ..LibrarySource::new(String::new(), String::new(), Origin::Project) };
    let checked = check_for_run(root, &Nothing, &mut names, &limits);
    assert_eq!(checked.report.errors(), 0, "{:?}", checked.report.diagnostics().first());
    let program = labcraft_ir::lower(&checked, 0, &mut names).unwrap();
    assert_eq!(labcraft_ir::verify(&program), Vec::new());
    assert!(labcraft_ir::print(&program, &names).lines().count() > 1);
}

#[test]
fn a_structure_nested_to_the_ceiling_lowers_on_the_stack_of_a_host() {
    let depth = MAX_NESTING as usize;
    for (kind, program) in [("loops", deep_loops as fn(usize) -> String), ("matches", deep_matches), ("constant", deep_constant)] {
        let text = program(depth);
        let result = std::thread::Builder::new().stack_size(STACK_SIZE).spawn(move || lower_at_the_ceiling(text)).unwrap().join();
        assert!(result.is_ok(), "{kind}");
    }
}
