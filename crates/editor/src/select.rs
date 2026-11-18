//! The selection on the drawing (notation 2.5, 2.6): what a click, a rectangle or a key selects, worked out from the
//! drawing alone. The selection itself is the host's (`view.select`, architecture 12.6): the canvas finds on the drawing
//! what the host names, and gives back, as an intent, the selection that a gesture asks for, named as the editing
//! commands name what they edit (commands.md 9.2).

use std::collections::{BTreeSet, HashMap, VecDeque};

use egui::{Pos2, Rect, Vec2};
use labcraft_diagram::Drawing;

/// A selection, named as the editing commands name what they edit (commands.md 9.2): nodes (`dmm`, `#close`), wires
/// (`dmm -> reading.session`) and terminals of the frame of the function (`input limit`, `output err`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub nodes: Vec<String>,
    pub wires: Vec<String>,
    pub terminals: Vec<String>,
}

impl Selection {
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.wires.is_empty() && self.terminals.is_empty()
    }
}

/// A selection by what the keyboard does to it (notation 2.6), which the host binds to its keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Every node of the frame that holds the focus (Ctrl+A).
    All,
    /// What feeds the selection, one level (Alt+Left), or every level (Shift+Alt+Left).
    Upstream { every: bool },
    /// What the selection feeds, one level (Alt+Right), or every level (Shift+Alt+Right).
    Downstream { every: bool },
}

/// What can be selected under a point: a terminal of the frame of the function, a node, a wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Hit {
    /// By its index in [`Drawing::hints`].
    Terminal(usize),
    /// By its index in [`Drawing::placed`].
    Node(usize),
    /// By its group, in [`Drawing::groups`].
    Wire(usize),
}

/// What a selection is on the drawing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Found {
    pub nodes: BTreeSet<usize>,
    pub groups: BTreeSet<usize>,
    pub terminals: BTreeSet<usize>,
}

impl Found {
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.groups.is_empty() && self.terminals.is_empty()
    }

    pub(crate) fn one(hit: Hit) -> Found {
        let mut found = Found::default();
        found.toggle(hit);
        found
    }

    fn has(&self, hit: Hit) -> bool {
        match hit {
            Hit::Terminal(index) => self.terminals.contains(&index),
            Hit::Node(index) => self.nodes.contains(&index),
            Hit::Wire(index) => self.groups.contains(&index),
        }
    }

    /// Adds it, or takes it out when it is in.
    fn toggle(&mut self, hit: Hit) {
        let (set, index) = match hit {
            Hit::Terminal(index) => (&mut self.terminals, index),
            Hit::Node(index) => (&mut self.nodes, index),
            Hit::Wire(index) => (&mut self.groups, index),
        };
        if !set.remove(&index) {
            set.insert(index);
        }
    }

    /// The one thing selected, if there is one.
    fn single(&self) -> Option<Hit> {
        let count = self.nodes.len() + self.groups.len() + self.terminals.len();
        if count != 1 {
            return None;
        }
        let terminal = self.terminals.first().map(|&index| Hit::Terminal(index));
        let node = self.nodes.first().map(|&index| Hit::Node(index));
        terminal.or(node).or_else(|| self.groups.first().map(|&index| Hit::Wire(index)))
    }
}

/// What a drawing names, found once per drawing.
#[derive(Default)]
pub(crate) struct Index {
    /// Each node by its name, as a command names it.
    nodes: HashMap<String, usize>,
    /// Each wire by its name, to its group: the groups of the values.
    wires: HashMap<String, usize>,
    /// Each terminal of the frame of the function by its name, to its hint.
    terminals: HashMap<String, usize>,
    /// By node: the nodes that feed it, and those that it feeds.
    upstream: Vec<Vec<usize>>,
    downstream: Vec<Vec<usize>>,
    /// By group of a wire: the node it leaves and the nodes it reaches.
    ends: HashMap<usize, (Option<usize>, Vec<usize>)>,
    /// By body: the structure that holds it, by its index in [`Drawing::placed`], and the arm in tabs that it is, by its
    /// index in [`Drawing::arms`].
    owners: HashMap<usize, (usize, Option<usize>)>,
    /// By node: the arm in tabs that holds it.
    within: Vec<Option<usize>>,
}

