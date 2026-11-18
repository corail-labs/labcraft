//! `node.add` and `constant.add` (commands.md 6.7): a new line in the body of the function, named at once (N-14), with
//! its required inputs unwired (`LC2101`, 9.1), and an entry of `layout` when it is given a place.

use labcraft_format::{is_name, read};
use labcraft_graph::{
    Arg, Attached, Call, FieldInit, Label, Layout, LayoutItem, LayoutKind, NodeRef, Numeral, NumeralKind, Operand, Path, Rhs, Statement,
    StatementKind, StructLit, Target, Use,
};
use labcraft_types::Name;

use crate::model::Edit;
use crate::names::{fresh, given, of_output, taken};
use crate::wire::FORMAT;
use crate::{EditError, Edited, Judge, Kind, Lookup, MAX_COORDINATE, Point};

pub(crate) fn node(
    edit: &mut Edit,
    judge: &dyn Judge,
    call: &str,
    at: Option<Point>,
    name: Option<&str>,
    edited: &mut Edited,
) -> Result<(), EditError> {
    let segments: Vec<&str> = call.split('.').collect();
    if call.is_empty() || !segments.iter().all(|segment| is_name(segment)) {
        return Err(EditError::invalid("call", format!("`{call}` is not the name of a function, a struct type or a variant: names joined by `.`")));
    }
    let callable = match judge.callable(&edit.path, call) {
        Lookup::Found(callable) => callable,
        Lookup::Unknown(nearest) => {
            return Err(EditError::new("edit.unknown", format!("`{call}` names no function, struct type or variant with values here"))
                .of("call")
                .expecting(nearest));
        }
    };
    let taken = taken(edit);
    let path = path_of(edit, &segments);
    let id = edit.id();
    let mut statement = Statement {
        id,
        comments: Attached::default(),
        label: None,
        kind: StatementKind::Call(Box::new(Call { id, callee: path.clone(), type_args: Box::new([]), args: Box::new([]) })),
    };
    let named = match callable.kind {
        Kind::Function => {
            let mut args = Vec::new();
            if call == FORMAT {
                let id = edit.id();
                args.push(Arg::Value { id, name: None, value: Operand::Text(id, "".into()) });
            }
            let call_node = Box::new(Call { id, callee: path, type_args: Box::new([]), args: args.into_boxed_slice() });
            if callable.outputs.is_empty() {
                // A node with no output is named by a label (N-14).
                let label = match name {
                    Some(text) => given(text, &taken, "name")?,
                    None => fresh(segments.last().copied().unwrap_or("node"), &taken),
                };
                let label_name = edit.intern(&label);
                let label_id = edit.id();
                statement.label = Some(Label { id: label_id, name: label_name });
                statement.kind = StatementKind::Call(call_node);
                format!("#{label}")
            } else {
                let first = match name {
                    Some(text) => given(text, &taken, "name")?,
                    None => fresh(&of_output(&callable, call, 0).unwrap_or_else(|| "node".to_owned()), &taken),
                };
                let mut targets = Vec::with_capacity(callable.outputs.len());
                let first_name = edit.intern(&first);
                targets.push(Target::Name(edit.id(), first_name));
                for _ in 1..callable.outputs.len() {
                    targets.push(Target::Skip(edit.id()));
                }
                statement.kind = StatementKind::Binding { targets: targets.into_boxed_slice(), value: Rhs::Call(call_node) };
                first
            }
        }
        Kind::Struct | Kind::Variant => {
            let first = match name {
                Some(text) => given(text, &taken, "name")?,
                None => fresh(&of_output(&callable, call, 0).unwrap_or_else(|| "value".to_owned()), &taken),
            };
            let value = if callable.kind == Kind::Struct {
                let mut fields = Vec::with_capacity(callable.inputs.len());
                for input in &callable.inputs {
                    let field = edit.intern(&input.name);
                    let id = edit.id();
                    fields.push(FieldInit { id, name: field, value: Operand::Hole(id) });
                }
                Rhs::Struct(Box::new(StructLit { id, path, fields }))
            } else {
                // Each value of a variant gets a hole: the text cannot leave one out (9.1).
                let mut args = Vec::with_capacity(callable.inputs.len());
                for _ in &callable.inputs {
                    let id = edit.id();
                    args.push(Arg::Value { id, name: None, value: Operand::Hole(id) });
                }
                Rhs::Call(Box::new(Call { id, callee: path, type_args: Box::new([]), args: args.into_boxed_slice() }))
            };
            let first_name = edit.intern(&first);
            statement.kind = StatementKind::Binding { targets: Box::new([Target::Name(edit.id(), first_name)]), value };
            first
        }
    };
    if let Some(library) = &callable.library {
        add_use(edit, library);
    }
    edit.body_function_mut()?.body.statements.push(statement);
    if let Some(at) = at {
        place(edit, &named, at)?;
    }
    edited.names.insert("node", named);
    Ok(())
}

