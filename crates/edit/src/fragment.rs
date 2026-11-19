//! `fragment.copy` and `fragment.paste` (commands.md 6.7, visual notation 2.6): nodes as the text of a function
//! `clipboard`, in canonical form, and the body of such a text put into a frame. The text is a file of the language, so
//! that an agent, a text editor or a message reads it; what the selection reads from outside it is left out, or gets a
//! hole (9.1), and what the fragment names that the function has already gets a suffix (N-14).
//!
//! Each runs in a time near linear in the size of the function: the nodes are found by how `layout` names them through
//! one index, and the renames of a paste are one pass.

use std::collections::{BTreeSet, HashMap, HashSet};

use labcraft_format::{Lexer, TokenKind, canonicalize, read};
use labcraft_graph::{
    Arg, Attached, Body, Decl, File, Function, Layout, LayoutItem, LayoutKind, NodeRef, Operand, OrderRef, Signature, StatementKind, Target, Use,
    View,
};
use labcraft_types::{Name, Report};

use crate::add::{coordinate, in_range};
use crate::delete::{label_readers, readers, take_out};
use crate::layout::{Designation, shown, value_of};
use crate::model::Edit;
use crate::names::{fresh, taken};
use crate::places::{args, args_mut, bracket_name, operand, operand_mut, slots};
use crate::refs::{NodeName, node};
use crate::scope::{self, Bound, Loc, Step, Value};
use crate::{EditError, Edited, Judge, MAX_COORDINATE, Point, Renamed};

/// The largest text of a fragment, copied or pasted *(proposed)*: above it, `file.too_large`, never cut.
pub const MAX_FRAGMENT: usize = 4 << 20;

/// The name of the function of a copied text.
const CLIPBOARD: &str = "clipboard";

fn too_large(parameter: &'static str) -> EditError {
    EditError::new("file.too_large", format!("the text of the fragment is over the limit of {MAX_FRAGMENT} bytes")).of(parameter)
}

/// The statements of a function by how the `layout` section names a node (spec 11): by its label, or by the name of
/// its line, each with every statement that bears it. A `collect` or an `until` is no node.
pub(crate) type Index = HashMap<(bool, Name), Vec<Loc>>;

pub(crate) fn index(function: &Function) -> Index {
    let mut found: Index = HashMap::new();
    for loc in scope::all(function) {
        let Some(statement) = scope::statement(function, &loc) else { continue };
        if let Some(label) = statement.label {
            found.entry((true, label.name)).or_default().push(loc.clone());
        }
        if !matches!(statement.kind, StatementKind::Gather(_, _) | StatementKind::Until(_))
            && let Some(name) = scope::line_name(statement)
        {
            found.entry((false, name)).or_default().push(loc);
        }
    }
    found
}

/// The one statement that a node of `layout` names, if it names one.
pub(crate) fn named_by<'i>(index: &'i Index, node: &NodeRef) -> Option<&'i Loc> {
    match index.get(&(node.is_label, node.name)).map(Vec::as_slice) {
        Some([one]) => Some(one),
        _ => None,
    }
}

/// How the `layout` section names the node at `loc`, as `layout::designation` does, through the index.
pub(crate) fn designated(index: &Index, function: &Function, loc: &Loc) -> Option<Designation> {
    let statement = scope::statement(function, loc)?;
    if let Some(label) = statement.label {
        return Some(Designation { is_label: true, name: label.name });
    }
    let name = scope::line_name(statement)?;
    matches!(index.get(&(false, name)).map(Vec::as_slice), Some([one]) if one == loc).then_some(Designation { is_label: false, name })
}

/// The `node` entry of a node of the fragment.
fn node_item(edit: &mut Edit, named: Designation, at: Point, view: Option<View>) -> LayoutItem {
    let node = NodeRef { id: edit.id(), is_label: named.is_label, name: named.name };
    let point = labcraft_graph::Point { id: edit.id(), x: coordinate(edit, at.x), y: coordinate(edit, at.y) };
    LayoutItem { id: edit.id(), comments: Attached::default(), kind: LayoutKind::Node { node, at: Some(point), view } }
}

