//! The names of commands.md 9.2, written as in the `layout` section: a node by its label, by the name of its line, or by
//! `@N`, the line where it starts; an input as the end of a wire, a wire by `value -> node` or `value -> node.input`, a
//! terminal by `output x`.

use labcraft_graph::{Function, StatementKind};

use crate::EditError;
use crate::model::Edit;
use crate::scope::{self, Loc};

/// A node as a command names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NodeName {
    pub text: String,
    pub is_label: bool,
    /// `@N`: the node that starts on line `N` of the file, from 1, as the reports name a node with no name (`docs/cli.md`,
    /// 4.3) and as a name of several bodies is told apart (decision of the owner, 2026-11-17).
    pub line: Option<u32>,
}

impl NodeName {
    pub fn parse(text: &str) -> NodeName {
        if let Some(line) = text.strip_prefix('@').and_then(|line| line.parse::<u32>().ok()).filter(|line| *line > 0) {
            return NodeName { text: text.to_owned(), is_label: false, line: Some(line) };
        }
        match text.strip_prefix('#') {
            Some(label) => NodeName { text: label.to_owned(), is_label: true, line: None },
            None => NodeName { text: text.to_owned(), is_label: false, line: None },
        }
    }

    pub fn shown(&self) -> String {
        if self.is_label { format!("#{}", self.text) } else { self.text.clone() }
    }
}

/// What follows the node in the name of an input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Part {
    /// The node alone: its only input, or its only input without a name.
    Alone,
    /// A parameter, a field, a carry, an `in` clause, or `after`.
    Name(String),
    /// An input without a name, by its place among those of its node, from 0.
    Index(usize),
    /// A value of a variadic parameter, by its place in it, from 0.
    Variadic(String, usize),
    /// `n.next.x`.
    Next(String),
}

/// The end of a wire.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum InputName {
    Node(NodeName, Part),
    /// `output x`, a terminal of the frame of the function.
    Output(String),
}

impl InputName {
    pub fn parse(text: &str, parameter: &'static str) -> Result<InputName, EditError> {
        let text = text.trim();
        if let Some(rest) = text.strip_prefix("output ") {
            return Ok(InputName::Output(rest.trim().to_owned()));
        }
        let mut pieces = text.split('.');
        let node = pieces
            .next()
            .filter(|piece| !piece.is_empty())
            .ok_or_else(|| EditError::invalid(parameter, "an input is named `node`, `node.input` or `output x`"))?;
        let rest: Vec<&str> = pieces.collect();
        let number = |piece: &str| piece.parse::<usize>().ok();
        let part = match rest.as_slice() {
            [] => Part::Alone,
            ["next", name] => Part::Next((*name).to_owned()),
            [piece] => match number(piece) {
                Some(index) => Part::Index(index),
                None => Part::Name((*piece).to_owned()),
            },
            [name, piece] => match number(piece) {
                Some(index) => Part::Variadic((*name).to_owned(), index),
                None => {
                    return Err(EditError::invalid(
                        parameter,
                        format!("`{text}` names no input: a value of a variadic parameter is numbered (`msg.parts.2`)"),
                    ));
                }
            },
            _ => return Err(EditError::invalid(parameter, format!("`{text}` names no input"))),
        };
        Ok(InputName::Node(NodeName::parse(node), part))
    }
}

/// `value -> node`, `value -> node.input`.
pub(crate) fn split_wire(text: &str) -> Result<(String, String), EditError> {
    let Some((from, to)) = text.split_once("->") else {
        return Err(EditError::invalid("wire", "a wire is named `value -> node` or `value -> node.input`"));
    };
    let (from, to) = (from.trim(), to.trim());
    if from.is_empty() || to.is_empty() {
        return Err(EditError::invalid("wire", "a wire is named `value -> node` or `value -> node.input`"));
    }
    Ok((from.to_owned(), to.to_owned()))
}

/// The statements that a node name names.
pub(crate) fn find_nodes(edit: &Edit, function: &Function, name: &NodeName) -> Vec<Loc> {
    if let Some(line) = name.line {
        let mut found: Vec<Loc> = edit
            .lines
            .iter()
            .filter(|(_, at)| **at == line)
            .map(|(loc, _)| loc.clone())
            .filter(|loc| {
                scope::statement(function, loc)
                    .is_some_and(|statement| !matches!(statement.kind, StatementKind::Gather(_, _) | StatementKind::Until(_)))
            })
            .collect();
        found.sort();
        return found;
    }
    let Some(wanted) = edit.names.find(&name.text) else { return Vec::new() };
    scope::all(function)
        .into_iter()
        .filter(|loc| {
            let Some(statement) = scope::statement(function, loc) else { return false };
            if name.is_label {
                statement.label.is_some_and(|label| label.name == wanted)
            } else {
                !matches!(statement.kind, StatementKind::Gather(_, _) | StatementKind::Until(_)) && scope::line_name(statement) == Some(wanted)
            }
        })
        .collect()
}

/// The one statement that a node name names, or `edit.unknown` with the nearest names, or `edit.ambiguous`.
pub(crate) fn node(edit: &Edit, function: &Function, name: &NodeName, parameter: &'static str) -> Result<Loc, EditError> {
    let found = find_nodes(edit, function, name);
    match found.as_slice() {
        [one] => Ok(one.clone()),
        [] => {
            let known = node_names(edit, function);
            Err(EditError::new("edit.unknown", format!("no node is named `{}` in this function", name.shown()))
                .of(parameter)
                .expecting(nearest(&name.shown(), &known)))
        }
        _ => Err(EditError::new(
            "edit.ambiguous",
            format!("`{}` names {} nodes, in several bodies: name it by a label (`node.set_label`)", name.shown(), found.len()),
        )
        .of(parameter)
        .expecting(vec![name.shown(); found.len()])),
    }
}

/// The names by which a command can name the nodes of the function.
pub(crate) fn node_names(edit: &Edit, function: &Function) -> Vec<String> {
    let mut names = Vec::new();
    for loc in scope::all(function) {
        let Some(statement) = scope::statement(function, &loc) else { continue };
        if let Some(label) = statement.label {
            names.push(format!("#{}", edit.text(label.name)));
        }
        if let Some(name) = scope::line_name(statement) {
            names.push(edit.text(name).to_owned());
        }
    }
    names.sort();
    names.dedup();
    names
}

/// The three names nearest to `text`, nearest first.
pub(crate) fn nearest(text: &str, known: &[String]) -> Vec<String> {
    let mut scored: Vec<(usize, &String)> = known.iter().map(|name| (distance(text, name), name)).collect();
    scored.sort();
    let bound = text.chars().count().max(3);
    scored.into_iter().filter(|(score, _)| *score <= bound).take(3).map(|(_, name)| name.clone()).collect()
}

/// The edit distance between two texts, in code points.
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut current = Vec::with_capacity(b.len() + 1);
        current.push(i + 1);
        for (j, cb) in b.iter().enumerate() {
            let substitute = previous.get(j).copied().unwrap_or(usize::MAX).saturating_add(usize::from(ca != *cb));
            let insert = current.get(j).copied().unwrap_or(usize::MAX).saturating_add(1);
            let delete = previous.get(j + 1).copied().unwrap_or(usize::MAX).saturating_add(1);
            current.push(substitute.min(insert).min(delete));
        }
        previous = current;
    }
    previous.last().copied().unwrap_or(0)
}
