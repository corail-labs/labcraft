//! `structure.surround` (commands.md 9.6): nodes of one frame put in a new `match`, `handle`, `for` or `loop`, or an empty
//! one made where it is asked for. Nothing moves on the screen: the nodes keep their places, counted in the new frame,
//! and the structure is placed so that its frame starts where they were. A value that crosses the frame is passed in, or
//! becomes an output that keeps its name, and the value inside takes a suffix (N-14).

use std::collections::{BTreeSet, HashMap, HashSet};

use labcraft_format::{is_name, read};
use labcraft_graph::{
    Attached, Carry, Clause, ClauseKind, LayoutItem, LayoutKind, NodeRef, Operand, Path, Rhs, Statement, StatementKind, Target, View,
};
use labcraft_types::Name;

use crate::delete::{inside, label_readers};
use crate::fragment;
use crate::frame::{Within, target};
use crate::layout::{Designation, designate, designation, pick_place, shown, value_of};
use crate::model::Edit;
use crate::names::{fresh, rename, taken};
use crate::places::{operand_mut, rename_read, slots};
use crate::refs::{NodeName, node};
use crate::scope::{self, BodyPath, Bound, Loc, Value};
use crate::wire::{closes_a_cycle, depends_on_itself, names_in, node_name_of};
use crate::{EditError, Edited, Judge, Point, Renamed, StructureKind, TypeKind, ValueType};

/// What `structure.surround` is given besides the nodes and the frame.
pub(crate) struct Given<'a> {
    pub kind: StructureKind,
    pub selector: Option<&'a str>,
    pub over: Option<&'a str>,
    pub arm: Option<&'a str>,
    pub at: Option<Point>,
}

/// A value made inside that is read outside: an output of the structure.
struct Output {
    /// Its name, which the output keeps.
    name: Name,
    /// The place of its line among the nodes surrounded, and among the targets of that line.
    rank: usize,
    target: usize,
    /// The name that it takes inside.
    inner: String,
    /// It carries the error chain out (9.6).
    chain: bool,
}