/// The `frame … size` entry of a structure of the fragment.
fn frame_item(edit: &mut Edit, named: Designation, size: Point) -> LayoutItem {
    let node = NodeRef { id: edit.id(), is_label: named.is_label, name: named.name };
    let size = labcraft_graph::Point { id: edit.id(), x: coordinate(edit, size.x), y: coordinate(edit, size.y) };
    LayoutItem { id: edit.id(), comments: Attached::default(), kind: LayoutKind::Frame { node: Some(node), size } }
}

/// A point of `layout` moved by `[dx, dy]`, within the bounds of a coordinate or `params.invalid` of `parameter`; one that
/// is not an integer stays as written (`LC5005`).
fn shifted(edit: &mut Edit, point: &mut labcraft_graph::Point, by: Point, parameter: &'static str) -> Result<(), EditError> {
    let (Some(x), Some(y)) = (value_of(edit, &point.x), value_of(edit, &point.y)) else { return Ok(()) };
    let (x, y) = (x.saturating_add(by.x), y.saturating_add(by.y));
    in_range(Point { x, y }, parameter)?;
    point.x = coordinate(edit, x);
    point.y = coordinate(edit, y);
    Ok(())
}

/// The libraries that a text calls or names: each name of the text followed by `.`. A string or a comment holds no name.
fn named_before_dot(edit: &Edit, text: &str) -> HashSet<Name> {
    let mut names = edit.names.clone();
    let mut report = Report::for_limits(edit.limits());
    let mut found = HashSet::new();
    let mut previous: Option<Name> = None;
    for token in Lexer::new(text, &mut names, &mut report) {
        let Ok(token) = token else { break };
        match token.kind {
            TokenKind::Dot => {
                if let Some(name) = previous {
                    found.insert(name);
                }
                previous = None;
            }
            TokenKind::Name(name) => previous = Some(name),
            _ => previous = None,
        }
    }
    found
}

/// The name by which the code calls a library that a `use` line brings: its last segment.
fn used_as(used: &Use) -> Name {
    used.path.rest.last().copied().unwrap_or(used.path.first)
}

/// The `use` lines of `uses` whose library the text of the file of `edit` names.
fn needed(edit: &Edit, uses: &[Use]) -> Result<Vec<Use>, EditError> {
    let named = named_before_dot(edit, &edit.render()?);
    Ok(uses.iter().filter(|used| named.contains(&used_as(used))).cloned().collect())
}

/// The names that an operand reads: each value, the first segment of a path (`step` in `step.low`), `[x]`. With `bare`,
/// only a name alone, as a value is read (`x`, not the `Verdict` of `Verdict.Pass`).
fn read_names(operand: &Operand, bare: bool, found: &mut Vec<Name>) {
    match operand {
        Operand::Path(path) if !bare || path.rest.is_empty() => found.push(path.first),
        Operand::Bracket(_, unit) if !bare => found.extend(bracket_name(unit)),
        Operand::Array(_, values) => values.iter().for_each(|value| read_names(value, bare, found)),
        Operand::Path(_) | Operand::Bracket(..) => {}
        Operand::Number(..) | Operand::Text(..) | Operand::Bool(..) | Operand::None(_) | Operand::Hole(_) => {}
    }
}

/// Every name that the function reads, in its operands and its `after:`, and every label that an `after:` waits for,
/// each once, in the order of the text.
fn reads_of(function: &Function, bare: bool) -> (Vec<Name>, Vec<Name>) {
    let (mut values, mut labels) = (Vec::new(), Vec::new());
    for loc in scope::all(function) {
        let Some(statement) = scope::statement(function, &loc) else { continue };
        for (slot, _) in slots(statement, &loc) {
            if let Some(read) = operand(statement, slot) {
                read_names(read, bare, &mut values);
            }
        }
        for arg in args(statement).unwrap_or_default() {
            let Arg::After { refs, .. } = arg else { continue };
            for reference in refs {
                match reference {
                    OrderRef::Value(read) => read_names(read, bare, &mut values),
                    OrderRef::Label(label) => labels.push(label.name),
                }
            }
        }
    }
    once(&mut values);
    once(&mut labels);
    (values, labels)
}

