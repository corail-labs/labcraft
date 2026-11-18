//! Where copy and paste fall on the canvas (notation 2.6): the corner of the nodes selected, from which Duplicate counts,
//! and the place of a paste, under the pointer or in the middle of what the view shows of the frame.

use egui::{Pos2, Rect};

use crate::Canvas;
use crate::quick::frame_point;
use crate::rerouting::unit;
use crate::select::Selection;

impl Canvas {
    /// The top-left corner of the nodes selected, in units of their frame (spec 11): the corner that `fragment.copy`
    /// counts from, so that Duplicate puts the copy 16 to the right and 16 below. None with no node selected that the
    /// drawing places.
    pub fn selection_corner(&self, selection: &Selection) -> Option<(i64, i64)> {
        let drawing = self.drawing.as_ref()?;
        let found = self.index.find(selection);
        let places: Vec<(i64, i64)> = found.nodes.iter().filter_map(|placed| drawing.in_frame(*placed)).collect();
        let x = places.iter().map(|(x, _)| *x).min()?;
        let y = places.iter().map(|(_, y)| *y).min()?;
        Some((unit(x), unit(y)))
    }

    /// Whether every node selected is in the body of the function, outside any structure: a paste goes there until
    /// step 5 brings the frames of structures (`inside`, `arm`).
    pub fn selection_in_function_body(&self, selection: &Selection) -> bool {
        let Some(drawing) = self.drawing.as_ref() else { return false };
        let found = self.index.find(selection);
        found
            .nodes
            .iter()
            .all(|placed| drawing.placed.get(*placed).and_then(|node| drawing.bodies.get(node.body)).is_some_and(|body| body.owner.is_none()))
    }

    /// Where a paste puts the top-left corner of its fragment, in units of the frame of the function, on the grid:
    /// under `pointer` when it is on the canvas, else in the middle of the part of the frame that the view shows, or of
    /// the view when it shows none of it (decision of the owner, 2026-11-17). None without a drawing.
    pub fn paste_at(&self, pointer: Option<Pos2>) -> Option<(i64, i64)> {
        let drawing = self.drawing.as_ref()?;
        let rect = self.rect;
        let unit_of = |at: Pos2| (self.viewport.origin + (at - rect.min) / self.viewport.zoom).to_pos2();
        if let Some(under) = pointer.filter(|at| rect.contains(*at)) {
            return Some(frame_point(drawing, unit_of(under)));
        }
        let shown = Rect::from_min_max(unit_of(rect.min), unit_of(rect.max));
        let middle = drawing
            .bodies
            .first()
            .map(|body| {
                let (x, y, width, height) = body.frame;
                let hundredth = |value: i64| value as f32 / 100.0;
                Rect::from_min_size(Pos2::new(hundredth(x), hundredth(y)), egui::vec2(hundredth(width), hundredth(height)))
            })
            .map(|frame| frame.intersect(shown))
            .filter(|visible| visible.is_positive())
            .map_or(shown.center(), |visible| visible.center());
        Some(frame_point(drawing, middle))
    }
}
