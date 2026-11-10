//! L7: the canvas that a Craft app embeds to show the drawings of `diagram` (`docs/architecture.md`, 11.4; D-27),
//! read-only in M0v. It depends on `diagram` and egui only: no engine, no file, no window.
//!
//! It paints, and decides nothing of the notation: every position, text and colour comes from the [`Drawing`]. What a
//! click asks for comes back in [`CanvasResponse`], and the host makes it a command (`view.set_state`).
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable,
    clippy::indexing_slicing,
    clippy::string_slice
)]
#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::Arc;

use egui::{Color32, Pos2, Rect, Sense, Ui, Vec2};
use labcraft_diagram::style;
use labcraft_diagram::{COARSE_NODE, Drawing, Shape};

mod fonts;
pub(crate) mod paint;
pub(crate) mod view;

pub use fonts::{FAMILY, family, has_family, install_fonts};
pub use view::{GLIDE_SECONDS, MAX_ZOOM, MIN_ZOOM};

use paint::{Painted, Painter};
use view::{Glide, Grid, Viewport};

/// The colours of the theme tokens (notation 8): those of the style in force in `diagram`, light or dark.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Theme {
    #[default]
    Light,
    Dark,
}

impl Theme {
    /// The colour of a token; an unknown token is magenta, which a test of every drawing keeps from happening.
    pub fn color(self, token: &str) -> Color32 {
        let Some(rgb) = style::color(token, self == Theme::Dark) else { return Color32::from_rgb(255, 0, 255) };
        let [_, red, green, blue] = rgb.to_be_bytes();
        Color32::from_rgb(red, green, blue)
    }
}

/// What the host shows of a drawing without drawing it again (architecture 11.4): the arm shown of each structure in
/// tabs, by the key of the structure; the first when it is not said, or when the structure has fewer arms now.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shown {
    pub arms: HashMap<String, usize>,
}

impl Shown {
    pub fn arm_of(&self, structure: &str) -> usize {
        self.arms.get(structure).copied().unwrap_or(0)
    }
}

/// What a frame of the canvas gives back to the host.
pub struct CanvasResponse {
    pub response: egui::Response,
    /// A tab clicked: the key of its structure and the rank of its arm, for `view.set_state`.
    pub tab: Option<(String, usize)>,
    /// The node under the pointer, by its index in [`Drawing::placed`].
    pub hovered: Option<usize>,
}

/// The canvas: the drawing shown, its viewport, its caches, the glide.
pub struct Canvas {
    drawing: Option<Arc<Drawing>>,
    /// Changes with each drawing, so that the caches know theirs.
    generation: u64,
    viewport: Viewport,
    grid: Grid,
    glide: Option<Glide>,
    reduced_motion: bool,
    painter: Painter,
}

impl Default for Canvas {
    fn default() -> Self {
        Canvas::new()
    }
}

impl Canvas {
    pub fn new() -> Canvas {
        Canvas {
            drawing: None,
            generation: 0,
            viewport: Viewport::default(),
            grid: Grid::default(),
            glide: None,
            reduced_motion: false,
            painter: Painter::default(),
        }
    }

    /// Shows another drawing, keeping the viewport (scenario 6, point 3). With `glide`, which the host asks for when the
    /// file of the function shown changed (notation 2.1), each node goes from its old place to its new one, found by
    /// its key, unless the system asks for reduced motion; not for another function.
    pub fn set_drawing(&mut self, drawing: Arc<Drawing>, glide: bool) {
        self.glide = match (&self.drawing, glide && !self.reduced_motion) {
            (Some(old), true) => Glide::between(old, &drawing),
            _ => None,
        };
        self.grid = Grid::of(&drawing);
        self.generation = self.generation.wrapping_add(1);
        self.drawing = Some(drawing);
    }

    pub fn drawing(&self) -> Option<&Arc<Drawing>> {
        self.drawing.as_ref()
    }

    /// No glide: what the system asks for (`prefers-reduced-motion` in a browser), or a preference of the viewer.
    pub fn set_reduced_motion(&mut self, reduced: bool) {
        self.reduced_motion = reduced;
        if reduced {
            self.glide = None;
        }
    }

    /// The zoom, 1 for 100 %, and the point of the drawing at the top-left corner of the canvas, in units.
    pub fn viewport(&self) -> (f32, Vec2) {
        (self.viewport.zoom, self.viewport.origin)
    }

    pub fn set_viewport(&mut self, zoom: f32, origin: Vec2) {
        self.viewport.zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        self.viewport.origin = origin;
        self.viewport.fitted = true;
    }