/// The nodes that `nodes` names, each once, through the index: by its label or the name of its line; `@N`, a name that
/// names no node or several, as `refs::node` judges it, which says why.
fn resolve(edit: &Edit, nodes_of: &Index, nodes: &[String]) -> Result<Vec<Loc>, EditError> {
    let function = edit.body_function()?;
    let mut found: Vec<Loc> = Vec::with_capacity(nodes.len());
    let mut seen = HashSet::new();
    for text in nodes {
        let parsed = NodeName::parse(text.trim());
        if parsed.text.is_empty() {
            return Err(EditError::invalid("nodes", "a node is named by its label (`#close`), the name of its line, or `@N`"));
        }
        let one = match (parsed.line, edit.names.find(&parsed.text)) {
            (None, Some(name)) => match nodes_of.get(&(parsed.is_label, name)).map(Vec::as_slice) {
                Some([one]) => Some(one.clone()),
                _ => None,
            },
            _ => None,
        };
        let loc = match one {
            Some(loc) => loc,
            None => node(edit, function, &parsed, "nodes")?,
        };
        if seen.insert(loc.clone()) {
            found.push(loc);
        }
    }
    Ok(found)
}

/// Whether a line belongs only in the body of a loop: `next x = …`, `collect`, `until`.
fn loop_only(statement: &labcraft_graph::Statement) -> bool {
    match &statement.kind {
        StatementKind::Gather(_, _) | StatementKind::Until(_) => true,
        StatementKind::Binding { targets, .. } => targets.iter().any(|target| matches!(target, Target::Next(..))),
        StatementKind::Call(_) => false,
    }
}

