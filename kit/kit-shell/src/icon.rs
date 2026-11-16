//! The icons of the shell, drawn as lines: a glyph that no installed face has would stop egui, and the web has no
//! fonts of the system to fall back on.

use egui::{Pos2, Rect, Response, Sense, Stroke, Vec2, pos2};

/// An icon of the shell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Home,
    Undo,
    Redo,
    Close,
    Minimize,
    Maximize,
    /// Two squares, one behind the other: the window is maximized, a click restores it.
    Restore,
}

/// A button showing an icon, with its hover text; greyed when not enabled.
pub fn button(ui: &mut egui::Ui, icon: Icon, enabled: bool, hover: &str) -> Response {
    let size = Vec2::splat(ui.spacing().interact_size.y);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        if response.hovered() && enabled {
            ui.painter().rect_filled(rect, visuals.corner_radius, visuals.weak_bg_fill);
        }
        let colour = if enabled { visuals.fg_stroke.color } else { ui.visuals().weak_text_color() };
        paint(ui.painter(), rect.shrink(rect.width() * 0.25), icon, Stroke::new(1.4, colour));
    }
    response.on_hover_text(hover)
}

/// Paints `icon` within `rect`.
pub fn paint(painter: &egui::Painter, rect: Rect, icon: Icon, stroke: Stroke) {
    let at = |x: f32, y: f32| pos2(rect.left() + x * rect.width(), rect.top() + y * rect.height());
    let line = |points: &[Pos2]| painter.add(egui::Shape::line(points.to_vec(), stroke));
    match icon {
        Icon::Home => {
            line(&[at(0.0, 0.5), at(0.5, 0.05), at(1.0, 0.5)]);
            line(&[at(0.15, 0.4), at(0.15, 1.0), at(0.85, 1.0), at(0.85, 0.4)]);
        }
        Icon::Undo | Icon::Redo => {
            // A hook: the arrow points back for undo, forward for redo.
            let flip = |x: f32| if icon == Icon::Undo { x } else { 1.0 - x };
            let arc: Vec<Pos2> = (0..=12)
                .map(|step| {
                    let angle = std::f32::consts::PI * (1.5 - f32::from(u8::try_from(step).unwrap_or(0)) / 12.0);
                    at(flip(0.55 + 0.4 * angle.cos()), 0.6 - 0.35 * angle.sin())
                })
                .collect();
            line(&arc);
            line(&[at(flip(0.35), 0.0), at(flip(0.1), 0.25), at(flip(0.35), 0.5)]);
            line(&[at(flip(0.1), 0.25), at(flip(0.55), 0.25)]);
        }
        Icon::Close => {
            line(&[at(0.0, 0.0), at(1.0, 1.0)]);
            line(&[at(1.0, 0.0), at(0.0, 1.0)]);
        }
        Icon::Minimize => {
            line(&[at(0.0, 0.85), at(1.0, 0.85)]);
        }
        Icon::Maximize => {
            painter.rect_stroke(rect, 1.0, stroke, egui::StrokeKind::Inside);
        }
        Icon::Restore => {
            painter.rect_stroke(Rect::from_min_max(at(0.0, 0.25), at(0.75, 1.0)), 1.0, stroke, egui::StrokeKind::Inside);
            line(&[at(0.25, 0.25), at(0.25, 0.0), at(1.0, 0.0), at(1.0, 0.75), at(0.75, 0.75)]);
        }
    }
}
