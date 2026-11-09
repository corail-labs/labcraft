//! `types::text` against what it comes from: the test file of UAX #29 that Unicode publishes with the tables, and the
//! file of the font that the canvas installs (`docs/architecture.md`, section 11.1).

use labcraft_types::text;
use sha2::{Digest, Sha256};

/// Every line of `GraphemeBreakTest.txt` (UCD 18.0.0): `÷ 0061 × 0308 ÷ 0062 ÷`, the expected boundaries given by
/// Unicode, not by this code.
#[test]
fn graphemes_are_those_of_the_test_file_of_unicode() {
    const FILE: &str = include_str!("data/GraphemeBreakTest.txt");
    let (major, minor, update) = text::UNICODE_VERSION;
    assert!(FILE.starts_with(&format!("# GraphemeBreakTest-{major}.{minor}.{update}.txt")), "the file is of another version");
    let mut cases = 0;
    let mut failures = Vec::new();
    for line in FILE.lines() {
        let data = line.split('#').next().unwrap_or_default().trim();
        if data.is_empty() {
            continue;
        }
        let mut expected: Vec<String> = Vec::new();
        let mut current = String::new();
        for token in data.split_whitespace() {
            match token {
                "\u{F7}" => {
                    if !current.is_empty() {
                        expected.push(std::mem::take(&mut current));
                    }
                }
                "\u{D7}" => {}
                hex => {
                    let point = u32::from_str_radix(hex, 16).ok().and_then(char::from_u32);
                    let Some(c) = point else {
                        failures.push(format!("{line}: `{hex}`"));
                        continue;
                    };
                    current.push(c);
                }
            }
        }
        let input: String = expected.concat();
        let got: Vec<&str> = text::graphemes(&input).collect();
        if got != expected {
            failures.push(format!("{line}\n  got {got:?}"));
        }
        cases += 1;
    }
    // The lines that start with `÷`, counted by `grep -c "^÷"`.
    assert_eq!(cases, 853);
    assert!(failures.is_empty(), "{} of {cases} cases fail:\n{}", failures.len(), failures.join("\n"));
}

/// The test file is the one that `cargo xtask unicode` pinned: its SHA-256 is at the top of the tables it wrote.
#[test]
fn the_test_file_of_unicode_is_the_one_the_tables_name() {
    const FILE: &[u8] = include_bytes!("data/GraphemeBreakTest.txt");
    const TABLES: &str = include_str!("../src/text/tables.rs");
    let hex: String = Sha256::digest(FILE).iter().map(|byte| format!("{byte:02x}")).collect();
    assert!(TABLES.contains(&format!("//! - `auxiliary/GraphemeBreakTest.txt`, SHA-256 `{hex}`")), "{hex}");
}

/// The file of the font: its SHA-256, and the metrics read from its tables `head`, `hhea`, `hmtx` and `OS/2`
/// (OpenType 1.9), so that another version of Hack is a reviewed change of the drawing.
#[test]
fn the_font_is_hack_with_the_metrics_of_the_notation() {
    const FONT: &[u8] = include_bytes!("../../../assets/fonts/Hack-Regular.ttf");
    let digest = Sha256::digest(FONT);
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    assert_eq!(hex, text::FONT_SHA256);
    assert!(text::FONT_FILE.ends_with("Hack-Regular.ttf"));

    let font = Font::new(FONT);
    let head = font.table(b"head");
    assert_eq!(font.u16(head + 18), Some(text::UNITS_PER_EM as u16));
    let hhea = font.table(b"hhea");
    assert_eq!(font.u16(hhea + 4), Some(text::ASCENT as u16));
    assert_eq!(font.u16(hhea + 6).map(|descender| -(descender as i16)), Some(text::DESCENT as i16));
    let os2 = font.table(b"OS/2");
    assert_eq!(font.u16(os2 + 88), Some(text::CAP_HEIGHT as u16));
    // Every glyph that advances advances by the same: a monospaced font.
    let metrics = font.u16(hhea + 34).unwrap_or_default();
    let hmtx = font.table(b"hmtx");
    let mut advances: Vec<u16> = (0..usize::from(metrics)).filter_map(|glyph| font.u16(hmtx + 4 * glyph)).collect();
    advances.sort_unstable();
    advances.dedup();
    assert_eq!(advances, [0, text::ADVANCE as u16]);
    // The name of the font says its version (table `name`, record 5).
    let version = "Version 3.003".encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
    assert!(FONT.windows(version.len()).any(|window| window == version), "Version {}", text::FONT_VERSION);
}

struct Font<'a> {
    bytes: &'a [u8],
}

impl<'a> Font<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Font { bytes }
    }

    fn u16(&self, at: usize) -> Option<u16> {
        Some(u16::from_be_bytes([*self.bytes.get(at)?, *self.bytes.get(at + 1)?]))
    }

    fn u32(&self, at: usize) -> Option<u32> {
        Some(u32::from_be_bytes([*self.bytes.get(at)?, *self.bytes.get(at + 1)?, *self.bytes.get(at + 2)?, *self.bytes.get(at + 3)?]))
    }

    /// The offset of a table, from the table directory.
    fn table(&self, tag: &[u8; 4]) -> usize {
        let tables = usize::from(self.u16(4).unwrap_or_default());
        (0..tables)
            .map(|index| 12 + 16 * index)
            .find(|&record| self.bytes.get(record..record + 4) == Some(tag.as_slice()))
            .and_then(|record| self.u32(record + 8))
            .map_or(usize::MAX / 2, |offset| offset as usize)
    }
}