/// `fragment.copy`: the text of the function `clipboard` that holds the nodes named, and what they hold.
pub(crate) fn copy(edit: &Edit, judge: &dyn Judge, nodes: &[String]) -> Result<String, EditError> {
    let function = edit.body_function()?;
    let nodes_of = index(function);
    let mut outer = resolve(edit, &nodes_of, nodes)?;
    outer.sort();
    let Some(body_path) = outer.first().map(|first| first.body.clone()) else {
        return Err(EditError::invalid("nodes", "`nodes` names at least one node"));
    };
    if outer.iter().any(|loc| loc.body != body_path) {
        return Err(EditError::invalid("nodes", "the nodes copied are in one frame: those of another frame are copied apart"));
    }
    let frame = body_path.len();
    if outer.iter().filter_map(|loc| scope::statement(function, loc)).any(loop_only) {
        return Err(EditError::invalid(
            "nodes",
            "a `next`, a `collect` or an `until` is copied with its loop: the text of a copy is the body of a function, which holds none (R-6)",
        ));
    }
    // Each selected statement by its index in its body, with its place in the function `clipboard`.
    let position: HashMap<usize, usize> = outer.iter().enumerate().map(|(at, loc)| (loc.index, at)).collect();
    // The statement of the selection that holds `loc`, or is it, by its index.
    let selected_of = |loc: &Loc| -> Option<usize> {
        if !loc.body.starts_with(&body_path) {
            return None;
        }
        let index = if loc.body.len() == frame { loc.index } else { loc.body.get(frame)?.stmt };
        position.contains_key(&index).then_some(index)
    };
    let held: HashSet<Loc> = scope::all(function).into_iter().filter(|loc| selected_of(loc).is_some()).collect();

    let values = scope::values(function);
    let bound_inside: HashSet<Name> = values
        .iter()
        .filter(|value| match &value.bound {
            Bound::Line(loc, _) | Bound::Head(loc, _) => held.contains(loc),
            Bound::Input => false,
        })
        .map(|value| value.name)
        .collect();
    let labels_inside: HashSet<Name> = scope::labels(function).into_iter().filter(|(_, loc)| held.contains(loc)).map(|(label, _)| label).collect();

    // The entries of `layout` of what is copied, each read once: the place and the view of a node, the route of a wire
    // between the nodes selected or inside what they hold (an order wire from a label too), with whether it is in the
    // frame of the selection.
    let mut entries: HashMap<Loc, (Option<Point>, Option<View>)> = HashMap::new();
    let mut sizes: HashMap<Loc, Point> = HashMap::new();
    let mut wires: Vec<(LayoutItem, bool)> = Vec::new();
    for item in edit.file.layout.iter().flat_map(|layout| &layout.items) {
        match &item.kind {
            LayoutKind::Frame { node: None, .. } | LayoutKind::Terminal { .. } => {}
            LayoutKind::Frame { node: Some(node), size } => {
                let Some(loc) = named_by(&nodes_of, node).filter(|loc| held.contains(*loc)) else { continue };
                if let (Some(x), Some(y)) = (value_of(edit, &size.x), value_of(edit, &size.y)) {
                    sizes.insert(loc.clone(), Point { x, y });
                }
            }
            LayoutKind::Node { node, at, view } => {
                let Some(loc) = named_by(&nodes_of, node).filter(|loc| held.contains(*loc)) else { continue };
                let at = at.as_ref().and_then(|point| Some(Point { x: value_of(edit, &point.x)?, y: value_of(edit, &point.y)? }));
                entries.insert(loc.clone(), (at, *view));
            }
            LayoutKind::Wire { from, to, .. } => {
                let Some(loc) = named_by(&nodes_of, &to.node).filter(|loc| held.contains(*loc)) else { continue };
                let source = if from.node.is_label { &labels_inside } else { &bound_inside };
                if source.contains(&from.node.name) {
                    wires.push((item.clone(), loc.body.len() == frame));
                }
            }
        }
    }

    // Where each node is, before anything moves: its entry, or where the drawing places it; none when it is not drawn.
    let free = held.iter().any(|loc| entries.get(loc).is_none_or(|(fixed, _)| fixed.is_none()));
    let drawn = if free { judge.places(&edit.path, &edit.original) } else { Vec::new() };
    let mut on_line: HashMap<u32, Vec<&crate::Place>> = HashMap::new();
    let mut by_name: HashMap<&str, &crate::Place> = HashMap::new();
    for place in &drawn {
        on_line.entry(place.line).or_default().push(place);
        by_name.entry(place.name.as_str()).or_insert(place);
    }
    let mut at: Vec<(Loc, Point, Option<View>)> = Vec::new();
    let mut order: Vec<&Loc> = held.iter().collect();
    order.sort();
    for loc in order {
        let (fixed, view) = entries.get(loc).copied().unwrap_or_default();
        let point = fixed.or_else(|| {
            // As `layout::drawn_at` finds it.
            let name = designated(&nodes_of, function, loc).map(|named| shown(edit, named));
            let here = edit.lines.get(loc).and_then(|line| on_line.get(line)).map(Vec::as_slice).unwrap_or_default();
            crate::layout::pick_place(here, name.as_deref(), |name| by_name.get(name).copied()).map(|place| place.at)
        });
        let Some(point) = point else { continue };
        at.push((loc.clone(), point, view));
    }
    // The selected ones are counted from the top-left corner of the selection.
    let corner =
        |pick: fn(&Point) -> i64| at.iter().filter(|(loc, _, _)| loc.body.len() == frame).map(|(_, point, _)| pick(point)).min().unwrap_or(0);
    let origin = Point { x: corner(|point| point.x), y: corner(|point| point.y) };

    // The function `clipboard`: the selected statements, in the order of the text.
    let statements = outer.iter().filter_map(|loc| scope::statement(function, loc).cloned()).collect();
    let mut clip = edit.derived(File {
        version: 1,
        header_comment: None,
        use_comments: Vec::new(),
        uses: Vec::new(),
        decl: Decl::Function(Function {
            id: function.id,
            comments: Attached::default(),
            name: function.name,
            signature: Signature { generics: Vec::new(), params: Vec::new(), params_end: Vec::new(), outputs: Vec::new(), outputs_end: Vec::new() },
            open: None,
            body: Body { statements, end: Vec::new() },
        }),
        panel: None,
        layout: None,
        icon: None,
        end: Vec::new(),
    });
    let name = clip.intern(CLIPBOARD);
    clip.body_function_mut()?.name = name;

    // What the selection reads from outside: left out, or a hole (9.1). A value that a node outside reads is no output.
    let (read, waited) = reads_of(clip.body_function()?, false);
    let (read, waited): (HashSet<Name>, HashSet<Name>) = (read.into_iter().collect(), waited.into_iter().collect());
    let top: HashSet<Name> = clip.body_function()?.body.statements.iter().flat_map(scope::targets).collect();
    let mut outside: Vec<Name> = values
        .iter()
        .filter(|value| match &value.bound {
            Bound::Line(loc, _) | Bound::Head(loc, _) => !held.contains(loc),
            Bound::Input => true,
        })
        .map(|value| value.name)
        .filter(|name| read.contains(name) && !top.contains(name))
        .collect();
    once(&mut outside);
    let labels: Vec<Name> =
        scope::labels(function).into_iter().filter(|(label, loc)| !held.contains(loc) && waited.contains(label)).map(|(label, _)| label).collect();
    take_out_all(&mut clip, judge, outside, labels)?;

    // Its `layout`: a `node … at` entry for each node that has a name, and the routes of the wires between them.
    let clip_function = clip.body_function()?.clone();
    let clip_nodes = index(&clip_function);
    let mut items = Vec::new();
    for (loc, point, view) in at {
        let Some(selected) = selected_of(&loc) else { continue };
        let Some(&place_index) = position.get(&selected) else { continue };
        // In the function `clipboard`, the body path of `loc` starts at the selected statement, now at `place_index`.
        let mut body: Vec<Step> = loc.body.get(frame..).map(<[Step]>::to_vec).unwrap_or_default();
        let place = match body.first_mut() {
            Some(first) => {
                first.stmt = place_index;
                Loc { body, index: loc.index }
            }
            None => Loc { body, index: place_index },
        };
        let Some(named) = designated(&clip_nodes, &clip_function, &place) else { continue };
        let point = if loc.body.len() == frame { Point { x: point.x - origin.x, y: point.y - origin.y } } else { point };
        if in_range(point, "nodes").is_err() {
            return Err(EditError::invalid(
                "nodes",
                format!("the nodes selected are more than {MAX_COORDINATE} apart: a copy counts them from their top-left corner"),
            ));
        }
        items.push(node_item(&mut clip, named, point, view));
        if let Some(size) = sizes.get(&loc).copied() {
            items.push(frame_item(&mut clip, named, size));
        }
    }
    for (mut item, of_selection) in wires {
        if of_selection && let LayoutKind::Wire { via, .. } = &mut item.kind {
            for point in via.iter_mut() {
                shifted(&mut clip, point, Point { x: -origin.x, y: -origin.y }, "nodes")?;
            }
        }
        items.push(item);
    }
    if !items.is_empty() {
        let id = clip.id();
        clip.file.layout = Some(Layout { id, comments: Attached::default(), items, end: Vec::new() });
    }

    // The `use` lines that it needs: those of the file whose library the text names.
    clip.file.uses = needed(&clip, &edit.file.uses)?;
    let text = clip.render()?;
    if text.len() > MAX_FRAGMENT {
        return Err(too_large("nodes"));
    }
    Ok(text)
}

