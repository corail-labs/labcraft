//! The inspector as a list of fields (ide.md 4 and 11): what the selection is, as sections of named values and a
//! documentation. A field that edits gives back what the user chose ([`Answer`]), and the host calls the command of the
//! same gesture (ide.md 4): an integer or a text typed (Enter or leaving the field), a choice, a button.

use egui::RichText;

/// A named value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub label: String,
    pub value: String,
    /// The value is code: shown in the monospace face.
    pub code: bool,
    /// What changes it, if it changes; `key` names it in the [`Answer`].
    pub control: Option<Control>,
}

/// What a field that edits shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Control {
    /// An integer, typed.
    Integer { key: String, value: i64 },
    /// One of a few choices, the one in force first among them by its place.
    Choice { key: String, options: Vec<String>, chosen: usize },
    /// A button, its text.
    Button { key: String, text: String },
    /// A text, typed: on one line, or on several (Ctrl+Enter writes it), with a hint shown while it is empty.
    Text { key: String, value: String, lines: bool, hint: String },
}

/// What the user did to a field that edits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    Integer { key: String, value: i64 },
    Choice { key: String, chosen: usize },
    Pressed { key: String },
    Text { key: String, value: String },
}

impl Field {
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Field {
        Field { label: label.into(), value: value.into(), code: false, control: None }
    }

    pub fn code(label: impl Into<String>, value: impl Into<String>) -> Field {
        Field { code: true, ..Field::new(label, value) }
    }

    /// An integer that the user types.
    pub fn integer(label: impl Into<String>, key: impl Into<String>, value: i64) -> Field {
        Field { control: Some(Control::Integer { key: key.into(), value }), ..Field::new(label, value.to_string()) }
    }

    /// A choice among `options`, `chosen` the one in force.
    pub fn choice(label: impl Into<String>, key: impl Into<String>, options: &[&str], chosen: usize) -> Field {
        let options: Vec<String> = options.iter().map(|option| (*option).to_owned()).collect();
        let value = options.get(chosen).cloned().unwrap_or_default();
        Field { control: Some(Control::Choice { key: key.into(), options, chosen }), ..Field::new(label, value) }
    }

    /// A text that the user types on one line; `hint` shows while it is empty.
    pub fn text(label: impl Into<String>, key: impl Into<String>, value: impl Into<String>, hint: impl Into<String>) -> Field {
        let value = value.into();
        Field { control: Some(Control::Text { key: key.into(), value: value.clone(), lines: false, hint: hint.into() }), ..Field::new(label, value) }
    }

    /// A text of several lines, a documentation.
    pub fn text_lines(label: impl Into<String>, key: impl Into<String>, value: impl Into<String>, hint: impl Into<String>) -> Field {
        let value = value.into();
        Field { control: Some(Control::Text { key: key.into(), value: value.clone(), lines: true, hint: hint.into() }), ..Field::new(label, value) }
    }

