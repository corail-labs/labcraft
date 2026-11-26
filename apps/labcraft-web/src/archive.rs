//! The zip of `Download` (ide.md 12; `docs/architecture.md`, 12.9), written in the worker from the files of its
//! session: a local header and the bytes of each entry, then the central directory and its end record (the format of
//! PKWARE's APPNOTE). Each file is deflated (raw deflate, `flate2`), or stored when deflate does not make it smaller;
//! a folder is an entry of its own, its name ending in `/`. Names are UTF-8, said by bit 11 of the flags, with `/`.
//! No zip64: an archive past the limits of the plain format is refused, far above what the page holds
//! ([`folder::MAX_READ`](crate::folder::MAX_READ)).

use std::io::Write as _;

use flate2::Compression;
use flate2::write::DeflateEncoder;

/// Bit 11 of the flags: the name is UTF-8.
const UTF8: u16 = 1 << 11;
/// Version 2.0 of the format: deflate and folders.
const VERSION: u16 = 20;
/// Made by: Unix (3), version 2.0, so that the external attributes give the permissions.
const MADE_BY: u16 = (3 << 8) | VERSION;
const STORED: u16 = 0;
const DEFLATED: u16 = 8;
/// `-rw-r--r--` and `drwxr-xr-x`, as Unix writes them, and the MS-DOS bit of a folder.
const FILE_ATTRIBUTES: u32 = 0o100_644 << 16;
const FOLDER_ATTRIBUTES: u32 = (0o040_755 << 16) | 0x10;

/// The most entries, and the most bytes of a size or an offset: the plain format keeps `0xFFFF` and `0xFFFF_FFFF` for
/// zip64, which a reader would then look for. The page refuses a folder of more entries when it opens it
/// ([`folder::too_many`](crate::folder::too_many)).
pub const MAX_ENTRIES: usize = 0xFFFE;
const MAX_BYTES: u64 = 0xFFFF_FFFE;

/// What a zip holds: a file and its bytes, or a folder; its path from the root of the zip, with `/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    File { path: String, bytes: Vec<u8> },
    Folder(String),
}

/// The time of change of every entry, local time as MS-DOS keeps it: from 1980 to 2107, to two seconds. Out of that
/// range, the nearest time in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Stamp {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl Default for Stamp {
    /// The first time MS-DOS keeps: 1980-01-01 00:00:00.
    fn default() -> Stamp {
        Stamp { year: 1980, month: 1, day: 1, hour: 0, minute: 0, second: 0 }
    }
}

impl Stamp {
    /// The time and the date, as the headers write them.
    fn dos(self) -> (u16, u16) {
        if self.year < 1980 {
            return Stamp::default().dos();
        }
        let year = self.year.min(2107) - 1980;
        let time = (u16::from(self.hour.min(23)) << 11) | (u16::from(self.minute.min(59)) << 5) | u16::from(self.second.min(59) / 2);
        let date = (year << 9) | (u16::from(self.month.clamp(1, 12)) << 5) | u16::from(self.day.clamp(1, 31));
        (time, date)
    }
}

/// Whether a path can name an entry: relative, with `/`, each part a name (no `.`, `..` or empty part, no `\`).
fn plain(path: &str) -> bool {
    !path.is_empty() && !path.contains('\\') && path.split('/').all(|part| !part.is_empty() && part != "." && part != "..")
}

/// Why a zip is not written, with the code of an error of a command that says it (`docs/commands.md`, 3): a project past
/// the limits of the format, `file.too_large`; a path that cannot name an entry, `path.invalid`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused {
    pub code: &'static str,
    pub message: String,
}

impl Refused {
    fn too_large(message: String) -> Refused {
        Refused { code: "file.too_large", message }
    }

    fn path(message: String) -> Refused {
        Refused { code: "path.invalid", message }
    }
}

/// A number of the format, or why the archive passes its limits.
fn small(value: usize, what: &str) -> Result<u32, Refused> {
    u32::try_from(value).ok().filter(|value| u64::from(*value) <= MAX_BYTES).ok_or_else(|| too_large(what))
}

