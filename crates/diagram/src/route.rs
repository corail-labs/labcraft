//! Step 4 (`docs/architecture.md`, 11.3): routing (notation 2.2), body by body, in two passes.
//!
//! - **The plan**, which needs the heights only, so that placement can make each channel as wide as it needs before
//!   anything is placed across: the wires are grouped by the value they carry, one net per source. A reader in the
//!   column after the source is reached by a vertical of the net in the channel between them; a reader further on by a
//!   horizontal track above the nodes of the columns that the net passes over, and a vertical in the channel before its
//!   column. An error that leaves the main chain runs below it instead. The verticals of a channel each get a slot, 8
//!   apart, so that no two share a segment; the tracks stack 8 apart, above what they pass over.
//! - **The geometry**, once the columns are placed: each wire as a polyline from its source to its reader, the trunk
//!   of a net shared by its branches, and a junction point where a branch leaves it.
//!
//! Columns are counted from the left edge of the frame (0) to its right edge (`n + 1`), the nodes in `1..=n`; channel
//! `k` lies between columns `k - 1` and `k`. Every step is linear in the wires but for sorts and the trees of the slots
//! and the tracks.

use std::collections::BTreeMap;

use crate::graph::{End, Family};

/// Between two parallel wires, and between a wire and the edge of its channel.
pub const GAP: i64 = 800;
/// Between a track and what it passes over.
pub const CLEAR: i64 = 1600;
/// The narrowest channel: a margin on each side (notation 2.2).
pub const MIN_CHANNEL: i64 = 1600;

/// The side of a terminal, which the wire leaves or reaches in that direction (notation 2.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    /// The `after:` terminal on the top edge of a compact block: reached from above.
    Top,
}

/// Where a wire leaves or reaches a terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct At {
    pub x: i64,
    pub y: i64,
    pub side: Side,
    /// The height of the horizontal that reaches it: its own height, or above its node for the top edge.
    pub approach: i64,
}

impl At {
    pub fn side(x: i64, y: i64, side: Side) -> At {
        At { x, y, side, approach: y }
    }
}

/// The two ends of a wire, with their columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ends {
    pub from_column: usize,
    pub from: At,
    pub to_column: usize,
    pub to: At,
}

/// What the plan needs of a body.
pub struct BodyIn<'a> {
    /// By wire of the body; `None` for a wire whose ends are not drawn.
    pub ends: &'a [Option<Ends>],
    /// By wire: the source that groups it in a net, its family.
    pub sources: &'a [(End, Family)],
    /// The columns of nodes, `n`.
    pub columns: usize,
    /// By column `0..=n + 1`: the top of what it holds (a band or a label included) and its bottom; `None` when empty.
    pub extents: &'a [Option<(i64, i64)>],
    /// The height of the main error chain.
    pub axis: Option<i64>,
    /// By wire: the waypoints that the file gives it (spec 11).
    pub vias: &'a [Option<Vec<(i64, i64)>>],
}

/// A vertical of a net in a channel: from `y0` to `y1`, entered from the left at `entry`, left to the right at `outs`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Vertical {
    /// The net it belongs to.
    net: usize,
    channel: usize,
    y0: i64,
    y1: i64,
    entry: i64,
    outs: Vec<i64>,
    slot: u32,
    /// The part that leads into it when it jogs: the wire enters there, goes down that part to `entry`, then across.
    lead: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NetPlan {
    /// The wires of the net, by index in the body.
    wires: Vec<usize>,
    source: At,
    source_column: usize,
    /// The vertical in the channel after the source.
    first: Option<usize>,
    /// The height of the track, above the nodes, or below the chain.
    track: Option<i64>,
    /// By column reached beyond the next one: its vertical, if the readers there are not at the height of the track.
    drops: BTreeMap<usize, Option<usize>>,
}

/// The plan of a body: how many slots each channel holds, how far the tracks go, and each net.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// By channel `0..=n + 1` (channel 0 is never used).
    pub slots: Vec<u32>,
    /// The highest track above the nodes, and the lowest below the chain.
    pub highest: Option<i64>,
    pub lowest: Option<i64>,
    verticals: Vec<Vertical>,
    nets: Vec<NetPlan>,
    /// Wires that go back to the left, which only a position of the file gives: each routed around on its own.
    back: Vec<usize>,
    /// Wires with waypoints, each routed through them on its own.
    through: Vec<(usize, Vec<(i64, i64)>)>,
    /// By wire: an error that stays on the axis, straight, past columns that hold nothing on it (notation 2.1).
    straight: Vec<bool>,
}

