//! `node.move`, `node.place`, `node.set_view` and `layout.arrange` (commands.md 6.7): the entries of `layout` that fix a
//! node, set its view or route a wire (spec 11), and nothing else of the file, but the label that a node gets when the
//! section cannot name it otherwise (decision of the owner, 2026-11-17).

use std::collections::{BTreeSet, HashSet};

use labcraft_graph::{Attached, Function, Label, Layout, LayoutItem, LayoutKind, NodeRef, Numeral, NumeralKind, Rhs, StatementKind, View};
use labcraft_types::Name;

use crate::add::coordinate;
use crate::model::Edit;
use crate::names::fresh;
use crate::refs::{self, NodeName};
use crate::scope::{self, Loc};
use crate::{EditError, Edited, Judge, MAX_COORDINATE, Point};

/// How the `layout` section names a node (spec 11): by its label, or by the name of its line when no other node of the
/// function bears it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Designation {
    pub is_label: bool,
    pub name: Name,
}

/// The nodes that a command names, each once, in the order given.
fn locs(edit: &Edit, nodes: &[String], parameter: &'static str) -> Result<Vec<Loc>, EditError> {
    let function = edit.body_function()?;
    let mut found: Vec<Loc> = Vec::with_capacity(nodes.len());
    for name in nodes {
        let parsed = NodeName::parse(name.trim());
        if parsed.text.is_empty() {
            return Err(EditError::invalid(parameter, "a node is named by its label (`#close`), the name of its line, or `@N`"));
        }
        let loc = refs::node(edit, function, &parsed, parameter)?;
        if !found.contains(&loc) {
            found.push(loc);
        }
    }
    Ok(found)
}

/// The designation of the node at `loc`, if the section can name it as it is.
fn designation(edit: &Edit, function: &Function, loc: &Loc) -> Option<Designation> {
    let statement = scope::statement(function, loc)?;
    if let Some(label) = statement.label {
        return Some(Designation { is_label: true, name: label.name });
    }
    let name = scope::line_name(statement)?;
    let parsed = NodeName { text: edit.text(name).to_owned(), is_label: false, line: None };
    (refs::find_nodes(edit, function, &parsed).as_slice() == [loc.clone()]).then_some(Designation { is_label: false, name })
}

/// What a label of a node is made from (N-14): the name of its line, else what it calls, else what it is.
fn label_base(edit: &Edit, function: &Function, loc: &Loc) -> String {
    let Some(statement) = scope::statement(function, loc) else { return "node".to_owned() };
    if let Some(name) = scope::line_name(statement) {
        return edit.text(name).to_owned();
    }
    let callee = match &statement.kind {
        StatementKind::Call(call) => Some(&call.callee),
        StatementKind::Binding { value: Rhs::Call(call), .. } => Some(&call.callee),
        StatementKind::Binding { value: Rhs::Struct(found), .. } => Some(&found.path),
        _ => None,
    };
    if let Some(path) = callee {
        let last = path.rest.last().copied().unwrap_or(path.first);
        return edit.text(last).to_owned();
    }
    // A structure with no name: what it is, which is a reserved word, so not a label alone (`#for_node`).
    match &statement.kind {
        StatementKind::Binding { value: Rhs::Match(_), .. } => "match_node",
        StatementKind::Binding { value: Rhs::Handle(_), .. } => "handle_node",
        StatementKind::Binding { value: Rhs::For(_), .. } => "for_node",
        StatementKind::Binding { value: Rhs::Loop(_), .. } => "loop_node",
        _ => "node",
    }
    .to_owned()
}

