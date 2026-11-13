//! The font of the notation and the counting of cells (`docs/visual-notation.md`, section 2.3; `docs/architecture.md`,
//! section 11.1): low enough for `check`, which judges the text of an icon by its box (spec 11.1), and for `diagram`,
//! which places.
//!
//! Every length of text is counted in cells: a grapheme (UAX #29, extended grapheme clusters) takes one cell, or two
//! when one of its code points is East Asian Wide or Fullwidth or `Emoji_Presentation`, or is U+FE0F; a drawing keeps
//! the first 32 code points of a grapheme. The tables come from the Unicode Character Database of the version that names are read with (spec
//! 14.1), written by `cargo xtask unicode`.

use std::borrow::Cow;

mod tables;

use tables::{
    BREAK_MASK, CONJUNCT_CONSONANT, CONJUNCT_EXTEND, CONJUNCT_LINKER, CONJUNCT_SHIFT, CONTROL, CR, EXTEND, L, LF, LV, LVT, OTHER, PICTOGRAPHIC,
    PREPEND, PRESENTATION, RANGES, REGIONAL, SPACING_MARK, T, V, WIDE, ZWJ,
};

/// The version of Unicode of the tables: that of spec 14.1.
pub const UNICODE_VERSION: (u8, u8, u8) = tables::UNICODE_VERSION;

/// The font of the notation: Hack 3.003 Regular, in `assets/fonts/`, which the canvas installs. A test reads the file
/// and checks its SHA-256 and the metrics below, so that another version of the font is a reviewed change.
pub const FONT_NAME: &str = "Hack";
pub const FONT_VERSION: &str = "3.003";
pub const FONT_FILE: &str = "assets/fonts/Hack-Regular.ttf";
pub const FONT_SHA256: &str = "15f55cc0c85a2988d2b4b3a8cdb5d77fdfbaf319e1bb5309d725db9818fb7125";

/// The metrics of the font, in its units: every character has the same advance.
pub const UNITS_PER_EM: u32 = 2048;
pub const ADVANCE: u32 = 1233;
pub const ASCENT: u32 = 1901;
pub const DESCENT: u32 = 483;
pub const CAP_HEIGHT: u32 = 1493;

/// What ends a cut text, in one cell.
pub const ELLIPSIS: char = '\u{2026}';

/// A length in units of the font at a size, both in hundredths of a unit of the canvas, rounded to the nearest
/// hundredth, a half up: the one rounding of every length that comes from the font. `None` past `i64`.
pub fn scale(font_units: u64, size: u32) -> Option<i64> {
    let product = u128::from(font_units) * u128::from(size);
    let rounded = (product + u128::from(UNITS_PER_EM / 2)) / u128::from(UNITS_PER_EM);
    i64::try_from(rounded).ok()
}

/// The width of a text of `cells` cells at `size`, in hundredths of a unit: 722 for a cell at 12 (1200).
pub fn width(cells: u64, size: u32) -> Option<i64> {
    scale(cells.checked_mul(u64::from(ADVANCE))?, size)
}

/// The graphemes of a text, in its order (UAX #29, extended grapheme clusters).
pub fn graphemes(text: &str) -> Graphemes<'_> {
    Graphemes { rest: text }
}

/// The code points of a grapheme that a drawing keeps *(proposed)*: no real text has more (Unicode's stream-safe form
/// asks for at most 31), and a letter with a million accents cannot stretch a drawing (notation 2.3).
pub const MAX_GRAPHEME: usize = 32;

/// A grapheme as a drawing keeps it: its first [`MAX_GRAPHEME`] code points, without the Prepend characters at their
/// end when it is cut short, since those would join what a cut puts after them (`…`, GB9b).
pub fn drawn(grapheme: &str) -> &str {
    let Some((end, _)) = grapheme.char_indices().nth(MAX_GRAPHEME) else {
        return grapheme;
    };
    let kept = grapheme.get(..end).unwrap_or_default();
    kept.trim_end_matches(|c| value(c) & BREAK_MASK == PREPEND)
}