pub(crate) fn constant(edit: &mut Edit, value: &str, at: Option<Point>, name: Option<&str>, edited: &mut Edited) -> Result<(), EditError> {
    let operand = read_constant(edit, value)?;
    let taken = taken(edit);
    let named = match name {
        Some(text) => given(text, &taken, "name")?,
        None => fresh("constant", &taken),
    };
    let target = edit.intern(&named);
    let id = edit.id();
    let statement = Statement {
        id,
        comments: Attached::default(),
        label: None,
        kind: StatementKind::Binding { targets: Box::new([Target::Name(edit.id(), target)]), value: Rhs::Operand(operand) },
    };
    edit.body_function_mut()?.body.statements.push(statement);
    if let Some(at) = at {
        place(edit, &named, at)?;
    }
    edited.names.insert("node", named);
    Ok(())
}

/// A constant written as in a file, read by the grammar (commands.md 9.2): a literal, an array of them, or a variant
/// without values. A bare name is a value, not a constant.
pub(crate) fn read_constant(edit: &mut Edit, value: &str) -> Result<Operand, EditError> {
    let text = format!("labcraft 1\n\nfn f() -> () {{\n    x = {value}\n}}\n");
    let limits = edit.limits().clone();
    let reading = read(text.as_bytes(), &mut edit.names, &limits);
    let shown = crate::cut(value);
    let not_one = || EditError::invalid("value", format!("`{shown}` is not one constant of the language"));
    let Some(file) = reading.file else {
        let why = reading.report.diagnostics().first().map(|diagnostic| diagnostic.message.clone()).unwrap_or_default();
        return Err(EditError::invalid("value", format!("`{shown}` does not read: {why}")));
    };
    let labcraft_graph::Decl::Function(function) = file.decl else { return Err(not_one()) };
    let [statement] = function.body.statements.as_slice() else { return Err(not_one()) };
    if !function.body.end.is_empty() || statement.comments != Attached::default() || statement.label.is_some() {
        return Err(not_one());
    }
    let StatementKind::Binding { value: Rhs::Operand(operand), targets } = &statement.kind else {
        return Err(EditError::invalid("value", format!("`{shown}` is not a constant: a struct or a variant with values is a node (`node.add`)")));
    };
    if targets.len() != 1 {
        return Err(not_one());
    }
    if !is_constant(operand) {
        return Err(EditError::invalid(
            "value",
            format!("`{shown}` is a value, not a constant: a literal, an array of them, or a variant such as `Verdict.Pass`"),
        ));
    }
    Ok(operand.clone())
}

fn is_constant(operand: &Operand) -> bool {
    match operand {
        Operand::Path(path) => !path.rest.is_empty(),
        Operand::Array(_, values) => values.iter().all(is_constant),
        Operand::Hole(_) => false,
        Operand::Number(..) | Operand::Text(..) | Operand::Bool(..) | Operand::None(_) | Operand::Bracket(..) => true,
    }
}

fn path_of(edit: &mut Edit, segments: &[&str]) -> Path {
    let id = edit.id();
    let mut names: Vec<Name> = segments.iter().map(|segment| edit.intern(segment)).collect();
    let first = if names.is_empty() { edit.intern("node") } else { names.remove(0) };
    Path { id, first, rest: names.into_boxed_slice() }
}

/// The `use` line that a call of another library needs.
fn add_use(edit: &mut Edit, library: &str) {
    let name = edit.intern(library);
    if edit.file.uses.iter().any(|used| used.path.first == name && used.path.rest.is_empty()) {
        return;
    }
    let id = edit.id();
    let path_id = edit.id();
    edit.file.uses.push(Use { id, path: Path { id: path_id, first: name, rest: Box::new([]) }, line: None });
}

/// A coordinate as `layout` writes it.
pub(crate) fn coordinate(edit: &mut Edit, value: i64) -> Numeral {
    Numeral { negative: value < 0, kind: NumeralKind::Int(edit.intern(&value.unsigned_abs().to_string())) }
}

/// A point of `layout` within the bounds of a coordinate (spec 11), or `params.invalid` of `parameter`.
pub(crate) fn in_range(at: Point, parameter: &'static str) -> Result<(), EditError> {
    let range = -MAX_COORDINATE..=MAX_COORDINATE;
    if !range.contains(&at.x) || !range.contains(&at.y) {
        return Err(EditError::invalid(parameter, format!("a coordinate is between -{MAX_COORDINATE} and {MAX_COORDINATE}")));
    }
    Ok(())
}

/// Fixes a node where it is given: its `node … at` entry (spec 11).
pub(crate) fn place(edit: &mut Edit, named: &str, at: Point) -> Result<(), EditError> {
    in_range(at, "at")?;
    let (is_label, text) = match named.strip_prefix('#') {
        Some(label) => (true, label),
        None => (false, named),
    };
    let node = NodeRef { id: edit.id(), is_label, name: edit.intern(text) };
    let point = labcraft_graph::Point { id: edit.id(), x: coordinate(edit, at.x), y: coordinate(edit, at.y) };
    let item = LayoutItem { id: edit.id(), comments: Attached::default(), kind: LayoutKind::Node { node, at: Some(point), view: None } };
    match &mut edit.file.layout {
        Some(layout) => layout.items.push(item),
        None => {
            let id = edit.id();
            edit.file.layout = Some(Layout { id, comments: Attached::default(), items: vec![item], end: Vec::new() });
        }
    }
    Ok(())
}