impl Index {
    pub fn of(drawing: &Drawing) -> Index {
        let mut index = Index::default();
        let keys: HashMap<&str, usize> = drawing.placed.iter().enumerate().map(|(at, placed)| (placed.key.as_str(), at)).collect();
        for (at, placed) in drawing.placed.iter().enumerate() {
            if !placed.name.is_empty() {
                index.nodes.entry(placed.name.clone()).or_insert(at);
            }
            for (rank, &body) in placed.holds.iter().enumerate() {
                let arm = drawing.arms.iter().position(|arm| arm.structure == placed.key && arm.arm == rank);
                index.owners.insert(body, (at, arm));
            }
        }
        index.upstream = vec![Vec::new(); drawing.placed.len()];
        index.downstream = vec![Vec::new(); drawing.placed.len()];
        index.within = vec![None; drawing.placed.len()];
        for (at, group) in drawing.groups.iter().enumerate() {
            if let Some(placed) = group.placed.filter(|_| group.is_node())
                && let Some(slot) = index.within.get_mut(placed)
            {
                *slot = group.within;
            }
            let Some(wire) = &group.wire else { continue };
            for name in &wire.named {
                index.wires.entry(name.clone()).or_insert(at);
            }
            let from = wire.from_node.as_deref().and_then(|key| keys.get(key)).copied();
            let readers: Vec<usize> = wire.to_nodes.iter().filter_map(|key| keys.get(key.as_str())).copied().collect();
            index.ends.insert(at, (from, readers.clone()));
            let Some(from) = from else { continue };
            for to in readers {
                if to == from {
                    continue;
                }
                if let Some(list) = index.downstream.get_mut(from)
                    && !list.contains(&to)
                {
                    list.push(to);
                }
                if let Some(list) = index.upstream.get_mut(to)
                    && !list.contains(&from)
                {
                    list.push(from);
                }
            }
        }
        for (at, hint) in drawing.hints.iter().enumerate() {
            if let Some(name) = &hint.selects {
                index.terminals.entry(name.clone()).or_insert(at);
            }
        }
        index
    }

    /// What the host's selection is on the drawing; a name that the drawing does not have is left out.
    pub fn find(&self, selection: &Selection) -> Found {
        Found {
            nodes: selection.nodes.iter().filter_map(|name| self.nodes.get(name.as_str())).copied().collect(),
            groups: selection.wires.iter().filter_map(|name| self.wires.get(name.as_str())).copied().collect(),
            terminals: selection.terminals.iter().filter_map(|name| self.terminals.get(name.as_str())).copied().collect(),
        }
    }

    /// A selection found on the drawing, named: a wire is every wire of its value that a command names (notation 2.5);
    /// what no command names is left out.
    pub fn name(&self, drawing: &Drawing, found: &Found) -> Selection {
        let nodes = found
            .nodes
            .iter()
            .filter_map(|&at| drawing.placed.get(at))
            .filter(|placed| !placed.name.is_empty())
            .map(|placed| placed.name.clone())
            .collect();
        let wires = found.groups.iter().filter_map(|&at| drawing.groups.get(at)?.wire.as_ref()).flat_map(|wire| wire.named.iter().cloned()).collect();
        let terminals = found.terminals.iter().filter_map(|&at| drawing.hints.get(at)?.selects.clone()).collect();
        Selection { nodes, wires, terminals }
    }

    /// Whether a node shows: every arm in tabs around it is shown.
    fn node_shown(&self, placed: usize, visible: &dyn Fn(Option<usize>) -> bool) -> bool {
        visible(self.within.get(placed).copied().flatten())
    }

