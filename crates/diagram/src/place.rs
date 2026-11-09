//! Step 3 (`docs/architecture.md`, 11.3): placement (notation 2.1). Each body is placed on its own, the innermost
//! first, so that a structure is a node of the size of its frame in the body around it:
//!
//! - **columns**: a node stands in the column after the last of the nodes that feed it; a node with no input just
//!   before its leftmost reader, or in the last column;
//! - **the error chain** on one line, the axis: the longest chain of error pairs from where the error enters the body;
//! - **stacks**: every other node above the chain in its column, in the order that a fixed number of sweeps of
//!   barycentres gives, ties broken by the canonical order of lines;
//! - **the terminals of the frame** in their order, each at the height of what it feeds or what feeds it, as far as
//!   the order allows, `err` on the axis;
//! - **the `layout` section**: a node that it places keeps its place, the others move up out of its way.
//!
//! Every step is linear in the nodes and the wires of a body, but for the sorts of a column.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

use crate::graph::{BodyOf, End, FrameTerminal, Graph, Kind, Node, StructureKind};
use crate::route::{self, At, Ends, GAP, MIN_CHANNEL, Routed, Side};
use crate::size::{self, ANY_TEXT, GRID, MEDIUM, PADDING, SMALL, Sized, Text, pill_width};
/// Between two nodes stacked in a column, beyond what a band or a label adds above a node.
pub const STACK: i64 = 2400;
/// The bottom of a column with no node of the chain, above the axis.
pub const OFF_AXIS: i64 = 4000;
/// The header of a frame, its title, and the margin inside it.
pub const HEADER: i64 = 3200;
pub const PAD: i64 = 1600;
/// The title of an arm, and the gap between two arms.
pub const ARM_HEADER: i64 = 2400;
pub const ARM_GAP: i64 = 1600;
/// Between two terminals of a frame.
pub const SLOT: i64 = 2400;
/// Where the name of a terminal of a frame starts, from the edge.
pub const LABEL_START: i64 = 1200;
/// Between a label and what it labels (notation 3.1).
pub const LABEL_ABOVE: i64 = 400;
/// Sweeps of barycentres, each one way then the other.
const SWEEPS: usize = 4;

/// The name of a terminal of a frame, its unit and its default (notation 2: `low ◂ 220 V`).
pub struct TerminalLabel {
    pub name: Text,
    pub unit: Option<Text>,
    pub pill: Option<Text>,
}

impl TerminalLabel {
    pub fn of(terminal: &FrameTerminal) -> TerminalLabel {
        let terminal = &terminal.terminal;
        TerminalLabel {
            name: Text::new(&terminal.name, ANY_TEXT, MEDIUM),
            unit: terminal.unit.as_ref().map(|unit| Text::new(&format!("[{unit}]"), ANY_TEXT, SMALL)),
            pill: terminal.constant.as_ref().map(|constant| Text::new(&format!("\u{25C2} {constant}"), ANY_TEXT, MEDIUM)),
        }
    }

    pub fn width(&self) -> i64 {
        self.name.width + self.unit.as_ref().map_or(0, |unit| PADDING + unit.width) + self.pill.as_ref().map_or(0, |pill| GRID + pill_width(pill))
    }
}

/// Where a node is: the top-left corner of its block, and its place in the columns of its body.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NodeAt {
    pub x: i64,
    pub y: i64,
    pub column: u32,
    pub rank: u32,
}

/// Where the frame of a body is: the function's, a loop's (the frame of its structure), an arm's box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FrameAt {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
    /// The height of its title.
    pub header: i64,
    /// The heights of its terminals, from its top.
    pub left: Vec<i64>,
    pub right: Vec<i64>,
    /// The depth of nesting: 0 for the function.
    pub depth: u32,
    /// The height of its main error chain.
    pub axis: Option<i64>,
    /// Whether the file places a node of it: the rules that say "never" then hold only as far as they can (notation
    /// 2.1).
    pub from_file: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Placement {
    /// By node; a node of a body that was not placed has none.
    pub nodes: Vec<Option<NodeAt>>,
    /// By body.
    pub frames: Vec<FrameAt>,
    /// The wires of each body, routed, by net.
    pub routes: Vec<Vec<Routed>>,
}

/// A body placed, from the top-left corner of its frame.
#[derive(Clone, Debug, Default)]
struct BodyPlace {
    width: i64,
    height: i64,
    header: i64,
    nodes: Vec<(usize, NodeAt)>,
    left: Vec<i64>,
    right: Vec<i64>,
    axis: Option<i64>,
    /// How far the frame grows above its corner, to hold what a placed node pushed up out of it: the positions of the
    /// file stay measured from the corner (spec 11).
    rise: i64,
    /// Its wires, from the corner.
    routes: Vec<Routed>,
    from_file: bool,
}

/// The error pair of a node: its input and its output on its bottom line (notation 3.1), or those of a structure.
pub fn pair_of(node: &Node<'_>) -> Option<(usize, usize)> {
    if let Kind::Structure(structure) = &node.kind {
        return structure.pair;
    }
    let lines = size::lines(&node.inputs, &node.outputs);
    match lines.last() {
        Some(&(Some(input), Some(output)))
            if node.inputs.get(input).is_some_and(|terminal| terminal.family == crate::graph::Family::ERROR)
                && node.outputs.get(output).is_some_and(|terminal| terminal.family == crate::graph::Family::ERROR) =>
        {
            Some((input, output))
        }
        _ => None,
    }
}

/// The widths that titles need: of each body's frame, and of the header of each `match` or `handle`, by node.
#[derive(Clone, Debug, Default)]
pub struct Titles {
    pub bodies: Vec<i64>,
    pub structures: HashMap<usize, i64>,
}

