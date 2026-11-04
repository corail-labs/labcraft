//! Programs nested to a given depth, one kind of nesting each (spec 14.1). The tests of `format` and the
//! harness of the corpora read the same ones (`tests/acceptance/README.md`, family Nesting).

pub fn in_body(statements: &str) -> String {
    format!("labcraft 1\nfn f() -> () {{\n{statements}\n}}\n")
}

pub fn nested(depth: usize, open: &str, innermost: &str, close: &str) -> String {
    format!("{}{innermost}{}", open.repeat(depth), close.repeat(depth))
}

fn deep_match(depth: usize) -> String {
    in_body(&nested(depth - 1, "v = match a {\n_ => {\n", "v = match a {\n_ => yield 1\n}\n", "yield v\n}\n}\n"))
}

fn deep_handle(depth: usize) -> String {
    in_body(&nested(depth - 1, "v = handle a {\n_ => {\n", "v = handle a {\n_ => yield 1\n}\n", "yield v\n}\n}\n"))
}

pub fn deep_loop(depth: usize) -> String {
    in_body(&nested(depth, "v = loop {\n", "", "until a\n}\n"))
}

fn deep_for(depth: usize) -> String {
    in_body(&nested(depth, "v = for i in a {\n", "", "}\n"))
}

fn deep_unit(depth: usize) -> String {
    format!("labcraft 1\nfn f(a: f64[{}V{}]) -> () {{}}\n", "(".repeat(depth), ")".repeat(depth))
}

fn deep_unit_after_a_number(depth: usize) -> String {
    in_body(&format!("x = 1 {}V{}", "(".repeat(depth), ")".repeat(depth)))
}

pub fn deep_array_type(depth: usize) -> String {
    format!("labcraft 1\nfn f(a: {}f64{}) -> () {{}}\n", "[".repeat(depth), "]".repeat(depth))
}

fn deep_array_literal(depth: usize) -> String {
    in_body(&format!("x = {}1{}", "[".repeat(depth), "]".repeat(depth)))
}

fn deep_bracket_argument(depth: usize) -> String {
    in_body(&format!("x = f({}1{})", "[".repeat(depth), "]".repeat(depth)))
}

fn deep_struct(depth: usize) -> String {
    format!("labcraft 1\ntype T = {}\n", nested(depth, "struct { a: ", "f64", " }"))
}

fn deep_enum(depth: usize) -> String {
    format!("labcraft 1\ntype T = {}\n", nested(depth, "enum { A(", "f64", ") }"))
}

fn deep_fn_type(depth: usize) -> String {
    format!("labcraft 1\ntype T = {}\n", nested(depth, "fn(", "f64", ") -> ()"))
}

fn deep_type_arguments(depth: usize) -> String {
    format!("labcraft 1\ntype T = {}\n", nested(depth, "T<", "f64", ">"))
}

pub type Generator = fn(usize) -> String;

/// Every kind of nesting. Each one gives a program that reads at the limit and `LC0007` one level above it.
pub const DEEP: [(&str, Generator); 13] = [
    ("match", deep_match),
    ("handle", deep_handle),
    ("loop", deep_loop),
    ("for", deep_for),
    ("unit", deep_unit),
    ("unit after a number", deep_unit_after_a_number),
    ("array type", deep_array_type),
    ("array literal", deep_array_literal),
    ("bracket argument", deep_bracket_argument),
    ("struct", deep_struct),
    ("enum", deep_enum),
    ("fn type", deep_fn_type),
    ("type arguments", deep_type_arguments),
];
