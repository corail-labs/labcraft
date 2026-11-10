//! The error of a command (`docs/commands.md`, section 3): a code to decide on, a message to read, the parameter at
//! fault and what would have been accepted.

use labcraft_types::cut_text;
use serde_json::{Value, json};

/// At most this many names in `expected`: a longer list would not help.
const MAX_EXPECTED: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandError {
    pub code: &'static str,
    pub message: String,
    pub parameter: Option<String>,
    pub expected: Vec<String>,
}

impl CommandError {
    /// The message is cut past 200 code points (`types::MAX_TEXT`), with `…`: it can repeat what the caller sent, a path or an id.
    pub fn new(code: &'static str, message: impl Into<String>) -> CommandError {
        CommandError { code, message: cut_text(&message.into()).into_owned(), parameter: None, expected: Vec::new() }
    }

    /// An error about one parameter.
    pub fn of(code: &'static str, parameter: &str, message: impl Into<String>) -> CommandError {
        CommandError { parameter: Some(cut_text(parameter).into_owned()), ..CommandError::new(code, message) }
    }

    /// With what would have been accepted, when that list is short.
    pub fn expecting<S: AsRef<str>>(mut self, expected: &[S]) -> CommandError {
        if expected.len() <= MAX_EXPECTED {
            self.expected = expected.iter().map(|name| cut_text(name.as_ref()).into_owned()).collect();
        }
        self
    }

    pub fn internal(message: impl Into<String>) -> CommandError {
        CommandError::new("internal", message)
    }

    /// `{ "error": { … } }`.
    pub fn to_json(&self) -> Value {
        let mut error = json!({ "code": self.code, "message": self.message });
        if let Some(object) = error.as_object_mut() {
            if let Some(parameter) = &self.parameter {
                object.insert("parameter".to_owned(), json!(parameter));
            }
            if !self.expected.is_empty() {
                object.insert("expected".to_owned(), json!(self.expected));
            }
        }
        json!({ "error": error })
    }
}

/// ", did you mean `x`?" when a name is near, or nothing.
pub fn did_you_mean(name: &str, names: &[&str]) -> String {
    labcraft_runtime::simulation::nearest(name, names).map(|near| format!(": did you mean `{near}`?")).unwrap_or_default()
}
