//! The list of types of a terminal of the function (notation 2.4; decision of the owner, 2026-11-19): after the name of a
//! new terminal, or from "Type…", the types to choose from, searched as quick insertion searches, so that nobody types a
//! type. What is typed and is no entry of the list is offered as it is (`f64[mV]`); the command judges it.

use egui::RichText;
use serde_json::{Value, json};

/// The types of the language first (spec 3.1), the frequent ones at the top, then numbers with the units of an
/// instrument.
const BUILT_IN: &[&str] = &[
    "f64",
    "bool",
    "string",
    "i64",
    "error",
    "duration",
    "timestamp",
    "f64[V]",
    "f64[A]",
    "f64[s]",
    "f64[Hz]",
    "f64[Ω]",
    "f64[W]",
    "i32",
    "u8",
    "u16",
    "u32",
    "u64",
    "i8",
    "i16",
    "f32",
    "bytes",
    "c64",
    "c128",
];

/// The named units of v0 (spec 3.3), and whether they take a prefix.
const UNITS: &[(&str, bool)] = &[
    ("m", true),
    ("kg", false),
    ("s", true),
    ("A", true),
    ("K", true),
    ("mol", true),
    ("cd", true),
    ("g", true),
    ("Hz", true),
    ("N", true),
    ("Pa", true),
    ("J", true),
    ("W", true),
    ("C", true),
    ("V", true),
    ("F", true),
    ("Ω", true),
    ("S", true),
    ("Wb", true),
    ("T", true),
    ("H", true),
    ("min", false),
    ("h", false),
    ("°C", false),
    ("°F", false),
];

/// The prefixes of spec 3.3.
const PREFIXES: &[&str] = &["p", "n", "µ", "m", "k", "M", "G"];

/// At most this many units found by their name show (decision of the owner, 2026-11-19).
const UNITS_SHOWN: usize = 12;

/// Every unit that a type can carry without being composed: each named unit, and each with each prefix it takes.
fn all_units() -> Vec<String> {
    let mut found = Vec::new();
    for (unit, prefixed) in UNITS {
        found.push((*unit).to_owned());
        if *prefixed {
            found.extend(PREFIXES.iter().map(|prefix| format!("{prefix}{unit}")));
        }
    }
    found
}

/// What is typed as a unit is spelt: `ohm` is `Ω`, `deg` is `°`, and a `u` before a unit is `µ`, so that a keyboard
/// without them finds them.
fn spelt(query: &str) -> String {
    let mut text = query.replace("ohm", "Ω").replace("Ohm", "Ω").replace("deg", "°");
    if let Some(rest) = text.strip_prefix('u')
        && !rest.is_empty()
    {
        text = format!("µ{rest}");
    }
    text
}

/// The units that what is typed names (decision of the owner, 2026-11-19): the one it spells first, then those that start
/// with it, as typed, then whatever its case; at most [`UNITS_SHOWN`].
fn units_for(query: &str) -> Vec<String> {
    let query = spelt(query.trim());
    if query.is_empty() || query.contains(['[', ']', ' ']) {
        return Vec::new();
    }
    let units = all_units();
    let lower = query.to_lowercase();
    let mut found: Vec<String> = units.iter().filter(|unit| **unit == query).cloned().collect();
    found.extend(units.iter().filter(|unit| unit.starts_with(&query) && **unit != query).cloned());
    found.extend(units.iter().filter(|unit| !unit.starts_with(&query) && unit.to_lowercase().starts_with(&lower)).cloned());
    found.dedup();
    found.truncate(UNITS_SHOWN);
    found
}

/// At most this many rows show.
const SHOWN: usize = 40;

/// What the type chosen is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Purpose {
    /// `signature.add` of `terminal`, with its default for an input.
    Add { default: Option<String> },
    /// `signature.set_type` of `terminal`.
    Set,
}