    /// Whether a body shows: the function's, or one that its structure shows, the arm shown of one in tabs.
    pub(crate) fn body_shown(&self, body: usize, visible: &dyn Fn(Option<usize>) -> bool) -> bool {
        match self.owners.get(&body) {
            None => body == 0,
            Some(&(structure, arm)) => self.node_shown(structure, visible) && arm.is_none_or(|arm| visible(Some(arm))),
        }
    }
}

/// The box of a node, in units.
pub(crate) fn node_rect(drawing: &Drawing, placed: usize) -> Option<Rect> {
    let placed = drawing.placed.get(placed)?;
    Some(Rect::from_min_size(Pos2::new(placed.x as f32, placed.y as f32) / 100.0, Vec2::new(placed.width as f32, placed.height as f32) / 100.0))
}

pub(crate) fn body_rect(drawing: &Drawing, body: usize) -> Option<Rect> {
    let (x, y, width, height) = drawing.bodies.get(body)?.frame;
    Some(Rect::from_min_size(Pos2::new(x as f32, y as f32) / 100.0, Vec2::new(width as f32, height as f32) / 100.0))
}

fn hint_rect(drawing: &Drawing, hint: usize) -> Option<Rect> {
    let hint = drawing.hints.get(hint)?;
    Some(Rect::from_min_size(Pos2::new(hint.x as f32, hint.y as f32) / 100.0, Vec2::new(hint.width as f32, hint.height as f32) / 100.0))
}

/// Whether a point is on a node: in its box, and for a structure not inside one of its frames shown, which is canvas of
/// its own (its header and its edges are the structure).
fn on_node(drawing: &Drawing, index: &Index, placed: usize, at: Pos2, visible: &dyn Fn(Option<usize>) -> bool) -> bool {
    let Some(found) = drawing.placed.get(placed) else { return false };
    if !index.node_shown(placed, visible) || !node_rect(drawing, placed).is_some_and(|area| area.contains(at)) {
        return false;
    }
    !found.holds.iter().any(|&body| index.body_shown(body, visible) && body_rect(drawing, body).is_some_and(|area| area.contains(at)))
}

/// Whether `at`, in units, is within `reach` of the orthogonal segment from `a` to `b`, in hundredths.
pub(crate) fn near_segment(at: Pos2, a: (i64, i64), b: (i64, i64), reach: f32) -> bool {
    let (ax, ay, bx, by) = (a.0 as f32 / 100.0, a.1 as f32 / 100.0, b.0 as f32 / 100.0, b.1 as f32 / 100.0);
    let x = at.x.clamp(ax.min(bx), ax.max(bx));
    let y = at.y.clamp(ay.min(by), ay.max(by));
    (at.x - x).abs() <= reach && (at.y - y).abs() <= reach
}

/// The group of the wire within `reach` of `at`, among `groups`.
pub(crate) fn wire_at(drawing: &Drawing, groups: &[usize], at: Pos2, reach: f32, visible: &dyn Fn(Option<usize>) -> bool) -> Option<usize> {
    groups.iter().filter_map(|&index| Some((index, drawing.groups.get(index)?))).filter(|(_, group)| visible(group.within)).find_map(
        |(index, group)| {
            let wire = group.wire.as_ref()?;
            let near = wire.readers.iter().any(|(_, points)| {
                points.windows(2).any(|pair| match pair {
                    [a, b] => near_segment(at, *a, *b, reach),
                    _ => false,
                })
            });
            near.then_some(index)
        },
    )
}

