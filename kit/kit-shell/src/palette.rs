//! The command palette (ide.md 5): every command of the IDE by its label, with its key, searched as quick insertion
//! searches ([`crate::fuzzy`]); Enter runs the one chosen, and a command that needs parameters opens them in a small
//! form. Typed as an id followed by a JSON object, `node.add {"call": "core.ge"}`, it runs that command with those
//! parameters, and a badge says whether the object is valid before Enter. A command that cannot run now is listed,
//! greyed, with the reason; a planned one too, and choosing it opens its card ([`crate::Planned`]).
//!
//! The palette knows no command: the host gives the list each time it opens it, and runs what comes back.

use egui::{Color32, Key, RichText};
use serde_json::{Map, Value};

use crate::Planned;
use crate::fuzzy;

/// At most this many rows are listed *(proposed)*: the search narrows the rest.
pub const MAX_ROWS: usize = 200;

/// A command, as the palette lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: String,
    pub label: String,
    /// Its key, as the keymap shows it (`Ctrl+Shift+P`).
    pub key: Option<String>,
    pub state: State,
    /// Its parameters, in order.
    pub params: Vec<Param>,
}

impl Entry {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Entry {
        Entry { id: id.into(), label: label.into(), key: None, state: State::Enabled, params: Vec::new() }
    }
}

/// Whether a command can run now.
#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Enabled,
    /// Why not.
    Disabled(String),
    /// It comes with a milestone to come.
    Planned(Planned),
}

/// A parameter of a command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Param {
    pub name: String,
    pub required: bool,
    /// Its value is a text: the form takes it as it is typed, where the other values are JSON.
    pub text: bool,
}

/// What the user chose.
#[derive(Clone, Debug, PartialEq)]
pub enum Chosen {
    /// Run this command with these parameters.
    Run { id: String, params: Value },
    /// Show the card of a planned entry.
    Planned(Planned),
}

/// The form of a command's parameters.
#[derive(Clone, Debug, Default)]
struct Form {
    entry: usize,
    values: Vec<String>,
    problem: Option<String>,
}

/// The state of the palette between frames.
#[derive(Debug, Default)]
pub struct Palette {
    open: bool,
    query: String,
    /// The row under the arrows, among those listed.
    selected: usize,
    form: Option<Form>,
    /// The field takes the focus at the next frame.
    focus: bool,
}

/// What the query is: a search, or an id followed by a JSON object.
#[derive(Clone, Debug, PartialEq)]
pub enum Typed<'a> {
    Search(&'a str),
    /// The id, and the object or why it is not one.
    Call(&'a str, Result<Map<String, Value>, String>),
}

/// Reads the query: `id {…}` is a call, anything else a search.
pub fn read_query(query: &str) -> Typed<'_> {
    let query = query.trim();
    let Some(brace) = query.find('{') else { return Typed::Search(query) };
    let (id, object) = query.split_at(brace);
    let id = id.trim();
    if id.is_empty() || id.contains(char::is_whitespace) {
        return Typed::Search(query);
    }
    let parsed = match serde_json::from_str::<Value>(object) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err("not a JSON object".to_owned()),
        Err(error) => Err(format!("not JSON: {error}")),
    };
    Typed::Call(id, parsed)
}

/// Whether these parameters suit this command: each required one given, none it does not name.
pub fn judge(entry: &Entry, params: &Map<String, Value>) -> Result<(), String> {
    if let State::Disabled(why) = &entry.state {
        return Err(why.clone());
    }
    if let State::Planned(planned) = &entry.state {
        return Err(format!("planned: {}", planned.milestone));
    }
    if let Some(unknown) = params.keys().find(|name| !entry.params.iter().any(|param| param.name == **name)) {
        let known: Vec<&str> = entry.params.iter().map(|param| param.name.as_str()).collect();
        return Err(if known.is_empty() {
            format!("`{}` takes no parameter, not `{unknown}`", entry.id)
        } else {
            format!("`{}` has no parameter `{unknown}`: {}", entry.id, known.join(", "))
        });
    }
    if let Some(missing) = entry.params.iter().find(|param| param.required && !params.contains_key(&param.name)) {
        return Err(format!("`{}` is required", missing.name));
    }
    Ok(())
}

/// The parameters of a form: a text as typed, the rest as JSON; an empty field that is not required is left out.
fn form_params(entry: &Entry, values: &[String]) -> Result<Map<String, Value>, String> {
    let mut map = Map::new();
    for (param, value) in entry.params.iter().zip(values) {
        if value.is_empty() && !param.required {
            continue;
        }
        let value = if param.text {
            Value::String(value.clone())
        } else {
            serde_json::from_str(value).map_err(|error| format!("`{}` is not JSON: {error}", param.name))?
        };
        map.insert(param.name.clone(), value);
    }
    judge(entry, &map).map(|()| map)
}

impl Palette {
    /// Opens it, empty, the field in focus.
    pub fn open(&mut self) {
        *self = Palette { open: true, focus: true, ..Palette::default() };
    }