/// The cells of one grapheme as a drawing keeps it: 2 when one of its code points is East Asian Wide or Fullwidth or
/// `Emoji_Presentation`, or is U+FE0F; 1 otherwise; 0 for an empty text.
pub fn grapheme_cells(grapheme: &str) -> u64 {
    let drawn = drawn(grapheme);
    if drawn.is_empty() {
        0
    } else if drawn.chars().any(|c| c == '\u{FE0F}' || value(c) & (WIDE | PRESENTATION) != 0) {
        2
    } else {
        1
    }
}

/// A count as the documents write it, its thousands grouped: `1000` as `1,000`.
pub fn grouped(number: u64) -> String {
    let digits = number.to_string();
    let mut text = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            text.push(',');
        }
        text.push(digit);
    }
    text
}

/// The cells of a text.
pub fn cells(text: &str) -> u64 {
    graphemes(text).map(grapheme_cells).fold(0, u64::saturating_add)
}

/// The text whole when it holds in `max` cells; otherwise its first graphemes that hold in `max - 1`, and `…`. The cut
/// falls between two graphemes, before the one that would pass the limit, in logical order. A grapheme that a drawing
/// cuts short ([`drawn`]) ends the text too: what it keeps, then `…`, so that the code point it ends on cannot join
/// what followed it. What `cut` gives reads back as the same graphemes and cut again gives itself. It reads no
/// further than the grapheme that passes the limit.
pub fn cut(text: &str, max: u64) -> Cow<'_, str> {
    let mut used: u64 = 0;
    let mut at = 0;
    let mut room_end = 0;
    for grapheme in graphemes(text) {
        let kept = drawn(grapheme);
        let cells = grapheme_cells(grapheme);
        if kept.len() < grapheme.len() {
            let end = if used.saturating_add(cells) < max { at + kept.len() } else { room_end };
            return Cow::Owned(ended(text.get(..end).unwrap_or_default(), max));
        }
        used = used.saturating_add(cells);
        if used > max {
            return Cow::Owned(ended(text.get(..room_end).unwrap_or_default(), max));
        }
        at += grapheme.len();
        if used < max {
            room_end = at;
        }
    }
    Cow::Borrowed(text)
}

fn ended(kept: &str, max: u64) -> String {
    if max == 0 {
        return String::new();
    }
    let mut out = String::with_capacity(kept.len() + 3);
    out.push_str(kept);
    out.push(ELLIPSIS);
    out
}

/// The text whole when it holds in `max` cells; otherwise its start and its end around `…`, the end taking the extra
/// cell (`measu…ment_a` in 12, notation 3.2), the start what the end leaves. It reads the whole text: it serves the
/// names of libraries, which a band shows. A text with a grapheme that a drawing cuts short is cut as [`cut`] does.
pub fn cut_middle(text: &str, max: u64) -> Cow<'_, str> {
    let parts: Vec<(&str, u64)> = graphemes(text).map(|grapheme| (grapheme, grapheme_cells(grapheme))).collect();
    if parts.iter().any(|&(grapheme, _)| drawn(grapheme).len() < grapheme.len()) {
        return cut(text, max);
    }
    let total = parts.iter().map(|&(_, cells)| cells).fold(0, u64::saturating_add);
    if total <= max {
        return Cow::Borrowed(text);
    }
    let Some(room) = max.checked_sub(1) else {
        return Cow::Owned(String::new());
    };
    // The end first, with the larger half; the start takes what the end leaves, so that a wide grapheme at the end
    // does not leave a cell empty (`a…` for `a中` in 2).
    let (mut end, mut end_used) = (parts.len(), 0);
    while let Some(&(_, cells)) = end.checked_sub(1).and_then(|last| parts.get(last)) {
        if end_used + cells > room - room / 2 {
            break;
        }
        (end, end_used) = (end - 1, end_used + cells);
    }
    let (mut start, mut used) = (0, 0);
    while let Some(&(_, cells)) = parts.get(start).filter(|_| start < end) {
        if used + cells > room - end_used {
            break;
        }
        (start, used) = (start + 1, used + cells);
    }
    let mut out = String::new();
    parts.iter().take(start).for_each(|&(grapheme, _)| out.push_str(grapheme));
    out.push(ELLIPSIS);
    parts.iter().skip(end).for_each(|&(grapheme, _)| out.push_str(grapheme));
    Cow::Owned(out)
}

