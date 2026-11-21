//! The catalog of quick insertion and of the panel `Palette` (notation 2.6, ide.md 4): what the project can call, read
//! from the inputs and outputs that `functions.list` gives with each entry (commands.md 6.1), so that the list opened
//! from a wire holds only what takes it; no signature text is read here. The filter is the predicate of `wire.connect`
//! as the canvas judges a wire (`labcraft_diagram::wiring`): the same rule of fit (spec 3.3, 3.4), on what the engine
//! says the rule reads of each type; the command judges the insertion again (architecture 12.6). The order is
//! deterministic and works offline: what takes the type, those whose first required input takes it first, then the best
//! match of the search, then what the user inserted most recently, then the alphabet. No model ranks the list and
//! nothing asks the network.

use std::collections::{BTreeMap, HashMap};

use labcraft_diagram::Drawing;
use labcraft_types::listing::{Slot as ListedSlot, SlotFit, Typed};
use labcraft_types::{Bound, Fit, FitKey, MAX_NESTING, NumberKind, Ty, Unit, parse_unit};

/// An entry of `functions.list`, as the host reads it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listed {
    pub name: String,
    pub library: String,
    /// `function` or `type`.
    pub kind: String,
    pub signature: String,
    /// Its inputs and outputs, or the fields, variants or alias of a type, as `functions.list` gives them.
    pub typed: Typed,
}

/// What a row of the list inserts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    /// A call of a function.
    Function,
    /// A construction of a struct type (spec 5.1).
    Struct,
    /// A variant that holds values.
    Variant,
    /// A variant without values, a constant (`Verdict.Pass`).
    Constant,
}

/// What an input or an output takes or gives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SlotTy {
    /// A type of no parts, judged by the rule of fit.
    Key(FitKey),
    /// Any other type, by its text with its aliases followed (`relays.bank` is a `visa.session`).
    Named(String),
    /// A type variable bound to numbers (`N: Number`), or what such numbers give (`A·B`, `√N`).
    Numbers,
    /// A type variable bound to the integers without a unit (`I: Integer`), or to them and `bool` (`B: Bits`, spec 3.5).
    Bounded(Bound),
    /// A type variable without a bound, or a type the catalog cannot read: anything.
    Any,
    /// An array of a type variable: any array.
    AnyArray,
    /// What takes only text written in the call: the template of `string.format`, a `kind` (spec 5.4, A.1). Never a wire.
    Literal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slot {
    pub name: String,
    pub ty: SlotTy,
    pub required: bool,
    pub variadic: bool,
}

/// One row of the catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// As `node.add` takes it: `visa.open`, `settle`, `Step`, `Reading.Value`; for a constant, its text.
    pub call: String,
    /// `""` for the project itself.
    pub library: String,
    pub signature: String,
    pub kind: EntryKind,
    pub inputs: Vec<Slot>,
    pub outputs: Vec<Slot>,
}

/// The catalog, from the entries of `functions.list`.
#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub entries: Vec<Entry>,
    /// Each alias to what it names (`psu.session` to `visa.session`).
    aliases: HashMap<String, String>,
    /// Each call to the index of its entry, built once.
    by_call: HashMap<String, usize>,
}

/// The wire that the list is opened from: its type, and which way it goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wanted {
    /// True when the wire starts at an output: the node must take it at an input. False when it was drawn back from an
    /// input: the node must give it at an output.
    pub from_output: bool,
    pub key: Option<FitKey>,
    /// Its type as the language writes it; empty when the check does not know it.
    pub text: String,
    /// Its family, as the drawing writes it (`Number/0`), for a parameter of a primitive.
    pub family: String,
}

impl Wanted {
    /// The wire at a terminal of the drawing, by its index in [`Drawing::ports`]: none for a terminal that a wire cannot
    /// start from.
    pub fn of_port(drawing: &Drawing, port: usize) -> Option<Wanted> {
        let port = drawing.ports.get(port)?;
        if port.order || port.name.is_empty() || !port.never.is_empty() {
            return None;
        }
        let key = port.ty.and_then(|ty| drawing.type_keys.get(ty)).copied();
        let text = port.ty.and_then(|ty| drawing.types.get(ty)).cloned().unwrap_or_default();
        Some(Wanted { from_output: !port.input, key, text, family: port.family.clone() })
    }