/// Places every body, the innermost first; `sized` gets the size of each structure, from its frame. `titles` is the
/// width that the title of each body needs, `origin` where the frame of the function goes.
pub fn place(graph: &Graph<'_>, sized: &mut [Sized], titles: &Titles, origin: (i64, i64)) -> Placement {
    let count = graph.bodies.len();
    let mut bodies: Vec<BodyPlace> = vec![BodyPlace::default(); count];
    // Where each body of a structure is in the block of its structure, and how wide its frame is.
    let mut offsets: Vec<(i64, i64, i64)> = vec![(0, 0, 0); count];
    for index in (0..count).rev() {
        let Some(body) = graph.bodies.get(index) else { continue };
        for &node in &body.nodes {
            if let Some(found) = graph.nodes.get(node)
                && let Kind::Structure(_) = found.kind
            {
                size_structure(graph, node, &bodies, &mut offsets, sized, titles);
            }
        }
        let header = match body.owner.and_then(|owner| graph.nodes.get(owner)).map(|owner| &owner.kind) {
            Some(Kind::Structure(structure)) if matches!(structure.kind, StructureKind::Match | StructureKind::Handle) => ARM_HEADER,
            _ => HEADER,
        };
        let title = titles.bodies.get(index).copied().unwrap_or(0);
        if let Some(slot) = bodies.get_mut(index) {
            *slot = layout_body(graph, index, sized, header, title);
        }
    }
    // From the function inwards: each frame at its place, then its nodes.
    let mut placement = Placement { nodes: vec![None; graph.nodes.len()], frames: vec![FrameAt::default(); count], routes: vec![Vec::new(); count] };
    let mut corners: Vec<(i64, i64, u32)> = vec![(0, 0, 0); count];
    if let Some(first) = corners.first_mut() {
        *first = (origin.0, origin.1, 0);
    }
    for index in 0..count {
        let (Some(body), Some(placed), Some(&(x, y, depth))) = (graph.bodies.get(index), bodies.get(index), corners.get(index)) else { continue };
        let width = offsets.get(index).map_or(0, |offset| offset.2).max(placed.width);
        let rise = placed.rise;
        if let Some(frame) = placement.frames.get_mut(index) {
            *frame = FrameAt {
                x,
                y: y - rise,
                width,
                height: placed.height + rise,
                header: placed.header,
                left: placed.left.iter().map(|at| at + rise).collect(),
                right: placed.right.iter().map(|at| at + rise).collect(),
                depth,
                axis: placed.axis.map(|axis| y + axis),
                from_file: placed.from_file,
            };
        }
        if let Some(slot) = placement.routes.get_mut(index) {
            *slot = placed.routes.iter().cloned().map(|routed| routed.stretched(width - placed.width).moved(x, y)).collect();
        }
        for &(node, at) in &placed.nodes {
            let absolute = NodeAt { x: x + at.x, y: y + at.y, ..at };
            if let Some(slot) = placement.nodes.get_mut(node) {
                *slot = Some(absolute);
            }
            // The arms of a `match` are boxes inside its frame: one level deeper.
            let deeper = match graph.nodes.get(node).map(|node| &node.kind) {
                Some(Kind::Structure(structure)) if matches!(structure.kind, StructureKind::Match | StructureKind::Handle) => 2,
                _ => 1,
            };
            for &inner in graph.nodes.get(node).map_or(&[][..], |node| node.bodies.as_slice()) {
                let (dx, dy, _) = offsets.get(inner).copied().unwrap_or_default();
                if let Some(corner) = corners.get_mut(inner) {
                    *corner = (absolute.x + dx, absolute.y + dy, depth + deeper);
                }
            }
        }
        let _ = body;
    }
    placement
}

/// The size of a structure from its bodies, and where they are in it: a loop is the frame of its body; a `match` or a
/// `handle` unfolded stacks its arms, the last at the bottom, on the axis (notation 5.2).
fn size_structure(graph: &Graph<'_>, node: usize, bodies: &[BodyPlace], offsets: &mut [(i64, i64, i64)], sized: &mut [Sized], titles: &Titles) {
    let Some(found) = graph.nodes.get(node) else { return };
    let Kind::Structure(structure) = &found.kind else { return };
    let block = match structure.kind {
        StructureKind::Loop | StructureKind::For => {
            let Some((&body, placed)) = found.bodies.first().and_then(|&body| Some((found.bodies.first()?, bodies.get(body)?))) else { return };
            let rise = placed.rise;
            let mut block = Sized::new(placed.width, placed.height + rise, Vec::new());
            block.inputs = (0..found.inputs.len())
                .map(|input| {
                    let left = structure.inputs_at.get(input).and_then(|at| at.first()).map(|&(_, left)| left);
                    (0, rise + left.and_then(|left| placed.left.get(left).copied()).unwrap_or(placed.height / 2))
                })
                .collect();
            block.outputs = (0..found.outputs.len())
                .map(|output| (placed.width, rise + placed.right.get(output).copied().unwrap_or(placed.height / 2)))
                .collect();
            // The first value of a register given as a constant: its pill outside, left of the box (notation 5.1).
            block.left = found.inputs.iter().filter_map(|input| input.constant.as_ref()).map(|constant| constant_reach(constant)).max().unwrap_or(0);
            if let Some(offset) = offsets.get_mut(body) {
                *offset = (0, rise, placed.width);
            }
            block
        }
        StructureKind::Match | StructureKind::Handle => {
            let header_width = titles.structures.get(&node).copied().unwrap_or(0);
            let widest = found.bodies.iter().filter_map(|&body| bodies.get(body)).map(|placed| placed.width).max().unwrap_or(0);
            let width = size::up(widest.max(header_width));
            let mut y = HEADER + PAD;
            let mut arms = Vec::new();
            let mut corners = Vec::new();
            for &body in &found.bodies {
                let Some(placed) = bodies.get(body) else { continue };
                let corner = y + placed.rise;
                corners.push((body, corner));
                arms.push((corner, placed));
                y = corner + placed.height + ARM_GAP;
            }
            let height = (y - ARM_GAP + PAD).max(HEADER + 2 * PAD);
            let mut block = Sized::new(0, height, Vec::new());
            let last = arms.last();
            // The last arm is on the axis (notation 5.2): the error enters and leaves on that line, whichever arm reads it.
            let axis = last.and_then(|(y, placed)| placed.axis.map(|axis| y + axis));
            let entry = structure.pair.map(|(input, _)| input);
            block.inputs = (0..found.inputs.len())
                .map(|input| {
                    if let (Some(axis), true) = (axis, entry == Some(input)) {
                        return (0, axis);
                    }
                    if input == 0 {
                        // The selector of a `handle` is an error: on the bottom line, the axis when there is one.
                        let y = match structure.kind {
                            StructureKind::Handle => axis.unwrap_or(height - PAD),
                            _ => HEADER / 2,
                        };
                        return (0, y);
                    }
                    // The notch of the last arm that reads it.
                    let at = structure.inputs_at.get(input).map_or(&[][..], Vec::as_slice);
                    let found = at.iter().rev().find_map(|&(arm, left)| {
                        let (y, placed) = arms.get(arm)?;
                        Some(y + placed.left.get(left).copied()?)
                    });
                    (0, found.unwrap_or(HEADER / 2))
                })
                .collect();
            let outer_outputs: Vec<i64> = (0..found.outputs.len())
                .map(|output| last.and_then(|(y, placed)| Some(y + placed.right.get(output).copied()?)).unwrap_or(height / 2))
                .collect();
            // Between the edge of the frame and the boxes of the arms: a value that enters by a notch goes on to the notch
            // of each arm that reads it, and what each arm yields to the output of the frame, a vertical each where they
            // are not at one height.
            let entering: Vec<(i64, Vec<i64>)> = block
                .inputs
                .iter()
                .enumerate()
                .map(|(input, &(_, outer))| {
                    let at = structure.inputs_at.get(input).map_or(&[][..], Vec::as_slice);
                    (outer, at.iter().filter_map(|&(arm, left)| Some(arms.get(arm)?.0 + arms.get(arm)?.1.left.get(left)?)).collect())
                })
                .collect();
            let leaving: Vec<(i64, Vec<i64>)> = outer_outputs
                .iter()
                .enumerate()
                .map(|(output, &outer)| (outer, arms.iter().filter_map(|(y, placed)| Some(y + placed.right.get(output)?)).collect()))
                .collect();
            let (left_slots, left_gap) = gap_slots(&entering, |index| index);
            // On the right, the later outputs stand further left: their wires from the upper arms cross fewer verticals.
            let (right_slots, right_gap) = gap_slots(&leaving, |index| leaving.len() - 1 - index);
            for (body, corner) in corners {
                if let Some(offset) = offsets.get_mut(body) {
                    *offset = (left_gap, corner, width);
                }
            }
            let frame_width = left_gap + width + right_gap;
            block.width = frame_width;
            block.outputs = outer_outputs.iter().map(|&y| (frame_width, y)).collect();
            let core = "frame.fill.2";
            for (input, ((outer, arms_at), slot)) in entering.iter().zip(&left_slots).enumerate() {
                let family = found.inputs.get(input).map_or(crate::graph::Family::ERROR, |terminal| terminal.family);
                let x = GAP + GAP * i64::from(slot.unwrap_or(0));
                let branches: Vec<Vec<(i64, i64)>> =
                    arms_at.iter().map(|&arm_y| route::simplify(vec![(0, *outer), (x, *outer), (x, arm_y), (left_gap, arm_y)])).collect();
                gap_shapes(&branches, family, core, &mut block.shapes);
            }
            for (output, ((outer, arms_at), slot)) in leaving.iter().zip(&right_slots).enumerate() {
                let family = found.outputs.get(output).map_or(crate::graph::Family::ERROR, |terminal| terminal.family);
                let x = left_gap + width + GAP + GAP * i64::from(slot.unwrap_or(0));
                let branches: Vec<Vec<(i64, i64)>> = arms_at
                    .iter()
                    .map(|&arm_y| route::simplify(vec![(left_gap + width, arm_y), (x, arm_y), (x, *outer), (frame_width, *outer)]))
                    .collect();
                gap_shapes(&branches, family, core, &mut block.shapes);
            }
            block
        }
    };
    let mut block = block;
    // A label above the frame, as above a block (notation 3.1); an order wire from it starts right after it.
    if let Some(label) = &found.label {
        block.above = LABEL_ABOVE + size::line_height(MEDIUM);
        let width = Text::new(&format!("{label}:"), ANY_TEXT, MEDIUM).width;
        block.label_end = Some((width + PADDING, -LABEL_ABOVE - size::line_height(MEDIUM) / 2));
    }
    if let Some(slot) = sized.get_mut(node) {
        *slot = block;
    }
}

