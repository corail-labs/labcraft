//! The signatures of the primitives, as data, in the form of appendix A of the spec (`docs/architecture.md`, section 5).
//! Their bodies stay in `runtime` (M0b). Pass 3 reads what a call binds to: names, kinds, defaults, how many outputs. Pass 4
//! reads their types and the rules of units. A test compares this table with appendix A.

/// A library of the language, which a `use` always finds and nothing replaces (`docs/cli.md`, section 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PrimitiveLibrary {
    Core,
    String,
    Array,
    Errors,
    Time,
    Ui,
}

impl PrimitiveLibrary {
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        match name {
            "core" => Some(PrimitiveLibrary::Core),
            "string" => Some(PrimitiveLibrary::String),
            "array" => Some(PrimitiveLibrary::Array),
            "errors" => Some(PrimitiveLibrary::Errors),
            "time" => Some(PrimitiveLibrary::Time),
            "ui" => Some(PrimitiveLibrary::Ui),
            _ => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            PrimitiveLibrary::Core => "core",
            PrimitiveLibrary::String => "string",
            PrimitiveLibrary::Array => "array",
            PrimitiveLibrary::Errors => "errors",
            PrimitiveLibrary::Time => "time",
            PrimitiveLibrary::Ui => "ui",
        }
    }
}

/// What a parameter takes (R-1, R-5): a kind other than `Value` shows in place of a type in appendix A.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ParamKind {
    Value,
    /// `[V]`
    Unit,
    /// `io.timeout`
    Category,
    /// A panel element, even if an input or an output has its name.
    Element,
}

/// The type of a parameter or an output, in the form of appendix A.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PrimitiveTy {
    Bool,
    I32,
    F32,
    F64,
    Text,
    Texts,
    Timestamp,
    Duration,
    Error,
    /// The type variable at this position.
    Var(usize),
    /// An array of it.
    ArrayOf(usize),
    /// `fn(…) -> (…)`
    Function,
    /// A parameter of a kind other than `Value`, which shows that kind in place of a type.
    None,
}

/// What an output is. A rule of 3.3 cannot be said by a type alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PrimitiveOutput {
    Is(PrimitiveTy),
    /// `A·B`: the exponents add.
    Product,
    /// `A/B`: the exponents subtract, and integers give an `f64` (3.4).
    Quotient,
    /// `√N`: the exponents are halved, and integers give an `f64`.
    Root,
    /// `N^exponent`, and integers give an `f64`.
    Power,
    /// `f64[unit]`
    WithUnit,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PrimitiveParam {
    pub(crate) name: &'static str,
    pub(crate) kind: ParamKind,
    pub(crate) ty: PrimitiveTy,
    pub(crate) has_default: bool,
    pub(crate) variadic: bool,
}

const fn param(name: &'static str, kind: ParamKind, ty: PrimitiveTy, has_default: bool, variadic: bool) -> PrimitiveParam {
    PrimitiveParam { name, kind, ty, has_default, variadic }
}

const fn value(name: &'static str, ty: PrimitiveTy) -> PrimitiveParam {
    param(name, ParamKind::Value, ty, false, false)
}

const fn default(name: &'static str, ty: PrimitiveTy) -> PrimitiveParam {
    param(name, ParamKind::Value, ty, true, false)
}

const fn variadic(name: &'static str, ty: PrimitiveTy) -> PrimitiveParam {
    param(name, ParamKind::Value, ty, false, true)
}

const fn unit(name: &'static str) -> PrimitiveParam {
    param(name, ParamKind::Unit, PrimitiveTy::None, false, false)
}

const fn category(name: &'static str) -> PrimitiveParam {
    param(name, ParamKind::Category, PrimitiveTy::None, false, false)
}

const fn element(name: &'static str) -> PrimitiveParam {
    param(name, ParamKind::Element, PrimitiveTy::None, false, false)
}

/// The type variables of a primitive, which a call can write: `array.len<f64>(values)`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Variables {
    pub(crate) names: &'static [&'static str],
    /// They are bound by `Number` (3.5).
    pub(crate) numbers: bool,
}

