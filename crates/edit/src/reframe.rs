//! `node.reframe` (commands.md 6.7, visual notation 2.4): nodes of one frame moved into another one, at a place of it,
//! in one step. What crosses an edge follows the rules of a wire: a value that the new frame sees is passed in; a value
//! that leaves an arm becomes an output of its structure, the other arms yielding a hole; a value that leaves a loop goes
//! out as `through` says. The nodes keep their names; a value that leaves a structure keeps its name outside and takes a
//! suffix inside (N-14), as `structure.surround` does.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use labcraft_graph::{Arg, Function, Id, LayoutKind, OrderRef, StatementKind, Target};
use labcraft_types::Name;

use crate::delete::{Reader, label_readers, readers};
use crate::fragment::loop_only;
use crate::frame::{Within, target};
use crate::layout::{designate, designation, drawn_at, entry_of, fix, fixed_at, on_grid, shown, value_of};
use crate::model::Edit;
use crate::names::{fresh, rename, rename_in_layout, taken};
use crate::places::{Slot, operand, operand_mut, rename_read, slots};
use crate::scope::{self, BodyPath, Bound, Frame, Loc, Step, Value};
use crate::wire::{closes_a_cycle, depends_on_itself, give, names_in, node_name_of, pass};
use crate::{EditError, Edited, Judge, Passage, Point, Renamed, Through, TypeKind};

/// A value, by what binds it, so that it is found again when statements move: an input by its name, a line by its id and
/// its place among the targets, the head of a body by its statement, the body and the name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Key {
    Input(Name),
    Line(Id, usize),
    Head(Id, usize, Name),
}

/// Where a place reads: a slot of a statement, or its `after:`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    Slot(Id, Slot),
    After(Id),
}

