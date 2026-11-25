//! A move under way (notation 2.4 and 2.6): nodes dragged, or moved by Ctrl and the arrows, shown where they go with the
//! wires that reach them routed again by the preview of `diagram`, until the host writes the move and the new drawing
//! comes (architecture 12.4). Nothing is written meanwhile.

use std::collections::HashMap;
use std::sync::Arc;

use egui::{Pos2, Vec2};
use labcraft_diagram::{Drawing, Preview, preview};

use crate::paint::Painted;
use crate::rerouting::{snap, unit};

/// One step of the grid of 8: a drag moves by whole steps, and so does Ctrl with an arrow (notation 2.6).
pub(crate) const GRID: f32 = 8.0;
/// A run of presses of Ctrl and an arrow ends a second after the last one (notation 2.6).
pub(crate) const RUN_SECONDS: f64 = 1.0;
/// A move asked for keeps its nodes where they went this long at most, waiting for the new drawing (300 ms
/// *(proposed)*, architecture 12.4).
pub(crate) const ASKED_SECONDS: f64 = 3.0;

/// How the move goes on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum State {
    /// The primary button holds the nodes, from where it pressed, in units.
    Dragging { from: Pos2 },
    /// A run of presses of Ctrl and an arrow, the last one at this time.
    Keys { last: f64 },
    /// Asked of the host: the nodes stay where they went until the new drawing comes, or the host says it failed.
    Asked,
    /// Held by the host, which moves them itself ([`crate::Canvas::hold_move`]): the timing of `LABCRAFT_FRAMES`.
    Held,
}

pub(crate) struct Moving {
    /// The nodes moved, by their index in [`Drawing::placed`]; the first leads the move, the one a drag holds: it lands on
    /// the grid of 8 of its frame, where `node.move` puts it (notation 2.4).
    pub nodes: Vec<usize>,
    /// How far, in units, on the grid.
    pub by: (i64, i64),
    pub state: State,
    pub preview: Preview,
    /// What is painted of each wire of the preview, for `by`.
    pub painted: HashMap<usize, Arc<Painted>>,
    /// The place of each wire of the preview in it, by its group.
    pub rerouted: HashMap<usize, usize>,
    /// When the canvas first saw the move asked for, by the time of egui.
    pub asked_at: Option<f64>,
}

impl Moving {
    pub fn new(drawing: &Drawing, nodes: Vec<usize>, state: State) -> Moving {
        let preview = preview(drawing, &nodes, (0, 0));
        let rerouted = preview.wires.iter().enumerate().map(|(at, (group, _))| (*group, at)).collect();
        Moving { nodes, by: (0, 0), state, preview, painted: HashMap::new(), rerouted, asked_at: None }
    }

    /// Moves by about `by`, whole steps of the grid in units: as far as puts the first node on the nearest point of the
    /// grid of 8 of its frame, counted from its origin, the others going as far, as `node.move` moves them (notation 2.4),
    /// so that the preview shows them where the drop writes them. A move by nothing stays one. The preview follows.
    pub fn set_by(&mut self, drawing: &Drawing, by: (i64, i64)) {
        let by = match self.nodes.first().and_then(|&lead| drawing.in_frame(lead)) {
            Some((x, y)) if by != (0, 0) => {
                let (x, y) = (unit(x), unit(y));
                let land = |from: i64, by: i64| snap(from.saturating_add(by).saturating_mul(100), 0) / 100 - from;
                (land(x, by.0), land(y, by.1))
            }
            _ => by,
        };
        if by == self.by {
            return;
        }
        self.by = by;
        self.preview = preview(drawing, &self.nodes, (by.0 * 100, by.1 * 100));
        self.rerouted = self.preview.wires.iter().enumerate().map(|(at, (group, _))| (*group, at)).collect();
        self.painted.clear();
    }

    /// Whether it moves these nodes, in any order: a drag puts the node it holds first.
    pub fn moves(&self, nodes: &[usize]) -> bool {
        self.nodes.len() == nodes.len() && nodes.iter().all(|node| self.nodes.contains(node))
    }

    /// The same move over a new drawing, its nodes found again by their keys; none when one of them is gone.
    pub fn again(&self, old: &Drawing, new: &Drawing) -> Option<Moving> {
        let nodes: Vec<usize> = self
            .nodes
            .iter()
            .map(|&at| {
                let key = &old.placed.get(at)?.key;
                new.placed.iter().position(|placed| placed.key == *key)
            })
            .collect::<Option<Vec<usize>>>()?;
        let mut moving = Moving::new(new, nodes, self.state);
        moving.set_by(new, self.by);
        Some(moving)
    }

    /// Where each node stands from its place, by its index in [`Drawing::placed`].
    pub fn offsets(&self, count: usize) -> Vec<Vec2> {
        let by = Vec2::new(self.by.0 as f32, self.by.1 as f32);
        (0..count).map(|at| if self.preview.carried.contains(&at) { by } else { Vec2::ZERO }).collect()
    }

    /// The offset of the wires of a body: those inside a structure moved go with it.
    pub fn body_offset(&self, body: usize) -> Vec2 {
        if self.preview.bodies.contains(&body) { Vec2::new(self.by.0 as f32, self.by.1 as f32) } else { Vec2::ZERO }
    }
}

/// A distance dragged, in units, to whole steps of the grid.
pub(crate) fn snapped(delta: Vec2) -> (i64, i64) {
    let step = |value: f32| ((value / GRID).round() * GRID) as i64;
    (step(delta.x), step(delta.y))
}

#[cfg(test)]
mod tests {
    use super::snapped;
    use egui::Vec2;

    #[test]
    fn a_drag_moves_by_whole_steps_of_the_grid() {
        assert_eq!(snapped(Vec2::new(3.9, -4.1)), (0, -8));
        assert_eq!(snapped(Vec2::new(12.0, 100.0)), (16, 104));
    }
}
