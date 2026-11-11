//! What a run of one function reaches (`docs/cli.md`, 6.1): the functions it calls and names as references, and the
//! types they use, in every library. A run is refused for an error there, and for what lowering could not read.

use std::collections::{HashMap, HashSet};

use labcraft_graph::Decl;
use labcraft_types::{Interner, LibraryId, Name, Ty, TyId, normalize};

use crate::{Checked, Meaning};

/// What a run of one function reaches, by index in `Checked::files`, each file once, in the order it is found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reach {
    /// The function run first, then what it calls and names, through every library.
    pub functions: Vec<u32>,
    /// The `.lct` files of the types they use, and of the types those types use.
    pub types: Vec<u32>,
    /// What a run could not lower, though the file says nothing of it: the cause is in another file, which has its
    /// diagnostic (a file that did not read, a type with an error).
    pub unresolved: Vec<Unresolved>,
}

/// Why a reached function cannot be lowered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unresolved {
    /// A call of this file binds to nothing.
    Call { file: u32 },
    /// A value of this file, or its signature, has a type that an error left unknown.
    Type { file: u32 },
    /// A name of this file stays unknown because of a mistake in another file: a file that did not read may declare it, or
    /// its declaration has an error ([`crate::Resolved::leans_elsewhere`]).
    Elsewhere { file: u32 },
}

impl Reach {
    /// Whether `docs/cli.md`, 6.1, refuses the run: an error in what it reaches, its types included, or what could not be
    /// lowered; under `--deny-warnings`, also a warning in a reached file of the folder given. A warning in a dependency
    /// never refuses a run, since the user cannot fix it there. A file of the folder that the run reaches through a
    /// `--lib-path` copy of it, as a dependency, is a file of the folder: its warnings are those that the folder found in it.
    pub fn refuses(&self, checked: &Checked, deny_warnings: bool) -> bool {
        if !self.unresolved.is_empty() {
            return true;
        }
        let checked_library = |file: &crate::CheckedFile| checked.libraries.get(file.library as usize).is_some_and(|library| library.checked);
        // The files of the folder by how they are shown, which a copy shares.
        let of_the_folder: std::collections::HashMap<&str, usize> = if deny_warnings {
            checked.files.iter().enumerate().filter(|(_, file)| checked_library(file)).map(|(index, file)| (file.shown.as_str(), index)).collect()
        } else {
            std::collections::HashMap::new()
        };
        self.functions.iter().chain(&self.types).any(|file| {
            let counts = checked.counts_of(*file as usize);
            if counts.errors > 0 {
                return true;
            }
            let Some(reached) = checked.files.get(*file as usize).filter(|_| deny_warnings) else { return false };
            if checked_library(reached) {
                return counts.warnings > 0;
            }
            of_the_folder.get(reached.shown.as_str()).is_some_and(|index| checked.counts_of(*index).warnings > 0)
        })
    }
}

impl Checked {
    /// The file of the function that a run names from the root of the project (`docs/cli.md`, 6): `check_voltage`,
    /// `dmm_driver.measure`. Only a function of the folder given or of one of its sub-libraries.
    pub fn function_named(&self, path: &str, names: &Interner) -> Option<usize> {
        let segments: Vec<std::borrow::Cow<'_, str>> = path.split('.').map(normalize).collect();
        if segments.iter().any(|segment| segment.is_empty()) {
            return None;
        }
        let (function, library) = segments.split_last()?;
        let library = library.join(".");
        let function = names.find(function)?;
        let library = self.libraries.iter().find(|declared| declared.checked && declared.name == library)?;
        library.functions.get(&function)?.file.map(|file| file as usize)
    }