/// The bidirectional controls that never act in a drawing (`LC0005`, spec 2): each is shown as its code.
const BIDI_CONTROLS: [char; 9] = ['\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}', '\u{202E}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}'];

/// What a line break is drawn as, and a tab (notation 2.3).
pub const LINE_BREAK: &str = "\u{21B5}";
pub const TAB: &str = "\u{21E5}";

/// The ink of a piece of text: that of the text, or `mark.invalid` for a bidirectional control (notation 2.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ink {
    Text,
    Invalid,
}

/// A piece of a text as a drawing shows it: a grapheme as the drawing keeps it ([`drawn`]), or the sign of a control.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Piece<'a> {
    pub text: Cow<'a, str>,
    pub cells: u64,
    pub ink: Ink,
    /// A grapheme that the drawing cuts short, which ends the text.
    pub short: bool,
}

/// A control, which the drawing shows by a sign: `↵` for a line break (LF, CR, CRLF), `⇥` for a tab, and its code,
/// `⟨U+0007⟩`, for every other control (general category Cc) and for a bidirectional control of `LC0005`, which
/// alone is in `mark.invalid` (notation 2.3). A control is always a grapheme of its own (GB4, GB5).
fn sign(grapheme: &str) -> Option<(Cow<'static, str>, Ink)> {
    let mut chars = grapheme.chars();
    let (first, second) = (chars.next()?, chars.next());
    match (first, second) {
        ('\r', Some('\n')) | ('\r' | '\n', None) => Some((Cow::Borrowed(LINE_BREAK), Ink::Text)),
        ('\t', None) => Some((Cow::Borrowed(TAB), Ink::Text)),
        (c, None) if BIDI_CONTROLS.contains(&c) => Some((Cow::Owned(code(c)), Ink::Invalid)),
        (c, None) if c.is_control() => Some((Cow::Owned(code(c)), Ink::Text)),
        _ => None,
    }
}

/// `⟨U+202E⟩`.
pub fn code(c: char) -> String {
    format!("\u{27E8}U+{:04X}\u{27E9}", u32::from(c))
}

fn piece(grapheme: &str) -> Piece<'_> {
    if let Some((text, ink)) = sign(grapheme) {
        let cells = graphemes(&text).map(grapheme_cells).fold(0, u64::saturating_add);
        return Piece { text, cells, ink, short: false };
    }
    let kept = drawn(grapheme);
    Piece { text: Cow::Borrowed(kept), cells: grapheme_cells(grapheme), ink: Ink::Text, short: kept.len() < grapheme.len() }
}

/// The pieces of a text, in its logical order.
pub fn pieces(text: &str) -> impl Iterator<Item = Piece<'_>> {
    graphemes(text).map(piece)
}

/// Whether a code point is shown by a sign.
fn signed(c: char) -> bool {
    c.is_control() || BIDI_CONTROLS.contains(&c)
}

/// The text as a drawing shows it, every control by its sign (see [`pieces`]), so that no text can hide how it reads
/// (notation 2.3). U+061C, U+200E and U+200F stay, and act. `cells` of it counts what a drawing shows.
pub fn shown(text: &str) -> Cow<'_, str> {
    if !text.contains(signed) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len() + 16);
    for grapheme in graphemes(text) {
        match sign(grapheme) {
            Some((sign, _)) => out.push_str(&sign),
            None => out.push_str(grapheme),
        }
    }
    Cow::Owned(out)
}

/// A text as a drawing shows it: runs of one ink, and the cells they take.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShownText {
    pub runs: Vec<(Ink, String)>,
    pub cells: u64,
}

