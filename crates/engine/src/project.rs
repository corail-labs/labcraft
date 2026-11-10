//! `project.describe` (`docs/commands.md`, 6.2): the libraries of the root, their files with what each declares, the
//! simulation files with their resources, and the summary of a check of the root.

use std::collections::{BTreeMap, HashMap};

use labcraft_graph::Decl;
use serde_json::{Value, json};

use crate::error::CommandError;
use crate::session::Session;
use crate::simulations;
use crate::walk;

/// At most this many files listed *(proposed)*, programs and simulation files together.
pub const MAX_FILES: usize = 1000;

/// `project.describe`.
pub fn describe(session: &mut Session) -> Result<Value, CommandError> {
    let pending = session.first_check_pending();
    let walked = walk::walk(&*session.ws, std::slice::from_ref(&session.root), Some(&session.root), &session.config.limits);
    let mut paths: Vec<(String, bool)> = walked
        .files
        .iter()
        .map(|found| (found.shown.clone(), true))
        .chain(walked.refused.iter().map(|refused| (refused.shown.clone(), false)))
        .collect();
    paths.sort();
    let mut truncated = paths.len() > MAX_FILES;
    paths.truncate(MAX_FILES);
    let (summary, declared) = if pending {
        (json!({ "pending": true }), HashMap::new())
    } else {
        let root = session.root.clone();
        let kept = session.checked(&root)?;
        let document = serde_json::to_value(kept.checked.report.check_document(kept.checked.counts, false))
            .map_err(|error| CommandError::internal(format!("the report does not write as JSON: {error}")))?;
        let mut declared: HashMap<String, Value> = HashMap::new();
        for (checked, names) in &kept.checked.checked {
            for file in &checked.files {
                let what = match &file.file.decl {
                    Decl::Function(function) => json!({ "function": names.resolve(function.name) }),
                    Decl::Extern(function) => json!({ "function": names.resolve(function.name), "extern": true }),
                    Decl::Type(ty) => json!({ "type": names.resolve(ty.name) }),
                };
                declared.insert(file.shown.clone(), what);
            }
        }
        (document.get("summary").cloned().unwrap_or_default(), declared)
    };
    let mut libraries: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for (shown, read) in &paths {
        let folder = shown.rsplit_once('/').map_or("", |(folder, _)| folder).to_owned();
        let mut file = json!({ "file": shown });
        if let Some(object) = file.as_object_mut() {
            match declared.get(shown) {
                Some(what) => {
                    object.insert("declares".to_owned(), what.clone());
                }
                None if !read => {
                    object.insert("unread".to_owned(), json!(true));
                }
                None => {}
            }
        }
        libraries.entry(folder).or_default().push(file);
    }
    let libraries: Vec<Value> = libraries
        .into_iter()
        .map(|(folder, files)| json!({ "library": folder.replace('/', "."), "folder": if folder.is_empty() { ".".to_owned() } else { folder }, "files": files }))
        .collect();
    let (of_project, more) = simulations::of_project(session, MAX_FILES.saturating_sub(paths.len()));
    truncated |= more;
    let simulation = |listed: simulations::Listed| json!({ "file": listed.shown, "resources": listed.resources, "valid": listed.valid });
    let simulations: Vec<Value> = of_project.into_iter().chain(simulations::shipped(session)).map(simulation).collect();
    let name = session.root.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(json!({ "root": name, "libraries": libraries, "simulations": simulations, "summary": summary, "truncated": truncated }))
}