const NO_VARIABLE: Variables = Variables { names: &[], numbers: false };
const N: Variables = Variables { names: &["N"], numbers: true };
const AB_NUMBERS: Variables = Variables { names: &["A", "B"], numbers: true };
const T: Variables = Variables { names: &["T"], numbers: false };

#[derive(Clone, Copy, Debug)]
pub(crate) struct Primitive {
    pub(crate) library: PrimitiveLibrary,
    pub(crate) name: &'static str,
    pub(crate) variables: Variables,
    pub(crate) params: &'static [PrimitiveParam],
    /// `None` for `core.call`, whose outputs are those of the function it runs.
    pub(crate) outputs: Option<&'static [&'static str]>,
    /// One per output.
    pub(crate) results: &'static [PrimitiveOutput],
    /// It takes more than its parameters: an input per field of a template (5.4), or those of the function it runs.
    pub(crate) open: bool,
}

const fn f(
    library: PrimitiveLibrary,
    name: &'static str,
    variables: Variables,
    params: &'static [PrimitiveParam],
    outputs: &'static [&'static str],
    results: &'static [PrimitiveOutput],
) -> Primitive {
    Primitive { library, name, variables, params, outputs: Some(outputs), results, open: false }
}

use PrimitiveLibrary::{Array, Core, Errors, String as Strings, Time, Ui};
use PrimitiveOutput::{Is, Power, Product, Quotient, Root, WithUnit};
use PrimitiveTy::{ArrayOf, Bool, Duration, Error, F32, F64, Function, I32, Text, Texts, Timestamp, Var};

const V0: PrimitiveTy = Var(0);

const SAME_TWICE: &[PrimitiveParam] = &[value("a", V0), value("b", V0)];
const TWO_BOOLS: &[PrimitiveParam] = &[value("a", Bool), value("b", Bool)];
const ONE_VALUE: &[PrimitiveParam] = &[value("x", V0)];
const ONE_F64: &[PrimitiveParam] = &[value("x", F64)];
const RESULT: &[&str] = &["result"];
const ERR: &[&str] = &["err"];
const VALUE_ERR: &[&str] = &["value", "err"];
const TEXT_ERR: &[PrimitiveParam] = &[value("text", Text), default("err", Error)];
const GIVES_V0: &[PrimitiveOutput] = &[Is(V0)];
const GIVES_BOOL: &[PrimitiveOutput] = &[Is(Bool)];
const GIVES_F64: &[PrimitiveOutput] = &[Is(F64)];
const GIVES_I32: &[PrimitiveOutput] = &[Is(I32)];
const GIVES_ERROR: &[PrimitiveOutput] = &[Is(Error)];
const GIVES_TEXT: &[PrimitiveOutput] = &[Is(Text)];

