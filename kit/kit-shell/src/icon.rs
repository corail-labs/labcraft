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
    /// The views of a tab (ide.md 3): two nodes and a wire.
    Diagram,
    /// Lines of text.
    Text,
    /// A box with its terminals.
    Block,
    /// Two columns: the text beside the diagram.
    Split,
    /// A front panel: a window with a knob and a gauge.
    Panel,
    /// Four corners: fit the diagram to the view.
    Fit,
    /// A frame with an arrow leaving it at its top right: move the tab into a window of its own.
    Detach,
    /// A frame with an arrow coming into it: move the window back among the tabs.
    Dock,
    /// A window with a column on its right: the inspector, beside the canvas.
    Inspector,
    /// A window with a column of lines on its left: the palette of functions, beside the canvas.
    Palette,
}

/// A button showing an icon that can be on, as one of a group (the views of a tab): its hover text names it; the one on
/// has the background of the selection and a line of the accent under it.
pub fn toggle(ui: &mut egui::Ui, icon: Icon, on: bool, enabled: bool, hover: &str) -> Response {
    let size = Vec2::new(ui.spacing().interact_size.y + 4.0, ui.spacing().interact_size.y);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    if ui.is_rect_visible(rect) {
        let visuals = ui.visuals();
        if on {
            ui.painter().rect_filled(rect, visuals.widgets.active.corner_radius, visuals.selection.bg_fill);
            let line = egui::Rect::from_min_max(egui::pos2(rect.left() + 5.0, rect.bottom() - 2.0), egui::pos2(rect.right() - 5.0, rect.bottom()));
            ui.painter().rect_filled(line, 1.0, visuals.hyperlink_color);
        } else if response.hovered() && enabled {
            ui.painter().rect_filled(rect, visuals.widgets.hovered.corner_radius, visuals.widgets.hovered.weak_bg_fill);
        }
        let colour = if !enabled {
            visuals.weak_text_color().gamma_multiply(0.6)
        } else if on {
            visuals.strong_text_color()
        } else {
            visuals.weak_text_color()
        };
        paint(ui.painter(), Rect::from_center_size(rect.center(), Vec2::splat(14.0)), icon, Stroke::new(1.3, colour));
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, enabled, on, hover));
    response.on_hover_text(hover)
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
        Icon::Diagram => {
            painter.rect_stroke(Rect::from_min_max(at(0.0, 0.1), at(0.35, 0.45)), 1.0, stroke, egui::StrokeKind::Inside);
            painter.rect_stroke(Rect::from_min_max(at(0.65, 0.55), at(1.0, 0.9)), 1.0, stroke, egui::StrokeKind::Inside);
            line(&[at(0.35, 0.28), at(0.5, 0.28), at(0.5, 0.72), at(0.65, 0.72)]);
        }
        Icon::Text => {
            for (y, end) in [(0.15, 1.0), (0.38, 0.8), (0.62, 1.0), (0.85, 0.6)] {
                line(&[at(0.0, y), at(end, y)]);
            }
        }
        Icon::Block => {
            painter.rect_stroke(Rect::from_min_max(at(0.2, 0.1), at(0.8, 0.9)), 1.0, stroke, egui::StrokeKind::Inside);
            for y in [0.3, 0.7] {
                line(&[at(0.0, y), at(0.2, y)]);
            }
            line(&[at(0.8, 0.5), at(1.0, 0.5)]);
        }
        Icon::Split => {
            painter.rect_stroke(Rect::from_min_max(at(0.0, 0.05), at(1.0, 0.95)), 1.0, stroke, egui::StrokeKind::Inside);
            line(&[at(0.5, 0.05), at(0.5, 0.95)]);
            for y in [0.3, 0.5, 0.7] {
                line(&[at(0.12, y), at(0.38, y)]);
            }
        }
        Icon::Panel => {
            painter.rect_stroke(Rect::from_min_max(at(0.0, 0.05), at(1.0, 0.95)), 1.0, stroke, egui::StrokeKind::Inside);
            painter.circle_stroke(at(0.3, 0.5), rect.width() * 0.16, stroke);
            line(&[at(0.6, 0.3), at(0.85, 0.3)]);
            line(&[at(0.6, 0.5), at(0.85, 0.5)]);
            line(&[at(0.6, 0.7), at(0.85, 0.7)]);
        }
        Icon::Fit => {
            let corner = |x: f32, y: f32, dx: f32, dy: f32| line(&[at(x + dx, y), at(x, y), at(x, y + dy)]);
            corner(0.0, 0.0, 0.3, 0.3);
            corner(1.0, 0.0, -0.3, 0.3);
            corner(0.0, 1.0, 0.3, -0.3);
            corner(1.0, 1.0, -0.3, -0.3);
        }
        Icon::Detach | Icon::Dock => {
            // The frame, open at its top right where the arrow passes.
            line(&[at(0.45, 0.1), at(0.0, 0.1), at(0.0, 1.0), at(0.9, 1.0), at(0.9, 0.55)]);
            if icon == Icon::Detach {
                line(&[at(0.4, 0.6), at(1.0, 0.0)]);
                line(&[at(0.6, 0.0), at(1.0, 0.0), at(1.0, 0.4)]);
            } else {
                line(&[at(1.0, 0.0), at(0.4, 0.6)]);
                line(&[at(0.4, 0.2), at(0.4, 0.6), at(0.8, 0.6)]);
            }
        }
        Icon::Inspector | Icon::Palette => {
            painter.rect_stroke(Rect::from_min_max(at(0.0, 0.05), at(1.0, 0.95)), 1.0, stroke, egui::StrokeKind::Inside);
            let (edge, from, to) = if icon == Icon::Inspector { (0.62, 0.72, 0.9) } else { (0.38, 0.1, 0.28) };
            line(&[at(edge, 0.05), at(edge, 0.95)]);
            for y in [0.3, 0.5, 0.7] {
                line(&[at(from, y), at(to, y)]);
            }
        }
    }
}
