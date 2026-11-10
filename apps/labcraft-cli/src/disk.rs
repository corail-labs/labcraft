//! The disk, as `engine` reaches it (`docs/architecture.md`, 10.8): `std::fs`, and atomic writes through `craft-atomic`,
//! which keeps a symbolic link a link (section 6).

use std::fs::{self, File};
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use labcraft_engine::workspace::{Entry, Kind, Metadata, Read, Workspace};

/// The disk, the current folder that of the process.
#[derive(Clone, Copy, Debug, Default)]
pub struct Disk;

fn metadata_of(metadata: &fs::Metadata) -> Metadata {
    let kind = kind_of(metadata.file_type());
    let modified = metadata.modified().ok().and_then(|time| time.duration_since(UNIX_EPOCH).ok()).map(|since| since.as_nanos());
    Metadata { kind, len: metadata.len(), modified, read_only: metadata.permissions().readonly() }
}

fn kind_of(file_type: fs::FileType) -> Kind {
    if file_type.is_symlink() {
        Kind::Link
    } else if file_type.is_dir() {
        Kind::Folder
    } else if file_type.is_file() {
        Kind::File
    } else {
        Kind::Other
    }
}

impl Workspace for Disk {
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        fs::canonicalize(path)
    }

    fn absolute(&self, path: &Path) -> io::Result<PathBuf> {
        std::path::absolute(path)
    }

    fn current(&self) -> Option<PathBuf> {
        std::env::current_dir().ok()
    }

    fn metadata(&self, path: &Path) -> io::Result<Metadata> {
        fs::metadata(path).map(|metadata| metadata_of(&metadata))
    }

    fn link_metadata(&self, path: &Path) -> io::Result<Metadata> {
        fs::symlink_metadata(path).map(|metadata| metadata_of(&metadata))
    }

    fn read_dir(&self, path: &Path, each: &mut dyn FnMut(io::Result<Entry>)) -> io::Result<()> {
        for entry in fs::read_dir(path)? {
            each(entry.map(|entry| Entry { name: entry.file_name(), kind: entry.file_type().map(kind_of) }));
        }
        Ok(())
    }

    fn read(&self, path: &Path, max: u64) -> io::Result<Read> {
        let file = File::open(path)?;
        let size = file.metadata()?.len();
        if size > max {
            return Ok(Read::TooLarge(size));
        }
        let mut bytes = Vec::new();
        // `with_capacity` aborts when memory runs out, and `--max-file-size` can ask for 1 GiB of it.
        bytes.try_reserve_exact(usize::try_from(size).unwrap_or(usize::MAX)).map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))?;
        file.take(max.saturating_add(1)).read_to_end(&mut bytes)?;
        Ok(Read::Bytes(bytes))
    }

    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        craft_atomic::atomic_write(path, bytes)
    }

    fn create_folders(&self, path: &Path) -> io::Result<()> {
        fs::create_dir_all(path)
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        fs::rename(from, to)
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        fs::remove_file(path)
    }
}