pub(crate) fn surround(
    edit: &mut Edit,
    judge: &dyn Judge,
    nodes: &[String],
    within: &Within,
    given: &Given,
    edited: &mut Edited,
) -> Result<(), EditError> {
    check_params(given)?;
    if nodes.is_empty() {
        return empty(edit, judge, within, given, edited);
    }
    if within.inside.is_some() {
        return Err(EditError::invalid("inside", "the structure goes in the frame of its nodes: `inside` is for an empty one"));
    }
    if within.arm.is_some() {
        return Err(EditError::invalid("inside_arm", "the structure goes in the frame of its nodes: `inside_arm` is for an empty one"));
    }
    if given.at.is_some() {
        return Err(EditError::invalid("at", "the structure goes where its nodes are: `at` is for an empty one"));
    }
    let function = edit.body_function()?.clone();
    // Each name through one index; `@N`, and a name that the index does not settle, as a command names it.
    let names_of = fragment::index(&function);
    let mut locs: Vec<Loc> = Vec::with_capacity(nodes.len());
    for name in nodes {
        let parsed = NodeName::parse(name.trim());
        if parsed.text.is_empty() {
            return Err(EditError::invalid("nodes", "a node is named by its label (`#close`), the name of its line, or `@N`"));
        }
        let indexed = match parsed.line {
            Some(_) => None,
            None => edit.names.find(&parsed.text).and_then(|text| match names_of.get(&(parsed.is_label, text)).map(Vec::as_slice) {
                Some([one]) => Some(one.clone()),
                _ => None,
            }),
        };
        let loc = match indexed {
            Some(loc) => loc,
            None => node(edit, &function, &parsed, "nodes")?,
        };
        locs.push(loc);
    }
    locs.sort();
    locs.dedup();
    let frame: BodyPath = locs.first().map(|loc| loc.body.clone()).unwrap_or_default();
    if locs.iter().any(|loc| loc.body != frame) {
        return Err(EditError::invalid("nodes", "the nodes surrounded are in one frame: the body of the function, of a loop, or an arm"));
    }
    let statements: Vec<Statement> = locs.iter().filter_map(|loc| scope::statement(&function, loc).cloned()).collect();
    if statements.iter().any(|statement| matches!(statement.kind, StatementKind::Collect(_) | StatementKind::Until(_)))
        || statements.iter().any(|statement| matches!(&statement.kind, StatementKind::Binding { targets, .. } if targets.iter().any(|target| matches!(target, Target::Next(..)))))
    {
        return Err(EditError::invalid("nodes", "a `next`, a `collect` or an `until` stays in its loop body (R-6): surround the nodes without it"));
    }
    // A label waited for outside: an `after:` does not see into a structure (R-17).
    for statement in &statements {
        if let Some(label) = statement.label
            && label_readers(&function, label.name).iter().any(|reader| !locs.iter().any(|selected| inside(reader_loc(reader), selected)))
        {
            return Err(EditError::invalid(
                "nodes",
                format!("`#{}` is waited for outside: an order wire does not leave a structure (R-17)", edit.text(label.name)),
            ));
        }
    }

    // What crosses the frame, in one walk of the function: what the nodes surrounded read, and where each value is read
    // outside them; what they and the nodes in them name, for the routes of the wires.
    let selected: HashSet<usize> = locs.iter().map(|loc| loc.index).collect();
    let in_selection = |loc: &Loc| {
        loc.body.starts_with(&frame)
            && match loc.body.get(frame.len()) {
                Some(step) => selected.contains(&step.stmt),
                None => loc.body.len() == frame.len() && selected.contains(&loc.index),
            }
    };
    let mut read_inside: HashSet<Name> = HashSet::new();
    let mut read_outside: HashMap<Name, Vec<BodyPath>> = HashMap::new();
    let mut held_values: HashSet<Name> = HashSet::new();
    let mut held_nodes: HashSet<Loc> = HashSet::new();
    let mut nodes_named: HashMap<(Name, bool), Vec<Loc>> = HashMap::new();
    for loc in scope::all(&function) {
        let Some(statement) = scope::statement(&function, &loc) else { continue };
        if let Some(label) = statement.label {
            nodes_named.entry((label.name, true)).or_default().push(loc.clone());
        }
        if let Some(name) = scope::line_name(statement)
            && !matches!(statement.kind, StatementKind::Collect(_) | StatementKind::Until(_))
        {
            nodes_named.entry((name, false)).or_default().push(loc.clone());
        }
        let inside_it = in_selection(&loc);
        let mut reads: Vec<(Name, BodyPath)> = Vec::new();
        for (slot, seen_in) in slots(statement, &loc) {
            let mut names = Vec::new();
            if let Some(operand) = crate::places::operand(statement, slot) {
                names_in(operand, &mut names);
            }
            reads.extend(names.into_iter().map(|name| (name, seen_in.clone())));
        }
        for arg in crate::places::args(statement).into_iter().flatten() {
            if let labcraft_graph::Arg::After { refs, .. } = arg {
                for reference in refs {
                    if let labcraft_graph::OrderRef::Value(operand) = reference {
                        let mut names = Vec::new();
                        names_in(operand, &mut names);
                        reads.extend(names.into_iter().map(|name| (name, loc.body.clone())));
                    }
                }
            }
        }
        if inside_it {
            read_inside.extend(reads.into_iter().map(|(name, _)| name));
            held_values.extend(scope::targets(statement));
            held_values.extend(statement.label.map(|label| label.name));
            for sub in 0..scope::subs(statement).len() {
                held_values.extend(scope::head_names(statement, sub));
            }
            held_nodes.insert(loc);
        } else {
            for (name, seen_in) in reads {
                read_outside.entry(name).or_default().push(seen_in);
            }
        }
    }
    let is_output_of_function = |name: Name| frame.is_empty() && function.signature.outputs.iter().any(|output| output.name == name);
    let values = scope::values(&function);
    let mut outputs: Vec<Output> = Vec::new();
    let mut chosen: BTreeSet<String> = taken(edit);
    for (rank, loc) in locs.iter().enumerate() {
        let Some(statement) = scope::statement(&function, loc) else { continue };
        let StatementKind::Binding { targets, .. } = &statement.kind else { continue };
        for (target, found) in targets.iter().enumerate() {
            let Target::Name(_, name) = found else { continue };
            let value = Value { name: *name, bound: Bound::Line(loc.clone(), target), seen_in: frame.clone() };
            let read_outside = read_outside.get(name).is_some_and(|places| places.iter().any(|seen_in| scope::sees(&function, &value, seen_in)));
            let bound_to_output = is_output_of_function(*name);
            if !read_outside && !bound_to_output {
                continue;
            }
            // The value inside: a suffix, or the name of its node when it was bound to an output (notation 2.4).
            let inner = if bound_to_output { node_name_of(edit, judge, &function, &value) } else { fresh(edit.text(*name), &chosen) };
            chosen.insert(inner.clone());
            outputs.push(Output { name: *name, rank, target, inner, chain: false });
        }
    }
    let type_of = |value: &Value| -> ValueType {
        let line = match &value.bound {
            Bound::Input => 0,
            Bound::Line(loc, _) | Bound::Head(loc, _) => edit.lines.get(loc).copied().unwrap_or(0),
        };
        judge.value_type(&edit.path, &edit.original, line, edit.text(value.name))
    };
    // The error chain: exactly one error comes in, and exactly one goes out.
    let leaving: Vec<usize> = outputs
        .iter()
        .enumerate()
        .filter(|(_, output)| {
            let Some(loc) = locs.get(output.rank) else { return false };
            let value = Value { name: output.name, bound: Bound::Line(loc.clone(), output.target), seen_in: frame.clone() };
            type_of(&value).kind == TypeKind::Error
        })
        .map(|(index, _)| index)
        .collect();
    let mut entering: Vec<Value> = Vec::new();
    for value in &values {
        if !read_inside.contains(&value.name) || entering.iter().any(|known| known.name == value.name) {
            continue;
        }
        let made_inside = match &value.bound {
            Bound::Line(loc, _) | Bound::Head(loc, _) => in_selection(loc),
            Bound::Input => false,
        };
        if !made_inside && scope::sees(&function, value, &frame) && type_of(value).kind == TypeKind::Error {
            entering.push(value.clone());
        }
    }
    let chain_in = match (entering.as_slice(), leaving.as_slice()) {
        ([one], [out]) => {
            if let Some(output) = outputs.get_mut(*out) {
                output.chain = true;
            }
            Some(one.name)
        }
        _ => None,
    };

    // The places, before anything moves: of each node surrounded, its entry or where the drawing has it, and the entries
    // that go; each found through one index, so that a large selection stays near linear.
    let nodes_of = fragment::index(&function);
    let mut entry_at: HashMap<Loc, (usize, Option<Point>, Option<View>)> = HashMap::new();
    for (index, item) in edit.file.layout.iter().flat_map(|layout| layout.items.iter().enumerate()) {
        let LayoutKind::Node { node, at, view } = &item.kind else { continue };
        let Some(loc) = fragment::named_by(&nodes_of, node).filter(|loc| loc.body == frame && selected.contains(&loc.index)) else { continue };
        let at = at.as_ref().and_then(|point| Some(Point { x: value_of(edit, &point.x)?, y: value_of(edit, &point.y)? }));
        entry_at.entry(loc.clone()).or_insert((index, at, *view));
    }
    let free = locs.iter().any(|loc| entry_at.get(loc).is_none_or(|(_, at, _)| at.is_none()));
    let drawn = if free { judge.places(&edit.path, &edit.original) } else { Vec::new() };
    let mut on_line: HashMap<u32, Vec<&crate::Place>> = HashMap::new();
    let mut by_name: HashMap<&str, &crate::Place> = HashMap::new();
    for place in &drawn {
        on_line.entry(place.line).or_default().push(place);
        by_name.entry(place.name.as_str()).or_insert(place);
    }
    let mut placed: Vec<(usize, Point, Option<View>)> = Vec::new();
    for (rank, loc) in locs.iter().enumerate() {
        let (fixed, view) = entry_at.get(loc).map_or((None, None), |(_, at, view)| (*at, *view));
        // As `layout::drawn_at` finds it: by its line and its name, by its line alone when it is the one node there, by
        // its name otherwise.
        let at = fixed.or_else(|| {
            let name = fragment::designated(&nodes_of, &function, loc).map(|named| shown(edit, named));
            let here = edit.lines.get(loc).and_then(|line| on_line.get(line)).map(Vec::as_slice).unwrap_or_default();
            pick_place(here, name.as_deref(), |name| by_name.get(name).copied()).map(|place| place.at)
        });
        if let Some(at) = at {
            placed.push((rank, at, view));
        }
    }
    let corner = Point { x: placed.iter().map(|(_, at, _)| at.x).min().unwrap_or(0), y: placed.iter().map(|(_, at, _)| at.y).min().unwrap_or(0) };
    let entries: HashSet<usize> = entry_at.values().map(|(index, _, _)| *index).collect();
    // The routes of `layout`, sorted before anything is renamed: between two nodes surrounded (kept, counted in the new
    // frame, when the wire is in their frame), across the edge (gone), or neither.
    let classes: Vec<Option<bool>> = edit
        .file
        .layout
        .iter()
        .flat_map(|layout| &layout.items)
        .map(|item| {
            let LayoutKind::Wire { from, to, .. } = &item.kind else { return None };
            let from_inside = held_values.contains(&from.node.name);
            let to_nodes = nodes_named.get(&(to.node.name, to.node.is_label)).map(Vec::as_slice).unwrap_or_default();
            let to_inside = !to_nodes.is_empty() && to_nodes.iter().all(|loc| held_nodes.contains(loc));
            let to_top = to_nodes.iter().all(|loc| selected.contains(&loc.index) && loc.body == frame);
            match (from_inside, to_inside) {
                (true, true) => Some(to_top),
                (false, false) => None,
                _ => Some(false),
            }
        })
        .collect();

    // The head, read from its text, then the nodes put in it.
    let selector_value = given.selector.map(|text| visible(edit, &function, text, &frame, "selector")).transpose()?;
    let over = given.over.map(|text| over_of(edit, &function, text, &frame)).transpose()?;
    // A selector or a range that the nodes surrounded make: the structure would read its own output.
    let made_inside = |name: Name| {
        values.iter().any(|value| {
            value.name == name && scope::sees(&function, value, &frame) && matches!(&value.bound, Bound::Line(loc, _) if in_selection(loc))
        })
    };
    let read_names: Vec<(Name, &'static str)> = selector_value
        .iter()
        .map(|value| (value.name, "selector"))
        .chain(over.iter().flat_map(|over| over.names.iter().map(|name| (*name, "over"))))
        .collect();
    if let Some((name, parameter)) = read_names.iter().find(|(name, _)| made_inside(*name)) {
        return Err(EditError::new(
            "wire.cycle",
            format!("`{}` is made by the nodes surrounded: the structure would read its own output", edit.text(*name)),
        )
        .of(parameter));
    }
    let selector_type = selector_value.as_ref().map(&type_of);
    let over_type = over.as_ref().map(|over| over.array.as_ref().map(&type_of).unwrap_or_default());
    let selector_given = selector_value.as_ref().zip(selector_type);
    let over_given = over.as_ref().zip(over_type);
    let mut structure = head(edit, given, selector_given, over_given, &chosen)?;
    let arm = arm_of(edit, &structure, given)?;
    let at = locs.first().map(|loc| loc.index).unwrap_or(0);
    {
        let body = scope::body_mut(edit.body_function_mut()?, &frame).ok_or_else(|| EditError::internal("a frame that went"))?;
        for loc in locs.iter().rev() {
            if loc.index < body.statements.len() {
                body.statements.remove(loc.index);
            }
        }
    }
    let carry_name = chain_in.map(|_| fresh("err", &chosen));
    fill(edit, &mut structure, arm, statements, &outputs, chain_in, carry_name.as_deref())?;
    let body = scope::body_mut(edit.body_function_mut()?, &frame).ok_or_else(|| EditError::internal("a frame that went"))?;
    let at = at.min(body.statements.len());
    body.statements.insert(at, structure);
    let structure_loc = Loc { body: frame.clone(), index: at };
    let inner_body = structure_loc.sub(arm);

    // Inside, the values take their names, and in a loop the error that came in is the carry.
    for output in &outputs {
        let value = Value {
            name: output.name,
            bound: Bound::Line(Loc { body: inner_body.clone(), index: output.rank }, output.target),
            seen_in: inner_body.clone(),
        };
        let inner = edit.intern(&output.inner);
        rename(edit, &value, inner)?;
        edited.renamed.push(Renamed { file: edit.path.clone(), from: edit.text(output.name).to_owned(), to: output.inner.clone() });
    }
    if let (Some(error), Some(carry)) = (chain_in, carry_name.as_deref())
        && matches!(given.kind, StructureKind::For | StructureKind::Loop)
    {
        let carry = edit.intern(carry);
        read_carry(edit, &inner_body, error, carry)?;
    }
    // A structure that reads what its own nodes make would close a cycle (spec 5.2).
    let function_now = edit.body_function()?.clone();
    for name in selector_value.iter().map(|value| value.name).chain(over.iter().flat_map(|over| over.names.iter().copied())) {
        let made_by = scope::values(&function_now).into_iter().find_map(|value| match value.bound {
            Bound::Line(loc, _) if value.name == name && loc.body == frame && loc != structure_loc => Some(loc),
            _ => None,
        });
        if let Some(made_by) = made_by
            && closes_a_cycle(&function_now, &made_by, &structure_loc)
        {
            return Err(EditError::new(
                "wire.cycle",
                format!("`{}` depends on what the nodes surrounded make: the structure would read its own output", edit.text(name)),
            )
            .of(if selector_value.is_some() { "selector" } else { "over" }));
        }
    }
    // Nodes not next to each other: a value read inside can itself read what the structure gives.
    if depends_on_itself(&function_now, &structure_loc) {
        return Err(EditError::new(
            "wire.cycle",
            "a node between the nodes surrounded reads what they make and is read by them: the structure would read its own output",
        )
        .of("nodes"));
    }

    // The layout: the nodes counted in the new frame, the routes between them kept, with the names that the values took
    // inside, those that cross the edge gone.
    let inner_names: HashMap<Name, Name> = outputs.iter().filter_map(|output| Some((output.name, edit.names.find(&output.inner)?))).collect();
    let mut kept_items = Vec::new();
    if let Some(layout) = edit.file.layout.take() {
        let mut layout = layout;
        for ((index, mut item), class) in std::mem::take(&mut layout.items).into_iter().enumerate().zip(classes) {
            if entries.contains(&index) {
                continue;
            }
            match class {
                // Between two nodes of the frame: counted in the new frame.
                Some(true) => {
                    if let LayoutKind::Wire { from, to, via } = &mut item.kind {
                        if let Some(inner) = inner_names.get(&from.node.name).filter(|_| !from.node.is_label) {
                            from.node.name = *inner;
                        }
                        if let Some(inner) = inner_names.get(&to.node.name).filter(|_| !to.node.is_label) {
                            to.node.name = *inner;
                        }
                        for point in via.iter_mut() {
                            let (Some(x), Some(y)) = (value_of(edit, &point.x), value_of(edit, &point.y)) else { continue };
                            point.x = crate::add::coordinate(edit, x - corner.x);
                            point.y = crate::add::coordinate(edit, y - corner.y);
                        }
                    }
                    kept_items.push(item);
                }
                // Across the edge: the automatic routing draws it.
                Some(false) => {}
                None => kept_items.push(item),
            }
        }
        layout.items = kept_items;
        edit.file.layout = Some(layout);
    }
    let mut labelled = Vec::new();
    let nodes_now = fragment::index(edit.body_function()?);
    for (rank, at, view) in &placed {
        let loc = Loc { body: inner_body.clone(), index: *rank };
        // A label only where the section cannot name the node as it is.
        let named = match fragment::designated(&nodes_now, edit.body_function()?, &loc) {
            Some(named) => named,
            None => designate(edit, &loc, &mut labelled)?,
        };
        push_entry(edit, named, Point { x: at.x - corner.x, y: at.y - corner.y }, *view)?;
    }
    let named = designate(edit, &structure_loc, &mut labelled)?;
    let structure_shown = shown(edit, named);
    if !placed.is_empty() {
        push_entry(edit, named, corner, None)?;
        // Where the frame of the arm starts in the structure, from a drawing of the text: the structure goes that much up
        // and to the left, so that its nodes stay where they were. Its header can depend on where it stands (a frame
        // that grows up), so the drawing is asked again until the first node is where it was, a few times at most.
        let first = placed.first().map(|(rank, at, _)| (Loc { body: inner_body.clone(), index: *rank }, *at));
        let inner_shown = first.as_ref().and_then(|(loc, _)| designation(edit, edit.body_function().ok()?, loc)).map(|found| shown(edit, found));
        let mut at = corner;
        for _ in 0..4 {
            let text = edit.render()?;
            let places = judge.places(&edit.path, &text);
            let structure_place = places.iter().find(|place| place.name == structure_shown);
            let inner_place = inner_shown.as_ref().and_then(|name| places.iter().find(|place| &place.name == name));
            let (Some(outer), Some(inner), Some(wanted)) = (structure_place, inner_place, first.as_ref().map(|(_, wanted)| *wanted)) else { break };
            // Where the first node is now, in the frame of the structure.
            let now = Point { x: inner.absolute.x - outer.absolute.x + outer.at.x, y: inner.absolute.y - outer.absolute.y + outer.at.y };
            if now == wanted {
                break;
            }
            at = Point { x: at.x + wanted.x - now.x, y: at.y + wanted.y - now.y };
            crate::add::in_range(at, "nodes")?;
            set_entry(edit, named, at)?;
        }
    }
    // `edit.lines` holds the lines before the command: a node of the new arm was at its old place, and the structure,
    // new, had no `@N` to rename.
    for (loc, label) in &labelled {
        let before = if loc.body == inner_body { locs.get(loc.index) } else { None };
        if let Some(line) = before.and_then(|old| edit.lines.get(old)) {
            edited.renamed.push(Renamed { file: edit.path.clone(), from: format!("@{line}"), to: label.clone() });
        }
    }
    edited.names.insert("structure", structure_shown);
    // In the order of the targets of the structure: in a loop, the carry of the error chain comes first.
    let in_order: Vec<String> = edit
        .body_function()
        .ok()
        .and_then(|function| scope::statement(function, &structure_loc))
        .map(|statement| scope::targets(statement).into_iter().map(|name| edit.text(name).to_owned()).collect())
        .unwrap_or_default();
    edited.lists.insert("outputs", in_order);
    Ok(())
}

/// The statement of a reader.
fn reader_loc(reader: &crate::delete::Reader) -> &Loc {
    match reader {
        crate::delete::Reader::Slot(loc, _) | crate::delete::Reader::After(loc) => loc,
    }
}

/// The parameters that a kind takes (9.6).
fn check_params(given: &Given) -> Result<(), EditError> {
    let kind = given.kind.keyword();
    match given.kind {
        StructureKind::Match | StructureKind::Handle => {
            if given.selector.is_none() {
                return Err(EditError::invalid("selector", format!("a `{kind}` needs `selector`, the value it looks at")));
            }
            if given.over.is_some() {
                return Err(EditError::invalid("over", format!("a `{kind}` goes over nothing: `over` is for a `for`")));
            }
        }
        StructureKind::For => {
            if given.over.is_none() {
                return Err(EditError::invalid("over", "a `for` needs `over`, an array value or a range (`0..n`)"));
            }
            if given.selector.is_some() {
                return Err(EditError::invalid("selector", "a `for` has no selector: `selector` is for a `match` or a `handle`"));
            }
        }
        StructureKind::Loop => {
            if given.selector.is_some() {
                return Err(EditError::invalid("selector", "a `loop` has no selector"));
            }
            if given.over.is_some() {
                return Err(EditError::invalid("over", "a `loop` goes over nothing: `over` is for a `for`"));
            }
        }
    }
    if given.arm.is_some() && matches!(given.kind, StructureKind::For | StructureKind::Loop) {
        return Err(EditError::invalid("arm", format!("a `{kind}` has one body and no arm")));
    }
    Ok(())
}

/// An empty structure, in the frame of `within` (9.6): it gives nothing.
fn empty(edit: &mut Edit, judge: &dyn Judge, within: &Within, given: &Given, edited: &mut Edited) -> Result<(), EditError> {
    let frame = target(edit, within, "inside", "inside_arm")?;
    let function = edit.body_function()?.clone();
    let selector_value = given.selector.map(|text| visible(edit, &function, text, &frame, "selector")).transpose()?;
    let over = given.over.map(|text| over_of(edit, &function, text, &frame)).transpose()?;
    let type_of = |value: &Value| -> ValueType {
        let line = match &value.bound {
            Bound::Input => 0,
            Bound::Line(loc, _) | Bound::Head(loc, _) => edit.lines.get(loc).copied().unwrap_or(0),
        };
        judge.value_type(&edit.path, &edit.original, line, edit.text(value.name))
    };
    let chosen = taken(edit);
    let selector_type = selector_value.as_ref().map(&type_of);
    let over_type = over.as_ref().map(|over| over.array.as_ref().map(&type_of).unwrap_or_default());
    let mut structure = head(edit, given, selector_value.as_ref().zip(selector_type), over.as_ref().zip(over_type), &chosen)?;
    arm_of(edit, &structure, given)?;
    fill(edit, &mut structure, 0, Vec::new(), &[], None, None)?;
    crate::add::push(edit, &frame, structure)?;
    let body = scope::body(edit.body_function()?, &frame).ok_or_else(|| EditError::internal("a frame that went"))?;
    let loc = Loc { body: frame.clone(), index: body.statements.len().saturating_sub(1) };
    let mut labelled = Vec::new();
    let named = designate(edit, &loc, &mut labelled)?;
    if let Some(at) = given.at {
        crate::add::in_range(at, "at")?;
        push_entry(edit, named, at, None)?;
    }
    edited.names.insert("structure", shown(edit, named));
    edited.lists.insert("outputs", Vec::new());
    Ok(())
}

/// A value that the frame sees, by its name.
fn visible(edit: &Edit, function: &labcraft_graph::Function, text: &str, frame: &[scope::Step], parameter: &'static str) -> Result<Value, EditError> {
    let text = text.trim();
    if !is_name(text) {
        return Err(EditError::invalid(parameter, format!("`{}` is not a value: a structure looks at a value, by its name", crate::cut(text))));
    }
    let Some(name) = edit.names.find(text) else {
        return Err(EditError::new("edit.unknown", format!("no value is named `{text}` in this function")).of(parameter));
    };
    let mut candidates: Vec<Value> = scope::values(function).into_iter().filter(|value| value.name == name).collect();
    if candidates.iter().any(|value| matches!(value.bound, Bound::Input)) {
        candidates.retain(|value| !matches!(&value.bound, Bound::Line(loc, _) if loc.body.is_empty()));
    }
    let seen: Vec<Value> = candidates.into_iter().filter(|value| scope::sees(function, value, frame)).collect();
    match seen.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err(EditError::invalid(parameter, format!("`{text}` cannot be seen from the frame of the structure"))),
        _ => Err(EditError::new("edit.ambiguous", format!("`{text}` names several values that the frame sees")).of(parameter)),
    }
}

