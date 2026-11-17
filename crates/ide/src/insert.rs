//! Quick insertion (notation 2.6): the list opened where the work is, one catalog with the panel `Palette`, and the
//! commands that a choice makes, one step of undo. The node and its wire come in one `commands.batch`, the wire naming
//! the node by `$0` (commands.md 6.7; decision of the owner, 2026-11-17).

use egui::RichText;
use labcraft_editor::insertion::{Catalog, Entry, EntryKind, Row};
use labcraft_editor::{Quick, Source};
use serde_json::{Map, Value, json};

/// The list of quick insertion, open on a function.
pub struct QuickList {
    pub function: String,
    pub file: String,
    pub quick: Quick,
    pub query: String,
    /// The row chosen with the arrows.
    pub chosen: usize,
    /// The canvas that had the keyboard, which gets it back when the list closes.
    pub canvas: Option<egui::Id>,
    focused: bool,
}

/// What a row inserts.
#[derive(Clone, Debug, PartialEq)]
pub enum Picked {
    /// A constant: typed text that reads as one, or a variant without values (`constant.add`).
    Constant(String),
    Entry(Entry),
}

/// What the list says after a frame.
pub enum Closed {
    Open,
    Cancelled,
    Picked(Picked),
}

/// The rows the list shows at most *(proposed)*: the search narrows them.
const SHOWN: usize = 200;

impl QuickList {
    pub fn new(function: String, file: String, quick: Quick, canvas: Option<egui::Id>) -> QuickList {
        QuickList { function, file, quick, query: String::new(), chosen: 0, canvas, focused: false }
    }

    /// The rows for the search, in their order: for a wire to go into, only what goes into it.
    pub fn rows(&self, catalog: &Catalog, recent: &[String]) -> Vec<Row> {
        match &self.quick.source {
            Source::Nothing => catalog.rows(&self.query, None, recent),
            Source::Terminal { wanted, .. } | Source::Node { wanted, .. } => catalog.rows(&self.query, Some(wanted), recent),
            Source::Wire { wanted, .. } => catalog
                .rows(&self.query, None, recent)
                .into_iter()
                .filter(|row| match row {
                    Row::Entry(index) => catalog.entries.get(*index).is_some_and(|entry| catalog.goes_into(entry, wanted)),
                    Row::Typed(_) => false,
                })
                .collect(),
        }
    }

    /// Draws the list where quick insertion opened: a search field, then the rows, each with its signature and its
    /// library. The arrows move in the list, Enter inserts, Escape or a click elsewhere closes it.
    pub fn show(&mut self, ctx: &egui::Context, catalog: &Catalog, recent: &[String]) -> Closed {
        let rows = self.rows(catalog, recent);
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
        let count = rows.len().min(SHOWN);
        if down && count > 0 {
            self.chosen = (self.chosen + 1) % count;
        }
        if up && count > 0 {
            self.chosen = self.chosen.checked_sub(1).unwrap_or(count - 1);
        }
        self.chosen = self.chosen.min(count.saturating_sub(1));
        let mut clicked = None;
        let area = egui::Area::new(egui::Id::unique("labcraft-quick-insertion")).order(egui::Order::Foreground).fixed_pos(self.quick.screen).show(
            ctx,
            |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(380.0);
                    let before = self.query.clone();
                    let field = ui.add(egui::TextEdit::singleline(&mut self.query).hint_text(hint(&self.quick.source)).desired_width(f32::INFINITY));
                    if !self.focused {
                        field.request_focus();
                        self.focused = true;
                    }
                    if self.query != before {
                        self.chosen = 0;
                    }
                    egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                        if rows.is_empty() {
                            ui.label(RichText::new(nothing(&self.quick.source)).weak());
                        }
                        for (at, row) in rows.iter().take(SHOWN).enumerate() {
                            let (title, detail) = match row {
                                Row::Typed(text) => (format!("constant {text}"), String::new()),
                                Row::Entry(index) => match catalog.entries.get(*index) {
                                    Some(entry) => (entry.call.clone(), entry.signature.clone()),
                                    None => continue,
                                },
                            };
                            let response = ui.selectable_label(at == self.chosen, RichText::new(title).monospace());
                            let response = if detail.is_empty() { response } else { response.on_hover_text(&detail) };
                            if at == self.chosen && (down || up) {
                                response.scroll_to_me(None);
                            }
                            if response.clicked() {
                                clicked = Some(at);
                            }
                        }
                    });
                });
            },
        );
        let picked = clicked.or(enter.then_some(self.chosen));
        if let Some(row) = picked.and_then(|at| rows.get(at)) {
            return match row {
                Row::Typed(text) => Closed::Picked(Picked::Constant(text.clone())),
                Row::Entry(index) => match catalog.entries.get(*index) {
                    Some(entry) if entry.kind == EntryKind::Constant => Closed::Picked(Picked::Constant(entry.call.clone())),
                    Some(entry) => Closed::Picked(Picked::Entry(entry.clone())),
                    None => Closed::Cancelled,
                },
            };
        }
        let clicked_elsewhere = ctx.input(|input| input.pointer.any_click().then(|| input.pointer.interact_pos()).flatten());
        if clicked_elsewhere.is_some_and(|at| !area.response.rect.contains(at)) {
            return Closed::Cancelled;
        }
        Closed::Open
    }
}