/// Takes out of the function of `edit` each value and each label that it reads without binding them: left out, or a
/// hole (9.1), as `node.delete` takes out what a node gave. Each on the function as the ones before left it.
fn take_out_all(edit: &mut Edit, judge: &dyn Judge, values: Vec<Name>, labels: Vec<Name>) -> Result<(), EditError> {
    for name in values {
        let found = readers(edit.body_function()?, &Value { name, bound: Bound::Input, seen_in: Vec::new() }, &[]);
        take_out(edit, judge, name, false, &found)?;
    }
    for label in labels {
        let found = label_readers(edit.body_function()?, label);
        take_out(edit, judge, label, true, &found)?;
    }
    Ok(())
}

/// Each name once, in the order first met.
fn once(names: &mut Vec<Name>) {
    let mut seen = HashSet::new();
    names.retain(|name| seen.insert(*name));
}

/// What the fragment reads without binding it where it reads it: its inputs, a name read alone that no line of its body
/// binds (a reader inside a structure that binds it sees that one, R-3), the labels its `after:` waits for and that it
/// lacks.
fn unbound(function: &Function) -> (Vec<Name>, Vec<Name>) {
    let at_top: HashSet<Name> = function.body.statements.iter().flat_map(scope::targets).collect();
    let labels: HashSet<Name> = scope::labels(function).into_iter().map(|(label, _)| label).collect();
    let (read, waited) = reads_of(function, true);
    let mut values: Vec<Name> = function.signature.params.iter().map(|param| param.name).collect();
    values.extend(read.into_iter().filter(|name| !at_top.contains(name)));
    once(&mut values);
    (values, waited.into_iter().filter(|label| !labels.contains(label)).collect())
}