/// What a `for` goes over: an array value, or a range whose bounds are integers or values.
struct Over {
    /// The text of the clause after `in`.
    text: String,
    /// The values it reads.
    names: Vec<Name>,
    /// The value of an array, for the name of its element; none for a range.
    array: Option<Value>,
}

fn over_of(edit: &Edit, function: &labcraft_graph::Function, text: &str, frame: &[scope::Step]) -> Result<Over, EditError> {
    let text = text.trim();
    match text.split_once("..") {
        Some((low, high)) => {
            let mut names = Vec::new();
            let mut bound = |piece: &str| -> Result<String, EditError> {
                let piece = piece.trim();
                let integer = piece.strip_prefix('-').unwrap_or(piece);
                if !integer.is_empty() && integer.chars().all(|c| c.is_ascii_digit() || c == '_') && integer.starts_with(|c: char| c.is_ascii_digit())
                {
                    // A bound that no integer holds is no range that `check` takes (`LC1006`).
                    if piece.replace('_', "").parse::<i64>().is_err() {
                        return Err(EditError::invalid("over", format!("`{}` is too large a bound for a range", crate::cut(piece))));
                    }
                    return Ok(piece.to_owned());
                }
                let value = visible(edit, function, piece, frame, "over")?;
                names.push(value.name);
                Ok(piece.to_owned())
            };
            let low = bound(low)?;
            let high = bound(high)?;
            Ok(Over { text: format!("{low}..{high}"), names, array: None })
        }
        None => {
            let value = visible(edit, function, text, frame, "over")?;
            Ok(Over { text: text.to_owned(), names: vec![value.name], array: Some(value) })
        }
    }
}

