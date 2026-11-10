//! `run.trace` (`docs/commands.md`, 6.4): the trace a run wrote, read in pages through the workspace, so that an agent
//! debugs a run without reading 256 MiB, and needs no file tool. A page holds the first line of the trace, the events from
//! `from` that match the filters, at most `limit`, the `call` events of the frames they belong to, so that every value
//! decodes, and `next`.
//!
//! The file is read from its start at each call, one line at a time; a `call` is kept by its place in the file, and read
//! again for the frames of the page only, so a page costs one pass over the file and the memory of its own events.

use std::collections::{BTreeSet, HashMap};
use std::io::{self, BufRead, BufReader, Read as _};
use std::path::Path;

use serde_json::{Value as Json, json};

use super::task::{in_place, trace_path};
use super::{Runs, unknown};
use crate::error::CommandError;
use crate::params::Params;
use crate::paths::{self, Place, Wanted};
use crate::walk::io_text;
use crate::workspace::Workspace;

/// A line of a trace stays within 16 MiB (commands.md 8); one longer is cut there, and read as what it is.
const LINE_MAX: u64 = (16 << 20) + 1024;

/// What the events and the calls of a page may take, within the bound of a result.
const PAGE_BYTES: usize = 3 << 20;

/// One line, without its end, at most [`LINE_MAX`] bytes; `None` at the end of the file.
fn line(reader: &mut impl BufRead) -> io::Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    let read = reader.by_ref().take(LINE_MAX).read_until(b'\n', &mut bytes)?;
    if read == 0 {
        return Ok(None);
    }
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
    } else if read as u64 == LINE_MAX {
        // The rest of a line too long for any writer: skipped.
        let mut rest = Vec::new();
        reader.read_until(b'\n', &mut rest)?;
    }
    if bytes.last() == Some(&b'\r') {
        bytes.pop();
    }
    Ok(Some(bytes))
}

