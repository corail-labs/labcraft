//! `cargo xtask unicode`: the tables of `types::text` (`docs/architecture.md`, section 11.1), generated from the Unicode
//! Character Database of the version that names are read with (spec 14.1), rather than taken from a crate that can lag
//! behind it. The files are downloaded once into `target/`, checked against the SHA-256 below, and never committed, but
//! `GraphemeBreakTest.txt`, which the tests of `types` read.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

pub const VERSION: (u8, u8, u8) = (18, 0, 0);

/// The files of the database, by their path under `https://www.unicode.org/Public/<version>/ucd/`, with their SHA-256.
const FILES: [(&str, &str); 5] = [
    ("auxiliary/GraphemeBreakProperty.txt", "0839dcb79e4ac639ecd538b1abf7c9d22e3f9dd265b7e182d33627aa4d75b45a"),
    ("auxiliary/GraphemeBreakTest.txt", "b0cf047ee94485bbdc846de2b902f5f8a815f6b674f9d04223cddadd91c9df31"),
    ("DerivedCoreProperties.txt", "09c928886a178fcafd93c29e4bd59073a058e5a100b716d425cb563ab50f68c9"),
    ("EastAsianWidth.txt", "a0cf29eacd00cfcaec4381c6b7c281685f18dbb4e7ff82b4076ccb342ca839aa"),
    ("emoji/emoji-data.txt", "80d00f8e616a0ef27fd6b8de3b758c06383b5d917e2977709578e68baf733bf1"),
];

const TABLES: &str = "crates/types/src/text/tables.rs";
const BREAK_TEST: &str = "crates/types/tests/data/GraphemeBreakTest.txt";
const LICENSE: &str = "crates/types/LICENSE-Unicode.txt";

/// The values of `Grapheme_Cluster_Break`, in the order of their numbers in the tables (0 is `Other`).
const BREAKS: [&str; 14] =
    ["Other", "CR", "LF", "Control", "Extend", "ZWJ", "Regional_Indicator", "Prepend", "SpacingMark", "L", "V", "T", "LV", "LVT"];
/// The values of `Indic_Conjunct_Break`, from 1 (0 is `None`).
const CONJUNCT: [&str; 3] = ["Linker", "Consonant", "Extend"];

const BREAK_MASK: u16 = 0x0f;
const PICTOGRAPHIC: u16 = 1 << 4;
const CONJUNCT_SHIFT: u16 = 5;
const WIDE: u16 = 1 << 7;
const PRESENTATION: u16 = 1 << 8;

const CODE_POINTS: usize = 0x11_0000;

pub fn generate(root: &Path) -> Result<(), String> {
    let (major, minor, update) = VERSION;
    let dir = root.join("target").join(format!("ucd-{major}.{minor}.{update}"));
    let mut texts = Vec::new();
    for (file, sha) in FILES {
        texts.push(fetch(&dir, file, sha)?);
    }
    let [grapheme, break_test, derived, east_asian, emoji] = texts.try_into().map_err(|_| "unicode: five files".to_owned())?;
    for (file, text) in [("GraphemeBreakProperty", &grapheme), ("DerivedCoreProperties", &derived), ("EastAsianWidth", &east_asian)] {
        let header = format!("# {file}-{major}.{minor}.{update}.txt");
        if !text.starts_with(&header) {
            return Err(format!("unicode: {file}.txt does not start with `{header}`"));
        }
    }
    if !emoji.contains(&format!("# Version: {major}.{minor}.{update}\n")) {
        return Err(format!("unicode: emoji-data.txt is not of version {major}.{minor}.{update}"));
    }

    let mut values = vec![0u16; CODE_POINTS];
    for (range, value) in fields(&grapheme)? {
        let number = BREAKS.iter().position(|name| *name == value).ok_or(format!("unicode: Grapheme_Cluster_Break={value}"))?;
        set(&mut values, range, BREAK_MASK, number as u16);
    }
    for (range, value) in fields(&emoji)? {
        match value.as_str() {
            "Extended_Pictographic" => set(&mut values, range, PICTOGRAPHIC, PICTOGRAPHIC),
            "Emoji_Presentation" => set(&mut values, range, PRESENTATION, PRESENTATION),
            _ => {}
        }
    }
    for (range, value) in fields(&derived)? {
        if let Some(conjunct) = value.strip_prefix("InCB;") {
            let conjunct = conjunct.trim();
            let number = CONJUNCT.iter().position(|name| *name == conjunct).ok_or(format!("unicode: InCB={conjunct}"))?;
            set(&mut values, range, 0b11 << CONJUNCT_SHIFT, (number as u16 + 1) << CONJUNCT_SHIFT);
        }
    }
    // The defaults of `@missing` first, in their order, then the lines of data (UAX #44, section 4.2.10).
    for (range, value) in missing(&east_asian)?.into_iter().chain(fields(&east_asian)?) {
        let wide = if matches!(value.as_str(), "W" | "F") { WIDE } else { 0 };
        set(&mut values, range, WIDE, wide);
    }

    let license = std::fs::read_to_string(root.join(LICENSE)).map_err(|error| format!("{LICENSE}: {error}"))?;
    let tables = render(&values, &license)?;
    write(&root.join(TABLES), tables.as_bytes())?;
    write(&root.join(BREAK_TEST), break_test.as_bytes())?;
    println!("unicode: {TABLES} and {BREAK_TEST} written from the UCD {major}.{minor}.{update}");
    Ok(())
}

