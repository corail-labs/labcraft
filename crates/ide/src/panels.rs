//! The panels of the window (ide.md 4), as 1a has them: `Project`, the tree of the folder; `Palette`, the functions
//! the project can call, searched; `Inspector`, what the selection is, or the function itself;
//! `Problems`, the diagnostics by file, error first; `Run`, the trace of the function shown. Each shows the
//! window it is given: the one that has focus, or for the inspector of a detached window, that window (ide.md 9).

use std::collections::BTreeMap;
use std::sync::Arc;

use egui::RichText;
use kit_shell::icon::{self, Icon};
use kit_shell::inspector::{self, Field, Section};
use labcraft_diagram::{Drawing, Overlay};
use labcraft_editor::Dragged;
use labcraft_editor::insertion::{self, Catalog, EntryKind};
use labcraft_engine::drawings::TracePart;
use labcraft_engine::view::{Listed, Published, Shown};
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

/// Whether `file` holds one function only, named like it (its last name): it is shown as that function, on one line.
fn alone(file: &str, functions: &[&Listed]) -> bool {
    let stem = file.strip_suffix(".lcf").unwrap_or(file);
    matches!(functions, [only] if only.function.rsplit('.').next() == Some(stem))
}

/// `Project`: the files of the folder, as a tree of folders, each file with its functions.
/// `focused`: the function of the window that has focus, marked in the tree.
pub fn project(ui: &mut egui::Ui, published: &Published, focused: Option<&str>, preview_tabs: bool) -> Option<Asked> {
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
                    // A file of one function of its name is that function: one line, its file on hover. Else the
                    // file, then its functions under it.
                    let alone = alone(file, functions);
                    if !alone {
                        ui.horizontal(|ui| {
                            icon::leading(ui, Icon::File, ui.visuals().weak_text_color());
                            ui.label(RichText::new(*file).small().weak());
                        });
                    }
                    for listed in functions {
                        let shown = focused == Some(listed.function.as_str());
                        let warned = !listed.drawable && listed.errors > 0;
                        let text = if warned {
                            RichText::new(format!("{}  ({} error{})", listed.function, listed.errors, if listed.errors == 1 { "" } else { "s" }))
                                .color(ui.visuals().warn_fg_color)
                        } else {
                            RichText::new(&listed.function)
                        };
                        // A function opens on its diagram: the icon of the view `Diagram`, in the colour of its text.
                        let line = |ui: &mut egui::Ui| {
                            ui.horizontal(|ui| {
                                let colour = if warned { ui.visuals().warn_fg_color } else { ui.visuals().text_color() };
                                icon::leading(ui, Icon::Diagram, colour);
                                ui.selectable_label(shown, text)
                            })
                            .inner
                        };
                        let response = if alone { line(ui) } else { ui.indent(("project", &listed.file, &listed.function), line).inner };
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
                let id = ui.make_persistent_id(("project-folder", folders));
                egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
                    .show_header(ui, |ui| {
                        icon::leading(ui, Icon::Folder, ui.visuals().weak_text_color());
                        ui.label(folders.join("/"));
                    })
                    .body(body);
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
    /// `function` or `type`.
    pub kind: String,
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
    /// The catalog of quick insertion and of the palette, read from the list (notation 2.6).
    pub catalog: Arc<Catalog>,
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
                                    kind: text(entry, "kind").unwrap_or_default(),
                                    signature: text(entry, "signature").unwrap_or_default(),
                                    doc: text(entry, "doc"),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let listed: Vec<insertion::Listed> = self
                    .list
                    .iter()
                    .map(|callable| insertion::Listed {
                        name: callable.name.clone(),
                        library: callable.library.clone(),
                        kind: callable.kind.clone(),
                        signature: callable.signature.clone(),
                    })
                    .collect();
                self.catalog = Arc::new(Catalog::new(&listed));
            }
            Err(error) => self.failure = Some(error.message),
        }
    }

    /// The entry of a function of the project: its library is the root, `""`.
    pub fn own(&self, function: &str) -> Option<&Callable> {
        self.list.iter().find(|callable| callable.library.is_empty() && callable.name == function)
    }
}

