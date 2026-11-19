//! `functions.list` (`docs/commands.md`, 6.1): what the project can call, each as a signature written in the language,
//! as appendix A of the spec writes it. The primitives come from the table of `check`, the shipped libraries from their
//! files, and the project's own entries from the last check of the root, so that they agree with it.

use std::collections::HashMap;

use labcraft_check::manifest::Manifest;
use labcraft_check::primitives::{PRIMITIVES, ParamKind, Primitive, PrimitiveLibrary, PrimitiveOutput, PrimitiveParam, PrimitiveTy, Variables};
use labcraft_graph::{Attached, CommentKind, Decl, File, Generic, NamedType, Param, Path as NamePath, TypeExpr};
use labcraft_types::listing::{Slot, SlotFit, Typed, VariantSlots};
use labcraft_types::{Interner, Limits};
use serde_json::{Value, json};

use crate::error::CommandError;
use crate::params::Params;
use crate::session::Session;

/// At most this many entries *(proposed)*, primitives and shipped libraries first.
pub const MAX_ENTRIES: usize = 1000;
/// A signature longer than this is cut, with `…`.
const MAX_SIGNATURE: usize = 2000;
/// The typed parts of one entry (its inputs, outputs, fields, variants or alias) hold at most about this many bytes
/// *(proposed)*: past it, the rest is left out and the entry says `truncated`.
pub const MAX_TYPED: usize = 64 * 1024;
/// An array of a type variable is an `array` in at most this many arrays *(proposed)*; in more, a `shape`.
const MAX_ARRAYS: usize = 16;

struct Entry {
    library: String,
    name: String,
    kind: &'static str,
    signature: String,
    always: bool,
    categories: Vec<String>,
    doc: Vec<String>,
    typed: Typed,
}

/// The categories of the errors that a primitive can make itself (spec, appendix A and 7.2).
fn categories_of(primitive: &Primitive) -> &'static [&'static str] {
    match (primitive.library.as_str(), primitive.name) {
        ("array", "get" | "set" | "slice") => &["array.index_out_of_range"],
        ("array", "repeat") => &["array.negative_size"],
        ("array", "from_rows") => &["array.unequal_rows"],
        ("string", "to_f64" | "to_i32") => &["string.parse"],
        ("bytes", "to_string") => &["bytes.invalid_text"],
        (
            "bytes",
            "slice" | "read_u8" | "read_u16" | "read_i16" | "read_u32" | "read_i32" | "read_f32" | "from_u8" | "from_u16" | "from_i16" | "from_u32"
            | "from_i32",
        ) => &["bytes.out_of_range"],
        ("core", "call") => &["core.invalid_function"],
        _ => &[],
    }
}

fn type_text(ty: PrimitiveTy, variables: &[&str]) -> String {
    ty.text(variables)
}

fn param_text(param: &PrimitiveParam, variables: &[&str]) -> String {
    let ty = match param.kind {
        ParamKind::Value => type_text(param.ty, variables),
        ParamKind::Unit => "kind unit".to_owned(),
        ParamKind::Category => "kind category".to_owned(),
        ParamKind::Element => "kind element".to_owned(),
    };
    let default = param.default.map_or_else(String::new, |default| format!(" = {}", default.text()));
    format!("{}: {ty}{default}{}", param.name, if param.variadic { "..." } else { "" })
}

fn output_text(output: PrimitiveOutput, variables: &[&str]) -> String {
    match output {
        PrimitiveOutput::Is(ty) => type_text(ty, variables),
        PrimitiveOutput::Product => "A·B".to_owned(),
        PrimitiveOutput::Quotient => "A/B".to_owned(),
        PrimitiveOutput::Root => "√N".to_owned(),
        PrimitiveOutput::Power => "N^exponent".to_owned(),
        PrimitiveOutput::WithUnit => "f64[unit]".to_owned(),
        PrimitiveOutput::Slice => "[T]".to_owned(),
    }
}