/// The file from `target/`, or downloaded with `curl`, which Windows, macOS and Linux all have; refused when its
/// SHA-256 is another.
fn fetch(dir: &Path, file: &str, sha: &str) -> Result<String, String> {
    let name = file.rsplit('/').next().unwrap_or(file);
    let path = dir.join(name);
    if !path.is_file() {
        std::fs::create_dir_all(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
        let (major, minor, update) = VERSION;
        let url = format!("https://www.unicode.org/Public/{major}.{minor}.{update}/ucd/{file}");
        eprintln!("$ curl {url}");
        let status = Command::new("curl")
            .args(["--fail", "--silent", "--show-error", "--location", "--output"])
            .arg(&path)
            .arg(&url)
            .status()
            .map_err(|error| format!("curl: {error}"))?;
        if !status.success() {
            let _ = std::fs::remove_file(&path);
            return Err(format!("curl {url}: {status}"));
        }
    }
    let bytes = std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let got = hex(&Sha256::digest(&bytes));
    if got != sha {
        return Err(format!("{}: SHA-256 {got}, expected {sha}; delete it to download it again", path.display()));
    }
    String::from_utf8(bytes).map_err(|error| format!("{}: {error}", path.display()))
}

fn write(path: &PathBuf, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    }
    std::fs::write(path, bytes).map_err(|error| format!("{}: {error}", path.display()))
}

type Range = (usize, usize);

/// The lines of data, `0600..0605 ; Prepend # Cf …`: the range, and the fields after it up to the comment, joined by
/// `;` without spaces (`InCB;Linker`).
fn fields(text: &str) -> Result<Vec<(Range, String)>, String> {
    let mut lines = Vec::new();
    for line in text.lines() {
        let data = line.split('#').next().unwrap_or_default().trim();
        if data.is_empty() {
            continue;
        }
        let mut parts = data.split(';').map(str::trim);
        let range = range(parts.next().unwrap_or_default())?;
        lines.push((range, parts.collect::<Vec<_>>().join(";")));
    }
    Ok(lines)
}

/// The lines `# @missing: 3400..4DBF; W`.
fn missing(text: &str) -> Result<Vec<(Range, String)>, String> {
    let mut lines = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("# @missing:") {
            let mut parts = rest.split(';').map(str::trim);
            let range = range(parts.next().unwrap_or_default())?;
            lines.push((range, parts.next().unwrap_or_default().to_owned()));
        }
    }
    Ok(lines)
}

fn range(text: &str) -> Result<Range, String> {
    let point = |text: &str| usize::from_str_radix(text, 16).map_err(|error| format!("unicode: `{text}`: {error}"));
    let (start, end) = match text.split_once("..") {
        Some((start, end)) => (point(start)?, point(end)?),
        None => (point(text)?, point(text)?),
    };
    if start > end || end >= CODE_POINTS {
        return Err(format!("unicode: range `{text}`"));
    }
    Ok((start, end))
}

fn set(values: &mut [u16], (start, end): Range, mask: u16, bits: u16) {
    for value in values.iter_mut().take(end + 1).skip(start) {
        *value = (*value & !mask) | bits;
    }
}

