//! What builds a value (5.1): a struct, a `waveform` or a variant, on a line or as the default value of a parameter (R-21), which
//! holds the same forms. A construction costs what is written in it: the fields of a type are walked only to name the few that are
//! missing, since a struct can hold a million fields and be built a million times.

use std::collections::HashMap;

use labcraft_graph::{Arg, Call, Constant, FieldInit, Id, Operand, Path, Signature, StructLit};
use labcraft_types::{Code, Name, Ty, TyId, cut_text};

use super::super::super::DeclaredFunction;
use super::super::{WAVEFORM_FIELDS, definition_of};
use super::{Details, Fit, Typer, Within};
use crate::resolved::{Meaning, ResolvedCall, Terminal};

/// The fields that a diagnostic names when they are missing.
const MISSING_SHOWN: usize = 5;

impl<'a> Typer<'_, '_, '_, 'a> {
    /// In a default every mistake is one of that default (`docs/cli.md`, 4.2).
    pub(super) fn construction_code(&self) -> Code {
        if matches!(self.within, Within::Default(_) | Within::Panel(_)) { Code::Lc3017 } else { Code::Lc3015 }
    }

    /// What nothing says the type of, inside a default, is a mistake of that default.
    pub(super) fn unknown_type_code(&self) -> Code {
        if matches!(self.within, Within::Default(_) | Within::Panel(_)) { Code::Lc3017 } else { Code::Lc3019 }
    }

    fn piece_terminal(&self, piece: &str) -> String {
        match self.within {
            Within::Default(name) => format!("fn.{}", cut_text(self.text(name))),
            Within::Panel(name) => format!("panel.{}", cut_text(self.text(name))),
            Within::Diagram | Within::Construction => self.input_terminal(piece),
        }
    }

    /// A field or a value against what the type says of it.
    pub(super) fn check_piece(&mut self, operand: &'a Operand, expected: TyId, piece: &str) {
        let found = self.value_of(operand, Some(expected));
        self.judge_piece(operand, found, expected, piece);
    }

    pub(super) fn judge_piece(&mut self, operand: &'a Operand, found: TyId, expected: TyId, piece: &str) {
        match self.fit_of(operand, found, expected) {
            Fit::Same => {}
            Fit::Coerce => self.resolved.add_coercion(operand.id(), found, expected),
            Fit::Narrow | Fit::No => {
                let (expected_text, found_text) = (self.show(expected), self.show(found));
                let message = format!("`{}` has the type `{expected_text}`, and this is `{found_text}`", cut_text(piece));
                let fixes = self.conversion_fixes(operand, found, expected);
                let wires = vec![self.wire_of(operand, self.piece_terminal(piece))];
                let details = Details { wires, expected: Some(expected_text), found: Some(found_text), fixes, ..Details::default() };
                self.say(self.construction_code(), message, operand.id(), details);
            }
        }
    }

    /// What a mistake elsewhere leaves unjudged still has its names and its numbers read, and says nothing that depends on a type.
    fn unjudged(&mut self, operand: &'a Operand) {
        self.value_of(operand, Some(TyId::ERROR));
    }

    fn enter_construction(&mut self) -> Within {
        let around = self.within;
        if around == Within::Diagram {
            self.within = Within::Construction;
        }
        around
    }

    /// `Reading.Value` for a variant that holds values gives no default value (5.1).
    pub(super) fn judge_bare_variant(&mut self, path: &Path, ty: TyId, variant: Name) {
        let definition = definition_of(self.pass, self.context, ty);
        let holds = self.variant_values(definition, variant).map_or(0, <[TyId]>::len);
        if holds > 0 {
            let message = format!("this variant holds {holds} values, and this gives none: a variant has no default value (5.1)");
            self.say(self.construction_code(), message, path.id, Details::default());
        }
    }

    /// `Reading.Value(v)`: positions, one argument per value (5.1). Each value of a variant is looked up alone, as a variant can hold a
    /// million of them.
    pub(super) fn variant_construction(&mut self, call: &'a Call, bound: &ResolvedCall, ty: TyId, variant: Name) {
        let around = self.enter_construction();
        let definition = definition_of(self.pass, self.context, ty);
        let holds = self.variant_values(definition, variant).map_or(0, <[TyId]>::len);
        let mut given = 0_usize;
        for (arg, terminal) in call.args.iter().zip(&bound.terminals) {
            let (Arg::Value { value, .. }, Terminal::Param(position)) = (arg, terminal) else { continue };
            given += 1;
            let expected = self.variant_values(definition, variant).and_then(|values| values.get(*position as usize)).copied();
            match expected {
                Some(expected) => self.check_piece(value, expected, &position.to_string()),
                None => self.unjudged(value),
            }
        }
        if given != holds {
            let message = format!("this variant holds {holds} values, and this gives {given} (5.1)");
            self.say(self.construction_code(), message, call.id, Details::default());
        }
        self.within = around;
    }

    /// What a `{ }` builds. `hint` is what it goes to, which only a `waveform` reads, for the `T` of an empty array.
    pub(super) fn construction(&mut self, literal: &'a StructLit, hint: Option<TyId>) -> TyId {
        let around = self.enter_construction();
        let ty = match self.resolved.of(literal.id) {
            Meaning::Waveform => self.waveform_construction(literal, None, hint),
            Meaning::NotAStruct => self.not_a_struct(literal),
            Meaning::Type(ty) => {
                let definition = definition_of(self.pass, self.context, ty);
                match self.pass.table.get(definition) {
                    Ty::Struct(_) => {
                        self.struct_construction(literal, definition);
                        ty
                    }
                    Ty::Waveform(element) => {
                        let element = *element;
                        self.waveform_construction(literal, Some(element), hint);
                        ty
                    }
                    Ty::Error => self.nothing_known(literal),
                    _ => self.not_a_struct(literal),
                }
            }
            // An error upstream has its diagnostic.
            _ => self.nothing_known(literal),
        };
        self.within = around;
        self.resolved.set_type(literal.id, ty);
        ty
    }

    fn nothing_known(&mut self, literal: &'a StructLit) -> TyId {
        for field in &literal.fields {
            self.unjudged(&field.value);
        }
        TyId::ERROR
    }

    fn not_a_struct(&mut self, literal: &'a StructLit) -> TyId {
        let message = format!(
            "`{}` is not a struct: only a struct or a `waveform` is built with `{{ }}` (5.1)",
            cut_text(&super::path_text(&literal.path, self.pass.names))
        );
        self.say(self.construction_code(), message, literal.path.id, Details::default());
        self.nothing_known(literal)
    }

    /// The second of a field given twice, with the first in `related`.
    fn field_given_twice(&mut self, field: &'a FieldInit, first: Id) {
        let text = cut_text(self.text(field.name)).into_owned();
        let related = vec![self.here(format!("`{text}` is first given here"), first)];
        let message = format!("the field `{text}` is given twice in this construction");
        self.say(self.construction_code(), message, field.id, Details { related, ..Details::default() });
        self.unjudged(&field.value);
    }

    fn fields_missing(&mut self, literal: &StructLit, missing: usize, shown: &[String]) {
        let list = shown.iter().map(|name| format!("`{name}`")).collect::<Vec<_>>().join(", ");
        let others = missing.saturating_sub(shown.len());
        let more = if others > 0 { format!(" and {others} more") } else { String::new() };
        let message = format!("this construction gives no value to {list}{more}: every field is given (5.1)");
        self.say(self.construction_code(), message, literal.id, Details::default());
    }

    fn struct_construction(&mut self, literal: &'a StructLit, definition: TyId) {
        let mut given: HashMap<Name, Id> = HashMap::with_capacity(literal.fields.len());
        let mut mistaken = false;
        for field in &literal.fields {
            if let Some(first) = given.get(&field.name).copied() {
                self.field_given_twice(field, first);
                mistaken = true;
                continue;
            }
            given.insert(field.name, field.id);
            match self.field_type(definition, field.name) {
                Some(expected) => self.check_piece(&field.value, expected, self.pass.names.resolve(field.name)),
                // Pass 3 said that it has no such field.
                None => {
                    self.unjudged(&field.value);
                    mistaken = true;
                }
            }
        }
        if mistaken {
            return;
        }
        let Ty::Struct(fields) = self.pass.table.get(definition) else { return };
        // A struct has no field twice, or it would be unknown.
        let total = fields.len();
        if given.len() >= total {
            return;
        }
        // It stops at the last one shown, having passed at most what the construction gives.
        let shown: Vec<String> = fields
            .iter()
            .filter(|(name, _)| !given.contains_key(name))
            .take(MISSING_SHOWN)
            .map(|(name, _)| cut_text(self.pass.names.resolve(*name)).into_owned())
            .collect();
        self.fields_missing(literal, total - given.len(), &shown);
    }

    /// `T` comes from `samples`, or from what the type says (`fixed`, of an alias), or else from what it goes to, for what has no type
    /// of its own. Without conversion: a `waveform<f64[V]>` is not a `waveform<f64[mV]>` (3.2).
    fn waveform_construction(&mut self, literal: &'a StructLit, fixed: Option<TyId>, hint: Option<TyId>) -> TyId {
        let wanted = fixed.or_else(|| {
            let definition = definition_of(self.pass, self.context, hint?);
            match self.pass.table.get(definition) {
                Ty::Waveform(element) => Some(*element),
                _ => None,
            }
        });
        // What is unknown around it says nothing of an empty array.
        let samples_type = if hint == Some(TyId::ERROR) && fixed.is_none() { hint } else { wanted.map(|element| self.array_of(element, literal.id)) };
        let mut firsts: [Option<Id>; 3] = [None; 3];
        let mut element = None;
        let mut mistaken = false;
        for field in &literal.fields {
            let text = self.text(field.name);
            let Some(place) = WAVEFORM_FIELDS.iter().position(|name| *name == text) else {
                // Pass 3 said that it has no such field.
                self.unjudged(&field.value);
                mistaken = true;
                continue;
            };
            if let Some(first) = firsts.get(place).copied().flatten() {
                self.field_given_twice(field, first);
                mistaken = true;
                continue;
            }
            if let Some(slot) = firsts.get_mut(place) {
                *slot = Some(field.id);
            }
            match place {
                0 => {
                    let expected = self.plain(Ty::Timestamp);
                    self.check_piece(&field.value, expected, text);
                }
                1 => {
                    let expected = self.plain(Ty::Duration);
                    self.check_piece(&field.value, expected, text);
                }
                _ => element = self.samples(&field.value, fixed, samples_type),
            }
        }
        if !mistaken {
            let shown: Vec<String> =
                WAVEFORM_FIELDS.iter().zip(firsts).filter(|(_, first)| first.is_none()).map(|(name, _)| (*name).to_owned()).collect();
            if !shown.is_empty() {
                self.fields_missing(literal, shown.len(), &shown);
            }
        }
        match element {
            Some(element) => self.intern_at(Ty::Waveform(element), literal.id),
            None => TyId::ERROR,
        }
    }

    /// The `T` that this `samples` gives, or nothing when it has a diagnostic.
    fn samples(&mut self, value: &'a Operand, fixed: Option<TyId>, expected: Option<TyId>) -> Option<TyId> {
        let found = self.value_of(value, expected);
        if found == TyId::ERROR {
            return None;
        }
        if let (Some(element), Some(expected)) = (fixed, expected) {
            self.judge_piece(value, found, expected, "samples");
            return Some(element);
        }
        let Some(element) = self.element_of(found) else {
            let found_text = self.show(found);
            let message = format!("`samples` is an array of numbers, and this is `{found_text}`");
            self.say(self.construction_code(), message, value.id(), Details { found: Some(found_text), ..Details::default() });
            return None;
        };
        if self.pass.table.has_error(element) {
            return None;
        }
        if self.number_of(element).is_none() {
            let found_text = self.show(found);
            let message = format!("a `waveform` holds numbers (3.2), and these `samples` are `{found_text}`");
            self.say(self.unknown_type_code(), message, value.id(), Details { found: Some(found_text), ..Details::default() });
            return None;
        }
        Some(element)
    }

    /// Each default against the type of its parameter (R-21). A variadic parameter has no default, which has its diagnostic.
    pub(super) fn defaults(&mut self, signature: &'a Signature, declared: &DeclaredFunction) {
        for (param, declared) in signature.params.iter().zip(&declared.params) {
            let Some(default) = param.default.as_ref().filter(|_| !declared.variadic) else { continue };
            self.within = Within::Default(param.name);
            let expected = declared.ty;
            match default {
                Constant::Operand(operand) => self.check_piece(operand, expected, self.pass.names.resolve(param.name)),
                Constant::Call(call) => {
                    let Some(bound) = self.resolved.call(call.id).cloned() else { continue };
                    let Meaning::Variant { ty, variant } = bound.callee else { continue };
                    self.variant_construction(call, &bound, ty, variant);
                    self.judge_built(call.id, ty, expected, param.name);
                }
                Constant::Struct(literal) => {
                    let found = self.construction(literal, Some(expected));
                    self.judge_built(literal.id, found, expected, param.name);
                }
            }
        }
        self.within = Within::Diagram;
    }

    /// What a default builds, against the type of its parameter. A declared type converts to nothing, and a `waveform` neither.
    fn judge_built(&mut self, id: Id, found: TyId, expected: TyId, param: Name) {
        if self.pass.table.matches(found, expected) {
            return;
        }
        let (expected_text, found_text) = (self.show(expected), self.show(found));
        let name = cut_text(self.text(param)).into_owned();
        let message = format!("`{name}` has the type `{expected_text}`, and this is `{found_text}`");
        let wires = vec![labcraft_types::Wire { value: None, from: None, to: Some(format!("fn.{name}")) }];
        self.say(Code::Lc3017, message, id, Details { wires, expected: Some(expected_text), found: Some(found_text), ..Details::default() });
    }
}
