//! `functions.list` (`docs/commands.md`, 6.1): what the project can call, each as a signature written in the language,
//! as appendix A of the spec writes it. The primitives come from the table of `check`, the shipped libraries from their
//! files, and the project's own entries from the last check of the root, so that they agree with it.

use labcraft_check::manifest::Manifest;
use labcraft_check::primitives::{PRIMITIVES, ParamKind, Primitive, PrimitiveDefault, PrimitiveOutput, PrimitiveParam, PrimitiveTy};
use labcraft_graph::{Attached, CommentKind, Decl, File, NamedType, Path as NamePath, TypeExpr};
use labcraft_types::{Interner, Limits};
use serde_json::{Value, json};

use crate::error::CommandError;
use crate::params::Params;
use crate::session::Session;

/// At most this many entries *(proposed)*, primitives and shipped libraries first.
pub const MAX_ENTRIES: usize = 1000;
/// A signature longer than this is cut, with `…`.
const MAX_SIGNATURE: usize = 2000;

struct Entry {
    library: String,
    name: String,
    kind: &'static str,
    signature: String,
    always: bool,
    categories: Vec<String>,
    doc: Vec<String>,
}

/// The categories of the errors that a primitive can make itself (spec, appendix A and 7.2).
fn categories_of(primitive: &Primitive) -> &'static [&'static str] {
    match (primitive.library.as_str(), primitive.name) {
        ("array", "get" | "set" | "slice") => &["array.index_out_of_range"],
        ("array", "repeat") => &["array.negative_size"],
        ("array", "from_rows") => &["array.unequal_rows"],
        ("string", "to_f64" | "to_i32") => &["string.parse"],
        ("core", "call") => &["core.invalid_function"],
        _ => &[],
    }
}

fn type_text(ty: PrimitiveTy, variables: &[&str]) -> String {
    let variable = |index: usize| variables.get(index).copied().unwrap_or("T").to_owned();
    match ty {
        PrimitiveTy::Bool => "bool".to_owned(),
        PrimitiveTy::I32 => "i32".to_owned(),
        PrimitiveTy::F32 => "f32".to_owned(),
        PrimitiveTy::F64 => "f64".to_owned(),
        PrimitiveTy::Text => "string".to_owned(),
        PrimitiveTy::Texts => "[string]".to_owned(),
        PrimitiveTy::Timestamp => "timestamp".to_owned(),
        PrimitiveTy::Duration => "duration".to_owned(),
        PrimitiveTy::Error => "error".to_owned(),
        PrimitiveTy::Var(index) => variable(index),
        PrimitiveTy::ArrayOf(index) => format!("[{}]", variable(index)),
        PrimitiveTy::Rows(index) => format!("[[{}]]", variable(index)),
        PrimitiveTy::Rectangle(index) => format!("[{}; 2]", variable(index)),
        PrimitiveTy::Function => "fn(…) -> (…)".to_owned(),
        PrimitiveTy::None => String::new(),
    }
}

fn param_text(param: &PrimitiveParam, variables: &[&str]) -> String {
    let ty = match param.kind {
        ParamKind::Value => type_text(param.ty, variables),
        ParamKind::Unit => "kind unit".to_owned(),
        ParamKind::Category => "kind category".to_owned(),
        ParamKind::Element => "kind element".to_owned(),
    };
    let default = match param.default {
        None => "",
        Some(PrimitiveDefault::NoError) => " = none",
        Some(PrimitiveDefault::Zero) => " = 0",
        Some(PrimitiveDefault::EmptyText) => " = \"\"",
        Some(PrimitiveDefault::ToTheEnd) => " = …",
    };
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
        let always = kind == "function" && manifest.is_some_and(|manifest| manifest.always.contains(&short));
        let categories = if kind == "function" { manifest.map(|manifest| manifest.categories.clone()).unwrap_or_default() } else { Vec::new() };
        entries.push(Entry { library: library.to_owned(), name: full, kind, signature, always, categories, doc });
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

/// The project's own, from the check of the root: each library of the folder and its sub-libraries.
fn project_entries(session: &mut Session) -> Result<Vec<Entry>, CommandError> {
    let root = session.root.clone();
    let kept = session.checked(&root)?;
    let mut entries = Vec::new();
    for (checked, names) in &kept.checked.checked {
        let mut names = names.clone();
        for (index, library) in checked.libraries.iter().enumerate() {
            if !library.checked {
                continue;
            }
            let files: Vec<&File> = checked.files.iter().filter(|file| file.library as usize == index).map(|file| &file.file).collect();
            entries.extend(entries_of(&library.name, &files, &mut names, None));
        }
    }
    Ok(entries)
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
    });
    let mut all: Vec<Entry> = primitives.collect();
    all.extend(shipped_entries(session, &limits));
    all.extend(project_entries(session)?);
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