pub(crate) const PRIMITIVES: &[Primitive] = &[
    f(Core, "add", N, SAME_TWICE, RESULT, GIVES_V0),
    f(Core, "sub", N, SAME_TWICE, RESULT, GIVES_V0),
    f(Core, "mul", AB_NUMBERS, &[value("a", V0), value("b", Var(1))], RESULT, &[Product]),
    f(Core, "div", AB_NUMBERS, &[value("a", V0), value("b", Var(1))], RESULT, &[Quotient]),
    f(Core, "abs", N, ONE_VALUE, RESULT, GIVES_V0),
    f(Core, "neg", N, ONE_VALUE, RESULT, GIVES_V0),
    f(Core, "sqrt", N, ONE_VALUE, RESULT, &[Root]),
    f(Core, "pow", N, &[value("x", V0), value("exponent", I32)], RESULT, &[Power]),
    f(Core, "min", N, SAME_TWICE, RESULT, GIVES_V0),
    f(Core, "max", N, SAME_TWICE, RESULT, GIVES_V0),
    f(Core, "lt", N, SAME_TWICE, RESULT, GIVES_BOOL),
    f(Core, "le", N, SAME_TWICE, RESULT, GIVES_BOOL),
    f(Core, "gt", N, SAME_TWICE, RESULT, GIVES_BOOL),
    f(Core, "ge", N, SAME_TWICE, RESULT, GIVES_BOOL),
    f(Core, "eq", T, SAME_TWICE, RESULT, GIVES_BOOL),
    f(Core, "ne", T, SAME_TWICE, RESULT, GIVES_BOOL),
    f(Core, "and", NO_VARIABLE, TWO_BOOLS, RESULT, GIVES_BOOL),
    f(Core, "or", NO_VARIABLE, TWO_BOOLS, RESULT, GIVES_BOOL),
    f(Core, "not", NO_VARIABLE, &[value("a", Bool)], RESULT, GIVES_BOOL),
    f(Core, "select", T, &[value("condition", Bool), value("if_true", V0), value("if_false", V0)], RESULT, GIVES_V0),
    f(Core, "with_unit", NO_VARIABLE, &[value("x", F64), unit("unit")], RESULT, &[WithUnit]),
    f(Core, "unitless", N, &[value("x", V0), unit("unit")], RESULT, GIVES_F64),
    f(Core, "to_i32", NO_VARIABLE, ONE_F64, RESULT, GIVES_I32),
    f(Core, "round_to_i32", NO_VARIABLE, ONE_F64, RESULT, GIVES_I32),
    f(Core, "to_f32", NO_VARIABLE, ONE_F64, RESULT, &[Is(F32)]),
    Primitive {
        library: Core,
        name: "call",
        variables: NO_VARIABLE,
        params: &[value("function", Function)],
        outputs: None,
        results: &[],
        open: true,
    },
    f(Array, "len", T, &[value("array", ArrayOf(0))], &["length"], GIVES_I32),
    f(Array, "get", T, &[value("array", ArrayOf(0)), value("index", I32), default("err", Error)], &["item", "err"], &[Is(V0), Is(Error)]),
    f(Array, "push", T, &[value("array", ArrayOf(0)), value("item", V0)], &["array"], &[Is(ArrayOf(0))]),
    f(Array, "sum", N, &[value("values", ArrayOf(0))], &["sum"], GIVES_V0),
    f(Strings, "concat", NO_VARIABLE, &[variadic("parts", Text)], &["text"], GIVES_TEXT),
    f(Strings, "join", NO_VARIABLE, &[value("parts", Texts)], &["text"], GIVES_TEXT),
    Primitive {
        library: Strings,
        name: "format",
        variables: NO_VARIABLE,
        params: &[value("template", Text)],
        outputs: Some(&["text"]),
        results: GIVES_TEXT,
        open: true,
    },
    f(Strings, "split", NO_VARIABLE, &[value("text", Text), value("separator", Text)], &["parts"], &[Is(Texts)]),
    f(Strings, "to_f64", NO_VARIABLE, TEXT_ERR, VALUE_ERR, &[Is(F64), Is(Error)]),
    f(Strings, "to_i32", NO_VARIABLE, TEXT_ERR, VALUE_ERR, &[Is(I32), Is(Error)]),
    f(Time, "now", NO_VARIABLE, &[], &["timestamp"], &[Is(Timestamp)]),
    f(Time, "elapsed", NO_VARIABLE, &[value("since", Timestamp)], &["duration"], &[Is(Duration)]),
    f(Time, "wait", NO_VARIABLE, &[value("duration", Duration), default("err", Error)], ERR, GIVES_ERROR),
    f(Errors, "make", NO_VARIABLE, &[category("category"), default("code", I32), default("message", Text)], ERR, GIVES_ERROR),
    f(
        Errors,
        "raise_if",
        NO_VARIABLE,
        &[value("condition", Bool), category("category"), default("message", Text), default("err", Error)],
        ERR,
        GIVES_ERROR,
    ),
    f(Errors, "merge", NO_VARIABLE, &[variadic("errs", Error)], ERR, GIVES_ERROR),
    f(Ui, "get", T, &[element("element")], &["value"], GIVES_V0),
    f(Ui, "set", T, &[element("element"), value("value", V0)], &["value"], GIVES_V0),
];

#[cfg(test)]
#[allow(clippy::string_slice)]
mod tests {
    use super::*;

    /// What appendix A says "same for": they have no line of their own, and the same signature as the line that says it.
    const WRITTEN_AS_ANOTHER: [(&str, &str); 5] = [("le", "lt"), ("gt", "lt"), ("ge", "lt"), ("ne", "eq"), ("or", "and")];

