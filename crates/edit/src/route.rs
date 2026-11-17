//! `wire.route` (commands.md 6.7): the `wire … via` entry of a wire (spec 11); and the junction of a branch, which
//! follows its trunk after every command (notation 2.4, 2.5; decision of the owner, 2026-11-17).

use labcraft_graph::{Attached, Layout, LayoutItem, LayoutKind, NodeRef, Numeral, TerminalRef};

use crate::add::coordinate;
use crate::layout::{designate, value_of};
use crate::model::Edit;
use crate::refs::{self, NodeName};
use crate::{EditError, Edited, Judge, MAX_COORDINATE, Point, WireDrawn};

/// At most this many points in one `via` *(proposed)*.
pub const MAX_VIA: usize = 256;

/// The value, the node and the input of a wire as a command names it: `reading -> verdict.a`, `e3 -> err`,
/// `#settle -> second.after`.
fn parts(name: &str) -> Option<(&str, &str, Option<&str>)> {
    let (value, end) = name.split_once("->")?;
    let end = end.trim();
    let (node, input) = match end.split_once('.') {
        Some((node, input)) => (node, Some(input)),
        None => (end, None),
    };
    Some((value.trim(), node, input))
}

/// Whether two names of nodes of `edit` name the same node: the same text, or the label and the name of the line of one
/// node (spec 11).
fn same_node(edit: &Edit, a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let Ok(function) = edit.body_function() else { return false };
    let found = |text: &str| match refs::find_nodes(edit, function, &NodeName::parse(text)).as_slice() {
        [one] => Some(one.clone()),
        _ => None,
    };
    found(a).is_some_and(|loc| found(b) == Some(loc))
}

/// The wires drawn that `value -> node.input` names in `edit`: the input left out, every wire of that value to that node;
/// several, those that reach an input without a name (spec 11).
fn named<'a>(edit: &Edit, drawn: &[&'a WireDrawn], value: &str, node: &str, input: Option<&str>) -> Vec<&'a WireDrawn> {
    let found: Vec<&WireDrawn> = drawn
        .iter()
        .copied()
        .filter(|candidate| {
            parts(&candidate.name).is_some_and(|(v, n, i)| v == value && input.is_none_or(|input| Some(input) == i) && same_node(edit, n, node))
        })
        .collect();
    if found.len() > 1 && input.is_none() {
        let without: Vec<&WireDrawn> =
            found.iter().copied().filter(|candidate| parts(&candidate.name).is_some_and(|(_, _, i)| i.is_none())).collect();
        if without.len() == 1 {
            return without;
        }
    }
    found
}

/// The drawn wires of each value, so that a name costs the wires of its value, not all of them.
pub(crate) type ByValue<'a> = std::collections::HashMap<&'a str, Vec<&'a WireDrawn>>;

pub(crate) fn by_value(drawn: &[WireDrawn]) -> ByValue<'_> {
    let mut found = ByValue::new();
    for wire in drawn {
        if let Some((value, _, _)) = parts(&wire.name) {
            found.entry(value).or_default().push(wire);
        }
    }
    found
}

/// The wire drawn that `wire` names: by its name, or by its value and its node alone when one wire joins them, or one
/// of them reaches an input without a name.
fn chosen<'a>(edit: &Edit, of_value: &ByValue<'a>, wire: &str) -> Result<&'a WireDrawn, EditError> {
    let Some((value, node, input)) = parts(wire) else {
        return Err(EditError::invalid("wire", "a wire is named `value -> node` or `value -> node.input`"));
    };
    let found = named(edit, of_value.get(value).map_or(&[][..], Vec::as_slice), value, node, input);
    match found.as_slice() {
        [one] => Ok(one),
        [] => Err(EditError::new("edit.unknown", format!("no wire `{value} -> {}` is drawn in this function", end_of(node, input))).of("wire")),
        several => Err(EditError::new("edit.ambiguous", format!("`{value}` reaches `{node}` by several wires: name the input"))
            .of("wire")
            .expecting(several.iter().map(|candidate| candidate.name.clone()).collect())),
    }
}

fn end_of(node: &str, input: Option<&str>) -> String {
    match input {
        Some(input) => format!("{node}.{input}"),
        None => node.to_owned(),
    }
}

/// A name of the file as the `layout` section writes it: `#close` or `dmm`.
fn node_ref(edit: &mut Edit, text: &str) -> NodeRef {
    let (is_label, name) = match text.strip_prefix('#') {
        Some(label) => (true, label),
        None => (false, text),
    };
    let name = edit.intern(name);
    NodeRef { id: edit.id(), is_label, name }
}

fn shown(edit: &Edit, node: &NodeRef) -> String {
    if node.is_label { format!("#{}", edit.text(node.name)) } else { edit.text(node.name).to_owned() }
}

