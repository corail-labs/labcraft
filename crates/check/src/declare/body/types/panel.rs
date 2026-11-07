//! The elements of a panel and the nodes of `ui` (spec 10). An element has one type. Its name gives it, as an input or an output of the
//! function, or its kind, when that kind holds only one type, or its `default:`: those are known before the diagram is walked, and a
//! wire converts to them as on a terminal. An element that the diagram reads has to say its type in one of these ways. So only an element
//! that is never read takes it from its wires, and what these read never depends on it. They are compared once the walk has seen them all,
//! since the diagram has no order and no wire comes first: wires that differ are a mistake of the element (`LC5013`), and a constant takes
//! their type, as on a terminal (3.5).

use std::collections::{HashMap, HashSet};

use labcraft_graph::{Arg, Call, Id, Numeral, NumeralKind, Operand, PanelDirection, PanelItem, Signature, Statement};
use labcraft_types::{Code, Edit, Fix, Interner, Name, Ty, TyId, Unit, Wire, cut_text};

use super::super::definition_of;
use super::{Details, Label, Out, Typer, Within};
use crate::primitives::PrimitiveLibrary;
use crate::resolved::{Meaning, Resolved, Terminal};

/// The wires that a diagnostic names.
const WIRES_SHOWN: usize = 5;

/// The kinds of M0 (spec 10). Another one has no widget and accepts everything (`LC5003`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Numeric,
    TextBox,
    Button,
    Switch,
    Led,
    Chart,
}

/// Whether M0 knows this kind (`LC5003` otherwise).
pub(in crate::declare) fn known_kind(text: &str) -> bool {
    Kind::of(text).is_some()
}

impl Kind {
    fn of(text: &str) -> Option<Self> {
        match text {
            "numeric" => Some(Kind::Numeric),
            "text_box" => Some(Kind::TextBox),
            "button" => Some(Kind::Button),
            "switch" => Some(Kind::Switch),
            "led" => Some(Kind::Led),
            "chart" => Some(Kind::Chart),
            _ => None,
        }
    }

    /// What a message says that this kind shows.
    fn shows(self) -> &'static str {
        match self {
            Kind::Numeric => "a number",
            Kind::TextBox => "a `string`",
            Kind::Button | Kind::Switch => "a `bool`",
            Kind::Led => "a `bool` or an enumeration",
            Kind::Chart => "a number, an array of numbers or a `waveform`",
        }
    }

    /// A `default:` that the fix of `LC5010` can name, since this kind shows it.
    fn example_default(self) -> &'static str {
        match self {
            Kind::Led => "default: false",
            _ => "default: 0 V",
        }
    }
}

/// A value that a `ui.set` gives an element that its wires give a type to.
#[derive(Clone, Copy)]
struct Writer<'a> {
    value: &'a Operand,
    found: TyId,
    statement: &'a Statement,
}

/// An element, with what it has so far.
pub(super) struct Element<'a> {
    item: &'a PanelItem,
    /// `None` for an element that is never read, until the walk has seen its wires.
    ty: Option<TyId>,
    /// The `default:` that gave its type.
    default: Option<&'a Operand>,
    /// The types of the wires that say what is wrong with its `default:`.
    against_default: HashSet<TyId>,
    wires: Vec<Writer<'a>>,
    constants: Vec<Writer<'a>>,
}

/// What a node of `ui` holds.
pub(in crate::declare) struct UiNode<'a> {
    pub(in crate::declare) element: Id,
    pub(in crate::declare) value: Option<&'a Operand>,
    pub(in crate::declare) is_set: bool,
}

/// The node of `ui` that this call is, when it names a known element.
pub(in crate::declare) fn ui_node<'a>(call: &'a Call, resolved: &Resolved, names: &Interner) -> Option<UiNode<'a>> {
    let bound = resolved.call(call.id)?;
    let Meaning::Primitive { library: PrimitiveLibrary::Ui, name } = bound.callee else { return None };
    let mut element = None;
    let mut value = None;
    for (arg, terminal) in call.args.iter().zip(&bound.terminals) {
        match (arg, terminal) {
            (Arg::Value { value: Operand::Path(path), .. }, Terminal::Param(0)) => {
                if let Meaning::Element(id) = resolved.of(path.id) {
                    element = Some(id);
                }
            }
            (Arg::Value { value: operand, .. }, Terminal::Param(1)) => value = Some(operand),
            _ => {}
        }
    }
    element.map(|element| UiNode { element, value, is_set: names.resolve(name) == "set" })
}

