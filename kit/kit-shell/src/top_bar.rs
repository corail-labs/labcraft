//! The top bar (ide.md 2): the name of the IDE, a house that opens the start page, the menus, undo and redo, the
//! workspace selector; in the middle, the name of the project and the field that opens the command palette; on the
//! right, segments (the agent, the settings, the help, the version) and, where the desktop draws no frame, the buttons
//! of the window. An empty part of the bar moves the window, a double click maximizes it (`craft-window`).
//!
//! The menus are data (`craft-menu`), laid out by the host for its platform and drawn here.

use craft_menu::{CommandRef, MenuBar, MenuBarUi};
use craft_window::{Button, Frame};
use egui::{RichText, Vec2};

use crate::icon::{self, Icon};
use crate::status::Segment;
use crate::workspaces::{self, Asked, Workspace};

/// What the bar shows.
pub struct Bar<'a> {
    /// The name of the IDE, shown first.
    pub name: &'a str,
    /// The menus, laid out for the platform (`craft_menu::layout`), with their keys and states applied.
    pub menus: &'a MenuBar,
    /// Whether undo and redo can run, and what they would undo or redo.
    pub undo: (bool, Option<String>),
    pub redo: (bool, Option<String>),
    pub workspaces: &'a [Workspace],
    pub workspace: &'a str,
    /// The project open, none on the start page.
    pub project: Option<&'a str>,
    /// The key of the command palette, shown in its field.
    pub palette_key: Option<&'a str>,
    pub right: &'a [Segment],
    /// Who draws the frame of the window.
    pub frame: &'a Frame,
}

/// What the bar asks for this frame.
#[derive(Clone, Debug, PartialEq)]
pub enum Answer {
    Home,
    /// An entry of a menu.
    Menu(CommandRef),
    Undo,
    Redo,
    Workspace(Asked),
    OpenPalette,
    /// The command of a segment of the right.
    Segment(String),
}

/// Shows the bar in `ui`, a top panel.
pub fn show(ui: &mut egui::Ui, bar: &Bar) -> Vec<Answer> {
    let mut answers = Vec::new();
    // Under the controls, so that they take their own clicks.
    craft_window::title_bar::empty_area(ui, ui.max_rect());
    let height = ui.max_rect().height();
    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 0.0);
        if let Some(layout) = bar.frame.layout() {
            window_buttons(ui, &layout.left, height);
        }
        // macOS draws its buttons over the bar (the content reaches under its title bar).
        // In full screen they are hidden, and so is the room.
        if cfg!(target_os = "macos") && bar.frame.decorations() && !ui.input(|input| input.viewport().fullscreen.unwrap_or(false)) {
            ui.add_space(crate::frame::MACOS_BUTTONS);
        }
        ui.label(RichText::new(bar.name).strong());
        if icon::button(ui, Icon::Home, true, "Start page").clicked() {
            answers.push(Answer::Home);
        }
        answers.extend(MenuBarUi::new(bar.menus).shrink_to_fit(ui.available_width() * 0.4).show(ui).into_iter().map(Answer::Menu));
        ui.separator();
        if icon::button(ui, Icon::Undo, bar.undo.0, &hover("Undo", bar.undo.1.as_deref())).clicked() {
            answers.push(Answer::Undo);
        }
        if icon::button(ui, Icon::Redo, bar.redo.0, &hover("Redo", bar.redo.1.as_deref())).clicked() {
            answers.push(Answer::Redo);
        }
        ui.separator();
        if let Some(asked) = workspaces::selector(ui, bar.workspaces, bar.workspace) {
            answers.push(Answer::Workspace(asked));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(layout) = bar.frame.layout()
                && !layout.right.is_empty()
            {
                window_buttons(ui, &layout.right.iter().rev().copied().collect::<Vec<_>>(), height);
                ui.separator();
            }
            for segment in bar.right.iter().rev() {
                let mut response = ui.add(egui::Button::new(RichText::new(&segment.text).small()).frame(false));
                if let Some(hover) = &segment.hover {
                    response = response.on_hover_text(hover.as_str());
                }
                if response.clicked()
                    && let Some(command) = &segment.command
                {
                    answers.push(Answer::Segment(command.clone()));
                }
            }
            // The middle: the project, and the field of the palette.
            // Centred on the window as in VS Code, and narrow enough to leave empty bar on both sides to move the window.
            if let Some(rect) = field_rect(ui.available_rect_before_wrap(), ui.ctx().content_rect().center().x) {
                let text = match (bar.project, bar.palette_key) {
                    (Some(project), Some(key)) => format!("{project}  —  search  ({key})"),
                    (Some(project), None) => format!("{project}  —  search"),
                    (None, Some(key)) => format!("Search or run a command  ({key})"),
                    (None, None) => "Search or run a command".to_owned(),
                };
                let field = egui::Button::new(RichText::new(text).weak()).truncate();
                if ui.put(rect, field).on_hover_text("Search or run a command").clicked() {
                    answers.push(Answer::OpenPalette);
                }
            }
        });
    });
    answers
}