/// What a click at `at` selects (notation 2.6): a terminal of the frame of the function, then a node that is no
/// structure, then a wire within `reach`, then a structure by its header or its edges. `groups`: those the canvas looks
/// at, around the point.
pub(crate) fn hit(drawing: &Drawing, index: &Index, groups: &[usize], at: Pos2, reach: f32, visible: &dyn Fn(Option<usize>) -> bool) -> Option<Hit> {
    let terminal = drawing
        .hints
        .iter()
        .enumerate()
        .filter(|(_, hint)| hint.selects.is_some() && visible(hint.within))
        .find(|(at_hint, _)| hint_rect(drawing, *at_hint).is_some_and(|area| area.contains(at)))
        .map(|(at_hint, _)| Hit::Terminal(at_hint));
    if terminal.is_some() {
        return terminal;
    }
    let node = node_at(drawing, index, at, visible);
    let plain = node.filter(|&placed| drawing.placed.get(placed).is_some_and(|found| found.holds.is_empty()));
    if let Some(placed) = plain {
        return Some(Hit::Node(placed));
    }
    if let Some(group) = wire_at(drawing, groups, at, reach, visible) {
        return Some(Hit::Wire(group));
    }
    node.map(Hit::Node)
}

/// The node under a point, for a click and for the hover alike (architecture 12.4: one box): the innermost one, drawn
/// last, on whose box the point is, a structure only by its header and its edges.
pub(crate) fn node_at(drawing: &Drawing, index: &Index, at: Pos2, visible: &dyn Fn(Option<usize>) -> bool) -> Option<usize> {
    (0..drawing.placed.len()).rev().find(|&placed| on_node(drawing, index, placed, at, visible))
}

/// The selection after a click: the thing hit alone, or with Shift or Ctrl the selection with it added or taken out;
/// nothing hit, nothing selected, or with a modifier the selection as it is.
pub(crate) fn click(found: &Found, hit: Option<Hit>, adding: bool) -> Found {
    match (hit, adding) {
        (Some(hit), true) => {
            let mut found = found.clone();
            found.toggle(hit);
            found
        }
        (Some(hit), false) => Found::one(hit),
        (None, true) => found.clone(),
        (None, false) => Found::default(),
    }
}

/// The frame that holds a point: the innermost body shown whose frame holds it; the function's when none does.
pub(crate) fn frame_at(drawing: &Drawing, index: &Index, at: Pos2, visible: &dyn Fn(Option<usize>) -> bool) -> usize {
    (1..drawing.bodies.len())
        .filter(|&body| index.body_shown(body, visible))
        .filter_map(|body| Some((body, body_rect(drawing, body)?)))
        .filter(|(_, area)| area.contains(at))
        .min_by(|(_, a), (_, b)| a.area().total_cmp(&b.area()))
        .map_or(0, |(body, _)| body)
}

/// The nodes of the frame of `from` that a rectangle from `from` to `to` touches (notation 2.6).
pub(crate) fn rectangle(drawing: &Drawing, index: &Index, from: Pos2, to: Pos2, visible: &dyn Fn(Option<usize>) -> bool) -> BTreeSet<usize> {
    let area = Rect::from_two_pos(from, to);
    let frame = frame_at(drawing, index, from, visible);
    (0..drawing.placed.len())
        .filter(|&placed| drawing.placed.get(placed).is_some_and(|found| found.body == frame) && index.node_shown(placed, visible))
        .filter(|&placed| node_rect(drawing, placed).is_some_and(|rect| rect.intersects(area)))
        .collect()
}

/// The frame that holds the focus: that of the first node selected, else of the first wire, else the function's.
fn focus_frame(drawing: &Drawing, found: &Found) -> usize {
    let node = found.nodes.first().and_then(|&placed| drawing.placed.get(placed)).map(|placed| placed.body);
    let wire = || found.groups.first().and_then(|&group| drawing.groups.get(group)?.wire.as_ref()).map(|wire| wire.body);
    node.or_else(wire).unwrap_or(0)
}