impl Plan {
    /// The width that channel `k` needs: its slots 8 apart, a margin of 8 on each side.
    pub fn channel(&self, k: usize) -> i64 {
        let slots = i64::from(self.slots.get(k).copied().unwrap_or(0));
        (GAP * (slots + 1)).max(MIN_CHANNEL)
    }
}

/// A net routed: a polyline per wire, from its source to its reader, and the junction points of the net.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Routed {
    pub wires: Vec<usize>,
    pub branches: Vec<Vec<(i64, i64)>>,
    pub junctions: Vec<(i64, i64)>,
    /// By branch: whether it ends on the right edge of the frame.
    pub right: Vec<bool>,
}

impl Routed {
    /// The branches that end on the right edge reach it `dx` further right.
    pub fn stretched(mut self, dx: i64) -> Routed {
        if dx == 0 {
            return self;
        }
        for (branch, &right) in self.branches.iter_mut().zip(&self.right) {
            if let (true, Some(last)) = (right, branch.last_mut()) {
                last.0 += dx;
            }
        }
        self
    }

    pub fn moved(mut self, dx: i64, dy: i64) -> Routed {
        for branch in &mut self.branches {
            for point in branch.iter_mut() {
                *point = (point.0 + dx, point.1 + dy);
            }
        }
        for point in &mut self.junctions {
            *point = (point.0 + dx, point.1 + dy);
        }
        self
    }
}

