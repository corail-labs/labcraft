//! The `labcraft.toml` of a library (spec, sections 1 and 9). That of a library shipped with LabCraft is read whole; of
//! any other, only `name`, `always` and `resources` until M2 (`docs/cli.md`, section 3), its other keys ignored.

use serde::Deserialize;
use toml::{Table, Value};

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    /// The libraries it uses.
    #[serde(default)]
    pub dependencies: Vec<String>,
    /// Native functions that run even when `err` holds an error (spec 7.2).
    #[serde(default)]
    pub always: Vec<String>,
    /// Opaque types that are resources (spec 8.1).
    #[serde(default)]
    pub resources: Vec<String>,
    /// Categories of errors (spec 7.1).
    #[serde(default)]
    pub categories: Vec<String>,
}

impl Manifest {
    /// The error says why, in one line, for `LC0105`.
    pub fn parse(text: &str) -> Result<Manifest, String> {
        toml::from_str(text).map_err(|error| error.message().to_owned())
    }
}

/// What `check` reads of the manifest of a user library: `name`, when it is given, and the lists `always` and
/// `resources`. A mask alone (spec 11.3) has no `name`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct UserManifest {
    pub name: Option<String>,
    pub always: Vec<String>,
    pub resources: Vec<String>,
    /// What one of these keys says that it cannot say, each in one line, for `LC0105`: that key is then read as absent.
    pub wrong: Vec<String>,
}

impl UserManifest {
    /// The error is a text that is not TOML, in one line, for `LC0106`.
    pub fn parse(text: &str) -> Result<UserManifest, String> {
        let table: Table = toml::from_str(text).map_err(|error| error.message().to_owned())?;
        let mut read = UserManifest::default();
        match table.get("name") {
            None => {}
            Some(Value::String(name)) => read.name = Some(name.clone()),
            Some(other) => read.wrong.push(format!("`name` must be a string, and is {}", other.type_str())),
        }
        read.always = strings(&table, "always", &mut read.wrong);
        read.resources = strings(&table, "resources", &mut read.wrong);
        Ok(read)
    }
}

fn strings(table: &Table, key: &str, wrong: &mut Vec<String>) -> Vec<String> {
    let Some(value) = table.get(key) else { return Vec::new() };
    let Value::Array(items) = value else {
        wrong.push(format!("`{key}` must be an array of strings, and is {}", value.type_str()));
        return Vec::new();
    };
    let texts: Option<Vec<String>> = items.iter().map(|item| item.as_str().map(str::to_owned)).collect();
    texts.unwrap_or_else(|| {
        wrong.push(format!("`{key}` must be an array of strings"));
        Vec::new()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_manifest_gives_its_fields() {
        let text = "name = \"a\"\nversion = \"1\"\nalways = [\"close\"]\nresources = [\"session\"]\n";
        let manifest = Manifest::parse(text).unwrap();
        assert_eq!((manifest.name.as_str(), manifest.version.as_str()), ("a", "1"));
        assert_eq!((manifest.always, manifest.resources), (vec!["close".to_owned()], vec!["session".to_owned()]));
        assert!(manifest.dependencies.is_empty() && manifest.categories.is_empty());
    }

    #[test]
    fn what_a_manifest_cannot_say_is_refused() {
        for text in ["", "name = \"a\"", "name = 1\nversion = \"1\"", "name = \"a\"\nversion = \"1\"\nnonsense = 1", "name = "] {
            assert!(Manifest::parse(text).is_err(), "{text}");
        }
    }

    #[test]
    fn a_user_manifest_gives_three_keys_and_ignores_the_others() {
        let text = "name = \"a\"\nversion = \"1\"\nnonsense = 1\nalways = [\"close\"]\nresources = [\"session\"]\n[mask]\nband = \"A\"\n";
        let read = UserManifest::parse(text).unwrap();
        let expected =
            UserManifest { name: Some("a".to_owned()), always: vec!["close".to_owned()], resources: vec!["session".to_owned()], wrong: Vec::new() };
        assert_eq!(read, expected);
        assert_eq!(UserManifest::parse("").unwrap(), UserManifest::default());
        assert_eq!(UserManifest::parse("[mask]\nband = \"A\"\n").unwrap(), UserManifest::default());
    }

    #[test]
    fn a_user_manifest_that_is_not_toml_is_refused_and_a_key_of_the_wrong_kind_is_wrong() {
        assert!(UserManifest::parse("name = ").is_err());
        assert!(UserManifest::parse("always = [\"close\"").is_err());
        let read = UserManifest::parse("name = 1\nalways = \"close\"\nresources = [\"session\", 2]\n").unwrap();
        assert_eq!((read.name, read.always, read.resources), (None, Vec::new(), Vec::new()));
        assert_eq!(
            read.wrong,
            [
                "`name` must be a string, and is integer",
                "`always` must be an array of strings, and is string",
                "`resources` must be an array of strings"
            ]
        );
    }
}
