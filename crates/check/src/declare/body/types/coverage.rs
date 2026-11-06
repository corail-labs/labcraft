//! What the arms of a `match` or a `handle` cover: every value has an arm (R-8, R-10), none has two (R-18, 6.1, 7.3), and a `_` that
//! can catch nothing has a warning. A list of patterns can be as long as the file, and an enumeration too, so each `match` costs
//! what its patterns cost: variants are walked only to name the few that are missing.

use std::borrow::Cow;
use std::collections::HashMap;

use labcraft_graph::{Arm, Branching, Id, IntBound, Numeral, NumeralKind, Operand, Pattern};
use labcraft_types::{Code, Edit, Fix, Name, NumberKind, Span, Ty, TyId, cut_text};

use super::super::{definition_of, integer_fits};
use super::{Details, Typer, fits_kind, integer_value};
use crate::resolved::Meaning;

/// The variants that `LC3010` names.
const MISSING_SHOWN: usize = 5;

/// What a `handle` cannot have twice. A category and `a.*` are not the same arm: the most specific one wins (R-18).
#[derive(PartialEq, Eq, Hash)]
enum HandleArm {
    Wildcard,
    Ok,
    Code(i128),
    Category(Vec<Name>, bool),
}

fn pattern_id(pattern: &Pattern) -> Id {
    match pattern {
        Pattern::Literal(operand) => operand.id(),
        Pattern::Range { id, .. }
        | Pattern::Variant { id, .. }
        | Pattern::Wildcard { id, .. }
        | Pattern::Ok { id }
        | Pattern::Category { id, .. }
        | Pattern::Code { id, .. } => *id,
    }
}

/// The last value of a range, or nothing when it holds none.
pub(super) fn last_of_range(low: i128, high: i128, inclusive: bool) -> Option<i128> {
    let last = if inclusive { high } else { high.checked_sub(1)? };
    (low <= last).then_some(last)
}

/// The text of a string, as it runs: the lexer let through only the escapes that the grammar has (14.1).
pub(super) fn read_escapes(text: &str) -> Cow<'_, str> {
    if !text.contains('\\') {
        return Cow::Borrowed(text);
    }
    let mut read = String::with_capacity(text.len());
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            read.push(character);
            continue;
        }
        match characters.next() {
            Some('n') => read.push('\n'),
            Some('t') => read.push('\t'),
            Some('r') => read.push('\r'),
            Some(other) => read.push(other),
            None => {}
        }
    }
    Cow::Owned(read)
}