/// Renames, in one pass, each line of the function of `edit` whose name `values` maps, with each reader that sees one
/// of these lines (not the carry, the pattern or the clause of a structure that hides it, R-3), each label that
/// `labels` maps with the `after:` that wait for it, and their entries of `layout`.
fn rename_all(edit: &mut Edit, values: &HashMap<Name, Name>, labels: &HashMap<Name, Name>) -> Result<(), EditError> {
    let function = edit.body_function()?.clone();
    let all = scope::all(&function);
    // The names that the lines of each body bind.
    let mut bound: HashMap<Vec<Step>, HashSet<Name>> = HashMap::new();
    for loc in &all {
        if let Some(statement) = scope::statement(&function, loc) {
            bound.entry(loc.body.clone()).or_default().extend(scope::targets(statement));
        }
    }
    // Whether a reader in the body `reader` sees a line that binds `name`, and not a head that hides it.
    let sees_line = |name: Name, reader: &[Step]| -> bool {
        for depth in (0..=reader.len()).rev() {
            let Some(path) = reader.get(..depth) else { return false };
            if bound.get(path).is_some_and(|names| names.contains(&name)) {
                return true;
            }
            let Some((last, outer)) = path.split_last() else { return false };
            let held = scope::body(&function, outer).and_then(|body| body.statements.get(last.stmt));
            if held.is_some_and(|statement| scope::head_names(statement, last.sub).contains(&name)) {
                return false;
            }
        }
        false
    };
    let rename_reads = |read: &mut Operand, seen_in: &[Step]| {
        let mut names = Vec::new();
        read_names(read, false, &mut names);
        for name in names {
            if let Some(to) = values.get(&name)
                && sees_line(name, seen_in)
            {
                crate::places::rename_read(read, name, *to);
            }
        }
    };
    let target = edit.body_function_mut()?;
    for loc in &all {
        let Some(before) = scope::statement(&function, loc) else { continue };
        let slots_of = slots(before, loc);
        let Some(statement) = scope::statement_mut(target, loc) else { continue };
        if let Some(label) = &mut statement.label
            && let Some(to) = labels.get(&label.name)
        {
            label.name = *to;
        }
        if let StatementKind::Binding { targets, .. } = &mut statement.kind {
            for each in targets.iter_mut() {
                if let Target::Name(_, name) = each
                    && let Some(to) = values.get(name)
                {
                    *name = *to;
                }
            }
        }
        for (slot, seen_in) in slots_of {
            if let Some(read) = operand_mut(statement, slot) {
                rename_reads(read, &seen_in);
            }
        }
        for arg in args_mut(statement).into_iter().flat_map(|args| args.iter_mut()) {
            let Arg::After { refs, .. } = arg else { continue };
            for reference in refs.iter_mut() {
                match reference {
                    OrderRef::Value(read) => rename_reads(read, &loc.body),
                    OrderRef::Label(label) => {
                        if let Some(to) = labels.get(&label.name) {
                            label.name = *to;
                        }
                    }
                }
            }
        }
    }
    let of = |node: &mut NodeRef| {
        let map = if node.is_label { labels } else { values };
        if let Some(to) = map.get(&node.name) {
            node.name = *to;
        }
    };
    for item in edit.file.layout.iter_mut().flat_map(|layout| layout.items.iter_mut()) {
        match &mut item.kind {
            LayoutKind::Frame { node: None, .. } | LayoutKind::Terminal { .. } => {}
            LayoutKind::Frame { node: Some(node), .. } | LayoutKind::Node { node, .. } => of(node),
            LayoutKind::Wire { from, to, .. } => {
                of(&mut from.node);
                of(&mut to.node);
            }
        }
    }
    Ok(())
}

