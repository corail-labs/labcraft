//! The registry itself (`docs/commands.md`, 6.6): `commands.list` and `commands.describe`.

use serde_json::{Value, json};

use crate::error::{CommandError, did_you_mean};
use crate::params::Params;
use crate::registry::{self, Command};
use crate::session::Session;

fn state(session: &Session, command: &Command) -> (bool, Option<String>) {
    let reason = session.not_enabled(command);
    (reason.is_none(), reason)
}

/// `commands.list`.
pub fn list(session: &Session, params: &Params) -> Value {
    let filter = params.str("filter").unwrap_or_default();
    let commands: Vec<Value> = registry::commands()
        .iter()
        .filter(|command| command.id.contains(filter))
        .map(|command| {
            let (enabled, reason) = state(session, command);
            let mut entry = json!({
                "id": command.id,
                "summary": command.summary,
                "effects": command.effects.iter().map(|effect| effect.as_str()).collect::<Vec<_>>(),
                "long": command.long,
                "enabled": enabled,
            });
            if let (Some(reason), Some(object)) = (reason, entry.as_object_mut()) {
                object.insert("reason".to_owned(), json!(reason));
            }
            entry
        })
        .collect();
    json!({ "commands": commands })
}

/// `commands.describe`.
pub fn describe(session: &Session, params: &Params) -> Result<Value, CommandError> {
    let id = params.text("id")?;
    let Some(command) = registry::command(id) else {
        let ids: Vec<&str> = registry::commands().iter().map(|command| command.id).collect();
        let near: Vec<&str> = labcraft_runtime::simulation::nearest(id, &ids).into_iter().collect();
        return Err(CommandError::of("command.unknown", "id", format!("no command has the id `{id}`{}", did_you_mean(id, &ids))).expecting(&near));
    };
    let mut described = command.describe();
    let (enabled, reason) = state(session, command);
    if let Some(object) = described.as_object_mut() {
        object.insert("enabled".to_owned(), json!(enabled));
        if let Some(reason) = reason {
            object.insert("reason".to_owned(), json!(reason));
        }
    }
    Ok(described)
}
