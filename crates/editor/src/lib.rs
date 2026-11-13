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
use labcraft_diagram::{COARSE_NODE, Choice, Drawing, Overlay, Series, Shape, TabKind, Trace};

mod fonts;
pub(crate) mod paint;
pub(crate) mod series;
pub(crate) mod view;

pub use fonts::{FAMILY, family, has_family, install_fonts};
pub use view::{GLIDE_SECONDS, MAX_ZOOM, MIN_ZOOM};

use paint::{Over, Painted, Painter};
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
    /// An iteration chosen, by a selector or on the values of a wire pinned: the key of the loop and the iteration, from
    /// 0, for `view.set_state` (notation 6.1).
    pub iteration: Option<(String, u64)>,
    /// The node under the pointer, by its index in [`Drawing::placed`].
    pub hovered: Option<usize>,
}

/// The canvas: the drawing shown, its viewport, its caches, the glide.
pub struct Canvas {
    drawing: Option<Arc<Drawing>>,
    /// Changes with each drawing and each overlay, so that the caches know theirs.
    generation: u64,
    viewport: Viewport,
    grid: Grid,
    glide: Option<Glide>,
    reduced_motion: bool,
    painter: Painter,
    /// The list of the arms of a selector, open: its structure, by its index in [`Drawing::placed`], and the corner it
    /// opens from, in units, so that it follows a pan and a zoom (notation 5.2).
    list: Option<(usize, Pos2)>,
    /// The values of a run over the drawing, and whether they show (notation 6.1).
    values: Option<Values>,
    values_shown: bool,
    /// The trace of those values and the iterations chosen: the values of a wire over the iterations of its loop come
    /// from it, worked out when the wire is first hovered.
    run: Option<(Arc<Trace>, Choice)>,
    /// The series of each wire hovered or selected, by its group: none for a wire outside every loop.
    series: HashMap<usize, Option<Arc<Series>>>,
    /// What a click selected (notation 6.1; decisions of the owner, 2026-11-13).
    selected: Option<Selected>,
    /// A number being typed into the selector of a loop.
    typing: Option<Typing>,
    /// What the wheel turned over a selector and did not make a step yet, in points.
    wheel: f32,
    /// The iteration asked for last, until the values for it come: the next step goes from it, and the same is not asked
    /// twice.
    asked: Option<(String, u64)>,
}

/// What a click selected: a node, by its key, which follows it to a new drawing; or a wire, by its group, with the point
/// where it was clicked, in units, by which its values stay pinned.
#[derive(Clone, Debug, PartialEq)]
enum Selected {
    Node(String),
    Wire { group: usize, at: Pos2 },
}

/// The number typed into the selector of a loop: the loop, its iterations, the text, the corner of the field in units.
struct Typing {
    key: String,
    count: u64,
    text: String,
    /// The iteration shown, from 1, which the empty field shows faintly.
    hint: String,
    at: Pos2,
    focused: bool,
}

/// The wheel turned this far over a selector makes one step *(proposed)*: a notch of a mouse.
const WHEEL_STEP: f32 = 40.0;

/// The values of a run over a drawing, with the box of each of their parts, in units, to find what the viewport shows and
/// what the pointer is over.
struct Values {
    overlay: Arc<Overlay>,
    under: Vec<Rect>,
    banner: Rect,
    /// Nothing for a bubble that has no place: its value is read on hover of its wire.
    bubbles: Vec<Rect>,
    selectors: Vec<Rect>,
}

impl Values {
    fn of(overlay: Arc<Overlay>) -> Values {
        let under = overlay.under.iter().map(|layer| boxed(&layer.shapes).unwrap_or(Rect::NOTHING)).collect();
        let banner = boxed(&overlay.banner).unwrap_or(Rect::NOTHING);
        let bubbles = overlay.bubbles.iter().map(|bubble| boxed(&bubble.layer.shapes).unwrap_or(Rect::NOTHING)).collect();
        let selectors = overlay.selectors.iter().map(|selector| boxed(&selector.layer.shapes).unwrap_or(Rect::NOTHING)).collect();
        Values { overlay, under, banner, bubbles, selectors }
    }

    /// The selector shown under `unit`.
    fn selector_at(&self, unit: Pos2, visible: &dyn Fn(Option<usize>) -> bool) -> Option<&labcraft_diagram::Selector> {
        self.overlay
            .selectors
            .iter()
            .zip(&self.selectors)
            .find(|(selector, area)| visible(selector.layer.within) && area.contains(unit))
            .map(|(selector, _)| selector)
    }
}