/// The structure with its head and its arms, empty, read from its text (`_ = match x { … }`): what the grammar reads is
/// what the command writes.
fn head(
    edit: &mut Edit,
    given: &Given,
    selector: Option<(&Value, ValueType)>,
    over: Option<(&Over, ValueType)>,
    chosen: &BTreeSet<String>,
) -> Result<Statement, EditError> {
    let mut chosen = chosen.clone();
    let text = match given.kind {
        StructureKind::Match => {
            let Some((selector, found)) = selector else { return Err(EditError::internal("a `match` without selector")) };
            let name = edit.text(selector.name).to_owned();
            let arms: Vec<String> = match &found.kind {
                TypeKind::Bool => vec!["true".to_owned(), "false".to_owned()],
                TypeKind::Integer | TypeKind::Text | TypeKind::Unknown => vec!["_".to_owned()],
                TypeKind::Enum { library, variants } => {
                    if let Some(library) = library {
                        crate::add::add_use(edit, library);
                    }
                    variants
                        .iter()
                        .map(|variant| {
                            if variant.values.is_empty() {
                                return variant.path.clone();
                            }
                            let bound: Vec<String> = variant
                                .values
                                .iter()
                                .map(|base| {
                                    let name = fresh(base, &chosen);
                                    chosen.insert(name.clone());
                                    name
                                })
                                .collect();
                            format!("{}({})", variant.path, bound.join(", "))
                        })
                        .collect()
                }
                TypeKind::Error | TypeKind::Array | TypeKind::Other => {
                    return Err(EditError::new(
                        "wire.type_mismatch",
                        format!(
                            "`{name}` is a `{}`: a `match` looks at a boolean, an integer without a unit, a string or an enumeration (spec 6.1)",
                            found.shown
                        ),
                    )
                    .of("selector"));
                }
            };
            let mut text = format!("_ = match {name} {{\n");
            for arm in arms {
                text.push_str(&format!("    {arm} => {{\n    }}\n"));
            }
            text.push_str("}\n");
            text
        }
        StructureKind::Handle => {
            let Some((selector, found)) = selector else { return Err(EditError::internal("a `handle` without selector")) };
            let name = edit.text(selector.name).to_owned();
            if !matches!(found.kind, TypeKind::Error | TypeKind::Unknown) {
                return Err(EditError::new(
                    "wire.type_mismatch",
                    format!("`{name}` is a `{}`: a `handle` looks at an `error` (spec 7.3)", found.shown),
                )
                .of("selector"));
            }
            format!("_ = handle {name} {{\n    ok => {{\n    }}\n    _ => {{\n    }}\n}}\n")
        }
        StructureKind::For => {
            let Some((over, found)) = over else { return Err(EditError::internal("a `for` without `over`")) };
            let base = match &over.array {
                Some(array) => {
                    let name = edit.text(array.name).to_owned();
                    if !matches!(found.kind, TypeKind::Array | TypeKind::Unknown) {
                        return Err(EditError::new(
                            "wire.type_mismatch",
                            format!("`{name}` is a `{}`: a `for` goes over an array or a range (spec 6.2)", found.shown),
                        )
                        .of("over"));
                    }
                    match name.strip_suffix('s') {
                        Some(single) if !single.is_empty() && is_name(single) => single.to_owned(),
                        _ => "item".to_owned(),
                    }
                }
                None => "i".to_owned(),
            };
            let element = fresh(&base, &chosen);
            format!("_ = for {element} in {} {{\n}}\n", over.text)
        }
        StructureKind::Loop => "_ = loop {\n    until ?\n}\n".to_owned(),
    };
    let wrapped = format!("labcraft 1\n\nfn f() -> () {{\n{text}}}\n");
    let limits = edit.limits().clone();
    let reading = read(wrapped.as_bytes(), &mut edit.names, &limits);
    let not_read = || EditError::internal("the head of a structure does not read: a bug of `edit`");
    let file = reading.file.ok_or_else(not_read)?;
    let labcraft_graph::Decl::Function(function) = file.decl else { return Err(not_read()) };
    let mut statements = function.body.statements;
    if statements.len() != 1 {
        return Err(not_read());
    }
    let mut statement = statements.pop().ok_or_else(not_read)?;
    statement.id = edit.id();
    Ok(statement)
}