/// `fn core.add<N: Number>(a: N, b: N) -> (result: N)`, as appendix A writes it.
pub fn primitive_signature(primitive: &Primitive) -> String {
    let variables = primitive.variables.names;
    let generics = if variables.is_empty() {
        String::new()
    } else if primitive.variables.numbers {
        format!("<{}>", variables.iter().map(|name| format!("{name}: Number")).collect::<Vec<_>>().join(", "))
    } else {
        format!("<{}>", variables.join(", "))
    };
    let mut params: Vec<String> = primitive.params.iter().map(|param| param_text(param, variables)).collect();
    if primitive.open {
        params.push("…".to_owned());
    }
    let outputs = match primitive.outputs {
        Some(names) => {
            let outputs: Vec<String> =
                names.iter().zip(primitive.results).map(|(name, result)| format!("{name}: {}", output_text(*result, variables))).collect();
            format!("({})", outputs.join(", "))
        }
        None => "(…)".to_owned(),
    };
    format!("fn {}.{}{generics}({}) -> {outputs}", primitive.library.as_str(), primitive.name, params.join(", "))
}

/// A slot of a type that the rule of fit judges: `text` as the language writes it.
fn slot(name: &str, text: String, fit: Option<SlotFit>) -> Slot {
    Slot { name: name.to_owned(), ty: text, fit, ..Slot::default() }
}

/// The name of a type variable of a primitive, `T` past those it has.
fn variable_name(variables: &Variables, index: usize) -> String {
    variables.names.get(index).copied().unwrap_or("T").to_owned()
}

fn primitive_variable(variables: &Variables, index: usize) -> SlotFit {
    SlotFit::Variable { variable: variable_name(variables, index), bound: variables.numbers.then(|| "Number".to_owned()) }
}

/// What the rule of fit reads of a type of appendix A; None for a parameter of a kind, which takes no wire.
fn primitive_fit(ty: PrimitiveTy, variables: &Variables) -> Option<SlotFit> {
    let number = |number: &str| SlotFit::Number { number: number.to_owned(), unit: None };
    let base = |base: &str| SlotFit::Base { base: base.to_owned() };
    Some(match ty {
        PrimitiveTy::Bool => base("bool"),
        PrimitiveTy::I32 => number("i32"),
        PrimitiveTy::F32 => number("f32"),
        PrimitiveTy::F64 => number("f64"),
        PrimitiveTy::U8 => number("u8"),
        PrimitiveTy::U16 => number("u16"),
        PrimitiveTy::I16 => number("i16"),
        PrimitiveTy::U32 => number("u32"),
        PrimitiveTy::I64 => number("i64"),
        PrimitiveTy::Text => base("string"),
        PrimitiveTy::Bytes => base("bytes"),
        PrimitiveTy::Timestamp => base("timestamp"),
        PrimitiveTy::Duration => base("duration"),
        PrimitiveTy::Error => base("error"),
        PrimitiveTy::Var(index) => primitive_variable(variables, index),
        PrimitiveTy::ArrayOf(index) => SlotFit::Array { array: Box::new(primitive_variable(variables, index)), dimensions: None },
        PrimitiveTy::Rows(index) => SlotFit::Array {
            array: Box::new(SlotFit::Array { array: Box::new(primitive_variable(variables, index)), dimensions: None }),
            dimensions: None,
        },
        PrimitiveTy::Rectangle(index) => SlotFit::Array { array: Box::new(primitive_variable(variables, index)), dimensions: Some(2) },
        PrimitiveTy::Texts | PrimitiveTy::U8s | PrimitiveTy::Function => SlotFit::Shape { shape: ty.text(variables.names) },
        PrimitiveTy::None => return None,
    })
}