/// `Palette`: the functions to place, by library, searched as quick insertion searches; each dragged onto the canvas.
pub fn palette(ui: &mut egui::Ui, callables: &mut Callables) {
    ui.add(egui::TextEdit::singleline(&mut callables.search).hint_text("Search a function").desired_width(f32::INFINITY));
    if let Some(failure) = &callables.failure {
        ui.label(RichText::new(failure).color(ui.visuals().warn_fg_color));
    }
    // Ranked as quick insertion ranks (`Catalog::search`): the best match first, then the order of the list.
    let mut ranked: Vec<(i64, usize)> = callables
        .list
        .iter()
        .enumerate()
        .filter_map(|(at, callable)| Catalog::search(callables.search.trim(), &call_of(callable), &callable.library).map(|score| (score, at)))
        .collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let catalog = callables.catalog.clone();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        if callables.search.trim().is_empty() {
            let mut by_library: BTreeMap<&str, Vec<&Callable>> = BTreeMap::new();
            for callable in &callables.list {
                by_library.entry(callable.library.as_str()).or_default().push(callable);
            }
            for (library, listed) in by_library {
                let title = if library.is_empty() { "This project" } else { library };
                egui::CollapsingHeader::new(title).default_open(library.is_empty()).show(ui, |ui| {
                    for callable in listed {
                        row(ui, callable, &catalog);
                    }
                });
            }
        } else {
            for callable in ranked.iter().filter_map(|&(_, at)| callables.list.get(at)) {
                row(ui, callable, &catalog);
            }
        }
        if callables.truncated {
            ui.label(RichText::new("The list is cut: search to narrow it").small().weak());
        }
    });
}

/// How a call names the callable: `visa.open`, `settle`.
fn call_of(callable: &Callable) -> String {
    if callable.library.is_empty() || callable.name.contains('.') { callable.name.clone() } else { format!("{}.{}", callable.library, callable.name) }
}

/// A row of the palette: dragged onto the canvas, its function is a node added there, or into the wire it is dropped on
/// (`node.add`, notation 2.5).
fn row(ui: &mut egui::Ui, callable: &Callable, catalog: &Arc<Catalog>) {
    let name = call_of(callable);
    let entry = catalog.index_of(&name).filter(|&at| catalog.entries.get(at).is_some_and(|entry| entry.kind != EntryKind::Constant));
    let response =
        ui.add(egui::Label::new(RichText::new(&name).monospace()).sense(if entry.is_some() { egui::Sense::drag() } else { egui::Sense::hover() }));
    if let Some(entry) = entry {
        response.dnd_set_drag_payload(Dragged { catalog: catalog.clone(), entry });
        if response.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
    }
    let hover = match &callable.doc {
        Some(doc) => format!("{}\n\n{doc}", callable.signature),
        None => callable.signature.clone(),
    };
    response.on_hover_text(hover);
}

/// `Inspector`: what the selection is (ide.md 4; decision of the owner, 2026-11-14): one node, one wire (its whole value),
/// one terminal of the frame, several; nothing selected, the function itself, its signature, its documentation and its
/// frame (decisions of the owner, 2026-11-16). From step 2b, a node's place (`node.place`), its view (`node.set_view`)
/// and "Remove the fixed position" (`layout.arrange`) change there, each the command of the same gesture (decision of
/// the owner, 2026-11-17); its name and its constants come with step 4.
pub fn inspector(ui: &mut egui::Ui, published: &Published, shown: &Shown, callables: &Callables) -> Option<Asked> {
    let Some(function) = &shown.function else {
        inspector::show(ui, "Nothing shown", &[]);
        return None;
    };
    let selection = &shown.selection;
    let drawing = shown.drawing.as_deref();
    let (Some(drawing), true) = (drawing, selection.function == *function && !selection.is_empty()) else {
        of_function(ui, published, shown, function, callables);
        return None;
    };
    let file = published.functions.iter().find(|listed| listed.function == *function).map(|listed| listed.file.clone());
    // The values that the selection names: a wire is selected with every wire of its value.
    let values: std::collections::BTreeSet<usize> = selection
        .wires
        .iter()
        .filter_map(|name| drawing.groups.iter().position(|group| group.wire.as_ref().is_some_and(|wire| wire.named.contains(name))))
        .collect();
    let overlay = shown.overlay.as_deref();
    let answer = match (selection.nodes.as_slice(), values.len(), selection.terminals.as_slice()) {
        ([node], 0, []) => of_node(ui, drawing, overlay, node, callables),
        ([], 1, []) => {
            let group = values.first().copied()?;
            // "Arrange this wire" (notation 2.4): every wire of the value that the file routes.
            of_wire(ui, drawing, overlay, group)?;
            let wire = drawing.groups.get(group).and_then(|group| group.wire.as_ref())?;
            let wires: Vec<&String> = wire.entries.iter().zip(&wire.vias).filter(|(_, via)| **via).filter_map(|(entry, _)| entry.as_ref()).collect();
            // No wire named would arrange the whole function.
            if wires.is_empty() {
                return None;
            }
            return Some(Asked::Command("layout.arrange".to_owned(), json!({ "file": file?, "wires": wires })));
        }
        ([], 0, [terminal]) => {
            of_terminal(ui, drawing, terminal);
            None
        }
        (nodes, values, terminals) => {
            let mut fields = Vec::new();
            for (count, label, what) in [(nodes.len(), "Nodes", "node"), (values, "Wires", "wire"), (terminals.len(), "Terminals", "terminal")] {
                if count > 0 {
                    fields.push(Field::new(label, counted(count as u64, what)));
                }
            }
            let placed: Vec<&labcraft_diagram::Placed> =
                nodes.iter().filter_map(|name| drawing.placed.iter().find(|placed| placed.name == *name)).collect();
            fields.extend(layout_fields(&placed));
            inspector::show(ui, "Selection", &[Section { title: None, fields, text: None }])
        }
    };
    let file = file?;
    asked_of(answer?, &file, &selection.nodes, drawing)
}

