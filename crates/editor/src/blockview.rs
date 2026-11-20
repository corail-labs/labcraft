//! The block of the view `Block` (ide.md 3): the block of a function alone, enlarged, where a drag on a terminal moves it
//! onto a place of a side (spec 11.2) and a drag of the right edge, the bottom edge or their corner changes its size; and
//! the same block at its real size. Every change is a command that the host sends (commands.md 9.4).

use std::sync::Arc;

use egui::{Color32, PointerButton, Pos2, Rect, Sense, Ui, Vec2};
use labcraft_diagram::{Drawing, MAX_PLACES, OwnBlock, style};

use crate::Theme;
use crate::blockdrag::{self, BlockDrag};
use crate::paint::{Key, Painter};

/// The most that the block is enlarged (decision of the owner, 2026-11-20).
pub const MAX_ENLARGED: f32 = 8.0;

/// The places are 8 units apart (spec 11.2).
const PITCH: f32 = labcraft_diagram::PLACE_PITCH as f32 / 100.0;

/// How near an edge, in points, a drag takes it.
const EDGE_REACH: f32 = 6.0;

/// What a gesture on the block asks for: the host makes it a command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlockIntent {
    /// `block.set_terminal`: a terminal (`input x`) dropped on a place of a side.
    Terminal { terminal: String, side: &'static str, place: u32 },
    /// `block.set_size`: the sizes that the drag of an edge changed.
    Size { top: Option<u32>, left: Option<u32> },
    /// What the view says, for a gesture that it refuses.
    Say(String),
}

/// Which edges a drag moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Edges {
    right: bool,
    bottom: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum Drag {
    Terminal(BlockDrag),
    /// The edges, and the sizes where they are now.
    Edges {
        edges: Edges,
        top: u32,
        left: u32,
    },
}

/// Where each picture of the block is painted: enlarged, then at its real size in each theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Picture {
    Enlarged,
    Real(Theme),
}

/// The state of the view between two frames: the drag under way, and what is painted of the drawing, tessellated once per
/// drawing, theme and zoom, as the canvas does.
#[derive(Default)]
pub struct BlockView {
    drag: Option<Drag>,
    /// The drawing whose block is shown, and a count that changes with it, for the caches.
    shown: Option<Arc<Drawing>>,
    generation: u64,
    painters: [Painter; 3],
}

/// Hundredths to units.
fn units(value: i64) -> f32 {
    value as f32 / 100.0
}

/// The box of the block, in units.
fn box_of(drawing: &Drawing) -> Option<Rect> {
    let placed = drawing.placed.first()?;
    Some(Rect::from_min_size(Pos2::new(units(placed.x), units(placed.y)), Vec2::new(units(placed.width), units(placed.height))))
}

/// The size of a drawing, in units.
fn size_of(drawing: &Drawing) -> Vec2 {
    Vec2::new(units(drawing.width), units(drawing.height))
}

/// The zoom of the enlarged block in `room`: the largest whole one that holds it, from 1 to [`MAX_ENLARGED`].
pub fn enlarged_zoom(drawing: &Drawing, room: Vec2) -> f32 {
    let size = size_of(drawing);
    if size.x <= 0.0 || size.y <= 0.0 {
        return 1.0;
    }
    (room.x / size.x).min(room.y / size.y).floor().clamp(1.0, MAX_ENLARGED)
}

impl BlockView {
    pub fn new() -> BlockView {
        BlockView::default()
    }