/// An entry of `layout` in text: its value, its node, its input.
type Texts = (String, String, Option<String>);

fn texts_of(edit: &Edit, from: &TerminalRef, to: &TerminalRef) -> Texts {
    (shown(edit, &from.node), shown(edit, &to.node), to.terminal.map(|name| edit.text(name).to_owned()))
}

/// The wire drawn that an entry designates, read as spec 11 reads it, its nodes read in `edit`: none when it designates
/// none, or several.
fn designated<'a>(edit: &Edit, of_value: &ByValue<'a>, (value, node, input): &Texts) -> Option<&'a WireDrawn> {
    match named(edit, of_value.get(value.as_str()).map_or(&[][..], Vec::as_slice), value, node, input.as_deref()).as_slice() {
        [one] => Some(*one),
        _ => None,
    }
}

/// The name of the wire drawn that `wire`, as a command names it, designates; none when it designates none, or several.
pub(crate) fn resolve(edit: &Edit, of_value: &ByValue<'_>, wire: &str) -> Option<String> {
    chosen(edit, of_value, wire).ok().map(|found| found.name.clone())
}

/// Whether the entry `from -> to` of `edit` is the one of `wire`, as `layout.arrange` takes it: the entry designates the
/// wire drawn that `wire` designates, `drawn` (spec 11); with no drawing, the same value, node and input.
pub(crate) fn entry_names(edit: &Edit, of_value: &ByValue<'_>, from: &TerminalRef, to: &TerminalRef, wire: &str, drawn: Option<&str>) -> bool {
    let texts = texts_of(edit, from, to);
    if let Some(drawn) = drawn {
        return designated(edit, of_value, &texts).is_some_and(|found| found.name == drawn);
    }
    let Some((v, n, i)) = parts(wire) else { return false };
    v == texts.0 && texts.2.as_deref() == i && same_node(edit, n, &texts.1)
}

/// `wire.route`: the wire named goes through the points of `via`, in the frame of the wire (spec 11): its `wire … via`
/// entry, made or changed. The node it reaches gets a label when the section could not name it otherwise.
pub(crate) fn route(edit: &mut Edit, judge: &dyn Judge, wire: &str, via: &[Point], edited: &mut Edited) -> Result<(), EditError> {
    if via.is_empty() {
        return Err(EditError::invalid("via", "`via` gives at least one point; `layout.arrange` with `wires` gives a wire its automatic route back"));
    }
    if via.len() > MAX_VIA {
        return Err(EditError::invalid("via", format!("a route has at most {MAX_VIA} points")));
    }
    let range = -MAX_COORDINATE..=MAX_COORDINATE;
    if via.iter().any(|point| !range.contains(&point.x) || !range.contains(&point.y)) {
        return Err(EditError::invalid("via", format!("a coordinate is between -{MAX_COORDINATE} and {MAX_COORDINATE}")));
    }
    let drawn = judge.wires(&edit.path, &edit.original);
    if drawn.is_empty() {
        return Err(EditError::invalid("wire", "the function is not drawn now, so no wire of it can be routed"));
    }
    let of_value = by_value(&drawn);
    let found = chosen(edit, &of_value, wire)?.clone();
    let Some((value, node, input)) = parts(&found.name) else { return Err(EditError::internal("a wire drawn with no name")) };
    if input.is_some_and(|input| !labcraft_format::is_name(input)) {
        return Err(EditError::invalid("wire", "the `layout` section does not number the values of a variadic parameter yet (spec 11)"));
    }
    // The input only when the value reaches that node by several wires (spec 11).
    let alone = of_value
        .get(value)
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .filter(|other| parts(&other.name).is_some_and(|(_, n, _)| same_node(edit, n, node)))
        .count()
        == 1;
    let (value, node, input) = (value.to_owned(), node.to_owned(), input.map(str::to_owned));
    let function = edit.body_function()?;
    let output = input.is_none() && function.signature.outputs.iter().any(|output| edit.text(output.name) == node) && {
        let name = NodeName::parse(&node);
        refs::find_nodes(edit, function, &name).is_empty()
    };
    let to_node = if output {
        node_ref(edit, &node)
    } else {
        let loc = refs::node(edit, function, &NodeName::parse(&node), "wire")?;
        let mut labelled = Vec::new();
        let designation = designate(edit, &loc, &mut labelled)?;
        for (loc, label) in labelled {
            if let Some(line) = edit.lines.get(&loc) {
                edited.renamed.push(crate::Renamed { file: edit.path.clone(), from: format!("@{line}"), to: label });
            }
        }
        NodeRef { id: edit.id(), is_label: designation.is_label, name: designation.name }
    };
    let terminal = match input.as_deref() {
        Some(input) if !alone => Some(edit.intern(input)),
        _ => None,
    };
    let from = TerminalRef { id: edit.id(), node: node_ref(edit, &value), terminal: None };
    let to = TerminalRef { id: edit.id(), node: to_node, terminal };
    let points: Vec<labcraft_graph::Point> =
        via.iter().map(|point| labcraft_graph::Point { id: edit.id(), x: coordinate(edit, point.x), y: coordinate(edit, point.y) }).collect();
    let (layout_id, item_id) = (edit.id(), edit.id());
    let mut layout = edit.file.layout.take().unwrap_or(Layout { id: layout_id, comments: Attached::default(), items: Vec::new(), end: Vec::new() });
    // The entries that designate this wire go: a wire has one entry (`LC5007`); the new one takes the place of the first.
    // An entry without its input designates the wire that spec 11 gives it, not every wire of its value to that node.
    let mut at = None;
    let mut index = 0;
    layout.items.retain(|item| {
        let keep = match &item.kind {
            LayoutKind::Wire { from: old_from, to: old_to, .. } => {
                designated(edit, &of_value, &texts_of(edit, old_from, old_to)).is_none_or(|other| other.name != found.name)
            }
            LayoutKind::Node { .. } => true,
        };
        if !keep && at.is_none() {
            at = Some(index);
        }
        if keep {
            index += 1;
        }
        keep
    });
    let item = LayoutItem { id: item_id, comments: Attached::default(), kind: LayoutKind::Wire { from, to, via: points } };
    match at {
        Some(at) if at <= layout.items.len() => layout.items.insert(at, item),
        _ => layout.items.push(item),
    }
    edit.file.layout = Some(layout);
    edited.names.insert("wire", found.name.clone());
    Ok(())
}

