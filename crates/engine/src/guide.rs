//! `guide.read` (`docs/commands.md`, 6.1): the skills of `skills/`, built into LabCraft by `build.rs`, so that every MCP
//! client reads them and they match the version of the tool. Nothing here reads a path from the disk.

use serde_json::{Value, json};

use crate::error::{CommandError, did_you_mean};
use crate::params::Params;

/// One skill: a folder of `skills/` with a `SKILL.md`.
#[derive(Debug)]
pub struct Skill {
    pub name: &'static str,
    /// `SKILL.md` first, then the others by path.
    pub files: &'static [SkillFile],
}

/// A Markdown file of a skill, by its path in the folder of the skill (`reference.md`).
#[derive(Debug)]
pub struct SkillFile {
    pub path: &'static str,
    pub text: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/skills.rs"));

/// The skill whose `SKILL.md` a call without `skill` gives: the one that writes a program, so one call starts the loop.
pub const FIRST: &str = "labcraft-writing";

/// The file a call without `file` reads.
pub const SKILL_FILE: &str = "SKILL.md";

/// Every skill, in the order of their names.
pub fn skills() -> &'static [Skill] {
    SKILLS
}

/// Names are compared exactly: they are names of embedded documents, never paths to open.
pub fn find(name: &str) -> Option<&'static Skill> {
    SKILLS.iter().find(|skill| skill.name == name)
}

impl Skill {
    pub fn file(&self, path: &str) -> Option<&'static SkillFile> {
        self.files.iter().find(|file| file.path == path)
    }

    /// A field of the front matter of its `SKILL.md` (`name`, `description`), which says when to use it.
    pub fn front_matter(&self, key: &str) -> Option<&'static str> {
        front_matter(self.file(SKILL_FILE)?.text, key)
    }

    fn paths(&self) -> Vec<&'static str> {
        self.files.iter().map(|file| file.path).collect()
    }
}

/// `key: value` between the two `---` lines that open a file of the Agent Skills format.
fn front_matter(text: &'static str, key: &str) -> Option<&'static str> {
    let mut lines = text.lines();
    if lines.next()? != "---" {
        return None;
    }
    lines
        .take_while(|line| *line != "---")
        .find_map(|line| line.strip_prefix(key)?.strip_prefix(':').map(str::trim))
        .filter(|value| !value.is_empty())
}

/// `guide.read`.
pub fn read(params: &Params) -> Result<Value, CommandError> {
    let path = params.str("file").unwrap_or(SKILL_FILE);
    let names: Vec<&str> = SKILLS.iter().map(|skill| skill.name).collect();
    let Some(name) = params.str("skill") else {
        if path != SKILL_FILE {
            let message = format!("`file` names a file of a skill: give `skill` too, one of {}", quoted(&names));
            return Err(CommandError::of("params.invalid", "skill", message).expecting(&names));
        }
        let Some(first) = find(FIRST).and_then(|skill| skill.file(SKILL_FILE)) else {
            return Err(CommandError::internal(format!("the skill `{FIRST}` is not built in")));
        };
        let skills: Vec<Value> = SKILLS
            .iter()
            .map(|skill| json!({ "name": skill.name, "description": skill.front_matter("description").unwrap_or(""), "files": skill.paths() }))
            .collect();
        return Ok(json!({ "skills": skills, "skill": FIRST, "file": SKILL_FILE, "text": first.text }));
    };
    let Some(skill) = find(name) else {
        let message = format!("no skill is named `{name}`{}", did_you_mean(name, &names));
        return Err(CommandError::of("params.invalid", "skill", message).expecting(&names));
    };
    let Some(file) = skill.file(path) else {
        let paths = skill.paths();
        let message = format!("the skill `{name}` has no file `{path}`{}", did_you_mean(path, &paths));
        return Err(CommandError::of("params.invalid", "file", message).expecting(&paths));
    };
    Ok(json!({ "skill": skill.name, "file": file.path, "text": file.text }))
}

fn quoted(names: &[&str]) -> String {
    names.iter().map(|name| format!("`{name}`")).collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_skill_names_itself_and_says_when_to_use_it() {
        assert!(SKILLS.len() >= 3, "{SKILLS:?}");
        assert!(find(FIRST).is_some());
        for skill in SKILLS {
            assert_eq!(skill.front_matter("name"), Some(skill.name), "the `name` of {} is its folder", skill.name);
            let description = skill.front_matter("description").unwrap_or_default();
            assert!(description.starts_with("Use when"), "{}: {description}", skill.name);
            assert_eq!(skill.files.first().map(|file| file.path), Some(SKILL_FILE));
        }
    }

    #[test]
    fn a_front_matter_is_read_between_its_two_lines_only() {
        assert_eq!(front_matter("---\nname: a\ndescription: b c\n---\n", "description"), Some("b c"));
        assert_eq!(front_matter("---\nname: a\n---\ndescription: after\n", "description"), None);
        assert_eq!(front_matter("name: a\n", "name"), None);
        assert_eq!(front_matter("---\nnames: a\nname:\n---\n", "name"), None);
    }
}