/// The designation of the node at `loc`, after giving it a label when the section could not name it: a node with no
/// name, or with a name of several bodies (commands.md 9.2; decision of the owner, 2026-11-17). The label is the name of
/// its line or of what it calls, numbered when another label has it (N-14).
pub(crate) fn designate(edit: &mut Edit, loc: &Loc, labelled: &mut Vec<(Loc, String)>) -> Result<Designation, EditError> {
    let function = edit.body_function()?;
    if let Some(found) = designation(edit, function, loc) {
        return Ok(found);
    }
    let base = label_base(edit, function, loc);
    let taken: BTreeSet<String> = scope::labels(function).into_iter().map(|(label, _)| edit.text(label).to_owned()).collect();
    let label = fresh(&base, &taken);
    if !labcraft_format::is_name(&label) {
        return Err(EditError::internal(format!("`{label}` is no label: a bug of `edit`")));
    }
    let name = edit.intern(&label);
    let id = edit.id();
    let statement = scope::statement_mut(edit.body_function_mut()?, loc).ok_or_else(|| EditError::internal("a node named went away"))?;
    statement.label = Some(Label { id, name });
    labelled.push((loc.clone(), format!("#{label}")));
    Ok(Designation { is_label: true, name })
}

/// The name of a node as a command names it now.
fn shown(edit: &Edit, designation: Designation) -> String {
    if designation.is_label { format!("#{}", edit.text(designation.name)) } else { edit.text(designation.name).to_owned() }
}

/// The value of a coordinate of `layout`; none for one that is not an integer (`LC5005`).
pub(crate) fn value_of(edit: &Edit, numeral: &Numeral) -> Option<i64> {
    let NumeralKind::Int(text) = numeral.kind else { return None };
    let digits: String = edit.text(text).chars().filter(|letter| *letter != '_').collect();
    let value = digits.parse::<i64>().ok()?;
    Some(if numeral.negative { -value } else { value })
}

/// The index of the `node` entry that designates the node at `loc`, by its label or the name of its line.
fn entry_of(edit: &Edit, function: &Function, loc: &Loc) -> Option<usize> {
    let layout = edit.file.layout.as_ref()?;
    layout.items.iter().position(|item| {
        let LayoutKind::Node { node, .. } = &item.kind else { return false };
        let parsed = NodeName { text: edit.text(node.name).to_owned(), is_label: node.is_label, line: None };
        refs::find_nodes(edit, function, &parsed).as_slice() == [loc.clone()]
    })
}

/// Where the `node` entry of the node at `loc` fixes it, in its frame; none for a free node.
fn fixed_at(edit: &Edit, function: &Function, loc: &Loc) -> Option<Point> {
    let index = entry_of(edit, function, loc)?;
    let item = edit.file.layout.as_ref()?.items.get(index)?;
    let LayoutKind::Node { at: Some(at), .. } = &item.kind else { return None };
    Some(Point { x: value_of(edit, &at.x)?, y: value_of(edit, &at.y)? })
}

/// Where a free node is, in its frame, as the drawing of the text before the operation places it (architecture 12.3).
fn drawn_at(edit: &Edit, judge: &dyn Judge, places: &mut Option<Vec<crate::Place>>, loc: &Loc, parameter: &'static str) -> Result<Point, EditError> {
    let function = edit.body_function()?;
    let all = places.get_or_insert_with(|| judge.places(&edit.path, &edit.original));
    let line = edit.lines.get(loc).copied();
    let name = designation(edit, function, loc).map(|found| shown(edit, found));
    // By its line and its name: a structure and a node it holds can start on one line; by its line alone when it is
    // the one node there; by its name otherwise.
    let on_line: Vec<&crate::Place> = all.iter().filter(|place| line.is_some_and(|line| place.line == line)).collect();
    let named = |place: &&crate::Place| name.as_ref().is_some_and(|name| place.name == *name);
    let alone = match on_line.as_slice() {
        [one] => Some(*one),
        _ => None,
    };
    let found = on_line.iter().copied().find(named).or(alone).or_else(|| all.iter().find(named));
    match found {
        Some(place) => Ok(place.at),
        None => Err(EditError::invalid(
            parameter,
            "the function is not drawn now, so a node placed automatically has no place to move from: `node.place` gives one",
        )),
    }
}

