//! The value that a structure needs, asked for when it is made (notation 2.6; decision of the owner, 2026-11-18): the
//! selector of a `match` or a `handle`, what a `for` goes over. A list as quick insertion's, of the values that the frame
//! sees and that the kind takes, those that the nodes surrounded read first; Enter makes the structure, Escape writes
//! nothing.

use egui::RichText;
use labcraft_editor::{InFrame, Seen};
use serde_json::{Value, json};

/// What the structure surrounds: nodes of one frame, or nothing, an empty one placed in a frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Around {
    Nodes(Vec<String>),
    Empty {
        at: (i64, i64),
        within: InFrame,
    },
    /// "Add clause…" of a `for`, as a command names it: the value goes over in a new `in` clause (notation 2.6).
    Clause {
        structure: String,
    },
}

/// The list of values, open on a function.
pub struct ValueList {
    pub function: String,
    pub file: String,
    pub kind: &'static str,
    pub around: Around,
    pub screen: egui::Pos2,
    pub values: Vec<Seen>,
    pub query: String,
    pub chosen: usize,
    /// The canvas that had the keyboard, which gets it back when the list closes.
    pub canvas: Option<egui::Id>,
    focused: bool,
}

/// The rows the list shows at most *(proposed)*, as quick insertion's: the search narrows them.
const SHOWN: usize = 200;

/// What the list says after a frame.
pub enum Chosen {
    Open,
    Cancelled,
    /// The value, or for a `for` the range typed (`0..n`).
    Value(String),
}

impl ValueList {
    pub fn new(
        function: String,
        file: String,
        kind: &'static str,
        around: Around,
        screen: egui::Pos2,
        values: Vec<Seen>,
        canvas: Option<egui::Id>,
    ) -> Self {
        ValueList { function, file, kind, around, screen, values, query: String::new(), chosen: 0, canvas, focused: false }
    }

    /// The rows for the search, at most [`SHOWN`]: for a `for`, the range typed first (`None`); then the values that the
    /// kind takes, whose name holds the search, by their index in `values`.
    fn rows(&self) -> Vec<Option<usize>> {
        let query = self.query.trim();
        let mut rows = Vec::new();
        if self.kind == "for" && query.contains("..") {
            rows.push(None);
        }
        rows.extend(
            self.values
                .iter()
                .enumerate()
                .filter(|(_, value)| takes(self.kind, &value.ty) && (query.is_empty() || value.name.contains(query)))
                .map(|(index, _)| Some(index))
                .take(SHOWN),
        );
        rows
    }

    /// What a row chooses: the range typed, or the name of a value.
    fn chosen_text(&self, row: Option<usize>) -> Option<String> {
        match row {
            None => Some(self.query.trim().to_owned()),
            Some(index) => self.values.get(index).map(|value| value.name.clone()),
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Chosen {
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
            return Chosen::Cancelled;
        }
        let count = rows.len();
        if down && count > 0 {
            self.chosen = (self.chosen + 1) % count;
        }
        if up && count > 0 {
            self.chosen = self.chosen.checked_sub(1).unwrap_or(count - 1);
        }
        self.chosen = self.chosen.min(count.saturating_sub(1));
        let mut clicked = None;
        let area =
            egui::Area::new(egui::Id::unique("labcraft-structure-value")).order(egui::Order::Foreground).fixed_pos(self.screen).show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(320.0);
                    let before = self.query.clone();
                    let field = ui.add(egui::TextEdit::singleline(&mut self.query).hint_text(hint(self.kind)).desired_width(f32::INFINITY));
                    if !self.focused {
                        field.request_focus();
                        self.focused = true;
                    }
                    if self.query != before {
                        self.chosen = 0;
                    }
                    egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                        if rows.is_empty() {
                            ui.label(RichText::new(nothing(self.kind)).weak());
                        }
                        for (at, row) in rows.iter().enumerate() {
                            let text = match row.and_then(|index| self.values.get(index)) {
                                Some(value) if value.ty.is_empty() => value.name.clone(),
                                Some(value) => format!("{}: {}", value.name, value.ty),
                                None => self.query.trim().to_owned(),
                            };
                            let response = ui.selectable_label(at == self.chosen, RichText::new(text).monospace());
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
        if let Some(row) = clicked.or(enter.then_some(self.chosen)).and_then(|at| rows.get(at)) {
            return self.chosen_text(*row).map_or(Chosen::Cancelled, Chosen::Value);
        }
        let clicked_elsewhere = ctx.input(|input| input.pointer.any_click().then(|| input.pointer.interact_pos()).flatten());
        if clicked_elsewhere.is_some_and(|at| !area.response.rect.contains(at)) {
            return Chosen::Cancelled;
        }
        Chosen::Open
    }
}

/// Whether a structure of `kind` takes a value of this type, as the language writes it (spec 6.1, 6.2, 7.3): a type not
/// known yet is offered, and the command judges it.
pub fn takes(kind: &str, ty: &str) -> bool {
    if ty.is_empty() {
        return true;
    }
    match kind {
        "handle" => ty == "error",
        "for" => ty.starts_with('['),
        "match" => {
            let integer = ["i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64"].contains(&ty);
            // A type of a library or of the project (`Verdict`, `stations.State`): an enumeration, or a struct, which the
            // command refuses with its reason.
            let named = ty.rsplit('.').next().is_some_and(|last| last.starts_with(|c: char| c.is_uppercase()));
            ty == "bool" || ty == "string" || integer || named
        }
        _ => false,
    }
}

fn hint(kind: &str) -> &'static str {
    match kind {
        "handle" => "The error that the handle looks at",
        "for" => "The array to go over, or a range: 0..n",
        _ => "The value that the match looks at",
    }
}

fn nothing(kind: &str) -> &'static str {
    match kind {
        "handle" => "No error here",
        "for" => "No array here: type a range, 0..n",
        _ => "No value here that a match takes",
    }
}