/// The plan of a body (see the module).
pub fn plan(body: &BodyIn<'_>) -> Plan {
    let channels = body.columns + 2;
    let mut plan = Plan { slots: vec![0; channels], ..Plan::default() };
    // The nets, by source, in the order of their first wire.
    let mut by_source: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    let mut first_of: std::collections::HashMap<(End, bool), usize> = std::collections::HashMap::new();
    for (index, ends) in body.ends.iter().enumerate() {
        let (Some(ends), Some(&(source, family))) = (ends, body.sources.get(index)) else { continue };
        if let Some(Some(points)) = body.vias.get(index) {
            plan.through.push((index, points.clone()));
            continue;
        }
        if ends.to_column <= ends.from_column {
            plan.back.push(index);
            continue;
        }
        let _ = family;
        let first = *first_of.entry((source, false)).or_insert(index);
        by_source.entry(first).or_default().push(index);
    }
    let mut tops = Tree::new(body.extents.iter().map(|extent| extent.map_or(i64::MAX, |(top, _)| top)).collect(), i64::MAX, i64::min);
    let mut bottoms = Tree::new(body.extents.iter().map(|extent| extent.map_or(i64::MIN, |(_, bottom)| bottom)).collect(), i64::MIN, i64::max);
    // An error from the axis to the axis runs straight along it when no column on its way holds a node there: the main
    // chain past a structure off the chain, or to the `err` of the frame.
    let mut on_axis = vec![0usize; channels + 1];
    for (column, extent) in body.extents.iter().enumerate() {
        let held = body.axis.is_some_and(|axis| extent.is_some_and(|(top, bottom)| top <= axis && bottom > axis));
        if let Some(slot) = on_axis.get_mut(column + 1) {
            *slot = usize::from(held);
        }
    }
    for column in 1..on_axis.len() {
        let before = on_axis.get(column - 1).copied().unwrap_or(0);
        if let Some(slot) = on_axis.get_mut(column) {
            *slot += before;
        }
    }
    plan.straight = body
        .ends
        .iter()
        .zip(body.sources)
        .map(|(ends, (_, family))| {
            let Some(ends) = ends else { return false };
            let (from, to) = (ends.from_column + 1, ends.to_column);
            *family == Family::ERROR
                && body.axis == Some(ends.from.y)
                && ends.to.approach == ends.from.y
                && to > from
                && on_axis.get(to).copied().unwrap_or(0) == on_axis.get(from).copied().unwrap_or(0)
        })
        .collect();
    let straight = |wire: usize| plan.straight.get(wire).copied().unwrap_or(false);
    let reach = |wire: usize| if straight(wire) { None } else { body.ends.get(wire).copied().flatten().map(|ends| ends.to_column) };
    // The tracks: the shortest spans first, nearest the nodes.
    let mut nets: Vec<NetPlan> = Vec::new();
    let mut spans: Vec<(usize, usize, usize)> = Vec::new();
    for wires in by_source.into_values() {
        let Some(first) = wires.first().and_then(|&wire| body.ends.get(wire).copied().flatten()) else { continue };
        let farthest = wires.iter().filter_map(|&wire| reach(wire)).max().unwrap_or(0);
        let index = nets.len();
        if farthest > first.from_column + 1 {
            spans.push((farthest - first.from_column, first.from_column, index));
        }
        nets.push(NetPlan { wires, source: first.from, source_column: first.from_column, first: None, track: None, drops: BTreeMap::new() });
    }
    spans.sort_unstable();
    // The heights where a wire leaves or reaches each channel, with its net: a track that starts at the height of its
    // source crosses the whole channel after it, which must hold no other end within 8.
    let mut ends_in: Vec<Vec<(i64, usize)>> = vec![Vec::new(); channels];
    for (net, plan_of) in nets.iter().enumerate() {
        if let Some(list) = ends_in.get_mut(plan_of.source_column + 1) {
            list.push((plan_of.source.y, net));
        }
        for &wire in &plan_of.wires {
            if let Some(ends) = body.ends.get(wire).copied().flatten()
                && let Some(list) = ends_in.get_mut(ends.to_column)
            {
                list.push((ends.to.approach, net));
            }
        }
    }
    for list in &mut ends_in {
        list.sort_unstable();
    }
    let alone = |channel: usize, y: i64, net: usize| {
        let Some(list) = ends_in.get(channel) else { return true };
        let low = list.partition_point(|&(other, _)| other <= y - GAP);
        list.iter().skip(low).take_while(|&&(other, _)| other < y + GAP).all(|&(_, other)| other == net)
    };
    // Positions of the skylines: channel `k` at `2k`, column `c` at `2c + 1`.
    let positions = 2 * channels;
    let mut above = Tree::new(vec![i64::MAX; positions], i64::MAX, i64::min);
    let mut below = Tree::new(vec![i64::MIN; positions], i64::MIN, i64::max);
    for &(_, from_column, index) in &spans {
        let Some(net) = nets.get_mut(index) else { continue };
        let farthest = net.wires.iter().filter_map(|&wire| reach(wire)).max().unwrap_or(0);
        let error = net.wires.iter().any(|&wire| body.sources.get(wire).is_some_and(|(_, family)| *family == Family::ERROR));
        let leaves_chain = error && body.axis == Some(net.source.y);
        let (start, end) = (2 * (from_column + 1), 2 * farthest);
        let columns = (from_column + 1, farthest.saturating_sub(1));
        if leaves_chain {
            let lowest = bottoms.query(columns.0, columns.1).max(body.axis.unwrap_or(i64::MIN));
            let track = lowest.saturating_add(CLEAR).max(below.query(start, end).saturating_add(GAP));
            below.update(start, end, track);
            net.track = Some(track);
            plan.lowest = Some(plan.lowest.map_or(track, |lowest| lowest.max(track)));
        } else {
            let limit = tops.query(columns.0, columns.1).saturating_sub(CLEAR).min(above.query(start, end).saturating_sub(GAP));
            let track = if net.source.y <= limit && alone(from_column + 1, net.source.y, index) { net.source.y } else { limit };
            above.update(start, end, track);
            net.track = Some(track);
            plan.highest = Some(plan.highest.map_or(track, |highest| highest.min(track)));
        }
    }
    // The verticals of each net.
    for (index, net) in nets.iter_mut().enumerate() {
        let mut first: Vec<i64> = vec![net.source.y];
        let mut drops: BTreeMap<usize, Vec<i64>> = BTreeMap::new();
        for &wire in &net.wires {
            let Some(ends) = body.ends.get(wire).copied().flatten() else { continue };
            if straight(wire) {
                continue;
            }
            if ends.to_column == net.source_column + 1 {
                first.push(ends.to.approach);
            } else if let Some(track) = net.track {
                drops.entry(ends.to_column).or_insert_with(|| vec![track]).push(ends.to.approach);
            }
        }
        if let Some(track) = net.track {
            first.push(track);
        }
        net.first = vertical(&mut plan.verticals, index, net.source_column + 1, net.source.y, first.get(1..).unwrap_or_default());
        for (column, heights) in drops {
            let made = vertical(&mut plan.verticals, index, column, net.track.unwrap_or(0), heights.get(1..).unwrap_or_default());
            net.drops.insert(column, made);
        }
    }
    plan.nets = nets;
    jogs(&mut plan, channels);
    slots(&mut plan, channels);
    plan
}