/// The fragment, in `edit` in place of the file: what it reads from outside taken out, its names free in the function
/// (`taken`), and the entries of its `layout` that come, moved so that its top-left corner is at `at`; with the `use`
/// lines it needs.
fn prepare(
    edit: &mut Edit,
    judge: &dyn Judge,
    taken: &BTreeSet<String>,
    at: Point,
    edited: &mut Edited,
    file: &str,
) -> Result<(Vec<LayoutItem>, Vec<Use>), EditError> {
    let (values, waited) = unbound(edit.body_function()?);
    take_out_all(edit, judge, values, waited)?;

    // A name of a line or a label already taken in the function gets a suffix, and its readers in the fragment follow
    // (N-14); a suffix is free in the function and in the fragment, the names its structures bind included.
    let function = edit.body_function()?.clone();
    let mut names: Vec<(Name, bool)> =
        scope::values(&function).into_iter().filter(|value| matches!(value.bound, Bound::Line(..))).map(|value| (value.name, false)).collect();
    names.extend(scope::labels(&function).into_iter().map(|(label, _)| (label, true)));
    let mut seen = HashSet::new();
    names.retain(|named| seen.insert(*named));
    let mut all: BTreeSet<String> = taken.clone();
    all.extend(scope::values(&function).into_iter().map(|value| edit.text(value.name).to_owned()));
    all.extend(names.iter().map(|(name, _)| edit.text(*name).to_owned()));
    let (mut value_map, mut label_map) = (HashMap::new(), HashMap::new());
    for (name, is_label) in names {
        let from = edit.text(name).to_owned();
        if !taken.contains(&from) {
            continue;
        }
        let to = fresh(&from, &all);
        all.insert(to.clone());
        let to_name = edit.intern(&to);
        if is_label {
            label_map.insert(name, to_name);
            edited.renamed.push(Renamed { file: file.to_owned(), from: format!("#{from}"), to: format!("#{to}") });
        } else {
            value_map.insert(name, to_name);
            edited.renamed.push(Renamed { file: file.to_owned(), from, to });
        }
    }
    rename_all(edit, &value_map, &label_map)?;

    // Of its `layout`, the `node` entries of its nodes and the `wire` entries between them (an order wire from one of
    // its labels too); the rest stays behind.
    let function = edit.body_function()?.clone();
    let nodes_of = index(&function);
    let bound: HashSet<Name> = scope::values(&function).into_iter().filter(|value| value.bound != Bound::Input).map(|value| value.name).collect();
    let labels: HashSet<Name> = scope::labels(&function).into_iter().map(|(label, _)| label).collect();
    let mut kept: Vec<(LayoutItem, bool)> = Vec::new();
    for item in edit.file.layout.take().map(|layout| layout.items).unwrap_or_default() {
        let (node, from) = match &item.kind {
            // The terminals of the frame of the clipboard are not those of this one.
            LayoutKind::Frame { node: None, .. } | LayoutKind::Terminal { .. } => continue,
            LayoutKind::Frame { node: Some(node), .. } | LayoutKind::Node { node, .. } => (*node, None),
            LayoutKind::Wire { from, to, .. } => (to.node, Some(from.node)),
        };
        let Some(loc) = named_by(&nodes_of, &node) else { continue };
        if from.is_some_and(|from| !(if from.is_label { &labels } else { &bound }).contains(&from.name)) {
            continue;
        }
        kept.push((item, loc.body.is_empty()));
    }
    let corner = |pick: fn(&labcraft_graph::Point) -> &labcraft_graph::Numeral| {
        kept.iter()
            .filter_map(|(item, outer)| match &item.kind {
                LayoutKind::Node { at: Some(point), .. } if *outer => value_of(edit, pick(point)),
                _ => None,
            })
            .min()
    };
    let by = match (corner(|point| &point.x), corner(|point| &point.y)) {
        (Some(x), Some(y)) => Point { x: at.x.saturating_sub(x), y: at.y.saturating_sub(y) },
        _ => Point { x: 0, y: 0 },
    };
    let mut items = Vec::with_capacity(kept.len());
    for (mut item, outer) in kept {
        if outer {
            match &mut item.kind {
                LayoutKind::Node { at: Some(point), .. } => shifted(edit, point, by, "at")?,
                LayoutKind::Node { at: None, .. } | LayoutKind::Frame { .. } | LayoutKind::Terminal { .. } => {}
                LayoutKind::Wire { via, .. } => {
                    for point in via.iter_mut() {
                        shifted(edit, point, by, "at")?;
                    }
                }
            }
        }
        items.push(item);
    }
    let uses = needed(edit, &edit.file.uses)?;
    Ok((items, uses))
}