/// The slots of the verticals in a margin of a `match`, taken in the order that `rank` gives, and how wide the margin
/// is: 16 at least, 8 around them.
fn gap_slots(lists: &[(i64, Vec<i64>)], rank: impl Fn(usize) -> usize) -> (Vec<Option<u32>>, i64) {
    let mut order: Vec<usize> = (0..lists.len()).collect();
    order.sort_by_key(|&index| rank(index));
    let mut intervals = Vec::new();
    let mut which = Vec::new();
    for &index in &order {
        let Some((outer, ys)) = lists.get(index) else { continue };
        let low = ys.iter().copied().chain([*outer]).min().unwrap_or(*outer);
        let high = ys.iter().copied().chain([*outer]).max().unwrap_or(*outer);
        if low < high {
            intervals.push((low, high));
            which.push(index);
        }
    }
    let assigned = route::assign_slots(&intervals);
    let mut slots = vec![None; lists.len()];
    for (&index, &slot) in which.iter().zip(&assigned) {
        if let Some(entry) = slots.get_mut(index) {
            *entry = Some(slot);
        }
    }
    let count = i64::from(assigned.iter().map(|slot| slot + 1).max().unwrap_or(0));
    (slots, (GAP * (count + 1)).max(PAD))
}

/// The wires of a margin of a `match`, with their junction points.
fn gap_shapes(branches: &[Vec<(i64, i64)>], family: crate::graph::Family, core: crate::shape::Token, shapes: &mut Vec<crate::shape::Shape>) {
    let style = size::wire_style(family, core);
    shapes.extend(branches.iter().map(|points| crate::shape::Shape::Wire { points: points.clone(), style }));
    shapes.extend(route::junctions(branches).into_iter().map(|(x, y)| crate::shape::Shape::Circle {
        x,
        y,
        radius: 300,
        paint: crate::shape::Paint::fill(style.token),
    }));
}

/// How far the pill of an inline constant reaches left of its terminal (notation 3.2).
pub fn constant_reach(constant: &str) -> i64 {
    size::TERMINAL / 2 + GRID + pill_width(&Text::new(constant, size::COMPACT_CONSTANT, MEDIUM))
}

/// The height of a terminal of a node, from the top of its block.
fn input_y(sized: &Sized, terminal: usize) -> i64 {
    sized.inputs.get(terminal).map_or(sized.height / 2, |at| at.1)
}

fn output_y(sized: &Sized, terminal: usize) -> i64 {
    sized.outputs.get(terminal).map_or(sized.height / 2, |at| at.1)
}

/// The state of a body while it is placed: local indices are positions in the canonical order of the body.
struct Body<'g> {
    graph: &'g Graph<'g>,
    nodes: &'g [usize],
    local: HashMap<usize, usize>,
    sized: Vec<&'g Sized>,
    /// The top of each block, from the axis.
    y: Vec<i64>,
    /// The heights of the terminals of the frame while the nodes are ordered: their places, 24 apart, the last on the
    /// axis.
    pseudo_left: Vec<i64>,
    pseudo_right: Vec<i64>,
}