/// What a key of notation 2.6 makes of the selection.
pub(crate) fn step(drawing: &Drawing, index: &Index, found: &Found, step: Step, visible: &dyn Fn(Option<usize>) -> bool) -> Found {
    match step {
        Step::All => {
            let frame = focus_frame(drawing, found);
            let nodes = (0..drawing.placed.len())
                .filter(|&placed| drawing.placed.get(placed).is_some_and(|found| found.body == frame) && index.node_shown(placed, visible))
                .collect();
            Found { nodes, ..Found::default() }
        }
        Step::Upstream { every } => level(found, index, true, every),
        Step::Downstream { every } => level(found, index, false, every),
    }
}

/// One level up or down from the selection, or every level: from its nodes, by the wires between nodes; from its wires,
/// their ends are the first level.
fn level(found: &Found, index: &Index, upstream: bool, every: bool) -> Found {
    let links = if upstream { &index.upstream } else { &index.downstream };
    let ends = with_ends(found, index, upstream);
    if every {
        return spread(&ends, links, true);
    }
    let mut result = spread(found, links, false);
    result.nodes.extend(ends.nodes);
    result
}

/// The selection with, for each wire in it, the node it leaves (`upstream`) or the nodes it reaches: what feeds a wire
/// is its source, what it feeds its readers.
fn with_ends(found: &Found, index: &Index, upstream: bool) -> Found {
    let mut result = found.clone();
    for group in &found.groups {
        let Some((from, readers)) = index.ends.get(group) else { continue };
        if upstream {
            result.nodes.extend(from.iter().copied());
        } else {
            result.nodes.extend(readers.iter().copied());
        }
    }
    result
}

/// The selection with the nodes that `links` give for its nodes, one level, or every level.
fn spread(found: &Found, links: &[Vec<usize>], every: bool) -> Found {
    let mut result = found.clone();
    let mut queue: VecDeque<usize> = found.nodes.iter().copied().collect();
    while let Some(placed) = queue.pop_front() {
        for &next in links.get(placed).map(Vec::as_slice).unwrap_or_default() {
            let new = result.nodes.insert(next);
            if new && every {
                queue.push_back(next);
            }
        }
    }
    result
}

/// What Tab goes through in a frame, in the order of the text (notation 2.6): the inputs of the frame, its nodes, its
/// outputs. Only the frame of the function has terminals that can be selected; a node that no command names (one whose
/// line the file does not say) is passed, as a click selects it not: the session could not hold it, and the focus would
/// be lost.
fn order(drawing: &Drawing, index: &Index, frame: usize, visible: &dyn Fn(Option<usize>) -> bool) -> Vec<Hit> {
    let terminals = |side: &str| {
        drawing
            .hints
            .iter()
            .enumerate()
            .filter(|(_, hint)| frame == 0 && hint.selects.as_deref().is_some_and(|name| name.starts_with(side)))
            .map(|(at, _)| Hit::Terminal(at))
            .collect::<Vec<_>>()
    };
    let nodes = (0..drawing.placed.len())
        .filter(|&placed| drawing.placed.get(placed).is_some_and(|found| found.body == frame && !found.name.is_empty()))
        .filter(|&placed| index.node_shown(placed, visible))
        .map(Hit::Node);
    let mut items = terminals("input ");
    items.extend(nodes);
    items.extend(terminals("output "));
    items
}

/// The frame of what is selected alone, or of the focus.
fn frame_of(drawing: &Drawing, hit: Option<Hit>, found: &Found) -> usize {
    match hit {
        Some(Hit::Node(placed)) => drawing.placed.get(placed).map_or(0, |placed| placed.body),
        Some(Hit::Wire(group)) => drawing.groups.get(group).and_then(|group| group.wire.as_ref()).map_or(0, |wire| wire.body),
        Some(Hit::Terminal(_)) => 0,
        None => focus_frame(drawing, found),
    }
}