/// The box of shapes, in units; none for no shape.
fn boxed(shapes: &[Shape]) -> Option<Rect> {
    shapes.iter().map(extent).reduce(|a, b| a.union(b))
}

/// Whether two boxes share some area: boxes that only touch do not.
fn overlaps(a: &Rect, b: &Rect) -> bool {
    a.min.x < b.max.x && b.min.x < a.max.x && a.min.y < b.max.y && b.min.y < a.max.y
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
            list: None,
            painter: Painter::default(),
            values: None,
            values_shown: true,
            run: None,
            series: HashMap::new(),
            selected: None,
            typing: None,
            wheel: 0.0,
            asked: None,
        }
    }

    /// The values of a run over the drawing shown (notation 6.1), made for it, with the trace they come from and the
    /// iterations chosen; none to show none. A new drawing comes without them: its values are those of its own trace.
    pub fn set_values(&mut self, overlay: Option<Arc<Overlay>>, run: Option<(Arc<Trace>, Choice)>) {
        self.values = overlay.map(Values::of);
        self.run = run;
        self.series.clear();
        self.asked = None;
        // Only what is painted of the values goes: an iteration that moves draws nothing again.
        self.painter.clear_values();
    }

    /// The node selected, by its key; none when nothing is, or a wire.
    pub fn selected_node(&self) -> Option<&str> {
        match &self.selected {
            Some(Selected::Node(key)) => Some(key),
            _ => None,
        }
    }

    /// Selects the node of this key, as a click on it does; none to select nothing.
    pub fn select_node(&mut self, key: Option<&str>) {
        self.selected = key.map(|key| Selected::Node(key.to_owned()));
    }

    /// The group of the wire selected, whose values stay pinned.
    pub fn selected_wire(&self) -> Option<usize> {
        match &self.selected {
            Some(Selected::Wire { group, .. }) => Some(*group),
            _ => None,
        }
    }

    /// Selects the wire of the group `group`, as a click on it at `at`, in units, does: its values are pinned there.
    pub fn select_wire(&mut self, group: usize, at: Pos2) {
        self.selected = Some(Selected::Wire { group, at });
    }

    /// Hides the values of the run, or shows them again: a state of the view, which places nothing (notation 6.1).
    pub fn set_values_shown(&mut self, shown: bool) {
        self.values_shown = shown;
    }

    pub fn values_shown(&self) -> bool {
        self.values_shown
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
        // A list open on the drawing before: its structure may be another one now.
        self.list = None;
        self.typing = None;
        // A node selected stays, found by its key; a wire is another one.
        self.selected = match self.selected.take() {
            Some(Selected::Node(key)) if drawing.placed.iter().any(|placed| placed.key == key) => Some(Selected::Node(key)),
            _ => None,
        };
        self.generation = self.generation.wrapping_add(1);
        self.drawing = Some(drawing);
        self.set_values(None, None);
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
        let Some(drawing) = self.drawing.clone() else { return CanvasResponse { response, tab: None, iteration: None, hovered: None } };
        let ctx = ui.ctx().clone();
        let now = ctx.input(|input| input.time);
        let values = self.values.as_ref().filter(|_| self.values_shown).map(|values| values.overlay.clone());
        let room = values.as_ref().map_or((0.0, 0.0), |values| (values.top as f32 / 100.0, values.width as f32 / 100.0));
        let arms = arms_shown(&drawing, shown);
        let visible_arm = |within: Option<usize>| within.is_none_or(|arm| arms.get(arm).copied().unwrap_or(false));
        // The selectors show with the values, from `zoom.coarse` on (notation 7).
        let selectors_shown = values.is_some() && self.viewport.zoom >= style::factor(style::coarse_zoom());
        let unit_of = |viewport: &Viewport, at: Pos2| (viewport.origin + (at - rect.min) / viewport.zoom).to_pos2();
        // The wheel over a selector steps its iterations, down forward, and does not scroll (decisions of the owner,
        // 2026-11-13).
        // Not while nodes glide, when no selector shows.
        let under_wheel = response.hover_pos().filter(|_| selectors_shown && self.glide.is_none()).and_then(|at| {
            let selector = self.values.as_ref()?.selector_at(unit_of(&self.viewport, at), &visible_arm)?;
            Some((selector.key.clone(), self.shown_of(selector), selector.count))
        });
        let mut iteration = None;
        match &under_wheel {
            Some((key, shown, count)) if ctx.input(|input| input.zoom_delta()) == 1.0 => {
                self.wheel += ctx.input(|input| input.smooth_scroll_delta.y);
                let steps = (self.wheel / WHEEL_STEP).trunc();
                self.wheel -= steps * WHEEL_STEP;
                let next = (i128::from(*shown) - steps as i128).clamp(0, i128::from(count.saturating_sub(1)));
                if steps != 0.0 && next != i128::from(*shown) {
                    iteration = u64::try_from(next).ok().map(|next| (key.clone(), next));
                }
            }
            _ => self.wheel = 0.0,
        }
        self.viewport.read_input(&ctx, &response, rect, &drawing, room, under_wheel.is_some());
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
        // What is under the pointer: a node, and a wire within 3 units, or 5 points on the screen when that is more; no wire
        // while nodes glide, as wires do not show.
        let at = response.hover_pos().map(|at| unit_of(&self.viewport, at));
        let hovered = at.and_then(|unit| self.grid.node_at(&drawing, unit, &visible_arm));
        let wire_under = at.filter(|_| offsets.is_none()).and_then(|unit| {
            let reach = (5.0 / zoom).max(3.0);
            groups.iter().filter_map(|&index| Some((index, drawing.groups.get(index)?))).filter(|(_, group)| visible_arm(group.within)).find_map(
                |(index, group)| {
                    let wire = group.wire.as_ref()?;
                    let near = wire.readers.iter().any(|(_, points)| {
                        points.windows(2).any(|pair| match pair {
                            [a, b] => near_segment(unit, *a, *b, reach),
                            _ => false,
                        })
                    });
                    near.then_some(index)
                },
            )
        });
        // The title of the function, then each group in the order of drawing, then the tabs, then the marks.
        if let Some(painted) = self.painter.loose(&ctx, &drawing, theme, step) {
            put(&painted, Vec2::ZERO);
        }
        for &index in &groups {
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
        let mut opened = false;
        let pointer = response.interact_pointer_pos().filter(|_| response.clicked());
        for (index, found) in drawing.tabs.iter().enumerate() {
            let structure_shown = drawing.arms.get(found.arm).is_some_and(|arm| visible_arm(arm.within));
            // A part of a selector shows with the arm it stands for.
            let with_its_arm = found.when.is_none_or(|when| arms.get(when).copied().unwrap_or(false));
            if !structure_shown || !with_its_arm {
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
                if found.kind == TabKind::Name {
                    self.list = Some((found.structure, Pos2::new(unit.min.x, unit.max.y)));
                    opened = true;
                } else {
                    tab = Some((arm.structure.clone(), arm.arm));
                }
            }
        }
        // The list of the arms of a selector: one line per arm, the one shown selected; a choice, or a click elsewhere,
        // closes it.
        if let Some((structure, anchor)) = self.list {
            let corner = to_screen(anchor.to_vec2() + offset_of(Some(structure)));
            let entries: Vec<(&str, usize, bool)> = drawing
                .tabs
                .iter()
                .filter(|found| found.kind == TabKind::Name && found.structure == structure)
                .map(|found| (found.caption.as_str(), found.arm, arms.get(found.arm).copied().unwrap_or(false)))
                .collect();
            let mut chosen = None;
            let area = egui::Area::new(egui::Id::unique("labcraft-arms-list")).order(egui::Order::Foreground).fixed_pos(corner).show(&ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                        for &(caption, arm, is_shown) in &entries {
                            if ui.selectable_label(is_shown, caption).clicked() {
                                chosen = Some(arm);
                            }
                        }
                    });
                });
            });
            if let Some(arm) = chosen.and_then(|arm| drawing.arms.get(arm)) {
                tab = Some((arm.structure.clone(), arm.arm));
                self.list = None;
            } else {
                let clicked_at = ctx.input(|input| input.pointer.any_click().then(|| input.pointer.interact_pos()).flatten());
                let elsewhere = clicked_at.is_some_and(|at| !area.response.rect.contains(at));
                if entries.is_empty() || (elsewhere && !opened) {
                    self.list = None;
                }
            }
        }
        // The values of the run, but while nodes glide (notation 6.1): the veils, hatching, outlines and counters under the
        // marks, the banner and the bubbles over everything; the bubbles from `zoom.values` on, and on hover down to
        // `zoom.coarse` (notation 7).
        let shown_values = self.values.as_ref().filter(|_| values.is_some() && offsets.is_none());
        // A counter that a selector stands in for, when the header has room for only one.
        let replaced: Vec<usize> = shown_values
            .filter(|_| !coarse)
            .map(|found| {
                found
                    .overlay
                    .selectors
                    .iter()
                    .filter(|selector| visible_arm(selector.layer.within))
                    .filter_map(|selector| selector.replaces)
                    .collect()
            })
            .unwrap_or_default();
        if let Some(Values { overlay, under, .. }) = shown_values {
            for (index, layer) in overlay.under.iter().enumerate() {
                if visible_arm(layer.within)
                    && !replaced.contains(&index)
                    && under.get(index).is_some_and(|area| area.intersects(seen))
                    && let Some(painted) = self.painter.over(&ctx, &drawing, overlay, Over::Under(index), theme, step)
                {
                    put(&painted, Vec2::ZERO);
                }
            }
        }
        // The selection: the halo of a wire, the outline of a node (notation 6.1, 8).
        let selected_placed = match &self.selected {
            Some(Selected::Node(key)) => drawing.placed.iter().position(|placed| placed.key == *key),
            _ => None,
        };
        if offsets.is_none() {
            if let Some(Selected::Wire { group, .. }) = &self.selected
                && drawing.groups.get(*group).is_some_and(|found| visible_arm(found.within))
            {
                let group = *group;
                let painted =
                    self.painter.made(&ctx, &drawing, Over::SelectedWire(group), theme, step, || labcraft_diagram::selected_wire(&drawing, group));
                put(&painted, Vec2::ZERO);
            }
            if let Some(placed) = selected_placed.filter(|&placed| self.grid.shown(placed, &visible_arm)) {
                let painted =
                    self.painter.made(&ctx, &drawing, Over::SelectedNode(placed), theme, step, || labcraft_diagram::selected_node(&drawing, placed));
                put(&painted, Vec2::ZERO);
            }
            for index in 0..drawing.marks.len() {
                if let Some(painted) = self.painter.mark(&ctx, &drawing, index, theme, step) {
                    put(&painted, Vec2::ZERO);
                }
            }
        }
        // `zoom.values` may be set below `zoom.coarse`, under which no value shows (notation 7).
        let bubbles_drawn = !coarse && zoom >= style::factor(style::values_zoom());
        // The bubbles that a value at an input covers, by their index: neither drawn nor read on hover.
        let mut hidden = std::collections::BTreeSet::new();
        if let Some(Values { overlay, banner, bubbles, selectors, .. }) = shown_values {
            if banner.intersects(seen)
                && let Some(painted) = self.painter.over(&ctx, &drawing, overlay, Over::Banner, theme, step)
            {
                put(&painted, Vec2::ZERO);
            }
            for (index, selector) in overlay.selectors.iter().enumerate() {
                if !coarse
                    && visible_arm(selector.layer.within)
                    && selectors.get(index).is_some_and(|area| area.intersects(seen))
                    && let Some(painted) = self.painter.over(&ctx, &drawing, overlay, Over::Selector(index), theme, step)
                {
                    put(&painted, Vec2::ZERO);
                }
            }
            // The values at the inputs of the node hovered and of the node selected: the bubble of the same wire that one
            // covers gives the same value and is not drawn (notation 6.1; decision of the owner, 2026-11-13).
            let nodes: std::collections::BTreeSet<usize> =
                if coarse { std::collections::BTreeSet::new() } else { [hovered, selected_placed].into_iter().flatten().collect() };
            for &placed in &nodes {
                let boxes = self.painter.input_boxes(placed, || {
                    labcraft_diagram::input_boxes(&drawing, overlay, placed)
                        .into_iter()
                        .map(|(group, (left, top, right, bottom))| {
                            let area = Rect::from_min_max(Pos2::new(left as f32, top as f32) / 100.0, Pos2::new(right as f32, bottom as f32) / 100.0);
                            (group, area)
                        })
                        .collect()
                });
                for (index, (bubble, area)) in overlay.bubbles.iter().zip(bubbles).enumerate() {
                    if boxes.iter().any(|(group, input)| bubble.group == Some(*group) && overlaps(input, area)) {
                        hidden.insert(index);
                    }
                }
            }
            for (index, bubble) in overlay.bubbles.iter().enumerate() {
                if bubbles_drawn
                    && visible_arm(bubble.layer.within)
                    && !hidden.contains(&index)
                    && bubbles.get(index).is_some_and(|area| area.intersects(seen))
                    && let Some(painted) = self.painter.over(&ctx, &drawing, overlay, Over::Bubble(index), theme, step)
                {
                    put(&painted, Vec2::ZERO);
                }
            }
            // The values at the inputs, over everything (notation 6.1).
            for placed in nodes {
                let painted =
                    self.painter.made(&ctx, &drawing, Over::Inputs(placed), theme, step, || labcraft_diagram::inputs_of(&drawing, overlay, placed));
                put(&painted, Vec2::ZERO);
            }
        }
        // A click on a selector: a step back or forward, or the number, which opens a field to type one.
        let clicked_unit = pointer.map(|at| unit_of(&self.viewport, at));
        let mut consumed = tab.is_some() || opened;
        let at_selector = clicked_unit.zip(shown_values.filter(|_| !coarse && !consumed)).and_then(|(unit, found)| {
            let selector = found.selector_at(unit, &visible_arm)?;
            let shown = self.shown_of(selector);
            let inside = |(left, top, right, bottom): (i64, i64, i64, i64)| {
                Rect::from_min_max(Pos2::new(left as f32, top as f32) / 100.0, Pos2::new(right as f32, bottom as f32) / 100.0).contains(unit)
            };
            Some((selector.clone(), shown, inside(selector.back), inside(selector.forward)))
        });
        if let Some((selector, shown, back, forward)) = at_selector {
            consumed = true;
            if back {
                iteration = shown.checked_sub(1).map(|back| (selector.key.clone(), back));
            } else if forward {
                iteration = Some(shown + 1).filter(|next| *next < selector.count).map(|next| (selector.key.clone(), next));
            } else {
                let (left, _, _, bottom) = selector.number;
                let at = Pos2::new(left as f32, bottom as f32) / 100.0;
                let hint = (shown + 1).to_string();
                self.typing = Some(Typing { key: selector.key.clone(), count: selector.count, text: String::new(), hint, at, focused: false });
            }
        }
        // A click elsewhere selects the wire or the node under it, or nothing; Escape selects nothing.
        if pointer.is_some() && !consumed {
            self.selected = match (wire_under, clicked_unit, hovered) {
                (Some(group), Some(at), _) => Some(Selected::Wire { group, at }),
                (None, _, Some(placed)) => drawing.placed.get(placed).map(|found| Selected::Node(found.key.clone())),
                _ => None,
            };
        }
        if self.typing.is_none() && ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.selected = None;
        }
        // The number typed into a selector: Enter chooses that iteration, from 1 as the selector counts; Escape, or a click
        // elsewhere, closes the field.
        if let Some(typing) = &mut self.typing {
            let corner = to_screen(typing.at.to_vec2());
            let mut done = false;
            egui::Area::new(egui::Id::unique("labcraft-iteration-field")).order(egui::Order::Foreground).fixed_pos(corner).show(&ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    let field = ui.add(egui::TextEdit::singleline(&mut typing.text).desired_width(64.0).hint_text(typing.hint.as_str()));
                    if !typing.focused {
                        field.request_focus();
                        typing.focused = true;
                    }
                    if field.lost_focus() {
                        if ui.input(|input| input.key_pressed(egui::Key::Enter))
                            && let Ok(number) = typing.text.trim().replace(',', "").parse::<u64>()
                        {
                            iteration = Some((typing.key.clone(), number.clamp(1, typing.count.max(1)) - 1));
                        }
                        done = true;
                    }
                });
            });
            if done {
                self.typing = None;
            }
        }
        // The values that the tooltip and the pinned values show: those of the wire over the iterations of its loop.
        let readable = shown_values.filter(|_| !coarse);
        let pinned = match &self.selected {
            Some(Selected::Wire { group, at }) => {
                // Pinned where the wire was clicked, while that place is in sight.
                (readable.is_some()
                    && offsets.is_none()
                    && drawing.groups.get(*group).is_some_and(|found| visible_arm(found.within))
                    && seen.contains(*at))
                .then_some((*group, *at))
            }
            _ => None,
        };
        let run = self.run.clone();
        let cache = &mut self.series;
        let mut series_of = |group: usize| -> Option<Arc<Series>> {
            let (trace, choice) = run.as_ref()?;
            cache.entry(group).or_insert_with(|| labcraft_diagram::series(&drawing, trace, choice, group).map(Arc::new)).clone()
        };
        let named = |group: usize| -> Option<String> {
            let wire = drawing.groups.get(group)?.wire.as_ref()?;
            let value = labcraft_types::cut_text(&wire.value);
            let named = if wire.ty.is_empty() { value.into_owned() } else { format!("{value}: {}", wire.ty) };
            // While the values show, the value of the run under its name.
            let run = readable.and_then(|readable| readable.overlay.wires.get(&group)).map(|valued| valued.title.clone());
            Some(match run {
                Some(run) => format!("{named}\n{run}"),
                None => named,
            })
        };
        if let Some((group, unit)) = pinned
            && let Some(title) = named(group)
        {
            let series = series_of(group);
            let corner = to_screen(unit.to_vec2()) + Vec2::new(12.0, 12.0);
            egui::Area::new(egui::Id::unique("labcraft-pinned-values")).order(egui::Order::Foreground).fixed_pos(corner).show(&ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| match &series {
                    Some(series) => {
                        if let Some(chosen) = series::show(ui, &title, series, theme, true) {
                            iteration = Some((series.key.clone(), chosen));
                        }
                    }
                    None => {
                        ui.label(&title);
                    }
                });
            });
        }
        // The tooltip under the pointer: a bubble, its value whole (notation 6.1); a terminal, its name, its exact type and
        // the default of an input that no argument wires (notation 3.2, 4.1); else the node, the full name of what it calls
        // (notation 3.2); else a wire, its name, its exact type, its value, and its values over the iterations of its loop.
        let bubble = || {
            let (unit, readable) = (at?, readable?);
            readable.overlay.bubbles.iter().zip(&readable.bubbles).enumerate().rev().find_map(|(index, (bubble, area))| {
                (bubbles_drawn && !hidden.contains(&index) && visible_arm(bubble.layer.within) && area.contains(unit)).then(|| bubble.title.clone())
            })
        };
        let hint = at.and_then(|unit| {
            drawing.hints.iter().rev().filter(|hint| visible_arm(hint.within)).find_map(|hint| {
                let corner = Pos2::new(hint.x as f32 / 100.0, hint.y as f32 / 100.0) + offset_of(hint.placed);
                Rect::from_min_size(corner, Vec2::new(hint.width as f32, hint.height as f32) / 100.0).contains(unit).then(|| hint.text.clone())
            })
        });
        let title = || {
            hovered
                .and_then(|placed| self.grid.group_of(placed))
                .and_then(|group| drawing.groups.get(group))
                .map(|group| group.title.clone())
                .filter(|title| !title.is_empty())
        };
        let text = bubble().or(hint).or_else(title);
        let response = match (text, wire_under) {
            (Some(text), _) => response.on_hover_text_at_pointer(text),
            // The values pinned already show those of the wire selected.
            (None, Some(group)) if pinned.is_some_and(|(pinned, _)| pinned == group) => response,
            (None, Some(group)) => match (named(group), readable.and_then(|_| series_of(group))) {
                (Some(title), Some(series)) => response.on_hover_ui_at_pointer(|ui| {
                    series::show(ui, &title, &series, theme, false);
                }),
                (Some(title), None) => response.on_hover_text_at_pointer(title),
                (None, _) => response,
            },
            (None, None) => response,
        };
        // Never the same iteration twice while its values are on their way.
        let iteration = iteration.filter(|asked| self.asked.as_ref() != Some(asked));
        if iteration.is_some() {
            self.asked.clone_from(&iteration);
        }
        CanvasResponse { response, tab, iteration, hovered }
    }

    /// The iteration a selector shows, or the one asked for last when its values have not come yet.
    fn shown_of(&self, selector: &labcraft_diagram::Selector) -> u64 {
        match &self.asked {
            Some((key, asked)) if *key == selector.key => *asked,
            _ => selector.shown,
        }
    }
}

/// Whether `at`, in units, is within `reach` of the orthogonal segment from `a` to `b`, in hundredths.
fn near_segment(at: Pos2, a: (i64, i64), b: (i64, i64), reach: f32) -> bool {
    let (ax, ay, bx, by) = (a.0 as f32 / 100.0, a.1 as f32 / 100.0, b.0 as f32 / 100.0, b.1 as f32 / 100.0);
    let x = at.x.clamp(ax.min(bx), ax.max(bx));
    let y = at.y.clamp(ay.min(by), ay.max(by));
    (at.x - x).abs() <= reach && (at.y - y).abs() <= reach
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
