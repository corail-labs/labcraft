//! The inspector as a list of fields (ide.md 4 and 11): what the selection is, as sections of named values and a
//! documentation. Read-only here; a field that edits calls a command, which the host gives when it has one.

use egui::RichText;

/// A named value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub label: String,
    pub value: String,
    /// The value is code: shown in the monospace face.
    pub code: bool,
}

impl Field {
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Field {
        Field { label: label.into(), value: value.into(), code: false }
    }

    pub fn code(label: impl Into<String>, value: impl Into<String>) -> Field {
        Field { code: true, ..Field::new(label, value) }
    }
}

/// A part of what the inspector shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Section {
    pub title: Option<String>,
    pub fields: Vec<Field>,
    /// Text under the fields: a documentation.
    pub text: Option<String>,
}

/// Shows the inspector: its title (what is selected), then each section.
pub fn show(ui: &mut egui::Ui, title: &str, sections: &[Section]) {
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.label(RichText::new(title).strong());
        for (at, section) in sections.iter().enumerate() {
            ui.add_space(6.0);
            if let Some(title) = &section.title {
                ui.label(RichText::new(title).small().weak());
            }
            egui::Grid::new(("kit-shell-inspector", at)).num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
                for field in &section.fields {
                    ui.label(RichText::new(&field.label).weak());
                    let value = if field.code { RichText::new(&field.value).monospace() } else { RichText::new(&field.value) };
                    ui.add(egui::Label::new(value).wrap());
                    ui.end_row();
                }
            });
            if let Some(text) = &section.text {
                ui.add(egui::Label::new(text.as_str()).wrap());
            }
        }
    });
}