/// `fragment.paste`: the body of the function that `text` holds, at the end of the body `frame` of the function of `edit`.
pub(crate) fn paste(edit: &mut Edit, judge: &dyn Judge, text: &str, at: Point, frame: &[Step], edited: &mut Edited) -> Result<(), EditError> {
    if text.len() > MAX_FRAGMENT {
        return Err(too_large("text"));
    }
    in_range(at, "at")?;
    let limits = edit.limits().clone();
    let reading = read(text.as_bytes(), &mut edit.names, &limits);
    let Some(fragment) = reading.file else {
        let why = reading.report.diagnostics().first().map(|diagnostic| diagnostic.message.clone()).unwrap_or_default();
        return Err(EditError::invalid("text", format!("the text does not parse: {why}")));
    };
    let mut changes = Report::for_limits(&limits);
    let fragment = canonicalize(fragment, &reading.source_map, &mut edit.names, &mut changes);
    let Decl::Function(function) = &fragment.decl else {
        return Err(EditError::invalid("text", "the text holds no function with a body, so it has nothing to paste"));
    };
    if function.body.statements.iter().any(loop_only) {
        return Err(EditError::invalid(
            "text",
            "a `next`, a `collect` or an `until` stands only in a loop body, so the body of a function holds none (R-6)",
        ));
    }
    let taken = taken(edit);
    let path = edit.path.clone();
    // The fragment is edited as the file of `edit`, in the names of the file, then the file comes back whatever happens.
    let own = std::mem::replace(&mut edit.file, fragment);
    let own_lines = std::mem::take(&mut edit.lines);
    let prepared = prepare(edit, judge, &taken, at, edited, &path);
    let fragment = std::mem::replace(&mut edit.file, own);
    edit.lines = own_lines;
    let (items, uses) = prepared?;

    let Decl::Function(pasted) = fragment.decl else { return Err(EditError::internal("the fragment lost its function")) };
    for used in uses {
        let same = |other: &Use| other.path.first == used.path.first && other.path.rest == used.path.rest;
        if !edit.file.uses.iter().any(same) {
            edit.file.uses.push(Use { line: None, ..used });
        }
    }
    let body = scope::body_mut(edit.body_function_mut()?, frame).ok_or_else(|| EditError::internal("a frame that went"))?;
    let start = body.statements.len();
    body.statements.extend(pasted.body.statements);
    let end = body.statements.len();
    if !items.is_empty() {
        let id = edit.id();
        edit.file.layout.get_or_insert_with(|| Layout { id, comments: Attached::default(), items: Vec::new(), end: Vec::new() }).items.extend(items);
    }
    let function = edit.body_function()?;
    let nodes_of = index(function);
    let names: Vec<String> = (start..end)
        .filter_map(|index| designated(&nodes_of, function, &Loc { body: frame.to_vec(), index }).map(|named| shown(edit, named)))
        .collect();
    edited.lists.insert("nodes", names);
    Ok(())
}