/// Two wires that cross in a channel with an entry of each within 8 of an exit of the other: in whichever order their
/// verticals stand, two of their horizontals run closer than 8. The later of the two jogs: it enters by a vertical of
/// its own, left of the other, goes across at a height free in the channel, and goes on by its vertical, right of it.
fn jogs(plan: &mut Plan, channels: usize) {
    let mut by_channel: Vec<Vec<usize>> = vec![Vec::new(); channels];
    for (index, vertical) in plan.verticals.iter().enumerate() {
        if let Some(list) = by_channel.get_mut(vertical.channel) {
            list.push(index);
        }
    }
    for list in by_channel {
        // Every height where a wire enters or leaves the channel.
        let mut heights: Vec<i64> = list
            .iter()
            .filter_map(|&index| plan.verticals.get(index))
            .flat_map(|vertical| vertical.outs.iter().copied().chain([vertical.entry]))
            .collect();
        heights.sort_unstable();
        let near = |a: i64, b: i64| (a - b).abs() < GAP;
        let mut split = Vec::new();
        for (position, &one) in list.iter().enumerate() {
            for &other in list.iter().skip(position + 1) {
                let (Some(a), Some(b)) = (plan.verticals.get(one), plan.verticals.get(other)) else { continue };
                let crossing = a.outs.iter().any(|&out| near(out, b.entry)) && b.outs.iter().any(|&out| near(out, a.entry));
                if crossing && !split.contains(&other) {
                    split.push(other);
                }
            }
        }
        for index in split {
            let Some(vertical) = plan.verticals.get(index).cloned() else { continue };
            // A height on the grid inside it, free of every other entry and exit, the nearest its middle; none in 64 tries
            // leaves it as it is.
            let middle = (vertical.y0 + vertical.y1) / 2;
            let free = |y: i64| {
                let low = heights.partition_point(|&known| known <= y - GAP);
                heights.get(low).is_none_or(|&known| known >= y + GAP)
            };
            let found = (0..64i64)
                .flat_map(|step| [middle - middle.rem_euclid(GAP) - step * GAP, middle - middle.rem_euclid(GAP) + (step + 1) * GAP])
                .find(|&y| y > vertical.y0 && y < vertical.y1 && free(y));
            let Some(across) = found else { continue };
            let lead = Vertical {
                net: vertical.net,
                channel: vertical.channel,
                y0: vertical.entry.min(across),
                y1: vertical.entry.max(across),
                entry: vertical.entry,
                outs: vec![across],
                slot: 0,
                lead: None,
            };
            plan.verticals.push(lead);
            let lead_index = plan.verticals.len() - 1;
            if let Some(main) = plan.verticals.get_mut(index) {
                main.entry = across;
                main.y0 = main.outs.iter().copied().chain([across]).min().unwrap_or(across);
                main.y1 = main.outs.iter().copied().chain([across]).max().unwrap_or(across);
                main.lead = Some(lead_index);
            }
            if let Some(slot) = heights.binary_search(&across).err() {
                heights.insert(slot, across);
            }
        }
    }
}

/// A vertical from `entry` through the heights it leaves at, unless they are all one.
fn vertical(verticals: &mut Vec<Vertical>, net: usize, channel: usize, entry: i64, outs: &[i64]) -> Option<usize> {
    let y0 = outs.iter().copied().chain([entry]).min()?;
    let y1 = outs.iter().copied().chain([entry]).max()?;
    if y0 == y1 {
        return None;
    }
    let outs = outs.iter().copied().filter(|&out| out != entry).collect();
    verticals.push(Vertical { net, channel, y0, y1, entry, outs, slot: 0, lead: None });
    Some(verticals.len() - 1)
}