impl Body<'_> {
    fn local_of(&self, end: &End) -> Option<usize> {
        match end {
            End::Node { node, .. } | End::Order(node) => self.local.get(node).copied(),
            End::Left(_) | End::Right(_) => None,
        }
    }

    /// The height of where a wire starts.
    fn start_y(&self, end: &End) -> i64 {
        match end {
            End::Node { node, terminal } => {
                self.local.get(node).map_or(0, |&local| self.top(local) + self.sized.get(local).map_or(0, |sized| output_y(sized, *terminal)))
            }
            End::Order(node) => self.local.get(node).map_or(0, |&local| self.top(local)),
            End::Left(index) => self.pseudo_left.get(*index).copied().unwrap_or(0),
            End::Right(index) => self.pseudo_right.get(*index).copied().unwrap_or(0),
        }
    }

    /// The height of where a wire ends.
    fn end_y(&self, end: &End) -> i64 {
        match end {
            End::Node { node, terminal } => {
                self.local.get(node).map_or(0, |&local| self.top(local) + self.sized.get(local).map_or(0, |sized| input_y(sized, *terminal)))
            }
            other => self.start_y(other),
        }
    }

    fn top(&self, local: usize) -> i64 {
        self.y.get(local).copied().unwrap_or(0)
    }

    fn height(&self, local: usize) -> i64 {
        self.sized.get(local).map_or(0, |sized| sized.height)
    }

    fn above(&self, local: usize) -> i64 {
        self.sized.get(local).map_or(0, |sized| sized.above)
    }

    /// Stacks a column from the bottom: its node of the chain, its pair on the axis, then the others in order.
    fn stack(&mut self, chain: Option<(usize, i64)>, members: &[usize], has_axis: bool) {
        let mut limit = match chain {
            Some((local, pair)) => {
                let top = -pair;
                if let Some(y) = self.y.get_mut(local) {
                    *y = top;
                }
                top - self.above(local) - STACK
            }
            None if has_axis => -OFF_AXIS,
            None => 0,
        };
        for &member in members {
            let top = limit - self.height(member);
            if let Some(y) = self.y.get_mut(member) {
                *y = top;
            }
            limit = top - self.above(member) - STACK;
        }
    }

    /// How far the wires go up and down, summed: what the sweeps make small.
    fn score(&self, wires: &[crate::graph::Wire]) -> i64 {
        wires.iter().fold(0i64, |sum, wire| sum.saturating_add((self.start_y(&wire.from) - self.end_y(&wire.to)).abs()))
    }
}