    /// A button.
    pub fn button(label: impl Into<String>, key: impl Into<String>, text: impl Into<String>) -> Field {
        let text = text.into();
        Field { control: Some(Control::Button { key: key.into(), text: text.clone() }), ..Field::new(label, text) }
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

/// Shows the inspector: its title (what is selected), then each section; what the user did to a field that edits.
pub fn show(ui: &mut egui::Ui, title: &str, sections: &[Section]) -> Option<Answer> {
    let mut answer = None;
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
                    match &field.control {
                        None => {
                            let value = if field.code { RichText::new(&field.value).monospace() } else { RichText::new(&field.value) };
                            ui.add(egui::Label::new(value).wrap());
                        }
                        Some(control) => {
                            if let Some(done) = edit(ui, control) {
                                answer = Some(done);
                            }
                        }
                    }
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
    answer
}

/// The text being typed into an integer field, kept by egui while the field has the focus.
#[derive(Clone, Default)]
struct Typed(String);

/// Where a field of the inspector says that it holds a text changed and not given yet, until the host takes it.
fn typing_id() -> egui::Id {
    egui::Id::unique("kit-shell-inspector-typing")
}

/// A field of the inspector held, since the last call, a text typed that differs from its value and that it has not
/// given yet: for a host that must know when the user is in the middle of an edit. The host calls it once in each
/// frame, after [`show`], in whichever window the inspector was drawn.
pub fn take_typing(ctx: &egui::Context) -> bool {
    ctx.data_mut(|data| data.remove_temp::<bool>(typing_id())).unwrap_or(false)
}

/// The field holds a text that differs from its value: said for [`take_typing`].
fn typed(ui: &egui::Ui) {
    ui.data_mut(|data| data.insert_temp(typing_id(), true));
}

/// A field that edits; what the user did to it.
fn edit(ui: &mut egui::Ui, control: &Control) -> Option<Answer> {
    match control {
        Control::Integer { key, value } => {
            let id = ui.make_persistent_id(("kit-shell-inspector-integer", key));
            let mut text = ui.data(|data| data.get_temp::<Typed>(id)).map_or_else(|| value.to_string(), |typed| typed.0);
            let response = ui.add(egui::TextEdit::singleline(&mut text).desired_width(72.0).font(egui::TextStyle::Monospace));
            if response.has_focus() {
                if text.trim() != value.to_string() {
                    typed(ui);
                }
                ui.data_mut(|data| data.insert_temp(id, Typed(text.clone())));
                return None;
            }
            ui.data_mut(|data| data.remove::<Typed>(id));
            // Enter, or leaving the field: a new integer is the answer; Escape, or anything else, goes back to the value.
            let escaped = ui.input(|input| input.key_pressed(egui::Key::Escape));
            let typed = text.trim().parse::<i64>().ok().filter(|typed| typed != value && !escaped);
            match (response.lost_focus(), typed) {
                (true, Some(typed)) => Some(Answer::Integer { key: key.clone(), value: typed }),
                _ => None,
            }
        }
        Control::Choice { key, options, chosen } => {
            let mut answer = None;
            ui.horizontal(|ui| {
                for (at, option) in options.iter().enumerate() {
                    if ui.selectable_label(at == *chosen, option).clicked() && at != *chosen {
                        answer = Some(Answer::Choice { key: key.clone(), chosen: at });
                    }
                }
            });
            answer
        }
        Control::Button { key, text } => ui.button(text).clicked().then(|| Answer::Pressed { key: key.clone() }),
        Control::Text { key, value, lines, hint } => {
            let id = ui.make_persistent_id(("kit-shell-inspector-text", key));
            let mut text = ui.data(|data| data.get_temp::<Typed>(id)).map_or_else(|| value.clone(), |typed| typed.0);
            let edit = if *lines {
                egui::TextEdit::multiline(&mut text).desired_rows(3).desired_width(f32::INFINITY)
            } else {
                egui::TextEdit::singleline(&mut text).desired_width(160.0).font(egui::TextStyle::Monospace)
            };
            let response = ui.add(edit.hint_text(hint.as_str()));
            // Ctrl+Enter writes a text of several lines, Enter one of one line.
            let written = *lines && response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter) && input.modifiers.command);
            if response.has_focus() && !written {
                if text.trim_end() != value.trim_end() {
                    typed(ui);
                }
                ui.data_mut(|data| data.insert_temp(id, Typed(text.clone())));
                return None;
            }
            ui.data_mut(|data| data.remove::<Typed>(id));
            if written {
                response.surrender_focus();
            }
            // Enter, or leaving the field: a new text is the answer; Escape goes back to the value.
            let escaped = ui.input(|input| input.key_pressed(egui::Key::Escape));
            let typed = Some(text.trim_end().to_owned()).filter(|typed| typed != value.trim_end() && !escaped);
            match (response.lost_focus() || written, typed) {
                (true, Some(typed)) => Some(Answer::Text { key: key.clone(), value: typed }),
                _ => None,
            }
        }
    }
}

/// Code longer than this many characters goes under its label, on the whole width.
const LONG_CODE: usize = 24;

fn long(field: &Field) -> bool {
    field.code && field.value.chars().count() > LONG_CODE
}
