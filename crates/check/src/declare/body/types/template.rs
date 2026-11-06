//! The template of `string.format` (5.4): a field per `{name}`, each with its format, and an input per field. The template is on
//! the face of the node, so what is wrong with it is a mistake of that node (R-12).
//!
//! `{name:[fill align][+][#][0][width][.decimals][letter][ unit]}`, the form of C++20 with a name and a unit. Each file can have a
//! template of 16 MiB: reading it is linear, and what a type can show is made once for each type.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use labcraft_format::{is_fill, is_invisible, is_name};
use labcraft_graph::{Arg, Call, Operand};
use labcraft_types::{Code, Fix, Name, Ty, TyId, Unit, UnitError, cut_text, normalize, parse_unit};

use super::super::definition_of;
use super::coverage::read_escapes;
use super::{Details, Typer, UNIT_NESTING};
use crate::resolved::{ResolvedCall, Terminal};

const MAX_WIDTH: u32 = 1_000;
const MAX_DECIMALS: u32 = 100;
/// How many names a message gives, as for the fields of a struct.
const MAX_NAMES: usize = 5;

/// What a field asks of its value.
#[derive(Debug, PartialEq)]
struct Field<'t> {
    name: Cow<'t, str>,
    /// Decimals, a sign, zeros or a letter: what only a number has.
    needs_a_number: bool,
    /// `d`, `x`, `X`, `o`, `b`: an integer without a unit.
    needs_an_integer: bool,
    /// As a unit, and as it is written, which is what a message shows.
    unit: Option<(Unit, &'t str)>,
}

/// What a format says of its value.
#[derive(Debug, Default)]
struct Format<'t> {
    needs_a_number: bool,
    needs_an_integer: bool,
    unit: Option<(Unit, &'t str)>,
}

#[derive(Debug, PartialEq, Eq)]
struct Mistake {
    code: Code,
    message: String,
}

/// A mistake goes out as it is found: a template of 16 MiB can hold millions of them, and only the report bounds what it keeps.
type Say<'s> = &'s mut dyn FnMut(Mistake);

/// What is left of a format after one of its parts.
fn take(rest: &mut &str, c: char) -> bool {
    match rest.strip_prefix(c) {
        Some(after) => {
            *rest = after;
            true
        }
        None => false,
    }
}

/// The digits at the start, as a number, if there are any: beyond what fits, it is `u32::MAX`, which no limit accepts.
fn take_number(rest: &mut &str) -> Option<u32> {
    let after = rest.trim_start_matches(|c: char| c.is_ascii_digit());
    let digits = rest.strip_suffix(after)?;
    *rest = after;
    (!digits.is_empty()).then(|| digits.parse().unwrap_or(u32::MAX))
}

fn is_align(c: char) -> bool {
    matches!(c, '<' | '^' | '>')
}

/// The letter at the start, if nothing but a unit follows it: `{x:g}` is a letter, and `{x:gV}` a unit.
fn take_letter(rest: &mut &str) -> Option<char> {
    let mut chars = rest.chars();
    let letter = chars.next().filter(|c| matches!(c, 'd' | 'f' | 'g' | 'e' | 'E' | 'x' | 'X' | 'o' | 'b'))?;
    let after = chars.as_str();
    if !after.is_empty() && !after.starts_with(' ') {
        return None;
    }
    *rest = after;
    Some(letter)
}

/// Why a field does not show what its wire holds.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Refusal {
    NoText,
    NotAnInteger,
    NoNumber,
    OtherUnit,
}

/// `{}`, `{0}` and what follows them have no name to show: a message gives their place instead of their text.
fn shown(text: &str, position: usize) -> String {
    if !text.is_empty() && text.chars().all(|c| c.is_ascii_graphic()) { format!("`{}`", cut_text(text)) } else { format!("the field {position}") }
}

fn form_mistake(message: String) -> Mistake {
    Mistake { code: Code::Lc2106, message }
}