/// The slots of the verticals of each channel. A vertical whose wire enters from the left at the height where another
/// leaves to the right, within 8, stands left of it, or the two would share a segment. Otherwise those that rise sit
/// left of those that fall, so that a wire leaving a channel does not cross the vertical beside it: of two that rise,
/// the one that enters higher stands further left; of two that fall, the one that enters lower. Each takes the slot
/// right of every vertical before it that comes within 8 of it, and of those it must stand right of.
fn slots(plan: &mut Plan, channels: usize) {
    let mut by_channel: Vec<Vec<usize>> = vec![Vec::new(); channels];
    for (index, vertical) in plan.verticals.iter().enumerate() {
        if let Some(list) = by_channel.get_mut(vertical.channel) {
            list.push(index);
        }
    }
    for (channel, list) in by_channel.iter().enumerate() {
        if list.is_empty() {
            continue;
        }
        let verticals: Vec<&Vertical> = list.iter().filter_map(|&index| plan.verticals.get(index)).collect();
        let count = verticals.len();
        let key = |vertical: &Vertical| -> (u8, i64) {
            if vertical.entry == vertical.y1 {
                (0, vertical.entry)
            } else if vertical.entry == vertical.y0 {
                (2, -vertical.entry)
            } else {
                (1, vertical.entry)
            }
        };
        let keys: Vec<((u8, i64), usize)> = verticals.iter().enumerate().map(|(local, vertical)| (key(vertical), local)).collect();
        // Who must stand left of whom: an entry and an exit of two wires within 8.
        let mut outs: Vec<(i64, usize)> =
            verticals.iter().enumerate().flat_map(|(local, vertical)| vertical.outs.iter().map(move |&out| (out, local))).collect();
        outs.sort_unstable();
        let mut right_of: Vec<Vec<usize>> = vec![Vec::new(); count];
        let mut before = vec![0usize; count];
        for (local, vertical) in verticals.iter().enumerate() {
            let low = outs.partition_point(|&(out, _)| out <= vertical.entry - GAP);
            let high = outs.partition_point(|&(out, _)| out < vertical.entry + GAP);
            for &(_, other) in outs.get(low..high).unwrap_or_default() {
                let same = verticals.get(other).is_some_and(|other| other.net == vertical.net);
                if !same && let Some(list) = right_of.get_mut(local) {
                    list.push(other);
                    if let Some(count) = before.get_mut(other) {
                        *count += 1;
                    }
                }
            }
        }
        // In the order of the keys, as far as the constraints allow; a cycle is broken at its smallest key.
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        let mut ready: std::collections::BinaryHeap<std::cmp::Reverse<((u8, i64), usize)>> =
            keys.iter().filter(|(_, local)| before.get(*local) == Some(&0)).map(|&entry| std::cmp::Reverse(entry)).collect();
        let mut done = vec![false; count];
        let mut next_sorted = 0;
        let mut order = Vec::with_capacity(count);
        while order.len() < count {
            let local = match ready.pop() {
                Some(std::cmp::Reverse((_, local))) => local,
                None => {
                    while sorted.get(next_sorted).is_some_and(|(_, local)| done.get(*local) == Some(&true)) {
                        next_sorted += 1;
                    }
                    let Some(&(_, local)) = sorted.get(next_sorted) else { break };
                    local
                }
            };
            if done.get(local) != Some(&false) {
                continue;
            }
            if let Some(flag) = done.get_mut(local) {
                *flag = true;
            }
            order.push(local);
            for &other in right_of.get(local).map_or(&[][..], Vec::as_slice) {
                if let Some(count) = before.get_mut(other) {
                    *count = count.saturating_sub(1);
                    if *count == 0 && done.get(other) == Some(&false) {
                        ready.push(std::cmp::Reverse((keys.get(other).map_or((0, 0), |entry| entry.0), other)));
                    }
                }
            }
        }
        // Half-open intervals `[y0, y1 + 8)`.
        let mut ys: Vec<i64> = verticals.iter().flat_map(|vertical| [vertical.y0, vertical.y1.saturating_add(GAP)]).collect();
        ys.sort_unstable();
        ys.dedup();
        let at = |y: i64| ys.partition_point(|&known| known < y);
        let mut taken = Tree::new(vec![-1i64; ys.len()], -1, i64::max);
        let mut floor = vec![0i64; count];
        let mut slots = vec![0u32; count];
        for &local in &order {
            let Some(vertical) = verticals.get(local) else { continue };
            let (start, end) = (at(vertical.y0), at(vertical.y1.saturating_add(GAP)).saturating_sub(1));
            let slot = (taken.query(start, end) + 1).max(floor.get(local).copied().unwrap_or(0));
            taken.update(start, end, slot);
            for &other in right_of.get(local).map_or(&[][..], Vec::as_slice) {
                if let Some(at_least) = floor.get_mut(other) {
                    *at_least = (*at_least).max(slot + 1);
                }
            }
            if let Some(entry) = slots.get_mut(local) {
                *entry = u32::try_from(slot).unwrap_or(0);
            }
        }
        for (&index, &slot) in list.iter().zip(&slots) {
            if let Some(vertical) = plan.verticals.get_mut(index) {
                vertical.slot = slot;
            }
        }
        if let Some(most) = plan.slots.get_mut(channel) {
            *most = slots.iter().map(|slot| slot + 1).max().unwrap_or(0);
        }
    }
}