/// Where a junction lies on its trunk: the trunk by its name, the index of its segment, the distance from the start of
/// that segment, in hundredths, and how many segments the trunk has.
struct OnTrunk {
    entry: usize,
    trunk: String,
    segment: usize,
    along: i64,
    segments: usize,
}

/// The segment of `path` that `at` lies on first, and how far along it.
fn segment_of(path: &[(i64, i64)], at: (i64, i64)) -> Option<(usize, i64)> {
    path.windows(2).enumerate().find_map(|(index, pair)| {
        let (&a, &b) = (pair.first()?, pair.get(1)?);
        let within = |p: i64, q: i64, r: i64| p.min(q) <= r && r <= p.max(q);
        let on = (a.0 == b.0 && at.0 == a.0 && within(a.1, b.1, at.1)) || (a.1 == b.1 && at.1 == a.1 && within(a.0, b.0, at.0));
        on.then(|| (index, (at.0 - a.0).abs() + (at.1 - a.1).abs()))
    })
}

/// The point `along` from the start of the segment `segment` of `path`, cut to its length; none when the path has another
/// number of segments, or that segment another direction.
fn along_segment(path: &[(i64, i64)], segment: usize, along: i64, segments: usize, vertical: bool) -> Option<(i64, i64)> {
    if path.len().saturating_sub(1) != segments {
        return None;
    }
    let (&a, &b) = (path.get(segment)?, path.get(segment + 1)?);
    if (a.0 == b.0) != vertical {
        return None;
    }
    let length = (b.0 - a.0).abs() + (b.1 - a.1).abs();
    let step = along.min(length);
    Some((a.0 + (b.0 - a.0).signum() * step, a.1 + (b.1 - a.1).signum() * step))
}

/// An entry with waypoints: its index in the section, its two ends and its points in units.
type Routed = (usize, TerminalRef, TerminalRef, Vec<(i64, i64)>);

/// An entry with waypoints in text: its index in the section, its ends, its points in units.
type Kept = (usize, Texts, Vec<(i64, i64)>);

/// The entries with waypoints of a file.
fn routes(edit: &Edit) -> Vec<Routed> {
    let Some(layout) = &edit.file.layout else { return Vec::new() };
    layout
        .items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let LayoutKind::Wire { from, to, via } = &item.kind else { return None };
            if via.is_empty() {
                return None;
            }
            let points = via.iter().map(|point| Some((value_of(edit, &point.x)?, value_of(edit, &point.y)?))).collect::<Option<Vec<_>>>()?;
            Some((index, *from, *to, points))
        })
        .collect()
}