    /// The value of a wire of the drawing, by its group: what a node inserted into it takes and gives back.
    pub fn of_wire(drawing: &Drawing, group: usize) -> Option<Wanted> {
        let wire = drawing.groups.get(group)?.wire.as_ref()?;
        let port = drawing.ports.iter().position(|port| !port.input && port.group == Some(group) && port.value == wire.value)?;
        Wanted::of_port(drawing, port)
    }
}

/// What a row inserts, in the order of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    /// "constant `240 V`": the text typed, which reads as a constant (`constant.add`).
    Typed(String),
    /// An entry of the catalog, by its index.
    Entry(usize),
}

impl Catalog {
    pub fn new(listed: &[Listed]) -> Catalog {
        let mut aliases = HashMap::new();
        let mut entries = Vec::new();
        for item in listed {
            let name = full_name(item);
            let typed = &item.typed;
            let entry = |call: String, kind: EntryKind, inputs: Vec<Slot>| Entry {
                call,
                library: item.library.clone(),
                signature: item.signature.clone(),
                kind,
                inputs,
                outputs: Vec::new(),
            };
            if item.kind != "type" {
                entries.push(Entry {
                    inputs: typed.inputs.iter().map(slot).collect(),
                    outputs: typed.outputs.iter().map(slot).collect(),
                    ..entry(name, EntryKind::Function, Vec::new())
                });
                continue;
            }
            let itself = Slot { name: name.clone(), ty: SlotTy::Named(name.clone()), required: true, variadic: false };
            if !typed.fields.is_empty() {
                let fields = typed.fields.iter().map(|field| Slot { required: true, ..slot(field) }).collect();
                entries.push(Entry { outputs: vec![itself], ..entry(name, EntryKind::Struct, fields) });
            } else if !typed.variants.is_empty() {
                for variant in &typed.variants {
                    let call = format!("{name}.{}", variant.name);
                    let (kind, values) = if variant.values.is_empty() {
                        (EntryKind::Constant, Vec::new())
                    } else {
                        (EntryKind::Variant, variant.values.iter().map(|value| Slot { required: true, ..slot(value) }).collect())
                    };
                    entries.push(Entry { outputs: vec![itself.clone()], ..entry(call, kind, values) });
                }
            } else if let Some(is) = &typed.is {
                let named = match &is.fit {
                    Some(SlotFit::Shape { shape }) => shape.clone(),
                    _ => is.ty.clone(),
                };
                aliases.insert(name, named);
            }
        }
        entries.sort_by(|a, b| a.call.cmp(&b.call));
        let by_call = entries.iter().enumerate().map(|(index, entry)| (entry.call.clone(), index)).collect();
        let mut catalog = Catalog { entries, aliases, by_call };
        let aliases = catalog.aliases.clone();
        for entry in &mut catalog.entries {
            for slot in entry.inputs.iter_mut().chain(entry.outputs.iter_mut()) {
                if let SlotTy::Named(text) = &slot.ty {
                    slot.ty = SlotTy::Named(follow(&aliases, text));
                }
            }
        }
        catalog
    }

    /// The input of the entry that takes the wire, or the output that gives it, by its index: the first one. For a wire
    /// drawn back from an input, the output whose value goes to it.
    pub fn fits(&self, entry: &Entry, wanted: &Wanted) -> Option<usize> {
        if wanted.from_output {
            entry.inputs.iter().position(|input| !input.variadic && self.takes(&input.ty, wanted))
        } else {
            entry.outputs.iter().position(|output| self.gives(&output.ty, wanted))
        }
    }

    /// Whether the entry goes into a wire of this value: an input takes it and an output gives it back
    /// (`node.add` with `into`).
    pub fn goes_into(&self, entry: &Entry, wanted: &Wanted) -> bool {
        let taken = Wanted { from_output: true, ..wanted.clone() };
        let given = Wanted { from_output: false, ..wanted.clone() };
        entry.kind != EntryKind::Constant && self.fits(entry, &taken).is_some() && self.fits(entry, &given).is_some()
    }