pub(crate) fn reframe(
    edit: &mut Edit,
    judge: &dyn Judge,
    nodes: &[String],
    within: &Within,
    at: Option<Point>,
    through: &BTreeMap<String, Through>,
    edited: &mut Edited,
) -> Result<(), EditError> {
    if nodes.is_empty() {
        return Err(EditError::invalid("nodes", "`nodes` names at least one node"));
    }
    let locs = crate::layout::locs(edit, nodes, "nodes")?;
    let frame: BodyPath = locs.first().map(|loc| loc.body.clone()).unwrap_or_default();
    if locs.iter().any(|loc| loc.body != frame) {
        return Err(EditError::invalid("nodes", "the nodes moved together are in one frame: those of another frame move apart"));
    }
    let function = edit.body_function()?.clone();
    for loc in &locs {
        let statement = scope::statement(&function, loc).ok_or_else(|| EditError::internal("a node named went away"))?;
        if !scope::subs(statement).is_empty() {
            let shown_name = designation(edit, &function, loc).map(|found| shown(edit, found)).unwrap_or_else(|| "a node".to_owned());
            return Err(EditError::invalid(
                "nodes",
                format!("`{shown_name}` is a structure: moving a structure into another frame is not done yet; move the nodes it holds"),
            ));
        }
        if loop_only(statement) {
            return Err(EditError::invalid("nodes", "a `next`, a `collect`, a `last` or an `until` stays in its loop body (R-6)"));
        }
    }
    let to = target(edit, within, "inside", "arm")?;
    if to == frame {
        return Err(EditError::invalid("inside", "the nodes are in that frame already: `node.move` moves them in it"));
    }
    if let Some(at) = at {
        crate::add::in_range(at, "at")?;
    }
    let id_at = |loc: &Loc| scope::statement(&function, loc).map(|statement| statement.id);
    let moved: Vec<Id> = locs.iter().filter_map(id_at).collect();
    let moved_set: HashSet<Id> = moved.iter().copied().collect();
    // The structures around the two frames, which a move could make read their own outputs.
    let around: Vec<Id> = ancestors(&function, &frame).into_iter().chain(ancestors(&function, &to)).collect();
    // The body that takes the nodes, by the structure that holds it and its sub, found again after the move.
    let to_holder: Option<(Id, usize)> = to.split_last().and_then(|(last, outer)| {
        scope::body(&function, outer).and_then(|body| body.statements.get(last.stmt)).map(|statement| (statement.id, last.sub))
    });

    // Where the nodes were, before anything moves, for the place they keep among themselves.
    let mut olds: Vec<Option<Point>> = Vec::with_capacity(locs.len());
    if at.is_some() {
        let mut places = None;
        for loc in &locs {
            let old = match fixed_at(edit, &function, loc) {
                Some(found) => Some(found),
                None => drawn_at(edit, judge, &mut places, loc, "nodes").ok(),
            };
            olds.push(old);
        }
    }

    // What the nodes make, and who reads it outside them; what they read, and from where.
    let values_before = scope::values(&function);
    let key_of = |value: &Value| key(&function, value);
    let mut held: Vec<(Key, Name)> = Vec::new();
    let mut held_readers: HashMap<Key, Vec<Place>> = HashMap::new();
    let mut to_output: HashSet<Key> = HashSet::new();
    for value in &values_before {
        let Bound::Line(loc, _) = &value.bound else { continue };
        if !locs.contains(loc) {
            continue;
        }
        let Some(found) = key_of(value) else { continue };
        held.push((found, value.name));
        let places: Vec<Place> = readers(&function, value, &locs)
            .into_iter()
            .filter_map(|reader| match reader {
                Reader::Slot(loc, slot) => id_at(&loc).map(|id| Place::Slot(id, slot)),
                Reader::After(loc) => id_at(&loc).map(Place::After),
            })
            .collect();
        held_readers.insert(found, places);
        // A line of the body of the function with the name of an output binds that output (R-2).
        if frame.is_empty() && function.signature.outputs.iter().any(|output| output.name == value.name) {
            to_output.insert(found);
        }
    }
    let held_keys: HashSet<Key> = held.iter().map(|(found, _)| *found).collect();
    let named_before = by_name(&values_before);
    // What every other place of the function reads, to be found again after the move.
    let mut reads_before: Vec<(Place, Name, Key)> = Vec::new();
    for loc in scope::all(&function) {
        let Some(statement) = scope::statement(&function, &loc) else { continue };
        for (slot, seen_in) in slots(statement, &loc) {
            let mut names = Vec::new();
            if let Some(found) = operand(statement, slot) {
                names_in(found, &mut names);
            }
            for name in names {
                if let Some(found) = resolve(&function, &named_before, name, &seen_in).as_ref().and_then(key_of) {
                    reads_before.push((Place::Slot(statement.id, slot), name, found));
                }
            }
        }
    }
    // The order wires: of the nodes moved, and to them.
    let mut waits: Vec<(Id, OrderTo)> = Vec::new();
    for loc in &locs {
        let Some(statement) = scope::statement(&function, loc) else { continue };
        for arg in crate::places::args(statement).into_iter().flatten() {
            let Arg::After { refs, .. } = arg else { continue };
            for reference in refs {
                match reference {
                    OrderRef::Label(label) => {
                        if let Some((_, found)) = scope::labels(&function).into_iter().find(|(name, _)| *name == label.name) {
                            waits.extend(id_at(&found).map(|id| (statement.id, OrderTo::Statement(id))));
                        }
                    }
                    OrderRef::Value(found) => {
                        let mut names = Vec::new();
                        names_in(found, &mut names);
                        for name in names {
                            if let Some(value) = resolve(&function, &named_before, name, &loc.body).as_ref().and_then(key_of) {
                                waits.push((statement.id, OrderTo::Value(name, value)));
                            }
                        }
                    }
                }
            }
        }
        if let Some(label) = statement.label {
            for reader in label_readers(&function, label.name) {
                if let Reader::After(found) = reader
                    && !locs.contains(&found)
                    && let Some(id) = id_at(&found)
                {
                    waits.push((id, OrderTo::Statement(statement.id)));
                }
            }
        }
    }

    // An order wire from a value that a node moved makes, to a node that stays.
    for (found, name) in &held {
        for place in held_readers.get(found).map(Vec::as_slice).unwrap_or_default() {
            if let Place::After(waiter) = place {
                waits.push((*waiter, OrderTo::Value(*name, *found)));
            }
        }
    }

    // The entries of `layout`: of the nodes moved, gone; the routes between them, kept and shifted with them when `at`
    // places them; the routes that touch them otherwise, gone.
    let held_names: HashSet<Name> = held.iter().map(|(_, name)| *name).collect();
    let moved_names: HashSet<(Name, bool)> = locs
        .iter()
        .filter_map(|loc| scope::statement(&function, loc))
        .flat_map(|statement| {
            statement.label.map(|label| (label.name, true)).into_iter().chain(scope::line_name(statement).map(|name| (name, false)))
        })
        .collect();
    let entries: HashSet<usize> = locs.iter().filter_map(|loc| entry_of(edit, &function, loc)).collect();
    let mut gone: HashSet<usize> = HashSet::new();
    let mut shift: HashSet<usize> = HashSet::new();
    if let Some(layout) = &edit.file.layout {
        for (index, item) in layout.items.iter().enumerate() {
            let LayoutKind::Wire { from, to, .. } = &item.kind else { continue };
            // From a value a node moved makes, or from the label of one (an order wire).
            let from_held = if from.node.is_label { moved_names.contains(&(from.node.name, true)) } else { held_names.contains(&from.node.name) };
            let to_moved = moved_names.contains(&(to.node.name, to.node.is_label));
            match (from_held, to_moved) {
                (true, true) if at.is_some() => {
                    shift.insert(index);
                }
                (false, false) => {}
                _ => {
                    gone.insert(index);
                }
            }
        }
    }
    let first_old = olds.first().copied().flatten();
    let landed = at.map(|at| Point { x: on_grid(at.x), y: on_grid(at.y) });
    let delta = first_old.zip(landed).map(|(old, new)| Point { x: new.x.saturating_sub(old.x), y: new.y.saturating_sub(old.y) });
    if let Some(mut layout) = edit.file.layout.take() {
        let mut kept = Vec::with_capacity(layout.items.len());
        for (index, mut item) in std::mem::take(&mut layout.items).into_iter().enumerate() {
            if gone.contains(&index) {
                continue;
            }
            // A node moved: its place goes, and the view it is shown in stays (spec 11).
            if entries.contains(&index) {
                match &mut item.kind {
                    LayoutKind::Node { at, view: Some(_), .. } => *at = None,
                    _ => continue,
                }
            }
            if shift.contains(&index) {
                let Some(delta) = delta else { continue };
                if let LayoutKind::Wire { via, .. } = &mut item.kind {
                    for point in via.iter_mut() {
                        let (Some(x), Some(y)) = (value_of(edit, &point.x), value_of(edit, &point.y)) else { continue };
                        point.x = crate::add::coordinate(edit, x.saturating_add(delta.x));
                        point.y = crate::add::coordinate(edit, y.saturating_add(delta.y));
                    }
                }
            }
            kept.push(item);
        }
        layout.items = kept;
        edit.file.layout = Some(layout);
    }

    // The move: out of their frame, into the other, before what a loop body keeps last.
    let mut statements = Vec::with_capacity(locs.len());
    {
        let body = scope::body_mut(edit.body_function_mut()?, &frame).ok_or_else(|| EditError::internal("a frame that went"))?;
        let mut indices: Vec<usize> = locs.iter().map(|loc| loc.index).collect();
        indices.sort_unstable();
        for index in indices.into_iter().rev() {
            if index < body.statements.len() {
                statements.push(body.statements.remove(index));
            }
        }
    }
    statements.sort_by_key(|statement| moved.iter().position(|id| *id == statement.id));
    let to_now = match to_holder {
        None => Vec::new(),
        Some((holder, sub)) => {
            let function = edit.body_function()?;
            let loc = loc_of(function, holder).ok_or_else(|| EditError::internal("a frame that went"))?;
            loc.sub(sub)
        }
    };
    {
        let body = scope::body_mut(edit.body_function_mut()?, &to_now).ok_or_else(|| EditError::internal("a frame that went"))?;
        let at_end = body.statements.iter().rposition(|statement| !loop_only(statement)).map_or(0, |index| index + 1);
        body.statements.splice(at_end..at_end, statements);
    }

    // A name of the new frame that a node moved would hide, or that hides what it reads.
    let after_move = edit.body_function()?.clone();
    let values_now = scope::values(&after_move);
    for (_, name) in &held {
        let twice = values_now.iter().filter(|value| value.name == *name && value.seen_in == to_now).count() > 1;
        let gives_an_output = to_now.is_empty() && !frame.is_empty() && after_move.signature.outputs.iter().any(|output| output.name == *name);
        if gives_an_output {
            return Err(EditError::new(
                "name.taken",
                format!("`{}` is an output of the function: in its body the node would give it (R-2); rename the value first", edit.text(*name)),
            )
            .of("nodes"));
        }
        if twice {
            return Err(EditError::new(
                "name.taken",
                format!("`{}` is a value of that frame already: rename one of them, then move the node", edit.text(*name)),
            )
            .of("nodes"));
        }
    }

    // Lift what the nodes read and their new frame cannot see; then what they make and is read where it cannot be seen.
    let mut levels_of: HashMap<Key, (Name, Vec<(BodyPath, Name)>)> = HashMap::new();
    let mut used_through: BTreeSet<String> = BTreeSet::new();
    let mut outputs: Vec<String> = Vec::new();
    let mut lifted_reads: Vec<(Key, Name)> = Vec::new();
    let function = edit.body_function()?;
    let values = scope::values(function);
    let named = by_name(&values);
    for (place, name, found) in &reads_before {
        let Place::Slot(id, _) = place else { continue };
        if !moved_set.contains(id) || held_keys.contains(found) {
            continue;
        }
        let Some(value) = values.iter().find(|value| key(function, value) == Some(*found)) else {
            return Err(EditError::internal("a value read went away"));
        };
        let now = resolve(function, &named, *name, &to_now);
        if now.as_ref().and_then(|now| key(function, now)) == Some(*found) {
            continue;
        }
        // Seen there, or hidden on the way: another value of its name stands between (R-3).
        if scope::sees(function, value, &to_now) || scope::common_depth(&value.seen_in, &to_now) >= value.seen_in.len() {
            return Err(EditError::new(
                "wire.out_of_scope",
                format!("in that frame, `{}` names another value (R-3): the node would read it in place of the one it reads", edit.text(*name)),
            )
            .of("inside"));
        }
        if !lifted_reads.iter().any(|(known, _)| known == found) {
            lifted_reads.push((*found, *name));
        }
    }
    for (found, name) in &lifted_reads {
        let function = edit.body_function()?;
        let values = scope::values(function);
        let Some(value) = values.iter().find(|value| key(function, value) == Some(*found)).cloned() else {
            return Err(EditError::internal("a value read went away"));
        };
        let depth = scope::common_depth(&value.seen_in, &to_now);
        let given = through_of(edit, through, *name, &mut used_through);
        let before = values_before.iter().find(|known| key_of(known) == Some(*found));
        let levels = lift(edit, judge, &value, before, depth, given, false, edited)?;
        outputs.extend(levels.iter().skip(1).map(|(_, out)| edit.text(*out).to_owned()));
        levels_of.insert(*found, (*name, levels));
    }
    for (found, name) in &held {
        let function = edit.body_function()?;
        let values = scope::values(function);
        let Some(value) = values.iter().find(|value| key(function, value) == Some(*found)).cloned() else {
            return Err(EditError::internal("a value moved went away"));
        };
        let mut depth = value.seen_in.len();
        let at = locs_by_id(function);
        for place in held_readers.get(found).map(Vec::as_slice).unwrap_or_default() {
            // An `after:` is an order wire, judged below: it lifts nothing.
            if let Place::After(_) = place {
                continue;
            }
            let Some(seen_in) = seen_in_of(function, &at, place) else { continue };
            if !scope::sees(function, &value, &seen_in) {
                depth = depth.min(scope::common_depth(&value.seen_in, &seen_in));
            }
        }
        let output = to_output.contains(found);
        if output {
            depth = 0;
        }
        if depth >= value.seen_in.len() {
            continue;
        }
        let given = through_of(edit, through, *name, &mut used_through);
        let before = values_before.iter().find(|known| key_of(known) == Some(*found));
        let levels = lift(edit, judge, &value, before, depth, given, output, edited)?;
        outputs.extend(levels.iter().skip(1).map(|(_, out)| edit.text(*out).to_owned()));
        levels_of.insert(*found, (*name, levels));
    }
    if let Some(unused) = through.keys().find(|name| !used_through.contains(*name)) {
        return Err(EditError::invalid("through", format!("`{}` leaves no loop: `through` names a value that a loop holds", crate::cut(unused))));
    }

    // Each place reads, of a value lifted, the name of the deepest body that holds it and that the place is in.
    let at = locs_by_id(edit.body_function()?);
    for (place, name, found) in &reads_before {
        let Some((_, levels)) = levels_of.get(found) else { continue };
        let reads_it = match place {
            Place::Slot(id, _) => moved_set.contains(id) || held_readers.get(found).is_some_and(|places| places.contains(place)),
            Place::After(_) => false,
        };
        if !reads_it {
            continue;
        }
        let function = edit.body_function()?;
        let Some(seen_in) = seen_in_of(function, &at, place) else { continue };
        let Some((_, wanted)) = levels.iter().filter(|(body, _)| seen_in.starts_with(body)).max_by_key(|(body, _)| body.len()) else {
            return Err(EditError::internal("a reader outside every body of a value lifted"));
        };
        let Place::Slot(id, slot) = place else { continue };
        let Some(loc) = at.get(id).cloned() else { continue };
        let names_now: Vec<Name> = levels.iter().map(|(_, level)| *level).chain([*name]).collect();
        let wanted = *wanted;
        let function = edit.body_function_mut()?;
        if let Some(found_operand) = scope::statement_mut(function, &loc).and_then(|statement| operand_mut(statement, *slot)) {
            for from in names_now {
                if from != wanted {
                    rename_read(found_operand, from, wanted);
                }
            }
        }
    }

    // Each read now finds what it found before; a value moved into a frame that names it hides nothing. Only the names
    // that the move binds, lifts or reads can find another value.
    let function = edit.body_function()?.clone();
    let values = scope::values(&function);
    let named = by_name(&values);
    let at = locs_by_id(&function);
    let mut touched: HashSet<Name> = held.iter().map(|(_, name)| *name).collect();
    for (name, levels) in levels_of.values() {
        touched.insert(*name);
        touched.extend(levels.iter().map(|(_, level)| *level));
    }
    for (place, name, _) in &reads_before {
        if matches!(place, Place::Slot(id, _) if moved_set.contains(id)) {
            touched.insert(*name);
        }
    }
    for (place, name, found) in &reads_before {
        if !touched.contains(name) {
            continue;
        }
        let Some(seen_in) = seen_in_of(&function, &at, place) else { continue };
        let Place::Slot(id, slot) = place else { continue };
        let Some(loc) = at.get(id).cloned() else { continue };
        let Some(statement) = scope::statement(&function, &loc) else { continue };
        let mut names = Vec::new();
        if let Some(read) = operand(statement, *slot) {
            names_in(read, &mut names);
        }
        let read_name =
            levels_of.get(found).and_then(|(_, levels)| levels.iter().map(|(_, level)| *level).find(|level| names.contains(level))).unwrap_or(*name);
        let now = resolve(&function, &named, read_name, &seen_in);
        let expected = levels_of.contains_key(found);
        let same = now.as_ref().and_then(|now| key(&function, now)) == Some(*found);
        let lifted_here = expected && now.as_ref().is_some_and(|now| now.name == read_name);
        if !same && !lifted_here {
            let message = if held_keys.iter().any(|held| now.as_ref().and_then(|now| key(&function, now)) == Some(*held)) {
                format!("in that frame, the node moved would hide `{}` from what reads it: rename one of them first", edit.text(*name))
            } else {
                format!("`{}` would no longer be seen where it is read", edit.text(*name))
            };
            return Err(EditError::new("wire.out_of_scope", message).of("inside"));
        }
        // A place that reads what a structure gives, from inside it.
        if let Some(Bound::Line(made_by, _)) = now.as_ref().map(|now| &now.bound)
            && closes_a_cycle(&function, made_by, &loc)
        {
            return Err(EditError::new(
                "wire.cycle",
                format!("`{}` is made by a structure that the move puts around what reads it: it would read its own output", edit.text(read_name)),
            )
            .of("inside"));
        }
    }
    // A structure around the frames, or that a value left, which would now read what it gives.
    let mut holders: Vec<Id> = around.clone();
    holders.extend(ancestors(&function, &to_now));
    for (_, levels) in levels_of.values() {
        for (body, _) in levels {
            holders.extend(ancestors(&function, body));
        }
    }
    holders.sort_unstable();
    holders.dedup();
    for holder in holders {
        let Some(loc) = loc_of(&function, holder) else { continue };
        if depends_on_itself(&function, &loc) {
            return Err(EditError::new("wire.cycle", "the move would make a structure read what it gives").of("inside"));
        }
    }
    // An order wire does not leave a structure (R-17).
    for (waiter, waited) in &waits {
        let Some(waiter_loc) = at.get(waiter) else { continue };
        let seen = match waited {
            OrderTo::Statement(id) => at.get(id).is_some_and(|loc| {
                waiter_loc.body.starts_with(&loc.body) && waiter_loc.body.get(loc.body.len()).is_none_or(|step| step.stmt != loc.index)
            }),
            OrderTo::Value(name, found) => {
                resolve(&function, &named, *name, &waiter_loc.body).as_ref().and_then(|now| key(&function, now)) == Some(*found)
            }
        };
        if !seen {
            return Err(EditError::new(
                "wire.out_of_scope",
                "an order wire (`after:`) would cross the edge of a structure, which it does not (R-17)",
            )
            .of("inside"));
        }
    }

    // The places: the first node on the grid of its new frame at `at`, the others as far as they were from it.
    let mut labelled = Vec::new();
    let mut names = Vec::with_capacity(moved.len());
    let mut new_locs = Vec::with_capacity(moved.len());
    for id in &moved {
        new_locs.push(loc_of(edit.body_function()?, *id).ok_or_else(|| EditError::internal("a node moved went away"))?);
    }
    for (rank, loc) in new_locs.iter().enumerate() {
        let loc = loc.clone();
        let designated = designate(edit, &loc, &mut labelled)?;
        if let (Some(landed), Some(old), Some(first)) = (landed, olds.get(rank).copied().flatten(), first_old) {
            let at = Point { x: landed.x.saturating_add(old.x.saturating_sub(first.x)), y: landed.y.saturating_add(old.y.saturating_sub(first.y)) };
            fix(edit, &loc, designated, at, "at")?;
        } else if let (Some(landed), 0) = (landed, rank) {
            fix(edit, &loc, designated, landed, "at")?;
        }
        names.push(shown(edit, designated));
    }
    for (loc, label) in &labelled {
        let before = new_locs.iter().position(|found| found == loc);
        if let Some(line) = before.and_then(|rank| locs.get(rank)).and_then(|old| edit.lines.get(old)) {
            edited.renamed.push(Renamed { file: edit.path.clone(), from: format!("@{line}"), to: label.clone() });
        }
    }
    edited.lists.insert("nodes", names);
    edited.lists.insert("outputs", outputs);
    Ok(())
}