/// The inputs and outputs of a primitive, from its row of the table of `check`.
fn primitive_typed(primitive: &Primitive) -> Typed {
    let variables = &primitive.variables;
    let names = variables.names;
    let variadic_at = primitive.params.iter().position(|param| param.variadic);
    // The template of `string.format` takes only a literal (spec 5.4).
    let format = primitive.library.as_str() == "string" && primitive.name == "format";
    let inputs = primitive
        .params
        .iter()
        .enumerate()
        .map(|(rank, param)| {
            let kind = param.kind != ParamKind::Value;
            let literal = kind || (format && rank == 0);
            let text = if kind {
                param_text(param, names).split_once(": ").map(|(_, ty)| ty.to_owned()).unwrap_or_default()
            } else {
                type_text(param.ty, names)
            };
            let fit = if literal { None } else { primitive_fit(param.ty, variables) };
            Slot {
                required: param.default.is_none() && !param.variadic,
                variadic: param.variadic,
                named_only: param.default.is_some() && variadic_at.is_some_and(|at| at < rank),
                literal,
                ..slot(param.name, text, fit)
            }
        })
        .collect();
    let outputs = primitive
        .outputs
        .unwrap_or_default()
        .iter()
        .zip(primitive.results)
        .map(|(name, result)| {
            let all = || names.iter().map(|name| (*name).to_owned()).collect::<Vec<_>>();
            let fit = match result {
                PrimitiveOutput::Is(ty) => primitive_fit(*ty, variables),
                PrimitiveOutput::Product | PrimitiveOutput::Quotient | PrimitiveOutput::Root => Some(SlotFit::Numbers { numbers: all() }),
                PrimitiveOutput::Power => Some(SlotFit::Numbers { numbers: all().into_iter().chain(["exponent".to_owned()]).collect() }),
                PrimitiveOutput::WithUnit => Some(SlotFit::Numbers { numbers: vec!["unit".to_owned()] }),
                PrimitiveOutput::Slice => Some(SlotFit::Array { array: Box::new(primitive_variable(variables, 0)), dimensions: None }),
            };
            slot(name, output_text(*result, names), fit)
        })
        .collect();
    Typed { inputs, outputs, ..Typed::default() }
}

/// What a declaration's types are read with: its names, and the type variables in scope with their bounds.
struct Scope<'a> {
    names: &'a Interner,
    generics: &'a [Generic],
}

impl Scope<'_> {
    fn path(&self, path: &NamePath) -> String {
        std::iter::once(path.first).chain(path.rest.iter().copied()).map(|name| self.names.resolve(name)).collect::<Vec<_>>().join(".")
    }

    fn text(&self, ty: &TypeExpr) -> String {
        labcraft_format::write_type(ty, self.names, MAX_SIGNATURE)
    }

    /// What the rule of fit reads of a written type: a number and its unit, a type of no parts, a type variable, an
    /// array of one; for any other, its text.
    fn fit(&self, ty: &TypeExpr) -> SlotFit {
        self.fit_at(ty, 0)
    }

    /// [`Scope::fit`] at `depth` arrays down: past [`MAX_ARRAYS`], the whole array is a `shape`.
    fn fit_at(&self, ty: &TypeExpr, depth: usize) -> SlotFit {
        match ty {
            TypeExpr::Named(NamedType { path, args, unit, .. }) if path.rest.is_empty() && args.is_empty() => {
                if let Some(generic) = self.generics.iter().find(|generic| generic.name == path.first) {
                    return SlotFit::Variable {
                        variable: self.names.resolve(generic.name).to_owned(),
                        bound: generic.bound.as_ref().map(|bound| self.path(bound)),
                    };
                }
                match SlotFit::of_base(self.names.resolve(path.first)) {
                    Some(SlotFit::Number { number, .. }) => {
                        SlotFit::Number { number, unit: unit.as_ref().map(|unit| labcraft_format::write_unit(unit, self.names, MAX_SIGNATURE)) }
                    }
                    Some(fit) if unit.is_none() => fit,
                    _ => SlotFit::Shape { shape: self.text(ty) },
                }
            }
            TypeExpr::Array(array) if depth < MAX_ARRAYS => match self.fit_at(&array.element, depth + 1) {
                element @ (SlotFit::Variable { .. } | SlotFit::Array { .. }) => SlotFit::Array {
                    array: Box::new(element),
                    dimensions: array.dimensions.and_then(|dimensions| self.names.resolve(dimensions).parse().ok()),
                },
                _ => SlotFit::Shape { shape: self.text(ty) },
            },
            _ => SlotFit::Shape { shape: self.text(ty) },
        }
    }

    fn slot(&self, name: Option<labcraft_types::Name>, ty: &TypeExpr) -> Slot {
        slot(name.map(|name| self.names.resolve(name)).unwrap_or_default(), self.text(ty), Some(self.fit(ty)))
    }

    fn inputs(&self, params: &[Param]) -> Vec<Slot> {
        let variadic_at = params.iter().position(|param| param.variadic);
        params
            .iter()
            .enumerate()
            .map(|(rank, param)| Slot {
                required: param.default.is_none() && !param.variadic,
                variadic: param.variadic,
                named_only: param.default.is_some() && variadic_at.is_some_and(|at| at < rank),
                ..self.slot(Some(param.name), &param.ty)
            })
            .collect()
    }
}

