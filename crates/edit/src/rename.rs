//! The names that a user writes in a field in place (commands.md 6.7, visual notation 2.6): `value.rename`, the name of a
//! value or a label everywhere in the function; `node.set_label`, the one command that names a node by its line; and
//! `node.set_template`, the template of a `string.format`, whose fields are its other inputs.

use std::collections::BTreeSet;

use labcraft_graph::{Arg, Label, LayoutKind, Operand, OrderRef};
use labcraft_types::Name;

use crate::add::read_constant;
use crate::model::Edit;
use crate::names::{given, rename, taken};
use crate::refs::{NodeName, find_nodes, nearest, node};
use crate::scope::{self, Bound};
use crate::wire::{forget_wire, format_call, statement_node, step_five, template_of};
use crate::{EditError, Edited, Judge, cut};

/// `value.rename`.
pub(crate) fn value(edit: &mut Edit, value: &str, name: &str, edited: &mut Edited) -> Result<(), EditError> {
    let function = edit.body_function()?.clone();
    if let Some(label) = value.strip_prefix('#') {
        let new = name.strip_prefix('#').unwrap_or(name);
        let Some(old) = edit.names.find(label).filter(|old| scope::labels(&function).iter().any(|(found, _)| found == old)) else {
            let known: Vec<String> = scope::labels(&function).into_iter().map(|(found, _)| format!("#{}", edit.text(found))).collect();
            return Err(EditError::new("edit.unknown", format!("no label `{}` in this function", cut(value)))
                .of("value")
                .expecting(nearest(value, &known)));
        };
        if new != label {
            given(new, &labels(edit), "name")?;
            let new_name = edit.intern(new);
            rename_label(edit, old, new_name)?;
        }
        edited.names.insert("node", format!("#{new}"));
        return Ok(());
    }
    if function.signature.params.iter().any(|param| edit.text(param.name) == value)
        || function.signature.outputs.iter().any(|output| edit.text(output.name) == value)
    {
        return Err(EditError::invalid("value", format!("`{}` is an input or an output of the function: `signature.rename` renames it", cut(value))));
    }
    let values: Vec<scope::Value> = scope::values(&function).into_iter().filter(|found| edit.text(found.name) == value).collect();
    if values.is_empty() {
        let known: Vec<String> = scope::values(&function).iter().map(|found| edit.text(found.name).to_owned()).collect();
        return Err(EditError::new("edit.unknown", format!("no value `{}` in this function", cut(value)))
            .of("value")
            .expecting(nearest(value, &known)));
    }
    if values.iter().any(|found| !matches!(found.bound, Bound::Line(..))) {
        return Err(step_five("value", "a value that a structure gives (a clause of a loop)"));
    }
    if name != value {
        given(name, &taken(edit), "name")?;
        let new_name = edit.intern(name);
        // A name bound in several arms: each, so that the function names one value no more.
        for found in &values {
            rename(edit, found, new_name)?;
        }
    }
    edited.names.insert("value", name.to_owned());
    Ok(())
}

/// The labels of the function: a label is unique among them (spec R-17); a value may bear the same name.
fn labels(edit: &Edit) -> BTreeSet<String> {
    edit.function().map(|function| scope::labels(function).into_iter().map(|(label, _)| edit.text(label).to_owned()).collect()).unwrap_or_default()
}

/// Renames a label: the statement, the `after:` that wait for it, and the entries of `layout` that name it.
fn rename_label(edit: &mut Edit, from: Name, to: Name) -> Result<(), EditError> {
    let function = edit.body_function_mut()?;
    for loc in scope::all(function) {
        let Some(statement) = scope::statement_mut(function, &loc) else { continue };
        if let Some(label) = &mut statement.label
            && label.name == from
        {
            label.name = to;
        }
        let Some(args) = crate::places::args_mut(statement) else { continue };
        for arg in args.iter_mut() {
            if let Arg::After { refs, .. } = arg {
                for reference in refs.iter_mut() {
                    if let OrderRef::Label(label) = reference
                        && label.name == from
                    {
                        label.name = to;
                    }
                }
            }
        }
    }
    crate::names::rename_in_layout(edit, from, to, true);
    Ok(())
}