impl ShownText {
    fn push(&mut self, ink: Ink, text: &str, cells: u64) {
        self.cells = self.cells.saturating_add(cells);
        match self.runs.last_mut() {
            Some((last, run)) if *last == ink => run.push_str(text),
            _ => self.runs.push((ink, text.to_owned())),
        }
    }

    /// A text with no control in it, already cut: the name of a library in a band.
    pub fn plain(text: &str) -> ShownText {
        let mut shown = ShownText::default();
        if !text.is_empty() {
            shown.push(Ink::Text, text, cells(text));
        }
        shown
    }

    /// The text followed by a text with no control in it, in the ink of the text.
    #[must_use]
    pub fn then(mut self, text: &str) -> ShownText {
        if !text.is_empty() {
            self.push(Ink::Text, text, cells(text));
        }
        self
    }

    /// The text of every run, joined.
    pub fn text(&self) -> String {
        self.runs.iter().map(|(_, run)| run.as_str()).collect()
    }
}

/// The text as a drawing shows it, cut as [`cut`] cuts, on its pieces: whole when it holds in `max` cells, otherwise
/// the pieces that hold in `max - 1`, and `…`. The sign of a control is never split: it is whole, or gone after `…`.
/// It reads no further than the piece that passes the limit.
pub fn show(text: &str, max: u64) -> ShownText {
    let mut kept: Vec<Piece<'_>> = Vec::new();
    let (mut used, mut room): (u64, usize) = (0, 0);
    for piece in pieces(text) {
        if piece.short {
            if used.saturating_add(piece.cells) < max {
                kept.push(piece);
                room = kept.len();
            }
            return ended_pieces(kept.get(..room).unwrap_or(&[]), max);
        }
        used = used.saturating_add(piece.cells);
        if used > max {
            return ended_pieces(kept.get(..room).unwrap_or(&[]), max);
        }
        kept.push(piece);
        if used < max {
            room = kept.len();
        }
    }
    let mut shown = ShownText::default();
    kept.iter().for_each(|piece| shown.push(piece.ink, &piece.text, piece.cells));
    shown
}

