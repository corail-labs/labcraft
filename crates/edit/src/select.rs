//! What the selection of the window names (`view.select`): nodes, wires and terminals of the frame, named as an editing
//! command names them (commands.md 9.2). Nothing is changed: a wire is found as `wire.disconnect` finds it, by the same
//! function, so that what can be selected and what can be unwired never differ.

use labcraft_graph::Function;
use labcraft_types::Limits;

use crate::model::Edit;
use crate::refs::{NodeName, nearest, node};
use crate::{EditError, Files, Judge, wire};

/// Whether these names name what the function of `path` has, as an editing command names them (commands.md 9.2):
/// for the selection of the window (`view.select`). The first one that names nothing is `edit.unknown` (with the nearest
/// names in `expected`) or `edit.ambiguous`, its parameter `nodes`, `wires` or `terminals`; a name of the wrong form is
/// `params.invalid`.
pub fn selectable(
    files: &Files,
    path: &str,
    nodes: &[String],
    wires: &[String],
    terminals: &[String],
    judge: &dyn Judge,
    limits: &Limits,
) -> Result<(), EditError> {
    let Some(text) = files.get(path) else {
        return Err(EditError::new("path.not_found", format!("`{path}` is not a file of the project")).of("file"));
    };
    let edit = Edit::read(path, text, limits)?;
    let function = edit.body_function()?;
    for name in nodes {
        let parsed = NodeName::parse(name.trim());
        if parsed.text.is_empty() {
            return Err(EditError::invalid("nodes", "a node is named by its label (`#close`) or the name of its line"));
        }
        node(&edit, function, &parsed, "nodes")?;
    }
    for name in wires {
        wire::find(&edit, judge, function, name).map_err(|error| match error.parameter {
            Some(_) => error.of("wires"),
            None => error,
        })?;
    }
    for name in terminals {
        terminal(&edit, function, name)?;
    }
    Ok(())
}

/// `input x` or `output x`, a terminal of the frame of the function.
pub(crate) fn terminal(edit: &Edit, function: &Function, name: &str) -> Result<(), EditError> {
    let text = name.trim();
    let (side, rest) = match (text.strip_prefix("input "), text.strip_prefix("output ")) {
        (Some(rest), _) => ("input", rest.trim()),
        (None, Some(rest)) => ("output", rest.trim()),
        (None, None) => return Err(EditError::invalid("terminals", format!("`{text}` names no terminal: it is `input x` or `output x`"))),
    };
    if rest.is_empty() || rest.contains(char::is_whitespace) {
        return Err(EditError::invalid("terminals", format!("`{text}` names no terminal: it is `input x` or `output x`")));
    }
    let signature = &function.signature;
    let found = match side {
        "input" => signature.params.iter().any(|param| edit.text(param.name) == rest),
        _ => signature.outputs.iter().any(|output| edit.text(output.name) == rest),
    };
    if found {
        return Ok(());
    }
    let known: Vec<String> = signature
        .params
        .iter()
        .map(|param| format!("input {}", edit.text(param.name)))
        .chain(signature.outputs.iter().map(|output| format!("output {}", edit.text(output.name))))
        .collect();
    let shown = format!("{side} {rest}");
    Err(EditError::new("edit.unknown", format!("the function has no terminal `{shown}`")).of("terminals").expecting(nearest(&shown, &known)))
}