fn too_large(what: &str) -> Refused {
    Refused::too_large(format!("{what} passes the 4 GiB of a zip without zip64, which LabCraft does not write"))
}

/// What the central directory keeps of an entry.
struct Written {
    name: Vec<u8>,
    name_length: u16,
    method: u16,
    crc: u32,
    compressed: u32,
    size: u32,
    attributes: u32,
    offset: u32,
}

/// The zip of `entries`, in their order, each changed at `stamp`; or why it cannot be written.
pub fn write(entries: &[Entry], stamp: Stamp) -> Result<Vec<u8>, Refused> {
    if entries.len() > MAX_ENTRIES {
        let message = format!("the project holds {} files and folders, more than the {MAX_ENTRIES} of a zip without zip64", entries.len());
        return Err(Refused::too_large(message));
    }
    let (time, date) = stamp.dos();
    let mut out = Vec::new();
    let mut directory = Vec::with_capacity(entries.len());
    for entry in entries {
        let (shown, bytes, attributes) = match entry {
            Entry::File { path, bytes } => (path.as_str(), bytes.as_slice(), FILE_ATTRIBUTES),
            Entry::Folder(path) => (path.as_str(), [].as_slice(), FOLDER_ATTRIBUTES),
        };
        if !plain(shown) {
            return Err(Refused::path(format!("`{shown}` cannot name an entry of a zip: a path from the project, with `/`")));
        }
        // A folder is named with a `/` at its end.
        let name = if matches!(entry, Entry::Folder(_)) { format!("{shown}/") } else { shown.to_owned() }.into_bytes();
        let Ok(name_length) = u16::try_from(name.len()) else { return Err(Refused::path(format!("the path `{shown}` is too long for a zip"))) };
        let size = small(bytes.len(), &format!("`{shown}`"))?;
        let deflated = if bytes.is_empty() { None } else { Some(deflate(bytes)?).filter(|deflated| deflated.len() < bytes.len()) };
        let (method, data) = match &deflated {
            Some(deflated) => (DEFLATED, deflated.as_slice()),
            None => (STORED, bytes),
        };
        let written = Written {
            name,
            name_length,
            method,
            crc: crc32fast::hash(bytes),
            // Never more than `size`: a file that deflate does not make smaller is stored.
            compressed: u32::try_from(data.len()).unwrap_or(size),
            size,
            attributes,
            offset: small(out.len(), "the zip")?,
        };
        put32(&mut out, 0x0403_4b50);
        put16(&mut out, VERSION);
        put16(&mut out, UTF8);
        put16(&mut out, written.method);
        put16(&mut out, time);
        put16(&mut out, date);
        put32(&mut out, written.crc);
        put32(&mut out, written.compressed);
        put32(&mut out, written.size);
        put16(&mut out, written.name_length);
        put16(&mut out, 0);
        out.extend_from_slice(&written.name);
        out.extend_from_slice(data);
        directory.push(written);
    }
    let start = small(out.len(), "the zip")?;
    for written in &directory {
        put32(&mut out, 0x0201_4b50);
        put16(&mut out, MADE_BY);
        put16(&mut out, VERSION);
        put16(&mut out, UTF8);
        put16(&mut out, written.method);
        put16(&mut out, time);
        put16(&mut out, date);
        put32(&mut out, written.crc);
        put32(&mut out, written.compressed);
        put32(&mut out, written.size);
        put16(&mut out, written.name_length);
        // No extra field, no comment, the first disk, no internal attributes.
        put16(&mut out, 0);
        put16(&mut out, 0);
        put16(&mut out, 0);
        put16(&mut out, 0);
        put32(&mut out, written.attributes);
        put32(&mut out, written.offset);
        out.extend_from_slice(&written.name);
    }
    let length = small(out.len() - start as usize, "the central directory of the zip")?;
    small(out.len(), "the zip")?;
    // At most `MAX_ENTRIES`, checked above.
    let count = u16::try_from(entries.len()).map_err(|_| too_large("the number of entries"))?;
    put32(&mut out, 0x0605_4b50);
    put16(&mut out, 0);
    put16(&mut out, 0);
    put16(&mut out, count);
    put16(&mut out, count);
    put32(&mut out, length);
    put32(&mut out, start);
    put16(&mut out, 0);
    Ok(out)
}

