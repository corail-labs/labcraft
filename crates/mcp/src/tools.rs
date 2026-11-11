//! The tools, made from the registry (`docs/mcp.md`, 2): one per command enabled in the session, named from its id with
//! `_` for `.`, its description, schema and annotations from the command. And what a call returns (section 3).

use labcraft_engine::registry::{self, Command, Effect};
use labcraft_engine::{CommandError, Session};
use serde_json::{Value as Json, json};

/// The commands that can replace or remove a file (`docs/mcp.md`, 2).
const DESTRUCTIVE: &[&str] = &["file.write", "file.rename", "file.delete", "file.format"];

/// The commands answered on the thread that reads requests, never queued, so that an agent can always stop what it
/// started (`docs/mcp.md`, 5). They wait on the run, never on the session.
pub(crate) const AT_ONCE: &[&str] = &["run.status", "run.set_element", "run.abort"];

/// A tool: the command behind it, and its name.
pub(crate) struct Tool {
    pub name: String,
    pub command: &'static Command,
}

/// The tools of the session: its enabled commands, but `view.*` when no viewer serves it (decision of the owner,
/// 2026-11-11: headless in M0b, they reach an agent when the server is attached to the open IDE).
pub(crate) fn tools_of(session: &Session, viewer: bool) -> Vec<Tool> {
    registry::commands()
        .iter()
        .filter(|command| session.not_enabled(command).is_none())
        .filter(|command| viewer || !command.id.starts_with("view."))
        .map(|command| Tool { name: command.id.replace('.', "_"), command })
        .collect()
}

/// The entry of `tools/list` of a tool. All four hints are given, since MCP takes a missing `destructiveHint` or
/// `openWorldHint` as true.
pub(crate) fn listed(tool: &Tool) -> Json {
    let command = tool.command;
    let description =
        if command.description.is_empty() { command.summary.to_owned() } else { format!("{}\n\n{}", command.summary, command.description) };
    json!({
        "name": tool.name,
        "title": command.label,
        "description": description,
        "inputSchema": command.params.schema(),
        "annotations": {
            "title": command.label,
            "readOnlyHint": command.effects.is_empty(),
            "destructiveHint": DESTRUCTIVE.contains(&command.id),
            "idempotentHint": false,
            "openWorldHint": command.effects.contains(&Effect::Instruments),
        },
    })
}

/// The result of `tools/call`: the object of the command in `structuredContent`, and the same JSON as text in
/// `content`; a failed command, its error with `isError` (`docs/mcp.md`, 3).
pub(crate) fn result_of(outcome: Result<Json, CommandError>) -> Json {
    let (value, failed) = match outcome {
        Ok(value) => (value, false),
        Err(error) => (error.to_json(), true),
    };
    let text = serde_json::to_string(&value).unwrap_or_default();
    let mut result = json!({ "content": [{ "type": "text", "text": text }], "isError": failed });
    if value.is_object()
        && let Some(object) = result.as_object_mut()
    {
        object.insert("structuredContent".to_owned(), value);
    }
    result
}

/// The instructions of the server (`docs/mcp.md`, 4), which name `guide_read` once it is a tool.
pub(crate) fn instructions(tools: &[Tool]) -> String {
    let guide = if tools.iter().any(|tool| tool.command.id == "guide.read") { "`guide_read` once, " } else { "" };
    format!(
        "LabCraft is a graphical dataflow language for test benches; its programs are text files. A library is a folder: \
         one function per `.lcf` file, one type per `.lct` file, each file named after what it declares. Loop: {guide}\
         `functions_list` to see what you can call, `project_describe` to see the files and the simulation files, \
         `file_write` (it checks the library and returns the diagnostics), fix until the report has no error, then \
         `run_start` against a simulation file (`simulation_describe` lists its instruments) and read `status`, `outputs` \
         and `error`. Every diagnostic has a code, the node and the wires at fault, and often a fix."
    )
}

/// At most this many loops named in a progress message.
const LOOPS_SHOWN: usize = 4;

/// The message of a progress notification: the clock and where each loop is, from the report `running`.
pub(crate) fn progress_message(report: &Json) -> String {
    let mut parts = Vec::new();
    if let Some(clock) = report.get("clock").and_then(Json::as_object) {
        for (name, value) in clock {
            if let Some(ms) = value.as_f64() {
                parts.push(format!("{} {ms} ms", name.trim_end_matches("_ms")));
            }
        }
    }
    let loops = report.get("loops").and_then(Json::as_array).map(Vec::as_slice).unwrap_or_default();
    for at in loops.iter().take(LOOPS_SHOWN) {
        let text = |key: &str| at.get(key).and_then(Json::as_str).map(|text| labcraft_types::cut_text(text).into_owned()).unwrap_or_default();
        // Counted from 1, as the command line shows it to a human (cli.md 6.7): the report counts from 0.
        let iteration = at.get("iteration").and_then(Json::as_u64).unwrap_or_default().saturating_add(1);
        parts.push(format!("loop `{}` of `{}` at iteration {iteration}", text("node"), text("function")));
    }
    if loops.len() > LOOPS_SHOWN {
        parts.push(format!("{} loops more", loops.len() - LOOPS_SHOWN));
    }
    parts.join(", ")
}