/// Fixes the node of this designation at `at`: its `node … at` entry, made or changed, its view kept (spec 11).
fn fix(edit: &mut Edit, loc: &Loc, designation: Designation, at: Point, parameter: &'static str) -> Result<(), EditError> {
    let range = -MAX_COORDINATE..=MAX_COORDINATE;
    if !range.contains(&at.x) || !range.contains(&at.y) {
        return Err(EditError::invalid(parameter, format!("a coordinate is between -{MAX_COORDINATE} and {MAX_COORDINATE}")));
    }
    let point = labcraft_graph::Point { id: edit.id(), x: coordinate(edit, at.x), y: coordinate(edit, at.y) };
    let node = NodeRef { id: edit.id(), is_label: designation.is_label, name: designation.name };
    let index = entry_of(edit, edit.body_function()?, loc);
    let (layout_id, item_id) = (edit.id(), edit.id());
    let layout = edit.file.layout.get_or_insert_with(|| Layout { id: layout_id, comments: Attached::default(), items: Vec::new(), end: Vec::new() });
    match index.and_then(|index| layout.items.get_mut(index)) {
        Some(item) => {
            if let LayoutKind::Node { node: named, at: place, .. } = &mut item.kind {
                *named = node;
                *place = Some(point);
            }
        }
        None => {
            layout.items.push(LayoutItem {
                id: item_id,
                comments: Attached::default(),
                kind: LayoutKind::Node { node, at: Some(point), view: None },
            });
        }
    }
    Ok(())
}

/// The names of the nodes, as a command names them now, for `names.nodes`; and each node that got a label in `renamed`,
/// from its line to its label, so that a selection follows it.
fn report(edit: &Edit, locs: &[Loc], labelled: &[(Loc, String)], edited: &mut Edited) -> Result<Vec<String>, EditError> {
    let function = edit.body_function()?;
    let names: Vec<String> = locs.iter().filter_map(|loc| designation(edit, function, loc).map(|found| shown(edit, found))).collect();
    for (loc, label) in labelled {
        if let Some(line) = edit.lines.get(loc) {
            edited.renamed.push(crate::Renamed { file: edit.path.clone(), from: format!("@{line}"), to: label.clone() });
        }
    }
    Ok(names)
}

/// `node.move`: the nodes of one frame by `by`, each fixed where it goes (notation 2.4). A move by nothing changes nothing.
pub(crate) fn move_nodes(edit: &mut Edit, judge: &dyn Judge, nodes: &[String], by: Point, edited: &mut Edited) -> Result<(), EditError> {
    if nodes.is_empty() {
        return Err(EditError::invalid("nodes", "`nodes` names at least one node"));
    }
    let locs = locs(edit, nodes, "nodes")?;
    if locs.iter().any(|loc| locs.first().is_some_and(|first| first.body != loc.body)) {
        return Err(EditError::invalid("nodes", "the nodes of one move are in one frame: those of another frame move apart"));
    }
    if by == (Point { x: 0, y: 0 }) {
        let names = report(edit, &locs, &[], edited)?;
        edited.lists.insert("nodes", names);
        return Ok(());
    }
    let mut places = None;
    let mut from = Vec::with_capacity(locs.len());
    for loc in &locs {
        let function = edit.body_function()?;
        let at = match fixed_at(edit, function, loc) {
            Some(at) => at,
            None => drawn_at(edit, judge, &mut places, loc, "nodes")?,
        };
        from.push(at);
    }
    let mut labelled = Vec::new();
    for (loc, at) in locs.iter().zip(from) {
        let designation = designate(edit, loc, &mut labelled)?;
        let to = Point { x: at.x.saturating_add(by.x), y: at.y.saturating_add(by.y) };
        fix(edit, loc, designation, to, "by")?;
    }
    let names = report(edit, &locs, &labelled, edited)?;
    edited.lists.insert("nodes", names);
    Ok(())
}

