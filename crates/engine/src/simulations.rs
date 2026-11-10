//! Simulation files (`docs/commands.md`, 6.3): what one defines, part by part, as each library that simulates describes
//! its part (`libs/visa/simulation.md`, section 9), and the simulation files of a project, with their resources.

use std::path::{Path, PathBuf};

use labcraft_runtime::native::{Described, LibraryDiagnostic, NativeLibrary, Simulation};
use serde_json::{Map, Value, json};

use crate::error::CommandError;
use crate::files::{bytes_of, not_simulation, reads_as_simulation, text_of};
use crate::params::Params;
use crate::paths::{self, Place, Wanted};
use crate::session::Session;
use crate::workspace::Kind;

/// What a library judged of its part of a file: its simulation, or its diagnostics.
type Part<'a> = (&'a dyn NativeLibrary, Result<Simulation, Vec<LibraryDiagnostic>>);

/// Each library that simulates and has a part in the file, in the order the app gave them, with what it judged. The text
/// reads as a simulation file.
fn parts<'s>(session: &'s Session, text: &str) -> Vec<Part<'s>> {
    let Ok(file) = labcraft_runtime::simulation::read(text, session.config.limits.max_nesting()) else { return Vec::new() };
    session
        .config
        .natives
        .iter()
        .filter(|library| library.simulates())
        .filter_map(|library| file.section(library.name()).map(|section| (&**library, library.simulate(&section))))
        .collect()
}

pub(crate) fn described(found: &Described) -> Value {
    match found {
        Described::Null => Value::Null,
        Described::Bool(value) => json!(value),
        Described::Integer(value) => json!(value),
        Described::Number(value) => serde_json::Number::from_f64(*value).map_or_else(|| json!(value.to_string()), Value::Number),
        Described::Text(text) => json!(text),
        Described::List(items) => Value::Array(items.iter().map(described).collect()),
        Described::Map(fields) => Value::Object(fields.iter().map(|(key, value)| (key.clone(), described(value))).collect::<Map<String, Value>>()),
    }
}

/// A diagnostic of a library, as `labcraft-check/1` writes one (`docs/cli.md`, 4.2).
fn diagnostic(file: &str, found: &LibraryDiagnostic) -> Value {
    let mut written = json!({
        "code": found.code,
        "severity": match found.severity { labcraft_types::Severity::Error => "error", labcraft_types::Severity::Warning => "warning" },
        "message": found.message,
        "file": file,
    });
    if let Some(object) = written.as_object_mut() {
        if let Some(span) = found.span {
            object.insert(
                "span".to_owned(),
                json!({ "start": { "line": span.start.line, "column": span.start.column }, "end": { "line": span.end.line, "column": span.end.column } }),
            );
        }
        if let Some(fix) = &found.fix {
            object.insert("fix".to_owned(), json!(fix));
        }
    }
    written
}

/// `simulation.describe`.
pub fn describe(session: &mut Session, params: &Params) -> Result<Value, CommandError> {
    let place = paths::judge(&*session.ws, &session.root, params.text("path")?, "path", Wanted::Simulation, true)?;
    let (bytes, shown) = bytes_of(session, &place, "path")?;
    let text = text_of(bytes, "path", &shown)?;
    if let Err(why) = reads_as_simulation(session, &text) {
        return Err(not_simulation("path", &shown, "does not read as a simulation file", &why));
    }
    let mut described_parts = Vec::new();
    let mut diagnostics = Vec::new();
    for (library, judged) in parts(session, &text) {
        match judged {
            Ok(simulation) => {
                let description = library.describe(&simulation).map_or(Value::Null, |found| described(&found));
                described_parts.push(json!({ "library": library.name(), "description": description }));
            }
            Err(found) => diagnostics.extend(found.iter().map(|found| diagnostic(&shown, found))),
        }
    }
    if diagnostics.is_empty() {
        Ok(json!({ "file": shown, "parts": described_parts }))
    } else {
        Ok(json!({ "file": shown, "diagnostics": diagnostics }))
    }
}

/// A simulation file, and the resources it defines; none when a library refused its part.
pub(crate) struct Listed {
    pub shown: String,
    pub resources: Vec<String>,
    pub valid: bool,
}