fn render(values: &[u16], license: &str) -> Result<String, String> {
    let (major, minor, update) = VERSION;
    let mut out = String::new();
    let w = |error: std::fmt::Error| error.to_string();
    writeln!(out, "//! Generated by `cargo xtask unicode` from the Unicode Character Database {major}.{minor}.{update}: do not edit.").map_err(w)?;
    writeln!(out, "//!").map_err(w)?;
    for (file, sha) in FILES {
        writeln!(out, "//! - `{file}`, SHA-256 `{sha}`").map_err(w)?;
    }
    writeln!(out, "//!").map_err(w)?;
    writeln!(out, "//! The data is the Unicode Consortium's, under this notice (also in `crates/types/LICENSE-Unicode.txt`):").map_err(w)?;
    writeln!(out, "//!").map_err(w)?;
    for line in license.lines() {
        if line.is_empty() { writeln!(out, "//!").map_err(w)? } else { writeln!(out, "//! {line}").map_err(w)? }
    }
    writeln!(out).map_err(w)?;
    writeln!(out, "pub(super) const UNICODE_VERSION: (u8, u8, u8) = ({major}, {minor}, {update});").map_err(w)?;
    writeln!(out).map_err(w)?;
    writeln!(out, "// The bits of a value: `Grapheme_Cluster_Break`, `Extended_Pictographic`, `Indic_Conjunct_Break`, East Asian").map_err(w)?;
    writeln!(out, "// Width Wide or Fullwidth, `Emoji_Presentation`.").map_err(w)?;
    writeln!(out, "pub(super) const BREAK_MASK: u16 = {BREAK_MASK:#06x};").map_err(w)?;
    for (number, name) in BREAKS.iter().enumerate() {
        writeln!(out, "pub(super) const {}: u16 = {number};", constant(name)).map_err(w)?;
    }
    writeln!(out, "pub(super) const PICTOGRAPHIC: u16 = {PICTOGRAPHIC:#06x};").map_err(w)?;
    writeln!(out, "pub(super) const CONJUNCT_SHIFT: u16 = {CONJUNCT_SHIFT};").map_err(w)?;
    for (number, name) in CONJUNCT.iter().enumerate() {
        writeln!(out, "pub(super) const CONJUNCT_{}: u16 = {};", constant(name), number + 1).map_err(w)?;
    }
    writeln!(out, "pub(super) const WIDE: u16 = {WIDE:#06x};").map_err(w)?;
    writeln!(out, "pub(super) const PRESENTATION: u16 = {PRESENTATION:#06x};").map_err(w)?;
    writeln!(out).map_err(w)?;
    let ranges = runs(values);
    writeln!(out, "/// The code points whose value is not 0, as ranges sorted by their first code point, which never overlap.").map_err(w)?;
    writeln!(out, "#[rustfmt::skip]").map_err(w)?;
    writeln!(out, "pub(super) static RANGES: [(u32, u32, u16); {}] = [", ranges.len()).map_err(w)?;
    for chunk in ranges.chunks(6) {
        let line: Vec<String> = chunk.iter().map(|(start, end, value)| format!("({start:#x}, {end:#x}, {value:#x})")).collect();
        writeln!(out, "    {},", line.join(", ")).map_err(w)?;
    }
    writeln!(out, "];").map_err(w)?;
    Ok(out)
}

fn runs(values: &[u16]) -> Vec<(usize, usize, u16)> {
    let mut ranges: Vec<(usize, usize, u16)> = Vec::new();
    for (point, value) in values.iter().copied().enumerate() {
        match ranges.last_mut() {
            Some((_, end, last)) if *last == value && *end + 1 == point => *end = point,
            _ if value == 0 => {}
            _ => ranges.push((point, point, value)),
        }
    }
    ranges
}

fn constant(name: &str) -> String {
    match name {
        "Regional_Indicator" => "REGIONAL".to_owned(),
        "SpacingMark" => "SPACING_MARK".to_owned(),
        other => other.to_ascii_uppercase(),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_gives_its_range_and_its_value() {
        let lines = fields("0600..0605    ; Prepend # Cf   [6] ARABIC\n\n# comment\n06DD          ; Prepend\n");
        assert_eq!(lines, Ok(vec![((0x600, 0x605), "Prepend".to_owned()), ((0x6dd, 0x6dd), "Prepend".to_owned())]));
        let lines = fields("094D          ; InCB; Linker # Mn       DEVANAGARI SIGN VIRAMA\n0041 ; Alphabetic\n");
        assert_eq!(lines, Ok(vec![((0x94d, 0x94d), "InCB;Linker".to_owned()), ((0x41, 0x41), "Alphabetic".to_owned())]));
    }

    #[test]
    fn the_defaults_of_missing_lines_come_in_their_order() {
        let text = "# @missing: 0000..10FFFF; N\n# @missing: 3400..4DBF; W\n3400..4DBF;W\n";
        assert_eq!(missing(text), Ok(vec![((0, 0x10ffff), "N".to_owned()), ((0x3400, 0x4dbf), "W".to_owned())]));
    }

    #[test]
    fn a_range_out_of_the_code_points_is_refused() {
        assert!(range("110000").is_err());
        assert!(range("0042..0041").is_err());
        assert_eq!(range("0041..0042"), Ok((0x41, 0x42)));
    }

    #[test]
    fn runs_join_neighbours_of_one_value_and_leave_out_zero() {
        assert_eq!(runs(&[0, 3, 3, 0, 3, 5, 5]), vec![(1, 2, 3), (4, 4, 3), (5, 6, 5)]);
    }
}