/// The slots of vertical intervals taken in this order: each right of every one before it that comes within 8 of it,
/// so that two verticals of one slot keep 8 between them.
pub fn assign_slots(intervals: &[(i64, i64)]) -> Vec<u32> {
    // Half-open intervals `[y0, y1 + 8)`.
    let mut ys: Vec<i64> = intervals.iter().flat_map(|&(y0, y1)| [y0, y1.saturating_add(GAP)]).collect();
    ys.sort_unstable();
    ys.dedup();
    let at = |y: i64| ys.partition_point(|&known| known < y);
    let mut taken = Tree::new(vec![-1i64; ys.len()], -1, i64::max);
    intervals
        .iter()
        .map(|&(y0, y1)| {
            let (start, end) = (at(y0), at(y1.saturating_add(GAP)).saturating_sub(1));
            let slot = taken.query(start, end) + 1;
            taken.update(start, end, slot);
            u32::try_from(slot).unwrap_or(0)
        })
        .collect()
}

/// The geometry of a body, once its channels are placed: `channels[k]` is where channel `k` starts.
pub fn geometry(plan: &Plan, ends: &[Option<Ends>], channels: &[i64]) -> Vec<Routed> {
    let slot_x = |index: usize| -> Option<i64> {
        let vertical = plan.verticals.get(index)?;
        Some(channels.get(vertical.channel)? + GAP + GAP * i64::from(vertical.slot))
    };
    // Into a vertical at the height `from`: by its lead first when it jogs.
    let enter = |index: usize, from: i64, points: &mut Vec<(i64, i64)>| {
        let Some(x) = slot_x(index) else { return };
        if let Some((lead, vertical)) = plan.verticals.get(index).and_then(|vertical| Some((vertical.lead?, vertical)))
            && let Some(lead_x) = slot_x(lead)
        {
            points.push((lead_x, from));
            points.push((lead_x, vertical.entry));
            points.push((x, vertical.entry));
        } else {
            points.push((x, from));
        }
    };
    let mut routed = Vec::with_capacity(plan.nets.len() + plan.back.len());
    for net in &plan.nets {
        let mut out = Routed::default();
        let source = net.source;
        for &wire in &net.wires {
            let Some(found) = ends.get(wire).copied().flatten() else { continue };
            let mut points = vec![(source.x, source.y)];
            let first = net.first.and_then(|index| Some((index, slot_x(index)?)));
            if plan.straight.get(wire).copied().unwrap_or(false) {
                // Along the axis.
            } else if found.to_column == net.source_column + 1 {
                if let Some((index, x)) = first {
                    enter(index, source.y, &mut points);
                    points.push((x, found.to.approach));
                }
            } else if let Some(track) = net.track {
                if let Some((index, x)) = first {
                    enter(index, source.y, &mut points);
                    points.push((x, track));
                }
                if let Some((index, x)) = net.drops.get(&found.to_column).copied().flatten().and_then(|index| Some((index, slot_x(index)?))) {
                    enter(index, track, &mut points);
                    points.push((x, found.to.approach));
                }
            }
            tail(&found.to, &mut points);
            out.wires.push(wire);
            out.branches.push(simplify(points));
            out.right.push(found.to.side == Side::Left && found.to_column == plan.slots.len().saturating_sub(1));
        }
        out.junctions = junctions(&out.branches);
        routed.push(out);
    }
    // Through the waypoints of the file: across, then down to each; down to the height of the reader from the last.
    for (wire, waypoints) in &plan.through {
        let Some(found) = ends.get(*wire).copied().flatten() else { continue };
        let mut points = vec![(found.from.x, found.from.y)];
        for &(x, y) in waypoints {
            let height = points.last().map_or(y, |point| point.1);
            points.push((x, height));
            points.push((x, y));
        }
        if let Some(&(x, _)) = points.last() {
            points.push((x, found.to.approach));
        }
        tail(&found.to, &mut points);
        let right = found.to_column == plan.slots.len().saturating_sub(1);
        routed.push(Routed { wires: vec![*wire], branches: vec![simplify(points)], junctions: Vec::new(), right: vec![right] });
    }
    // Back to the left: out to the right, over both ends, down to the reader from its left.
    for &wire in &plan.back {
        let Some(found) = ends.get(wire).copied().flatten() else { continue };
        let (from, to) = (found.from, found.to);
        let over = from.y.min(to.approach) - 2 * CLEAR;
        let mut points = vec![(from.x, from.y), (from.x + CLEAR, from.y), (from.x + CLEAR, over), (to.x - CLEAR, over), (to.x - CLEAR, to.approach)];
        tail(&to, &mut points);
        let right = found.to_column == plan.slots.len().saturating_sub(1);
        routed.push(Routed { wires: vec![wire], branches: vec![simplify(points)], junctions: Vec::new(), right: vec![right] });
    }
    routed
}