/// What an order wire waits for: a node, by its label, or a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OrderTo {
    Statement(Id),
    Value(Name, Key),
}

/// The passage that `through` gives the value `name`, which it then counts as used.
fn through_of<'t>(edit: &Edit, through: &'t BTreeMap<String, Through>, name: Name, used: &mut BTreeSet<String>) -> Option<&'t Through> {
    let text = edit.text(name);
    let found = through.get(text)?;
    used.insert(text.to_owned());
    Some(found)
}

/// A value of a body that the frame `depth` deep does not see: each structure around it gives it, up to that body, as
/// `wire.connect` lifts a wire (commands.md 6.7), the arms of a `match` yielding it and a hole, a loop as `through` says.
/// A value of a line takes a suffix inside, or the name its node would have had when it was bound to an output of the
/// function, and the last output its name, when nothing else bears it. The name it has in each body, from its own.
#[allow(clippy::too_many_arguments)]
fn lift(
    edit: &mut Edit,
    judge: &dyn Judge,
    value: &Value,
    before: Option<&Value>,
    depth: usize,
    through: Option<&Through>,
    bound_to_output: bool,
    edited: &mut Edited,
) -> Result<Vec<(BodyPath, Name)>, EditError> {
    let original = value.name;
    let shown_name = edit.text(original).to_owned();
    let function = edit.body_function()?.clone();
    let mut frames = Vec::new();
    for level in depth + 1..=value.seen_in.len() {
        frames.push(scope::frame(&function, value.seen_in.get(..level).unwrap_or(&[])).ok_or_else(|| EditError::internal("a frame that went"))?);
    }
    let loops = frames.iter().filter(|frame| **frame == Frame::Loop).count();
    let passages: Vec<Passage> = match through {
        None if loops > 0 => {
            return Err(EditError::new(
                "wire.out_of_scope",
                format!("`{shown_name}` would leave a loop: `through` says how, `last`, `collect` or `carry`"),
            )
            .of("through"));
        }
        None => Vec::new(),
        Some(_) if loops == 0 => {
            return Err(EditError::invalid("through", format!("`{shown_name}` leaves no loop: `through` names a value that a loop holds")));
        }
        Some(Through::All(passage)) => vec![*passage; loops],
        Some(Through::Each(list)) if list.len() == loops => list.clone(),
        Some(Through::Each(list)) => {
            return Err(EditError::invalid(
                "through",
                format!("`{shown_name}` leaves {loops} loop(s), and `through` lists {}: one passage per loop, from the inside out", list.len()),
            ));
        }
    };
    // Its type, for a carry, as the check saw it before the move.
    let mut ty = if passages.contains(&Passage::Carry) {
        let found = before.map(|before| crate::surround::type_of(edit, judge, before)).unwrap_or_default();
        (found.kind != TypeKind::Unknown && !found.shown.is_empty()).then_some(found.shown)
    } else {
        None
    };
    // Inside, a value of a line takes another name, and the readers that see it there follow.
    let mut current = original;
    if let Bound::Line(..) = &value.bound {
        let inner = if bound_to_output { node_name_of(edit, judge, &function, value) } else { fresh(&shown_name, &taken(edit)) };
        let inner_name = edit.intern(&inner);
        rename(edit, value, inner_name)?;
        edited.renamed.push(Renamed { file: edit.path.clone(), from: shown_name.clone(), to: inner });
        current = inner_name;
    }
    let mut levels = vec![(value.seen_in.clone(), current)];
    let mut passages = passages.into_iter();
    let mut path = value.seen_in.clone();
    let mut last_structure = None;
    while path.len() > depth {
        let level = path.len() - depth - 1;
        let Some(step) = path.pop() else { break };
        let loc = Loc { body: path.clone(), index: step.stmt };
        current = match frames.get(level) {
            Some(Frame::Arm) => give(edit, &loc, step.sub, current)?,
            Some(Frame::Loop) => {
                let passage = passages.next().ok_or_else(|| EditError::internal("a loop without its passage"))?;
                let out = pass(edit, &loc, current, passage, ty.as_deref())?;
                if passage == Passage::Collect {
                    ty = ty.map(|inner| format!("[{inner}]"));
                }
                out
            }
            None => return Err(EditError::internal("a frame that went")),
        };
        levels.push((path.clone(), current));
        last_structure = Some(loc);
    }
    // The last output takes the name of the value, when nothing else of the function bears it now.
    if let Some(loc) = last_structure
        && current != original
        && (bound_to_output || !taken(edit).contains(&shown_name))
    {
        let function = edit.body_function_mut()?;
        if let Some(StatementKind::Binding { targets, .. }) = scope::statement_mut(function, &loc).map(|statement| &mut statement.kind) {
            for found in targets.iter_mut() {
                if let Target::Name(_, name) = found
                    && *name == current
                {
                    *name = original;
                }
            }
        }
        rename_in_layout(edit, current, original, false);
        if let Some(last) = levels.last_mut() {
            last.1 = original;
        }
    }
    Ok(levels)
}