/// The list open, over the window.
pub(crate) struct TypeList {
    pub function: String,
    file: String,
    /// `input x` or `output x`.
    terminal: String,
    purpose: Purpose,
    /// The types of the project and of its libraries (`Verdict`, `visa.session`), after those of the language.
    project: Vec<String>,
    at: egui::Pos2,
    query: String,
    chosen: usize,
    focused: bool,
}

/// How the list closed.
pub(crate) enum Closed {
    Open,
    Cancelled,
    /// The command to send, and its parameters.
    Picked(&'static str, Value),
}

impl TypeList {
    pub fn new(function: String, file: String, terminal: String, purpose: Purpose, project: Vec<String>, at: egui::Pos2) -> TypeList {
        TypeList { function, file, terminal, purpose, project, at, query: String::new(), chosen: 0, focused: false }
    }

    /// The rows for the query: the types that hold it, those of the language first; what is typed, when no row is it.
    fn rows(&self) -> Vec<String> {
        let query = self.query.trim().to_lowercase();
        let units: Vec<String> = units_for(&self.query).into_iter().map(|unit| format!("f64[{unit}]")).collect();
        // A unit spelt exactly comes first: `mV` gives `f64[mV]`.
        let spelt_exactly = format!("f64[{}]", spelt(self.query.trim()));
        let mut rows: Vec<String> = units.iter().filter(|ty| **ty == spelt_exactly).cloned().collect();
        for ty in BUILT_IN.iter().map(|ty| (*ty).to_owned()).chain(self.project.iter().cloned()).chain(units.iter().cloned()) {
            if (query.is_empty() || ty.to_lowercase().contains(&query) || units.contains(&ty)) && !rows.contains(&ty) {
                rows.push(ty);
            }
        }
        let typed = self.query.trim();
        if !typed.is_empty() && !rows.iter().any(|row| row == typed) {
            rows.push(typed.to_owned());
        }
        rows
    }

    /// The command that the type `ty` asks for.
    fn command(&self, ty: &str) -> (&'static str, Value) {
        match &self.purpose {
            Purpose::Add { default } => {
                let mut params = json!({ "file": self.file, "terminal": self.terminal, "type": ty });
                if let (Some(default), Some(object)) = (default, params.as_object_mut()) {
                    object.insert("default".to_owned(), json!(default));
                }
                ("signature.add", params)
            }
            Purpose::Set => ("signature.set_type", json!({ "file": self.file, "terminal": self.terminal, "type": ty })),
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Closed {
        let rows = self.rows();
        let (down, up, enter, escape) = ctx.input_mut(|input| {
            (
                input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                input.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                input.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
            )
        });
        if escape {
            return Closed::Cancelled;
        }
        if down && !rows.is_empty() {
            self.chosen = (self.chosen + 1) % rows.len().min(SHOWN);
        }
        if up && !rows.is_empty() {
            self.chosen = self.chosen.checked_sub(1).unwrap_or(rows.len().min(SHOWN) - 1);
        }
        self.chosen = self.chosen.min(rows.len().saturating_sub(1));
        let mut clicked = None;
        let title = match self.purpose {
            Purpose::Add { .. } => format!("The type of {}", self.terminal),
            Purpose::Set => format!("A new type for {}", self.terminal),
        };
        let area = egui::Area::new(egui::Id::unique("labcraft-type-list")).order(egui::Order::Foreground).fixed_pos(self.at).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_width(260.0);
                ui.label(RichText::new(title).small().weak());
                let before = self.query.clone();
                let field = ui.add(egui::TextEdit::singleline(&mut self.query).hint_text("Search a type").desired_width(f32::INFINITY));
                if !self.focused {
                    field.request_focus();
                    self.focused = true;
                }
                if self.query != before {
                    self.chosen = 0;
                }
                egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                    for (at, row) in rows.iter().take(SHOWN).enumerate() {
                        let response = ui.selectable_label(at == self.chosen, RichText::new(row).monospace());
                        if at == self.chosen && (down || up) {
                            response.scroll_to_me(None);
                        }
                        if response.clicked() {
                            clicked = Some(at);
                        }
                    }
                });
            });
        });
        let picked = clicked.or(enter.then_some(self.chosen)).and_then(|at| rows.get(at));
        if let Some(ty) = picked {
            let (id, params) = self.command(ty);
            return Closed::Picked(id, params);
        }
        let clicked_elsewhere = ctx.input(|input| input.pointer.any_click().then(|| input.pointer.interact_pos()).flatten());
        if clicked_elsewhere.is_some_and(|at| !area.response.rect.contains(at)) {
            return Closed::Cancelled;
        }
        Closed::Open
    }
}

