//! The panels of the window (ide.md 4), as 1a has them: `Project`, the tree of the folder; `Palette`, the functions
//! the project can call, searched; `Inspector`, the function itself (node, wire and terminal come with the selection,
//! step 2); `Problems`, the diagnostics by file, error first; `Run`, the trace of the function shown.

use std::collections::BTreeMap;

use egui::RichText;
use kit_shell::fuzzy;
use kit_shell::inspector::{self, Field, Section};
use labcraft_engine::drawings::TracePart;
use labcraft_engine::view::{Listed, Published};
use labcraft_types::Severity;
use serde_json::{Value, json};

use crate::host::{Host, Reply};

/// What a panel asks the window for.
#[derive(Clone, Debug, PartialEq)]
pub enum Asked {
    /// Show this function: in preview after a single click, kept after a double one (ide.md 3).
    Show { function: String, preview: bool },
    /// A command of the session.
    Command(String, Value),
    /// An action of the window (`ide.*`).
    Window(&'static str),
}

/// `Project`: the files of the folder, as a tree of folders, each file with its functions.
pub fn project(ui: &mut egui::Ui, published: &Published, preview_tabs: bool) -> Option<Asked> {
    let Some(folder) = &published.folder else {
        ui.label(RichText::new("No folder open").weak());
        return None;
    };
    let mut asked = None;
    // By folder, then by file.
    let mut tree: BTreeMap<Vec<&str>, BTreeMap<&str, Vec<&Listed>>> = BTreeMap::new();
    for listed in &published.functions {
        let mut parts: Vec<&str> = listed.file.split('/').collect();
        let file = parts.pop().unwrap_or_default();
        tree.entry(parts).or_default().entry(file).or_default().push(listed);
    }
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        if folder != "." {
            ui.label(RichText::new(folder).weak().small());
        }
        for (folders, files) in &tree {
            let mut body = |ui: &mut egui::Ui| {
                for (file, functions) in files {
                    ui.label(RichText::new(*file).small().weak());
                    for listed in functions {
                        let shown = published.function.as_deref() == Some(listed.function.as_str());
                        let text = if listed.drawable || listed.errors == 0 {
                            RichText::new(&listed.function)
                        } else {
                            RichText::new(format!("{}  ({} error{})", listed.function, listed.errors, if listed.errors == 1 { "" } else { "s" }))
                                .color(ui.visuals().warn_fg_color)
                        };
                        let response = ui.indent(("project", &listed.file, &listed.function), |ui| ui.selectable_label(shown, text)).inner;
                        let response = response.on_hover_text(&listed.file);
                        if response.double_clicked() {
                            asked = Some(Asked::Show { function: listed.function.clone(), preview: false });
                        } else if response.clicked() {
                            asked = Some(Asked::Show { function: listed.function.clone(), preview: preview_tabs });
                        }
                    }
                }
            };
            if folders.is_empty() {
                body(ui);
            } else {
                egui::CollapsingHeader::new(folders.join("/")).default_open(true).show(ui, body);
            }
        }
        if published.functions.is_empty() {
            ui.label(RichText::new("No function in this folder").weak());
        }
    });
    asked
}

/// An entry of `functions.list`.
#[derive(Clone, Debug, PartialEq)]
pub struct Callable {
    pub name: String,
    pub library: String,
    pub signature: String,
    pub doc: Option<String>,
}

/// What `Palette` keeps: the list of `functions.list`, asked again after each check.
#[derive(Default)]
pub struct Callables {
    pub list: Vec<Callable>,
    pub truncated: bool,
    /// The check it was listed for (`Published::checks`): a new check lists again.
    checks: Option<u64>,
    asked: Option<Reply>,
    pub failure: Option<String>,
    pub search: String,
}