    /// The whole drawing in `size`, at 100 % at most (notation 7), at the next frame.
    pub fn fit(&mut self) {
        self.viewport.fitted = false;
    }

    /// Whether a glide is under way.
    pub fn gliding(&self) -> bool {
        self.glide.is_some()
    }

    /// Paints the drawing in all the room that `ui` gives, and reads the pointer: drag to pan, the wheel with Ctrl or
    /// a pinch to zoom, the wheel alone to scroll, a click on a tab.
    pub fn show(&mut self, ui: &mut Ui, theme: Theme, shown: &Shown) -> CanvasResponse {
        // With the values its drawing was drawn with (notation 10).
        let style = self.drawing.as_ref().map(|drawing| drawing.style.clone()).unwrap_or_default();
        style::scope(&style, || self.show_in_style(ui, theme, shown))
    }

    fn show_in_style(&mut self, ui: &mut Ui, theme: Theme, shown: &Shown) -> CanvasResponse {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, theme.color("canvas.fill"));
        let Some(drawing) = self.drawing.clone() else { return CanvasResponse { response, tab: None, hovered: None } };
        let ctx = ui.ctx().clone();
        let now = ctx.input(|input| input.time);
        self.viewport.read_input(&ctx, &response, rect, &drawing);
        let zoom = self.viewport.zoom;
        // A frame when the zoom has settled, to lay its texts out again at the zoom itself.
        if let Some(left) = self.viewport.settles_in(now) {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(left));
        }
        // The texts wait for the fonts of the notation: a host that never installed them, or set its fonts again.
        let texts = has_family(&ctx);
        if !texts {
            install_fonts(&ctx);
            ctx.request_repaint();
        }
        let offsets = match &mut self.glide {
            Some(glide) => {
                let done = glide.advance(now);
                ctx.request_repaint();
                if done {
                    self.glide = None;
                    None
                } else {
                    self.glide.as_ref().map(Glide::offsets)
                }
            }
            None => None,
        };
        let arms = arms_shown(&drawing, shown);
        let visible_arm = |within: Option<usize>| within.is_none_or(|arm| arms.get(arm).copied().unwrap_or(false));
        let to_screen = |unit: Vec2| rect.min + (unit - self.viewport.origin) * zoom;
        let seen = Rect::from_min_max(self.viewport.origin.to_pos2(), (self.viewport.origin + rect.size() / zoom).to_pos2());
        let step = self.viewport.step(now);
        let key = paint::Key { generation: self.generation, theme, step: step.to_bits(), pixels: ctx.pixels_per_point().to_bits() };
        self.painter.begin(&ctx, key);
        let coarse = zoom < style::factor(style::coarse_zoom());
        let put = |painted: &Painted, offset: Vec2| painted.paint(&painter, to_screen(offset), zoom / step, zoom, texts);
        let offset_of = |placed: Option<usize>| match (&offsets, placed) {
            (Some(offsets), Some(placed)) => offsets.get(placed).copied().unwrap_or(Vec2::ZERO),
            _ => Vec2::ZERO,
        };
        // While nodes glide, those whose way crosses the viewport too.
        let mut groups = self.grid.seen(seen);
        if offsets.is_some()
            && let Some(glide) = &self.glide
        {
            groups.extend(self.grid.gliding(glide.from(), seen));
            groups.sort_unstable();
            groups.dedup();
        }
        // The frame of the function, then each group in the order of drawing, then the tabs, then the marks.
        if let Some(painted) = self.painter.loose(&ctx, &drawing, theme, step) {
            put(&painted, Vec2::ZERO);
        }
        for index in groups {
            let Some(group) = drawing.groups.get(index) else { continue };
            if !visible_arm(group.within) {
                continue;
            }
            // The new wires show at the end of the glide.
            if offsets.is_some() && group.wire.is_some() {
                continue;
            }
            let offset = offset_of(group.placed);
            let placed = group.placed.and_then(|placed| drawing.placed.get(placed));
            if coarse
                && group.arm.is_none()
                && let Some(placed) = placed.filter(|placed| placed.holds.is_empty())
            {
                // A node as a plain rectangle (notation 7).
                let corner = to_screen(offset + Vec2::new(placed.x as f32, placed.y as f32) / 100.0);
                let size = Vec2::new(placed.width as f32, placed.height as f32) / 100.0 * zoom;
                painter.add(paint::coarse_rect(Rect::from_min_size(corner, size), COARSE_NODE, theme, zoom));
                continue;
            }
            if let Some(painted) = self.painter.group(&ctx, &drawing, index, theme, step) {
                put(&painted, offset);
            }
        }
        let mut tab = None;
        let pointer = response.interact_pointer_pos().filter(|_| response.clicked());
        for (index, found) in drawing.tabs.iter().enumerate() {
            let structure_shown = drawing.arms.get(found.arm).is_some_and(|arm| visible_arm(arm.within));
            if !structure_shown {
                continue;
            }
            let offset = offset_of(Some(found.structure));
            let unit = Rect::from_min_size(
                Pos2::new(found.x as f32 / 100.0, found.y as f32 / 100.0) + offset,
                Vec2::new(found.width as f32, found.height as f32) / 100.0,
            );
            if !unit.intersects(seen) {
                continue;
            }
            let is_shown = arms.get(found.arm).copied().unwrap_or(false);
            if let Some(painted) = self.painter.tab(&ctx, &drawing, index, is_shown, theme, step) {
                put(&painted, offset);
            }
            let area = Rect::from_min_size(to_screen(unit.min.to_vec2()), unit.size() * zoom);
            if pointer.is_some_and(|at| area.contains(at))
                && let Some(arm) = drawing.arms.get(found.arm)
            {
                tab = Some((arm.structure.clone(), arm.arm));
            }
        }
        if offsets.is_none() {
            for index in 0..drawing.marks.len() {
                if let Some(painted) = self.painter.mark(&ctx, &drawing, index, theme, step) {
                    put(&painted, Vec2::ZERO);
                }
            }
        }
        // The tooltip of the node under the pointer: the full name of what it calls (notation 3.2).
        let hovered = response.hover_pos().and_then(|at| {
            let unit = self.viewport.origin + (at - rect.min) / zoom;
            self.grid.node_at(&drawing, unit.to_pos2(), &visible_arm)
        });
        let title = hovered
            .and_then(|placed| self.grid.group_of(placed))
            .and_then(|group| drawing.groups.get(group))
            .map(|group| group.title.clone())
            .filter(|title| !title.is_empty());
        let response = match title {
            Some(title) => response.on_hover_text_at_pointer(title),
            None => response,
        };
        CanvasResponse { response, tab, hovered }
    }
}