/// Whether the text of a new terminal gives its type: `name: type`, before any `=`.
pub(crate) fn has_type(text: &str) -> bool {
    text.split('=').next().is_some_and(|head| head.contains(':'))
}

/// `name` or `name = default` as a field gives it, with no type: the name, and the default.
pub(crate) fn name_and_default(text: &str) -> (String, Option<String>) {
    match text.split_once('=') {
        Some((name, default)) => (name.trim().to_owned(), Some(default.trim().to_owned()).filter(|default| !default.is_empty())),
        None => (text.trim().to_owned(), None),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Purpose, TypeList, name_and_default};

    fn list(purpose: Purpose) -> TypeList {
        TypeList::new("main".to_owned(), "main.lcf".to_owned(), "input gain".to_owned(), purpose, vec!["Verdict".to_owned()], egui::Pos2::ZERO)
    }

    /// The rows, written by hand: the types that hold what is typed, those of the language first, then what is typed.
    #[test]
    fn the_rows_hold_what_is_typed() {
        let mut types = list(Purpose::Set);
        types.query = "verd".to_owned();
        assert_eq!(types.rows(), ["Verdict", "verd"]);
        types.query = "f64[V]".to_owned();
        assert_eq!(types.rows(), ["f64[V]"]);
        types.query = "f64[mV]".to_owned();
        assert_eq!(types.rows(), ["f64[mV]"]);
        types.query = String::new();
        assert_eq!(types.rows().first().map(String::as_str), Some("f64"));
        // A unit by its name (spec 3.3; decision of the owner, 2026-11-19).
        types.query = "mV".to_owned();
        assert_eq!(types.rows().first().map(String::as_str), Some("f64[mV]"));
        types.query = "kohm".to_owned();
        assert_eq!(types.rows().first().map(String::as_str), Some("f64[kΩ]"));
        types.query = "uA".to_owned();
        assert_eq!(types.rows().first().map(String::as_str), Some("f64[µA]"));
        types.query = "degC".to_owned();
        assert_eq!(types.rows().first().map(String::as_str), Some("f64[°C]"));
        types.query = "kg".to_owned();
        assert_eq!(types.rows().first().map(String::as_str), Some("f64[kg]"));
        types.query = String::new();
        assert_eq!(types.rows().last().map(String::as_str), Some("Verdict"));
    }

    #[test]
    fn a_type_chosen_sends_its_command() {
        let add = list(Purpose::Add { default: Some("2".to_owned()) });
        assert_eq!(add.command("f64"), ("signature.add", json!({ "file": "main.lcf", "terminal": "input gain", "type": "f64", "default": "2" })));
        assert_eq!(
            list(Purpose::Set).command("bool"),
            ("signature.set_type", json!({ "file": "main.lcf", "terminal": "input gain", "type": "bool" }))
        );
        assert_eq!(name_and_default("gain = 2"), ("gain".to_owned(), Some("2".to_owned())));
        assert_eq!(name_and_default(" gain "), ("gain".to_owned(), None));
        assert!(super::has_type("gain: f64 = 2") && !super::has_type("gain = \"a:b\"") && !super::has_type("gain"));
    }
}