    /// What a run of the function in this file of `files` reaches. Only for a [`crate::check_for_run`] (`for_run`): otherwise
    /// the files of the dependencies are not there, and nothing of theirs is reached.
    pub fn reach(&self, function: usize) -> Reach {
        let mut reach = Reach::default();
        let libraries: HashMap<LibraryId, usize> = self.libraries.iter().enumerate().map(|(index, library)| (library.id, index)).collect();
        let mut seen_files: HashSet<u32> = HashSet::new();
        let mut seen_types: HashSet<TyId> = HashSet::new();
        let mut pending_functions: Vec<u32> = Vec::new();
        let mut pending_types: Vec<TyId> = Vec::new();
        if let Ok(first) = u32::try_from(function)
            && self.files.get(function).is_some()
        {
            seen_files.insert(first);
            pending_functions.push(first);
        }
        // Calls and references, file by file: a work list, since a chain of calls can be as long as a library.
        let mut next = 0;
        while let Some(&file) = pending_functions.get(next) {
            next += 1;
            reach.functions.push(file);
            let Some(checked) = self.files.get(file as usize) else { continue };
            let mut unresolved_call = false;
            let mut unresolved_type = false;
            let function_of = |library: u32, name: Name| self.libraries.get(library as usize)?.functions.get(&name)?.file;
            for meaning in checked.resolved.meanings() {
                let callee = match *meaning {
                    Meaning::Function { library, name } | Meaning::ArrayOfFunction { library, name } => function_of(library, name),
                    Meaning::Call(index) => match checked.resolved.calls().get(index as usize).map(|call| call.callee) {
                        Some(Meaning::Function { library, name }) => function_of(library, name),
                        Some(Meaning::None) | None => {
                            unresolved_call = true;
                            None
                        }
                        // A primitive, or a variant built by a call.
                        Some(_) => None,
                    },
                    Meaning::Value(id) => {
                        unresolved_type |= checked.resolved.ty(id) == TyId::ERROR;
                        None
                    }
                    _ => None,
                };
                if let Some(callee) = callee
                    && seen_files.insert(callee)
                {
                    pending_functions.push(callee);
                }
            }
            // The types of its signature, read even when nothing reads an input.
            let signature = match &checked.file.decl {
                Decl::Function(function) => Some(function.name),
                Decl::Extern(function) => Some(function.name),
                Decl::Type(_) => None,
            };
            if let Some(name) = signature
                && let Some(declared) = self.libraries.get(checked.library as usize).and_then(|library| library.functions.get(&name))
            {
                let tys = declared.params.iter().map(|param| param.ty).chain(declared.outputs.iter().map(|output| output.ty));
                for ty in tys {
                    unresolved_type |= self.types.has_error(ty);
                    if seen_types.insert(ty) {
                        pending_types.push(ty);
                    }
                }
            }
            // A type that holds the error type, `[Place]` with a `Place` that has an error, is no value a run can make, whatever
            // passes 3 and 4 marked: a net under every place where they stay silent. The error type alone is also what an id
            // with no type gets, so only a value says it is unknown (above).
            for ty in checked.resolved.types() {
                if *ty == TyId::ERROR {
                    continue;
                }
                unresolved_type |= self.types.has_error(*ty);
                if seen_types.insert(*ty) {
                    pending_types.push(*ty);
                }
            }
            if unresolved_call {
                reach.unresolved.push(Unresolved::Call { file });
            }
            if unresolved_type {
                reach.unresolved.push(Unresolved::Type { file });
            }
            if checked.resolved.leans_elsewhere() {
                reach.unresolved.push(Unresolved::Elsewhere { file });
            }
        }
        // The types, down to the `.lct` files that declare them.
        while let Some(ty) = pending_types.pop() {
            let mut children: Vec<TyId> = match self.types.get(ty) {
                Ty::Array { element: child, .. } | Ty::Waveform(child) | Ty::Queue(child) | Ty::Ref(child) => vec![*child],
                Ty::Struct(fields) => fields.iter().map(|(_, ty)| *ty).collect(),
                Ty::Enum(variants) => variants.iter().flat_map(|variant| variant.values.iter().copied()).collect(),
                Ty::Function { params, outputs } => params.iter().map(|param| param.ty).chain(outputs.iter().copied()).collect(),
                Ty::Named { library, name } => {
                    let declared = libraries.get(library).and_then(|index| self.libraries.get(*index));
                    if let Some(file) = declared.and_then(|declared| declared.type_files.get(name))
                        && seen_files.insert(*file)
                    {
                        reach.types.push(*file);
                    }
                    declared.and_then(|declared| declared.types.get(name)).map(|declared| declared.definition).into_iter().collect()
                }
                Ty::Error
                | Ty::Bool
                | Ty::Number { .. }
                | Ty::String
                | Ty::Bytes
                | Ty::Timestamp
                | Ty::Duration
                | Ty::ErrorValue
                | Ty::Variable(_) => Vec::new(),
            };
            children.retain(|child| seen_types.insert(*child));
            pending_types.append(&mut children);
        }
        reach
    }
}

#[cfg(test)]
mod tests;