/// The names of the items of a panel that are given more than once.
pub(in crate::declare) fn names_given_twice(items: &[PanelItem]) -> HashSet<Name> {
    let mut seen = HashSet::with_capacity(items.len());
    items.iter().filter(|item| !seen.insert(item.name)).map(|item| item.name).collect()
}

/// A constant that a `default:` does not hold: it holds a number, a text, a boolean, a variant without values, or an array of those (10).
pub(in crate::declare) fn refused_constant(value: &Operand, resolved: &Resolved) -> bool {
    match value {
        Operand::Array(_, values) => values.iter().any(|value| refused_constant(value, resolved)),
        Operand::None(_) => true,
        // `[1]` is an array of one number, and what is neither has its diagnostic.
        Operand::Bracket(id, _) => matches!(resolved.of(*id), Meaning::ArrayOfFunction { .. }),
        Operand::Path(path) => matches!(resolved.of(path.id), Meaning::Function { .. } | Meaning::Primitive { .. }),
        Operand::Number(..) | Operand::Text(..) | Operand::Bool(..) => false,
    }
}

/// The `default:` of an item, the first one if it has two, which has its diagnostic.
fn default_of<'a>(item: &'a PanelItem, names: &Interner) -> Option<&'a Operand> {
    item.args.iter().flatten().find_map(|arg| match arg {
        Arg::Value { name: Some(name), value, .. } if names.resolve(*name) == "default" => Some(value),
        _ => None,
    })
}