/// The views a node can have, in the order of the inspector: `node.set_view` takes them so.
const VIEWS: [&str; 3] = ["default", "standard", "compact"];

/// The fields of the place and the view of nodes, which edit: one node, its place too (notation 2.4).
fn layout_fields(placed: &[&labcraft_diagram::Placed]) -> Vec<Field> {
    let mut fields = Vec::new();
    let Some(first) = placed.first() else { return fields };
    // The view that every node has, or none chosen when they differ.
    let view =
        |placed: &labcraft_diagram::Placed| VIEWS.iter().position(|view| *view == placed.view || (view == &"default" && placed.view.is_empty()));
    let chosen = view(first).filter(|chosen| placed.iter().all(|other| view(other) == Some(*chosen))).unwrap_or(usize::MAX);
    fields.push(Field::choice("View", "view", &["Default", "Standard", "Compact"], chosen));
    if placed.iter().any(|placed| placed.at.is_some()) {
        fields.push(Field::button("Fixed position", "arrange", "Remove the fixed position"));
    }
    fields
}

/// The command that a field edited asks for, on the nodes selected.
fn asked_of(answer: inspector::Answer, file: &str, nodes: &[String], drawing: &Drawing) -> Option<Asked> {
    match answer {
        inspector::Answer::Integer { key, value } => {
            // One node: its place in its frame, the other coordinate as it is (spec 11).
            let [node] = nodes else { return None };
            let at = drawing.placed.iter().position(|placed| placed.name == *node)?;
            let (x, y) = place_in_frame(drawing, at)?;
            let at = if key == "x" { [value, y] } else { [x, value] };
            Some(Asked::Command("node.place".to_owned(), json!({ "file": file, "node": node, "at": at })))
        }
        inspector::Answer::Choice { chosen, .. } => {
            let view = VIEWS.get(chosen)?;
            Some(Asked::Command("node.set_view".to_owned(), json!({ "file": file, "nodes": nodes, "view": view })))
        }
        inspector::Answer::Pressed { .. } => Some(Asked::Command("layout.arrange".to_owned(), json!({ "file": file, "nodes": nodes }))),
    }
}

/// Where a node is in its frame, in units: where `layout` fixes it, else where the automatic placement puts it.
fn place_in_frame(drawing: &Drawing, at: usize) -> Option<(i64, i64)> {
    let (x, y) = drawing.in_frame(at)?;
    Some(((x + 50).div_euclid(100), (y + 50).div_euclid(100)))
}

