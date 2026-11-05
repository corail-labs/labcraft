//! The `labcraft.toml` of a library shipped with LabCraft (spec, sections 1 and 9). Manifests of user libraries come in
//! M2, and any other `labcraft.toml` is ignored (`docs/cli.md`, section 3).

use serde::Deserialize;

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
}