/// The arm that takes the nodes: `arm` by its patterns, or the first.
fn arm_of(edit: &Edit, structure: &Statement, given: &Given) -> Result<usize, EditError> {
    let Some(text) = given.arm else { return Ok(0) };
    let StatementKind::Binding { value: Rhs::Match(branching) | Rhs::Handle(branching), .. } = &structure.kind else {
        return Err(EditError::invalid("arm", "a loop has one body and no arm"));
    };
    crate::frame::arm_index(edit, branching, text).ok_or_else(|| {
        EditError::new("edit.unknown", format!("the new structure has no arm `{}`", crate::cut(text)))
            .of("arm")
            .expecting(crate::frame::arm_names(edit, branching))
    })
}

/// Puts the nodes in the arm `arm` (or the body of a loop), and makes the outputs: its targets, and for each the `yield`
/// of each arm, or the `collect` of a loop; the error chain through the other arms, or through a carry.
fn fill(
    edit: &mut Edit,
    structure: &mut Statement,
    arm: usize,
    statements: Vec<Statement>,
    outputs: &[Output],
    chain_in: Option<Name>,
    carry: Option<&str>,
) -> Result<(), EditError> {
    let path = |edit: &mut Edit, name: Name| Operand::Path(Path { id: edit.id(), first: name, rest: Box::new([]) });
    let mut targets: Vec<Target> = Vec::new();
    match &mut structure.kind {
        StatementKind::Binding { value: Rhs::Match(branching) | Rhs::Handle(branching), .. } => {
            for output in outputs {
                targets.push(Target::Name(edit.id(), output.name));
            }
            let arms = branching.arms.len();
            for rank in 0..arms {
                let mut values = Vec::with_capacity(outputs.len());
                for output in outputs {
                    let value = if rank == arm {
                        path(edit, output.name)
                    } else if let (true, Some(error)) = (output.chain, chain_in) {
                        path(edit, error)
                    } else {
                        Operand::Hole(edit.id())
                    };
                    values.push(value);
                }
                if let Some(each) = branching.arms.get_mut(rank) {
                    each.yields.values = values;
                }
            }
            let Some(each) = branching.arms.get_mut(arm) else { return Err(EditError::internal("an arm that went")) };
            each.body.statements = statements;
        }
        StatementKind::Binding { value: Rhs::For(_) | Rhs::Loop(_), .. } => {
            // The nodes first, so that each keeps its place among them; then what the head holds (`until`), then the
            // `collect` and `next` lines.
            let mut lines = Vec::new();
            let mut clauses = Vec::new();
            for output in outputs {
                if let (true, Some(error), Some(carry)) = (output.chain, chain_in, carry) {
                    let carry = edit.intern(carry);
                    targets.insert(0, Target::Name(edit.id(), output.name));
                    let value = path(edit, error);
                    clauses.push(Clause {
                        comments: Attached::default(),
                        kind: ClauseKind::Carry(Carry { id: edit.id(), name: carry, ty: None, value }),
                    });
                    let next = path(edit, output.name);
                    lines.push(Statement {
                        id: edit.id(),
                        comments: Attached::default(),
                        label: None,
                        kind: StatementKind::Binding { targets: Box::new([Target::Next(edit.id(), carry)]), value: Rhs::Operand(next) },
                    });
                } else {
                    targets.push(Target::Name(edit.id(), output.name));
                    let value = path(edit, output.name);
                    lines.push(Statement { id: edit.id(), comments: Attached::default(), label: None, kind: StatementKind::Collect(value) });
                }
            }
            let StatementKind::Binding { value, .. } = &mut structure.kind else { return Err(EditError::internal("a loop that went")) };
            let (own_clauses, body) = match value {
                Rhs::For(each) => (&mut each.clauses, &mut each.body),
                Rhs::Loop(repeat) => (&mut repeat.clauses, &mut repeat.body),
                _ => return Err(EditError::internal("a loop that went")),
            };
            own_clauses.extend(clauses);
            let head = std::mem::take(&mut body.statements);
            body.statements = statements;
            body.statements.extend(head);
            body.statements.extend(lines);
        }
        _ => return Err(EditError::internal("a structure that is not one")),
    }
    if !targets.is_empty()
        && let StatementKind::Binding { targets: own, .. } = &mut structure.kind
    {
        *own = targets.into_boxed_slice();
    }
    Ok(())
}

