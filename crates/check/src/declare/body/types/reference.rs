//! A primitive as a value (section 4). One without type variables has a function type of its own. A generic one takes its variables
//! from the function type that it goes to, and matches it exactly, since no node would hold a conversion. A parameter of kind unit,
//! category or element has no function type, and neither has what takes more than its parameters.

use labcraft_graph::Path;
use labcraft_types::{Code, FnParam, Name, NumberKind, Ty, TyId, Unit, cut_text};

use super::super::definition_of;
use super::{Details, Primitive, PrimitiveLibrary, PrimitiveOutput, PrimitiveTy, Typer, Within, floated, is_ordered};
use crate::primitives::ParamKind;

impl<'a> Typer<'_, '_, '_, 'a> {
    /// Where a value goes decides the code, as for any reference (`docs/cli.md`, 4.2).
    fn reference_code(&self) -> Code {
        match self.within {
            Within::Diagram => Code::Lc3008,
            Within::Construction | Within::Default(_) => self.construction_code(),
        }
    }

    pub(super) fn primitive_reference(&mut self, path: &Path, library: PrimitiveLibrary, name: Name, hint: Option<TyId>) -> TyId {
        let Some(primitive) = self.context.primitives.signature(library, name) else { return TyId::ERROR };
        if hint == Some(TyId::ERROR) {
            return TyId::ERROR;
        }
        let names = self.pass.names;
        let text = || cut_text(&super::path_text(path, names)).into_owned();
        if primitive.open || primitive.params.iter().any(|param| param.kind != ParamKind::Value) {
            let text = text();
            let message = format!("`{text}` has no function type: it takes a unit, a category, an element or a free list of inputs (4)");
            self.say(self.reference_code(), message, path.id, Details::default());
            return TyId::ERROR;
        }
        if primitive.variables.names.is_empty() {
            return self.concrete_reference(primitive);
        }
        let Some(hint) = hint else {
            let text = text();
            let message = format!("`{text}` is generic, and its type comes from the function type that it goes to: nothing says one here (4)");
            self.say(self.unknown_type_code(), message, path.id, Details::default());
            return TyId::ERROR;
        };
        if self.instantiates(primitive, hint) {
            return hint;
        }
        let (text, expected) = (text(), self.show(hint));
        let message = format!("`{text}` does not match the type expected, `{expected}`: a reference matches exactly (4)");
        self.say(self.reference_code(), message, path.id, Details { expected: Some(expected), ..Details::default() });
        TyId::ERROR
    }

    /// A default value does not shorten a signature, and a variadic parameter counts as an array (4).
    fn concrete_reference(&mut self, primitive: &'static Primitive) -> TyId {
        let mut params = Vec::with_capacity(primitive.params.len());
        for param in primitive.params {
            let Some(ty) = self.concrete(param.ty) else { return TyId::ERROR };
            let ty = if param.variadic { self.plain(Ty::Array { element: ty, dimensions: 1 }) } else { ty };
            params.push(FnParam { name: None, ty });
        }
        let mut outputs = Vec::with_capacity(primitive.results.len());
        for result in primitive.results {
            let PrimitiveOutput::Is(ty) = result else { return TyId::ERROR };
            let Some(ty) = self.concrete(*ty) else { return TyId::ERROR };
            outputs.push(ty);
        }
        self.plain(Ty::Function { params, outputs })
    }

    /// Whether this function type can be this generic primitive, with its variables taken from the inputs.
    fn instantiates(&mut self, primitive: &'static Primitive, hint: TyId) -> bool {
        let definition = definition_of(self.pass, self.context, hint);
        let Ty::Function { params, outputs } = self.pass.table.get(definition) else { return false };
        // Compared before anything is copied: a function type can have a million inputs and be expected a million times.
        if params.len() != primitive.params.len() || outputs.len() != primitive.results.len() {
            return false;
        }
        let (inputs, results): (Vec<TyId>, Vec<TyId>) = (params.iter().map(|param| param.ty).collect(), outputs.clone());
        let mut variables: [Option<TyId>; 2] = [None; 2];
        for (param, input) in primitive.params.iter().zip(inputs) {
            let fits = match param.ty {
                PrimitiveTy::Var(index) => self.bind_variable(primitive, &mut variables, index, input),
                PrimitiveTy::ArrayOf(index) => match self.element_of(input) {
                    Some(element) => self.bind_variable(primitive, &mut variables, index, element),
                    None => false,
                },
                other => self.concrete(other).is_some_and(|ty| self.pass.table.matches(ty, input)),
            };
            if !fits {
                return false;
            }
        }
        primitive.results.iter().zip(results).all(|(result, expected)| {
            let output = self.reference_output(primitive, *result, &variables);
            output.is_some_and(|output| self.pass.table.matches(output, expected))
        })
    }

    /// A `duration` counts as an `f64[s]`, as for a node (3.5).
    fn bind_variable(&mut self, primitive: &'static Primitive, variables: &mut [Option<TyId>; 2], index: usize, ty: TyId) -> bool {
        let mut ty = ty;
        if primitive.variables.numbers {
            let Some(number) = self.as_number(ty) else { return false };
            ty = number;
            if is_ordered(primitive) && self.number_of(ty).is_some_and(|(kind, _)| kind.is_complex()) {
                return false;
            }
        }
        let Some(slot) = variables.get_mut(index) else { return false };
        match slot {
            Some(known) => self.pass.table.matches(*known, ty),
            None => {
                *slot = Some(ty);
                true
            }
        }
    }

    /// What a node of this primitive gives, by the rules of 3.3 and 3.4. `core.pow` has a unit only with a constant exponent.
    fn reference_output(&mut self, primitive: &'static Primitive, result: PrimitiveOutput, variables: &[Option<TyId>; 2]) -> Option<TyId> {
        let variable = |index: usize| variables.get(index).copied().flatten();
        match result {
            PrimitiveOutput::Is(PrimitiveTy::Var(index)) => {
                let ty = variable(index)?;
                let is_sub = primitive.library == PrimitiveLibrary::Core && primitive.name == "sub";
                match self.number_of(ty) {
                    Some((kind, unit)) if is_sub && unit.is_temperature_scale() => Some(self.number(floated(kind)?, Unit::KELVIN)),
                    _ => Some(ty),
                }
            }
            PrimitiveOutput::Is(PrimitiveTy::ArrayOf(index)) => Some(self.plain(Ty::Array { element: variable(index)?, dimensions: 1 })),
            PrimitiveOutput::Is(ty) => self.concrete(ty),
            PrimitiveOutput::Product | PrimitiveOutput::Quotient => {
                let ((a_kind, a_unit), (b_kind, b_unit)) = (self.number_of(variable(0)?)?, self.number_of(variable(1)?)?);
                let met = a_kind.meeting(b_kind)?;
                let (kind, unit) =
                    if result == PrimitiveOutput::Product { (met, a_unit.mul(&b_unit)?) } else { (floated(met)?, a_unit.div(&b_unit)?) };
                Some(self.number(kind, unit))
            }
            PrimitiveOutput::Root => {
                let (kind, unit) = self.number_of(variable(0)?)?;
                Some(self.number(floated(kind)?, unit.sqrt()?))
            }
            PrimitiveOutput::Power => {
                let (kind, unit) = self.number_of(variable(0)?)?;
                let kind: NumberKind = floated(kind)?;
                (unit == Unit::ONE).then(|| self.number(kind, Unit::ONE))
            }
            PrimitiveOutput::WithUnit => None,
        }
    }
}
