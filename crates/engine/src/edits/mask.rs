//! `library.set_mask` (commands.md 6.7): writes fields of the table `[mask]` of the manifest of a library (spec 11.3),
//! the rest of the manifest, its comments and its order kept as written; makes the manifest, holding the table alone,
//! when the library has none. One step of history.

use std::sync::Arc;

use labcraft_graph::{BandPlace, Rgb};
use labcraft_types::Interner;
use serde_json::Map;
use toml_edit::{DocumentMut, Item, Table, Value};

use super::{bytes_now, judge_revisions_of, record, result, write_changes};
use crate::error::CommandError;
use crate::history::{Author, Change};
use crate::params::Params;
use crate::paths::{self, Wanted};
use crate::registry::Command;
use crate::session::Session;

/// The fields of the mask, in the order the table writes them when it makes them.
const FIELDS: [&str; 6] = ["band", "band_place", "color", "fill", "stroke", "outline"];

/// What the command does to one field: `None` leaves it, `Some(None)` removes it, `Some(Some(text))` writes it.
type Field = Option<Option<String>>;

pub(crate) fn set_mask(session: &mut Session, author: &Author, command: &Command, params: &Params) -> Result<serde_json::Value, CommandError> {
    let fields: Vec<(&str, Field)> = FIELDS.iter().map(|&key| Ok((key, field(params, key)?))).collect::<Result<_, CommandError>>()?;
    judge_revisions_of(session, params, true)?;
    let (_, folder_shown) = paths::existing_in_project(&*session.ws, &session.root, params.text("folder")?, "folder", Wanted::Folder)?;
    // The manifest itself goes through the rules of paths: a link that leads out of the project is refused (commands.md 1).
    let given = labcraft_check::manifest::shown(&folder_shown);
    let (path, shown, _) = paths::in_project(&*session.ws, &session.root, &given, "folder", Wanted::Manifest)?;
    let before_bytes = bytes_now(session, &path)?;
    if before_bytes.is_none() {
        crate::files::refuse_case_twin(session, &path, &shown, "folder", None)?;
    }
    let before_text = match &before_bytes {
        None => None,
        Some(bytes) => Some(
            std::str::from_utf8(bytes)
                .map_err(|_| CommandError::of("file.unparsed", "folder", format!("`{shown}` is not UTF-8: its mask cannot be written")))?,
        ),
    };
    // A byte order mark is kept as written.
    let bom = before_text.is_some_and(|text| text.starts_with('\u{feff}'));
    let after = written(before_text.map(|text| text.trim_start_matches('\u{feff}')), &fields)
        .map(|after| if bom && !after.trim().is_empty() { format!("\u{feff}{after}") } else { after })
        .map_err(|why| CommandError::of("file.unparsed", "folder", format!("`{shown}` {why}")))?;
    if after.len() as u64 > session.config.limits.max_file_size {
        return Err(CommandError::of("file.too_large", "folder", format!("`{shown}` would be over the limit of `--max-file-size`")));
    }
    let before = session.root_check_document()?;
    // Nothing to write: a manifest unchanged, or none made for a table left empty. A manifest with nothing left goes.
    let empty = after.trim().is_empty();
    let unchanged = before_text.map_or(empty, |text| text == after);
    let changes = if unchanged {
        Vec::new()
    } else {
        let after = (!empty).then(|| Arc::from(after.into_bytes()));
        vec![Change { path, shown, before: before_bytes.map(Arc::from), after }]
    };
    write_changes(session, &changes, false)?;
    let step = record(session, author, command.label.to_owned(), changes.clone());
    result(session, step, &changes, false, &before, Map::new())
}

/// One field of the call, judged as the mask judges it (spec 11.3).
fn field(params: &Params, key: &str) -> Result<Field, CommandError> {
    let text = match params.get(key) {
        None if params.is_null(key) => return Ok(Some(None)),
        None => return Ok(None),
        Some(serde_json::Value::String(text)) => text.as_str(),
        Some(_) => return Err(CommandError::of("params.invalid", key, format!("`{key}` is a text, or `null`"))),
    };
    let refused = |why: String| CommandError::of("params.invalid", key, why);
    let shown = labcraft_edit::cut(text);
    let kept = match key {
        "band" => {
            if let Some(why) = labcraft_check::mask::band_refused(text) {
                return Err(refused(why));
            }
            text.to_owned()
        }
        "band_place" => {
            BandPlace::from_word(text).ok_or_else(|| refused(format!("`{shown}` is no place of a band: `inside`, `above` or `hidden`")))?;
            text.to_owned()
        }
        "outline" => outline(text).map_err(refused)?,
        _ => {
            Rgb::parse(text).ok_or_else(|| refused(format!("`{shown}` is no colour: `#RRGGBB`")))?;
            text.to_owned()
        }
    };
    Ok(Some(Some(kept)))
}