/// Bounds of the search for simulation files: those of a walk (`docs/cli.md`, section 1).
const LEVELS: usize = 32;
const FOLDERS: usize = 100_000;

/// At most this many bytes of `.toml` files read for one `project.describe` *(proposed)*: the files that may be
/// simulation files are read to know, and a project can hold many large ones that are not. Simulation files are judged
/// at about 6 MB a second, so this keeps the command within about 2 s (scenario 4).
const MAX_READ_BYTES: u64 = 8 << 20;

/// The simulation files of the project, in the order of their paths: every `.toml` file that reads as one, with no hidden
/// folder and no link followed, `labcraft.toml` aside. At most `max` of them, within the bounds of a walk and of
/// [`MAX_READ_BYTES`], then `true` for those left out.
pub(crate) fn of_project(session: &Session, max: usize) -> (Vec<Listed>, bool) {
    let mut found: Vec<PathBuf> = Vec::new();
    let mut folders = vec![(session.root.clone(), 1usize)];
    let mut seen = 0usize;
    let mut more = false;
    while let Some((folder, level)) = folders.pop() {
        seen += 1;
        if seen > FOLDERS || found.len() > max.saturating_mul(4).max(1000) {
            more = true;
            break;
        }
        let mut subs = Vec::new();
        let _ = session.ws.read_dir(&folder, &mut |entry| {
            let Ok(entry) = entry else { return };
            let name = entry.name.to_string_lossy();
            if name.starts_with('.') {
                return;
            }
            match entry.kind {
                Ok(Kind::Folder) if level < LEVELS => subs.push(folder.join(&entry.name)),
                Ok(Kind::Folder) => more = true,
                Ok(Kind::File)
                    if Path::new(&entry.name).extension().is_some_and(|extension| extension == "toml")
                        && !name.eq_ignore_ascii_case("labcraft.toml") =>
                {
                    found.push(folder.join(&entry.name));
                }
                _ => {}
            }
        });
        subs.sort();
        folders.extend(subs.into_iter().rev().map(|sub| (sub, level + 1)));
    }
    found.sort();
    let mut listed = Vec::new();
    let mut read: u64 = 0;
    for path in found {
        // A file that `--max-file-size` refuses is not read, and costs nothing.
        let size = session.ws.metadata(&path).map_or(0, |metadata| metadata.len);
        if size > session.config.limits.max_file_size {
            continue;
        }
        if read.saturating_add(size) > MAX_READ_BYTES {
            more = true;
            break;
        }
        read += size;
        let shown =
            path.strip_prefix(&session.root).map_or_else(|_| path.to_string_lossy().into_owned(), |rest| rest.to_string_lossy().replace('\\', "/"));
        let place = Place::Project { path, shown: shown.clone(), exists: true };
        let Ok((bytes, _)) = bytes_of(session, &place, "path") else { continue };
        let Ok(text) = String::from_utf8(bytes.into_owned()) else { continue };
        if reads_as_simulation(session, &text).is_err() {
            continue;
        }
        if listed.len() >= max {
            more = true;
            break;
        }
        listed.push(of_text(session, shown, &text));
    }
    (listed, more)
}

/// The simulation files shipped with each library, `<labcraft>/visa/sim/bench.toml`.
pub(crate) fn shipped(session: &Session) -> Vec<Listed> {
    let mut listed = Vec::new();
    for library in session.config.shipped.libraries() {
        for file in &library.simulations {
            let shown = format!("{}{}/{}", crate::shipped::PREFIX, library.name, file.name);
            if let Ok(text) = std::str::from_utf8(file.bytes) {
                listed.push(of_text(session, shown, text));
            }
        }
    }
    listed
}

fn of_text(session: &Session, shown: String, text: &str) -> Listed {
    let mut resources = Vec::new();
    let mut valid = true;
    for (library, judged) in parts(session, text) {
        match judged {
            Ok(simulation) => resources.extend(library.resources(&simulation)),
            Err(_) => valid = false,
        }
    }
    Listed { shown, resources, valid }
}