fn read_unit(text: &str, position: usize) -> Result<Unit, Mistake> {
    let unknown =
        || Mistake { code: Code::Lc2008, message: format!("unknown unit in {}: it is not in the table of units (3.3)", shown(text, position)) };
    // What shows nothing is never a unit, and a message never shows it.
    if text.chars().any(is_invisible) {
        return Err(unknown());
    }
    match parse_unit(text, UNIT_NESTING) {
        Ok(unit) => Ok(unit),
        Err(UnitError::Unknown(_)) => Err(unknown()),
        Err(_) => Err(form_mistake(format!("{} is not a unit (3.3)", shown(text, position)))),
    }
}

/// What is after the `:` of a field.
fn read_format<'t>(format: &'t str, position: usize, say: Say<'_>) -> Result<Format<'t>, Mistake> {
    if format.is_empty() {
        return Err(form_mistake(format!("nothing follows the `:` of the field {position}: remove it, or write a format (5.4)")));
    }
    let mut rest = format;
    let mut chars = rest.chars();
    let (first, second) = (chars.next(), chars.next());
    let mut align = false;
    // A letter before `^` starts a unit: `{a:m^2}` is in square metres.
    if let (Some(fill), Some(second)) = (first, second)
        && is_align(second)
        && !(second == '^' && fill.is_alphabetic())
    {
        if fill == '{' || !is_fill(fill) {
            return Err(form_mistake(format!("the fill of the field {position} is a character that shows, other than a brace (5.4)")));
        }
        align = true;
        rest = chars.as_str();
    } else if first.is_some_and(is_align) {
        align = true;
        let mut after = rest.chars();
        after.next();
        rest = after.as_str();
    }
    let plus = take(&mut rest, '+');
    let alternate = take(&mut rest, '#');
    let zero = take(&mut rest, '0');
    let width = take_number(&mut rest);
    if width.is_some_and(|width| width > MAX_WIDTH) {
        return Err(form_mistake(format!("the width of the field {position} goes up to {MAX_WIDTH} (5.4)")));
    }
    let mut decimals = false;
    if take(&mut rest, '.') {
        let Some(count) = take_number(&mut rest) else {
            return Err(form_mistake(format!("the `.` of the field {position} takes a number of decimals (5.4)")));
        };
        if count > MAX_DECIMALS {
            return Err(form_mistake(format!("the decimals of the field {position} go up to {MAX_DECIMALS} (5.4)")));
        }
        decimals = true;
    }
    let letter = take_letter(&mut rest);
    let is_base = letter.is_some_and(|letter| matches!(letter, 'x' | 'X' | 'o' | 'b'));
    let is_integer = is_base || letter == Some('d');
    let written = rest.len() != format.len();
    let unit_text = match rest.strip_prefix(' ') {
        Some(unit) => Some(unit),
        None if rest.is_empty() => None,
        None if !written => Some(rest),
        None => return Err(form_mistake(format!("the format of the field {position} ends in a letter or in one space and a unit (5.4)"))),
    };
    if unit_text.is_some_and(|text| text.is_empty() || text.starts_with([' ', '\t']) || text.ends_with([' ', '\t'])) {
        return Err(form_mistake(format!("the unit of the field {position} follows one space, and nothing follows it (5.4)")));
    }
    if unit_text.is_some_and(|text| text.contains('$')) {
        return Err(form_mistake(format!("the width and the decimals of the field {position} are numbers: a wire cannot give them (5.4)")));
    }
    if zero && align {
        return Err(form_mistake(format!("the field {position} pads with `0` and with an alignment: keep one of them (5.4)")));
    }
    if alternate && !is_base {
        return Err(form_mistake(format!("`#` in the field {position} goes with `x`, `X`, `o` or `b` (5.4)")));
    }
    if is_base && plus {
        return Err(form_mistake(format!("`+` in the field {position} does not go with `x`, `X`, `o` or `b` (5.4)")));
    }
    if is_integer && decimals {
        return Err(form_mistake(format!("an integer has no decimals: remove them from the field {position}, or its letter (5.4)")));
    }
    if is_integer && unit_text.is_some() {
        return Err(form_mistake(format!(
            "`d`, `x`, `X`, `o` and `b` show an integer without a unit: remove the unit of the field {position} (5.4)"
        )));
    }
    let unit = match unit_text.map(|text| read_unit(&normalize(text), position).map(|unit| (unit, text))).transpose() {
        Ok(unit) => unit,
        // The field stays, with its terminal: only its unit is unknown.
        Err(mistake) if mistake.code == Code::Lc2008 => {
            say(mistake);
            None
        }
        Err(mistake) => return Err(mistake),
    };
    Ok(Format { needs_a_number: plus || zero || decimals || letter.is_some(), needs_an_integer: is_integer, unit })
}