fn ended_pieces(kept: &[Piece<'_>], max: u64) -> ShownText {
    let mut shown = ShownText::default();
    if max > 0 {
        kept.iter().for_each(|piece| shown.push(piece.ink, &piece.text, piece.cells));
        shown.push(Ink::Text, "\u{2026}", 1);
    }
    shown
}

/// The value of a code point in the tables.
fn value(c: char) -> u16 {
    let point = u32::from(c);
    if point < 0x80 {
        return match point {
            0x0d => CR,
            0x0a => LF,
            0x00..0x20 | 0x7f => CONTROL,
            _ => OTHER,
        };
    }
    match RANGES.binary_search_by(|&(start, end, _)| {
        if end < point {
            std::cmp::Ordering::Less
        } else if start > point {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    }) {
        Ok(index) => RANGES.get(index).map_or(0, |&(_, _, value)| value),
        Err(_) => 0,
    }
}

fn conjunct(value: u16) -> u16 {
    (value >> CONJUNCT_SHIFT) & 0b11
}

/// The graphemes of a text: see [`graphemes`].
#[derive(Clone, Debug)]
pub struct Graphemes<'a> {
    rest: &'a str,
}

/// What the rules of UAX #29 need to know of the cluster so far.
struct Cluster {
    last: u16,
    /// The cluster ends in `\p{Extended_Pictographic} Extend*` (GB11).
    pictographic: bool,
    /// The cluster ends in `\p{Extended_Pictographic} Extend* ZWJ` (GB11).
    pictographic_zwj: bool,
    /// The cluster ends in an odd number of regional indicators (GB12, GB13).
    odd_regional: bool,
    /// The cluster ends in `\p{InCB=Linker} \p{InCB=Extend}*` (GB9c, as Unicode 18.0 writes it).
    linker: bool,
}

impl Cluster {
    fn new(value: u16) -> Self {
        let mut cluster = Cluster { last: 0, pictographic: false, pictographic_zwj: false, odd_regional: false, linker: false };
        cluster.push(value);
        cluster
    }

    /// Whether the rules break before a code point of this value.
    fn breaks_before(&self, value: u16) -> bool {
        let (before, after) = (self.last, value & BREAK_MASK);
        match (before, after) {
            (CR, LF) => false,                                                  // GB3
            (CONTROL | CR | LF, _) | (_, CONTROL | CR | LF) => true,            // GB4, GB5
            (L, L | V | LV | LVT) | (LV | V, V | T) | (LVT | T, T) => false,    // GB6, GB7, GB8
            (_, EXTEND | ZWJ | SPACING_MARK) | (PREPEND, _) => false,           // GB9, GB9a, GB9b
            _ if self.linker && conjunct(value) == CONJUNCT_CONSONANT => false, // GB9c
            _ if self.pictographic_zwj && value & PICTOGRAPHIC != 0 => false,   // GB11
            (REGIONAL, REGIONAL) => !self.odd_regional,                         // GB12, GB13
            _ => true,                                                          // GB999
        }
    }

    fn push(&mut self, value: u16) {
        let kind = value & BREAK_MASK;
        self.pictographic_zwj = kind == ZWJ && self.pictographic;
        self.pictographic = value & PICTOGRAPHIC != 0 || (kind == EXTEND && self.pictographic);
        self.odd_regional = kind == REGIONAL && !self.odd_regional;
        self.linker = match conjunct(value) {
            CONJUNCT_LINKER => true,
            CONJUNCT_EXTEND => self.linker,
            _ => false,
        };
        self.last = kind;
    }
}

impl<'a> Iterator for Graphemes<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        let mut chars = self.rest.char_indices();
        let (_, first) = chars.next()?;
        let mut cluster = Cluster::new(value(first));
        let mut end = self.rest.len();
        for (at, c) in chars {
            let value = value(c);
            if cluster.breaks_before(value) {
                end = at;
                break;
            }
            cluster.push(value);
        }
        let (grapheme, rest) = self.rest.split_at_checked(end)?;
        self.rest = rest;
        Some(grapheme)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_count_is_grouped_by_thousands() {
        assert_eq!([0, 999, 1000, 1_234_567].map(grouped), ["0", "999", "1,000", "1,234,567"]);
    }

    /// Spec 14.1, version of Unicode: tables of another version change the spec in the same pull request.
    #[test]
    fn the_tables_are_of_the_unicode_of_the_spec() {
        const SPEC: &str = include_str!("../../../docs/language-spec-v0.md");
        let (major, minor, _) = UNICODE_VERSION;
        assert!(SPEC.contains(&format!("Names are read with Unicode {major}.{minor}:")), "Unicode {major}.{minor}");
    }

    /// The numbers of notation 2.3, worked out by hand: 1233/2048 of 12 is 7.2246…, of 11 6.6225…, of 9 5.4184….
    #[test]
    fn a_cell_is_as_wide_as_the_notation_says() {
        assert_eq!(width(1, 1200), Some(722));
        assert_eq!(width(1, 1100), Some(662));
        assert_eq!(width(1, 900), Some(542));
        // 33 cells at 12: 33 × 1233 × 1200 = 48,826,800, and 48,826,800 / 2048 = 23,841.21…
        assert_eq!(width(33, 1200), Some(23_841));
        // 2048 × 1201 / 2048 is exact; 1024 / 2048 of a hundredth is a half, rounded up.
        assert_eq!(scale(2048, 1201), Some(1201));
        assert_eq!(scale(1, 1024), Some(1));
        assert_eq!(scale(1, 1023), Some(0));
        assert_eq!(width(u64::MAX, u32::MAX), None);
        assert_eq!(width(u64::MAX / 1233, u32::MAX), None);
    }

    /// The text of the ASCII range never reaches the tables; they give it the same values.
    #[test]
    fn ascii_takes_the_values_of_the_tables() {
        for point in 0u32..0x80 {
            let from_tables = RANGES.iter().find(|&&(start, end, _)| start <= point && point <= end).map_or(0, |&(_, _, value)| value);
            let c = char::from_u32(point).unwrap_or_default();
            assert_eq!(value(c), from_tables, "U+{point:04X}");
        }
    }

    /// Worked out by hand from the properties named in notation 2.3, looked up in the UCD 18.0 files.
    #[test]
    fn cells_follow_the_notation() {
        let cases: [(&str, u64); 16] = [
            ("", 0),
            ("measurement_channel_voltage_limit", 33),
            ("e\u{301}", 1),                                    // e and a combining acute: one grapheme
            ("\u{4E2D}\u{6587}", 4),                            // 中文: East Asian Wide
            ("\u{FF21}", 2),                                    // Ａ: Fullwidth
            ("\u{AC00}", 2),                                    // 가: Wide
            ("\u{1100}\u{1161}\u{11A8}", 2),                    // a syllable of jamos, L V T: one grapheme, L is Wide
            ("\u{1F44D}", 2),                                   // 👍: Emoji_Presentation
            ("\u{1F44D}\u{1F3FD}", 2),                          // with a skin tone: one grapheme
            ("\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}", 2), // a family joined by ZWJ: one grapheme
            ("\u{1F1EB}\u{1F1F7}\u{1F1E9}", 4),                 // a flag and a lone regional indicator
            ("\u{A9}", 1),                                      // ©: neither Wide nor Emoji_Presentation
            ("\u{A9}\u{FE0F}", 2),                              // ©️: followed by U+FE0F
            ("\u{2122}\u{25B6}", 2),                            // ™ and ▶: one cell each
            ("\r\n", 1),                                        // GB3
            ("\u{5D0}\u{5D1}", 2),                              // Hebrew: one cell each, in logical order
        ];
        for (text, expected) in cases {
            assert_eq!(cells(text), expected, "{text:?}");
        }
    }

    /// Decided after the first review: two cells when any code point of the grapheme is wide, not only its first.
    #[test]
    fn a_wide_code_point_anywhere_makes_two_cells() {
        // ☝ (U+261D) is neither Wide nor Emoji_Presentation; the skin tone U+1F3FD is both.
        assert_eq!(cells("\u{261D}"), 1);
        assert_eq!(cells("\u{261D}\u{1F3FD}"), 2);
        // U+0600, ARABIC NUMBER SIGN, is Prepend: one grapheme with the Wide 中 after it (GB9b).
        assert_eq!(graphemes("\u{600}\u{4E2D}").count(), 1);
        assert_eq!(cells("\u{600}\u{4E2D}"), 2);
    }

    /// A drawing keeps the first 32 code points of a grapheme (notation 2.3).
    #[test]
    fn a_grapheme_keeps_32_code_points() {
        let long = format!("a{}", "\u{301}".repeat(2_000_000));
        assert_eq!(graphemes(&long).count(), 1);
        assert_eq!(cells(&long), 1);
        assert_eq!(drawn(&long).chars().count(), 32);
        // `a` and 31 accents of 2 bytes, then `…` of 3: a grapheme cut short ends the text.
        assert_eq!(cut(&long, 40).len(), 1 + 31 * 2 + 3);
        let text = format!("x{long}y");
        assert_eq!(cut(&text, 40), format!("x{}\u{2026}", drawn(&long)));
        assert_eq!(cut(&text, 3), format!("x{}\u{2026}", drawn(&long)));
        assert_eq!(cut(&text, 2), "x\u{2026}");
        assert_eq!(cut_middle(&text, 12), cut(&text, 12));
        // 32 code points stay whole, borrowed.
        let exact = format!("a{}", "\u{301}".repeat(31));
        assert!(matches!(cut(&exact, 40), Cow::Borrowed(_)));
        assert!(matches!(cut_middle(&exact, 40), Cow::Borrowed(_)));
    }

    /// Found by the targeted review: a grapheme cut at 32 code points can end on one that joins what follows, a virama
    /// (GB9c), a ZWJ (GB11), a Prepend (GB9b), a jamo (GB6). What `cut` gives reads back as the graphemes it kept, and
    /// cut again gives itself.
    #[test]
    fn a_grapheme_cut_short_joins_nothing_after_it() {
        let conjunct = format!("\u{915}{}\u{916}", "\u{94D}\u{915}".repeat(20)); // 2 graphemes: क्क…क्क and ख
        let joined = format!("\u{1F468}{}\u{1F468}", "\u{200D}\u{1F468}".repeat(19)); // 2 graphemes: 👨‍👨… and 👨
        let prepended = format!("{}a", "\u{600}".repeat(40)); // 1 grapheme of 41 code points
        let jamos = format!("{}\u{AC00}", "\u{1100}".repeat(40)); // L × 40, then LV: 1 grapheme
        assert_eq!(graphemes(&conjunct).count(), 2);
        assert_eq!(graphemes(&joined).count(), 2);
        for text in [&conjunct, &joined, &prepended, &jamos] {
            let once = cut(text, 40);
            let kept: Vec<&str> = graphemes(&once).collect();
            assert_eq!(kept.last(), Some(&"\u{2026}"), "{text:?}");
            assert_eq!(kept.len(), if text == &prepended { 1 } else { 2 }, "{text:?}");
            assert_eq!(cut(&once, 40), once, "{text:?}");
            assert_eq!(cells(&once), kept.iter().map(|grapheme| grapheme_cells(grapheme)).sum::<u64>());
        }
        // The Prepend characters at the end of what is kept are left out: here, all of them.
        assert_eq!(drawn(&prepended), "");
        assert_eq!(cut(&prepended, 40), "\u{2026}");
    }

    #[test]
    fn a_text_that_holds_is_whole() {
        assert!(matches!(cut("measurement_channel_voltage_limit", 40), Cow::Borrowed(_)));
        assert!(matches!(cut("bench_s", 8), Cow::Borrowed(_)));
        assert_eq!(cut("abcdefghijkl", 12), "abcdefghijkl");
        assert_eq!(cut("", 0), "");
    }

    /// Notation 3.2 and 2.3: 12 cells whole, beyond that 11 and `…`; 40 cells, beyond that 39 and `…`.
    #[test]
    fn a_long_text_keeps_what_holds_before_the_ellipsis() {
        assert_eq!(cut("abcdefghijklm", 12), "abcdefghijk\u{2026}");
        assert_eq!(cells(&cut("abcdefghijklm", 12)), 12);
        let long = "x".repeat(1_000_000);
        assert_eq!(cut(&long, 40), format!("{}\u{2026}", "x".repeat(39)));
        // A library named in Japanese is cut after 5 characters: 10 cells, and `…` in the eleventh.
        let japanese = "\u{6E2C}\u{5B9A}\u{30E9}\u{30A4}\u{30D6}\u{30E9}\u{30EA}";
        assert_eq!(cut(japanese, 12), "\u{6E2C}\u{5B9A}\u{30E9}\u{30A4}\u{30D6}\u{2026}");
        // A cut never splits a grapheme: the accent stays with its letter.
        assert_eq!(cut("abcdefghije\u{301}xyz", 12), "abcdefghije\u{301}\u{2026}");
        assert_eq!(cut("abc", 2), "a\u{2026}");
        assert_eq!(cut("abc", 1), "\u{2026}");
        assert_eq!(cut("abc", 0), "");
    }

    /// Notation 3.2: `measurement_a` and `measurement_b` give `measu…ment_a` and `measu…ment_b` in 12.
    #[test]
    fn a_cut_in_the_middle_keeps_the_start_and_the_end() {
        assert_eq!(cut_middle("measurement_a", 12), "measu\u{2026}ment_a");
        assert_eq!(cut_middle("measurement_b", 12), "measu\u{2026}ment_b");
        assert!(matches!(cut_middle("measurement", 12), Cow::Borrowed(_)));
        assert_eq!(cut_middle("abcdef", 4), "a\u{2026}ef");
        assert_eq!(cut_middle("abcdef", 1), "\u{2026}");
        assert_eq!(cut_middle("abcdef", 0), "");
        // Wide graphemes: 3 cells of room give 1 at the start (none fits) and 2 at the end.
        assert_eq!(cut_middle("\u{4E00}\u{4E01}\u{4E02}", 4), "\u{2026}\u{4E02}");
        // 1 cell of room: the wide end does not fit, so the start takes it.
        assert_eq!(cut_middle("a\u{4E2D}", 2), "a\u{2026}");
    }

    #[test]
    fn a_bidirectional_control_is_shown_as_its_code() {
        assert_eq!(shown("ab\u{202E}cd"), "ab\u{27E8}U+202E\u{27E9}cd");
        assert!(matches!(shown("a\u{200F}b\u{61C}"), Cow::Borrowed(_)));
        let all: String = BIDI_CONTROLS.iter().collect();
        assert_eq!(shown(&all).matches('\u{27E8}').count(), 9);
    }

    /// Decided at the start of step 2 of M0v (notation 2.3): `↵` for a line break, `⇥` for a tab, the code of every
    /// other control in the ink of the text, and only a bidirectional control in `mark.invalid`. In the UCD 18.0 files,
    /// U+21B5 and U+21E5 are Neutral and not emoji, U+27E8 and U+27E9 Narrow: one cell each, so a code is 8 cells.
    #[test]
    fn a_control_is_shown_by_its_sign() {
        assert_eq!(shown("a\nb\r\nc\rd\te"), "a\u{21B5}b\u{21B5}c\u{21B5}d\u{21E5}e");
        assert_eq!(shown("a\u{7}b\u{7F}c\u{85}"), "a\u{27E8}U+0007\u{27E9}b\u{27E8}U+007F\u{27E9}c\u{27E8}U+0085\u{27E9}");
        assert_eq!(cells(&shown("a\nb")), 3);
        assert_eq!(cells(&shown("\u{7}")), 8);
        assert_eq!(cells(&shown("\u{202E}")), 8);
        // U+2028 is a line separator, not a control: it stays.
        assert!(matches!(shown("a\u{2028}b"), Cow::Borrowed(_)));
        let runs = |text: &str, max: u64| show(text, max).runs;
        assert_eq!(
            runs("a\u{202E}b\u{7}", 40),
            [(Ink::Text, "a".to_owned()), (Ink::Invalid, "\u{27E8}U+202E\u{27E9}".to_owned()), (Ink::Text, "b\u{27E8}U+0007\u{27E9}".to_owned()),]
        );
        assert_eq!(show("a\u{202E}b\u{7}", 40).cells, 18);
    }

    /// A cut never splits a sign: `a` and the 8 cells of a code need 9, then `…`.
    #[test]
    fn a_cut_keeps_a_sign_whole_or_leaves_it_out() {
        assert_eq!(show("a\u{202E}b", 10).text(), "a\u{27E8}U+202E\u{27E9}b");
        assert_eq!(show("a\u{202E}bc", 10).text(), "a\u{27E8}U+202E\u{27E9}\u{2026}");
        assert_eq!(show("a\u{202E}bc", 9).text(), "a\u{2026}");
        assert_eq!(show("a\u{202E}bc", 9).cells, 2);
        assert_eq!(show("ab", 0), ShownText::default());
        // As `cut`, on a text with no control, and as many cells as it shows.
        for (text, max) in [("abcdefghijklm", 12), ("\u{6E2C}\u{5B9A}\u{30E9}\u{30A4}\u{30D6}\u{30E9}\u{30EA}", 12), ("abc", 1), ("", 3)] {
            let shown = show(text, max);
            assert_eq!(shown.text(), cut(text, max), "{text:?}");
            assert_eq!(shown.cells, cells(&shown.text()), "{text:?}");
        }
        let long = format!("x{}", "\u{301}".repeat(100));
        assert_eq!(show(&format!("{long}y"), 40).text(), cut(&format!("{long}y"), 40));
        // It reads no further than the piece that passes the limit.
        let huge = "\u{7}".repeat(1_000_000);
        assert_eq!(show(&huge, 40).cells, 33);
    }
}