/// Tab or Shift+Tab: the next or the previous one in the frame, in the order of the text; none past either end, where
/// the focus leaves the canvas (notation 2.6).
pub(crate) fn tab(drawing: &Drawing, index: &Index, found: &Found, forward: bool, visible: &dyn Fn(Option<usize>) -> bool) -> Option<Found> {
    let single = found.single();
    let items = order(drawing, index, frame_of(drawing, single, found), visible);
    let now = single.and_then(|hit| items.iter().position(|item| *item == hit));
    let next = match (now, forward) {
        (None, true) => Some(0),
        (None, false) => items.len().checked_sub(1),
        (Some(at), true) => Some(at + 1).filter(|next| *next < items.len()),
        (Some(at), false) => at.checked_sub(1),
    };
    next.and_then(|next| items.get(next)).map(|&hit| Found::one(hit))
}

/// The centre of what is selected, in units.
fn centre(drawing: &Drawing, hit: Hit) -> Option<Pos2> {
    match hit {
        Hit::Node(placed) => node_rect(drawing, placed).map(|rect| rect.center()),
        Hit::Terminal(at) => hint_rect(drawing, at).map(|rect| rect.center()),
        Hit::Wire(_) => None,
    }
}

/// An arrow: the nearest node or terminal in its direction, in the frame of what is selected; in the order of the text
/// when two are as near (notation 2.6). Nothing selected: the first in the frame.
pub(crate) fn arrow(drawing: &Drawing, index: &Index, found: &Found, direction: Vec2, visible: &dyn Fn(Option<usize>) -> bool) -> Option<Found> {
    let single = found.single();
    let items = order(drawing, index, frame_of(drawing, single, found), visible);
    let Some(from) = single.and_then(|hit| centre(drawing, hit)) else { return items.first().map(|&hit| Found::one(hit)) };
    let mut best: Option<(f32, Hit)> = None;
    for &item in &items {
        if Some(item) == single {
            continue;
        }
        let Some(to) = centre(drawing, item) else { continue };
        let delta = to - from;
        let along = delta.dot(direction);
        let across = (delta - direction * along).length();
        // In its direction, within 45 degrees of it.
        if along <= 0.0 || across > along {
            continue;
        }
        let distance = delta.length();
        if best.is_none_or(|(nearest, _)| distance < nearest) {
            best = Some((distance, item));
        }
    }
    best.map(|(_, hit)| Found::one(hit))
}

/// The box of what is selected alone, in units: what the canvas scrolls to (notation 2.6).
pub(crate) fn focus_rect(drawing: &Drawing, found: &Found) -> Option<Rect> {
    match found.single()? {
        Hit::Node(placed) => node_rect(drawing, placed),
        Hit::Terminal(at) => hint_rect(drawing, at),
        Hit::Wire(_) => None,
    }
}

/// Escape: up one level, a node inside a structure to that structure; otherwise nothing (notation 2.6).
pub(crate) fn up(drawing: &Drawing, index: &Index, found: &Found) -> Found {
    let owner = match found.single() {
        Some(Hit::Node(placed)) => drawing.placed.get(placed).and_then(|placed| index.owners.get(&placed.body)).map(|&(structure, _)| structure),
        _ => None,
    };
    owner.map_or_else(Found::default, |structure| Found::one(Hit::Node(structure)))
}

/// Enter on a structure: into its frame, the arm shown, on its first node (notation 2.6). A node that is no structure
/// goes to its terminals with the wiring of step 3.
pub(crate) fn enter(drawing: &Drawing, index: &Index, found: &Found, visible: &dyn Fn(Option<usize>) -> bool) -> Option<Found> {
    let Some(Hit::Node(placed)) = found.single() else { return None };
    let structure = drawing.placed.get(placed)?;
    let body = structure.holds.iter().copied().find(|&body| index.body_shown(body, visible))?;
    order(drawing, index, body, visible).first().map(|&hit| Found::one(hit))
}

/// Whether the selection holds this hit.
pub(crate) fn holds(found: &Found, hit: Hit) -> bool {
    found.has(hit)
}