    /// Whether an input of this type takes the value.
    fn takes(&self, ty: &SlotTy, wanted: &Wanted) -> bool {
        match ty {
            SlotTy::Literal => false,
            SlotTy::Any => true,
            SlotTy::Numbers => match wanted.key {
                Some(FitKey::Number(..)) => true,
                Some(_) => false,
                None => wanted.family.is_empty() || wanted.family.starts_with("Number/0"),
            },
            SlotTy::Bounded(bound) => bits_hold(*bound, wanted),
            SlotTy::AnyArray => wanted.text.is_empty() || wanted.text.starts_with('['),
            SlotTy::Key(expected) => match wanted.key {
                Some(
                    found @ (FitKey::Number(..)
                    | FitKey::Duration
                    | FitKey::Bool
                    | FitKey::String
                    | FitKey::Bytes
                    | FitKey::Timestamp
                    | FitKey::Error),
                ) => matches!(found.fit(expected), Fit::Same | Fit::Coerce),
                // A type that the check does not know takes anything, as the canvas judges it.
                _ => wanted.text.is_empty(),
            },
            SlotTy::Named(text) => wanted.text.is_empty() || follow(&self.aliases, &wanted.text) == *text,
        }
    }

    /// Whether an output of this type goes to the input the wire was drawn from.
    fn gives(&self, ty: &SlotTy, wanted: &Wanted) -> bool {
        match ty {
            SlotTy::Literal => false,
            SlotTy::Any => true,
            SlotTy::Numbers => match wanted.key {
                Some(key) => matches!(key, FitKey::Number(..)),
                None => wanted.family.is_empty() || wanted.family.starts_with("Number/") || wanted.family.starts_with("Unknown/0"),
            },
            SlotTy::Bounded(bound) => bits_give(*bound, wanted),
            SlotTy::AnyArray => wanted.text.is_empty() || wanted.text.starts_with('['),
            SlotTy::Key(found) => match wanted.key {
                Some(expected) => matches!(found.fit(&expected), Fit::Same | Fit::Coerce),
                None => wanted.family.is_empty() || family_of(found).is_none_or(|family| wanted.family.starts_with(family)),
            },
            SlotTy::Named(text) => wanted.text.is_empty() || follow(&self.aliases, &wanted.text) == *text,
        }
    }

    /// The rows of the list for a search, in their order (notation 2.6). With a wire, only what takes it, the entries
    /// whose first required input takes it first; then the best match of the search; then what `recent` holds, the most
    /// recent first; then the alphabet. Text that reads as a constant puts it first, when the wire takes one.
    pub fn rows(&self, query: &str, wanted: Option<&Wanted>, recent: &[String]) -> Vec<Row> {
        let query = query.trim();
        let mut rows = Vec::new();
        if let Some(ty) = constant_type(query)
            && wanted.is_none_or(|wanted| !wanted.from_output && self.gives(&ty, wanted))
        {
            rows.push(Row::Typed(query.to_owned()));
        }
        let mut found: Vec<(u8, i64, usize, usize)> = Vec::new();
        for (index, entry) in self.entries.iter().enumerate() {
            let tier = match wanted {
                None => 0,
                Some(wanted) => match self.fits(entry, wanted) {
                    None => continue,
                    Some(at) if wanted.from_output && entry.inputs.iter().position(|input| input.required && !input.variadic) == Some(at) => 0,
                    Some(_) if wanted.from_output => 1,
                    Some(_) => 0,
                },
            };
            // A constant has no input: a wire from an output never goes to one.
            if entry.kind == EntryKind::Constant && wanted.is_some_and(|wanted| wanted.from_output) {
                continue;
            }
            let score = if query.is_empty() {
                0
            } else {
                match Catalog::search(query, &entry.call, &entry.library) {
                    Some(score) => score,
                    None => continue,
                }
            };
            let recency = recent.iter().position(|call| *call == entry.call).unwrap_or(usize::MAX);
            found.push((tier, score, recency, index));
        }
        found.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(&a.1)).then_with(|| a.2.cmp(&b.2)).then_with(|| a.3.cmp(&b.3)));
        rows.extend(found.into_iter().map(|(_, _, _, index)| Row::Entry(index)));
        rows
    }

    /// The entry of a call, as `node.add` names it.
    pub fn entry(&self, call: &str) -> Option<&Entry> {
        self.index_of(call).and_then(|index| self.entries.get(index))
    }

    /// The index of the entry of `call`.
    pub fn index_of(&self, call: &str) -> Option<usize> {
        self.by_call.get(call).copied()
    }

    /// The score of `call` and its library for a search, as the list ranks them (the panel `Palette` too): None when the
    /// query does not match.
    pub fn search(query: &str, call: &str, library: &str) -> Option<i64> {
        [score(query, call), score(query, library)].into_iter().flatten().max()
    }
}

