//! What changes a whole project, or makes a file (commands.md 6.7, "Functions, signatures and types"): `function.rename`
//! and `type.rename`, the declaration and every use of it in the project in one step (decision of the owner,
//! 2026-11-15), and the text of `type.create`. The file of what is renamed is renamed too (R-22): `engine` moves it, from
//! the text that this module gives under the old path.
//!
//! A use is found as the check would find it: a path whose name is that of the function or the type, seen from its file
//! through [`Judge::function_file`] and [`Judge::type_file`], and not a value of the function that holds it (a value
//! hides a function of its name, spec 4).

use std::collections::HashSet;

use labcraft_format::{format, is_name, read, write};
use labcraft_graph::{Decl, File, Path, PathAt, Signature};
use labcraft_types::{Interner, Limits, Name};

use crate::{EditError, Edited, Files, Judge, cut};

/// The kind of what is renamed.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Renamed {
    Function,
    Type,
}

impl Renamed {
    fn word(self) -> &'static str {
        match self {
            Renamed::Function => "function",
            Renamed::Type => "type",
        }
    }
}

/// `function.rename` (commands.md 6.7): the function of `path` becomes `new`, and each call and each reference to it in
/// the files of `files` names it so. Its own text is given under `path`, which `engine` writes under its new name.
pub fn rename_function(files: &Files, path: &str, new: &str, judge: &dyn Judge, limits: &Limits) -> Result<Edited, EditError> {
    rename(files, path, new, Renamed::Function, judge, limits)
}

/// `type.rename` (commands.md 6.7): the type of `path` becomes `new`, and each use of it in the files of `files` names it
/// so, in the other types too.
pub fn rename_type(files: &Files, path: &str, new: &str, judge: &dyn Judge, limits: &Limits) -> Result<Edited, EditError> {
    rename(files, path, new, Renamed::Type, judge, limits)
}

fn rename(files: &Files, path: &str, new: &str, kind: Renamed, judge: &dyn Judge, limits: &Limits) -> Result<Edited, EditError> {
    let Some(text) = files.get(path) else {
        return Err(EditError::new("path.not_found", format!("`{path}` is not a file of the project")).of("file"));
    };
    if !is_name(new) {
        return Err(EditError::new(
            "name.invalid",
            format!("`{}` is not a name for a {}: a letter, then letters, digits and `_`, and no reserved word (R-22)", cut(new), kind.word()),
        )
        .of("name"));
    }
    let mut own = Parsed::read(path, text, limits)?;
    let old = match (&mut own.file.decl, kind) {
        (Decl::Function(function), Renamed::Function) => std::mem::replace(&mut function.name, own.names.intern(new)),
        (Decl::Type(declared), Renamed::Type) => std::mem::replace(&mut declared.name, own.names.intern(new)),
        (Decl::Extern(_), Renamed::Function) => {
            return Err(EditError::invalid("file", format!("`{path}` declares an `extern` function: its native library names it")));
        }
        _ => return Err(EditError::invalid("file", format!("`{path}` holds no {}", kind.word()))),
    };
    let old_text = own.names.resolve(old).to_owned();
    let mut edited = Edited::default();
    edited.names.insert(kind.word(), new.to_owned());
    if old_text == new {
        return Ok(edited);
    }
    // A function that calls itself, a type that holds itself in an array.
    own.uses(&old_text, new, kind, path, judge)?;
    edited.texts.insert(path.to_owned(), own.render(limits)?);
    // A use writes the name: only a file that holds it can use it, and only such a file that does not parse stops the
    // rename (commands.md 6.7).
    for (other, other_text) in files.iter() {
        let readable = other.ends_with(".lcf") || (kind == Renamed::Type && other.ends_with(".lct"));
        if other == path || !readable || !other_text.contains(old_text.as_str()) {
            continue;
        }
        let mut parsed = Parsed::read(other, other_text, limits)?;
        if parsed.uses(&old_text, new, kind, path, judge)? {
            let written = parsed.render(limits)?;
            if written != other_text {
                edited.texts.insert(other.to_owned(), written);
            }
        }
    }
    Ok(edited)
}

/// A file read as it is, for the paths it holds.
struct Parsed {
    path: String,
    names: Interner,
    file: File,
}

impl Parsed {
    fn read(path: &str, text: &str, limits: &Limits) -> Result<Parsed, EditError> {
        let mut names = Interner::new();
        let reading = read(text.as_bytes(), &mut names, limits);
        let Some(file) = reading.file else {
            let why = reading.report.diagnostics().first().map(|diagnostic| diagnostic.message.clone()).unwrap_or_default();
            return Err(EditError::new("file.unparsed", format!("`{path}` does not parse, so its uses cannot be renamed: {why}")).of("file"));
        };
        Ok(Parsed { path: path.to_owned(), names, file })
    }