/// The inputs and outputs of a declaration, or the fields, variants or what it names of a type; its own types already
/// qualified.
fn declaration_typed(decl: &Decl, names: &Interner) -> Typed {
    match decl {
        Decl::Function(function) => signature_typed(&function.signature, names),
        Decl::Extern(function) => signature_typed(&function.signature, names),
        Decl::Type(ty) => {
            let scope = Scope { names, generics: &ty.generics };
            match &ty.ty {
                TypeExpr::Struct(structure) => {
                    Typed { fields: structure.fields.iter().map(|field| scope.slot(Some(field.name), &field.ty)).collect(), ..Typed::default() }
                }
                TypeExpr::Enum(enumeration) => Typed {
                    variants: enumeration
                        .variants
                        .iter()
                        .map(|variant| VariantSlots {
                            name: names.resolve(variant.name).to_owned(),
                            values: variant.payload.iter().map(|ty| scope.slot(None, ty)).collect(),
                        })
                        .collect(),
                    ..Typed::default()
                },
                TypeExpr::Named(NamedType { path, args, unit: None, .. })
                    if path.rest.is_empty() && args.is_empty() && names.resolve(path.first) == "opaque" =>
                {
                    Typed::default()
                }
                other => Typed { is: Some(scope.slot(None, other)), ..Typed::default() },
            }
        }
    }
}

fn signature_typed(signature: &labcraft_graph::Signature, names: &Interner) -> Typed {
    let scope = Scope { names, generics: &signature.generics };
    Typed {
        inputs: scope.inputs(&signature.params),
        outputs: signature.outputs.iter().map(|output| scope.slot(Some(output.name), &output.ty)).collect(),
        ..Typed::default()
    }
}

/// What a fit is followed to: an alias to what it names (`psu.session` is a `visa.session`), across the libraries, at
/// most 16 times.
fn resolve<'a>(aliases: &'a HashMap<String, SlotFit>, fit: &'a SlotFit) -> &'a SlotFit {
    let mut now = fit;
    for _ in 0..16 {
        match now {
            SlotFit::Shape { shape } => match aliases.get(shape.as_str()) {
                Some(next) if next != now => now = next,
                _ => break,
            },
            _ => break,
        }
    }
    now
}

/// About the bytes that a fit takes once written.
fn fit_size(fit: &SlotFit) -> usize {
    16 + match fit {
        SlotFit::Number { number, unit } => number.len() + unit.as_ref().map_or(0, String::len),
        SlotFit::Base { base } => base.len(),
        SlotFit::Variable { variable, bound } => variable.len() + bound.as_ref().map_or(0, String::len),
        SlotFit::Numbers { numbers } => numbers.iter().map(|name| name.len() + 4).sum(),
        SlotFit::Array { array, .. } => fit_size(array),
        SlotFit::Shape { shape } => shape.len(),
    }
}

/// What is left of [`MAX_TYPED`] for one entry.
struct Budget<'a> {
    aliases: &'a HashMap<String, SlotFit>,
    used: usize,
    full: bool,
}

impl Budget<'_> {
    /// Whether the slot is kept, its alias followed: false once the budget is spent, for it and every slot after it.
    fn keep(&mut self, slot: &mut Slot) -> bool {
        if self.full {
            return false;
        }
        let resolved = slot.fit.as_ref().map(|fit| resolve(self.aliases, fit));
        let size = 32 + slot.name.len() + slot.ty.len() + resolved.map_or(0, fit_size);
        if self.used + size > MAX_TYPED {
            self.full = true;
            return false;
        }
        self.used += size;
        let resolved = resolved.cloned();
        slot.fit = resolved;
        true
    }
}

/// Each alias followed to what it names, and the typed parts of each entry cut at [`MAX_TYPED`]: an alias of a long type
/// named by many slots would otherwise be copied into each.
fn follow_aliases(entries: &mut [Entry]) {
    let aliases: HashMap<String, SlotFit> =
        entries.iter().filter_map(|entry| Some((entry.name.clone(), entry.typed.is.as_ref()?.fit.clone()?))).collect();
    for entry in entries {
        let typed = &mut entry.typed;
        let mut budget = Budget { aliases: &aliases, used: 0, full: false };
        typed.inputs.retain_mut(|slot| budget.keep(slot));
        typed.outputs.retain_mut(|slot| budget.keep(slot));
        typed.fields.retain_mut(|slot| budget.keep(slot));
        typed.variants.retain_mut(|variant| {
            if budget.full || budget.used + 32 + variant.name.len() > MAX_TYPED {
                budget.full = true;
                return false;
            }
            budget.used += 32 + variant.name.len();
            variant.values.retain_mut(|slot| budget.keep(slot));
            true
        });
        if let Some(is) = &mut typed.is
            && !budget.keep(is)
        {
            typed.is = None;
        }
        typed.truncated = budget.full;
    }
}