/// The fuzzy score of quick insertion and of the panel `Palette`: the letters of the query in order, a start of a word
/// and letters in a row counting more.
fn score(query: &str, text: &str) -> Option<i64> {
    let query: Vec<char> = query.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect();
    if query.is_empty() {
        return Some(0);
    }
    let text: Vec<char> = text.chars().collect();
    let mut total: i64 = 0;
    let mut at = 0;
    let mut previous: Option<usize> = None;
    for wanted in &query {
        let found = (at..text.len()).find(|&index| text.get(index).is_some_and(|c| c.to_lowercase().eq(std::iter::once(*wanted))))?;
        let starts_word = found == 0 || text.get(found - 1).is_some_and(|before| !before.is_alphanumeric());
        total += if starts_word { 8 } else { 1 };
        if previous.is_some_and(|previous| previous + 1 == found) {
            total += 4;
        }
        total -= i64::try_from(found - at).unwrap_or(16).min(16);
        previous = Some(found);
        at = found + 1;
    }
    Some(total * 64 - i64::try_from(text.len()).unwrap_or(63).min(63))
}

/// The family that a type of no parts belongs to, as the drawing writes it.
fn family_of(key: &FitKey) -> Option<&'static str> {
    Some(match key {
        FitKey::Number(..) => "Number/",
        FitKey::Bool => "Bool/",
        FitKey::String => "Text/",
        FitKey::Error => "Error/",
        _ => return None,
    })
}

/// The type of a text that reads as a constant (spec 5.1): a number with or without a unit, a string, `true` or
/// `false`, an array. None for anything else: the command reads it again, and refuses what is not one.
pub fn constant_type(text: &str) -> Option<SlotTy> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if text == "true" || text == "false" {
        return Some(SlotTy::Key(FitKey::Bool));
    }
    if text.len() >= 2 && text.starts_with('"') && text.ends_with('"') {
        return Some(SlotTy::Key(FitKey::String));
    }
    if text.starts_with('[') && text.ends_with(']') {
        return Some(SlotTy::AnyArray);
    }
    let digits = text.strip_prefix('-').unwrap_or(text);
    // `0x00FF`, `0b1010_0001 V` (spec 3.4): an integer, with or without a unit.
    let (word, after) = digits.split_once(' ').unwrap_or((digits, ""));
    if !labcraft_types::integer::is_decimal(word) && labcraft_types::integer::is_integer(word) {
        let after = after.trim();
        if after.is_empty() {
            return Some(SlotTy::Key(FitKey::Number(NumberKind::I32, Unit::ONE)));
        }
        return Some(SlotTy::Key(FitKey::Number(NumberKind::F64, parse_unit(after, MAX_NESTING).ok()?)));
    }
    let number: String = digits.chars().take_while(|c| c.is_ascii_digit() || matches!(c, '.' | '_' | 'e' | 'E')).collect();
    if !number.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    let rest = digits.get(number.len()..).unwrap_or_default().trim();
    let kind = if number.contains(['.', 'e', 'E']) { NumberKind::F64 } else { NumberKind::I32 };
    if rest.is_empty() {
        return Some(SlotTy::Key(FitKey::Number(kind, Unit::ONE)));
    }
    let unit = parse_unit(rest, MAX_NESTING).ok()?;
    Some(SlotTy::Key(FitKey::Number(NumberKind::F64, unit)))
}

