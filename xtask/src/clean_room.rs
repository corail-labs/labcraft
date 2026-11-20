//! `cargo xtask clean-room`: the repository holds no LabVIEW file (D-23, `AGENTS.md`). It is part of `lint`, which the job
//! `lint` of the workflow `checks` runs on every pull request, one that changes only prose included: the hook of
//! `.claude/hooks/` protects Claude Code alone, and a contributor may use any agent.

use std::path::Path;
use std::process::Command;

/// The extensions that `.ignore` and `.claude/hooks/guard.py` list, in their order; a test keeps the three in step.
/// `.vim` (a malleable VI) is left out, as in the hook: it is also the extension of Vim scripts.
pub const EXTENSIONS: &[&str] = &["vi", "ctl", "lvproj", "lvlib", "vit", "ctt", "llb", "lvclass"];

/// The files that git tracks, and those it would add (untracked, not ignored), so that the local gate sees a file
/// before its commit. A nested repository, such as a worktree, is not walked into.
pub fn check(root: &Path) -> Result<(), String> {
    let files = repository_files(root, "clean-room")?;
    let found: Vec<String> = files
        .iter()
        .filter(|path| is_labview(path))
        .map(|path| format!("{path} is a LabVIEW file: D-23 refuses it (AGENTS.md, Non-negotiables); remove it"))
        .collect();
    crate::report("clean-room", files.len(), "files", found)
}

/// Whether the name of the file ends with one of [`EXTENSIONS`], in any case: `a.VI` too, but not `a.vim`, `a.vi.md`
/// nor a file named `vi`.
fn is_labview(path: &str) -> bool {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let Some((_, extension)) = name.rsplit_once('.') else {
        return false;
    };
    EXTENSIONS.iter().any(|known| known.eq_ignore_ascii_case(extension))
}

/// The files of the repository, by their path from `root` with `/`: those that git tracks, and those it would add
/// (untracked, not ignored). A nested repository, such as a worktree, is not walked into. `who` names the check in errors.
pub fn repository_files(root: &Path, who: &str) -> Result<Vec<String>, String> {
    let output = Command::new("git")
        .current_dir(root)
        .args(["ls-files", "-z", "--cached", "--others", "--exclude-standard"])
        .output()
        .map_err(|error| format!("{who}: git, which lists the files of the repository, did not run: {error}"))?;
    if !output.status.success() {
        return Err(format!("{who}: git ls-files: {}\n{}", output.status, String::from_utf8_lossy(&output.stderr)));
    }
    Ok(String::from_utf8_lossy(&output.stdout).split('\0').filter(|path| !path.is_empty()).map(str::to_owned).collect())
}

#[cfg(test)]
mod tests {
    use super::{EXTENSIONS, is_labview};

    // The cases are written by hand from D-23 and the list of the hook, not from the output of `is_labview`.
    #[test]
    fn labview_files_are_found_in_any_case_and_any_folder() {
        for path in ["main.vi", "a/b/Control.ctl", "Project.LVPROJ", "lib/x.lvlib", "t.vit", "t.ctt", "old.llb", "c.lvclass", "a\\b.vi", ".vi"] {
            assert!(is_labview(path), "{path}");
        }
    }

    #[test]
    fn other_files_are_not() {
        for path in ["script.vim", "notes.vi.md", "vi", "docs/vi/readme.md", "a.vi2", "a.vi-old", "x.lcf", "vi.rs", "Makefile"] {
            assert!(!is_labview(path), "{path}");
        }
    }

    /// `.ignore` keeps the files out of a search, the hook refuses a call that names one, and this check refuses a
    /// pull request that holds one: the three lists must be one.
    #[test]
    fn the_list_is_the_list_of_ignore_and_of_the_hook() {
        let root = crate::root();
        let ignore = std::fs::read_to_string(root.join(".ignore")).unwrap_or_default();
        let from_ignore: Vec<&str> = ignore.lines().filter(|line| !line.starts_with('#')).filter_map(|line| line.trim().strip_prefix("*.")).collect();
        assert_eq!(from_ignore, EXTENSIONS, ".ignore");

        let hook = std::fs::read_to_string(root.join(".claude/hooks/guard.py")).unwrap_or_default();
        let line = hook.lines().find(|line| line.starts_with("LABVIEW = (")).unwrap_or_default();
        let from_hook: Vec<&str> = line
            .trim_start_matches("LABVIEW = (")
            .trim_end_matches(')')
            .split(',')
            .map(|item| item.trim().trim_matches('"'))
            .filter(|item| !item.is_empty())
            .collect();
        assert_eq!(from_hook, EXTENSIONS, "guard.py");
    }
}