impl Callables {
    pub fn refresh(&mut self, host: &dyn Host, checks: u64) {
        if self.checks != Some(checks) && self.asked.is_none() {
            self.checks = Some(checks);
            self.asked = Some(host.send("functions.list", json!({ "docs": true })));
        }
        let Some(result) = self.asked.as_ref().and_then(Reply::take) else { return };
        self.asked = None;
        match result {
            Ok(value) => {
                self.failure = None;
                self.truncated = value.get("truncated").and_then(Value::as_bool).unwrap_or(false);
                let text = |entry: &Value, key: &str| entry.get(key).and_then(Value::as_str).map(str::to_owned);
                self.list = value
                    .get("entries")
                    .and_then(Value::as_array)
                    .map(|entries| {
                        entries
                            .iter()
                            .filter_map(|entry| {
                                Some(Callable {
                                    name: text(entry, "name")?,
                                    library: text(entry, "library").unwrap_or_default(),
                                    signature: text(entry, "signature").unwrap_or_default(),
                                    doc: text(entry, "doc"),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
            }
            Err(error) => self.failure = Some(error.message),
        }
    }

    /// The entry of a function of the project: its library is the root, `""`.
    pub fn own(&self, function: &str) -> Option<&Callable> {
        self.list.iter().find(|callable| callable.library.is_empty() && callable.name == function)
    }
}

/// `Palette`: the functions to place, by library, searched as quick insertion searches. Placing one comes with step 4.
pub fn palette(ui: &mut egui::Ui, callables: &mut Callables) {
    ui.add(egui::TextEdit::singleline(&mut callables.search).hint_text("Search a function").desired_width(f32::INFINITY));
    if let Some(failure) = &callables.failure {
        ui.label(RichText::new(failure).color(ui.visuals().warn_fg_color));
    }
    let ranked = fuzzy::rank(&callables.search, &callables.list, |callable| vec![callable.name.as_str(), callable.library.as_str()]);
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        if callables.search.trim().is_empty() {
            let mut by_library: BTreeMap<&str, Vec<&Callable>> = BTreeMap::new();
            for callable in &callables.list {
                by_library.entry(callable.library.as_str()).or_default().push(callable);
            }
            for (library, callables) in by_library {
                let title = if library.is_empty() { "This project" } else { library };
                egui::CollapsingHeader::new(title).default_open(library.is_empty()).show(ui, |ui| {
                    for callable in callables {
                        row(ui, callable);
                    }
                });
            }
        } else {
            for callable in ranked.iter().filter_map(|&at| callables.list.get(at)) {
                row(ui, callable);
            }
        }
        if callables.truncated {
            ui.label(RichText::new("The list is cut: search to narrow it").small().weak());
        }
    });
}

fn row(ui: &mut egui::Ui, callable: &Callable) {
    let name = if callable.library.is_empty() { callable.name.clone() } else { format!("{}.{}", callable.library, callable.name) };
    let response = ui.label(RichText::new(name).monospace());
    let hover = match &callable.doc {
        Some(doc) => format!("{}\n\n{doc}", callable.signature),
        None => callable.signature.clone(),
    };
    response.on_hover_text(hover);
}

/// `Inspector`, with nothing selected: the function itself, its signature, its documentation and its frame (ide.md 4;
/// decisions of the owner, 2026-11-16).
pub fn inspector(ui: &mut egui::Ui, published: &Published, callables: &Callables) {
    let Some(function) = &published.function else {
        inspector::show(ui, "Nothing shown", &[]);
        return;
    };
    let mut sections = Vec::new();
    let listed = published.functions.iter().find(|listed| listed.function == *function);
    let mut fields = vec![Field::code("Function", function.as_str())];
    if let Some(listed) = listed {
        fields.push(Field::code("File", listed.file.as_str()));
        if listed.errors + listed.warnings > 0 {
            fields.push(Field::new("Problems", format!("{}, {}", counted(listed.errors, "error"), counted(listed.warnings, "warning"))));
        }
    }
    let own = callables.own(function);
    if let Some(callable) = own {
        fields.push(Field::code("Signature", callable.signature.as_str()));
    }
    if let Some(drawing) = &published.drawing {
        // In units of the notation, as `layout` writes `frame size` (spec 11); the drawing counts hundredths.
        fields.push(Field::new("Frame", format!("{} × {}", drawing.width / 100, drawing.height / 100)));
        fields.push(Field::new("Nodes", drawing.placed.len().to_string()));
    }
    sections.push(Section { title: None, fields, text: None });
    if let Some(doc) = own.and_then(|callable| callable.doc.as_ref()) {
        sections.push(Section { title: Some("Documentation".to_owned()), fields: Vec::new(), text: Some(doc.clone()) });
    }
    inspector::show(ui, "Function", &sections);
}

/// What `Problems` filters.
#[derive(Clone, Debug, Default)]
pub struct ProblemsFilter {
    pub errors_only: bool,
    pub shown_only: bool,
}

/// `Problems`: the diagnostics of the project, by file, error first; a click shows the function.
pub fn problems(ui: &mut egui::Ui, published: &Published, filter: &mut ProblemsFilter) -> Option<Asked> {
    let mut asked = None;
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{}, {}", counted(published.errors, "error"), counted(published.warnings, "warning"))).strong());
        ui.checkbox(&mut filter.errors_only, "Errors only");
        ui.checkbox(&mut filter.shown_only, "Function shown only");
    });
    if published.kept {
        ui.label(RichText::new("This function does not draw now: its last drawing stays, with the diagnostics of its file.").weak());
    }
    let Some(report) = &published.report else { return None };
    let shown_file = published
        .function
        .as_ref()
        .and_then(|function| published.functions.iter().find(|listed| listed.function == *function))
        .map(|listed| listed.file.as_str());
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let mut by_file: BTreeMap<&str, Vec<&labcraft_types::Diagnostic>> = BTreeMap::new();
        for diagnostic in report.diagnostics() {
            if filter.errors_only && diagnostic.severity() != Severity::Error {
                continue;
            }
            if filter.shown_only && shown_file != Some(diagnostic.file.as_str()) {
                continue;
            }
            by_file.entry(diagnostic.file.as_str()).or_default().push(diagnostic);
        }
        for (file, mut diagnostics) in by_file {
            diagnostics.sort_by_key(|diagnostic| diagnostic.severity() != Severity::Error);
            ui.label(RichText::new(if file.is_empty() { "(the project)" } else { file }).strong().small());
            for diagnostic in diagnostics {
                let colour = if diagnostic.severity() == Severity::Error { ui.visuals().error_fg_color } else { ui.visuals().warn_fg_color };
                let at = diagnostic.span.map(|span| format!(":{}:{}", span.start.line, span.start.column)).unwrap_or_default();
                let text = RichText::new(format!("{} {}{at}  {}", diagnostic.severity().as_str(), diagnostic.code().as_str(), diagnostic.message))
                    .color(colour);
                let response = ui.add(egui::Label::new(text).sense(egui::Sense::click()).wrap());
                if response.clicked() {
                    let function = diagnostic
                        .function
                        .clone()
                        .or_else(|| published.functions.iter().find(|listed| listed.file == diagnostic.file).map(|listed| listed.function.clone()));
                    if let Some(function) = function.filter(|function| published.functions.iter().any(|listed| listed.function == *function)) {
                        asked = Some(Asked::Show { function, preview: false });
                    }
                }
            }
        }
        if report.diagnostics().is_empty() {
            ui.label(RichText::new("No problem").weak());
        }
    });
    asked
}

