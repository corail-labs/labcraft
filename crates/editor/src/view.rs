//! The viewport, the glide and the grid that finds what the viewport shows.

use std::collections::HashMap;

use egui::{Pos2, Rect, Response, Vec2};
use labcraft_diagram::Drawing;

/// The zooms the canvas allows, 1 for 100 %.
pub const MIN_ZOOM: f32 = 0.05;
pub const MAX_ZOOM: f32 = 4.0;
/// The glide of a node to its new place (notation 2.1, about 150 ms *(proposed)*).
pub const GLIDE_SECONDS: f64 = 0.15;
/// A zoom ends when it has not changed for this long: its texts are then laid out at the zoom itself.
const ZOOM_SETTLES: f64 = 0.12;
/// Within a zoom, the shapes are tessellated at steps of a sixteenth of an octave, and only transformed in between.
const STEPS_PER_OCTAVE: f32 = 16.0;
/// The side of a cell of the grid, in units.
const CELL: f32 = 256.0;
/// A wire or a frame that covers more cells than this is kept apart, and looked at whatever the viewport.
const MANY_CELLS: usize = 64;

#[derive(Clone, Copy, Debug)]
pub struct Viewport {
    pub zoom: f32,
    /// The point of the drawing at the top-left of the canvas, in units.
    pub origin: Vec2,
    /// Whether the viewport was set, by a fit or by the host; until then the next frame fits the drawing.
    pub fitted: bool,
    /// When the zoom last changed.
    pub changed: f64,
}

impl Default for Viewport {
    fn default() -> Self {
        Viewport { zoom: 1.0, origin: Vec2::ZERO, fitted: false, changed: f64::NEG_INFINITY }
    }
}

impl Viewport {
    /// Pans and zooms as the pointer says; fits the drawing the first time.
    pub fn read_input(&mut self, ctx: &egui::Context, response: &Response, rect: Rect, drawing: &Drawing) {
        let now = ctx.input(|input| input.time);
        if !self.fitted && rect.width() > 0.0 && rect.height() > 0.0 {
            let size = Vec2::new(drawing.width as f32, drawing.height as f32) / 100.0;
            let zoom = (rect.width() / size.x.max(1.0)).min(rect.height() / size.y.max(1.0)).clamp(MIN_ZOOM, 1.0);
            self.zoom = zoom;
            // Centred, or from its corner when it is larger than the canvas.
            self.origin = ((size - rect.size() / zoom) / 2.0).min(Vec2::ZERO);
            self.fitted = true;
            self.changed = now;
        }
        if response.dragged() {
            self.origin -= response.drag_delta() / self.zoom;
        }
        if response.hovered() {
            let (scroll, factor, pointer) = ctx.input(|input| (input.smooth_scroll_delta, input.zoom_delta(), input.pointer.hover_pos()));
            if factor != 1.0 {
                let anchor = pointer.unwrap_or(rect.center());
                let before = self.origin + (anchor - rect.min) / self.zoom;
                self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
                self.origin = before - (anchor - rect.min) / self.zoom;
                self.changed = now;
            } else if scroll != Vec2::ZERO {
                self.origin -= scroll / self.zoom;
            }
        }
    }

    /// How long until the zoom settles, when it has just changed.
    pub fn settles_in(&self, now: f64) -> Option<f64> {
        let left = ZOOM_SETTLES - (now - self.changed);
        (left > 0.0).then_some(left)
    }

    /// The zoom that the shapes are tessellated at: the zoom itself once it has settled, a step of it while it moves.
    pub fn step(&self, now: f64) -> f32 {
        if now - self.changed >= ZOOM_SETTLES {
            return self.zoom;
        }
        let octaves = (self.zoom.log2() * STEPS_PER_OCTAVE).round() / STEPS_PER_OCTAVE;
        octaves.exp2()
    }
}

/// Each node from its old place to its new one (notation 2.1).
pub struct Glide {
    /// By node of the new drawing, where it was, from where it is, in units.
    from: Vec<Vec2>,
    start: Option<f64>,
    progress: f32,
}

impl Glide {
    /// Nothing when no node moved.
    pub fn between(old: &Drawing, new: &Drawing) -> Option<Glide> {
        let before: HashMap<&str, (i64, i64)> = old.placed.iter().map(|placed| (placed.key.as_str(), (placed.x, placed.y))).collect();
        let from: Vec<Vec2> = new
            .placed
            .iter()
            .map(|placed| match before.get(placed.key.as_str()) {
                Some(&(x, y)) => Vec2::new((x - placed.x) as f32, (y - placed.y) as f32) / 100.0,
                None => Vec2::ZERO,
            })
            .collect();
        from.iter().any(|offset| *offset != Vec2::ZERO).then_some(Glide { from, start: None, progress: 0.0 })
    }

    /// Moves on to `now`; whether it is over.
    pub fn advance(&mut self, now: f64) -> bool {
        let start = *self.start.get_or_insert(now);
        let t = ((now - start) / GLIDE_SECONDS).clamp(0.0, 1.0) as f32;
        self.progress = t;
        t >= 1.0
    }

    /// Where each node stood, from its new place, when the glide began.
    pub fn from(&self) -> &[Vec2] {
        &self.from
    }