/// The documentation of a declaration: its `///` lines, without the marks.
fn doc_of(comments: &Attached) -> Vec<String> {
    comments
        .before
        .iter()
        .filter(|comment| comment.kind == CommentKind::Doc)
        .map(|comment| labcraft_types::cut_text(comment.text.strip_prefix(' ').unwrap_or(&comment.text).trim_end()).into_owned())
        .collect()
}

/// Names a type of its own library by the library, as a program that uses it writes it: `session` in `visa` is
/// `visa.session`. Only a path of one segment that names a type of the library is changed.
fn qualify(ty: &mut TypeExpr, library: &[labcraft_types::Name], own: &dyn Fn(labcraft_types::Name) -> bool) {
    match ty {
        TypeExpr::Named(NamedType { path, args, .. }) => {
            if path.rest.is_empty()
                && own(path.first)
                && let Some((first, rest)) = library.split_first()
            {
                let mut segments: Vec<labcraft_types::Name> = rest.to_vec();
                segments.push(path.first);
                *path = NamePath { id: path.id, first: *first, rest: segments.into_boxed_slice() };
            }
            for arg in args {
                qualify(arg, library, own);
            }
        }
        TypeExpr::Array(array) => qualify(&mut array.element, library, own),
        TypeExpr::Struct(structure) => structure.fields.iter_mut().for_each(|field| qualify(&mut field.ty, library, own)),
        TypeExpr::Enum(enumeration) => {
            enumeration.variants.iter_mut().for_each(|variant| variant.payload.iter_mut().for_each(|ty| qualify(ty, library, own)))
        }
        TypeExpr::Fn(function) => {
            function.params.iter_mut().for_each(|arg| qualify(&mut arg.ty, library, own));
            function.outputs.iter_mut().for_each(|ty| qualify(ty, library, own));
        }
    }
}

