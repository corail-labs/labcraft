//! The `use` that a command adds (commands.md 9.2, spec R-1): a value of the function that bears the name of the library
//! it imports is renamed (N-14), with its readers and its entries of `layout`, in the same step, and `renamed` says so; an
//! input or an output of that name, or a name that the caller gave, is `name.taken`, and nothing is written (decisions of
//! the owner, 2026-11-23).

use std::collections::BTreeSet;

use labcraft_types::Limits;

use crate::model::Edit;
use crate::names::{fresh, rename_everywhere, taken};
use crate::scope::{self, Bound};
use crate::{EditError, Edited, Renamed};

/// The libraries that the `use` lines of the file import, by the name that a value would hide: `a` for `use a.b`.
pub(crate) fn imported(edit: &Edit) -> BTreeSet<String> {
    edit.file.uses.iter().map(|used| edit.text(used.path.first).to_owned()).collect()
}

/// After a command: the values named after a library that it imported, and that `before` did not, renamed. `given` is
/// the name that the caller gave to what the command makes, if any.
pub(crate) fn follow(edit: &mut Edit, before: &BTreeSet<String>, given: Option<&str>, edited: &mut Edited) -> Result<(), EditError> {
    let new: Vec<String> = imported(edit).into_iter().filter(|library| !before.contains(library)).collect();
    rename_values(edit, &new, given, &BTreeSet::new(), edited)
}

/// `follow` on a text that a command has made, read again: what `node.add` with `into` gives, when it imported a library.
pub(crate) fn follow_text(path: &str, text: &str, before: &BTreeSet<String>, limits: &Limits, edited: &mut Edited) -> Result<String, EditError> {
    let mut edit = Edit::read(path, text, limits)?;
    follow(&mut edit, before, None, edited)?;
    edit.finish()
}

/// Renames the values of the function named after one of `libraries`, each to a name free in the function and outside
/// `also_taken`: the names that a paste is about to bring.
pub(crate) fn rename_values(
    edit: &mut Edit,
    libraries: &[String],
    given: Option<&str>,
    also_taken: &BTreeSet<String>,
    edited: &mut Edited,
) -> Result<(), EditError> {
    for library in libraries {
        // A file of a type has no values.
        let Some(function) = edit.function() else { return Ok(()) };
        let terminal = if function.signature.params.iter().any(|param| edit.text(param.name) == library) {
            Some("an input")
        } else if function.signature.outputs.iter().any(|output| edit.text(output.name) == library) {
            Some("an output")
        } else {
            None
        };
        if let Some(terminal) = terminal {
            return Err(EditError::new(
                "name.taken",
                format!(
                    "`{library}` is {terminal} of the function, and a value cannot bear the name of the library `{library}` that this \
                     command imports (R-1): rename it first (`signature.rename`)"
                ),
            ));
        }
        if !scope::values(function).iter().any(|value| value.bound != Bound::Input && edit.text(value.name) == library) {
            continue;
        }
        if given == Some(library.as_str()) {
            return Err(EditError::new(
                "name.taken",
                format!("`{library}` is the name of the library that this command imports, which a value cannot bear (R-1)"),
            )
            .of("name"));
        }
        let mut free = taken(edit);
        free.extend(also_taken.iter().cloned());
        // One name for all of them: the values that hid one another (R-3) still do.
        let new = fresh(library, &free);
        let from = edit.intern(library);
        let to = edit.intern(&new);
        rename_everywhere(edit, from, to)?;
        // What the command named after the library, before it imported it, bears the new name too.
        for name in edited.names.values_mut().chain(edited.lists.values_mut().flatten()).filter(|name| *name == library) {
            name.clone_from(&new);
        }
        for renamed in edited.renamed.iter_mut().filter(|renamed| renamed.to == *library) {
            renamed.to.clone_from(&new);
        }
        edited.renamed.push(Renamed { file: edit.path.clone(), from: library.clone(), to: new });
    }
    Ok(())
}
