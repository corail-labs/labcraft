//! The libraries shipped with LabCraft, as the app gives them: `engine` sees no crate that embeds them
//! (`cargo xtask libs`), so the app hands it their files (`labcraft-libs`). They read as `<labcraft>/visa/close.lcf`
//! (`docs/cli.md`, section 1), and are never written (`docs/commands.md`, section 2).

use std::borrow::Cow;

use labcraft_check::{Content, LibrarySource, Origin, SourceFile};

/// How a shipped file is named in reports and in the paths of commands.
pub const PREFIX: &str = "<labcraft>/";

#[derive(Clone, Copy, Debug)]
pub struct ShippedFile {
    /// Its path under the folder of its library: `close.lcf`, `sim/bench.toml`.
    pub name: &'static str,
    pub bytes: &'static [u8],
}

#[derive(Clone, Debug)]
pub struct ShippedLibrary {
    pub name: &'static str,
    /// The text of its `labcraft.toml`.
    pub manifest: &'static str,
    /// Its `.lcf` and `.lct` files.
    pub files: Vec<ShippedFile>,
    /// The simulation files it ships, under `sim/`.
    pub simulations: Vec<ShippedFile>,
}

#[derive(Clone, Debug, Default)]
pub struct Shipped {
    libraries: Vec<ShippedLibrary>,
}

impl Shipped {
    pub fn new(mut libraries: Vec<ShippedLibrary>) -> Shipped {
        libraries.sort_by_key(|library| library.name);
        for library in &mut libraries {
            library.files.sort_by_key(|file| file.name);
            library.simulations.sort_by_key(|file| file.name);
        }
        Shipped { libraries }
    }

    /// In the order of their names.
    pub fn libraries(&self) -> &[ShippedLibrary] {
        &self.libraries
    }

    /// Names are compared exactly, even on a file system that ignores case (`docs/cli.md`, section 3).
    pub fn find(&self, name: &str) -> Option<&ShippedLibrary> {
        self.libraries.iter().find(|library| library.name == name)
    }

    /// A shipped library as `check` reads it.
    pub fn source(&self, name: &str) -> Option<LibrarySource> {
        let library = self.find(name)?;
        let shown = format!("{PREFIX}{}", library.name);
        let files = library
            .files
            .iter()
            .map(|file| SourceFile { shown: format!("{shown}/{}", file.name), content: Content::Bytes(Cow::Borrowed(file.bytes)) })
            .collect();
        Some(LibrarySource {
            files,
            manifest: Some(library.manifest.to_owned()),
            ..LibrarySource::new(library.name.to_owned(), shown, Origin::Shipped)
        })
    }

    /// A file by its name in reports, `<labcraft>/visa/sim/bench.toml`: a program or a simulation file.
    pub fn file(&self, shown: &str) -> Option<&'static [u8]> {
        let (library, name) = shown.strip_prefix(PREFIX)?.split_once('/')?;
        let library = self.find(library)?;
        library.files.iter().chain(&library.simulations).find(|file| file.name == name).map(|file| file.bytes)
    }
}
