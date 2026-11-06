//! The signatures of the primitives, as data, in the form of appendix A of the spec (`docs/architecture.md`, section 5).
//! Their bodies stay in `runtime` (M0b). Pass 3 reads what a call binds to: names, kinds, defaults, how many outputs. Types
//! and the rules of units join each signature with pass 4. A test compares this table with appendix A.

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

#[derive(Clone, Copy, Debug)]
pub(crate) struct PrimitiveParam {
    pub(crate) name: &'static str,
    pub(crate) kind: ParamKind,
    pub(crate) has_default: bool,
    pub(crate) variadic: bool,
}

const fn param(name: &'static str, kind: ParamKind, has_default: bool, variadic: bool) -> PrimitiveParam {
    PrimitiveParam { name, kind, has_default, variadic }
}

const fn value(name: &'static str) -> PrimitiveParam {
    param(name, ParamKind::Value, false, false)
}

const fn default(name: &'static str) -> PrimitiveParam {
    param(name, ParamKind::Value, true, false)
}

const fn variadic(name: &'static str) -> PrimitiveParam {
    param(name, ParamKind::Value, false, true)
}

const fn unit(name: &'static str) -> PrimitiveParam {
    param(name, ParamKind::Unit, false, false)
}

const fn category(name: &'static str) -> PrimitiveParam {
    param(name, ParamKind::Category, false, false)
}

const fn element(name: &'static str) -> PrimitiveParam {
    param(name, ParamKind::Element, false, false)
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Primitive {
    pub(crate) library: PrimitiveLibrary,
    pub(crate) name: &'static str,
    /// How many type variables it has, which a call can write: `array.len<f64>(values)`.
    pub(crate) generics: usize,
    pub(crate) params: &'static [PrimitiveParam],
    /// `None` for `core.call`, whose outputs are those of the function it runs.
    pub(crate) outputs: Option<&'static [&'static str]>,
    /// It takes more than its parameters: an input per field of a template (5.4), or those of the function it runs.
    pub(crate) open: bool,
}

const fn f(
    library: PrimitiveLibrary,
    name: &'static str,
    generics: usize,
    params: &'static [PrimitiveParam],
    outputs: &'static [&'static str],
) -> Primitive {
    Primitive { library, name, generics, params, outputs: Some(outputs), open: false }
}

const AB: &[PrimitiveParam] = &[value("a"), value("b")];
const X: &[PrimitiveParam] = &[value("x")];
const RESULT: &[&str] = &["result"];
const ERR: &[&str] = &["err"];
const VALUE_ERR: &[&str] = &["value", "err"];
const TEXT_ERR: &[PrimitiveParam] = &[value("text"), default("err")];

use PrimitiveLibrary::{Array, Core, Errors, String as Strings, Time, Ui};