    /// Renames the paths that name what `target` declares as `old`: their last segment for a function, the segment of the
    /// type for a type (`State` of `State.Init`, of `a.b.State.Init`). Whether it changed anything. A file where the new
    /// name is already a value, a generic variable or a library it uses would read the renamed paths as those:
    /// `name.taken`, and nothing is written (decision of the owner, 2026-11-19).
    fn uses(&mut self, old: &str, new: &str, kind: Renamed, target: &str, judge: &dyn Judge) -> Result<bool, EditError> {
        let Some(old_name) = self.names.find(old) else { return Ok(false) };
        let new_name = self.names.intern(new);
        let values = values_of(&self.file);
        let generics = generics_of(&self.file);
        let mut taken: HashSet<Name> = values.iter().chain(&generics).copied().collect();
        taken.extend(self.file.uses.iter().map(|line| line.path.segments().last().unwrap_or(line.path.first)));
        let (names, path) = (&self.names, self.path.as_str());
        let text_of = |segments: &[Name]| segments.iter().map(|segment| names.resolve(*segment)).collect::<Vec<_>>().join(".");
        let mut changed = false;
        self.file.paths_mut(&mut |at, found: &mut Path| {
            let segments: Vec<Name> = found.segments().collect();
            if !segments.contains(&old_name) {
                return;
            }
            let place = match kind {
                Renamed::Function => function_use(at, &segments, &values, old_name, path, target, judge, &text_of),
                Renamed::Type => type_use(at, &segments, &values, &generics, old_name, path, target, judge, &text_of),
            };
            if let Some(place) = place {
                set_segment(found, place, new_name);
                changed = true;
            }
        });
        if changed && taken.contains(&new_name) {
            return Err(EditError::new(
                "name.taken",
                format!("`{}` names a value, a generic variable or a library in `{}`, which would read the renamed uses as it", cut(new), self.path),
            )
            .of("name"));
        }
        Ok(changed)
    }

    fn render(&self, limits: &Limits) -> Result<String, EditError> {
        let written = write(&self.file, &self.names, limits)
            .map_err(|_| EditError::new("file.too_large", format!("`{}` would be over the limit of {} bytes", self.path, limits.max_file_size)))?;
        let mut names = Interner::new();
        let formatted = format(written.as_bytes(), &mut names, limits);
        formatted.text.ok_or_else(|| {
            let why = formatted.report.diagnostics().first().map(|diagnostic| diagnostic.message.clone()).unwrap_or_default();
            EditError::internal(format!("the renamed text of `{}` does not read again: {why}", self.path))
        })
    }
}

/// The place of the segment of a path that names the function of `target` as `old`, if it does: `f(…)`, `lib.f(…)`, and
/// a reference `measure: f` that no value of the function hides.
#[allow(clippy::too_many_arguments)]
fn function_use(
    at: PathAt,
    segments: &[Name],
    values: &HashSet<Name>,
    old: Name,
    file: &str,
    target: &str,
    judge: &dyn Judge,
    text_of: &dyn Fn(&[Name]) -> String,
) -> Option<usize> {
    let last = segments.len().checked_sub(1)?;
    // `f`, `lib.f`, `a.b.f` of a sub-library (spec 4); an operand whose first name is a value reads that value.
    let ok = match at {
        PathAt::Callee => true,
        PathAt::Operand => !segments.first().is_some_and(|first| values.contains(first)),
        _ => false,
    };
    (ok && segments.get(last) == Some(&old) && judge.function_file(file, &text_of(segments)).as_deref() == Some(target)).then_some(last)
}

/// The place of the segment of a path that names the type of `target` as `old`, if it does: a type written (`Step`,
/// `lib.Step`), a construction (`Step { … }`), a variant (`State.Init`, `lib.State.Init`, `Reading.Value(v)`), unless a
/// generic variable or a value hides it.
#[allow(clippy::too_many_arguments)]
fn type_use(
    at: PathAt,
    segments: &[Name],
    values: &HashSet<Name>,
    generics: &HashSet<Name>,
    old: Name,
    file: &str,
    target: &str,
    judge: &dyn Judge,
    text_of: &dyn Fn(&[Name]) -> String,
) -> Option<usize> {
    let first = segments.first()?;
    let names_type = |count: usize| -> Option<usize> {
        let ty = segments.get(..count)?;
        let place = count.checked_sub(1)?;
        (ty.get(place) == Some(&old) && judge.type_file(file, &text_of(ty)).as_deref() == Some(target)).then_some(place)
    };
    match at {
        PathAt::Type if !generics.contains(first) => names_type(segments.len()),
        PathAt::Struct => names_type(segments.len()),
        // A struct built by a call (`Step(…)`), when no function bears the name, or a variant that holds values.
        PathAt::Callee => {
            if judge.function_file(file, &text_of(segments)).is_none()
                && let Some(place) = names_type(segments.len())
            {
                return Some(place);
            }
            names_type(segments.len().checked_sub(1)?)
        }
        PathAt::Variant => names_type(segments.len().checked_sub(1)?),
        PathAt::Operand if !values.contains(first) => names_type(segments.len().checked_sub(1)?),
        _ => None,
    }
}