impl<'a> Typer<'_, '_, '_, 'a> {
    /// The value of a bound, or nothing for a number beyond every type, which has its `LC1006`.
    pub(super) fn bound_value(&self, bound: IntBound) -> Option<i128> {
        let text = self.text(bound.digits);
        if !integer_fits(bound.negative, text) {
            return None;
        }
        integer_value(bound.negative, text)
    }

    fn say_repeated(&mut self, code: Code, message: &str, id: Id, first: Id) {
        let related = vec![self.here("the other pattern is here".to_owned(), first)];
        self.say(code, message.to_owned(), id, Details { related, ..Details::default() });
    }

    /// `LC3022` and `LC3012`, which no type decides: a selector of a wrong type does not hide them.
    pub(super) fn judge_handle_arms(&mut self, branching: &'a Branching) {
        let mut seen: HashMap<HandleArm, Id> = HashMap::new();
        for pattern in branching.arms.iter().flat_map(Arm::patterns) {
            let arm = match pattern {
                Pattern::Wildcard { .. } => HandleArm::Wildcard,
                Pattern::Ok { .. } => HandleArm::Ok,
                Pattern::Code { code, .. } => match self.bound_value(*code) {
                    Some(value) => HandleArm::Code(value),
                    None => continue,
                },
                Pattern::Category { path, wildcard, .. } => HandleArm::Category(path.segments().collect(), *wildcard),
                Pattern::Literal(_) | Pattern::Range { .. } | Pattern::Variant { .. } => continue,
            };
            let id = pattern_id(pattern);
            match seen.get(&arm).copied() {
                Some(first) => self.say_repeated(Code::Lc3022, "a `handle` has this pattern twice (7.3)", id, first),
                None => {
                    seen.insert(arm, id);
                }
            }
        }
        if !seen.contains_key(&HandleArm::Wildcard) {
            let message = "a `handle` has a `_` arm, since categories are open (R-10)".to_owned();
            self.say(Code::Lc3012, message, branching.id, Details::default());
        }
    }

    /// `mistaken`: one of its patterns has a diagnostic, so what it was meant to cover stays unknown. A repeated pattern counts
    /// too, since it may be the one that is missing.
    pub(super) fn judge_match_arms(&mut self, branching: &'a Branching, selector: TyId, mut mistaken: bool) {
        let mut wildcard = None;
        for pattern in branching.arms.iter().flat_map(Arm::patterns) {
            if let Pattern::Wildcard { id, .. } = pattern {
                match wildcard {
                    Some(first) => {
                        mistaken = true;
                        self.say_repeated(Code::Lc3022, "a `match` has one `_` (6.1)", *id, first);
                    }
                    None => wildcard = Some(*id),
                }
            }
        }
        if selector == TyId::ERROR {
            return;
        }
        let definition = definition_of(self.pass, self.context, selector);
        match self.pass.table.get(definition) {
            Ty::Bool => self.judge_bool_arms(branching, wildcard, mistaken),
            Ty::Enum(_) => self.judge_enum_arms(branching, selector, definition, wildcard, mistaken),
            Ty::String => self.judge_string_arms(branching, wildcard),
            Ty::Number { kind, .. } => {
                let kind = *kind;
                self.judge_integer_arms(branching, kind, wildcard);
            }
            _ => {}
        }
    }

    fn judge_bool_arms(&mut self, branching: &'a Branching, wildcard: Option<Id>, mut mistaken: bool) {
        let (mut yes, mut no) = (None, None);
        for pattern in branching.arms.iter().flat_map(Arm::patterns) {
            if let Pattern::Literal(Operand::Bool(id, value)) = pattern {
                let slot = if *value { &mut yes } else { &mut no };
                match *slot {
                    Some(first) => {
                        mistaken = true;
                        self.say_repeated(Code::Lc3011, "these patterns overlap (R-18)", *id, first);
                    }
                    None => *slot = Some(*id),
                }
            }
        }
        if mistaken {
            return;
        }
        match (wildcard, yes, no) {
            (Some(id), Some(_), Some(_)) => self.say_catches_nothing(branching, id),
            (None, None, None) => self.say_no_arm(branching, "`true` and `false`"),
            (None, None, Some(_)) => self.say_no_arm(branching, "`true`"),
            (None, Some(_), None) => self.say_no_arm(branching, "`false`"),
            _ => {}
        }
    }

    fn judge_enum_arms(&mut self, branching: &'a Branching, selector: TyId, definition: TyId, wildcard: Option<Id>, mut mistaken: bool) {
        let mut named: HashMap<Name, Id> = HashMap::new();
        for pattern in branching.arms.iter().flat_map(Arm::patterns) {
            let Pattern::Variant { path, .. } = pattern else { continue };
            let Meaning::Variant { ty, variant } = self.resolved.of(path.id) else { continue };
            if !self.pass.table.matches(ty, selector) {
                continue;
            }
            match named.get(&variant).copied() {
                Some(first) => {
                    mistaken = true;
                    self.say_repeated(Code::Lc3022, "a `match` names this variant twice (6.1)", path.id, first);
                }
                None => {
                    named.insert(variant, path.id);
                }
            }
        }
        if mistaken {
            return;
        }
        let Ty::Enum(variants) = self.pass.table.get(definition) else { return };
        let total = variants.len();
        match wildcard {
            Some(id) if named.len() >= total => self.say_catches_nothing(branching, id),
            None if named.len() < total => {
                let names = self.pass.names;
                let shown: Vec<String> = variants
                    .iter()
                    .filter(|variant| !named.contains_key(&variant.name))
                    .take(MISSING_SHOWN)
                    .map(|variant| format!("`{}`", cut_text(names.resolve(variant.name))))
                    .collect();
                let more = total.saturating_sub(named.len()).saturating_sub(shown.len());
                let list = shown.join(", ");
                let missing = if more == 0 { list } else { format!("{list} and {more} more") };
                self.say_no_arm(branching, &missing);
            }
            _ => {}
        }
    }

    fn judge_string_arms(&mut self, branching: &'a Branching, wildcard: Option<Id>) {
        let mut seen: HashMap<Cow<'a, str>, Id> = HashMap::new();
        for pattern in branching.arms.iter().flat_map(Arm::patterns) {
            if let Pattern::Literal(Operand::Text(id, text)) = pattern {
                let text = read_escapes(text);
                match seen.get(text.as_ref()).copied() {
                    Some(first) => self.say_repeated(Code::Lc3011, "these patterns overlap (R-18)", *id, first),
                    None => {
                        seen.insert(text, *id);
                    }
                }
            }
        }
        if wildcard.is_none() {
            self.say_no_arm(branching, "all the other strings: write `_`");
        }
    }

    /// Sorted by their start, a pattern overlaps another one when it starts before the furthest end so far.
    fn judge_integer_arms(&mut self, branching: &'a Branching, kind: NumberKind, wildcard: Option<Id>) {
        let mut ranges: Vec<(i128, i128, Id)> = Vec::new();
        for pattern in branching.arms.iter().flat_map(Arm::patterns) {
            match pattern {
                Pattern::Literal(Operand::Number(id, Numeral { negative, kind: NumeralKind::Int(digits) }, None)) => {
                    let bound = IntBound { negative: *negative, digits: *digits };
                    if let Some(value) = self.bound_value(bound).filter(|value| fits_kind(*value, kind)) {
                        ranges.push((value, value, *id));
                    }
                }
                Pattern::Range { id, low, high, inclusive } => {
                    let low = self.bound_value(*low).filter(|value| fits_kind(*value, kind));
                    let high = self.bound_value(*high).filter(|value| fits_kind(*value, kind));
                    if let (Some(low), Some(last)) = (low, high.and_then(|high| low.and_then(|low| last_of_range(low, high, *inclusive)))) {
                        ranges.push((low, last, *id));
                    }
                }
                _ => {}
            }
        }
        ranges.sort_by_key(|(low, ..)| *low);
        let mut furthest: Option<(i128, Id)> = None;
        for (low, high, id) in ranges {
            match furthest {
                Some((end, other)) if low <= end => {
                    self.say_repeated(Code::Lc3011, "these patterns overlap (R-18)", id, other);
                    if high > end {
                        furthest = Some((high, id));
                    }
                }
                _ => furthest = Some((high, id)),
            }
        }
        if wildcard.is_none() {
            self.say_no_arm(branching, "all the other integers: write `_`, even when ranges cover the whole type");
        }
    }

    fn say_no_arm(&mut self, branching: &'a Branching, missing: &str) {
        let message = format!("this `match` has no arm for {missing} (R-8)");
        self.say(Code::Lc3010, message, branching.id, Details::default());
    }

    fn say_catches_nothing(&mut self, branching: &'a Branching, id: Id) {
        let message = "this `_` can catch nothing: every value of the selector has its pattern (R-8)".to_owned();
        let fixes = self.removal_of_wildcard(branching, id).into_iter().collect();
        self.say(Code::Lc4106, message, id, Details { fixes, ..Details::default() });
    }

    /// Removes the `_` from its list. An arm that stands alone only gets a title: what it yields can decide the type of the
    /// others (R-7), so removing it is not mechanical.
    fn removal_of_wildcard(&self, branching: &'a Branching, id: Id) -> Option<Fix> {
        let map = &self.scope.model.source_map;
        let arm = branching.arms.iter().find(|arm| arm.patterns().any(|pattern| pattern_id(pattern) == id))?;
        if arm.others.is_empty() {
            let title = "Remove this arm, and check the type of the result: the first arm that decides gives it (R-7)".to_owned();
            return Some(Fix { title, edits: Vec::new() });
        }
        let ids: Vec<Id> = arm.patterns().map(pattern_id).collect();
        let at = ids.iter().position(|other| *other == id)?;
        let place = map.span(id)?;
        let span = match ids.get(at + 1) {
            Some(next) => Span { start: place.start, end: map.span(*next)?.start },
            None => Span { start: map.span(*ids.get(at.checked_sub(1)?)?)?.end, end: place.end },
        };
        Some(Fix { title: "Remove `_`".to_owned(), edits: vec![Edit { span, text: String::new() }] })
    }
}
