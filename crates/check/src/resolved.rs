//! What passes 3 and 4 found out about a file, by `Id`, so that the layers above never resolve again
//! (`docs/architecture.md`, section 5).

use labcraft_graph::Id;
use labcraft_types::{Name, TyId};

pub use crate::primitives::PrimitiveLibrary;

/// What an item of a function means. It stays small: a file can hold millions of operands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Meaning {
    /// Nothing to say: an error has its diagnostic, or the id is not of a kind that means something.
    #[default]
    None,
    /// A value, by the id of what defines it: an input, an output, a carry, an `in` clause, a pattern, a line. A target
    /// that binds a function output has the id of that output. The other segments of a path are fields, for pass 4.
    Value(Id),
    /// A function of the library at this index of `Checked::libraries`.
    Function {
        library: u32,
        name: Name,
    },
    Primitive {
        library: PrimitiveLibrary,
        name: Name,
    },
    Variant {
        ty: TyId,
        variant: Name,
    },
    /// An error category, which is open: no list says which exist (7.3).
    Category,
    /// A panel element, by the id of its item.
    Element(Id),
    /// `[V]`, for a parameter of kind unit (R-5).
    Unit,
    /// `[x]`, an array of one value (R-5), by the id of what defines `x`.
    ArrayOfValue(Id),
    /// `[f]`, an array of one function of this library, as `ArrayOfValue` when no value has that name.
    ArrayOfFunction {
        library: u32,
        name: Name,
    },
    /// `[1]`, an array of one number (R-5).
    ArrayOfNumber,
    /// What a struct construction builds, or the type written on a carry.
    Type(TyId),
    /// `waveform { … }`: its type comes from its `samples` or from what it goes to (3.2).
    Waveform,
    /// A construction on a type of the language that is not a struct, such as `f64 { … }`, for pass 4 to say (`LC3015`).
    NotAStruct,
    /// `after: #close`, by the id of the label of that node.
    Label(Id),
    /// See [`Resolved::call`].
    Call(u32),
}

/// A call or a variant construction, bound to what it calls.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedCall {
    /// `None` after an error that has its diagnostic.
    pub callee: Meaning,
    /// One per argument, in written order.
    pub terminals: Box<[Terminal]>,
    /// One per type argument, in written order.
    pub type_args: Box<[TyId]>,
}

/// Which terminal an argument wires. `string.format` and `core.call` have terminals that only their arguments say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Terminal {
    /// By its position in the signature, or among the values of a variant. A variadic parameter takes several arguments.
    Param(u32),
    /// Not one of the parameters of the signature.
    Free,
    /// `after:`
    Order,
}

/// A conversion on a wire, which the IDE shows (spec 3.3, 3.4): `mV` to `V`, `i32` to `f64`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Coercion {
    /// The operand, or the target when a value is bound to what expects another type.
    pub at: Id,
    pub from: TyId,
    pub to: TyId,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Resolved {
    meanings: Vec<Meaning>,
    calls: Vec<ResolvedCall>,
    types: Vec<TyId>,
    coercions: Vec<Coercion>,
    leans_elsewhere: bool,
}

impl Resolved {
    /// `places` is the length of the source map of the file.
    pub(crate) fn new(places: usize) -> Self {
        Resolved {
            meanings: vec![Meaning::None; places],
            calls: Vec::new(),
            types: vec![TyId::ERROR; places],
            coercions: Vec::new(),
            leans_elsewhere: false,
        }
    }

    pub(crate) fn set(&mut self, id: Id, meaning: Meaning) {
        if let Some(slot) = self.meanings.get_mut(id.index()) {
            *slot = meaning;
        }
    }

    pub(crate) fn add_call(&mut self, id: Id, call: ResolvedCall) {
        let index = u32::try_from(self.calls.len()).unwrap_or(u32::MAX);
        self.calls.push(call);
        self.set(id, Meaning::Call(index));
    }

    /// What this id means, in the file it belongs to. `Meaning::None` for an id of another file past its end.
    pub fn of(&self, id: Id) -> Meaning {
        self.meanings.get(id.index()).copied().unwrap_or_default()
    }

    pub(crate) fn lean_elsewhere(&mut self) {
        self.leans_elsewhere = true;
    }

    /// Something of this file stays unknown, without a word of its own, because of a mistake that has its diagnostic in another
    /// file: a name that a file that did not read may declare, a type with an error. A run cannot lower it (`docs/cli.md`, 6.1).
    pub fn leans_elsewhere(&self) -> bool {
        self.leans_elsewhere
    }

    /// What each id of the file means, by the index of the id.
    pub fn meanings(&self) -> &[Meaning] {
        &self.meanings
    }

    /// The calls of the file, which [`Meaning::Call`] numbers.
    pub fn calls(&self) -> &[ResolvedCall] {
        &self.calls
    }

    /// The type of each id of the file, by the index of the id: see [`Resolved::ty`].
    pub fn types(&self) -> &[TyId] {
        &self.types
    }

    pub(crate) fn set_type(&mut self, id: Id, ty: TyId) {
        if let Some(slot) = self.types.get_mut(id.index()) {
            *slot = ty;
        }
    }

    pub(crate) fn add_coercion(&mut self, at: Id, from: TyId, to: TyId) {
        self.coercions.push(Coercion { at, from, to });
    }

    /// The type of what this id defines or reads: a value, a target, an operand. [`TyId::ERROR`] for what has none, or
    /// what an error left unknown.
    pub fn ty(&self, id: Id) -> TyId {
        self.types.get(id.index()).copied().unwrap_or(TyId::ERROR)
    }

    pub fn coercions(&self) -> &[Coercion] {
        &self.coercions
    }

    /// The call at this id.
    pub fn call(&self, id: Id) -> Option<&ResolvedCall> {
        match self.of(id) {
            Meaning::Call(index) => self.calls.get(index as usize),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_meaning_stays_small() {
        assert!(size_of::<Meaning>() <= 12, "{}", size_of::<Meaning>());
    }
}