/// In the body `inner` of a loop, the nodes that read the error that came in read the carry.
fn read_carry(edit: &mut Edit, inner: &[scope::Step], error: Name, carry: Name) -> Result<(), EditError> {
    let function = edit.body_function()?.clone();
    let places: Vec<(Loc, crate::places::Slot)> = scope::all(&function)
        .into_iter()
        .filter(|loc| loc.body.starts_with(inner))
        .flat_map(|loc| {
            scope::statement(&function, &loc)
                .map(|statement| slots(statement, &loc).into_iter().map(|(slot, _)| (loc.clone(), slot)).collect::<Vec<_>>())
                .unwrap_or_default()
        })
        .collect();
    let function = edit.body_function_mut()?;
    for (loc, slot) in places {
        // The `next` of the carry gives the error that leaves: it is not read from the carry.
        let is_next = scope::statement(function, &loc).is_some_and(|statement| {
            matches!(&statement.kind, StatementKind::Binding { targets, .. } if targets.iter().any(|target| matches!(target, Target::Next(_, name) if *name == carry)))
        });
        if is_next {
            continue;
        }
        if let Some(operand) = scope::statement_mut(function, &loc).and_then(|statement| operand_mut(statement, slot)) {
            rename_read(operand, error, carry);
        }
    }
    Ok(())
}

