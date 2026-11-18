//! The size of a frame by hand (notation 2.6, spec 11): the right edge, the bottom edge or the corner of a structure, or
//! of the frame of the function, dragged; `frame.resize` at the drop. The header and the left edge of a structure move
//! it (`node.move`). Nothing is written while the edge moves: the canvas shows the box where it goes.

use egui::{CursorIcon, Pos2, Rect, Vec2};
use labcraft_diagram::Drawing;

use crate::moving::GRID;
use crate::select::{self, EDGE_GRAB, Index};

/// How wide the edge of a frame is lit under the pointer, in units: the band that takes a press (decision of the owner,
/// 2026-11-18).
pub(crate) const EDGE_BAND: f32 = EDGE_GRAB;

/// What a press on the edge of a frame takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Grip {
    /// The header or the left edge of a structure: it moves.
    Move,
    Right,
    Bottom,
    /// The bottom-right corner: both.
    Corner,
}

impl Grip {
    /// The pointer over it, or while it is dragged.
    pub fn cursor(self, dragging: bool) -> CursorIcon {
        match self {
            Grip::Move if dragging => CursorIcon::Grabbing,
            Grip::Move => CursorIcon::Grab,
            Grip::Right => CursorIcon::ResizeHorizontal,
            Grip::Bottom => CursorIcon::ResizeVertical,
            Grip::Corner => CursorIcon::ResizeNwSe,
        }
    }

    /// Whether it moves the right edge, and the bottom edge.
    fn sides(self) -> (bool, bool) {
        match self {
            Grip::Move => (false, false),
            Grip::Right => (true, false),
            Grip::Bottom => (false, true),
            Grip::Corner => (true, true),
        }
    }
}

/// A frame that a resize takes: that of the function (none), or a structure by its index in [`Drawing::placed`].
pub(crate) type Target = Option<usize>;

/// A width and a height, in whole units.
pub(crate) type Units = (i64, i64);

/// A frame: the origin that its size is measured from (spec 11) and its box as drawn, in units; its size and the least
/// size its content needs from that origin, in whole units rounded up, worked out in hundredths so that no rounding of
/// the floats adds a unit.
pub(crate) fn frame_of(drawing: &Drawing, target: Target) -> Option<(Pos2, Rect, Units, Units)> {
    let point = |(x, y): (i64, i64)| Pos2::new(x as f32, y as f32) / 100.0;
    let up = |(x, y): (i64, i64)| ((x + 99).div_euclid(100), (y + 99).div_euclid(100));
    match target {
        None => {
            let body = drawing.bodies.first()?;
            let (x, y, width, height) = body.frame;
            let size = (x + width - body.origin.0, y + height - body.origin.1);
            Some((point(body.origin), select::body_rect(drawing, 0)?, up(size), up(body.least.unwrap_or_default())))
        }
        Some(placed) => {
            let found = drawing.placed.get(placed)?;
            let size = (found.width, found.height - found.over);
            Some((point((found.x, found.y + found.over)), select::node_rect(drawing, placed)?, up(size), up(found.least)))
        }
    }
}

/// What the point `at`, in units, is on: the edges and the header of the innermost structure under it, or else the right
/// edge, the bottom edge or the corner of the frame of the function, within `reach` of it, inside or out. None on a node
/// that is no structure, inside a body, or elsewhere.
pub(crate) fn grip_at(drawing: &Drawing, index: &Index, at: Pos2, reach: f32, visible: &dyn Fn(Option<usize>) -> bool) -> Option<(Target, Grip)> {
    if let Some(placed) = select::node_at(drawing, index, at, visible) {
        if drawing.placed.get(placed).is_none_or(|found| found.holds.is_empty()) {
            return None;
        }
        let area = select::node_rect(drawing, placed)?;
        let grip = match (at.x >= area.max.x - EDGE_GRAB, at.y >= area.max.y - EDGE_GRAB) {
            (true, true) => Grip::Corner,
            (true, false) => Grip::Right,
            (false, true) => Grip::Bottom,
            (false, false) => Grip::Move,
        };
        return Some((Some(placed), grip));
    }
    let area = select::body_rect(drawing, 0)?;
    let right = (at.x - area.max.x).abs() <= reach && at.y >= area.min.y && at.y <= area.max.y + reach;
    let bottom = (at.y - area.max.y).abs() <= reach && at.x >= area.min.x && at.x <= area.max.x + reach;
    let grip = match (right, bottom) {
        (true, true) => Grip::Corner,
        (true, false) => Grip::Right,
        (false, true) => Grip::Bottom,
        (false, false) => return None,
    };
    Some((None, grip))
}

/// How far the corner of a frame of fixed size reaches into it, in units: where its tooltip shows.
const CORNER: f32 = 12.0;