/// The value that a name read in `seen_in` designates: the nearest one that sees it. Reading the name of an input that is
/// also an output reads the input (R-2).
fn resolve(function: &Function, values: &HashMap<Name, Vec<&Value>>, name: Name, seen_in: &[Step]) -> Option<Value> {
    values
        .get(&name)?
        .iter()
        .filter(|value| !scope::binds_the_output_of_an_input(function, value) && scope::sees(function, value, seen_in))
        .max_by_key(|value| value.seen_in.len())
        .map(|value| (*value).clone())
}

fn key(function: &Function, value: &Value) -> Option<Key> {
    match &value.bound {
        Bound::Input => Some(Key::Input(value.name)),
        Bound::Line(loc, index) => scope::statement(function, loc).map(|statement| Key::Line(statement.id, *index)),
        Bound::Head(loc, sub) => scope::statement(function, loc).map(|statement| Key::Head(statement.id, *sub, value.name)),
    }
}

/// The statement of this id, where it is now.
fn loc_of(function: &Function, id: Id) -> Option<Loc> {
    scope::all(function).into_iter().find(|loc| scope::statement(function, loc).is_some_and(|statement| statement.id == id))
}

/// The values of each name, so that a name is resolved among its own values only.
fn by_name(values: &[Value]) -> HashMap<Name, Vec<&Value>> {
    let mut found: HashMap<Name, Vec<&Value>> = HashMap::new();
    for value in values {
        found.entry(value.name).or_default().push(value);
    }
    found
}

/// Where each statement of the function is, by its id: for a state of it that no statement leaves.
fn locs_by_id(function: &Function) -> HashMap<Id, Loc> {
    scope::all(function).into_iter().filter_map(|loc| scope::statement(function, &loc).map(|statement| (statement.id, loc))).collect()
}

/// The body where a place reads, its statement found in `at`.
fn seen_in_of(function: &Function, at: &HashMap<Id, Loc>, place: &Place) -> Option<BodyPath> {
    match place {
        Place::Slot(id, slot) => {
            let loc = at.get(id)?;
            let statement = scope::statement(function, loc)?;
            slots(statement, loc).into_iter().find(|(found, _)| found == slot).map(|(_, seen_in)| seen_in)
        }
        Place::After(id) => at.get(id).map(|loc| loc.body.clone()),
    }
}

/// The structures that hold the body at `path`, by their ids.
fn ancestors(function: &Function, path: &[Step]) -> Vec<Id> {
    (0..path.len())
        .filter_map(|depth| {
            let step = path.get(depth)?;
            scope::body(function, path.get(..depth)?)?.statements.get(step.stmt).map(|statement| statement.id)
        })
        .collect()
}