/// A `node … at` entry for a node that has none.
fn push_entry(edit: &mut Edit, named: Designation, at: Point, view: Option<View>) -> Result<(), EditError> {
    crate::add::in_range(at, "nodes")?;
    let point = labcraft_graph::Point { id: edit.id(), x: crate::add::coordinate(edit, at.x), y: crate::add::coordinate(edit, at.y) };
    let node = NodeRef { id: edit.id(), is_label: named.is_label, name: named.name };
    let item = LayoutItem { id: edit.id(), comments: Attached::default(), kind: LayoutKind::Node { node, at: Some(point), view } };
    let id = edit.id();
    edit.file
        .layout
        .get_or_insert_with(|| labcraft_graph::Layout { id, comments: Attached::default(), items: Vec::new(), end: Vec::new() })
        .items
        .push(item);
    Ok(())
}

/// Moves the entry of the node of this designation to `at`.
fn set_entry(edit: &mut Edit, named: Designation, at: Point) -> Result<(), EditError> {
    let x = crate::add::coordinate(edit, at.x);
    let y = crate::add::coordinate(edit, at.y);
    let Some(layout) = &mut edit.file.layout else { return Ok(()) };
    for item in &mut layout.items {
        if let LayoutKind::Node { node, at: Some(point), .. } = &mut item.kind
            && node.is_label == named.is_label
            && node.name == named.name
        {
            point.x = x;
            point.y = y;
        }
    }
    Ok(())
}
