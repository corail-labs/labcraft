//! The `Settings` dialog (ide.md 2; decision of the owner, 2026-11-20): modal over the window, the keys of commands.md
//! 9.7 by group, a field that keeps those whose name or help holds what is typed, the store it writes (`User` or
//! `Project`), and for each key its control, its help, where its value comes from and `Reset`. It writes nothing
//! itself: it says what to set, and the window sets it, by `settings.set` with a project, in its settings without one.

use egui::RichText;
use labcraft_engine::settings::{From, GROUPS, KEYS, Key, Kind, LIBRARY_COLORS, Stores};
use serde_json::{Map, Value, json};

/// The dialog, open or not.
#[derive(Debug, Default)]
pub struct Dialog {
    open: bool,
    search: String,
    /// The store it writes: the project's, else the user's.
    project: bool,
    /// The colour of a library being picked, sent once the pointer lets go: a drag in the picker is one write.
    picking: Option<(String, [u8; 3])>,
}

/// A setting to write: `value` null takes the key out of the store.
#[derive(Clone, Debug, PartialEq)]
pub struct Set {
    pub key: &'static str,
    pub value: Value,
    pub project: bool,
}

impl Dialog {
    pub fn open(&mut self) {
        self.open = true;
        self.search.clear();
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Shows the dialog when it is open: what to set. `project`: a project is open, whose store can be written;
    /// `libraries`: those that `Library colors` lists.
    pub fn show(&mut self, ctx: &egui::Context, stores: &Stores, project: bool, libraries: &[String]) -> Vec<Set> {
        let mut asked = Vec::new();
        if !self.open {
            return asked;
        }
        if !project {
            self.project = false;
        }
        let mut close = false;
        let height = (ctx.content_rect().height() * 0.8).max(240.0);
        let modal = egui::Modal::new(egui::Id::unique("ide-settings")).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Settings").strong().size(16.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if project {
                        ui.selectable_value(&mut self.project, true, "Project")
                            .on_hover_text("Writes `.labcraft/config.toml`, versioned with the project");
                        ui.selectable_value(&mut self.project, false, "User").on_hover_text("Writes the settings of this computer");
                    } else {
                        ui.label(RichText::new("User").weak()).on_hover_text("Open a project to write its settings");
                    }
                });
            });
            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Search settings").desired_width(f32::INFINITY));
            ui.add_space(4.0);
            let search = self.search.to_lowercase();
            // A fixed height: a modal whose size changes moves under the pointer between two frames.
            egui::ScrollArea::vertical().max_height(height - 120.0).min_scrolled_height(height - 120.0).auto_shrink([false, false]).show(ui, |ui| {
                let mut any = false;
                for (group, title) in GROUPS {
                    let keys: Vec<&Key> = KEYS.iter().filter(|key| key.group() == *group && matches(key, &search)).collect();
                    if keys.is_empty() {
                        continue;
                    }
                    any = true;
                    ui.add_space(6.0);
                    ui.label(RichText::new(*title).strong());
                    ui.separator();
                    for key in keys {
                        self.row(ui, key, stores, libraries, &mut asked);
                        ui.add_space(4.0);
                    }
                }
                if !any {
                    ui.label(RichText::new("No setting holds what is typed").weak());
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                close = ui.button("Close").clicked();
            });
        });
        if close || modal.should_close() {
            self.open = false;
            self.picking = None;
        }
        asked
    }

    fn row(&mut self, ui: &mut egui::Ui, key: &'static Key, stores: &Stores, libraries: &[String], asked: &mut Vec<Set>) {
        // In `User`, the value of the user's store, which a key of the project would hide; in `Project`, the one in force.
        let (value, from) = match stores.user.get(key.id) {
            Some(value) if !self.project => (value.clone(), From::User),
            None if !self.project => (key.default_value(), From::Default),
            _ => stores.get(key),
        };
        let writable = !self.project || key.project;
        let held = if self.project { stores.project.contains_key(key.id) } else { stores.user.contains_key(key.id) };
        let project = self.project;
        let set = |value| Set { key: key.id, value, project };
        ui.add_enabled_ui(writable, |ui| {
            ui.horizontal(|ui| {
                match key.kind {
                    Kind::Bool(_) => {
                        let mut on = value.as_bool().unwrap_or_default();
                        if ui.checkbox(&mut on, key.title).changed() {
                            asked.push(set(json!(on)));
                        }
                    }
                    Kind::Choice(names, _) => {
                        ui.label(key.title);
                        for name in names {
                            if ui.selectable_label(value.as_str() == Some(*name), crate::block::capital(name)).clicked()
                                && value.as_str() != Some(*name)
                            {
                                asked.push(set(json!(name)));
                            }
                        }
                    }
                    Kind::LibraryColors => {
                        ui.label(key.title);
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !writable {
                        ui.label(RichText::new("User only").weak().small());
                    } else if held && key.kind != Kind::LibraryColors && ui.small_button("Reset").clicked() {
                        asked.push(set(Value::Null));
                    }
                    ui.label(RichText::new(from_text(from)).weak().small());
                });
            });
            ui.label(RichText::new(key.help).weak().small());
            if key.kind == Kind::LibraryColors {
                self.library_colors(ui, &value, libraries, asked);
            }
        });
    }

    /// One row per library: the colour the user picked, or "From the code" (commands.md 9.7).
    fn library_colors(&mut self, ui: &mut egui::Ui, value: &Value, libraries: &[String], asked: &mut Vec<Set>) {
        let given = value.as_object().cloned().unwrap_or_default();
        let mut names: Vec<String> = libraries.to_vec();
        names.extend(given.keys().filter(|name| !libraries.contains(name)).cloned());
        if names.is_empty() {
            ui.label(RichText::new("No library to call yet").weak());
            return;
        }
        let pressed = ui.input(|input| input.pointer.any_down());
        for name in names {
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                ui.label(&name);
                let written = given.get(&name).and_then(Value::as_str).and_then(rgb);
                let mut colour = match &self.picking {
                    Some((picking, colour)) if *picking == name => *colour,
                    _ => written.unwrap_or([0x64, 0x74, 0x8B]),
                };
                if egui::color_picker::color_edit_button_srgb(ui, &mut colour).changed() {
                    self.picking = Some((name.clone(), colour));
                }
                match written {
                    Some(_) => {
                        if ui.small_button("Reset").clicked() {
                            let mut libraries = given.clone();
                            libraries.remove(&name);
                            asked.push(colours(libraries));
                        }
                    }
                    None => {
                        ui.label(RichText::new("From the code").weak().small());
                    }
                }
            });
        }
        if !pressed && let Some((name, colour)) = self.picking.take() {
            let mut libraries = given;
            libraries.insert(name, json!(hex(colour)));
            asked.push(colours(libraries));
        }
    }
}

