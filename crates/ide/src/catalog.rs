//! The entries that the window ships as data (ide.md 6): the workspaces, the entries to come with their cards, and the
//! commands of the command palette, made from the registry of `engine` and the actions of the window.

use craft_keymap::{CommandRef, Keymap};
use kit_shell::Planned;
use kit_shell::palette::{Entry, Param, State};
use kit_shell::start::Example;
use kit_shell::workspaces::{Availability, Workspace};
use labcraft_engine::registry;
use serde_json::Value;

use crate::keys;

/// The repository of LabCraft, which the About card links to.
pub const REPOSITORY: &str = "https://github.com/corail-labs/labcraft";

/// The roadmap, where the milestones to come are said, with no date.
pub const ROADMAP: &str = "https://github.com/corail-labs/labcraft/blob/main/docs/roadmap.md";

fn issue(number: u32) -> String {
    format!("https://github.com/corail-labs/labcraft/issues/{number}")
}

/// An example of the start page (ide.md 10), by the name of its folder: `examples` or `docs/rfc`.
pub fn example(name: &str) -> Example {
    let (title, line) = match name {
        "examples" => ("The examples", "A bench: a power supply, a multimeter, relays, and the functions that test with them"),
        "docs/rfc" => ("The programs of the RFCs", "The programs that the RFCs of the language give"),
        _ => ("An example", ""),
    };
    Example { name: title.to_owned(), line: line.to_owned() }
}

/// The workspaces of ide.md 6.
pub fn workspaces() -> Vec<Workspace> {
    let planned = |milestone: &str| Availability::Planned { milestone: milestone.to_owned(), link: Some(ROADMAP.to_owned()) };
    vec![
        Workspace {
            id: "edit".to_owned(),
            label: "Edit".to_owned(),
            line: "Draw, wire and run functions; text and block side by side".to_owned(),
            state: Availability::Available,
        },
        Workspace {
            id: "panel".to_owned(),
            label: "Panel".to_owned(),
            line: "The front panel: controls and indicators of a function".to_owned(),
            state: planned("M3"),
        },
        Workspace {
            id: "debug".to_owned(),
            label: "Debug".to_owned(),
            line: "Probes, breakpoints, step by step on the diagram".to_owned(),
            state: planned("M5"),
        },
        Workspace {
            id: "build".to_owned(),
            label: "Build".to_owned(),
            line: "A standalone executable, a library with a C ABI, a .NET facade".to_owned(),
            state: planned("M6"),
        },
    ]
}

/// The card of an entry of the window to come, by its id.
pub fn planned(id: &str) -> Option<Planned> {
    let card = |label: &str, line: &str, milestone: &str, link: String| Planned {
        label: label.to_owned(),
        line: line.to_owned(),
        milestone: milestone.to_owned(),
        link: Some(link),
    };
    match id {
        keys::PLANNED_INSTRUMENTS => Some(card(
            "Target: real instruments",
            "Running a function on the instruments of the bench, not on the simulator",
            "M4",
            ROADMAP.to_owned(),
        )),
        keys::PLANNED_DETACHED_MODE => Some(card(
            "Every function in its window",
            "A mode where each function opens in a window of its own, the new ones too, once those windows have their place in the taskbar, minimize and maximize",
            "After craft-libs#1903",
            "https://github.com/storytold/craft-libs/issues/1903".to_owned(),
        )),
        keys::PLANNED_PAUSE => Some(card("Pause", "Pausing a run between two nodes, and going on", "M5", ROADMAP.to_owned())),
        keys::PLANNED_BREAKPOINT => {
            Some(card("Breakpoint", "A breakpoint on a node: the run pauses before it, step by step from there", "M5", ROADMAP.to_owned()))
        }
        keys::PLANNED_WATCH => Some(card("Watch", "Watching a value on a wire while the function runs", "M5", ROADMAP.to_owned())),
        keys::PLANNED_BUILD => {
            Some(card("Build", "A standalone executable, a library with a C ABI or a .NET facade, from the project", "M6", ROADMAP.to_owned()))
        }
        keys::PLANNED_AGENT => Some(card(
            "Connect an AI agent",
            "An agent attached to this window edits the project with you, in the one history, signed with its name",
            "M1, step 8",
            issue(167),
        )),
        "ide.planned.panel_view" => Some(card("Panel", "The front panel of the function: its controls and indicators", "M3", ROADMAP.to_owned())),
        "ide.planned.settings" => Some(card("Settings", "The settings of the user and of the project, and the keys", "M1, step 6", issue(165))),
        _ => None,
    }
}

