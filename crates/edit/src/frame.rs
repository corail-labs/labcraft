//! The frame that `inside` and `arm` name (commands.md 6.7): the body of the function, the body of a loop, or an arm of a
//! `match` or a `handle`, named by its patterns as the text writes them (`_`, not `other`).

use labcraft_graph::{Branching, Rhs, StatementKind};

use crate::EditError;
use crate::model::Edit;
use crate::refs::{self, NodeName};
use crate::scope::{self, BodyPath, Loc};

/// Where a command puts what it adds: `inside` a structure, in its `arm`; the body of the function when neither is given.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Within {
    pub inside: Option<String>,
    pub arm: Option<String>,
}

/// The body that `within` names, its parameters named `inside` and `arm` (or `inside_arm`) in the messages.
pub(crate) fn target(edit: &Edit, within: &Within, inside_param: &'static str, arm_param: &'static str) -> Result<BodyPath, EditError> {
    let Some(inside) = within.inside.as_deref() else {
        if within.arm.is_some() {
            return Err(EditError::invalid(arm_param, format!("`{arm_param}` names an arm of the structure that `{inside_param}` names")));
        }
        return Ok(Vec::new());
    };
    let function = edit.body_function()?;
    let name = NodeName::parse(inside.trim());
    if name.text.is_empty() {
        return Err(EditError::invalid(inside_param, "a structure is named as a node: its label (`#for_node`), the name of its line, or `@N`"));
    }
    let loc = refs::node(edit, function, &name, inside_param)?;
    let sub = sub_of(edit, &loc, &name.shown(), within.arm.as_deref(), inside_param, arm_param)?;
    Ok(loc.sub(sub))
}

/// The index of the body of the structure at `loc` that `arm` names: an arm by its patterns (the first by default), or
/// the body of a loop, which has no arm.
pub(crate) fn sub_of(
    edit: &Edit,
    loc: &Loc,
    shown: &str,
    arm: Option<&str>,
    inside_param: &'static str,
    arm_param: &'static str,
) -> Result<usize, EditError> {
    let function = edit.body_function()?;
    let statement = scope::statement(function, loc).ok_or_else(|| EditError::internal("a node named went away"))?;
    let StatementKind::Binding { value, .. } = &statement.kind else { return Err(not_a_structure(inside_param, shown)) };
    match value {
        Rhs::Match(branching) | Rhs::Handle(branching) => match arm {
            None => Ok(0),
            Some(text) => arm_index(edit, branching, text).ok_or_else(|| {
                EditError::new("edit.unknown", format!("`{shown}` has no arm `{}`", crate::cut(text)))
                    .of(arm_param)
                    .expecting(arm_names(edit, branching))
            }),
        },
        Rhs::For(_) | Rhs::Loop(_) => match arm {
            None => Ok(0),
            Some(_) => Err(EditError::invalid(arm_param, format!("`{shown}` is a loop: it has one body and no arm"))),
        },
        Rhs::Call(_) | Rhs::Struct(_) | Rhs::Operand(_) => Err(not_a_structure(inside_param, shown)),
    }
}

fn not_a_structure(parameter: &'static str, shown: &str) -> EditError {
    EditError::invalid(parameter, format!("`{shown}` is not a structure: a `match`, a `handle`, a `for` or a `loop`"))
}

/// The patterns of each arm, as the text writes them and `arm` names them (`1, 3..=5`, `io.timeout(err)`).
pub(crate) fn arm_names(edit: &Edit, branching: &Branching) -> Vec<String> {
    branching
        .arms
        .iter()
        .map(|arm| arm.patterns().map(|pattern| labcraft_format::write_pattern(pattern, &edit.names, usize::MAX)).collect::<Vec<_>>().join(", "))
        .collect()
}

/// The arm that `text` names by its patterns. Spaces do not count, so `1,3..=5` names `1, 3..=5`.
pub(crate) fn arm_index(edit: &Edit, branching: &Branching, text: &str) -> Option<usize> {
    let bare = |text: &str| text.chars().filter(|letter| !letter.is_whitespace()).collect::<String>();
    let wanted = bare(text);
    arm_names(edit, branching).iter().position(|name| bare(name) == wanted)
}
