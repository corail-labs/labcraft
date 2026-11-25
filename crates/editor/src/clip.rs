//! Where copy and paste fall on the canvas (notation 2.6): the corner of the nodes selected, from which Duplicate counts,
//! and the place of a paste, under the pointer or in the middle of what the view shows of the frame.

use egui::{Pos2, Rect};

use crate::quick::frame_point;
use crate::rerouting::unit;
use crate::select::Selection;
use crate::{Canvas, InFrame, Shown};

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

    /// The frame of the nodes selected, as `inside` and `arm` name it (commands.md 6.7): where Duplicate pastes. None
    /// when they are in several frames, or in a structure that a command cannot name.
    pub fn selection_frame(&self, selection: &Selection) -> Option<InFrame> {
        let drawing = self.drawing.as_ref()?;
        let found = self.index.find(selection);
        let mut bodies = found.nodes.iter().filter_map(|placed| drawing.placed.get(*placed).map(|node| node.body));
        let body = bodies.next()?;
        if bodies.any(|other| other != body) {
            return None;
        }
        crate::frames::in_frame_of(drawing, body)
    }

    /// Where a paste puts the top-left corner of its fragment, on the grid: under `pointer` when it is on the canvas, in
    /// the frame there (an arm shown, a loop body, the body of the function), else in the middle of the part of the frame
    /// of the function that the view shows, or of the view when it shows none of it (decision of the owner, 2026-11-17).
    /// None without a drawing.
    pub fn paste_at(&self, shown: &Shown, pointer: Option<Pos2>) -> Option<(InFrame, (i64, i64))> {
        let drawing = self.drawing.as_ref()?;
        let rect = self.rect;
        let unit_of = |at: Pos2| (self.viewport.origin + (at - rect.min) / self.viewport.zoom).to_pos2();
        if let Some(under) = pointer.filter(|at| rect.contains(*at)) {
            return self.frame_at(shown, under);
        }
        let view = Rect::from_min_max(unit_of(rect.min), unit_of(rect.max));
        let middle = drawing
            .bodies
            .first()
            .map(|body| {
                let (x, y, width, height) = body.frame;
                let hundredth = |value: i64| value as f32 / 100.0;
                Rect::from_min_size(Pos2::new(hundredth(x), hundredth(y)), egui::vec2(hundredth(width), hundredth(height)))
            })
            .map(|frame| frame.intersect(view))
            .filter(|visible| visible.is_positive())
            .map_or(view.center(), |visible| visible.center());
        Some((InFrame::default(), frame_point(drawing, middle)))
    }
}