/// Whether each arm in tabs is shown: the arm that `shown` chooses for its structure, inside an arm shown.
fn arms_shown(drawing: &Drawing, shown: &Shown) -> Vec<bool> {
    let counts: HashMap<&str, usize> = drawing.arms.iter().fold(HashMap::new(), |mut counts, arm| {
        *counts.entry(arm.structure.as_str()).or_insert(0) += 1;
        counts
    });
    let mut result: Vec<bool> = Vec::with_capacity(drawing.arms.len());
    for arm in &drawing.arms {
        let count = counts.get(arm.structure.as_str()).copied().unwrap_or(1);
        let chosen = shown.arm_of(&arm.structure);
        let chosen = if chosen < count { chosen } else { 0 };
        // An arm comes after the arm that holds its structure.
        let around = arm.within.is_none_or(|within| result.get(within).copied().unwrap_or(false));
        result.push(around && arm.arm == chosen);
    }
    result
}

/// The box of a shape, in units.
fn extent(shape: &Shape) -> Rect {
    let (x0, y0, x1, y1) = match shape {
        Shape::Rect { x, y, width, height, .. } => (*x, *y, x + width, y + height),
        Shape::Circle { x, y, radius, .. } => (x - radius, y - radius, x + radius, y + radius),
        Shape::Poly { points, .. } | Shape::Wire { points, .. } => {
            let fold = points.iter().fold((i64::MAX, i64::MAX, i64::MIN, i64::MIN), |(a, b, c, d), &(x, y)| (a.min(x), b.min(y), c.max(x), d.max(y)));
            let half = if let Shape::Wire { style, .. } = shape { style.width.max(600) / 2 } else { 100 };
            (fold.0 - half, fold.1 - half, fold.2 + half, fold.3 + half)
        }
        Shape::Text { x, y, size, width, .. } => (*x, y - i64::from(*size), x + width, y + i64::from(*size) / 2),
        Shape::Icon { x, y, side, .. } => (*x, *y, x + side, y + side),
    };
    Rect::from_min_max(Pos2::new(x0 as f32 / 100.0, y0 as f32 / 100.0), Pos2::new(x1 as f32 / 100.0, y1 as f32 / 100.0))
}

#[cfg(test)]
mod tests;