/// Places one body, from the top-left corner of its frame.
fn layout_body(graph: &Graph<'_>, index: usize, sized: &[Sized], header: i64, title: i64) -> BodyPlace {
    let Some(frame) = graph.bodies.get(index) else { return BodyPlace::default() };
    let nodes = frame.nodes.as_slice();
    let count = nodes.len();
    let empty = Sized::new(0, 0, Vec::new());
    let mut body = Body {
        graph,
        nodes,
        local: nodes.iter().enumerate().map(|(local, &node)| (node, local)).collect(),
        sized: nodes.iter().map(|&node| sized.get(node).unwrap_or(&empty)).collect(),
        y: vec![0; count],
        pseudo_left: vec![0; frame.left.len()],
        pseudo_right: vec![0; frame.right.len()],
    };
    for list in &frame.slots {
        let last = list.len().saturating_sub(1);
        for (place, slot) in list.iter().enumerate() {
            let y = -((last - place) as i64) * SLOT;
            if let Some(at) = slot.left.and_then(|left| body.pseudo_left.get_mut(left)) {
                *at = y;
            }
            if let Some(at) = slot.right.and_then(|right| body.pseudo_right.get_mut(right)) {
                *at = y;
            }
        }
    }
    let wires = frame.wires.as_slice();
    let mut incoming: Vec<Vec<usize>> = vec![Vec::new(); count];
    let mut outgoing: Vec<Vec<usize>> = vec![Vec::new(); count];
    for (wire_index, wire) in wires.iter().enumerate() {
        if let Some(list) = body.local_of(&wire.to).and_then(|local| incoming.get_mut(local)) {
            list.push(wire_index);
        }
        if let Some(list) = body.local_of(&wire.from).and_then(|local| outgoing.get_mut(local)) {
            list.push(wire_index);
        }
    }
    let columns = columns(&body, wires, &incoming, &outgoing);
    let chain = chain(&body, frame, wires, &incoming);
    // An error that crosses the body with no node on its way still has its line.
    let has_axis = !chain.is_empty() || frame.pair.0.is_some() || frame.pair.1.is_some();
    let column_count = columns.iter().copied().max().map_or(0, |last| last + 1);
    let mut chain_of: Vec<Option<(usize, i64)>> = vec![None; column_count];
    for &local in &chain {
        let pair = nodes.get(local).and_then(|&node| graph.nodes.get(node)).and_then(pair_of);
        let pair_y = pair.and_then(|(input, _)| body.sized.get(local).map(|sized| input_y(sized, input))).unwrap_or(0);
        if let Some(slot) = columns.get(local).and_then(|&column| chain_of.get_mut(column)) {
            *slot = Some((local, pair_y));
        }
    }
    let mut members: Vec<Vec<usize>> = vec![Vec::new(); column_count];
    for (local, &column) in columns.iter().enumerate() {
        if chain_of.get(column).copied().flatten().is_some_and(|(on_chain, _)| on_chain == local) {
            continue;
        }
        if let Some(list) = members.get_mut(column) {
            list.push(local);
        }
    }
    let stack_all = |body: &mut Body<'_>, members: &[Vec<usize>]| {
        for (column, list) in members.iter().enumerate() {
            body.stack(chain_of.get(column).copied().flatten(), list, has_axis);
        }
    };
    stack_all(&mut body, &members);
    // Sweeps: the key of a node is the mean height of the other ends of its wires, one way then the other.
    let mut best = (body.score(wires), members.clone());
    for _ in 0..SWEEPS {
        for forward in [true, false] {
            let order: Vec<usize> = if forward { (0..column_count).collect() } else { (0..column_count).rev().collect() };
            for column in order {
                let Some(list) = members.get(column) else { continue };
                let mut keyed: Vec<(i64, usize)> = list
                    .iter()
                    .map(|&local| {
                        let ends = if forward { incoming.get(local) } else { outgoing.get(local) };
                        let (sum, count) = ends.into_iter().flatten().filter_map(|&wire| wires.get(wire)).fold((0i64, 0i64), |(sum, count), wire| {
                            let y = if forward { body.start_y(&wire.from) } else { body.end_y(&wire.to) };
                            (sum.saturating_add(y), count + 1)
                        });
                        let key = if count > 0 { sum / count } else { body.top(local) + body.height(local) / 2 };
                        (key, local)
                    })
                    .collect();
                // The lowest first, nearest the axis; a tie keeps the canonical order.
                keyed.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
                let sorted: Vec<usize> = keyed.into_iter().map(|(_, local)| local).collect();
                body.stack(chain_of.get(column).copied().flatten(), &sorted, has_axis);
                if let Some(list) = members.get_mut(column) {
                    *list = sorted;
                }
            }
            let score = body.score(wires);
            if score < best.0 {
                best = (score, members.clone());
            }
        }
    }
    let members = best.1;
    stack_all(&mut body, &members);
    // The terminals of the frame, from the axis: each at the height of the first thing it meets.
    let mut left = vec![0i64; frame.left.len()];
    let mut right = vec![0i64; frame.right.len()];
    // The first wire from each terminal on the left, and into each on the right.
    let mut first_from: Vec<Option<usize>> = vec![None; frame.left.len()];
    let mut first_into: Vec<Option<usize>> = vec![None; frame.right.len()];
    for (index, wire) in wires.iter().enumerate() {
        if let End::Left(at) = wire.from
            && let Some(slot @ None) = first_from.get_mut(at)
        {
            *slot = Some(index);
        }
        if let End::Right(at) = wire.to
            && let Some(slot @ None) = first_into.get_mut(at)
        {
            *slot = Some(index);
        }
    }
    for list in &frame.slots {
        let prefer: Vec<Option<i64>> = list
            .iter()
            .map(|slot| {
                let from_left = slot
                    .left
                    .and_then(|left| first_from.get(left).copied().flatten())
                    .and_then(|wire| wires.get(wire))
                    .map(|wire| body.end_y(&wire.to));
                let from_right = slot
                    .right
                    .and_then(|right| first_into.get(right).copied().flatten())
                    .and_then(|wire| wires.get(wire))
                    .map(|wire| body.start_y(&wire.from));
                from_left.or(from_right)
            })
            .collect();
        let pinned: Vec<bool> = list
            .iter()
            .map(|slot| has_axis && ((slot.left.is_some() && slot.left == frame.pair.0) || (slot.right.is_some() && slot.right == frame.pair.1)))
            .collect();
        let heights = resolve(&prefer, &pinned);
        for (slot, y) in list.iter().zip(heights) {
            if let Some(at) = slot.left.and_then(|index| left.get_mut(index)) {
                *at = y;
            }
            if let Some(at) = slot.right.and_then(|index| right.get_mut(index)) {
                *at = y;
            }
        }
    }
    // The plan of the wires, on the heights alone: how wide each channel must be (notation 2.2).
    let extents = extents(&body, &columns, column_count);
    let sources: Vec<(End, crate::graph::Family)> = wires.iter().map(|wire| (wire.from, wire.family)).collect();
    let ys: Vec<i64> = (0..count).map(|local| body.top(local)).collect();
    let ends = wire_ends(&body, frame, &columns, column_count, &vec![0; count], &ys, &left, &right, 0);
    let axis = has_axis.then_some(0);
    let vias: Vec<Option<Vec<(i64, i64)>>> = (0..wires.len()).map(|wire| graph.vias.get(&(index, wire)).cloned()).collect();
    let plan = route::plan(&route::BodyIn { ends: &ends, sources: &sources, columns: column_count, extents: &extents, axis, vias: &vias });
    // Across: each column as wide as its widest node, its blocks aligned on the left, each channel as wide as its wires.
    let left_zone = frame.left.iter().map(|terminal| TerminalLabel::of(terminal).width()).max().map_or(0, |widest| LABEL_START + widest + GRID);
    let mut right_zone = frame.right.iter().map(|terminal| TerminalLabel::of(terminal).width()).max().map_or(0, |widest| LABEL_START + widest + GRID);
    // The pill of a conversion into a terminal on the right, before its name.
    for (wire_index, wire) in wires.iter().enumerate() {
        if let (End::Right(at), Some(pill), Some(terminal)) = (wire.to, graph.pills.get(&(index, wire_index)), wire_right(frame, wire)) {
            let _ = at;
            let reach = size::TERMINAL / 2 + GRID + pill_width(&Text::new(&pill.text, size::LABEL_LINE, MEDIUM));
            right_zone = right_zone.max(LABEL_START + TerminalLabel::of(terminal).width() + PADDING + reach + GRID);
        }
    }
    let mut x_of = vec![0i64; count];
    let mut channel_at = vec![0i64; column_count + 2];
    let mut x = left_zone;
    let mut by_column: Vec<Vec<usize>> = vec![Vec::new(); column_count];
    for (local, &column) in columns.iter().enumerate() {
        if let Some(list) = by_column.get_mut(column) {
            list.push(local);
        }
    }
    for (column, in_column) in by_column.iter().enumerate() {
        if let Some(at) = channel_at.get_mut(column + 1) {
            *at = x;
        }
        x += plan.channel(column + 1);
        let reach_left = in_column.iter().filter_map(|&local| body.sized.get(local)).map(|sized| sized.left).max().unwrap_or(0);
        let reach_right = in_column.iter().filter_map(|&local| body.sized.get(local)).map(|sized| sized.width + sized.right).max().unwrap_or(0);
        for &local in in_column {
            if let Some(at) = x_of.get_mut(local) {
                *at = x + reach_left;
            }
        }
        x += reach_left + reach_right;
    }
    if let Some(at) = channel_at.get_mut(column_count + 1) {
        *at = x;
    }
    x += plan.channel(column_count + 1);
    let width = size::up((x + right_zone).max(title).max(left_zone + plan.channel(1) + right_zone));
    // Down: from the header, a margin, what is highest, what is lowest, a margin.
    let tops = (0..count)
        .map(|local| body.top(local) - body.above(local))
        .chain(left.iter().chain(&right).map(|y| y - SLOT / 2))
        .chain(plan.highest.map(|track| track - TRACK_ROOM));
    let bottoms = (0..count)
        .map(|local| body.top(local) + body.height(local))
        .chain(left.iter().chain(&right).map(|y| y + SLOT / 2))
        .chain(plan.lowest.map(|track| track + GAP));
    let top = tops.min().unwrap_or(0);
    let bottom = bottoms.max().unwrap_or(0);
    let mut shift = header + PAD - top;
    // The axis goes through the first node of the chain, where the file puts it (notation 2.1).
    if let Some(&first) = chain.first()
        && let Some(at) = nodes.get(first).and_then(|&node| graph.nodes.get(node)).and_then(|node| node.at)
    {
        shift = at.1 - body.top(first);
    }
    let mut height = shift + bottom + PAD;
    let mut placed: Vec<(usize, NodeAt)> = Vec::with_capacity(count);
    let ranks = ranks(&members, &chain_of);
    for local in 0..count {
        let Some(&node) = nodes.get(local) else { continue };
        let column = columns.get(local).copied().unwrap_or(0);
        let rank = ranks.get(&local).copied().unwrap_or(0);
        placed.push((node, NodeAt { x: x_of.get(local).copied().unwrap_or(0), y: body.top(local) + shift, column: column as u32, rank }));
    }
    let mut width = width;
    keep_placed(graph, &body, &mut placed, &members);
    let mut highest = header + PAD;
    for (local, (_, at)) in placed.iter().enumerate() {
        let Some(sized) = body.sized.get(local) else { continue };
        width = width.max(size::up(at.x + sized.width + sized.right + MIN_CHANNEL));
        height = height.max(at.y + sized.height + PAD);
        highest = highest.min(at.y - sized.above);
    }
    // The wires, where the nodes ended.
    let xs: Vec<i64> = placed.iter().map(|(_, at)| at.x).collect();
    let ys: Vec<i64> = placed.iter().map(|(_, at)| at.y).collect();
    let left_at: Vec<i64> = left.iter().map(|y| y + shift).collect();
    let right_at: Vec<i64> = right.iter().map(|y| y + shift).collect();
    let ends = wire_ends(&body, frame, &columns, column_count, &xs, &ys, &left_at, &right_at, width);
    let mut placed_extents: Vec<Option<(i64, i64)>> = vec![None; column_count + 2];
    for (local, &column) in columns.iter().enumerate() {
        let (Some(sized), Some(&y)) = (body.sized.get(local), ys.get(local)) else { continue };
        if let Some(slot) = placed_extents.get_mut(column + 1) {
            let (top, bottom) = (y - sized.above, y + sized.height);
            *slot = Some(slot.map_or((top, bottom), |(a, b)| (a.min(top), b.max(bottom))));
        }
    }
    let axis = has_axis.then_some(shift);
    let plan = route::plan(&route::BodyIn { ends: &ends, sources: &sources, columns: column_count, extents: &placed_extents, axis, vias: &vias });
    let routes = route::geometry(&plan, &ends, &channel_at);
    if let Some(track) = plan.highest {
        highest = highest.min(track - TRACK_ROOM);
    }
    if let Some(track) = plan.lowest {
        height = height.max(track + GAP + PAD);
    }
    let rise = size::up(header + PAD - highest);
    BodyPlace {
        width,
        height,
        header,
        nodes: placed,
        left: left.iter().map(|y| y + shift).collect(),
        right: right.iter().map(|y| y + shift).collect(),
        axis: has_axis.then_some(shift),
        rise,
        routes,
        from_file: nodes.iter().any(|&node| graph.nodes.get(node).is_some_and(|node| node.at.is_some())),
    }
}