fn read_field<'t>(inside: &'t str, position: usize, say: Say<'_>) -> Result<Field<'t>, Mistake> {
    let (name, format) = match inside.split_once(':') {
        Some((name, format)) => (name, Some(format)),
        None => (inside, None),
    };
    if name.is_empty() || name.chars().all(|c| c.is_ascii_digit()) {
        return Err(form_mistake("a field has a name, which gives its terminal: `{}` and `{0}` have none (5.4)".to_owned()));
    }
    if !is_name(name) {
        return Err(form_mistake(format!("{} is not a name, so no terminal can take it (5.4, R-20)", shown(name, position))));
    }
    let name = normalize(name);
    if matches!(name.as_ref(), "template" | "after") {
        return Err(form_mistake(format!("a field cannot be named `{name}`, which the node already uses: choose another name (5.4)")));
    }
    let format = match format {
        Some(format) => read_format(format, position, say)?,
        None => Format::default(),
    };
    Ok(Field { name, needs_a_number: format.needs_a_number, needs_an_integer: format.needs_an_integer, unit: format.unit })
}

/// The fields of a template, each written form once, and whether one of them is malformed (R-12). A `}` after a malformed field is
/// what is left of that field.
fn read_template<'t>(text: &'t str, say: Say<'_>) -> (Vec<Field<'t>>, bool) {
    let mut fields = Vec::new();
    let mut seen: HashSet<&'t str> = HashSet::new();
    let mut malformed = false;
    let (mut rest, mut position) = (text, 0_usize);
    while let Some(at) = rest.find(['{', '}']) {
        let Some((_, tail)) = rest.split_at_checked(at) else { break };
        if let Some(after) = tail.strip_prefix("{{").or_else(|| tail.strip_prefix("}}")) {
            rest = after;
            continue;
        }
        if let Some(after) = tail.strip_prefix('}') {
            if !malformed {
                say(form_mistake("a `}` that closes nothing: write `}}` for a brace (5.4)".to_owned()));
                malformed = true;
            }
            rest = after;
            continue;
        }
        let inside = tail.strip_prefix('{').unwrap_or(tail);
        let Some((field, after)) = inside.split_once('}') else {
            say(form_mistake("a `{` that nothing closes: write `{{` for a brace (5.4)".to_owned()));
            malformed = true;
            break;
        };
        position += 1;
        rest = after;
        if !seen.insert(field) {
            continue;
        }
        match read_field(field, position, say) {
            Ok(field) => fields.push(field),
            Err(mistake) => {
                say(mistake);
                malformed = true;
            }
        }
    }
    (fields, malformed)
}

/// The dimension of the numbers of a type, as a unit has to convert them all.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Dimension {
    /// Nothing that has one, so far.
    None,
    One(Unit),
    Several,
}

/// What a type can show, at any depth.
#[derive(Clone, Copy, Debug)]
pub(in crate::declare::body) struct Shown {
    /// A type with no text inside it.
    refused: Option<TyId>,
    /// An error upstream hides what it holds.
    unknown: bool,
    has_a_number: bool,
    /// Each number that it has is an integer without a unit.
    only_integers: bool,
    dimension: Dimension,
}

impl Shown {
    const TEXT: Shown = Shown { refused: None, unknown: false, has_a_number: false, only_integers: true, dimension: Dimension::None };

    fn number(only_integers: bool, unit: Unit) -> Shown {
        Shown { has_a_number: true, only_integers, dimension: Dimension::One(unit), ..Shown::TEXT }
    }

    fn and(self, other: Shown) -> Shown {
        let dimension = match (self.dimension, other.dimension) {
            (Dimension::None, one) | (one, Dimension::None) => one,
            (Dimension::One(a), Dimension::One(b)) if a.same_dimension(&b) => Dimension::One(a),
            _ => Dimension::Several,
        };
        Shown {
            refused: self.refused.or(other.refused),
            unknown: self.unknown || other.unknown,
            has_a_number: self.has_a_number || other.has_a_number,
            only_integers: self.only_integers && other.only_integers,
            dimension,
        }
    }
}