    /// Where each node stands from its new place: all the way back at the start, nothing at the end, eased.
    pub fn offsets(&self) -> Vec<Vec2> {
        let left = 1.0 - self.progress;
        let eased = left * left * left;
        self.from.iter().map(|offset| *offset * eased).collect()
    }
}

/// The groups of a drawing by the cells of the grid that their shapes cover, so that a frame paints only what the
/// viewport shows, in the order of drawing.
#[derive(Default)]
pub struct Grid {
    cells: HashMap<(i32, i32), Vec<usize>>,
    /// Groups that cover many cells, looked at every frame.
    wide: Vec<(usize, Rect)>,
    /// The box of each node, by its index in `Drawing::placed`, and the group that draws it, for the pointer.
    nodes: Vec<Rect>,
    within: Vec<Option<usize>>,
    /// The groups of each node: the node, then the arms of a structure.
    groups: Vec<Vec<usize>>,
}

impl Grid {
    pub fn of(drawing: &Drawing) -> Grid {
        let mut grid = Grid::default();
        for (index, group) in drawing.groups.iter().enumerate() {
            let mut area = Rect::NOTHING;
            for shape in &group.shapes {
                area = area.union(crate::extent(shape));
            }
            if let Some(placed) = group.placed.and_then(|placed| drawing.placed.get(placed)) {
                area = area.union(placed_rect(placed));
            }
            if !area.is_positive() {
                continue;
            }
            let (low, high) = (cell_of(area.min), cell_of(area.max));
            let count = (i64::from(high.0) - i64::from(low.0) + 1).saturating_mul(i64::from(high.1) - i64::from(low.1) + 1);
            if usize::try_from(count).map_or(true, |count| count > MANY_CELLS) {
                grid.wide.push((index, area));
                continue;
            }
            for x in low.0..=high.0 {
                for y in low.1..=high.1 {
                    grid.cells.entry((x, y)).or_default().push(index);
                }
            }
        }
        grid.nodes = drawing.placed.iter().map(placed_rect).collect();
        grid.within = vec![None; drawing.placed.len()];
        grid.groups = vec![Vec::new(); drawing.placed.len()];
        for (index, group) in drawing.groups.iter().enumerate().filter(|(_, group)| group.wire.is_none()) {
            let Some(placed) = group.placed else { continue };
            if group.arm.is_none()
                && let Some(slot) = grid.within.get_mut(placed)
            {
                *slot = group.within;
            }
            if let Some(slot) = grid.groups.get_mut(placed) {
                slot.push(index);
            }
        }
        grid
    }

    /// The groups that meet `seen`, in the order of drawing, each once.
    pub fn seen(&self, seen: Rect) -> Vec<usize> {
        let (low, high) = (cell_of(seen.min), cell_of(seen.max));
        let mut found: Vec<usize> = Vec::new();
        // A canvas zoomed out far sees many cells: then every group is looked at.
        let count = (i64::from(high.0) - i64::from(low.0) + 1).saturating_mul(i64::from(high.1) - i64::from(low.1) + 1);
        if count > i64::try_from(self.cells.len()).unwrap_or(i64::MAX) {
            found.extend(self.cells.values().flatten().copied());
        } else {
            for x in low.0..=high.0 {
                for y in low.1..=high.1 {
                    if let Some(list) = self.cells.get(&(x, y)) {
                        found.extend(list.iter().copied());
                    }
                }
            }
        }
        found.extend(self.wide.iter().filter(|(_, area)| area.intersects(seen)).map(|(index, _)| *index));
        found.sort_unstable();
        found.dedup();
        found
    }

    /// The group that draws a node.
    pub fn group_of(&self, placed: usize) -> Option<usize> {
        self.groups.get(placed).and_then(|groups| groups.first()).copied()
    }

    /// The groups of the nodes that a glide brings across `seen`: those whose way, from where they were to where they
    /// are, meets it.
    pub fn gliding(&self, from: &[Vec2], seen: Rect) -> Vec<usize> {
        let mut found = Vec::new();
        for ((area, offset), groups) in self.nodes.iter().zip(from).zip(&self.groups) {
            if *offset != Vec2::ZERO && area.union(area.translate(*offset)).intersects(seen) {
                found.extend(groups.iter().copied());
            }
        }
        found
    }

    /// The innermost node shown under `at`: the last drawn that holds it.
    pub fn node_at(&self, _drawing: &Drawing, at: Pos2, visible: &dyn Fn(Option<usize>) -> bool) -> Option<usize> {
        self.nodes
            .iter()
            .enumerate()
            .filter(|(index, area)| area.contains(at) && visible(self.within.get(*index).copied().flatten()))
            .map(|(index, _)| index)
            .last()
    }
}

fn placed_rect(placed: &labcraft_diagram::Placed) -> Rect {
    Rect::from_min_size(Pos2::new(placed.x as f32 / 100.0, placed.y as f32 / 100.0), Vec2::new(placed.width as f32, placed.height as f32) / 100.0)
}

fn cell_of(at: Pos2) -> (i32, i32) {
    let cell = |value: f32| (value / CELL).floor().clamp(i32::MIN as f32, i32::MAX as f32) as i32;
    (cell(at.x), cell(at.y))
}