    /// Opens it with this text typed.
    pub fn open_with(&mut self, query: &str) {
        self.open();
        query.clone_into(&mut self.query);
    }

    pub fn close(&mut self) {
        self.open = false;
        self.form = None;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The rows listed for the query now, as indices into `entries`.
    pub fn rows(&self, entries: &[Entry]) -> Vec<usize> {
        match read_query(&self.query) {
            Typed::Search(search) => {
                let mut rows = fuzzy::rank(search, entries, |entry| vec![entry.label.as_str(), entry.id.as_str()]);
                rows.truncate(MAX_ROWS);
                rows
            }
            Typed::Call(id, _) => entries.iter().position(|entry| entry.id == id).into_iter().collect(),
        }
    }

    /// Shows it when it is open, over the window; what the user chose this frame.
    pub fn show(&mut self, ctx: &egui::Context, entries: &[Entry]) -> Option<Chosen> {
        if !self.open {
            return None;
        }
        let mut chosen = None;
        let modal = egui::Modal::new(egui::Id::unique("kit-shell-palette")).show(ctx, |ui| {
            ui.set_width(560.0_f32.min(ctx.content_rect().width() - 32.0));
            chosen = match self.form.take() {
                Some(form) => self.show_form(ui, entries, form),
                None => self.show_list(ui, entries),
            };
        });
        if modal.should_close() {
            self.close();
        }
        if chosen.is_some() {
            self.close();
        }
        chosen
    }

    fn show_list(&mut self, ui: &mut egui::Ui, entries: &[Entry]) -> Option<Chosen> {
        let field = ui.add(
            egui::TextEdit::singleline(&mut self.query).hint_text("Search a command, or type an id and a JSON object").desired_width(f32::INFINITY),
        );
        if std::mem::take(&mut self.focus) {
            field.request_focus();
        }
        if field.changed() {
            self.selected = 0;
        }
        let rows = self.rows(entries);
        let typed = read_query(&self.query);
        // The badge of a call typed as JSON.
        if let Typed::Call(id, object) = &typed {
            let verdict = match (entries.iter().find(|entry| entry.id == *id), object) {
                (None, _) => Err(format!("no command `{id}`")),
                (Some(_), Err(why)) => Err(why.clone()),
                (Some(entry), Ok(map)) => judge(entry, map),
            };
            let (text, color) = match &verdict {
                Ok(()) => ("valid: Enter runs it".to_owned(), ui.visuals().text_color()),
                Err(why) => (why.clone(), ui.visuals().warn_fg_color),
            };
            ui.label(RichText::new(text).color(color));
        }
        let (up, down, enter) = ui.input(|input| (input.key_pressed(Key::ArrowUp), input.key_pressed(Key::ArrowDown), input.key_pressed(Key::Enter)));
        if down {
            self.selected = (self.selected + 1).min(rows.len().saturating_sub(1));
        }
        if up {
            self.selected = self.selected.saturating_sub(1);
        }
        let mut clicked = None;
        egui::ScrollArea::vertical().max_height(360.0).auto_shrink([false, true]).show(ui, |ui| {
            for (at, &index) in rows.iter().enumerate() {
                let Some(entry) = entries.get(index) else { continue };
                let response = row(ui, entry, at == self.selected);
                if at == self.selected && (up || down) {
                    response.scroll_to_me(None);
                }
                if response.clicked() {
                    clicked = Some(at);
                }
            }
            if rows.is_empty() {
                ui.label(RichText::new("No command matches").weak());
            }
        });
        let at = clicked.or_else(|| enter.then_some(self.selected))?;
        let entry = rows.get(at).and_then(|&index| entries.get(index).map(|entry| (index, entry)));
        let (index, entry) = entry?;
        match (&entry.state, typed) {
            (State::Planned(planned), _) => Some(Chosen::Planned(planned.clone())),
            (State::Disabled(_), _) => None,
            (State::Enabled, Typed::Call(_, Ok(map))) if clicked.is_none() => {
                judge(entry, &map).ok().map(|()| Chosen::Run { id: entry.id.clone(), params: Value::Object(map) })
            }
            (State::Enabled, Typed::Call(..)) if clicked.is_none() => None,
            (State::Enabled, _) if entry.params.is_empty() => Some(Chosen::Run { id: entry.id.clone(), params: Value::Object(Map::new()) }),
            (State::Enabled, _) => {
                self.form = Some(Form { entry: index, values: vec![String::new(); entry.params.len()], problem: None });
                self.focus = true;
                None
            }
        }
    }

    fn show_form(&mut self, ui: &mut egui::Ui, entries: &[Entry], mut form: Form) -> Option<Chosen> {
        let entry = entries.get(form.entry)?;
        ui.label(RichText::new(&entry.label).strong());
        ui.label(RichText::new(&entry.id).monospace().weak());
        let mut first = true;
        egui::Grid::new("kit-shell-palette-form").num_columns(2).show(ui, |ui| {
            for (param, value) in entry.params.iter().zip(form.values.iter_mut()) {
                let name = if param.required { param.name.clone() } else { format!("{} (optional)", param.name) };
                ui.label(name);
                let hint = if param.text { "text" } else { "JSON" };
                let field = ui.add(egui::TextEdit::singleline(value).hint_text(hint).desired_width(320.0));
                if first && std::mem::take(&mut self.focus) {
                    field.request_focus();
                }
                first = false;
                ui.end_row();
            }
        });
        if let Some(problem) = &form.problem {
            ui.label(RichText::new(problem).color(ui.visuals().warn_fg_color));
        }
        let mut run = ui.input(|input| input.key_pressed(Key::Enter));
        ui.horizontal(|ui| {
            run |= ui.button("Run").clicked();
            if ui.button("Back").clicked() {
                self.focus = true;
                form.values.clear();
            }
        });
        if form.values.is_empty() {
            return None;
        }
        if run {
            match form_params(entry, &form.values) {
                Ok(map) => return Some(Chosen::Run { id: entry.id.clone(), params: Value::Object(map) }),
                Err(problem) => form.problem = Some(problem),
            }
        }
        self.form = Some(form);
        None
    }
}

/// A row of the list: the label, the id, the key; greyed with its reason when it cannot run.
fn row(ui: &mut egui::Ui, entry: &Entry, selected: bool) -> egui::Response {
    let weak = ui.visuals().weak_text_color();
    let (note, color): (Option<String>, Option<Color32>) = match &entry.state {
        State::Enabled => (None, None),
        State::Disabled(why) => (Some(why.clone()), Some(weak)),
        State::Planned(planned) => (Some(format!("Planned, {}", planned.milestone)), Some(weak)),
    };
    let response = ui
        .horizontal(|ui| {
            let mut label = RichText::new(&entry.label);
            if let Some(color) = color {
                label = label.color(color);
            }
            let response = ui.selectable_label(selected, label);
            ui.label(RichText::new(&entry.id).monospace().small().color(weak));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(key) = &entry.key {
                    ui.label(RichText::new(key).small().color(weak));
                }
                if let Some(note) = &note {
                    ui.label(RichText::new(note).small().italics().color(weak));
                }
            });
            response
        })
        .inner;
    if let State::Disabled(why) = &entry.state { response.on_hover_text(why.as_str()) } else { response }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn add() -> Entry {
        Entry {
            params: vec![
                Param { name: "call".to_owned(), required: true, text: true },
                Param { name: "at".to_owned(), required: false, text: false },
            ],
            ..Entry::new("node.add", "Add a node")
        }
    }

    #[test]
    fn an_id_and_an_object_is_a_call() {
        assert_eq!(
            read_query("node.add {\"call\": \"core.ge\"}"),
            Typed::Call("node.add", Ok(json!({"call": "core.ge"}).as_object().unwrap().clone()))
        );
        assert!(matches!(read_query("node.add {"), Typed::Call("node.add", Err(_))));
        assert!(matches!(read_query("node.add [1]"), Typed::Search(_)));
        assert!(matches!(read_query("node.add {1: 2}"), Typed::Call(_, Err(_))));
        assert!(matches!(read_query("open folder"), Typed::Search("open folder")));
        assert!(matches!(read_query("two words {}"), Typed::Search(_)));
    }

    #[test]
    fn the_badge_names_what_is_wrong() {
        let entry = add();
        assert_eq!(judge(&entry, json!({"call": "core.ge"}).as_object().unwrap()), Ok(()));
        assert_eq!(judge(&entry, &Map::new()), Err("`call` is required".to_owned()));
        let unknown = judge(&entry, json!({"call": "x", "colour": 1}).as_object().unwrap()).unwrap_err();
        assert!(unknown.contains("no parameter `colour`: call, at"), "{unknown}");
        let disabled = Entry { state: State::Disabled("open a project first".to_owned()), ..add() };
        assert_eq!(judge(&disabled, json!({"call": "x"}).as_object().unwrap()), Err("open a project first".to_owned()));
    }

    #[test]
    fn the_form_takes_a_text_as_typed_and_the_rest_as_json() {
        let entry = add();
        let map = form_params(&entry, &["core.ge".to_owned(), "[10, 20]".to_owned()]).unwrap();
        assert_eq!(Value::Object(map), json!({"call": "core.ge", "at": [10, 20]}));
        let map = form_params(&entry, &["core.ge".to_owned(), String::new()]).unwrap();
        assert_eq!(Value::Object(map), json!({"call": "core.ge"}));
        assert!(form_params(&entry, &["core.ge".to_owned(), "[10,".to_owned()]).unwrap_err().contains("`at` is not JSON"));
    }

    #[test]
    fn a_call_lists_its_command_alone() {
        let entries = vec![Entry::new("history.undo", "Undo"), add()];
        let mut palette = Palette::default();
        palette.open_with("node.add {}");
        assert_eq!(palette.rows(&entries), [1]);
        palette.open_with("undo");
        assert_eq!(palette.rows(&entries), [0]);
    }
}