/// The terminal on the right of the frame that a wire reaches, if it reaches one.
fn wire_right<'f>(frame: &'f BodyOf, wire: &crate::graph::Wire) -> Option<&'f FrameTerminal> {
    match wire.to {
        End::Right(at) => frame.right.get(at),
        _ => None,
    }
}

/// Above the highest track: room for the name of its wire (notation 4).
const TRACK_ROOM: i64 = 2400;

/// The top and the bottom of what each column holds, by column from the left edge (0) to the right edge (`n + 1`).
fn extents(body: &Body<'_>, columns: &[usize], column_count: usize) -> Vec<Option<(i64, i64)>> {
    let mut extents: Vec<Option<(i64, i64)>> = vec![None; column_count + 2];
    for (local, &column) in columns.iter().enumerate() {
        let (top, bottom) = (body.top(local) - body.above(local), body.top(local) + body.height(local));
        if let Some(slot) = extents.get_mut(column + 1) {
            *slot = Some(slot.map_or((top, bottom), |(a, b)| (a.min(top), b.max(bottom))));
        }
    }
    extents
}

/// Where each wire of a body leaves and reaches its terminals, with the columns of its ends: a node at `xs` and `ys`, a
/// terminal of the frame past its name, the frame `width` wide.
#[allow(clippy::too_many_arguments)]
fn wire_ends(
    body: &Body<'_>,
    frame: &BodyOf,
    columns: &[usize],
    column_count: usize,
    xs: &[i64],
    ys: &[i64],
    left: &[i64],
    right: &[i64],
    width: i64,
) -> Vec<Option<Ends>> {
    let node = |node: usize| -> Option<(usize, &Sized, i64, i64)> {
        let local = *body.local.get(&node)?;
        Some((columns.get(local)? + 1, *body.sized.get(local)?, *xs.get(local)?, *ys.get(local)?))
    };
    let start = |end: &End| -> Option<(usize, At)> {
        match *end {
            End::Node { node: index, terminal } => {
                let (column, sized, x, y) = node(index)?;
                let &(dx, dy) = sized.outputs.get(terminal)?;
                Some((column, At::side(x + dx, y + dy, Side::Right)))
            }
            // An order wire from a label starts right after the label.
            End::Order(index) => {
                let (column, sized, x, y) = node(index)?;
                let (dx, dy) = sized.label_end.unwrap_or((sized.width, GRID));
                Some((column, At::side(x + dx, y + dy, Side::Right)))
            }
            End::Left(index) => {
                let terminal = frame.left.get(index)?;
                Some((0, At::side(LABEL_START + TerminalLabel::of(terminal).width() + PADDING, *left.get(index)?, Side::Right)))
            }
            End::Right(_) => None,
        }
    };
    let finish = |end: &End| -> Option<(usize, At)> {
        match *end {
            End::Node { node: index, terminal } => {
                let (column, sized, x, y) = node(index)?;
                let &(dx, dy) = sized.inputs.get(terminal)?;
                Some((column, At::side(x + dx, y + dy, Side::Left)))
            }
            End::Order(index) => {
                let (column, sized, x, y) = node(index)?;
                Some(match sized.order {
                    // On the top edge: reached from above, past what is above the node.
                    Some((dx, dy)) if dy <= 0 && dx > 0 => {
                        (column, At { x: x + dx, y: y + dy, side: Side::Top, approach: y + dy - sized.above - GAP })
                    }
                    Some((dx, dy)) => (column, At::side(x + dx, y + dy, Side::Left)),
                    None => (column, At::side(x, y + sized.height / 2, Side::Left)),
                })
            }
            End::Right(index) => {
                let terminal = frame.right.get(index)?;
                let x = width - LABEL_START - TerminalLabel::of(terminal).width() - PADDING;
                Some((column_count + 1, At::side(x, *right.get(index)?, Side::Left)))
            }
            End::Left(_) => None,
        }
    };
    frame
        .wires
        .iter()
        .map(|wire| {
            let (from_column, from) = start(&wire.from)?;
            let (to_column, to) = finish(&wire.to)?;
            Some(Ends { from_column, from, to_column, to })
        })
        .collect()
}