/// The end of a polyline at its reader: from the left at its height, or from above for the top edge.
fn tail(to: &At, points: &mut Vec<(i64, i64)>) {
    match to.side {
        Side::Top => {
            points.push((to.x, to.approach));
            points.push((to.x, to.y));
        }
        Side::Left | Side::Right => {
            if let Some(&(x, _)) = points.last()
                && to.approach != to.y
            {
                points.push((x, to.y));
            }
            points.push((to.x, to.y));
        }
    }
}

/// Without repeated points, and without a point in the middle of a straight run.
pub fn simplify(points: Vec<(i64, i64)>) -> Vec<(i64, i64)> {
    let mut out: Vec<(i64, i64)> = Vec::with_capacity(points.len());
    for point in points {
        if out.last() == Some(&point) {
            continue;
        }
        if let [.., a, b] = out.as_slice()
            && ((a.0 == b.0 && b.0 == point.0) || (a.1 == b.1 && b.1 == point.1))
        {
            out.pop();
        }
        out.push(point);
    }
    out
}

/// The points where three directions of a net meet: where a branch leaves the trunk.
pub fn junctions(branches: &[Vec<(i64, i64)>]) -> Vec<(i64, i64)> {
    if branches.len() < 2 {
        return Vec::new();
    }
    // The segments of the net, merged by line.
    let mut vertical: BTreeMap<i64, Vec<(i64, i64)>> = BTreeMap::new();
    let mut horizontal: BTreeMap<i64, Vec<(i64, i64)>> = BTreeMap::new();
    for branch in branches {
        for pair in branch.windows(2) {
            let (Some(&a), Some(&b)) = (pair.first(), pair.get(1)) else { continue };
            if a.0 == b.0 {
                vertical.entry(a.0).or_default().push((a.1.min(b.1), a.1.max(b.1)));
            } else if a.1 == b.1 {
                horizontal.entry(a.1).or_default().push((a.0.min(b.0), a.0.max(b.0)));
            }
        }
    }
    for lines in [&mut vertical, &mut horizontal] {
        for list in lines.values_mut() {
            *list = merge(std::mem::take(list));
        }
    }
    // Whether a merged interval of the line runs on each side of `at`.
    let sides = |list: Option<&Vec<(i64, i64)>>, at: i64| -> (bool, bool) {
        let Some(list) = list else { return (false, false) };
        let index = list.partition_point(|&(start, _)| start <= at);
        match index.checked_sub(1).and_then(|index| list.get(index)) {
            Some(&(start, end)) if end >= at => (start < at, end > at),
            _ => (false, false),
        }
    };
    let mut found = std::collections::BTreeSet::new();
    for branch in branches {
        for &(x, y) in branch {
            let (up, down) = sides(vertical.get(&x), y);
            let (left, right) = sides(horizontal.get(&y), x);
            if u8::from(up) + u8::from(down) + u8::from(left) + u8::from(right) >= 3 {
                found.insert((x, y));
            }
        }
    }
    found.into_iter().collect()
}

/// Intervals merged where they touch or overlap, sorted.
fn merge(mut list: Vec<(i64, i64)>) -> Vec<(i64, i64)> {
    list.sort_unstable();
    let mut out: Vec<(i64, i64)> = Vec::with_capacity(list.len());
    for (start, end) in list {
        match out.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => out.push((start, end)),
        }
    }
    out
}

/// A segment tree that keeps, for a range, the best of its values under `pick` (min or max), and raises a whole range
/// to a value at least as good as what it holds: what the skylines of the tracks and the slots need.
struct Tree {
    size: usize,
    best: Vec<i64>,
    lazy: Vec<Option<i64>>,
    none: i64,
    pick: fn(i64, i64) -> i64,
}