/// `node.place`: one node at `at` in its frame, fixed.
pub(crate) fn place_node(edit: &mut Edit, node: &str, at: Point, edited: &mut Edited) -> Result<(), EditError> {
    let locs = locs(edit, &[node.to_owned()], "node")?;
    let Some(loc) = locs.first() else { return Err(EditError::invalid("node", "`node` names a node")) };
    let function = edit.body_function()?;
    if fixed_at(edit, function, loc) == Some(at) {
        edited.names.insert("node", designation(edit, function, loc).map(|found| shown(edit, found)).unwrap_or_default());
        return Ok(());
    }
    let mut labelled = Vec::new();
    let designation = designate(edit, loc, &mut labelled)?;
    fix(edit, loc, designation, at, "at")?;
    let names = report(edit, &locs, &labelled, edited)?;
    edited.names.insert("node", names.into_iter().next().unwrap_or_default());
    Ok(())
}

/// `node.set_view`: `view standard` or `view compact` on each node, or none with `default` (spec 11). An entry left with
/// neither a place nor a view goes.
pub(crate) fn set_view(edit: &mut Edit, nodes: &[String], view: Option<View>, edited: &mut Edited) -> Result<(), EditError> {
    if nodes.is_empty() {
        return Err(EditError::invalid("nodes", "`nodes` names at least one node"));
    }
    let locs = locs(edit, nodes, "nodes")?;
    let mut labelled = Vec::new();
    for loc in &locs {
        let function = edit.body_function()?;
        let index = entry_of(edit, function, loc);
        match (index, view) {
            (None, None) => {}
            (Some(index), _) => {
                if let Some(item) = edit.file.layout.as_mut().and_then(|layout| layout.items.get_mut(index))
                    && let LayoutKind::Node { view: shown, .. } = &mut item.kind
                {
                    *shown = view;
                }
            }
            (None, Some(view)) => {
                let designation = designate(edit, loc, &mut labelled)?;
                let node = NodeRef { id: edit.id(), is_label: designation.is_label, name: designation.name };
                let (item_id, layout_id) = (edit.id(), edit.id());
                let item = LayoutItem { id: item_id, comments: Attached::default(), kind: LayoutKind::Node { node, at: None, view: Some(view) } };
                let layout = edit.file.layout.get_or_insert_with(|| Layout {
                    id: layout_id,
                    comments: Attached::default(),
                    items: Vec::new(),
                    end: Vec::new(),
                });
                layout.items.push(item);
            }
        }
    }
    drop_empty(edit);
    let names = report(edit, &locs, &labelled, edited)?;
    edited.lists.insert("nodes", names);
    Ok(())
}

/// Removes the `node` entries with neither a place nor a view, and the section when nothing is left in it.
pub(crate) fn drop_empty(edit: &mut Edit) {
    if let Some(layout) = &mut edit.file.layout {
        layout.items.retain(|item| !matches!(item.kind, LayoutKind::Node { at: None, view: None, .. }));
        if layout.items.is_empty() && layout.end.is_empty() && layout.comments == Attached::default() {
            edit.file.layout = None;
        }
    }
}