/// `structure.surround` with the value chosen, as `selector` or `over`.
pub fn command(file: &str, kind: &str, around: &Around, value: Option<&str>) -> Value {
    if let Around::Clause { structure } = around {
        return json!({ "file": file, "structure": structure, "over": value.unwrap_or_default() });
    }
    let mut params = json!({ "file": file, "kind": kind });
    let Some(object) = params.as_object_mut() else { return params };
    if let Some(value) = value {
        object.insert(if kind == "for" { "over" } else { "selector" }.to_owned(), json!(value));
    }
    match around {
        Around::Nodes(nodes) => {
            object.insert("nodes".to_owned(), json!(nodes));
        }
        Around::Empty { at, within } => {
            object.insert("at".to_owned(), json!([at.0, at.1]));
            if let Some(inside) = &within.inside {
                object.insert("inside".to_owned(), json!(inside));
                if let Some(arm) = &within.arm {
                    object.insert("inside_arm".to_owned(), json!(arm));
                }
            }
        }
        Around::Clause { .. } => {}
    }
    params
}

/// The command that a value chosen in the list sends: `structure.surround`, or `structure.add_clause` for a clause.
pub fn command_id(around: &Around) -> &'static str {
    match around {
        Around::Clause { .. } => "structure.add_clause",
        Around::Nodes(_) | Around::Empty { .. } => "structure.surround",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// By hand from commands.md 9.6 and 6.7: the value goes as `selector`, or `over` for a `for`; an empty structure takes
    /// its place and its frame, `inside` and `inside_arm`.
    #[test]
    fn the_command_carries_the_value_and_the_frame() {
        let nodes = Around::Nodes(vec!["a".to_owned(), "#close".to_owned()]);
        assert_eq!(
            command("f.lcf", "match", &nodes, Some("ok")),
            json!({ "file": "f.lcf", "kind": "match", "selector": "ok", "nodes": ["a", "#close"] })
        );
        assert_eq!(
            command("f.lcf", "for", &nodes, Some("0..n")),
            json!({ "file": "f.lcf", "kind": "for", "over": "0..n", "nodes": ["a", "#close"] })
        );
        let within = InFrame { inside: Some("m".to_owned()), arm: Some("_".to_owned()) };
        let empty = Around::Empty { at: (40, 80), within };
        assert_eq!(
            command("f.lcf", "loop", &empty, None),
            json!({ "file": "f.lcf", "kind": "loop", "at": [40, 80], "inside": "m", "inside_arm": "_" })
        );
    }

    /// Spec 6.1, 6.2 and 7.3: what each kind looks at or goes over, a type not known yet offered to all.
    #[test]
    fn each_kind_lists_the_values_it_takes() {
        assert!(takes("match", "bool") && takes("match", "u8") && takes("match", "string") && takes("match", "Verdict"));
        assert!(takes("match", "stations.State") && takes("match", ""));
        assert!(!takes("match", "f64") && !takes("match", "f64[V]") && !takes("match", "error") && !takes("match", "[i32]"));
        assert!(takes("handle", "error") && !takes("handle", "bool"));
        assert!(takes("for", "[f64]") && takes("for", "[[i32]]") && !takes("for", "i32"));
    }
}