pub(crate) fn trace(runs: &Runs, ws: &dyn Workspace, root: &Path, params: &Params) -> Result<Json, CommandError> {
    if let Some(function) = params.str("function")
        && !(!function.is_empty() && function.split('.').all(labcraft_format::is_name))
    {
        return Err(CommandError::of("params.invalid", "function", format!("`{function}` is not names joined by `.`")));
    }
    if params.str("run").is_some() && (params.str("function").is_some() || params.str("folder").is_some()) {
        return Err(CommandError::of("params.invalid", "run", "give `run`, or `function` and its `folder`, not both"));
    }
    if params.str("folder").is_some() && params.str("function").is_none() {
        return Err(CommandError::of("params.invalid", "folder", "`folder` names the folder of `function`, which is missing"));
    }
    // The last trace of a function run from a folder (decision of the owner, 2026-11-10).
    let last = match params.str("function") {
        Some(function) => {
            let folder = match paths::judge(ws, root, params.str("folder").unwrap_or("."), "folder", Wanted::Folder, false)? {
                // `.labcraft` is reached by paths, but no function runs from a hidden folder (commands.md 2).
                Place::Project { shown, .. } if shown.split('/').any(|name| name.starts_with('.') && name != ".") => {
                    return Err(CommandError::of("path.outside_project", "folder", format!("`{shown}` is hidden: no function runs from it")));
                }
                Place::Project { shown, .. } => shown,
                Place::Shipped(shown) => {
                    return Err(CommandError::of("path.outside_project", "folder", format!("`{shown}` is shipped: no run writes there")));
                }
            };
            Some(trace_path(&folder, function))
        }
        None => None,
    };
    let shown = runs.trace_of(params.str("run"), last)?;
    // A link never leads a read out of the project: the folder of the trace and the trace itself are where they say.
    let folder_shown = shown.rsplit_once('/').map_or("", |(folder, _)| folder);
    let mut path = root.to_path_buf();
    path.extend(shown.split('/'));
    if ws.link_metadata(&path).is_ok() && (!in_place(ws, root, folder_shown) || !in_place(ws, root, &shown)) {
        return Err(CommandError::new("path.outside_project", format!("`{shown}` leads out of the project through a link")));
    }
    let file = ws.open(&path, 0).map_err(|error| match error.kind() {
        io::ErrorKind::NotFound => unknown(format!("there is no trace `{shown}`")),
        _ => CommandError::new("file.unreadable", format!("`{shown}` cannot be read: {}", io_text(&error))),
    })?;
    let frame = params.u64("frame");
    let node = params.str("node");
    let event = params.str("event");
    let from = params.u64("from").unwrap_or(1);
    let limit = usize::try_from(params.u64("limit").unwrap_or(500)).unwrap_or(500);
    let unreadable = |error: io::Error| CommandError::new("file.unreadable", format!("`{shown}` cannot be read: {}", io_text(&error)));

    let mut reader = BufReader::with_capacity(1 << 20, file);
    let Some(first) = line(&mut reader).map_err(unreadable)? else { return Err(unknown(format!("`{shown}` is empty"))) };
    let header: Json = serde_json::from_slice(&first).unwrap_or_else(|_| json!({ "$unreadable": true }));
    if header.get("schema").and_then(Json::as_str) != Some("labcraft-trace/1") {
        return Err(CommandError::new("file.unreadable", format!("`{shown}` does not start with the first line of a trace")));
    }
    let mut offset = first.len() as u64 + 1;
    let mut calls: HashMap<u64, u64> = HashMap::new();
    let mut events: Vec<Json> = Vec::new();
    let mut bytes = 0_usize;
    let mut truncated = false;
    let mut next = None;
    while let Some(text) = line(&mut reader).map_err(unreadable)? {
        let at = offset;
        offset += text.len() as u64 + 1;
        let Ok(found) = serde_json::from_slice::<Json>(&text) else { continue };
        let kind = found.get("event").and_then(Json::as_str);
        let frame_of = found.get("frame").and_then(Json::as_u64);
        if kind == Some("call")
            && let Some(number) = frame_of
        {
            calls.insert(number, at);
        }
        let seq = found.get("seq").and_then(Json::as_u64).unwrap_or(0);
        let matches = seq >= from
            && frame.is_none_or(|frame| frame_of == Some(frame))
            && node.is_none_or(|node| found.get("node").and_then(Json::as_str) == Some(node))
            && event.is_none_or(|event| kind == Some(event));
        if !matches {
            continue;
        }
        // A page always takes its first event, however large, so that paging goes on: the bound of a result cuts its values.
        if events.len() >= limit || (!events.is_empty() && bytes + text.len() > PAGE_BYTES) {
            truncated |= events.len() < limit;
            next = Some(seq);
            break;
        }
        bytes += text.len();
        events.push(found);
    }
    // The calls of the frames of the page, read again from their place, those already in the page aside.
    let in_page: BTreeSet<u64> = events
        .iter()
        .filter(|found| found.get("event").and_then(Json::as_str) == Some("call"))
        .filter_map(|found| found.get("frame").and_then(Json::as_u64))
        .collect();
    let wanted: BTreeSet<u64> =
        events.iter().filter_map(|found| found.get("frame").and_then(Json::as_u64)).filter(|frame| !in_page.contains(frame)).collect();
    let mut frames = Vec::new();
    for number in wanted {
        let Some(at) = calls.get(&number) else { continue };
        let Ok(file) = ws.open(&path, *at) else { continue };
        let mut reader = BufReader::new(file);
        if let Ok(Some(text)) = line(&mut reader)
            && let Ok(call) = serde_json::from_slice::<Json>(&text)
        {
            if bytes + text.len() > PAGE_BYTES {
                truncated = true;
                break;
            }
            bytes += text.len();
            frames.push(call);
        }
    }
    let mut page = json!({ "file": shown, "header": header, "calls": frames, "events": events, "next": next });
    if truncated && let Some(object) = page.as_object_mut() {
        object.insert("truncated".to_owned(), json!(true));
    }
    Ok(page)
}