/// After a command, each junction follows its trunk (notation 2.5): the first point of a branch that lay on a wire of
/// its value keeps its distance from the start of that segment of the trunk, cut to its length; when the trunk no
/// longer has that segment, the point goes, and the junction is automatic again. A branch whose entry the command
/// changed is as the command left it. Part of the same step as the command.
pub(crate) fn follow(path: &str, before: &str, after: String, judge: &dyn Judge, limits: &labcraft_types::Limits) -> Result<String, EditError> {
    if before == after {
        return Ok(after);
    }
    let mut edit = Edit::read(path, &after, limits)?;
    let now = routes(&edit);
    if now.is_empty() {
        return Ok(after);
    }
    let old = Edit::read(path, before, limits)?;
    // The entries in text, each read in its own file: the two texts do not share their names.
    let was: std::collections::HashSet<(Texts, Vec<(i64, i64)>)> =
        routes(&old).into_iter().map(|(_, from, to, points)| (texts_of(&old, &from, &to), points)).collect();
    // The entries that the command left as they were.
    let kept: Vec<Kept> = now
        .into_iter()
        .map(|(entry, from, to, points)| (entry, texts_of(&edit, &from, &to), points))
        .filter(|(_, texts, points)| was.contains(&(texts.clone(), points.clone())))
        .collect();
    if kept.is_empty() {
        return Ok(after);
    }
    let drawn_before = judge.wires(path, before);
    let of_value = by_value(&drawn_before);
    let branches: Vec<(usize, &WireDrawn, (i64, i64))> = kept
        .iter()
        .filter_map(|(entry, texts, points)| {
            let &(x, y) = points.first()?;
            Some((*entry, designated(&old, &of_value, texts)?, (x.saturating_mul(100), y.saturating_mul(100))))
        })
        .collect();
    // A branch runs along its trunk up to its junction: the trunk of another branch is a wire without a junction of its
    // own, when one goes through that point, since the path of a branch follows its junction only after this pass.
    let with_junction: std::collections::HashSet<&str> = branches.iter().map(|(_, branch, _)| branch.name.as_str()).collect();
    let mut junctions: Vec<(OnTrunk, bool, (i64, i64))> = Vec::new();
    for (entry, branch, at) in branches {
        let wires = of_value.get(branch.value.as_str()).map_or(&[][..], Vec::as_slice);
        let on = |other: &&WireDrawn| other.body == branch.body && other.name != branch.name;
        let trunk = wires
            .iter()
            .copied()
            .filter(on)
            .filter(|other| !with_junction.contains(other.name.as_str()))
            .chain(wires.iter().copied().filter(on))
            .find_map(|other| segment_of(&other.path, at).map(|(segment, along)| (other, segment, along)));
        let Some((trunk, segment, along)) = trunk else { continue };
        let vertical = matches!((trunk.path.get(segment), trunk.path.get(segment + 1)), (Some(a), Some(b)) if a.0 == b.0);
        let segments = trunk.path.len().saturating_sub(1);
        junctions.push((OnTrunk { entry, trunk: trunk.name.clone(), segment, along, segments }, vertical, at));
    }
    if junctions.is_empty() {
        return Ok(after);
    }
    let drawn_after = judge.wires(path, &after);
    if drawn_after.is_empty() {
        return Ok(after);
    }
    let by_name: std::collections::HashMap<&str, &WireDrawn> = drawn_after.iter().map(|drawn| (drawn.name.as_str(), drawn)).collect();
    let mut changed = false;
    let mut emptied = Vec::new();
    let unit = |hundredths: i64| (hundredths + 50).div_euclid(100);
    for (junction, vertical, at) in junctions {
        // At the same distance along the same segment; or where it was, when it still lies on the trunk (a part of it
        // that did not move); otherwise it goes. A point that the file cannot write on the trunk, off the grid of units,
        // goes too.
        let moved = by_name.get(junction.trunk.as_str()).and_then(|trunk| {
            along_segment(&trunk.path, junction.segment, junction.along, junction.segments, vertical)
                .or_else(|| segment_of(&trunk.path, at).map(|_| at))
                .map(|(x, y)| (unit(x), unit(y)))
                .filter(|&(x, y)| segment_of(&trunk.path, (x.saturating_mul(100), y.saturating_mul(100))).is_some())
        });
        let first: Option<(Numeral, Numeral)> = moved.map(|(x, y)| (coordinate(&mut edit, x), coordinate(&mut edit, y)));
        let Some(layout) = edit.file.layout.as_mut() else { break };
        let Some(LayoutItem { kind: LayoutKind::Wire { via, .. }, .. }) = layout.items.get_mut(junction.entry) else { continue };
        match first {
            Some((x, y)) => {
                if let Some(point) = via.first_mut()
                    && (point.x != x || point.y != y)
                {
                    point.x = x;
                    point.y = y;
                    changed = true;
                }
            }
            None => {
                if !via.is_empty() {
                    via.remove(0);
                    changed = true;
                    if via.is_empty() {
                        emptied.push(junction.entry);
                    }
                }
            }
        }
    }
    if !changed {
        return Ok(after);
    }
    // The entries that this pass left with no point go; one that the file writes with none stays.
    if let Some(layout) = edit.file.layout.as_mut() {
        let mut index = 0;
        layout.items.retain(|_| {
            index += 1;
            !emptied.contains(&(index - 1))
        });
    }
    crate::layout::drop_empty(&mut edit);
    edit.finish()
}