/// The column of each node (notation 2.1), by its place in the canonical order.
fn columns(body: &Body<'_>, wires: &[crate::graph::Wire], incoming: &[Vec<usize>], outgoing: &[Vec<usize>]) -> Vec<usize> {
    let count = body.nodes.len();
    let fed: Vec<bool> = incoming.iter().map(|list| !list.is_empty()).collect();
    let mut column: Vec<Option<i64>> = vec![None; count];
    for local in 0..count {
        if !fed.get(local).copied().unwrap_or(false) {
            continue;
        }
        let mut at = 0;
        for wire in incoming.get(local).into_iter().flatten().filter_map(|&wire| wires.get(wire)) {
            // Only what comes before in the order: a body that cannot be ordered keeps its written order.
            if let Some(from) = body.local_of(&wire.from).filter(|&from| from < local && fed.get(from).copied().unwrap_or(false))
                && let Some(Some(before)) = column.get(from)
            {
                at = at.max(before + 1);
            }
        }
        if let Some(slot) = column.get_mut(local) {
            *slot = Some(at);
        }
    }
    let last = column.iter().flatten().copied().max().unwrap_or(0);
    let mut opened = false;
    for local in 0..count {
        if fed.get(local).copied().unwrap_or(false) {
            continue;
        }
        let readers = outgoing.get(local).into_iter().flatten().filter_map(|&wire| wires.get(wire)).filter_map(|wire| body.local_of(&wire.to));
        let at = readers.filter_map(|reader| column.get(reader).copied().flatten()).min().map_or(last, |leftmost| leftmost - 1);
        opened |= at < 0;
        if let Some(slot) = column.get_mut(local) {
            *slot = Some(at);
        }
    }
    column.into_iter().map(|at| usize::try_from(at.unwrap_or(0) + i64::from(opened)).unwrap_or(0)).collect()
}

/// The main error chain of a body (notation 2.1), by local index: the longest chain of error pairs from where the error
/// enters the body; on a tie, the one that reaches where it leaves, then the first in the canonical order.
fn chain(body: &Body<'_>, frame: &crate::graph::BodyOf, wires: &[crate::graph::Wire], incoming: &[Vec<usize>]) -> Vec<usize> {
    let count = body.nodes.len();
    let pairs: Vec<Option<(usize, usize)>> = body.nodes.iter().map(|&node| body.graph.nodes.get(node).and_then(pair_of)).collect();
    let pair = |local: usize| pairs.get(local).copied().flatten();
    let mut next: Vec<Vec<usize>> = vec![Vec::new(); count];
    let mut starts = Vec::new();
    let mut ends = vec![false; count];
    for wire in wires {
        match (wire.from, wire.to) {
            (End::Node { node: from, terminal: out }, End::Node { node: to, terminal: input }) => {
                let (Some(&a), Some(&b)) = (body.local.get(&from), body.local.get(&to)) else { continue };
                if b > a
                    && pair(a).is_some_and(|(_, output)| output == out)
                    && pair(b).is_some_and(|(entry, _)| entry == input)
                    && let Some(list) = next.get_mut(a)
                {
                    list.push(b);
                }
            }
            (End::Left(left), End::Node { node, terminal }) if Some(left) == frame.pair.0 => {
                if let Some(&local) = body.local.get(&node)
                    && pair(local).is_some_and(|(entry, _)| entry == terminal)
                {
                    starts.push(local);
                }
            }
            (End::Node { node, terminal }, End::Right(right)) if Some(right) == frame.pair.1 => {
                if let Some(&local) = body.local.get(&node)
                    && pair(local).is_some_and(|(_, output)| output == terminal)
                    && let Some(end) = ends.get_mut(local)
                {
                    *end = true;
                }
            }
            _ => {}
        }
    }
    if starts.is_empty() {
        // No error enters: the first node, in the order, whose error input is not wired.
        starts.extend((0..count).find(|&local| {
            pair(local).is_some_and(|(entry, _)| {
                !incoming
                    .get(local)
                    .into_iter()
                    .flatten()
                    .filter_map(|&wire| wires.get(wire))
                    .any(|wire| matches!(wire.to, End::Node { terminal, .. } if terminal == entry))
            })
        }));
    }
    // From the last: the length of the best chain from each node, whether it reaches the end, and what follows.
    let mut best: Vec<(usize, bool, Option<usize>)> = vec![(0, false, None); count];
    for local in (0..count).rev() {
        if pair(local).is_none() {
            continue;
        }
        let mut found = (1, ends.get(local).copied().unwrap_or(false), None);
        for &after in next.get(local).into_iter().flatten() {
            let Some(&(length, reaches, _)) = best.get(after) else { continue };
            let better = (length + 1, reaches) > (found.0, found.1)
                || ((length + 1, reaches) == (found.0, found.1) && found.2.is_some_and(|other| after < other));
            if better {
                found = (length + 1, reaches, Some(after));
            }
        }
        if let Some(slot) = best.get_mut(local) {
            *slot = found;
        }
    }
    let mut start: Option<usize> = None;
    for &candidate in &starts {
        let Some(&(length, reaches, _)) = best.get(candidate) else { continue };
        let better = match start.and_then(|start| best.get(start).map(|found| (start, found))) {
            None => true,
            Some((start, &(other_length, other_reaches, _))) => {
                (length, reaches) > (other_length, other_reaches) || ((length, reaches) == (other_length, other_reaches) && candidate < start)
            }
        };
        if better {
            start = Some(candidate);
        }
    }
    let mut chain = Vec::new();
    let mut current = start;
    while let Some(local) = current {
        chain.push(local);
        current = best.get(local).and_then(|found| found.2);
    }
    chain
}

/// The rank of each node in its column, from the bottom: the node of the chain first.
fn ranks(members: &[Vec<usize>], chain_of: &[Option<(usize, i64)>]) -> HashMap<usize, u32> {
    let mut ranks = HashMap::new();
    for (column, list) in members.iter().enumerate() {
        let on_chain = chain_of.get(column).copied().flatten();
        if let Some((local, _)) = on_chain {
            ranks.insert(local, 0);
        }
        let first = u32::from(on_chain.is_some());
        for (rank, &local) in list.iter().enumerate() {
            ranks.insert(local, first + rank as u32);
        }
    }
    ranks
}