/// `visa.open`, or `settle` for the project itself.
fn full_name(item: &Listed) -> String {
    if item.library.is_empty() || item.name.contains('.') { item.name.clone() } else { format!("{}.{}", item.library, item.name) }
}

/// Whether a variable bound by `Integer` or `Bits` holds the value (spec 3.5), as [`Bound::holds`] says of its type: an
/// integer without a unit, and a `bool` for `Bits`. A wire whose type is not known yet goes by its family.
fn bits_hold(bound: Bound, wanted: &Wanted) -> bool {
    let with_bool = bound.holds(&Ty::Bool);
    match wanted.key {
        Some(FitKey::Number(kind, unit)) => bound.holds(&Ty::Number { kind, unit }),
        Some(FitKey::Bool) => with_bool,
        Some(_) => false,
        None => wanted.family.is_empty() || wanted.family.starts_with("Number/0") || (with_bool && wanted.family.starts_with("Bool/0")),
    }
}

/// Whether an output of a variable bound by `Integer` or `Bits` goes to an input of this type: what the bound holds, or a
/// type that an integer widens to, an `f64` without a unit (spec 3.4).
fn bits_give(bound: Bound, wanted: &Wanted) -> bool {
    match wanted.key {
        Some(FitKey::Number(NumberKind::F64, unit)) => unit == Unit::ONE,
        _ => bits_hold(bound, wanted),
    }
}

/// An input, an output, a field or a value, from what `functions.list` gives of it.
fn slot(listed: &ListedSlot) -> Slot {
    Slot { name: listed.name.clone(), ty: slot_ty(listed), required: listed.required, variadic: listed.variadic }
}

/// What a slot takes or gives, from what the rule of fit reads of its type (commands.md 6.1).
fn slot_ty(listed: &ListedSlot) -> SlotTy {
    if listed.literal {
        return SlotTy::Literal;
    }
    match &listed.fit {
        None => SlotTy::Any,
        Some(fit @ SlotFit::Number { .. }) => fit.key().map_or(SlotTy::Numbers, SlotTy::Key),
        Some(fit @ SlotFit::Base { .. }) => fit.key().map_or(SlotTy::Any, SlotTy::Key),
        Some(SlotFit::Variable { bound, .. }) => match bound.as_deref().and_then(Bound::from_name) {
            Some(Bound::Number) => SlotTy::Numbers,
            Some(bound) => SlotTy::Bounded(bound),
            None => SlotTy::Any,
        },
        Some(SlotFit::Numbers { .. }) => SlotTy::Numbers,
        Some(SlotFit::Array { .. }) => SlotTy::AnyArray,
        Some(SlotFit::Shape { shape }) => SlotTy::Named(shape.clone()),
    }
}

/// An alias followed to what it names, at most 16 times.
fn follow(aliases: &HashMap<String, String>, text: &str) -> String {
    let mut now = text.to_owned();
    for _ in 0..16 {
        match aliases.get(&now) {
            Some(next) => now.clone_from(next),
            None => break,
        }
    }
    now
}

/// The insertions of a user, the most recent first, kept on their machine (notation 2.6): at most this many.
pub const RECENT: usize = 32;

/// `call` inserted now: first, once.
pub fn remember(recent: &mut Vec<String>, call: &str) {
    recent.retain(|known| known != call);
    recent.insert(0, call.to_owned());
    recent.truncate(RECENT);
}

/// The entries by library, for the panel `Palette`: the project first (`""`), then the libraries in alphabetical order.
pub fn by_library(catalog: &Catalog) -> BTreeMap<&str, Vec<usize>> {
    let mut found: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (index, entry) in catalog.entries.iter().enumerate() {
        found.entry(entry.library.as_str()).or_default().push(index);
    }
    found
}

#[cfg(test)]
mod tests;