/// The inspector of the function itself.
fn of_function(ui: &mut egui::Ui, published: &Published, shown: &Shown, function: &str, callables: &Callables) {
    let mut sections = Vec::new();
    let listed = published.functions.iter().find(|listed| listed.function == *function);
    let mut fields = vec![Field::code("Function", function)];
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
    if let Some(drawing) = &shown.drawing {
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

/// The inspector of one node: what it calls, its signature and documentation, where it is, the values of the last run
/// on the wires it gives (ide.md 4).
fn of_node(ui: &mut egui::Ui, drawing: &Drawing, overlay: Option<&Overlay>, name: &str, callables: &Callables) -> Option<inspector::Answer> {
    let Some((placed_at, placed)) = drawing.placed.iter().enumerate().find(|(_, placed)| placed.name == name) else {
        inspector::show(ui, "Node", &[Section { title: None, fields: vec![Field::code("Node", name)], text: None }]);
        return None;
    };
    let calls =
        drawing.groups.iter().find(|group| group.placed == Some(placed_at) && group.is_node()).map(|group| group.title.as_str()).unwrap_or_default();
    let callable = callables.list.iter().find(|callable| {
        let full = if callable.library.is_empty() { callable.name.clone() } else { format!("{}.{}", callable.library, callable.name) };
        !calls.is_empty() && full == calls
    });
    let mut fields = vec![Field::code("Node", name)];
    if !calls.is_empty() {
        fields.push(Field::code("Calls", calls));
    }
    if let Some(callable) = callable {
        fields.push(Field::code("Signature", callable.signature.as_str()));
    }
    // In units of the notation, from the corner of its frame, as `layout` writes it (spec 11); a value typed fixes it.
    let position = if placed.at.is_some() { "Fixed" } else { "Placed automatically" };
    fields.push(Field::new("Position", position));
    if let Some((x, y)) = place_in_frame(drawing, placed_at) {
        fields.push(Field::integer("X", "x", x));
        fields.push(Field::integer("Y", "y", y));
    }
    fields.extend(layout_fields(&[placed]));
    if placed.line > 0 {
        fields.push(Field::new("Line", placed.line.to_string()));
    }
    let mut sections = vec![Section { title: None, fields, text: None }];
    // The values of the last run on the wires that the node gives.
    if let Some(overlay) = overlay {
        let given: Vec<Field> = drawing
            .groups
            .iter()
            .enumerate()
            .filter_map(|(group, found)| Some((group, found.wire.as_ref()?)))
            .filter(|(_, wire)| wire.from_node.as_deref() == Some(placed.key.as_str()))
            .filter_map(|(group, wire)| Some(Field::code(wire.value.clone(), overlay.wires.get(&group)?.title.clone())))
            .collect();
        if !given.is_empty() {
            sections.push(Section { title: Some("Last run".to_owned()), fields: given, text: None });
        }
    }
    if let Some(doc) = callable.and_then(|callable| callable.doc.as_ref()) {
        sections.push(Section { title: Some("Documentation".to_owned()), fields: Vec::new(), text: Some(doc.clone()) });
    }
    inspector::show(ui, "Node", &sections)
}

/// The inspector of one wire, its whole value: its type, where it comes from and goes, its route, its value of the last
/// run (ide.md 4).
fn of_wire(ui: &mut egui::Ui, drawing: &Drawing, overlay: Option<&Overlay>, group: usize) -> Option<inspector::Answer> {
    let wire = drawing.groups.get(group).and_then(|group| group.wire.as_ref())?;
    let mut fields = vec![Field::code("Value", wire.value.as_str())];
    if !wire.ty.is_empty() {
        fields.push(Field::code("Type", wire.ty.as_str()));
    }
    if let Some(from) = &wire.from {
        fields.push(Field::code("From", from.as_str()));
    }
    if !wire.to.is_empty() {
        fields.push(Field::code("To", wire.to.join(", ")));
    }
    if wire.via {
        fields.push(Field::button("Route", "arrange", "Arrange this wire"));
    } else {
        fields.push(Field::new("Route", "Automatic"));
    }
    if let Some(valued) = overlay.and_then(|overlay| overlay.wires.get(&group)) {
        fields.push(Field::code("Last run", valued.title.as_str()));
    }
    inspector::show(ui, "Wire", &[Section { title: None, fields, text: None }])
}

/// The inspector of one terminal of the frame: its name, its type and its default, as its tooltip gives them. Whether it
/// is fixed comes with `input x at y` and `terminal.move` (step 6).
fn of_terminal(ui: &mut egui::Ui, drawing: &Drawing, name: &str) {
    let mut fields = vec![Field::code("Terminal", name)];
    if let Some(hint) = drawing.hints.iter().find(|hint| hint.selects.as_deref() == Some(name)) {
        let mut lines = hint.text.lines();
        if let Some((_, ty)) = lines.next().and_then(|line| line.split_once(": ")) {
            fields.push(Field::code("Type", ty));
        }
        for line in lines {
            if let Some((label, value)) = line.split_once(": ") {
                let mut chars = label.chars();
                let label: String = chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default();
                fields.push(Field::code(label, value));
            }
        }
    }
    inspector::show(ui, "Terminal", &[Section { title: None, fields, text: None }]);
}

/// What `Problems` filters.
#[derive(Clone, Debug, Default)]
pub struct ProblemsFilter {
    pub errors_only: bool,
    pub shown_only: bool,
}

/// `Problems`: the diagnostics of the project, by file, error first; a click shows the function.
pub fn problems(ui: &mut egui::Ui, published: &Published, shown: &Shown, filter: &mut ProblemsFilter) -> Option<Asked> {
    let mut asked = None;
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{}, {}", counted(published.errors, "error"), counted(published.warnings, "warning"))).strong());
        ui.checkbox(&mut filter.errors_only, "Errors only");
        ui.checkbox(&mut filter.shown_only, "Function shown only");
    });
    if shown.kept {
        ui.label(RichText::new("This function does not draw now: its last drawing stays, with the diagnostics of its file.").weak());
    }
    let Some(report) = &published.report else { return None };
    let shown_file = shown
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

/// What the `Run` panel says of the run of a function that the window started.
pub struct RunSaid {
    /// Going, or how it ended and its outputs.
    pub said: String,
    /// It ran and wrote no trace (`Record values` off): a trace shown is of an earlier run.
    pub unrecorded: bool,
}

/// `Run`: the trace of the function shown and how its run ended (the bar of the viewer of M0v, as a panel); `said`,
/// the run of that function that the window started, going or ended.
pub fn run(ui: &mut egui::Ui, shown: &Shown, values_shown: bool, said: Option<&RunSaid>) -> Option<Asked> {
    let mut asked = None;
    let Some(function) = &shown.function else {
        ui.label(RichText::new("No function shown").weak());
        return None;
    };
    if let Some(said) = said {
        ui.label(RichText::new(&said.said).strong());
    }
    let unrecorded = said.is_some_and(|said| said.unrecorded);
    match &shown.trace {
        Some(trace) => {
            ui.horizontal(|ui| {
                ui.label(format!("{}: {}", if shown.trace_opened { "Trace" } else { "Last run" }, trace.file));
                if shown.trace_opened && kit_shell::icon::button(ui, kit_shell::icon::Icon::Close, true, "Back to the last run").clicked() {
                    asked = Some(Asked::Command("view.open_trace".to_owned(), json!({ "function": function })));
                }
            });
            ui.label(trace_status(trace));
            if unrecorded {
                let stale = "This run wrote no trace (Record values off): this trace, and the values on the wires, are of an earlier run";
                ui.label(RichText::new(stale).color(ui.visuals().warn_fg_color));
            }
        }
        // A run without `Record values` wrote no trace: its outputs are above.
        None if unrecorded => {
            ui.label(RichText::new("No trace: Record values was off for this run").weak());
        }
        // Going, refused or failed: what is above says it.
        None if said.is_some() => {}
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

#[cfg(test)]
mod tests {
    use super::*;

    fn listed(function: &str, file: &str) -> Listed {
        Listed { function: function.to_owned(), file: file.to_owned(), drawable: true, errors: 0, warnings: 0, needs: Vec::new() }
    }

    #[test]
    fn a_file_of_one_function_of_its_name_is_one_line_and_any_other_keeps_its_name() {
        let main = listed("main", "main.lcf");
        let helper = listed("helper", "main.lcf");
        let qualified = listed("psu.set_voltage", "psu/set_voltage.lcf");
        assert!(alone("main.lcf", &[&main]));
        assert!(alone("set_voltage.lcf", &[&qualified]));
        assert!(!alone("main.lcf", &[&main, &helper]));
        assert!(!alone("main.lcf", &[&helper]));
        assert!(!alone("main.lcf", &[]));
    }
}