impl<'a> Typer<'_, '_, '_, 'a> {
    /// What the name, the kind or the `default:` of each element gives, before the diagram is walked. `read` and `written` say which
    /// elements have a `ui.get` and a `ui.set`.
    pub(super) fn open_panel(&mut self, signature: &'a Signature, read: &HashSet<Id>) {
        let Some(panel) = &self.scope.model.file.panel else { return };
        let mut inputs: HashMap<Name, TyId> = HashMap::with_capacity(signature.params.len());
        for param in &signature.params {
            inputs.entry(param.name).or_insert_with(|| self.resolved.ty(param.id));
        }
        let mut outputs: HashMap<Name, TyId> = HashMap::with_capacity(signature.outputs.len());
        for output in &signature.outputs {
            outputs.entry(output.name).or_insert_with(|| self.resolved.ty(output.id));
        }
        let twice = names_given_twice(&panel.items);
        let mut seen = HashSet::with_capacity(panel.items.len());
        for item in &panel.items {
            if !seen.insert(item.name) {
                continue;
            }
            // It has its `LC5001`, and which item counts would depend on their order, which `fmt` changes.
            if twice.contains(&item.name) {
                let element =
                    Element { item, ty: Some(TyId::ERROR), default: None, against_default: HashSet::new(), wires: Vec::new(), constants: Vec::new() };
                self.elements.insert(item.id, element);
                continue;
            }
            // What pass 3 refused has its diagnostic.
            let default = default_of(item, self.pass.names);
            let refused = default.is_some_and(|default| refused_constant(default, self.resolved));
            let default = default.filter(|_| !refused);
            let kind = Kind::of(self.text(item.kind));
            // Of an input and an output of one name, which can differ: a control reads the input, an indicator shows the output (10).
            let (first, second) = match item.direction {
                PanelDirection::Control => (&inputs, &outputs),
                PanelDirection::Indicator => (&outputs, &inputs),
            };
            let mut gave_default = None;
            let ty = if let Some(ty) = first.get(&item.name).or_else(|| second.get(&item.name)).copied() {
                // Its `default:` has its `LC5012`.
                self.judge_kind_of_name(item, kind, ty);
                Some(ty)
            } else if let Some(ty) = match kind {
                Some(Kind::Button | Kind::Switch) => Some(self.plain(Ty::Bool)),
                Some(Kind::TextBox) => Some(self.plain(Ty::String)),
                _ => None,
            } {
                if let Some(default) = default {
                    self.judge_panel_default(item.name, default, ty);
                }
                Some(ty)
            } else if refused {
                Some(TyId::ERROR)
            } else if let Some(default) = default {
                gave_default = Some(default);
                Some(self.type_of_default(item, default, kind))
            } else if read.contains(&item.id) {
                // What it gives stays unknown: a kind that M0 does not know has only its warning.
                if let Some(kind) = kind.filter(|_| self.function.is_some()) {
                    let name = cut_text(self.text(item.name)).into_owned();
                    let message = format!(
                        "`{name}` is read by a `ui.get`, so it has to say its type: it is neither an input nor an output, its kind holds several, and it has no `default:` (10)"
                    );
                    self.say_without_type(item, message, &name, kind);
                }
                Some(TyId::ERROR)
            } else {
                None
            };
            if let Some(ty) = ty {
                self.resolved.set_type(item.id, ty);
            }
            let element = Element { item, ty, default: gave_default, against_default: HashSet::new(), wires: Vec::new(), constants: Vec::new() };
            self.elements.insert(item.id, element);
            self.element_order.push(item.id);
        }
    }

    fn say_without_type(&mut self, item: &'a PanelItem, message: String, name: &str, kind: Kind) {
        let fixes =
            vec![Fix { title: format!("Give `{name}` a `default:` that says its type, such as `{}`", kind.example_default()), edits: Vec::new() }];
        self.say(Code::Lc5010, message, item.id, Details { fixes, ..Details::default() });
    }

    /// What the wires of an element that is never read give, once the walk has seen them all. A `ui.set` that the walk did not reach is
    /// behind a cycle of the diagram, which pass 5 says (`LC4001`), so `written` is what the plan found and not what the walk saw. A mistake
    /// in this function can be a `ui.set` that nobody could bind: then "no `ui.set`" is not said.
    pub(super) fn close_panel(&mut self, written: &HashSet<Id>) {
        (self.statement, self.node) = (None, None);
        let clean = self.errors() == self.pass.function_errors;
        for id in std::mem::take(&mut self.element_order) {
            let Some(element) = self.elements.get_mut(&id) else { continue };
            if element.ty.is_some() {
                let (item, default, against) = (element.item, element.default, std::mem::take(&mut element.against_default));
                if let Some(default) = default {
                    self.judge_default_against(item, default, against);
                }
                continue;
            }
            let (item, wires, mut constants) = (element.item, std::mem::take(&mut element.wires), std::mem::take(&mut element.constants));
            let kind = Kind::of(self.text(item.kind));
            if !written.contains(&id) {
                // An `extern` has no diagram to write it.
                if let Some(kind) = kind.filter(|_| clean && self.function.is_some()) {
                    let name = cut_text(self.text(item.name)).into_owned();
                    let message = format!(
                        "nothing gives `{name}` a type: it is neither an input nor an output, its kind holds several, it has no `default:`, and no `ui.set` writes it (10)"
                    );
                    self.say_without_type(item, message, &name, kind);
                }
                continue;
            }
            let ty = match self.type_of_values(item, kind, &wires) {
                Some(ty) => {
                    for constant in &constants {
                        self.at_node(constant.statement);
                        // Without a type to go to, it still says what is wrong with it.
                        if ty == TyId::ERROR {
                            self.value_of(constant.value, None);
                        } else {
                            self.check_input(constant.value, ty, "value");
                        }
                    }
                    ty
                }
                None => {
                    for constant in &mut constants {
                        self.at_node(constant.statement);
                        constant.found = self.value_of(constant.value, None);
                    }
                    self.type_of_values(item, kind, &constants).unwrap_or(TyId::ERROR)
                }
            };
            (self.statement, self.node) = (None, None);
            self.resolved.set_type(item.id, ty);
        }
    }

    /// `default: 0.5` and wires with a unit, or `default: 0` and decimal wires: one diagnostic, with the type that comes first by its text,
    /// since no wire comes first.
    fn judge_default_against(&mut self, item: &'a PanelItem, default: &'a Operand, against: HashSet<TyId>) {
        let first = against.into_iter().map(|ty| (self.show(ty), ty)).min_by(|a, b| a.0.cmp(&b.0));
        let Some((_, ty)) = first else { return };
        if self.number_of(ty).is_some_and(|(_, unit)| unit == Unit::ONE) {
            self.say_whole_default(item, default, ty);
        } else {
            self.judge_panel_default(item.name, default, ty);
        }
    }

    /// `default: 0` is an integer, and the developer who wires decimals to it meant `0.0` (3.4).
    fn say_whole_default(&mut self, item: &'a PanelItem, default: &'a Operand, wire: TyId) {
        let Operand::Number(_, numeral, _) = default else { return };
        let (name, expected, found) = (cut_text(self.text(item.name)).into_owned(), self.show(wire), self.show(self.resolved.ty(item.id)));
        let message = format!("`{name}` takes the type `{found}` from its `default:`, and a wire brings `{expected}`");
        let written = self.literal_text(*numeral).map(|text| format!("{text}.0"));
        let title = written.as_ref().map_or_else(
            || "Write this number with a decimal point (3.4)".to_owned(),
            |written| format!("Write `{written}`: a number without a decimal point is an integer (3.4)"),
        );
        let span = self.scope.model.source_map.span(default.id());
        let edits = written.zip(span).map_or_else(Vec::new, |(text, span)| vec![Edit { span, text }]);
        let wires = vec![self.wire_of(default, format!("panel.{name}"))];
        let fixes = vec![Fix { title, edits }];
        let details = Details { wires, expected: Some(expected), found: Some(found), fixes, ..Details::default() };
        self.say(Code::Lc3017, message, default.id(), details);
    }

    /// How many errors the library has so far: what was just judged may have said something.
    fn errors(&self) -> u64 {
        self.pass.reports.get(self.scope.library).map_or(0, labcraft_types::Report::errors)
    }

    fn at_node(&mut self, statement: &'a Statement) {
        self.statement = Some(statement);
        self.node = None;
    }

    /// The type that these values agree on, or `None` when none of them has one. No value is first: each one that the kind refuses is wrong
    /// by itself, and those that differ are a mistake of the element, which then has no type (`TyId::ERROR`).
    fn type_of_values(&mut self, item: &'a PanelItem, kind: Option<Kind>, values: &[Writer<'a>]) -> Option<TyId> {
        let mut accepted: Vec<Writer<'a>> = Vec::new();
        for value in values {
            // Unknown, also in part, as in `[x]`: it has its diagnostic, or the file has one, and it says nothing of the others.
            if self.pass.table.has_error(value.found) {
                continue;
            }
            if kind.is_some_and(|kind| !self.kind_accepts(kind, value.found)) {
                self.refuse(item, kind, value);
                continue;
            }
            accepted.push(*value);
        }
        let known = accepted.first()?.found;
        if accepted.iter().all(|value| self.pass.table.matches(known, value.found)) {
            return Some(known);
        }
        (self.statement, self.node) = (None, None);
        self.values_differ(item, &accepted);
        Some(TyId::ERROR)
    }

    fn judge_kind_of_name(&mut self, item: &'a PanelItem, kind: Option<Kind>, ty: TyId) {
        let Some(kind) = kind else { return };
        if self.kind_accepts(kind, ty) {
            return;
        }
        let (name, shown, shows) = (cut_text(self.text(item.name)).into_owned(), self.show(ty), kind.shows());
        let kind_text = self.text(item.kind);
        let message = format!("`{name}` has the type `{shown}` from the signature, and a `{kind_text}` shows {shows}");
        self.say(Code::Lc5011, message, item.id, Details { found: Some(shown), ..Details::default() });
    }

    /// Whether a widget of this kind can show this type. What is unknown has its diagnostic.
    fn kind_accepts(&mut self, kind: Kind, ty: TyId) -> bool {
        if self.pass.table.has_error(ty) {
            return true;
        }
        let definition = definition_of(self.pass, self.context, ty);
        match kind {
            Kind::Numeric => self.is_number(definition),
            Kind::TextBox => matches!(self.pass.table.get(definition), Ty::String),
            Kind::Button | Kind::Switch => matches!(self.pass.table.get(definition), Ty::Bool),
            Kind::Led => matches!(self.pass.table.get(definition), Ty::Bool | Ty::Enum(_)),
            Kind::Chart => match self.pass.table.get(definition) {
                Ty::Waveform(_) => true,
                Ty::Array { element, dimensions } => {
                    let (element, dimensions) = (*element, *dimensions);
                    let element = definition_of(self.pass, self.context, element);
                    dimensions <= 2 && self.is_number(element)
                }
                _ => self.is_number(definition),
            },
        }
    }

    /// A `duration` counts as an `f64[s]` (3.5).
    fn is_number(&self, definition: TyId) -> bool {
        matches!(self.pass.table.get(definition), Ty::Number { .. } | Ty::Duration)
    }

    fn judge_panel_default(&mut self, name: Name, default: &'a Operand, ty: TyId) {
        let around = std::mem::replace(&mut self.within, Within::Panel(name));
        let before = self.errors();
        let found = self.value_of(default, Some(ty));
        // What is wrong with it has been said.
        if self.errors() == before {
            self.judge_piece(default, found, ty, self.pass.names.resolve(name));
        }
        self.within = around;
    }

    /// `LC3001` on the value, with its node.
    fn refuse(&mut self, item: &'a PanelItem, kind: Option<Kind>, value: &Writer<'a>) {
        let Some(kind) = kind else { return };
        self.at_node(value.statement);
        let (name, found, shows) = (cut_text(self.text(item.name)).into_owned(), self.show(value.found), kind.shows());
        let message = format!("`{name}` is a `{}`, which shows {shows}, and this is `{found}`", self.text(item.kind));
        let wires = vec![self.wire_of(value.value, self.input_terminal("value"))];
        self.say(Code::Lc3001, message, value.value.id(), Details { wires, found: Some(found), ..Details::default() });
    }

    /// By the text of the types, then of the wires, since no line comes first: `fmt` changes nothing here. One wire of each type, then others.
    fn values_differ(&mut self, item: &'a PanelItem, values: &[Writer<'a>]) {
        let mut texts: HashMap<TyId, String> = HashMap::new();
        let mut entries: Vec<(String, Option<String>, Option<String>, Wire)> = Vec::with_capacity(values.len());
        for value in values {
            let text = texts.entry(value.found).or_insert_with(|| self.pass.table.display(value.found, self.viewer, self.pass.names)).clone();
            let wire = self.wire_of(value.value, format!("{}.value", self.node_prefix(value.statement)));
            entries.push((text, wire.value.clone(), wire.to.clone(), wire));
        }
        entries.sort_by(|a, b| (&a.0, &a.1, &a.2).cmp(&(&b.0, &b.1, &b.2)));
        let mut first_of_type = Vec::new();
        let mut others = Vec::new();
        for (index, entry) in entries.iter().enumerate() {
            if index == 0 || entries.get(index - 1).is_some_and(|before| before.0 != entry.0) {
                first_of_type.push(index);
            } else {
                others.push(index);
            }
        }
        let text_at = |place: usize| first_of_type.get(place).and_then(|index| entries.get(*index)).map_or_else(String::new, |entry| entry.0.clone());
        let (expected, found) = (text_at(0), text_at(1));
        let name = cut_text(self.text(item.name)).into_owned();
        let message = format!(
            "`{name}` receives values of different types, `{expected}` and `{found}`: an element has one type, and the diagram does not say which one comes first. A `default:` would say it (10)"
        );
        let wires: Vec<Wire> =
            first_of_type.iter().chain(&others).take(WIRES_SHOWN).filter_map(|index| entries.get(*index)).map(|entry| entry.3.clone()).collect();
        self.say(Code::Lc5013, message, item.id, Details { wires, expected: Some(expected), found: Some(found), ..Details::default() });
    }

    /// The type of a `default:` alone, which goes with the kind.
    fn type_of_default(&mut self, item: &'a PanelItem, default: &'a Operand, kind: Option<Kind>) -> TyId {
        let around = std::mem::replace(&mut self.within, Within::Panel(item.name));
        let before = self.errors();
        let ty = self.value_of(default, None);
        self.within = around;
        // What is wrong with it has been said.
        if self.errors() != before {
            return TyId::ERROR;
        }
        if ty != TyId::ERROR && kind.is_some_and(|kind| !self.kind_accepts(kind, ty)) {
            let shows = kind.map_or("", Kind::shows);
            let (name, found) = (cut_text(self.text(item.name)).into_owned(), self.show(ty));
            let message = format!("`{name}` is a `{}`, which shows {shows}, and this is `{found}`", self.text(item.kind));
            let wires = vec![Wire { value: None, from: None, to: Some(format!("panel.{name}")) }];
            self.say(Code::Lc3017, message, default.id(), Details { wires, found: Some(found), ..Details::default() });
            return TyId::ERROR;
        }
        ty
    }

    /// A node of `ui`. It gives the type of its element, or for an element that its wires give a type to, what this node was given (10).
    pub(super) fn ui_call(&mut self, call: &'a Call) -> Vec<Out<'a>> {
        let Some(node) = ui_node(call, self.resolved, self.pass.names) else {
            // An element that is not known says nothing of the value, whose names and numbers are still read.
            for arg in &call.args {
                if let Arg::Value { value, .. } = arg {
                    self.value_of(value, Some(TyId::ERROR));
                }
            }
            return Vec::new();
        };
        let written = self.resolved.call(call.id).and_then(|bound| bound.type_args.first().copied());
        let ty = match self.elements.get(&node.element).map(|element| element.ty) {
            Some(Some(ty)) => self.at_known_element(call, &node, ty, written),
            Some(None) => self.at_free_element(&node, written),
            None => TyId::ERROR,
        };
        vec![Out { ty, label: Label::Text("value"), operand: None }]
    }

    fn at_known_element(&mut self, call: &Call, node: &UiNode<'a>, ty: TyId, written: Option<TyId>) -> TyId {
        // What a call writes has to be the type of the element, which it does not give (appendix A). Which of the two was meant is not said.
        if let Some(written) = written.filter(|written| !self.pass.table.matches(*written, ty)) {
            let (expected, found) = (self.show(ty), self.show(written));
            let message = format!("this element has the type `{expected}`, and the call says `{found}`");
            self.say(Code::Lc3001, message, call.id, Details { expected: Some(expected), found: Some(found), ..Details::default() });
            if let Some(value) = node.value {
                self.value_of(value, Some(TyId::ERROR));
            }
            return TyId::ERROR;
        }
        let Some(value) = node.value else { return ty };
        let found = self.value_of(value, Some(ty));
        if self.blame_the_default(node.element, value, found, ty) {
            // Writing the unit answers this too.
            return TyId::ERROR;
        }
        self.judge_input(value, found, ty, "value");
        ty
    }

    /// `default: 0.5` and a wire of `f64[V]`: what is wrong is the number without a unit, and `0.5 V` says what was meant (3.4). `default: 0`
    /// and a wire of `f64`: what is wrong is the number without decimals, and `0.0` says what was meant. A `duration` counts as an `f64[s]`
    /// (3.5). A default that has its own mistake has no type to compare.
    fn blame_the_default(&mut self, id: Id, value: &'a Operand, found: TyId, ty: TyId) -> bool {
        let Some(default) = self.elements.get(&id).and_then(|element| element.default) else { return false };
        let as_number = self.as_number(found);
        let is_whole = matches!(default, Operand::Number(_, Numeral { kind: NumeralKind::Int(_), .. }, None));
        let blamed = as_number
            .and_then(|number| self.number_of(number))
            .is_some_and(|(kind, unit)| unit != Unit::ONE || (is_whole && kind.is_inexact() && !kind.is_complex()));
        if ty == TyId::ERROR || !self.is_bare_number(default) || !blamed || !self.holds_wire(value) {
            return false;
        }
        if let (Some(element), Some(number)) = (self.elements.get_mut(&id), as_number) {
            element.against_default.insert(number);
        }
        true
    }

    /// A `ui.set` of an element that is never read. Its wires are compared when the walk ends. A constant takes their type then, and until
    /// that it gives what it is. A type that the call writes is what this value is, as on any generic primitive (3.5).
    fn at_free_element(&mut self, node: &UiNode<'a>, written: Option<TyId>) -> TyId {
        let (Some(value), Some(statement)) = (node.value, self.statement) else { return TyId::ERROR };
        let is_wire = written.is_some() || self.holds_wire(value);
        let found = if let Some(written) = written {
            let before = self.errors();
            self.check_input(value, written, "value");
            // What is wrong with it has been said, and its kind has nothing to add.
            if self.errors() == before { written } else { TyId::ERROR }
        } else if is_wire {
            self.value_of(value, None)
        } else {
            let outer = self.trying.replace(false);
            let own = self.value_of(value, None);
            self.trying = outer;
            own
        };
        if let Some(element) = self.elements.get_mut(&node.element) {
            let writer = Writer { value, found, statement };
            if is_wire { element.wires.push(writer) } else { element.constants.push(writer) }
        }
        found
    }
}
