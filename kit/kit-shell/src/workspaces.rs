//! The workspace selector of the top bar (ide.md 6): the ways of working on the same project, as data. Each entry has a
//! name, a state, one line on what it does and, when it is planned, its milestone and a link; a planned entry can never
//! be chosen, and a click on it opens its card.

use egui::RichText;

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

/// The selector: the current workspace, and the list on a click.
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