    const SPEC: &str = include_str!("../../../docs/language-spec-v0.md");

    /// The commas that are not inside brackets of any kind: `fn(…) -> (…)` and `a: [T]` are one parameter each.
    fn split_commas(text: &str) -> Vec<&str> {
        let (mut parts, mut depth, mut start, mut before) = (Vec::new(), 0, 0, ' ');
        for (index, c) in text.char_indices() {
            match c {
                '(' | '[' | '<' => depth += 1,
                // The `>` of `->` closes nothing.
                '>' if before == '-' => {}
                ')' | ']' | '>' => depth -= 1,
                ',' if depth == 0 => {
                    parts.push(text[start..index].trim());
                    start = index + 1;
                }
                _ => {}
            }
            before = c;
        }
        parts.push(text[start..].trim());
        parts.retain(|part| !part.is_empty());
        parts
    }

    /// The text after the `(` that starts `text`, up to its `)`, and what follows.
    fn inside_parentheses(text: &str) -> (&str, &str) {
        let mut depth = 0;
        for (index, c) in text.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return (&text[1..index], &text[index + 1..]);
                    }
                }
                _ => {}
            }
        }
        panic!("no closing parenthesis in {text}");
    }

    struct Written {
        name: String,
        generics: Vec<String>,
        params: Vec<String>,
        outputs: Vec<String>,
    }

    /// The declarations of A.1 and A.2, which can run over several lines.
    fn appendix() -> Vec<Written> {
        let (_, after) = SPEC.split_once("## Appendix A. Library signatures").unwrap();
        let (section, _) = after.split_once("### A.3").unwrap();
        let mut found = Vec::new();
        for block in section.split("```text").skip(1) {
            let (block, _) = block.split_once("```").unwrap();
            let lines: Vec<&str> = block.lines().map(|line| line.split("//").next().unwrap().trim()).collect();
            let joined = lines.join(" ");
            for declaration in joined.split("fn ").filter(|text| text.contains(".") && !text.trim().is_empty()) {
                let declaration = declaration.trim();
                let Some(open) = declaration.find(['<', '(']) else { continue };
                let name = declaration[..open].to_owned();
                let mut rest = &declaration[open..];
                let mut generics = Vec::new();
                if rest.starts_with('<') {
                    let close = rest.find('>').unwrap();
                    generics = split_commas(&rest[1..close]).into_iter().map(str::to_owned).collect();
                    rest = &rest[close + 1..];
                }
                let (params, after_params) = inside_parentheses(rest);
                let (_, outputs) = after_params.split_once("->").unwrap();
                let (outputs, _) = inside_parentheses(outputs.trim());
                found.push(Written {
                    name,
                    generics,
                    params: split_commas(params).into_iter().map(str::to_owned).collect(),
                    outputs: split_commas(outputs).into_iter().map(str::to_owned).collect(),
                });
            }
        }
        found
    }

    fn describe(param: &PrimitiveParam) -> String {
        let kind = match param.kind {
            ParamKind::Value => "",
            ParamKind::Unit => " kind unit",
            ParamKind::Category => " kind category",
            ParamKind::Element => " kind element",
        };
        format!("{}{kind}{}{}", param.name, if param.has_default { " =" } else { "" }, if param.variadic { " ..." } else { "" })
    }

    fn type_text(ty: PrimitiveTy, variables: &[&str]) -> String {
        let variable = |index: usize| variables.get(index).copied().unwrap_or("?").to_owned();
        match ty {
            Bool => "bool".to_owned(),
            I32 => "i32".to_owned(),
            F32 => "f32".to_owned(),
            F64 => "f64".to_owned(),
            Text => "string".to_owned(),
            Texts => "[string]".to_owned(),
            Timestamp => "timestamp".to_owned(),
            Duration => "duration".to_owned(),
            Error => "error".to_owned(),
            Var(index) => variable(index),
            ArrayOf(index) => format!("[{}]", variable(index)),
            Function => "fn(…) -> (…)".to_owned(),
            PrimitiveTy::None => String::new(),
        }
    }

    fn output_text(output: PrimitiveOutput, variables: &[&str]) -> String {
        match output {
            Is(ty) => type_text(ty, variables),
            Product => "A·B".to_owned(),
            Quotient => "A/B".to_owned(),
            Root => "√N".to_owned(),
            Power => "N^exponent".to_owned(),
            WithUnit => "f64[unit]".to_owned(),
        }
    }

    /// `a: N`, `err: error = none`, `parts: string...`: the type alone.
    fn written_type(param: &str) -> String {
        let (_, ty) = param.split_once(':').unwrap_or((param, ""));
        let ty = ty.trim();
        if ty.starts_with("kind ") {
            return String::new();
        }
        let ty = ty.split(" =").next().unwrap().trim();
        ty.strip_suffix("...").unwrap_or(ty).to_owned()
    }

    fn describe_written(param: &str) -> String {
        let (name, ty) = param.split_once(':').unwrap_or((param, ""));
        let ty = ty.trim();
        let kind = ty.strip_prefix("kind ").map_or(String::new(), |rest| format!(" kind {}", rest.split_whitespace().next().unwrap()));
        format!("{}{kind}{}{}", name.trim(), if ty.contains('=') { " =" } else { "" }, if ty.ends_with("...") { " ..." } else { "" })
    }

    #[test]
    fn the_table_says_what_appendix_a_says() {
        let written = appendix();
        assert!(written.len() >= 30, "{}", written.len());
        for declaration in &written {
            let primitive = PRIMITIVES
                .iter()
                .find(|primitive| format!("{}.{}", primitive.library.as_str(), primitive.name) == declaration.name)
                .unwrap_or_else(|| panic!("{} is not in the table", declaration.name));
            let name = &declaration.name;
            let variables = primitive.variables;
            let generics: Vec<String> = variables
                .names
                .iter()
                .map(|variable| if variables.numbers { format!("{variable}: Number") } else { (*variable).to_owned() })
                .collect();
            assert_eq!(generics, declaration.generics, "{name}");
            let params: Vec<&String> = declaration.params.iter().filter(|param| param.as_str() != "…").collect();
            let expected: Vec<String> = params.iter().map(|param| describe_written(param)).collect();
            let table: Vec<String> = primitive.params.iter().map(describe).collect();
            assert_eq!(table, expected, "{name}");
            let written: Vec<String> = params.iter().map(|param| written_type(param)).collect();
            let types: Vec<String> = primitive.params.iter().map(|param| type_text(param.ty, variables.names)).collect();
            assert_eq!(types, written, "{name}");
            assert_eq!(primitive.open, declaration.params.iter().any(|param| param == "…"), "{name}");
            let outputs: Vec<&str> =
                declaration.outputs.iter().filter(|output| output.as_str() != "…").map(|output| output.split(':').next().unwrap().trim()).collect();
            if declaration.outputs.iter().any(|output| output == "…") {
                assert_eq!(primitive.outputs, None, "{name}");
            } else {
                assert_eq!(primitive.outputs, Some(outputs.as_slice()), "{name}");
                let written: Vec<String> =
                    declaration.outputs.iter().map(|output| output.split_once(':').map_or(String::new(), |(_, ty)| ty.trim().to_owned())).collect();
                let types: Vec<String> = primitive.results.iter().map(|output| output_text(*output, variables.names)).collect();
                assert_eq!(types, written, "{name}");
            }
        }
        for primitive in PRIMITIVES {
            let name = format!("{}.{}", primitive.library.as_str(), primitive.name);
            let same_as = WRITTEN_AS_ANOTHER.iter().find(|(row, _)| primitive.library == Core && *row == primitive.name);
            if let Some((_, line)) = same_as {
                let other = PRIMITIVES.iter().find(|other| other.library == Core && other.name == *line).unwrap();
                let shape = |one: &Primitive| format!("{:?} {:?} {:?} {:?} {}", one.variables, one.params, one.outputs, one.results, one.open);
                assert_eq!(shape(primitive), shape(other), "{name}");
            } else {
                assert!(written.iter().any(|declaration| declaration.name == name), "{name} is not in appendix A");
            }
        }
    }
}
