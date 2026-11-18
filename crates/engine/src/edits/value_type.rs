//! The type of a value, for `structure.surround` (commands.md 9.6): what kind of selector it is, and the variants of an
//! enumeration as a pattern writes them from the file.

use labcraft_check::Checked;
use labcraft_edit::{TypeKind, ValueType, VariantShape};
use labcraft_graph::{Body, ClauseKind, Decl, Id, Rhs, SourceMap, Statement, StatementKind, Target};
use labcraft_types::{Interner, LibraryId, Name, Ty, TyId, Unit};

/// The type of the value `name` bound on `line` of the file at `index` of `checked`, or of the input `name` when `line` is 0.
pub(super) fn of(checked: &Checked, names: &Interner, index: usize, line: u32, name: &str) -> ValueType {
    let Some(file) = checked.files.get(index) else { return ValueType::default() };
    let Some(wanted) = names.find(name) else { return ValueType::default() };
    let Decl::Function(function) = &file.file.decl else { return ValueType::default() };
    let id = if line == 0 {
        function.signature.params.iter().find(|param| param.name == wanted).map(|param| param.id)
    } else {
        find(&function.body, &file.source_map, line, wanted)
    };
    let Some(id) = id else { return ValueType::default() };
    let viewer = checked.libraries.get(file.library as usize).map(|library| library.id);
    describe(checked, names, file.resolved.ty(id), viewer)
}

/// The id that binds `name` in the statement that starts on `line`: a target, a pattern, a clause or a carry.
fn find(body: &Body, map: &SourceMap, line: u32, name: Name) -> Option<Id> {
    for statement in &body.statements {
        let starts = map.span(statement.id).is_some_and(|span| span.start.line == line);
        if starts && let Some(id) = binds(statement, name) {
            return Some(id);
        }
        if let StatementKind::Binding { value, .. } = &statement.kind {
            let inner: Vec<&Body> = match value {
                Rhs::Match(branching) | Rhs::Handle(branching) => branching.arms.iter().map(|arm| &arm.body).collect(),
                Rhs::For(each) => vec![&each.body],
                Rhs::Loop(repeat) => vec![&repeat.body],
                Rhs::Call(_) | Rhs::Struct(_) | Rhs::Operand(_) => Vec::new(),
            };
            for body in inner {
                if let Some(id) = find(body, map, line, name) {
                    return Some(id);
                }
            }
        }
    }
    None
}

fn binds(statement: &Statement, name: Name) -> Option<Id> {
    let StatementKind::Binding { targets, value } = &statement.kind else { return None };
    let target = targets.iter().find_map(|target| match target {
        Target::Name(id, found) if *found == name => Some(*id),
        _ => None,
    });
    if target.is_some() {
        return target;
    }
    match value {
        Rhs::Match(branching) | Rhs::Handle(branching) => branching
            .arms
            .iter()
            .flat_map(|arm| arm.patterns().flat_map(|pattern| pattern.bindings().iter()))
            .find(|binding| binding.name == name)
            .map(|binding| binding.id),
        Rhs::For(each) => each.clauses.iter().find_map(|clause| clause_binds(&clause.kind, name)),
        Rhs::Loop(repeat) => repeat.clauses.iter().find_map(|clause| clause_binds(&clause.kind, name)),
        Rhs::Call(_) | Rhs::Struct(_) | Rhs::Operand(_) => None,
    }
}

fn clause_binds(clause: &ClauseKind, name: Name) -> Option<Id> {
    match clause {
        ClauseKind::In(clause) if clause.name == name => Some(clause.id),
        ClauseKind::Carry(carry) if carry.name == name => Some(carry.id),
        _ => None,
    }
}

/// What a structure needs of a type, an alias followed to what it names.
fn describe(checked: &Checked, names: &Interner, ty: TyId, viewer: Option<LibraryId>) -> ValueType {
    let shown = viewer.map(|viewer| checked.types.display(ty, viewer, names)).unwrap_or_default();
    let mut current = ty;
    // A named type stands for its definition; an alias of an alias, at most a few steps.
    let mut named: Option<(LibraryId, Name)> = None;
    for _ in 0..16 {
        let Ty::Named { library, name } = checked.types.get(current) else { break };
        let Some(declared) = checked.libraries.iter().find(|found| found.id == *library).and_then(|found| found.types.get(name)) else {
            return ValueType { kind: TypeKind::Unknown, shown };
        };
        named = Some((*library, *name));
        if declared.definition == current {
            break;
        }
        current = declared.definition;
    }
    let kind = match checked.types.get(current) {
        Ty::Error => TypeKind::Unknown,
        Ty::Bool => TypeKind::Bool,
        Ty::Number { kind, unit } if kind.is_integer() && *unit == Unit::ONE => TypeKind::Integer,
        Ty::String => TypeKind::Text,
        Ty::ErrorValue => TypeKind::Error,
        Ty::Array { .. } => TypeKind::Array,
        Ty::Enum(variants) => {
            let Some((library, type_name)) = named else { return ValueType { kind: TypeKind::Other, shown } };
            let other = (Some(library) != viewer).then(|| checked.libraries.iter().find(|found| found.id == library)).flatten();
            let prefix = match other {
                Some(found) => format!("{}.{}", found.name, names.resolve(type_name)),
                None => names.resolve(type_name).to_owned(),
            };
            let variants = variants
                .iter()
                .map(|variant| VariantShape {
                    path: format!("{prefix}.{}", names.resolve(variant.name)),
                    values: variant.values.iter().map(|value| base_of(checked, names, *value)).collect(),
                })
                .collect();
            TypeKind::Enum { library: other.map(|found| found.name.clone()), variants }
        }
        _ => TypeKind::Other,
    };
    ValueType { kind, shown }
}

/// The base of the name of a value of a type (N-14): `step` for a `Step`, `value` for a type of spec 3.1.
fn base_of(checked: &Checked, names: &Interner, ty: TyId) -> String {
    match checked.types.get(ty) {
        Ty::Named { name, .. } => labcraft_edit::of_type(names.resolve(*name)),
        _ => "value".to_owned(),
    }
}