/// `node.set_label`.
pub(crate) fn set_label(edit: &mut Edit, line: u32, label: &str, edited: &mut Edited) -> Result<(), EditError> {
    let new = label.strip_prefix('#').unwrap_or(label);
    let function = edit.body_function()?.clone();
    let found = find_nodes(edit, &function, &NodeName { text: format!("@{line}"), is_label: false, line: Some(line) });
    let [loc] = found.as_slice() else {
        return Err(EditError::new("edit.unknown", format!("no node starts on line {line} of `{}`", edit.path)).of("line"));
    };
    let statement = scope::statement(&function, loc).ok_or_else(|| EditError::internal("a node that went"))?;
    match statement.label {
        Some(old) if edit.text(old.name) == new => {}
        Some(old) => {
            given(new, &labels(edit), "label")?;
            let new_name = edit.intern(new);
            rename_label(edit, old.name, new_name)?;
        }
        None => {
            given(new, &labels(edit), "label")?;
            let name = edit.intern(new);
            let id = edit.id();
            // The entries of `layout` that named the node by the name of its line name it by its label now (spec 11): a
            // node with a label is named by it.
            let line_name = scope::line_name(statement);
            let statement = scope::statement_mut(edit.body_function_mut()?, loc).ok_or_else(|| EditError::internal("a node that went"))?;
            statement.label = Some(Label { id, name });
            if let Some(line_name) = line_name {
                by_label(edit, line_name, name);
            }
        }
    }
    edited.names.insert("node", format!("#{new}"));
    Ok(())
}

/// The `node` entries, and the ends of `wire` entries that end at the node, that name it by `line_name` name it by
/// `label`; a wire that starts at a value keeps its value (spec 11).
fn by_label(edit: &mut Edit, line_name: Name, label: Name) {
    let Some(layout) = &mut edit.file.layout else { return };
    for item in &mut layout.items {
        match &mut item.kind {
            LayoutKind::Node { node, .. } => {
                if !node.is_label && node.name == line_name {
                    node.is_label = true;
                    node.name = label;
                }
            }
            LayoutKind::Wire { to, .. } => {
                if !to.node.is_label && to.node.name == line_name {
                    to.node.is_label = true;
                    to.node.name = label;
                }
            }
        }
    }
}

/// `node.set_template`.
pub(crate) fn set_template(edit: &mut Edit, judge: &dyn Judge, node_name: &str, template: &str) -> Result<(), EditError> {
    let function = edit.body_function()?.clone();
    let loc = node(edit, &function, &NodeName::parse(node_name), "node")?;
    let statement = scope::statement(&function, &loc).cloned().ok_or_else(|| EditError::internal("a node that went"))?;
    let Some(call) = format_call(edit, &statement) else {
        return Err(EditError::invalid("node", format!("`{}` is not a `string.format`", cut(node_name))));
    };
    let written = read_constant(edit, template).map_err(|error| EditError { parameter: Some("template"), ..error })?;
    let Operand::Text(_, literal) = &written else {
        return Err(EditError::invalid(
            "template",
            format!("`{}` is not a string literal: a template is written on the node (spec 5.4)", cut(template)),
        ));
    };
    let fields: BTreeSet<String> = judge.template(literal).map_err(|message| EditError::invalid("template", message))?.into_iter().collect();
    // A call that does not give its template yet gets it first, by position (R-11).
    let at = template_of(edit, call).map(|(at, _)| at);
    // The named arguments of the fields that are gone, with the values they read.
    let gone: Vec<(usize, String, Vec<Name>)> = call
        .args
        .iter()
        .enumerate()
        .filter_map(|(index, arg)| match arg {
            Arg::Value { name: Some(name), value, .. } if Some(index) != at && !fields.contains(edit.text(*name)) => {
                Some((index, edit.text(*name).to_owned(), crate::constant::values_read(&function, value)))
            }
            _ => None,
        })
        .collect();
    let template_id = edit.id();
    let function_mut = edit.body_function_mut()?;
    let statement_mut = scope::statement_mut(function_mut, &loc).ok_or_else(|| EditError::internal("a node that went"))?;
    let args = crate::places::args_mut(statement_mut).ok_or_else(|| EditError::internal("a call that went"))?;
    let mut list = std::mem::take(args).into_vec();
    // The template first, at its place before any argument goes; the arguments of the fields gone, last first.
    let given = match at.and_then(|at| list.get_mut(at)) {
        Some(Arg::Value { value, .. }) => {
            *value = written.clone();
            true
        }
        _ => false,
    };
    for (index, _, _) in gone.iter().rev() {
        if *index < list.len() {
            list.remove(*index);
        }
    }
    if !given {
        list.insert(0, Arg::Value { id: template_id, name: None, value: written });
    }
    *args = list.into_boxed_slice();
    for (_, field, read) in &gone {
        for name in read {
            forget_wire(edit, *name, statement_node(&statement), Some(field));
        }
    }
    Ok(())
}