    /// The block of `drawing` ([`Drawing::block`]); the count of the drawing shown changes with the drawing.
    fn take<'d>(&mut self, drawing: &'d Arc<Drawing>) -> Option<&'d OwnBlock> {
        if !self.shown.as_ref().is_some_and(|shown| Arc::ptr_eq(shown, drawing)) {
            self.shown = Some(drawing.clone());
            self.generation += 1;
        }
        drawing.block.as_deref()
    }

    /// Paints the block with the top-left corner of its drawing at `corner`, at `zoom`, on the canvas colour of `theme`.
    fn paint(&mut self, ui: &Ui, own: &OwnBlock, (area, corner, zoom): (Rect, Pos2, f32), theme: Theme, picture: Picture) {
        let drawing = &own.drawing;
        let painter = ui.painter_at(area);
        painter.rect_filled(area, 4.0, theme.color("canvas.fill"));
        let ctx = ui.ctx();
        let slot = match picture {
            Picture::Enlarged => 0,
            Picture::Real(Theme::Light) => 1,
            Picture::Real(Theme::Dark) => 2,
        };
        let generation = self.generation;
        let Some(cache) = self.painters.get_mut(slot) else { return };
        cache.begin(ctx, Key { generation, theme, step: zoom.to_bits(), pixels: ctx.pixels_per_point().to_bits() });
        for index in 0..drawing.groups.len() {
            if let Some(painted) = cache.group(ctx, drawing, index, theme, zoom) {
                painted.paint(&painter, corner, 1.0, zoom, true);
            }
        }
    }

    /// The block of `drawing` at its real size, in `theme`, whatever the theme of the window: the two of them side by side
    /// show how it reads in each (decision of the owner, 2026-11-20).
    pub fn real_size(&mut self, ui: &mut Ui, drawing: &Arc<Drawing>, theme: Theme) -> Option<egui::Response> {
        let own = self.take(drawing)?;
        let (area, response) = ui.allocate_exact_size(size_of(&own.drawing), Sense::hover());
        style::scope(&own.drawing.style, || self.paint(ui, own, (area, area.min, 1.0), theme, Picture::Real(theme)));
        Some(response)
    }

    /// The block of `drawing` enlarged in the room left in `ui`; `editable` false when the commands of the block cannot
    /// change it ([`OwnBlock::fixed`]), which a drag then says.
    pub fn show(&mut self, ui: &mut Ui, drawing: &Arc<Drawing>, theme: Theme, editable: bool) -> Vec<BlockIntent> {
        let Some(own) = self.take(drawing) else { return Vec::new() };
        style::scope(&own.drawing.style, || self.show_in_style(ui, own, theme, editable))
    }

    fn show_in_style(&mut self, ui: &mut Ui, own: &OwnBlock, theme: Theme, editable: bool) -> Vec<BlockIntent> {
        let drawing = &own.drawing;
        let (area, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        let zoom = enlarged_zoom(drawing, area.size());
        let corner = area.center() - size_of(drawing) * zoom / 2.0;
        let to_screen = |unit: Pos2| corner + unit.to_vec2() * zoom;
        let unit_of = |at: Pos2| ((at - corner) / zoom).to_pos2();
        self.paint(ui, own, (area, corner, zoom), theme, Picture::Enlarged);
        let mut intents = Vec::new();
        let Some(block) = box_of(drawing) else { return intents };
        let screen_box = Rect::from_min_max(to_screen(block.min), to_screen(block.max));
        let hover = response.hover_pos();
        let port_under = |at: Pos2| {
            let unit = unit_of(at);
            let reach = (4.0f32).max(EDGE_REACH / zoom);
            drawing
                .ports
                .iter()
                .enumerate()
                .filter(|(_, port)| port.placed.is_some() && !port.order)
                .map(|(index, port)| (index, Pos2::new(units(port.x), units(port.y)).distance(unit)))
                .filter(|(_, distance)| *distance <= reach)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(index, _)| index)
        };
        let edges_under = |at: Pos2| {
            let along_x = at.x >= screen_box.min.x - EDGE_REACH && at.x <= screen_box.max.x + EDGE_REACH;
            let along_y = at.y >= screen_box.min.y - EDGE_REACH && at.y <= screen_box.max.y + EDGE_REACH;
            let edges = Edges {
                right: along_y && (at.x - screen_box.max.x).abs() <= EDGE_REACH,
                bottom: along_x && (at.y - screen_box.max.y).abs() <= EDGE_REACH,
            };
            (edges.right || edges.bottom).then_some(edges)
        };
        // A drag starts on a terminal, or else on an edge.
        if response.drag_started_by(PointerButton::Primary)
            && let Some(from) = ui.ctx().input(|input| input.pointer.press_origin())
        {
            let (port, edges) = (port_under(from), edges_under(from));
            if !editable && (port.is_some() || edges.is_some()) {
                intents.push(BlockIntent::Say(own.fixed.clone()));
            } else if let Some(port) = port {
                match BlockDrag::new(drawing, port, unit_of(from)) {
                    Ok(moving) => self.drag = Some(Drag::Terminal(moving)),
                    Err(why) if !why.is_empty() => intents.push(BlockIntent::Say(why)),
                    Err(_) => {}
                }
            } else if let Some(edges) = edges {
                self.drag = Some(Drag::Edges { edges, top: own.top, left: own.left });
            }
        }
        // Escape puts it back (notation 2.6).
        if self.drag.is_some() && ui.ctx().input(|input| input.key_pressed(egui::Key::Escape)) {
            self.drag = None;
        }
        let pointer = response.interact_pointer_pos();
        match &mut self.drag {
            Some(Drag::Terminal(moving)) => {
                if let Some(at) = pointer {
                    moving.set_at(drawing, unit_of(at));
                }
            }
            Some(Drag::Edges { edges, top, left }) => {
                if let Some(at) = pointer.map(unit_of) {
                    let places =
                        |length: f32, least: u32| ((length / PITCH).round() as i64 - 1).clamp(i64::from(least), i64::from(MAX_PLACES)) as u32;
                    if edges.right {
                        *top = places(at.x - block.min.x, own.least.0.max(1));
                    }
                    if edges.bottom {
                        *left = places(at.y - block.min.y, own.least.1);
                    }
                }
            }
            None => {}
        }
        if response.drag_stopped_by(PointerButton::Primary) {
            match self.drag.take() {
                Some(Drag::Terminal(moving)) if moving.changed() => {
                    let (side, place) = moving.to;
                    intents.push(BlockIntent::Terminal { terminal: moving.terminal.clone(), side: side.word(), place });
                }
                Some(Drag::Edges { edges, top, left }) => {
                    let top = (edges.right && top != own.top).then_some(top);
                    let left = (edges.bottom && left != own.left).then_some(left);
                    if top.is_some() || left.is_some() {
                        intents.push(BlockIntent::Size { top, left });
                    }
                }
                _ => {}
            }
        }
        self.paint_drag(ui, drawing, (block, zoom, theme), &to_screen);
        // The cursor says what a drag would do; the tooltip of a terminal says what it is.
        if self.drag.is_none()
            && let Some(at) = hover
        {
            if let Some(port) = port_under(at) {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                let hint = drawing.ports.get(port).and_then(|port| {
                    drawing.hints.iter().find(|hint| {
                        let (x, y) = (port.x - hint.x, port.y - hint.y);
                        (0..=hint.width).contains(&x) && (0..=hint.height).contains(&y)
                    })
                });
                if let Some(hint) = hint {
                    response.clone().on_hover_text_at_pointer(&hint.text);
                }
            } else if let Some(edges) = edges_under(at) {
                ui.ctx().set_cursor_icon(match (edges.right, edges.bottom) {
                    (true, true) => egui::CursorIcon::ResizeNwSe,
                    (true, false) => egui::CursorIcon::ResizeHorizontal,
                    _ => egui::CursorIcon::ResizeVertical,
                });
            }
        }
        intents
    }

    /// While a terminal moves, the free places lit and the one it goes to ringed, with the note that every call changes
    /// (notation 2.4); while an edge moves, the box of the new size and the sizes.
    fn paint_drag(&self, ui: &Ui, drawing: &Drawing, (block, zoom, theme): (Rect, f32, Theme), to_screen: &dyn Fn(Pos2) -> Pos2) {
        let painter = ui.painter();
        let select = theme.color("select.stroke");
        let muted = theme.color("ink.muted");
        let font = egui::FontId::proportional(12.0);
        match &self.drag {
            Some(Drag::Terminal(moving)) => {
                for place in moving.free(drawing) {
                    if let Some(at) = blockdrag::place_at(drawing, moving.placed, place) {
                        painter.circle_filled(to_screen(at), (1.5 * zoom).max(2.0), select.gamma_multiply(0.35));
                    }
                }
                if let Some(at) = blockdrag::place_at(drawing, moving.placed, moving.to) {
                    painter.circle(to_screen(at), (3.0 * zoom).max(4.0), select.gamma_multiply(0.25), egui::Stroke::new(1.5, select));
                }
                painter.text(to_screen(moving.at) + Vec2::new(12.0, 12.0), egui::Align2::LEFT_TOP, &moving.note, font, muted);
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            }
            Some(Drag::Edges { edges, top, left }) => {
                let width = if edges.right { (*top + 1) as f32 * PITCH } else { block.width() };
                let height = if edges.bottom { (*left + 1) as f32 * PITCH } else { block.height() };
                let area = Rect::from_min_size(to_screen(block.min), Vec2::new(width, height) * zoom);
                painter.rect(area, 0.0, select.gamma_multiply(0.06), egui::Stroke::new(1.5, select), egui::StrokeKind::Inside);
                let mut said = Vec::new();
                if edges.right {
                    said.push(format!("top {top}"));
                }
                if edges.bottom {
                    said.push(format!("left {left}"));
                }
                painter.text(area.right_bottom() + Vec2::new(8.0, 8.0), egui::Align2::LEFT_TOP, said.join(", "), font, muted);
            }
            None => {}
        }
    }
}

/// The colour that a swatch of the view shows for a colour `#RRGGBB`, read as `check` reads it.
pub fn swatch(hex: &str) -> Option<Color32> {
    labcraft_diagram::written_rgb(hex).map(|[red, green, blue]| Color32::from_rgb(red, green, blue))
}