/// The frames whose size `layout` fixes (spec 11), which the editor marks with their corner (notation 2.4): that of the
/// function, if fixed, then each structure of fixed size, with the box of each, in units.
pub(crate) fn fixed(drawing: &Drawing, shown: &dyn Fn(usize) -> bool) -> Vec<(Target, Rect)> {
    let mut found = Vec::new();
    if drawing.bodies.first().is_some_and(|body| body.size.is_some())
        && let Some(area) = select::body_rect(drawing, 0)
    {
        found.push((None, area));
    }
    for (placed, node) in drawing.placed.iter().enumerate() {
        if node.size.is_some()
            && shown(placed)
            && let Some(area) = select::node_rect(drawing, placed)
        {
            found.push((Some(placed), area));
        }
    }
    found
}

/// The frame of fixed size whose corner is under `at`, in units, among `fixed`: the innermost.
pub(crate) fn corner_under(fixed: &[(Target, Rect)], at: Pos2) -> Option<Target> {
    fixed
        .iter()
        .filter(|(_, area)| Rect::from_min_max(area.max - Vec2::splat(CORNER), area.max).contains(at))
        .min_by(|(_, a), (_, b)| a.area().total_cmp(&b.area()))
        .map(|(target, _)| *target)
}

/// What the corner of a frame of fixed size says on hover (notation 2.4).
pub(crate) fn tooltip(drawing: &Drawing, target: Target) -> String {
    let (entry, size) = match target {
        None => ("frame size".to_owned(), drawing.bodies.first().and_then(|body| body.size)),
        Some(placed) => {
            let node = drawing.placed.get(placed);
            (format!("frame {} size", node.map(|node| node.name.as_str()).unwrap_or_default()), node.and_then(|node| node.size))
        }
    };
    let (width, height) = size.unwrap_or_default();
    format!("Size fixed (`{entry} ({}, {})` in `layout`). Right-click › Remove the fixed size", width / 100, height / 100)
}

/// How the resize goes on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum State {
    Dragging,
    /// Asked of the host: the box stays where it went until the new drawing comes, or the host says it failed.
    Asked,
}

/// A resize under way.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Resizing {
    pub target: Target,
    pub grip: Grip,
    pub state: State,
    /// Where the press was, in units.
    from: Pos2,
    origin: Pos2,
    /// The top-left corner of its box, which stays.
    corner: Pos2,
    /// Its size when the drag started, and the least, from the origin, in units.
    start: Units,
    least: Units,
    /// The size it goes to, in units.
    pub size: (i64, i64),
}

impl Resizing {
    pub fn new(drawing: &Drawing, target: Target, grip: Grip, from: Pos2) -> Option<Resizing> {
        if grip == Grip::Move {
            return None;
        }
        let (origin, area, start, least) = frame_of(drawing, target)?;
        Some(Resizing { target, grip, state: State::Dragging, from, origin, corner: area.min, start, least, size: start })
    }

    /// The pointer at `at`, in units: the edges it holds follow by whole steps of the grid of 8, as a move goes, never
    /// under what the frame holds (notation 2.6).
    pub fn set_at(&mut self, at: Pos2) {
        let (right, bottom) = self.grip.sides();
        let delta = at - self.from;
        let follow = |start: i64, by: f32, least: i64, moves: bool| -> i64 {
            if !moves {
                return start;
            }
            let steps = (by / GRID).round() as i64;
            (start + steps * GRID as i64).max(least)
        };
        self.size = (follow(self.start.0, delta.x, self.least.0, right), follow(self.start.1, delta.y, self.least.1, bottom));
    }

    /// Whether the size differs from the one it started from.
    pub fn changed(&self) -> bool {
        self.size != self.start
    }

    /// The box where it goes, in units.
    pub fn area(&self) -> Rect {
        Rect::from_min_max(self.corner, self.origin + Vec2::new(self.size.0 as f32, self.size.1 as f32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resizing(grip: Grip) -> Resizing {
        Resizing {
            target: None,
            grip,
            state: State::Dragging,
            from: Pos2::new(100.0, 50.0),
            origin: Pos2::new(0.0, 0.0),
            corner: Pos2::new(0.0, -10.0),
            start: (100, 50),
            // 60.5 units of content, rounded up.
            least: (61, 40),
            size: (100, 50),
        }
    }

    #[test]
    fn an_edge_follows_on_the_grid_and_stops_at_what_the_frame_holds() {
        let mut right = resizing(Grip::Right);
        right.set_at(Pos2::new(123.0, 90.0));
        // 23 to the right is 24 on the grid of 8; the bottom edge is not held, and stays.
        assert_eq!(right.size, (124, 50));
        right.set_at(Pos2::new(10.0, 50.0));
        // Never under the least size, rounded up to a whole unit.
        assert_eq!(right.size, (61, 50));
        let mut corner = resizing(Grip::Corner);
        corner.set_at(Pos2::new(140.0, 77.0));
        assert_eq!(corner.size, (140, 74));
        assert_eq!(corner.area(), Rect::from_min_max(Pos2::new(0.0, -10.0), Pos2::new(140.0, 74.0)));
        corner.set_at(Pos2::new(100.0, 50.0));
        assert!(!corner.changed());
    }
}
