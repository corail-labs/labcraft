//! What `labcraft PATH` opens (`docs/cli.md`, section 8): a folder, or the project of a `.lcf` or `.lct` file, with
//! that file to show.

use std::path::{Component, Path, PathBuf};

/// The folders looked at, from the folder of a file up, for its project *(proposed)*: as many as for a trace (cli.md 8).
pub const MAX_ABOVE: usize = 64;

/// The folder a session keeps its journal and its settings in: the mark of a folder opened as a project.
pub const MARK: &str = ".labcraft";

/// What the window opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub folder: PathBuf,
    /// The file to show, from `folder`, its folders joined by `/`.
    pub file: Option<String>,
}

/// What `path` opens: a folder, or what does not exist, as it is (the window says what is wrong); a `.lcf` or a `.lct`
/// file, in the nearest folder above it that holds [`MARK`], the folder of the file itself first, at most
/// [`MAX_ABOVE`], never the home folder, a folder above it or the root of a drive (mcp.md 1); with none, the folder of
/// the file. Another file is a wrong usage: why.
pub fn resolve(path: &Path) -> Result<Target, String> {
    if !path.is_file() {
        return Ok(Target { folder: path.to_path_buf(), file: None });
    }
    let is_program = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("lcf") || extension.eq_ignore_ascii_case("lct"));
    if !is_program {
        return Err(format!("`{}` is neither a folder nor a .lcf or .lct file", path.display()));
    }
    let path = std::path::absolute(path).map_err(|error| format!("`{}`: {error}", path.display()))?;
    // On Unix `absolute` keeps `..`, which the walk up would take for a folder: taken off here, as Windows does.
    let path = path.components().fold(PathBuf::new(), |mut kept, component| {
        match component {
            Component::ParentDir => {
                kept.pop();
            }
            Component::CurDir => {}
            other => kept.push(other),
        }
        kept
    });
    let Some(own) = path.parent() else { return Ok(Target { folder: path.clone(), file: None }) };
    let home = std::env::home_dir().and_then(|home| std::fs::canonicalize(home).ok());
    let folder = own
        .ancestors()
        .take(MAX_ABOVE)
        .take_while(|above| std::fs::canonicalize(above).is_ok_and(|real| labcraft_cli::mcp::too_wide(&real, home.as_deref()).is_none()))
        .find(|above| above.join(MARK).is_dir())
        .unwrap_or(own)
        .to_path_buf();
    let file = path.strip_prefix(&folder).ok().map(labcraft_engine::walk::slashed);
    Ok(Target { folder, file })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder of its own for a test, removed with it.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Scratch {
            let folder = std::env::temp_dir().join(format!("labcraft-target-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&folder);
            std::fs::create_dir_all(&folder).unwrap();
            Scratch(std::path::absolute(folder).unwrap())
        }

        fn file(&self, path: &str) -> PathBuf {
            let path = self.0.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "labcraft 1\n").unwrap();
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_file_opens_the_nearest_folder_above_it_that_was_opened_as_a_project() {
        let scratch = Scratch::new("mark");
        let deep = scratch.file("proj/power/check_power.lcf");
        let volts = scratch.file("proj/power/Volts.lct");
        // Never opened: the folder of the file.
        assert_eq!(resolve(&deep), Ok(Target { folder: scratch.0.join("proj").join("power"), file: Some("check_power.lcf".to_owned()) }));
        // `proj` opened once: its `.labcraft` names it.
        std::fs::create_dir_all(scratch.0.join("proj").join(MARK)).unwrap();
        assert_eq!(resolve(&deep), Ok(Target { folder: scratch.0.join("proj"), file: Some("power/check_power.lcf".to_owned()) }));
        // A `..` in the path is no folder of the walk: the project of `proj/other/../power/…` is still `proj`.
        std::fs::create_dir_all(scratch.0.join("proj").join("other").join(MARK)).unwrap();
        let roundabout = scratch.0.join("proj").join("other").join("..").join("power").join("check_power.lcf");
        assert_eq!(resolve(&roundabout).unwrap().folder, scratch.0.join("proj"));
        assert_eq!(resolve(&volts).unwrap().file.as_deref(), Some("power/Volts.lct"));
        // The nearest wins: the library opened as a project of its own.
        std::fs::create_dir_all(scratch.0.join("proj").join("power").join(MARK)).unwrap();
        assert_eq!(resolve(&deep).unwrap().folder, scratch.0.join("proj").join("power"));
    }

    #[test]
    fn a_folder_stays_a_folder_and_another_file_is_wrong_usage() {
        let scratch = Scratch::new("other");
        assert_eq!(resolve(&scratch.0), Ok(Target { folder: scratch.0.clone(), file: None }));
        let missing = scratch.0.join("missing.lcf");
        assert_eq!(resolve(&missing), Ok(Target { folder: missing.clone(), file: None }));
        let notes = scratch.file("notes.txt");
        assert_eq!(resolve(&notes), Err(format!("`{}` is neither a folder nor a .lcf or .lct file", notes.display())));
        // The extension in any case, as Windows gives it.
        assert!(resolve(&scratch.file("Main.LCF")).is_ok_and(|target| target.file.as_deref() == Some("Main.LCF")));
    }
}