/// `layout.arrange` (commands.md 9.1): removes the entries of what it arranges, and the automatic placement places it
/// again around the rest. The nodes and the routes of the wires between them, the routes of `wires`, the terminals; with
/// none of these, the whole function. A `node` entry keeps its view (spec 11): arranging places, it does not change a
/// view.
pub(crate) fn arrange(edit: &mut Edit, judge: &dyn Judge, nodes: Option<&[String]>, wires: &[String], terminals: &[String]) -> Result<(), EditError> {
    let whole = nodes.is_none() && wires.is_empty() && terminals.is_empty();
    let function = edit.body_function()?;
    for name in terminals {
        crate::select::terminal(edit, function, name)?;
    }
    // The routes of the wires named, as `wire.disconnect` names them, or as `wire.route` does: a wire that an entry
    // routes, into a structure or to its `until` too. Each name stands for the wire drawn that it designates (spec 11).
    let drawn = if wires.is_empty() { Vec::new() } else { judge.wires(&edit.path, &edit.original) };
    let of_value = crate::route::by_value(&drawn);
    let resolved: Vec<(&String, Option<String>)> = wires.iter().map(|wire| (wire, crate::route::resolve(edit, &of_value, wire))).collect();
    for (name, drawn) in &resolved {
        refs::split_wire(name).map_err(|error| error.of("wires"))?;
        let names = |item: &LayoutItem| match &item.kind {
            LayoutKind::Wire { from, to, .. } => crate::route::entry_names(edit, &of_value, from, to, name, None),
            LayoutKind::Node { .. } => false,
        };
        let routed = drawn.is_some() || edit.file.layout.as_ref().is_some_and(|layout| layout.items.iter().any(names));
        if !routed {
            crate::wire::find(edit, judge, function, name).map_err(|error| match error.parameter {
                Some(_) => error.of("wires"),
                None => error,
            })?;
        }
    }
    let locs = match nodes {
        Some(nodes) => locs(edit, nodes, "nodes")?,
        None => Vec::new(),
    };
    let function = edit.body_function()?;
    // What names the nodes arranged: their labels, the names of their lines and the values they bind, for the wires
    // between them.
    let mut named: HashSet<Name> = HashSet::new();
    let mut entries: HashSet<usize> = HashSet::new();
    for loc in &locs {
        if let Some(statement) = scope::statement(function, loc) {
            named.extend(statement.label.map(|label| label.name));
            named.extend(scope::targets(statement));
        }
        entries.extend(entry_of(edit, function, loc));
    }
    let Some(mut layout) = edit.file.layout.take() else { return Ok(()) };
    let mut items = std::mem::take(&mut layout.items);
    for (index, item) in items.iter_mut().enumerate() {
        match &mut item.kind {
            LayoutKind::Node { at, .. } => {
                if whole || entries.contains(&index) {
                    *at = None;
                }
            }
            LayoutKind::Wire { .. } => {}
        }
    }
    items.retain(|item| match &item.kind {
        LayoutKind::Node { .. } => true,
        LayoutKind::Wire { from, to, .. } => {
            let between = named.contains(&from.node.name) && named.contains(&to.node.name);
            let routed = resolved.iter().any(|(wire, drawn)| crate::route::entry_names(edit, &of_value, from, to, wire, drawn.as_deref()));
            !(whole || between || routed)
        }
    });
    layout.items = items;
    edit.file.layout = Some(layout);
    drop_empty(edit);
    Ok(())
}

/// Gives a label to each node that `lines` names by its line (`@N`) and that the `layout` section cannot name, in one
/// pass over the text, so that the lines name the nodes of that text whatever canonical form then moves: what
/// `commands.batch` does before its commands (commands.md 6.7; decision of the owner, 2026-11-17). Each line that names
/// one node gives how a command names it now, `@N` first; a line that names none, or several, is left to the commands.
pub(crate) fn label_lines(edit: &mut Edit, lines: &[u32]) -> Result<Vec<(String, String, bool)>, EditError> {
    let mut named = Vec::new();
    for &line in lines {
        let function = edit.body_function()?;
        let name = NodeName { text: format!("@{line}"), is_label: false, line: Some(line) };
        let found_here = refs::find_nodes(edit, function, &name);
        let [loc] = found_here.as_slice() else { continue };
        let loc = loc.clone();
        let mut labelled = Vec::new();
        let designation = designate(edit, &loc, &mut labelled)?;
        named.push((format!("@{line}"), shown(edit, designation), !labelled.is_empty()));
    }
    Ok(named)
}