/// Where the field of the palette goes in `free`, the part of the bar left in the middle: as wide as `FIELD_WIDTH` at
/// most, `DRAG_ROOM` of empty bar kept on each side, centred on `window_centre` as far as it can be. None when it would
/// be narrower than `FIELD_MIN_WIDTH`.
fn field_rect(free: egui::Rect, window_centre: f32) -> Option<egui::Rect> {
    let width = (free.width() - 2.0 * DRAG_ROOM).min(FIELD_WIDTH);
    if width.is_nan() || width < FIELD_MIN_WIDTH {
        return None;
    }
    let half = width / 2.0;
    let lowest = free.left() + DRAG_ROOM + half;
    // Rounding can put the highest a hair under the lowest, where `clamp` would panic.
    let highest = (free.right() - DRAG_ROOM - half).max(lowest);
    let centre = window_centre.max(lowest).min(highest);
    Some(egui::Rect::from_center_size(egui::pos2(centre, free.center().y), Vec2::new(width, FIELD_HEIGHT)))
}

fn hover(what: &str, detail: Option<&str>) -> String {
    match detail {
        Some(detail) => format!("{what}: {detail}"),
        None => format!("Nothing to {}", what.to_lowercase()),
    }
}

/// The field of the palette: its widest, its narrowest before it is left out, its height.
const FIELD_WIDTH: f32 = 360.0;
const FIELD_MIN_WIDTH: f32 = 140.0;
const FIELD_HEIGHT: f32 = 22.0;
/// The empty bar kept on each side of the field, to move the window.
const DRAG_ROOM: f32 = 48.0;

/// The buttons of the window, when the app draws its frame: cells of the full height of the bar, the one of closing
/// red under the pointer.
fn window_buttons(ui: &mut egui::Ui, buttons: &[Button], height: f32) {
    let maximized = crate::frame::maximized(ui.ctx());
    let spacing = std::mem::replace(&mut ui.spacing_mut().item_spacing.x, 0.0);
    for button in buttons {
        let (shown, hover) = match button {
            Button::Minimize => (Icon::Minimize, "Minimize"),
            Button::Maximize if maximized => (Icon::Restore, "Restore"),
            Button::Maximize => (Icon::Maximize, "Maximize"),
            Button::Close => (Icon::Close, "Close"),
        };
        let (rect, response) = ui.allocate_exact_size(Vec2::new(crate::frame::WINDOW_BUTTON, height), egui::Sense::click());
        if ui.is_rect_visible(rect) {
            let close = *button == Button::Close;
            let mut colour = ui.visuals().widgets.inactive.fg_stroke.color;
            if response.hovered() {
                let fill = if close { egui::Color32::from_rgb(0xc4, 0x2b, 0x1c) } else { ui.visuals().widgets.hovered.weak_bg_fill };
                ui.painter().rect_filled(rect, 0.0, fill);
                if close {
                    colour = egui::Color32::WHITE;
                }
            }
            icon::paint(ui.painter(), egui::Rect::from_center_size(rect.center(), Vec2::splat(10.0)), shown, egui::Stroke::new(1.0, colour));
        }
        if response.on_hover_text(hover).clicked() {
            let command = match button {
                Button::Minimize => egui::ViewportCommand::Minimized(true),
                Button::Maximize => egui::ViewportCommand::Maximized(!maximized),
                Button::Close => egui::ViewportCommand::Close,
            };
            ui.ctx().send_viewport_cmd(command);
        }
    }
    ui.spacing_mut().item_spacing.x = spacing;
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Rect, pos2};

    #[test]
    fn the_field_keeps_room_to_move_the_window_and_never_panics_on_rounding() {
        // The review found `clamp` panicking for about 0.7 % of these: lowest above highest by one ulp.
        let mut shown = 0;
        for left in 0..500 {
            for width in 0..600 {
                let left = 200.0 + left as f32 * 1.0013;
                let free = Rect::from_min_max(pos2(left, 0.0), pos2(left + width as f32 * 0.7731, 34.0));
                let Some(rect) = field_rect(free, 700.0) else {
                    assert!(free.width() - 2.0 * DRAG_ROOM < FIELD_MIN_WIDTH + 0.01, "{free:?}");
                    continue;
                };
                shown += 1;
                // From its centre and size: the width can move by rounding.
                assert!(rect.width() <= FIELD_WIDTH + 0.01 && rect.width() >= FIELD_MIN_WIDTH - 0.01, "{rect:?}");
                assert!(rect.left() >= free.left() + DRAG_ROOM - 0.01, "{free:?} {rect:?}");
                assert!(rect.right() <= free.right() - DRAG_ROOM + 0.01, "{free:?} {rect:?}");
            }
        }
        assert!(shown > 0);
    }

    #[test]
    fn the_field_is_centred_on_the_window_when_there_is_room() {
        let free = Rect::from_min_max(pos2(300.0, 0.0), pos2(1100.0, 34.0));
        assert_eq!(field_rect(free, 640.0).map(|rect| rect.center().x), Some(640.0));
        // Pushed right when the window's centre is too near the left of the free part.
        assert_eq!(field_rect(free, 310.0).map(|rect| rect.left()), Some(300.0 + DRAG_ROOM));
    }
}