/// What the list says when nothing is in it.
fn nothing(source: &Source) -> &'static str {
    match source {
        Source::Nothing => "Nothing matches",
        Source::Terminal { wanted, .. } if !wanted.from_output => "Nothing here gives this input its value",
        Source::Terminal { .. } | Source::Node { .. } => "Nothing here takes this value",
        Source::Wire { .. } => "Nothing here goes into this wire",
    }
}

/// What the search field says while it is empty.
fn hint(source: &Source) -> &'static str {
    match source {
        Source::Nothing => "Insert a function, a type or a constant",
        Source::Terminal { wanted, .. } if !wanted.from_output => "What gives this input its value",
        Source::Terminal { .. } | Source::Node { .. } => "What takes this value",
        Source::Wire { .. } => "What goes into this wire",
    }
}

/// The command that a choice makes, `(id, params)`: one `node.add` or `constant.add`, or a `commands.batch` of it and
/// its wire, one step (notation 2.6).
pub fn command(file: &str, quick: &Quick, picked: &Picked, catalog: &Catalog) -> (String, Value) {
    let mut add = Map::new();
    add.insert("file".to_owned(), json!(file));
    let (id, entry) = match picked {
        Picked::Constant(value) => {
            add.insert("value".to_owned(), json!(value));
            ("constant.add", None)
        }
        Picked::Entry(entry) => {
            add.insert("call".to_owned(), json!(entry.call));
            ("node.add", Some(entry))
        }
    };
    if let Some((x, y)) = quick.at {
        add.insert("at".to_owned(), json!([x, y]));
    }
    let wire = match (&quick.source, entry) {
        (Source::Wire { name, .. }, Some(_)) => {
            add.insert("into".to_owned(), json!(name));
            None
        }
        (Source::Terminal { name, wanted, .. } | Source::Node { name, wanted }, Some(entry)) if wanted.from_output => {
            catalog.fits(entry, wanted).map(|at| json!({ "file": file, "from": name, "to": input_of(entry, at) }))
        }
        (Source::Terminal { name, wanted, replace }, Some(entry)) => {
            catalog.fits(entry, wanted).map(|at| json!({ "file": file, "from": output_of(entry, at), "to": name, "replace": replace }))
        }
        // A constant drawn back from an input.
        (Source::Terminal { name, wanted, replace }, None) if !wanted.from_output => {
            Some(json!({ "file": file, "from": "$0", "to": name, "replace": replace }))
        }
        _ => None,
    };
    match wire {
        None => (id.to_owned(), Value::Object(add)),
        Some(wire) => {
            let commands = json!([{ "id": id, "params": Value::Object(add) }, { "id": "wire.connect", "params": wire }]);
            ("commands.batch".to_owned(), json!({ "commands": commands, "label": "Insert Node" }))
        }
    }
}

/// The input `at` of the new node, as `wire.connect` names it through `$0`: `$0.a`, or for a value of a variant, which
/// has no name, `$0` when it is alone and `$0.1` otherwise.
fn input_of(entry: &Entry, at: usize) -> String {
    match entry.inputs.get(at) {
        Some(input) if !input.name.is_empty() => format!("$0.{}", input.name),
        _ if entry.inputs.iter().filter(|input| input.name.is_empty()).count() == 1 => "$0".to_owned(),
        _ => format!("$0.{at}"),
    }
}

/// The output `at` of the new node: its first output is the node itself, the others `$0.err`.
fn output_of(entry: &Entry, at: usize) -> String {
    match entry.outputs.get(at) {
        Some(output) if at > 0 => format!("$0.{}", output.name),
        _ => "$0".to_owned(),
    }
}