impl<'a> Typer<'_, '_, '_, 'a> {
    fn shown(&mut self, ty: TyId) -> Shown {
        if let Some(known) = self.context.facts.shown.get(&ty) {
            return *known;
        }
        let definition = definition_of(self.pass, self.context, ty);
        let parts: Vec<TyId> = match self.pass.table.get(definition) {
            Ty::Array { element, .. } => vec![*element],
            // Its format goes to its samples: `t0` and `dt` keep theirs.
            Ty::Waveform(element) => vec![*element],
            Ty::Struct(fields) => fields.iter().map(|(_, field)| *field).collect(),
            Ty::Enum(variants) => variants.iter().flat_map(|variant| variant.values.iter().copied()).collect(),
            _ => Vec::new(),
        };
        let own = match self.pass.table.get(definition) {
            Ty::Error | Ty::Variable(_) => Shown { unknown: true, ..Shown::TEXT },
            Ty::Number { kind, unit } => Shown::number(kind.is_integer() && *unit == Unit::ONE, *unit),
            Ty::Duration => Shown::number(false, Unit::SECOND),
            // A type declared as opaque stands for itself.
            Ty::Function { .. } | Ty::Queue(_) | Ty::Ref(_) | Ty::Named { .. } => Shown { refused: Some(ty), ..Shown::TEXT },
            _ => Shown::TEXT,
        };
        let shown = parts.into_iter().fold(own, |shown, part| shown.and(self.shown(part)));
        self.context.facts.shown.insert(ty, shown);
        shown
    }

    /// `string.format`: the template, then what each field makes of its wire.
    pub(super) fn format_node(&mut self, call: &'a Call, bound: &ResolvedCall) {
        let mut template = None;
        let mut named: HashMap<&'a str, &'a Operand> = HashMap::new();
        let mut positional = false;
        let mut named_seen = false;
        for (arg, terminal) in call.args.iter().zip(&bound.terminals) {
            let Arg::Value { id, name, value } = arg else {
                named_seen = true;
                continue;
            };
            match (terminal, name) {
                (Terminal::Param(_), _) => template = Some(value),
                (_, Some(name)) => {
                    named_seen = true;
                    named.insert(self.text(*name), value);
                }
                // After a named one, it has its `LC2104`.
                (_, None) if named_seen => positional = true,
                (_, None) => {
                    let message = "a field takes its value by name, as in `u: voltage` (5.4, R-12)".to_owned();
                    self.say(Code::Lc2106, message, *id, Details::default());
                    positional = true;
                }
            }
            if let (Terminal::Param(_), Some(_)) = (terminal, name) {
                named_seen = true;
            }
        }
        for (arg, terminal) in call.args.iter().zip(&bound.terminals) {
            if let (Arg::Value { value, .. }, Terminal::Free) = (arg, terminal) {
                self.value_of(value, None);
            }
        }
        let Some(template) = template else { return };
        let Operand::Text(id, text) = template else {
            // A number or a wire is no template, and a named constant is no text on the face of a node.
            if self.value_of(template, None) != TyId::ERROR {
                let message = "the template of `string.format` is written on the node: it is not a wire, nor a named constant (5.4, R-12)".to_owned();
                self.say(Code::Lc2106, message, template.id(), Details::default());
            }
            return;
        };
        self.value_of(template, None);
        let text = read_escapes(text);
        let (fields, malformed) = read_template(&text, &mut |mistake| self.say(mistake.code, mistake.message, *id, Details::default()));
        let mut said = HashSet::new();
        for field in &fields {
            if let Some(operand) = named.get(field.name.as_ref()) {
                self.judge_field(field, operand, &mut said);
            }
        }
        if !malformed && !positional {
            self.compare_fields(*id, &fields, call);
        }
    }

    /// One diagnostic for each cause and each name, however many forms of it the template writes: `{a:x} {a:d}` has one thing to fix.
    fn judge_field<'f>(&mut self, field: &'f Field<'_>, operand: &'a Operand, said: &mut HashSet<(&'f str, Refusal)>) {
        let ty = self.resolved.ty(operand.id());
        if ty == TyId::ERROR {
            return;
        }
        let shown = self.shown(ty);
        if shown.unknown {
            return;
        }
        let converts = |unit: &Unit| match shown.dimension {
            Dimension::One(own) => own.same_dimension(unit),
            Dimension::None | Dimension::Several => false,
        };
        let refusal = if shown.refused.is_some() {
            Refusal::NoText
        } else if field.needs_an_integer && !(shown.has_a_number && shown.only_integers) {
            Refusal::NotAnInteger
        } else if field.needs_a_number && !shown.has_a_number {
            Refusal::NoNumber
        } else if field.unit.is_some_and(|(unit, _)| !converts(&unit)) {
            Refusal::OtherUnit
        } else {
            return;
        };
        let name = field.name.as_ref();
        if !said.insert((name, refusal)) {
            return;
        }
        // The text of a type can be long, and is made only for what is wrong, and only once.
        let found = self.show(ty);
        let cut = cut_text(name);
        let (code, message) = match (refusal, shown.refused, field.unit) {
            (Refusal::NoText, Some(refused), _) => {
                (Code::Lc3001, format!("the field `{cut}` shows a value as text, and `{}` has none", self.show(refused)))
            }
            (Refusal::NotAnInteger, ..) => {
                (Code::Lc3001, format!("the letter of the field `{cut}` shows an integer without a unit, and this is `{found}`"))
            }
            (Refusal::NoNumber, ..) => (Code::Lc3001, format!("the format of the field `{cut}` applies to numbers, and `{found}` holds none")),
            (_, _, Some((_, written))) => {
                (Code::Lc3005, format!("the field `{cut}` shows its value in `{}`, and `{found}` does not convert to it (3.3)", cut_text(written)))
            }
            _ => return,
        };
        let wires = vec![self.wire_of(operand, self.input_terminal(name))];
        self.say(code, message, operand.id(), Details { wires, found: Some(found), ..Details::default() });
    }

    /// One diagnostic for what differs, with a few of the names (R-12).
    fn compare_fields(&mut self, id: labcraft_graph::Id, fields: &[Field<'_>], call: &Call) {
        let wanted: HashSet<&str> = fields.iter().map(|field| field.name.as_ref()).collect();
        let mut wired: HashSet<Name> = HashSet::new();
        let extra: Vec<Name> = call
            .args
            .iter()
            .filter_map(|arg| match arg {
                Arg::Value { name: Some(name), .. } if self.text(*name) != "template" && wired.insert(*name) => Some(*name),
                _ => None,
            })
            .filter(|name| !wanted.contains(self.text(*name)))
            .collect();
        let wired_text: HashSet<&str> = wired.iter().map(|name| self.text(*name)).collect();
        let mut seen: HashSet<&str> = HashSet::new();
        let missing: Vec<&str> =
            fields.iter().map(|field| field.name.as_ref()).filter(|name| !wired_text.contains(name) && seen.insert(*name)).collect();
        if missing.is_empty() && extra.is_empty() {
            return;
        }
        let list = |names: Vec<String>| {
            let count = names.len();
            let shown: Vec<String> = names.iter().take(MAX_NAMES).map(|name| format!("`{}`", cut_text(name))).collect();
            let more = count.saturating_sub(MAX_NAMES);
            if more > 0 { format!("{} and {more} more", shown.join(", ")) } else { shown.join(", ") }
        };
        let mut parts = Vec::new();
        let mut fixes = Vec::new();
        if !missing.is_empty() {
            let names = list(missing.iter().map(|name| (*name).to_owned()).collect());
            parts.push(format!("no wire for the field {names}"));
            fixes.push(Fix { title: format!("Wire {names}, or remove it from the template"), edits: Vec::new() });
        }
        if !extra.is_empty() {
            let names = list(extra.iter().map(|name| self.text(*name).to_owned()).collect());
            parts.push(format!("no field for the argument {names}"));
            fixes.push(Fix { title: format!("Add a field for {names}, or remove that wire"), edits: Vec::new() });
        }
        let message = format!("{} (5.4, R-12)", parts.join(", and "));
        self.say(Code::Lc2106, message, id, Details { fixes, ..Details::default() });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str) -> (Vec<Field<'_>>, Vec<Mistake>) {
        let mut mistakes = Vec::new();
        let (fields, _) = read_template(text, &mut |mistake| mistakes.push(mistake));
        (fields, mistakes)
    }

    fn fields(text: &str) -> Vec<(String, bool, bool, bool)> {
        let (fields, mistakes) = read(text);
        assert_eq!(mistakes, [], "{text}");
        fields.iter().map(|field| (field.name.to_string(), field.needs_a_number, field.needs_an_integer, field.unit.is_some())).collect()
    }

    fn mistakes(text: &str) -> Vec<&'static str> {
        read(text).1.iter().map(|mistake| mistake.code.as_str()).collect()
    }

    #[test]
    fn a_template_has_its_fields_and_its_braces() {
        assert_eq!(fields("plain"), []);
        assert_eq!(fields("{{a}} }}{{"), []);
        assert_eq!(fields("{a}"), [("a".to_owned(), false, false, false)]);
        assert_eq!(fields("{{{a}}}"), [("a".to_owned(), false, false, false)]);
        assert_eq!(fields("{a} {a:.3}").len(), 2);
        assert_eq!(fields("{a:.3} {a:.3}").len(), 1, "one written form, once");
    }

    #[test]
    fn every_part_of_a_format_reads() {
        for format in [
            ".3",
            "mV",
            " mV",
            ".3 mV",
            "02",
            "8",
            "<8",
            "^8",
            ">8",
            "*^12",
            " <8",
            ">10.3",
            "+.2",
            "+",
            "#x",
            "#06x",
            "x",
            "X",
            "o",
            "b",
            "08b",
            "d",
            "+d",
            "05d",
            ">5d",
            "f",
            ".3f",
            "g",
            ".3g",
            "e",
            "E",
            ".3e",
            ".3E",
            "+.3e",
            ">10.3 mV",
            ".3f mV",
            "e mV",
            "g mV",
            "m/s",
            ".3 m/s",
            ".3 kg/(m*s)",
            ":<8",
            "0",
            "0.3",
            ".0",
            ".100",
            "1000",
            "\u{00B0}C",
        ] {
            let text = format!("{{a:{format}}}");
            let (fields, mistakes) = read(&text);
            assert_eq!(mistakes, [], "{text}");
            assert_eq!(fields.len(), 1, "{text}");
        }
    }

    #[test]
    fn a_letter_comes_before_a_unit_of_the_same_spelling() {
        assert_eq!(fields("{m:g}"), [("m".to_owned(), true, false, false)], "general format");
        assert_eq!(fields("{m: g}"), [("m".to_owned(), false, false, true)], "grams");
        assert_eq!(fields("{m:kg}"), [("m".to_owned(), false, false, true)]);
        assert_eq!(fields("{m:.1 g}"), [("m".to_owned(), true, false, true)]);
    }

    #[test]
    fn a_letter_before_a_caret_starts_a_unit() {
        assert_eq!(fields("{a:m^2}"), [("a".to_owned(), false, false, true)], "square metres");
        assert_eq!(fields("{a:V^2}"), [("a".to_owned(), false, false, true)]);
        assert_eq!(fields("{a:g^2}"), [("a".to_owned(), false, false, true)], "and no letter of format");
        assert_eq!(mistakes("{a:x^12}"), ["LC2008"], "never a fill without a word");
        assert_eq!(fields("{a:*^12}"), [("a".to_owned(), false, false, false)]);
        assert_eq!(fields("{a:m<8}"), [("a".to_owned(), false, false, false)], "no unit holds `<`");
        assert_eq!(fields("{a:m>8}"), [("a".to_owned(), false, false, false)]);
    }

    #[test]
    fn what_a_format_asks_of_its_value() {
        assert_eq!(fields("{a:>8}"), [("a".to_owned(), false, false, false)], "any value has a width");
        for format in [".3", "+", "02", "f", "g", "e", "E"] {
            assert_eq!(fields(&format!("{{a:{format}}}")), [("a".to_owned(), true, false, false)], "{format}");
        }
        for format in ["d", "x", "X", "o", "b", "#x", "+d"] {
            assert_eq!(fields(&format!("{{a:{format}}}")), [("a".to_owned(), true, true, false)], "{format}");
        }
    }

    #[test]
    fn what_is_not_a_field() {
        for text in [
            "{",
            "}",
            "{}",
            "{0}",
            "{12:.3}",
            "{a",
            "a}",
            "{ a }",
            "{a }",
            "{a b}",
            "{a:}",
            "{a{b}",
            "{for}",
            "{true}",
            "{_a}",
            "{1a}",
            "{a.b}",
            "{template}",
            "{after}",
            "{a:template}",
        ] {
            let mistakes = mistakes(text);
            assert!(!mistakes.is_empty(), "{text}");
            assert!(mistakes.iter().all(|code| *code == "LC2106" || *code == "LC2008"), "{text}: {mistakes:?}");
        }
        assert_eq!(mistakes("{{a}"), ["LC2106"], "a brace, and a closing brace alone");
    }

    #[test]
    fn what_a_format_refuses() {
        for format in [
            "1001",
            "99999999999999999999",
            ".101",
            ".",
            ".x",
            "8.",
            "*<05",
            "<05",
            "#",
            "#f",
            "#.3",
            "#d",
            "+x",
            "+#x",
            ".3x",
            ".3d",
            "x mV",
            "d mV",
            "o V",
            "8mV",
            ".3mV",
            ".3  mV",
            ".3 mV ",
            ".3 ",
            " ",
            "  mV",
            "\u{200B}<8",
            "\u{7}<8",
            "{<8",
            "()",
            "V(",
            "(((((((((((V)))))))))))",
            " V^999",
        ] {
            let text = format!("{{a:{format}}}");
            assert_eq!(mistakes(&text), ["LC2106"], "{text}");
        }
    }

    #[test]
    fn an_unknown_unit_has_its_code_and_never_its_text() {
        for unit in ["Vx", "\u{3164}", "m\u{FE0F}V", "\u{200B}V"] {
            let text = format!("{{a: {unit}}}");
            let (_, found) = read(&text);
            assert_eq!(found.iter().map(|mistake| mistake.code.as_str()).collect::<Vec<_>>(), ["LC2008"], "{text:?}");
            assert!(found.iter().all(|mistake| mistake.message.is_ascii()), "{text:?}");
        }
        let (_, found) = read("{\u{200B}a}");
        assert_eq!(found.len(), 1);
        assert!(found.iter().all(|mistake| mistake.code == Code::Lc2106 && mistake.message.is_ascii()));
    }

    #[test]
    fn a_name_has_one_spelling() {
        assert_eq!(fields("{\u{03BC}}"), [("\u{00B5}".to_owned(), false, false, false)]);
        assert_eq!(fields("{e\u{0301}}"), [("\u{00E9}".to_owned(), false, false, false)]);
    }

    #[test]
    fn each_field_says_what_is_wrong_with_it() {
        assert_eq!(mistakes("{a:.} {b:.} {c}"), ["LC2106", "LC2106"]);
        assert_eq!(read("{a:.} {b:.} {c}").0.len(), 1);
        assert_eq!(mistakes("{a:.} {a:.}"), ["LC2106"], "one written form");
        assert_eq!(mistakes("{} {} {}"), ["LC2106"]);
    }

    #[test]
    fn a_brace_after_a_malformed_field_is_what_is_left_of_it() {
        assert_eq!(mistakes("{a:{w}}"), ["LC2106"]);
        assert_eq!(mistakes("{a:}<8}"), ["LC2106"]);
        assert_eq!(mistakes("} } }"), ["LC2106"], "and one for all of them");
    }

    #[test]
    fn a_width_from_a_wire_says_so() {
        let (_, found) = read("{x:w$}");
        assert_eq!(found.len(), 1);
        assert!(found.iter().all(|mistake| mistake.code == Code::Lc2106 && mistake.message.contains("wire")));
    }

    #[test]
    fn a_fill_is_a_character_that_shows() {
        for fill in ["*", "-", "x", "\u{00E9}", " "] {
            assert_eq!(mistakes(&format!("{{a:{fill}<8}}")), [] as [&str; 0], "{fill:?}");
        }
        for fill in ["\u{00A0}", "\u{2028}", "\u{0301}", "\u{E000}", "\u{200B}", "\u{7}"] {
            assert_eq!(mistakes(&format!("{{a:{fill}<8}}")), ["LC2106"], "{fill:?}");
        }
    }
}