/// The items of an outline as the mask writes them, one per line, if `block.set_outline` would take them.
fn outline(items: &str) -> Result<String, String> {
    let lines: Vec<&str> = items.lines().map(str::trim_end).filter(|line| !line.trim().is_empty()).collect();
    let items = lines.join("\n");
    if items.contains("'''") || literal(&items).is_none() {
        return Err("the items hold `'''` or a character that a literal TOML string cannot".to_owned());
    }
    labcraft_check::outline::read_items(&items, &mut Interner::default())?;
    Ok(items)
}

/// The items as a multi-line literal TOML string, when one can hold them.
fn literal(items: &str) -> Option<Value> {
    let document: DocumentMut = format!("outline = '''\n{items}\n'''\n").parse().ok()?;
    document.get("outline").and_then(Item::as_value).cloned()
}

/// The manifest with these fields written; `Err` says why its mask cannot be, after its name.
fn written(manifest: Option<&str>, fields: &[(&str, Field)]) -> Result<String, String> {
    let mut document: DocumentMut = manifest.unwrap_or_default().parse().map_err(|_| "is not TOML: its mask cannot be written".to_owned())?;
    let had = document.contains_key("mask");
    if !had && fields.iter().all(|(_, field)| !matches!(field, Some(Some(_)))) {
        return Ok(manifest.unwrap_or_default().to_owned());
    }
    let table = document.entry("mask").or_insert_with(|| Item::Table(Table::new()));
    let Some(table) = table.as_table_like_mut() else {
        return Err("has a `mask` that is not a table: write it as a table first".to_owned());
    };
    for (key, field) in fields {
        match field {
            None => {}
            Some(None) => {
                table.remove(key);
            }
            Some(Some(text)) => {
                let mut value = if *key == "outline" {
                    // `outline` judged it already: the literal string holds it.
                    literal(text).ok_or_else(|| "cannot hold this outline".to_owned())?
                } else {
                    Value::from(text.as_str())
                };
                // A field written again keeps its comments.
                if let Some(old) = table.get(key).and_then(Item::as_value) {
                    *value.decor_mut() = old.decor().clone();
                }
                table.insert(key, Item::Value(value));
            }
        }
    }
    if table.is_empty() {
        document.remove("mask");
    }
    Ok(document.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(key: &'static str, text: &str) -> (&'static str, Field) {
        (key, Some(Some(text.to_owned())))
    }

    /// Worked out by hand from commands.md 6.7: the rest of the manifest, its comments and its order are kept; a field
    /// written again keeps its place; a new one goes at the end of the table.
    #[test]
    fn a_field_is_written_and_the_rest_kept() {
        let manifest = "# the driver\nname = \"dmm\"\n\n[mask]\nband = \"D\" # short\ncolor = \"#2F6F9F\"\n";
        let after = written(Some(manifest), &[set("band", "DMM"), set("stroke", "#1E4F73")]).unwrap();
        assert_eq!(after, "# the driver\nname = \"dmm\"\n\n[mask]\nband = \"DMM\" # short\ncolor = \"#2F6F9F\"\nstroke = \"#1E4F73\"\n");
    }

    #[test]
    fn a_manifest_is_made_with_the_table_alone_and_the_table_goes_when_empty() {
        assert_eq!(written(None, &[set("color", "#2F6F9F")]).unwrap(), "[mask]\ncolor = \"#2F6F9F\"\n");
        assert_eq!(written(None, &[("color", Some(None))]).unwrap(), "");
        let manifest = "name = \"dmm\"\n\n[mask]\ncolor = \"#2F6F9F\"\n";
        // The blank line before `[mask]` belongs to its header, and goes with it.
        assert_eq!(written(Some(manifest), &[("color", Some(None))]).unwrap(), "name = \"dmm\"\n");
    }

    #[test]
    fn an_outline_is_a_literal_string_of_its_items() {
        let after = written(None, &[set("outline", &outline("template d  \n\n").unwrap())]).unwrap();
        assert_eq!(after, "[mask]\noutline = '''\ntemplate d\n'''\n");
        assert!(outline("template d\ntemplate d").is_err(), "a second template is an error of 11.4");
        assert!(outline("path (0, 0) '''").is_err());
    }

    #[test]
    fn a_manifest_that_is_not_toml_or_a_mask_that_is_no_table_is_refused() {
        assert!(written(Some("name = "), &[set("band", "A")]).is_err());
        assert!(written(Some("mask = 1\n"), &[set("band", "A")]).is_err());
    }
}
