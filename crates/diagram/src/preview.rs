//! The preview of a drag (architecture 12.4): while nodes are dragged, nothing is written; the canvas moves their shapes,
//! and the wires that touch them are routed again here, from the drawing as it is, so that a frame never waits for the
//! placement. The others do not move. At the drop, the command writes the file, and the new drawing replaces the preview.

use std::collections::{BTreeSet, HashSet};

use crate::shape::{Drawing, Shape};

/// What a drag moves, and the wires it routes again.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Preview {
    /// The nodes that move, by their index in [`Drawing::placed`]: those dragged, and every node inside a structure
    /// dragged.
    pub carried: BTreeSet<usize>,
    /// The bodies inside a structure dragged, by their index in the graph: their wires move with it.
    pub bodies: BTreeSet<usize>,
    /// The wires that reach a node dragged from outside, by their group, with the shapes that stand for them meanwhile.
    pub wires: Vec<(usize, Vec<Shape>)>,
}

/// How far around the box of a node the end of a wire is still at the node, in hundredths: its terminals stand on the
/// edge of the box.
const AT_THE_NODE: i64 = 1_600;
/// A wire leaves a terminal and enters one straight for this far, in hundredths, before it turns back.
const STRAIGHT: i64 = 1_200;

/// The preview of the nodes `moved` (indices in [`Drawing::placed`]) moved by `by`, in hundredths. A wire between two
/// nodes that both move moves whole; a wire to a node that moves goes from its start to the end moved, square, through
/// the middle (notation 2.2 holds again at the drop).
pub fn preview(drawing: &Drawing, moved: &[usize], by: (i64, i64)) -> Preview {
    let mut preview = Preview::default();
    let mut pending: Vec<usize> = moved.iter().copied().filter(|&at| at < drawing.placed.len()).collect();
    while let Some(at) = pending.pop() {
        if !preview.carried.insert(at) {
            continue;
        }
        let Some(placed) = drawing.placed.get(at) else { continue };
        for &body in &placed.holds {
            if preview.bodies.insert(body) {
                pending.extend(drawing.placed.iter().enumerate().filter(|(_, inner)| inner.body == body).map(|(inner, _)| inner));
            }
        }
    }
    let keys: HashSet<&str> = preview.carried.iter().filter_map(|&at| drawing.placed.get(at)).map(|placed| placed.key.as_str()).collect();
    let boxes: Vec<(i64, i64, i64, i64)> = moved
        .iter()
        .filter_map(|&at| drawing.placed.get(at))
        .map(|placed| (placed.x - AT_THE_NODE, placed.y - AT_THE_NODE, placed.x + placed.width + AT_THE_NODE, placed.y + placed.height + AT_THE_NODE))
        .collect();
    let at_a_moved_node = |(x, y): (i64, i64)| boxes.iter().any(|&(left, top, right, bottom)| left <= x && x <= right && top <= y && y <= bottom);
    for (index, group) in drawing.groups.iter().enumerate() {
        let Some(wire) = &group.wire else { continue };
        if preview.bodies.contains(&wire.body) {
            continue;
        }
        let from_moves = wire.from_node.as_deref().is_some_and(|key| keys.contains(key));
        if !from_moves && !wire.to_nodes.iter().any(|key| keys.contains(key.as_str())) {
            continue;
        }
        let Some(style) = group.shapes.iter().find_map(|shape| match shape {
            Shape::Wire { style, .. } => Some(*style),
            _ => None,
        }) else {
            continue;
        };
        let shapes = wire
            .readers
            .iter()
            .filter_map(|(_, points)| {
                let (&start, &end) = (points.first()?, points.last()?);
                let shift = |(x, y): (i64, i64), moves: bool| if moves { (x + by.0, y + by.1) } else { (x, y) };
                let end_moves = at_a_moved_node(end);
                let points = if from_moves && end_moves {
                    points.iter().map(|&point| shift(point, true)).collect()
                } else if !from_moves && !end_moves {
                    // A reader that the drag does not reach keeps its route.
                    points.clone()
                } else {
                    square(shift(start, from_moves), shift(end, end_moves))
                };
                Some(Shape::Wire { points, style })
            })
            .collect();
        preview.wires.push((index, shapes));
    }
    preview
}

/// A square route from `from`, which leaves to the right, to `to`, which it enters from the left.
fn square(from: (i64, i64), to: (i64, i64)) -> Vec<(i64, i64)> {
    if from.1 == to.1 && to.0 >= from.0 {
        return vec![from, to];
    }
    if to.0 - from.0 >= 2 * STRAIGHT {
        let middle = from.0 + (to.0 - from.0) / 2;
        return vec![from, (middle, from.1), (middle, to.1), to];
    }
    // Back to the left: out, down to half way, across, down, in.
    let (out, into) = (from.0 + STRAIGHT, to.0 - STRAIGHT);
    let half = from.1 + (to.1 - from.1) / 2;
    vec![from, (out, from.1), (out, half), (into, half), (into, to.1), to]
}

#[cfg(test)]
mod tests {
    use super::square;

    #[test]
    fn a_square_route_turns_through_the_middle_or_goes_round() {
        assert_eq!(square((0, 0), (1_000, 0)), vec![(0, 0), (1_000, 0)]);
        assert_eq!(square((0, 0), (10_000, 4_000)), vec![(0, 0), (5_000, 0), (5_000, 4_000), (10_000, 4_000)]);
        assert_eq!(square((10_000, 0), (0, 4_000)), vec![(10_000, 0), (11_200, 0), (11_200, 2_000), (-1_200, 2_000), (-1_200, 4_000), (0, 4_000)]);
    }
}