/// `appearance.library_colors` as `libraries` hold it: none left takes the key out.
fn colours(libraries: Map<String, Value>) -> Set {
    let value = if libraries.is_empty() { Value::Null } else { Value::Object(libraries) };
    Set { key: LIBRARY_COLORS, value, project: false }
}

fn matches(key: &Key, search: &str) -> bool {
    search.is_empty() || [key.title, key.help, key.id].iter().any(|text| text.to_lowercase().contains(search))
}

fn from_text(from: From) -> &'static str {
    match from {
        From::Project => "from the project",
        From::User => "from the user",
        From::Default => "default",
    }
}

/// `#RRGGBB`, in either case, as the key holds it.
fn rgb(text: &str) -> Option<[u8; 3]> {
    labcraft_diagram::Rgb::parse(text).map(|colour| colour.0)
}

fn hex([red, green, blue]: [u8; 3]) -> String {
    format!("#{red:02X}{green:02X}{blue:02X}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_search_keeps_a_key_by_its_title_its_help_or_its_key() {
        let key = labcraft_engine::settings::find(labcraft_engine::settings::DRAG_PANS).unwrap();
        assert!(matches(key, ""));
        assert!(matches(key, "pans"));
        assert!(matches(key, "rectangle"));
        assert!(matches(key, "editor.drag"));
        assert!(!matches(key, "theme"));
    }

    #[test]
    fn the_last_library_reset_takes_the_key_out() {
        assert_eq!(colours(Map::new()), Set { key: LIBRARY_COLORS, value: Value::Null, project: false });
        assert_eq!(hex([0x25, 0x63, 0xEB]), "#2563EB");
        assert_eq!(rgb("#2563eb"), Some([0x25, 0x63, 0xEB]));
    }
}