/// The entries of the files of one library, named `library.name`, its own types qualified.
fn entries_of(library: &str, files: &[&File], names: &mut Interner, manifest: Option<&Manifest>) -> Vec<Entry> {
    let segments: Vec<labcraft_types::Name> =
        if library.is_empty() { Vec::new() } else { library.split('.').map(|segment| names.intern(segment)).collect() };
    let own_types: Vec<labcraft_types::Name> = files
        .iter()
        .filter_map(|file| match &file.decl {
            Decl::Type(ty) => Some(ty.name),
            _ => None,
        })
        .collect();
    let own = |name: labcraft_types::Name| own_types.contains(&name);
    let mut entries = Vec::new();
    for file in files {
        let mut decl = file.decl.clone();
        let (name, kind, doc) = match &mut decl {
            Decl::Function(function) => {
                function.signature.params.iter_mut().for_each(|param| qualify(&mut param.ty, &segments, &own));
                function.signature.outputs.iter_mut().for_each(|output| qualify(&mut output.ty, &segments, &own));
                (function.name, "function", doc_of(&function.comments))
            }
            Decl::Extern(function) => {
                function.signature.params.iter_mut().for_each(|param| qualify(&mut param.ty, &segments, &own));
                function.signature.outputs.iter_mut().for_each(|output| qualify(&mut output.ty, &segments, &own));
                (function.name, "function", doc_of(&function.comments))
            }
            Decl::Type(ty) => (ty.name, "type", doc_of(&ty.comments)),
        };
        let short = names.resolve(name).to_owned();
        let full = if library.is_empty() { short.clone() } else { format!("{library}.{short}") };
        let signature = labcraft_format::write_declaration(&decl, &full, names, MAX_SIGNATURE);
        // A type's own types are qualified for its parts, its signature staying as it was: `c: cell` in `lab` is a
        // `lab.cell`, never the `cell` of another library.
        let typed = match &decl {
            Decl::Type(ty) => {
                let mut qualified = ty.clone();
                qualify(&mut qualified.ty, &segments, &own);
                declaration_typed(&Decl::Type(qualified), names)
            }
            _ => declaration_typed(&decl, names),
        };
        let always = kind == "function" && manifest.is_some_and(|manifest| manifest.always.contains(&short));
        let categories = if kind == "function" { manifest.map(|manifest| manifest.categories.clone()).unwrap_or_default() } else { Vec::new() };
        entries.push(Entry { library: library.to_owned(), name: full, kind, signature, always, categories, doc, typed });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries
}

/// The shipped libraries, each file read alone, as `check` reads a dependency for its declarations.
fn shipped_entries(session: &Session, limits: &Limits) -> Vec<Entry> {
    let mut entries = Vec::new();
    for library in session.config.shipped.libraries() {
        let mut names = Interner::new();
        let files: Vec<File> = library.files.iter().filter_map(|file| labcraft_format::read(file.bytes, &mut names, limits).file).collect();
        let manifest = Manifest::parse(library.manifest).ok();
        let files: Vec<&File> = files.iter().collect();
        entries.extend(entries_of(library.name, &files, &mut names, manifest.as_ref()));
    }
    entries
}

/// The project's own, from the check of the root: each library of the folder and its sub-libraries; and the names of every
/// library that check read, dependencies included.
fn project_entries(session: &mut Session) -> Result<(Vec<Entry>, Vec<String>), CommandError> {
    let root = session.root.clone();
    let kept = session.checked(&root)?;
    let mut entries = Vec::new();
    let mut read = Vec::new();
    for (checked, names) in &kept.checked.checked {
        let mut names = names.clone();
        read.extend(checked.libraries.iter().map(|library| library.name.clone()));
        for (index, library) in checked.libraries.iter().enumerate() {
            if !library.checked {
                continue;
            }
            let files: Vec<&File> = checked.files.iter().filter(|file| file.library as usize == index).map(|file| &file.file).collect();
            entries.extend(entries_of(&library.name, &files, &mut names, None));
        }
    }
    Ok((entries, read))
}

/// `functions.list`.
pub fn list(session: &mut Session, params: &Params) -> Result<Value, CommandError> {
    let library = params.str("library");
    let filter = params.str("filter").unwrap_or_default();
    let docs = params.bool("docs");
    let limits = session.config.limits.clone();
    let primitives = PRIMITIVES.iter().map(|primitive| Entry {
        library: primitive.library.as_str().to_owned(),
        name: format!("{}.{}", primitive.library.as_str(), primitive.name),
        kind: "function",
        signature: primitive_signature(primitive),
        always: false,
        categories: categories_of(primitive).iter().map(|category| (*category).to_owned()).collect(),
        doc: Vec::new(),
        typed: primitive_typed(primitive),
    });
    let mut all: Vec<Entry> = primitives.collect();
    all.extend(shipped_entries(session, &limits));
    let (project, read) = project_entries(session)?;
    // A primitive added after M0 gives way to a library of its name, of the project or of a `--lib-path` (docs/cli.md,
    // section 3, rule 1).
    all.retain(|entry| {
        let gives_way = PrimitiveLibrary::from_name(&entry.library).is_some_and(PrimitiveLibrary::gives_way);
        !gives_way || !read.contains(&entry.library)
    });
    all.extend(project);
    follow_aliases(&mut all);
    let kept: Vec<Entry> =
        all.into_iter().filter(|entry| library.is_none_or(|library| entry.library == library) && entry.name.contains(filter)).collect();
    let truncated = kept.len() > MAX_ENTRIES;
    let entries: Vec<Value> = kept
        .into_iter()
        .take(MAX_ENTRIES)
        .map(|entry| {
            let doc = if docs { entry.doc.join("\n") } else { entry.doc.first().cloned().unwrap_or_default() };
            let mut written = json!({ "name": entry.name, "library": entry.library, "kind": entry.kind, "signature": entry.signature });
            if let Some(object) = written.as_object_mut() {
                if let Ok(Value::Object(typed)) = serde_json::to_value(&entry.typed) {
                    object.extend(typed);
                }
                if entry.always {
                    object.insert("always".to_owned(), json!(true));
                }
                if !entry.categories.is_empty() {
                    object.insert("categories".to_owned(), json!(entry.categories));
                }
                if !doc.is_empty() {
                    object.insert("doc".to_owned(), json!(doc));
                }
            }
            written
        })
        .collect();
    Ok(json!({ "entries": entries, "truncated": truncated }))
}
