//! The frame of the window, when the app draws it (`craft_window::Frame::App`): the window can be resized from its
//! edges and corners, and a thin line marks its border, so that it does not melt into the desktop. The window buttons
//! are in the top bar (`top_bar`). As the other Craft apps do on Windows (craft-anything, vectorcraft).

use craft_window::Frame;
use egui::viewport::ResizeDirection;
use egui::{Pos2, Rect, Stroke};

/// How close to the edge of the window the pointer resizes it, in points.
pub const EDGE: f32 = 5.0;

/// The width of a button of the window drawn by the app, as on Windows and in VS Code, in points.
pub const WINDOW_BUTTON: f32 = 46.0;

/// Room left for the buttons of the window that macOS draws over the top bar, in points.
pub const MACOS_BUTTONS: f32 = 84.0;

/// The edge or the corner of `window` that `pointer` would resize, if any.
pub fn edge_at(window: Rect, pointer: Pos2) -> Option<ResizeDirection> {
    let west = pointer.x - window.left() < EDGE;
    let east = window.right() - pointer.x < EDGE;
    let north = pointer.y - window.top() < EDGE;
    let south = window.bottom() - pointer.y < EDGE;
    Some(match (west, east, north, south) {
        (true, _, true, _) => ResizeDirection::NorthWest,
        (_, true, true, _) => ResizeDirection::NorthEast,
        (true, _, _, true) => ResizeDirection::SouthWest,
        (_, true, _, true) => ResizeDirection::SouthEast,
        (true, ..) => ResizeDirection::West,
        (_, true, ..) => ResizeDirection::East,
        (_, _, true, _) => ResizeDirection::North,
        (_, _, _, true) => ResizeDirection::South,
        _ => return None,
    })
}

/// Whether the window is maximized or full screen: it has no edge to resize then.
pub fn maximized(ctx: &egui::Context) -> bool {
    ctx.input(|input| input.viewport().maximized.unwrap_or(false) || input.viewport().fullscreen.unwrap_or(false))
}

/// Once a frame, when the app draws the frame: the cursor and the resizing over the edges, and the line of the border.
/// Nothing for `Frame::Desktop`, nor when the window is maximized.
pub fn edges(ctx: &egui::Context, frame: &Frame) {
    if frame.decorations() || maximized(ctx) {
        return;
    }
    let window = ctx.content_rect();
    // Over the buttons of the window the top edge is theirs: a press at the very top of Close closes.
    let buttons = frame.layout().map_or((0.0, 0.0), |layout| (button_width(layout.left.len()), button_width(layout.right.len())));
    let edge = ctx
        .input(|input| input.pointer.hover_pos())
        .and_then(|pointer| edge_at(window, pointer).filter(|direction| !over_buttons(window, pointer, *direction, buttons)));
    if let Some(direction) = edge {
        ctx.set_cursor_icon(cursor(direction));
        if ctx.input(|input| input.pointer.primary_pressed()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(direction));
        }
    }
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("kit-shell-frame")));
    let stroke = Stroke::new(1.0, ctx.global_style().visuals.widgets.noninteractive.bg_stroke.color);
    let border = window.shrink(0.5);
    // Windows draws a line of its own along the top of a window without a frame: a second one would be twice as thick.
    if !cfg!(windows) {
        painter.hline(border.x_range(), border.top(), stroke);
    }
    painter.hline(border.x_range(), border.bottom(), stroke);
    painter.vline(border.left(), border.y_range(), stroke);
    painter.vline(border.right(), border.y_range(), stroke);
}

fn button_width(count: usize) -> f32 {
    WINDOW_BUTTON * f32::from(u8::try_from(count).unwrap_or(u8::MAX))
}

/// Whether `direction`, at `pointer`, is the top edge over the buttons of the window: `buttons` the width they take on
/// the left and on the right.
fn over_buttons(window: Rect, pointer: Pos2, direction: ResizeDirection, buttons: (f32, f32)) -> bool {
    let top = matches!(direction, ResizeDirection::North | ResizeDirection::NorthEast | ResizeDirection::NorthWest);
    top && (pointer.x < window.left() + buttons.0 || pointer.x > window.right() - buttons.1)
}

fn cursor(direction: ResizeDirection) -> egui::CursorIcon {
    match direction {
        ResizeDirection::North => egui::CursorIcon::ResizeNorth,
        ResizeDirection::South => egui::CursorIcon::ResizeSouth,
        ResizeDirection::East => egui::CursorIcon::ResizeEast,
        ResizeDirection::West => egui::CursorIcon::ResizeWest,
        ResizeDirection::NorthEast => egui::CursorIcon::ResizeNorthEast,
        ResizeDirection::NorthWest => egui::CursorIcon::ResizeNorthWest,
        ResizeDirection::SouthEast => egui::CursorIcon::ResizeSouthEast,
        ResizeDirection::SouthWest => egui::CursorIcon::ResizeSouthWest,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;

    const WINDOW: Rect = Rect { min: pos2(0.0, 0.0), max: pos2(800.0, 600.0) };

    #[test]
    fn the_edges_and_corners_resize_and_the_inside_does_not() {
        assert_eq!(edge_at(WINDOW, pos2(1.0, 1.0)), Some(ResizeDirection::NorthWest));
        assert_eq!(edge_at(WINDOW, pos2(798.0, 599.0)), Some(ResizeDirection::SouthEast));
        assert_eq!(edge_at(WINDOW, pos2(400.0, 2.0)), Some(ResizeDirection::North));
        assert_eq!(edge_at(WINDOW, pos2(2.0, 300.0)), Some(ResizeDirection::West));
        assert_eq!(edge_at(WINDOW, pos2(797.0, 300.0)), Some(ResizeDirection::East));
        assert_eq!(edge_at(WINDOW, pos2(400.0, 598.0)), Some(ResizeDirection::South));
        assert_eq!(edge_at(WINDOW, pos2(400.0, 300.0)), None);
        // Exactly EDGE away is inside.
        assert_eq!(edge_at(WINDOW, pos2(EDGE, 300.0)), None);
    }

    #[test]
    fn the_top_edge_over_the_buttons_of_the_window_is_theirs() {
        let right = (0.0, 3.0 * WINDOW_BUTTON);
        assert!(over_buttons(WINDOW, pos2(790.0, 1.0), ResizeDirection::NorthEast, right));
        assert!(over_buttons(WINDOW, pos2(700.0, 1.0), ResizeDirection::North, right));
        assert!(!over_buttons(WINDOW, pos2(400.0, 1.0), ResizeDirection::North, right));
        // The right edge under the buttons still resizes.
        assert!(!over_buttons(WINDOW, pos2(798.0, 300.0), ResizeDirection::East, right));
    }
}