fn set_segment(path: &mut Path, place: usize, name: Name) {
    match place.checked_sub(1) {
        None => path.first = name,
        Some(rest) => {
            if let Some(segment) = path.rest.get_mut(rest) {
                *segment = name;
            }
        }
    }
}

/// The names of the values of the function of a file: its inputs, its outputs, and what its lines, clauses and patterns
/// bind, in any body. A path whose first segment is one of them reads a value, not a function or a type.
fn values_of(file: &File) -> HashSet<Name> {
    let Decl::Function(function) = &file.decl else { return HashSet::new() };
    let mut found: HashSet<Name> = crate::scope::values(function).into_iter().map(|value| value.name).collect();
    found.extend(function.signature.outputs.iter().map(|output| output.name));
    found
}

/// The generic variables of a file: a type of their name is the variable.
fn generics_of(file: &File) -> HashSet<Name> {
    let of = |signature: &Signature| signature.generics.iter().map(|generic| generic.name).collect::<Vec<_>>();
    match &file.decl {
        Decl::Function(function) => of(&function.signature).into_iter().collect(),
        Decl::Extern(function) => of(&function.signature).into_iter().collect(),
        Decl::Type(declared) => declared.generics.iter().map(|generic| generic.name).collect(),
    }
}

/// The text of a new type `name` (commands.md 6.7, `type.create`): `type name = definition`, with its documentation, in
/// canonical form. A name that is not one of the language is `name.invalid` of `path`, since the file names it (R-22); a
/// definition that does not read, or that is not a type, is `params.invalid` of `definition`, with why.
pub fn new_type(name: &str, definition: &str, doc: Option<&str>, limits: &Limits) -> Result<String, EditError> {
    if !is_name(name) {
        return Err(EditError::new(
            "name.invalid",
            format!("`{}` is not a name for a type: a letter, then letters, digits and `_`, and no reserved word (R-22)", cut(name)),
        )
        .of("path"));
    }
    let definition = definition.trim();
    if definition.is_empty() {
        return Err(EditError::invalid("definition", "a type needs its definition: a type of the language, a `struct { … }` or an `enum { … }`"));
    }
    let doc = crate::signature::doc_lines(doc.unwrap_or_default());
    if doc.iter().any(|line| line.text.chars().any(|c| c.is_control() && c != '\t')) {
        return Err(EditError::invalid("doc", "a documentation holds no control character but the tab and the line ending"));
    }
    let mut text = String::from("labcraft 1\n\n");
    for line in &doc {
        text.push_str("///");
        text.push_str(&line.text);
        text.push('\n');
    }
    text.push_str(&format!("type {name} = {definition}\n"));
    let mut names = Interner::new();
    let reading = read(text.as_bytes(), &mut names, limits);
    let why = |report: &labcraft_types::Report| report.diagnostics().first().map(|diagnostic| diagnostic.message.clone()).unwrap_or_default();
    // One declaration, of a type: a definition that closes the declaration early and starts another is refused.
    // Nothing after it either: a `layout` or a `panel` section would be the business of `check` (`LC0104`).
    let one_type = reading.file.as_ref().is_some_and(|file| {
        matches!(&file.decl, Decl::Type(declared) if names.resolve(declared.name) == name)
            && file.panel.is_none()
            && file.layout.is_none()
            && file.icon.is_none()
    });
    if reading.file.is_none() || !reading.report.diagnostics().is_empty() || !one_type {
        let message = match reading.file {
            None => format!("`{}` does not read as a type: {}", cut(definition), why(&reading.report)),
            Some(_) => format!("`{}` is not one type definition: {}", cut(definition), why(&reading.report)),
        };
        return Err(EditError::invalid("definition", message));
    }
    let formatted = format(text.as_bytes(), &mut Interner::new(), limits);
    formatted.text.ok_or_else(|| EditError::invalid("definition", format!("the type would not read: {}", why(&formatted.report))))
}
