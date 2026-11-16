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
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 0.0);
        if let Some(layout) = bar.frame.layout() {
            window_buttons(ui, &layout.left);
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
            if let Some(layout) = bar.frame.layout() {
                window_buttons(ui, &layout.right.iter().rev().copied().collect::<Vec<_>>());
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
            ui.with_layout(egui::Layout::centered_and_justified(egui::Direction::LeftToRight), |ui| {
                let text = match (bar.project, bar.palette_key) {
                    (Some(project), Some(key)) => format!("{project}  —  search or run a command  ({key})"),
                    (Some(project), None) => format!("{project}  —  search or run a command"),
                    (None, Some(key)) => format!("Search or run a command  ({key})"),
                    (None, None) => "Search or run a command".to_owned(),
                };
                let field = egui::Button::new(RichText::new(text).weak()).min_size(Vec2::new(0.0, 22.0));
                if ui.add_sized([ui.available_width().min(460.0), 22.0], field).clicked() {
                    answers.push(Answer::OpenPalette);
                }
            });
        });
    });
    answers
}

fn hover(what: &str, detail: Option<&str>) -> String {
    match detail {
        Some(detail) => format!("{what}: {detail}"),
        None => format!("Nothing to {}", what.to_lowercase()),
    }
}

/// The buttons of the window, when the app draws its frame.
fn window_buttons(ui: &mut egui::Ui, buttons: &[Button]) {
    for button in buttons {
        let (shown, hover) = match button {
            Button::Minimize => (Icon::Minimize, "Minimize"),
            Button::Maximize => (Icon::Maximize, "Maximize"),
            Button::Close => (Icon::Close, "Close"),
        };
        if icon::button(ui, shown, true, hover).clicked() {
            let command = match button {
                Button::Minimize => egui::ViewportCommand::Minimized(true),
                Button::Maximize => {
                    let maximized = ui.input(|input| input.viewport().maximized).unwrap_or(false);
                    egui::ViewportCommand::Maximized(!maximized)
                }
                Button::Close => egui::ViewportCommand::Close,
            };
            ui.ctx().send_viewport_cmd(command);
        }
    }
}
