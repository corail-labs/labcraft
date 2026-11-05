//! The libraries shipped with LabCraft, embedded at build time, so that `check` works in any folder and in the
//! WASM build (`docs/cli.md`, section 3). Apps and test harnesses depend on this crate; no crate of `crates/`
//! does (`cargo xtask libs`).
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable,
    clippy::indexing_slicing,
    clippy::string_slice
)]
#![forbid(unsafe_code)]

use std::borrow::Cow;

use labcraft_check::{Content, LibrarySource, Origin, SourceFile};

/// A `.lcf` or `.lct` file of a library, as it is stored.
#[derive(Debug)]
pub struct File {
    pub name: &'static str,
    pub bytes: &'static [u8],
}

#[derive(Debug)]
pub struct Library {
    pub name: &'static str,
    /// The text of its `labcraft.toml`.
    pub manifest: &'static str,
    pub files: &'static [File],
}

include!(concat!(env!("OUT_DIR"), "/libraries.rs"));

/// Names are compared exactly, even on a file system that ignores case (`docs/cli.md`, section 3).
pub fn find(name: &str) -> Option<&'static Library> {
    LIBRARIES.iter().find(|library| library.name == name)
}

/// A shipped library as `check` reads it. Its files show as `<labcraft>/visa/close.lcf` (`docs/cli.md`, section 1).
pub fn source(name: &str) -> Option<LibrarySource> {
    let library = find(name)?;
    let shown = format!("<labcraft>/{}", library.name);
    let files = library
        .files
        .iter()
        .map(|file| SourceFile { shown: format!("{shown}/{}", file.name), content: Content::Bytes(Cow::Borrowed(file.bytes)) })
        .collect();
    Some(LibrarySource { files, manifest: Some(library.manifest.to_owned()), ..LibrarySource::new(library.name.to_owned(), shown, Origin::Shipped) })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    use super::*;

    #[test]
    fn the_libraries_are_those_of_the_folder() {
        let names: Vec<&str> = LIBRARIES.iter().map(|library| library.name).collect();
        assert_eq!(names, ["scpi", "visa"]);
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        for library in LIBRARIES {
            let on_disk: BTreeSet<String> = std::fs::read_dir(root.join(library.name))
                .unwrap()
                .map(|entry| entry.unwrap().file_name().into_string().unwrap())
                .filter(|name| name.ends_with(".lcf") || name.ends_with(".lct"))
                .collect();
            let embedded: BTreeSet<String> = library.files.iter().map(|file| file.name.to_owned()).collect();
            assert_eq!(embedded, on_disk, "{}", library.name);
            assert!(library.manifest.contains(&format!("name = \"{}\"", library.name)), "{}", library.name);
        }
    }

    #[test]
    fn a_source_shows_where_its_files_come_from() {
        let source = source("visa").unwrap();
        assert_eq!((source.name.as_str(), source.origin), ("visa", Origin::Shipped));
        assert!(source.files.iter().any(|file| file.shown == "<labcraft>/visa/close.lcf"));
        assert!(source.manifest.is_some());
    }

    #[test]
    fn a_name_is_compared_exactly() {
        assert!(find("visa").is_some());
        assert!(find("VISA").is_none());
        assert!(find("embedded").is_none());
    }
}