pub(crate) const PRIMITIVES: &[Primitive] = &[
    f(Core, "add", 1, AB, RESULT),
    f(Core, "sub", 1, AB, RESULT),
    f(Core, "mul", 2, AB, RESULT),
    f(Core, "div", 2, AB, RESULT),
    f(Core, "abs", 1, X, RESULT),
    f(Core, "neg", 1, X, RESULT),
    f(Core, "sqrt", 1, X, RESULT),
    f(Core, "pow", 1, &[value("x"), value("exponent")], RESULT),
    f(Core, "min", 1, AB, RESULT),
    f(Core, "max", 1, AB, RESULT),
    f(Core, "lt", 1, AB, RESULT),
    f(Core, "le", 1, AB, RESULT),
    f(Core, "gt", 1, AB, RESULT),
    f(Core, "ge", 1, AB, RESULT),
    f(Core, "eq", 1, AB, RESULT),
    f(Core, "ne", 1, AB, RESULT),
    f(Core, "and", 0, AB, RESULT),
    f(Core, "or", 0, AB, RESULT),
    f(Core, "not", 0, &[value("a")], RESULT),
    f(Core, "select", 1, &[value("condition"), value("if_true"), value("if_false")], RESULT),
    f(Core, "with_unit", 0, &[value("x"), unit("unit")], RESULT),
    f(Core, "unitless", 1, &[value("x"), unit("unit")], RESULT),
    f(Core, "to_i32", 0, X, RESULT),
    f(Core, "round_to_i32", 0, X, RESULT),
    f(Core, "to_f32", 0, X, RESULT),
    Primitive { library: Core, name: "call", generics: 0, params: &[value("function")], outputs: None, open: true },
    f(Array, "len", 1, &[value("array")], &["length"]),
    f(Array, "get", 1, &[value("array"), value("index"), default("err")], &["item", "err"]),
    f(Array, "push", 1, &[value("array"), value("item")], &["array"]),
    f(Array, "sum", 1, &[value("values")], &["sum"]),
    f(Strings, "concat", 0, &[variadic("parts")], &["text"]),
    f(Strings, "join", 0, &[value("parts")], &["text"]),
    Primitive { library: Strings, name: "format", generics: 0, params: &[value("template")], outputs: Some(&["text"]), open: true },
    f(Strings, "split", 0, &[value("text"), value("separator")], &["parts"]),
    f(Strings, "to_f64", 0, TEXT_ERR, VALUE_ERR),
    f(Strings, "to_i32", 0, TEXT_ERR, VALUE_ERR),
    f(Time, "now", 0, &[], &["timestamp"]),
    f(Time, "elapsed", 0, &[value("since")], &["duration"]),
    f(Time, "wait", 0, &[value("duration"), default("err")], ERR),
    f(Errors, "make", 0, &[category("category"), default("code"), default("message")], ERR),
    f(Errors, "raise_if", 0, &[value("condition"), category("category"), default("message"), default("err")], ERR),
    f(Errors, "merge", 0, &[variadic("errs")], ERR),
    f(Ui, "get", 1, &[element("element")], &["value"]),
    f(Ui, "set", 1, &[element("element"), value("value")], &["value"]),
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
        generics: usize,
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
                let mut generics = 0;
                if rest.starts_with('<') {
                    let close = rest.find('>').unwrap();
                    generics = split_commas(&rest[1..close]).len();
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
            assert_eq!(primitive.generics, declaration.generics, "{name}");
            let params: Vec<&String> = declaration.params.iter().filter(|param| param.as_str() != "…").collect();
            let expected: Vec<String> = params.iter().map(|param| describe_written(param)).collect();
            let table: Vec<String> = primitive.params.iter().map(describe).collect();
            assert_eq!(table, expected, "{name}");
            assert_eq!(primitive.open, declaration.params.iter().any(|param| param == "…"), "{name}");
            let outputs: Vec<&str> =
                declaration.outputs.iter().filter(|output| output.as_str() != "…").map(|output| output.split(':').next().unwrap().trim()).collect();
            if declaration.outputs.iter().any(|output| output == "…") {
                assert_eq!(primitive.outputs, None, "{name}");
            } else {
                assert_eq!(primitive.outputs, Some(outputs.as_slice()), "{name}");
            }
        }
        for primitive in PRIMITIVES {
            let name = format!("{}.{}", primitive.library.as_str(), primitive.name);
            let same_as = WRITTEN_AS_ANOTHER.iter().find(|(row, _)| primitive.library == Core && *row == primitive.name);
            if let Some((_, line)) = same_as {
                let other = PRIMITIVES.iter().find(|other| other.library == Core && other.name == *line).unwrap();
                let shape = |one: &Primitive| format!("{} {:?} {:?} {}", one.generics, one.params, one.outputs, one.open);
                assert_eq!(shape(primitive), shape(other), "{name}");
            } else {
                assert!(written.iter().any(|declaration| declaration.name == name), "{name} is not in appendix A");
            }
        }
    }
}