/// Raw deflate in memory, which fails only by a bug: `internal`.
fn deflate(bytes: &[u8]) -> Result<Vec<u8>, Refused> {
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(bytes).and_then(|()| encoder.finish()).map_err(|error| Refused { code: "internal", message: format!("deflate: {error}") })
}

fn put16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// The names that Windows keeps for its devices, whatever their case and their extension (`con.txt` names `CON`).
fn reserved(name: &str) -> bool {
    let device = name.split('.').next().unwrap_or_default().trim_end().to_ascii_uppercase();
    let numbered = |prefix: &str| device.strip_prefix(prefix).is_some_and(|digit| digit.len() == 1 && digit.chars().all(|c| c.is_ascii_digit()));
    matches!(device.as_str(), "CON" | "PRN" | "AUX" | "NUL") || numbered("COM") || numbered("LPT")
}

/// The name of the zip of a project, `NAME.zip`, from the name the window shows: what a system refuses in a file name
/// made `_`, a name that Windows keeps for a device given a `_` before it, and `project` for a name left empty.
pub fn file_name(project: &str) -> String {
    let refused = |c: char| c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|');
    let name: String = project.chars().map(|c| if refused(c) { '_' } else { c }).collect();
    // Windows drops a trailing dot or space, and a name of dots alone names a folder.
    let name = name.trim().trim_end_matches('.').trim();
    let name: String = name.chars().take(120).collect();
    if name.is_empty() || name.chars().all(|c| c == '.' || c == '_') {
        "project.zip".to_owned()
    } else if reserved(&name) {
        format!("_{name}.zip")
    } else {
        format!("{name}.zip")
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::*;

    const STAMP: Stamp = Stamp { year: 2026, month: 11, day: 25, hour: 14, minute: 30, second: 17 };

    fn file(path: &str, bytes: &[u8]) -> Entry {
        Entry::File { path: path.to_owned(), bytes: bytes.to_vec() }
    }

    /// The fields of the MS-DOS time, by hand: 14:30:17 is 14 << 11 | 30 << 5 | 8, 2026-11-25 is 46 << 9 | 11 << 5 | 25.
    #[test]
    fn the_time_is_written_as_ms_dos_keeps_it() {
        assert_eq!(STAMP.dos(), ((14 << 11) | (30 << 5) | 8, (46 << 9) | (11 << 5) | 25));
        assert_eq!(Stamp { year: 1970, ..STAMP }.dos(), (0, (1 << 5) | 1), "before 1980: its first day");
        assert_eq!(Stamp { year: 2200, ..STAMP }.dos().1 >> 9, 127, "after 2107: its last year");
    }

    #[test]
    fn a_path_that_is_not_plain_is_refused() {
        for path in ["", "/a.lcf", "a//b.lcf", "../a.lcf", "a/./b.lcf", "a\\b.lcf", "a/..", "a/"] {
            assert_eq!(write(&[file(path, b"x")], STAMP).map_err(|refused| refused.code), Err("path.invalid"), "{path:?}");
        }
        assert_eq!(write(&[Entry::Folder("a/../b".to_owned())], STAMP).map_err(|refused| refused.code), Err("path.invalid"));
    }

    #[test]
    fn too_many_entries_are_refused() {
        let entries: Vec<Entry> = (0..=MAX_ENTRIES).map(|at| Entry::Folder(format!("f{at}"))).collect();
        let refused = write(&entries, STAMP).unwrap_err();
        assert!(refused.code == "file.too_large" && refused.message.contains("zip64"), "{refused:?}");
    }

    /// The bytes of a small zip, worked out by hand from the APPNOTE: one empty folder, stored.
    #[test]
    fn an_empty_folder_is_one_entry_written_as_the_format_says() {
        let zip = write(&[Entry::Folder("e".to_owned())], Stamp::default()).unwrap();
        // Local header: 30 bytes and the name `e/`.
        let local = [0x50, 0x4b, 0x03, 0x04, 20, 0, 0x00, 0x08, 0, 0, 0, 0, 0x21, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, b'e', b'/'];
        assert_eq!(zip.get(..32), Some(local.as_slice()));
        // The end record: one entry, a directory of 46 + 2 bytes at offset 32.
        let end = [0x50, 0x4b, 0x05, 0x06, 0, 0, 0, 0, 1, 0, 1, 0, 48, 0, 0, 0, 32, 0, 0, 0, 0, 0];
        assert_eq!(zip.get(zip.len() - 22..), Some(end.as_slice()));
        assert_eq!(zip.len(), 32 + 48 + 22);
    }

    /// Read back by another reader, the `zip` crate: each entry, its name, its bytes, whether it is a folder, the method
    /// that made it smaller or none, and the time.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn another_reader_reads_back_every_entry() {
        use std::io::Read as _;
        let text = "labcraft 1\n\nfn main() -> () {}\n".repeat(40);
        let entries = [
            file("main.lcf", text.as_bytes()),
            file("sub/ünïcode é.lcf", b"x"),
            file(".labcraft/config.toml", b"[editor]\ndrag_pans = true\n"),
            file("empty.lcf", b""),
            Entry::Folder("nothing/inside".to_owned()),
        ];
        let bytes = write(&entries, STAMP).unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!(archive.len(), entries.len());
        for (at, entry) in entries.iter().enumerate() {
            let mut read = archive.by_index(at).unwrap();
            let modified = read.last_modified().unwrap();
            assert_eq!((modified.year(), modified.month(), modified.day()), (2026, 11, 25));
            assert_eq!((modified.hour(), modified.minute(), modified.second()), (14, 30, 16), "to two seconds");
            match entry {
                Entry::File { path, bytes } => {
                    assert_eq!(read.name().unwrap(), path.as_str());
                    assert!(read.is_file());
                    let mut back = Vec::new();
                    read.read_to_end(&mut back).unwrap();
                    assert_eq!(&back, bytes, "{path}");
                    let method = if path == "main.lcf" { zip::CompressionMethod::Deflated } else { zip::CompressionMethod::Stored };
                    assert_eq!(read.compression(), method, "{path}: deflated only when it is smaller");
                    assert_eq!(read.unix_mode(), Some(0o100_644));
                }
                Entry::Folder(path) => {
                    assert_eq!(read.name().unwrap(), format!("{path}/"));
                    assert!(read.is_dir());
                    assert_eq!(read.size(), 0);
                }
            }
        }
    }

    #[test]
    fn the_name_of_the_zip_is_a_file_name() {
        for (project, name) in [
            ("examples", "examples.zip"),
            ("docs/rfc", "docs_rfc.zip"),
            ("new project", "new project.zip"),
            ("", "project.zip"),
            ("  ", "project.zip"),
            ("..", "project.zip"),
            ("a:b*c?", "a_b_c_.zip"),
            ("bench.", "bench.zip"),
            ("line\nbreak", "line_break.zip"),
            ("con", "_con.zip"),
            ("NUL", "_NUL.zip"),
            ("Aux.tar", "_Aux.tar.zip"),
            ("com7", "_com7.zip"),
            ("LPT0", "_LPT0.zip"),
            ("prn .x", "_prn .x.zip"),
            ("console", "console.zip"),
            ("com10", "com10.zip"),
            ("lpt", "lpt.zip"),
        ] {
            assert_eq!(file_name(project), name, "{project:?}");
        }
        assert_eq!(file_name(&"x".repeat(300)).len(), 124);
    }
}
