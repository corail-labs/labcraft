//! The workspaces of the top bar (ide.md 6): the ways of working on the same project, as data. Each entry has a name, a
//! state, one line on what it does and, when it is planned, its milestone and a link; a planned entry can never be
//! chosen, and a click on it opens its card. They show as a group of buttons, the planned ones greyed; in a bar too
//! narrow, as a selector with the list on a click.

use egui::{RichText, Sense, Vec2};

use crate::Planned;

/// A workspace, as the IDE ships the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Workspace {
    pub id: String,
    pub label: String,
    pub line: String,
    pub state: Availability,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Availability {
    Available,
    Planned { milestone: String, link: Option<String> },
}

impl Workspace {
    /// Its card, when it is planned.
    pub fn planned(&self) -> Option<Planned> {
        match &self.state {
            Availability::Available => None,
            Availability::Planned { milestone, link } => {
                Some(Planned { label: self.label.clone(), line: self.line.clone(), milestone: milestone.clone(), link: link.clone() })
            }
        }
    }
}

/// What the selector asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Asked {
    /// Change to this workspace.
    Choose(String),
    /// Show the card of a planned one.
    Planned(Planned),
}

/// The width of the group of `buttons` in `ui`.
pub fn buttons_width(ui: &egui::Ui, workspaces: &[Workspace]) -> f32 {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let labels: f32 = workspaces
        .iter()
        .map(|workspace| ui.painter().layout_no_wrap(workspace.label.clone(), font.clone(), egui::Color32::WHITE).size().x + 2.0 * PADDING)
        .sum();
    labels + GAP * workspaces.len().saturating_sub(1) as f32 + 2.0 * MARGIN
}

/// The room on each side of a label, between two buttons, and around the group.
const PADDING: f32 = 8.0;
const GAP: f32 = 2.0;
const MARGIN: f32 = 2.0;

/// The workspaces as a group of buttons, as the views of a tab: the current one on, the planned ones greyed; the hover
/// says what each one is.
pub fn buttons(ui: &mut egui::Ui, workspaces: &[Workspace], current: &str) -> Option<Asked> {
    let tokens = crate::theme::Tokens::of(ui.ctx());
    let font = egui::TextStyle::Button.resolve(ui.style());
    let mut asked = None;
    egui::Frame::new()
        .fill(tokens.sunken)
        .stroke(egui::Stroke::new(1.0, tokens.border))
        .corner_radius(egui::CornerRadius::same(crate::theme::RADIUS))
        .inner_margin(egui::Margin::same(MARGIN as i8))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            for workspace in workspaces {
                let planned = workspace.planned();
                let on = workspace.id == current;
                let colour = match (&planned, on) {
                    (Some(_), _) => tokens.text_weak.gamma_multiply(0.6),
                    (None, true) => tokens.text,
                    (None, false) => tokens.text_weak,
                };
                let galley = ui.painter().layout_no_wrap(workspace.label.clone(), font.clone(), colour);
                let size = Vec2::new(galley.size().x + 2.0 * PADDING, ui.spacing().interact_size.y - 2.0 * MARGIN);
                let (rect, response) = ui.allocate_exact_size(size, Sense::click());
                if ui.is_rect_visible(rect) {
                    let visuals = ui.visuals();
                    if on {
                        ui.painter().rect_filled(rect, visuals.widgets.active.corner_radius, visuals.selection.bg_fill);
                        let line = egui::Rect::from_min_max(
                            egui::pos2(rect.left() + 5.0, rect.bottom() - 2.0),
                            egui::pos2(rect.right() - 5.0, rect.bottom()),
                        );
                        ui.painter().rect_filled(line, 1.0, visuals.hyperlink_color);
                    } else if response.hovered() {
                        ui.painter().rect_filled(rect, visuals.widgets.hovered.corner_radius, visuals.widgets.hovered.weak_bg_fill);
                    }
                    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, colour);
                }
                let hover = match &planned {
                    None => format!("Workspace {}: {}", workspace.label, workspace.line),
                    Some(planned) => format!("Workspace {}: planned, {}. {}", workspace.label, planned.milestone, workspace.line),
                };
                response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, &hover));
                if response.on_hover_text(&hover).clicked() {
                    asked = Some(match planned {
                        None => Asked::Choose(workspace.id.clone()),
                        Some(planned) => Asked::Planned(planned),
                    });
                }
            }
        });
    asked.filter(|asked| *asked != Asked::Choose(current.to_owned()))
}

/// The selector, for a bar too narrow for the buttons: the current workspace, and the list on a click.
pub fn selector(ui: &mut egui::Ui, workspaces: &[Workspace], current: &str) -> Option<Asked> {
    let label = workspaces.iter().find(|workspace| workspace.id == current).map_or(current, |workspace| workspace.label.as_str());
    let mut asked = None;
    let shown = egui::ComboBox::from_id_salt("kit-shell-workspace").selected_text(label).width(96.0).show_ui(ui, |ui| {
        ui.set_min_width(300.0);
        for workspace in workspaces {
            let planned = workspace.planned();
            let response = ui
                .vertical(|ui| {
                    let title = match &planned {
                        None => RichText::new(&workspace.label),
                        Some(planned) => RichText::new(format!("{}  ·  Planned, {}", workspace.label, planned.milestone)).weak(),
                    };
                    let response = ui.selectable_label(workspace.id == current, title);
                    ui.label(RichText::new(&workspace.line).small().weak());
                    response
                })
                .inner;
            if response.clicked() {
                asked = Some(match planned {
                    None => Asked::Choose(workspace.id.clone()),
                    Some(planned) => Asked::Planned(planned),
                });
            }
        }
    });
    shown.response.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::ComboBox, true, format!("Workspace: {label}")));
    asked.filter(|asked| *asked != Asked::Choose(current.to_owned()))
}
