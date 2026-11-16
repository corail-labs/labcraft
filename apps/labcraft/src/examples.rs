//! The examples of the start page (ide.md 10): the program files of `examples/` and of `docs/rfc/`, built into the app,
//! each opened as a copy in a folder the user chooses, never the files shipped with LabCraft.

use std::path::Path;

use labcraft_engine::CommandError;

/// A folder built into the app.
#[derive(Debug)]
pub struct Bundled {
    pub name: &'static str,
    pub files: &'static [BundledFile],
}

#[derive(Debug)]
pub struct BundledFile {
    /// From the folder, with `/`.
    pub path: &'static str,
    pub bytes: &'static [u8],
}

include!(concat!(env!("OUT_DIR"), "/bundled.rs"));

/// Copies the example of this index into `folder`, which must be empty or not exist yet.
pub fn copy(at: usize, folder: &Path) -> Result<(), CommandError> {
    let Some(bundled) = BUNDLED.get(at) else {
        return Err(CommandError::internal(format!("no example {at}")));
    };
    crate::session::make_new_folder(folder)?;
    for file in bundled.files {
        let path = folder.join(file.path);
        let written = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::write(&path, file.bytes));
        if let Err(error) = written {
            return Err(CommandError::of("path.invalid", "folder", format!("{}: {error}", path.display())));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_examples_and_the_programs_of_the_rfcs_are_built_in() {
        assert_eq!(BUNDLED.iter().map(|bundled| bundled.name).collect::<Vec<_>>(), ["examples", "docs/rfc"]);
        assert!(BUNDLED.iter().all(|bundled| !bundled.files.is_empty()));
        assert!(BUNDLED.iter().all(|bundled| labcraft_ide::catalog::example(bundled.name).name != "An example"));
    }

    #[test]
    fn a_copy_goes_into_an_empty_folder_only() {
        let parent = std::env::temp_dir().join(format!("labcraft-example-{}", std::process::id()));
        let folder = parent.join("copy");
        copy(0, &folder).unwrap();
        let first = &BUNDLED[0].files[0];
        assert_eq!(std::fs::read(folder.join(first.path)).unwrap(), first.bytes);
        assert_eq!(copy(0, &folder).unwrap_err().code, "path.exists");
        let _ = std::fs::remove_dir_all(&parent);
    }
}