/// The commands of the command palette: every command of the registry, then the actions of the window, with their
/// keys. Without a project, only what opens one can run.
pub fn palette_entries(keymap: &Keymap, project: bool) -> Vec<Entry> {
    let key = |id: &str| keys::key_of(keymap, &CommandRef::new(id));
    let opens = |id: &str| matches!(id, "project.new" | "project.open");
    let mut entries = Vec::new();
    for command in registry::commands() {
        let state = match command.planned {
            Some(planned) => State::Planned(Planned {
                label: command.label.to_owned(),
                line: command.summary.to_owned(),
                milestone: planned.to_owned(),
                link: Some(ROADMAP.to_owned()),
            }),
            None if !project && !opens(command.id) => State::Disabled("Open a project first".to_owned()),
            None => State::Enabled,
        };
        let params = command
            .params
            .properties
            .iter()
            .map(|(name, schema)| Param {
                name: (*name).to_owned(),
                required: command.params.required.contains(name),
                text: schema.get("type").and_then(Value::as_str) == Some("string"),
            })
            .collect();
        entries.push(Entry { id: command.id.to_owned(), label: command.label.to_owned(), key: key(command.id), state, params });
    }
    for (id, label) in keys::WINDOW_ACTIONS {
        let state = if !project && matches!(*id, keys::FIT | keys::VALUES | keys::OPEN_TRACE | keys::RUN | keys::STOP) {
            State::Disabled("Open a project first".to_owned())
        } else if let Some(why) = labcraft_engine::runs::not_enabled_here().filter(|_| matches!(*id, keys::RUN | keys::STOP)) {
            State::Disabled(why.to_owned())
        } else {
            State::Enabled
        };
        entries.push(Entry { id: (*id).to_owned(), label: (*label).to_owned(), key: key(id), state, params: Vec::new() });
    }
    for id in [
        keys::PLANNED_PAUSE,
        keys::PLANNED_BREAKPOINT,
        keys::PLANNED_WATCH,
        keys::PLANNED_INSTRUMENTS,
        keys::PLANNED_BUILD,
        keys::PLANNED_DETACHED_MODE,
        keys::PLANNED_AGENT,
    ] {
        if let Some(planned) = planned(id) {
            entries.push(Entry { id: id.to_owned(), label: planned.label.clone(), key: None, state: State::Planned(planned), params: Vec::new() });
        }
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_of_the_registry_is_in_the_palette_once() {
        let keymap = keys::keymap(craft_keymap::Platform::Windows);
        let entries = palette_entries(&keymap, true);
        for command in registry::commands() {
            assert_eq!(entries.iter().filter(|entry| entry.id == command.id).count(), 1, "{}", command.id);
        }
        let undo = entries.iter().find(|entry| entry.id == "history.undo").unwrap();
        assert_eq!(undo.key.as_deref(), Some("Ctrl+Z"));
    }

    #[test]
    fn without_a_project_only_what_opens_one_runs() {
        let keymap = keys::keymap(craft_keymap::Platform::Windows);
        let entries = palette_entries(&keymap, false);
        let state = |id: &str| entries.iter().find(|entry| entry.id == id).map(|entry| entry.state.clone()).unwrap();
        assert_eq!(state("project.open"), State::Enabled);
        assert!(matches!(state("file.read"), State::Disabled(_)));
        assert!(matches!(state(keys::PLANNED_AGENT), State::Planned(_)));
    }

    #[test]
    fn a_planned_workspace_has_its_card() {
        let workspaces = workspaces();
        assert!(workspaces.iter().find(|workspace| workspace.id == "edit").unwrap().planned().is_none());
        assert_eq!(workspaces.iter().find(|workspace| workspace.id == "debug").unwrap().planned().unwrap().milestone, "M5");
    }
}
