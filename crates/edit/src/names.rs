//! The names that the editor gives (visual notation N-14) and the renames that follow the gestures (2.4): a value renamed
//! is renamed in its line, in every reader that sees it, and in `layout`.

use std::collections::BTreeSet;

use labcraft_format::is_name;
use labcraft_graph::{Arg, Function, LayoutKind, OrderRef, Target};
use labcraft_types::Name;

use crate::model::Edit;
use crate::places::{rename_read, slots};
use crate::scope::{self, Bound, Value};
use crate::{Callable, EditError, Kind};

/// Every name that a new value or label of the function would meet: its inputs and outputs, its values and labels in
/// every body, and the libraries it uses, whose calls a value of that name would hide.
pub(crate) fn taken(edit: &Edit) -> BTreeSet<String> {
    let mut taken = BTreeSet::new();
    let Some(function) = edit.function() else { return taken };
    for param in &function.signature.params {
        taken.insert(edit.text(param.name).to_owned());
    }
    for output in &function.signature.outputs {
        taken.insert(edit.text(output.name).to_owned());
    }
    for value in scope::values(function) {
        taken.insert(edit.text(value.name).to_owned());
    }
    for (label, _) in scope::labels(function) {
        taken.insert(edit.text(label).to_owned());
    }
    for library in &edit.file.uses {
        taken.insert(edit.text(library.path.first).to_owned());
    }
    taken
}

/// `base`, or `base2`, `base3`… the first that is free (N-14).
pub(crate) fn fresh(base: &str, taken: &BTreeSet<String>) -> String {
    if !taken.contains(base) {
        return base.to_owned();
    }
    (2u64..).map(|n| format!("{base}{n}")).find(|name| !taken.contains(name)).unwrap_or_else(|| base.to_owned())
}

/// A name that the caller gives: one name of the language, free in the function.
pub(crate) fn given(text: &str, taken: &BTreeSet<String>, parameter: &'static str) -> Result<String, EditError> {
    if !is_name(text) {
        return Err(EditError::new(
            "name.invalid",
            format!("`{}` is not a name: a letter, then letters, digits and `_`, and no reserved word", crate::cut(text)),
        )
        .of(parameter));
    }
    if taken.contains(text) {
        return Err(EditError::new("name.taken", format!("`{}` is already a name of this function", crate::cut(text))).of(parameter));
    }
    Ok(text.to_owned())
}

/// `StepResult` gives `step_result`: the name of a value of a type.
pub(crate) fn of_type(type_name: &str) -> String {
    let mut name = String::with_capacity(type_name.len() + 4);
    for (index, c) in type_name.chars().enumerate() {
        if c.is_uppercase() {
            if index > 0 && !name.ends_with('_') {
                name.push('_');
            }
            name.extend(c.to_lowercase());
        } else {
            name.push(c);
        }
    }
    if is_name(&name) { name } else { format!("{name}_value") }
}

/// The name of an output that says nothing of its value: the node takes its name from its function (N-14).
const SAYS_NOTHING: &str = "result";

/// What the outputs of `core` that say nothing give: `sum`, `product` (N-14). Another function gives its own name.
const RESULTS: &[(&str, &str)] = &[("add", "sum"), ("sub", "difference"), ("mul", "product"), ("div", "quotient")];

/// The base of the name of an output of a node that calls `call` (N-14).
pub(crate) fn of_output(callable: &Callable, call: &str, index: usize) -> Option<String> {
    let output = callable.outputs.get(index)?;
    match callable.kind {
        Kind::Struct | Kind::Variant => Some(of_type(output)),
        Kind::Function if output == SAYS_NOTHING => {
            let function = call.rsplit('.').next().unwrap_or(call);
            let library_is_core = call.split('.').next() == Some("core");
            let from_table = RESULTS.iter().find(|(name, _)| library_is_core && *name == function).map(|(_, given)| (*given).to_owned());
            Some(from_table.unwrap_or_else(|| function.to_owned()))
        }
        Kind::Function => Some(output.clone()),
    }
}

/// Renames a value bound by a line: the target, every reader that sees it, and the entries of `layout` that name it.
pub(crate) fn rename(edit: &mut Edit, value: &Value, to: Name) -> Result<(), EditError> {
    let Bound::Line(loc, target) = &value.bound else {
        return Err(EditError::internal("only a value of a line is renamed"));
    };
    let from = value.name;
    let function = edit.body_function()?.clone();
    let readers: Vec<(scope::Loc, crate::places::Slot)> = scope::all(&function)
        .into_iter()
        .flat_map(|reader| {
            let statement = scope::statement(&function, &reader).cloned();
            statement
                .map(|statement| {
                    slots(&statement, &reader)
                        .into_iter()
                        .filter(|(_, seen_in)| scope::sees(&function, value, seen_in))
                        .map(|(slot, _)| (reader.clone(), slot))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
        .collect();
    let after_readers: Vec<scope::Loc> = scope::all(&function).into_iter().filter(|reader| scope::sees(&function, value, &reader.body)).collect();
    let target_function: &mut Function = edit.body_function_mut()?;
    if let Some(statement) = scope::statement_mut(target_function, loc)
        && let labcraft_graph::StatementKind::Binding { targets, .. } = &mut statement.kind
        && let Some(Target::Name(_, name)) = targets.get_mut(*target)
    {
        *name = to;
    }
    for (reader, slot) in readers {
        if let Some(operand) = scope::statement_mut(target_function, &reader).and_then(|statement| crate::places::operand_mut(statement, slot)) {
            rename_read(operand, from, to);
        }
    }
    for reader in after_readers {
        let Some(args) = scope::statement_mut(target_function, &reader).and_then(crate::places::args_mut) else { continue };
        for arg in args.iter_mut() {
            if let Arg::After { refs, .. } = arg {
                for reference in refs.iter_mut() {
                    if let OrderRef::Value(operand) = reference {
                        rename_read(operand, from, to);
                    }
                }
            }
        }
    }
    let still_named = scope::values(edit.body_function()?).iter().any(|other| other.name == from);
    if !still_named {
        rename_in_layout(edit, from, to, false);
    }
    Ok(())
}

/// The entries of `layout` follow a rename (spec 11): the `node` line, and the `wire` lines that start or end there; of a
/// label when `is_label`, of a value otherwise.
pub(crate) fn rename_in_layout(edit: &mut Edit, from: Name, to: Name, is_label: bool) {
    let Some(layout) = &mut edit.file.layout else { return };
    for item in &mut layout.items {
        match &mut item.kind {
            LayoutKind::Node { node, .. } => {
                if node.is_label == is_label && node.name == from {
                    node.name = to;
                }
            }
            LayoutKind::Wire { from: start, to: end, .. } => {
                if start.node.is_label == is_label && start.node.name == from {
                    start.node.name = to;
                }
                if end.node.is_label == is_label && end.node.name == from {
                    end.node.name = to;
                }
            }
        }
    }
}
