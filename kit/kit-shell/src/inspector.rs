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
            egui::Grid::new(("kit-shell-inspector", at)).num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                for field in section.fields.iter().filter(|field| !long(field)) {
                    ui.label(RichText::new(&field.label).weak());
                    let value = if field.code { RichText::new(&field.value).monospace() } else { RichText::new(&field.value) };
                    ui.add(egui::Label::new(value).wrap());
                    ui.end_row();
                }
            });
            // Long code takes the whole width, under its label: a signature cut into a narrow column reads badly.
            for field in section.fields.iter().filter(|field| long(field)) {
                ui.add_space(4.0);
                ui.label(RichText::new(&field.label).weak());
                egui::Frame::new()
                    .fill(ui.visuals().extreme_bg_color)
                    .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                    .corner_radius(ui.visuals().widgets.noninteractive.corner_radius)
                    .inner_margin(egui::Margin::symmetric(8, 6))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.add(egui::Label::new(RichText::new(&field.value).monospace()).wrap());
                    });
            }
            if let Some(text) = &section.text {
                ui.add_space(4.0);
                ui.add(egui::Label::new(text.as_str()).wrap());
            }
        }
    });
}

/// Code longer than this many characters goes under its label, on the whole width.
const LONG_CODE: usize = 24;

fn long(field: &Field) -> bool {
    field.code && field.value.chars().count() > LONG_CODE
}