/// `Run`: the trace of the function shown and how its run ended (the bar of the viewer of M0v, as a panel).
pub fn run(ui: &mut egui::Ui, published: &Published, values_shown: bool) -> Option<Asked> {
    let mut asked = None;
    if published.function.is_none() {
        ui.label(RichText::new("No function shown").weak());
        return None;
    }
    match &published.trace {
        Some(trace) => {
            ui.horizontal(|ui| {
                ui.label(format!("{}: {}", if published.trace_opened { "Trace" } else { "Last run" }, trace.file));
                if published.trace_opened && kit_shell::icon::button(ui, kit_shell::icon::Icon::Close, true, "Back to the last run").clicked() {
                    asked = Some(Asked::Command("view.open_trace".to_owned(), json!({})));
                }
            });
            ui.label(trace_status(trace));
        }
        None => {
            ui.label(RichText::new("No run of this function yet").weak());
        }
    }
    ui.horizontal(|ui| {
        let mut shown = values_shown;
        if ui.checkbox(&mut shown, "Values on the wires").changed() {
            asked = Some(Asked::Window(crate::keys::VALUES));
        }
        if ui.button("Open a trace…").clicked() {
            asked = Some(Asked::Window(crate::keys::OPEN_TRACE));
        }
    });
    asked
}

/// `1 error`, `2 errors`.
fn counted(count: u64, what: &str) -> String {
    format!("{count} {what}{}", if count == 1 { "" } else { "s" })
}

/// How the run of a trace ended, in a few words.
pub fn trace_status(trace: &TracePart) -> String {
    let mut said = vec![format!("{} event{}", trace.events, if trace.events == 1 { "" } else { "s" })];
    if let Some(status) = &trace.status {
        said.push(status.clone());
    } else if !trace.cut {
        said.push("no end".to_owned());
    }
    if trace.cut {
        said.push("cut".to_owned());
    }
    if !trace.stale.is_empty() {
        said.push("the files have changed since".to_owned());
    }
    said.join(", ")
}