/// The heights of the terminals of one list, from the top: each where it prefers, at least `SLOT` below the one before,
/// a pinned one on the axis.
pub(crate) fn resolve(prefer: &[Option<i64>], pinned: &[bool]) -> Vec<i64> {
    let count = prefer.len();
    // What has no preference goes after the one before it, or before the one after: the nearest known on each side,
    // found in one pass each way.
    let mut before: Vec<Option<(usize, i64)>> = vec![None; count];
    let mut last = None;
    for (index, slot) in before.iter_mut().enumerate() {
        *slot = last;
        if let Some(y) = prefer.get(index).copied().flatten() {
            last = Some((index, y));
        }
    }
    let mut after: Vec<Option<(usize, i64)>> = vec![None; count];
    let mut next = None;
    for (index, slot) in after.iter_mut().enumerate().rev() {
        *slot = next;
        if let Some(y) = prefer.get(index).copied().flatten() {
            next = Some((index, y));
        }
    }
    let mut wanted: Vec<i64> = vec![0; count];
    for (index, slot) in wanted.iter_mut().enumerate() {
        if pinned.get(index).copied().unwrap_or(false) {
            *slot = 0;
            continue;
        }
        *slot = match prefer.get(index).copied().flatten() {
            Some(y) => y,
            None => match (before.get(index).copied().flatten(), after.get(index).copied().flatten()) {
                (Some((at, y)), _) => y + (index - at) as i64 * SLOT,
                (None, Some((at, y))) => y - (at - index) as i64 * SLOT,
                (None, None) => -((count - 1 - index) as i64) * SLOT,
            },
        };
    }
    // Down, then up: a pinned one never moves, and pushes those above it.
    let mut heights = wanted;
    for index in 1..count {
        let before = heights.get(index - 1).copied().unwrap_or(0);
        if let Some(y) = heights.get_mut(index)
            && !pinned.get(index).copied().unwrap_or(false)
        {
            *y = (*y).max(before + SLOT);
        }
    }
    for index in (0..count.saturating_sub(1)).rev() {
        let after = heights.get(index + 1).copied().unwrap_or(0);
        if let Some(y) = heights.get_mut(index) {
            *y = (*y).min(after - SLOT);
        }
    }
    heights
}

/// The nodes that the `layout` section places keep their place; a node that it does not place, and that would cover
/// one, moves up above it, and those above it in its column as far as needed (notation 2.1).
fn keep_placed(graph: &Graph<'_>, body: &Body<'_>, placed: &mut [(usize, NodeAt)], members: &[Vec<usize>]) {
    let fixed: Vec<usize> = (0..placed.len())
        .filter(|&local| body.nodes.get(local).and_then(|&node| graph.nodes.get(node)).is_some_and(|node| node.at.is_some()))
        .collect();
    if fixed.is_empty() {
        return;
    }
    let mut is_fixed = vec![false; placed.len()];
    for &local in &fixed {
        if let Some(flag) = is_fixed.get_mut(local) {
            *flag = true;
        }
    }
    for &local in &fixed {
        let at = body.nodes.get(local).and_then(|&node| graph.nodes.get(node)).and_then(|node| node.at);
        if let (Some((_, slot)), Some((x, y))) = (placed.get_mut(local), at) {
            slot.x = x;
            slot.y = y;
        }
    }
    let span = |local: usize, placed: &[(usize, NodeAt)]| -> Option<(i64, i64, i64, i64)> {
        let sized = body.sized.get(local)?;
        let at = placed.get(local)?.1;
        Some((at.x - sized.left, at.x + sized.width + sized.right, at.y - sized.above, at.y + sized.height))
    };
    // Across, from the left: the placed nodes that reach into each column, found in one sweep, so that the work follows
    // the overlaps rather than the columns times the placed nodes.
    let mut spans: Vec<(i64, i64, usize)> = members
        .iter()
        .enumerate()
        .filter_map(|(column, list)| {
            let reach = list.iter().filter(|&&local| !is_fixed.get(local).copied().unwrap_or(false)).filter_map(|&local| span(local, placed));
            let (left, right) = reach.fold((i64::MAX, i64::MIN), |(left, right), (x0, x1, _, _)| (left.min(x0), right.max(x1)));
            (left < right).then_some((left, right, column))
        })
        .collect();
    spans.sort_unstable();
    let mut by_left: Vec<(i64, i64, i64, i64)> = fixed.iter().filter_map(|&local| span(local, placed)).collect();
    by_left.sort_unstable();
    let mut waiting = by_left.into_iter().peekable();
    let mut reaching: BinaryHeap<Reverse<(i64, i64, i64)>> = BinaryHeap::new();
    for (left, right, column) in spans {
        let Some(list) = members.get(column) else { continue };
        while let Some(&(x0, x1, y0, y1)) = waiting.peek() {
            if x0 >= right {
                break;
            }
            reaching.push(Reverse((x1, y0, y1)));
            waiting.next();
        }
        while reaching.peek().is_some_and(|Reverse((x1, _, _))| *x1 <= left) {
            reaching.pop();
        }
        // The placed nodes in the way of this column, the lowest first.
        let mut obstacles: Vec<(i64, i64)> = reaching.iter().map(|Reverse((_, y0, y1))| (y0 - STACK, y1 + STACK)).collect();
        obstacles.sort_by(|a, b| b.1.cmp(&a.1).then(b.0.cmp(&a.0)));
        let mut next = 0;
        let mut limit = i64::MAX;
        for &local in list {
            if is_fixed.get(local).copied().unwrap_or(false) {
                continue;
            }
            let (Some(sized), Some(at)) = (body.sized.get(local), placed.get(local).map(|slot| slot.1)) else { continue };
            let mut bottom = (at.y + sized.height).min(limit);
            while let Some(&(top_of, bottom_of)) = obstacles.get(next) {
                let top = bottom - sized.height - sized.above;
                if top_of >= bottom {
                    // Below: never in the way again, since what follows is higher.
                    next += 1;
                } else if bottom_of > top {
                    bottom = top_of;
                    next += 1;
                } else {
                    break;
                }
            }
            let y = bottom - sized.height;
            if let Some(slot) = placed.get_mut(local) {
                slot.1.y = y;
            }
            limit = y - sized.above - STACK;
        }
    }
}