impl Tree {
    fn new(values: Vec<i64>, none: i64, pick: fn(i64, i64) -> i64) -> Tree {
        let size = values.len().max(1).next_power_of_two();
        let mut best = vec![none; 2 * size];
        for (index, value) in values.into_iter().enumerate() {
            if let Some(slot) = best.get_mut(size + index) {
                *slot = value;
            }
        }
        for index in (1..size).rev() {
            let (a, b) = (best.get(2 * index).copied().unwrap_or(none), best.get(2 * index + 1).copied().unwrap_or(none));
            if let Some(slot) = best.get_mut(index) {
                *slot = pick(a, b);
            }
        }
        Tree { size, best, lazy: vec![None; 2 * size], none, pick }
    }

    /// The best value over `start..=end`; `none` for an empty range.
    fn query(&mut self, start: usize, end: usize) -> i64 {
        if start > end || start >= self.size {
            return self.none;
        }
        self.query_in(1, 0, self.size - 1, start, end.min(self.size - 1))
    }

    /// Every value of `start..=end` taken to the best of itself and `value`.
    fn update(&mut self, start: usize, end: usize, value: i64) {
        if start > end || start >= self.size {
            return;
        }
        self.update_in(1, 0, self.size - 1, start, end.min(self.size - 1), value);
    }

    fn apply(&mut self, node: usize, value: i64) {
        let pick = self.pick;
        if let Some(slot) = self.best.get_mut(node) {
            *slot = pick(*slot, value);
        }
        if let Some(slot) = self.lazy.get_mut(node) {
            *slot = Some(slot.map_or(value, |old| pick(old, value)));
        }
    }

    fn push(&mut self, node: usize) {
        if let Some(value) = self.lazy.get_mut(node).and_then(Option::take) {
            self.apply(2 * node, value);
            self.apply(2 * node + 1, value);
        }
    }

    fn query_in(&mut self, node: usize, low: usize, high: usize, start: usize, end: usize) -> i64 {
        if end < low || high < start {
            return self.none;
        }
        if start <= low && high <= end {
            return self.best.get(node).copied().unwrap_or(self.none);
        }
        self.push(node);
        let middle = low + (high - low) / 2;
        let a = self.query_in(2 * node, low, middle, start, end);
        let b = self.query_in(2 * node + 1, middle + 1, high, start, end);
        (self.pick)(a, b)
    }

    fn update_in(&mut self, node: usize, low: usize, high: usize, start: usize, end: usize, value: i64) {
        if end < low || high < start {
            return;
        }
        if start <= low && high <= end {
            self.apply(node, value);
            return;
        }
        self.push(node);
        let middle = low + (high - low) / 2;
        self.update_in(2 * node, low, middle, start, end, value);
        self.update_in(2 * node + 1, middle + 1, high, start, end, value);
        let (a, b) = (self.best.get(2 * node).copied().unwrap_or(self.none), self.best.get(2 * node + 1).copied().unwrap_or(self.none));
        if let Some(slot) = self.best.get_mut(node) {
            *slot = (self.pick)(a, b);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tree_keeps_the_best_of_a_range() {
        let mut tree = Tree::new(vec![5, 3, 8, 1], i64::MAX, i64::min);
        assert_eq!(tree.query(0, 2), 3);
        tree.update(1, 2, 2);
        assert_eq!(tree.query(2, 3), 1);
        assert_eq!(tree.query(2, 2), 2);
        assert_eq!(tree.query(0, 0), 5);
        assert_eq!(tree.query(3, 1), i64::MAX);
    }

    #[test]
    fn straight_runs_lose_their_middle_points() {
        assert_eq!(simplify(vec![(0, 0), (5, 0), (5, 0), (9, 0), (9, 4), (9, 8)]), vec![(0, 0), (9, 0), (9, 8)]);
    }

    #[test]
    fn a_branch_that_leaves_a_trunk_has_a_junction() {
        let branches = vec![vec![(0, 0), (10, 0), (10, 20), (30, 20)], vec![(0, 0), (10, 0), (10, 40), (30, 40)]];
        assert_eq!(junctions(&branches), vec![(10, 20)]);
        // A trunk that goes on straight through the split.
        let branches = vec![vec![(0, 0), (30, 0)], vec![(0, 0), (10, 0), (10, 40), (30, 40)]];
        assert_eq!(junctions(&branches), vec![(10, 0)]);
    }
}
