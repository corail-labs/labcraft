//! The `[mask]` table of the manifest of a library (spec 11.3): how the blocks of its functions look, read for every
//! library. A field that the mask does not know, or a value that it refuses, is a warning (`LC0107`) on its line of the
//! manifest, and the field keeps its default; the items of its `outline` are judged as those of a section, each warning
//! on its line of the manifest.

use labcraft_format::block::Colours;
use labcraft_graph::{BandPlace, ColourRole, Id, Outline, Rgb, SourceMap};
use labcraft_types::{Code, Interner, Position, Span, cut_text};
use toml::de::{DeTable, DeValue};

/// The longest `band`, in code points (decision of the owner, 2026-11-20): beyond, a warning.
pub const MAX_BAND: usize = 64;

/// What a mask says, each field `None` when it says nothing of it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mask {
    /// The text of the band, in place of the name of the library.
    pub band: Option<String>,
    pub band_place: Option<BandPlace>,
    /// Its `color`, `fill` and `stroke`.
    pub colours: Colours,
    pub outline: Option<MaskOutline>,
}

/// The `outline` of a mask, and where its items are in the manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaskOutline {
    pub outline: Outline,
    /// Where `read_outline_mapped` put each item.
    map: SourceMap,
    placed: Placed,
}

impl MaskOutline {
    /// Where an item of the outline is in the manifest.
    pub fn span(&self, id: Id) -> Option<Span> {
        self.placed.span(self.map.span(id))
    }
}

/// How the places that `read_outline_mapped` gives become those of the manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Placed {
    /// Added to each line, and to the columns of the first line of the items.
    lines: i64,
    first_columns: i64,
    /// Where a string that cannot be mapped line by line (one with escapes) puts each item: on its key.
    key: Span,
    mapped: bool,
}

impl Placed {
    /// `raw` is the string as the manifest writes it, quotes included, from `start`.
    fn of(lines: &Lines, (raw, start): (&str, usize), key: std::ops::Range<usize>) -> Placed {
        let key = lines.span(key);
        // A literal string reads as written; a basic one too, without a `\`. Either drops a newline right after its opening.
        let quote = ["'''", "\"\"\"", "'", "\""].into_iter().find(|quote| raw.starts_with(quote)).unwrap_or("");
        let mapped = !quote.is_empty() && (quote.starts_with('\'') || !raw.contains('\\'));
        let mut content = start + quote.len();
        let after = raw.get(quote.len()..).unwrap_or_default();
        if quote.len() == 3 {
            if after.starts_with("\r\n") {
                content += 2;
            } else if after.starts_with('\n') {
                content += 1;
            }
        }
        let at = lines.position(content);
        let first = i64::from(labcraft_format::OUTLINE_FIRST_LINE);
        Placed { lines: i64::from(at.line) - first, first_columns: i64::from(at.column) - 1, key, mapped }
    }

    /// Where a place of the items is in the manifest: on the key when it has none, or when the string cannot be mapped.
    fn span(&self, span: Option<Span>) -> Option<Span> {
        let Some(span) = span.filter(|_| self.mapped) else { return Some(self.key) };
        let first = labcraft_format::OUTLINE_FIRST_LINE;
        let moved = |position: Position| {
            let column = if position.line == first { shift(position.column, self.first_columns) } else { position.column };
            Position { line: shift(position.line, self.lines), column }
        };
        Some(Span { start: moved(span.start), end: moved(span.end) })
    }
}

fn shift(value: u32, by: i64) -> u32 {
    u32::try_from(i64::from(value).saturating_add(by).max(1)).unwrap_or(u32::MAX)
}

/// Why a mask refuses this `band` (11.3): one line, not empty, of [`MAX_BAND`] characters at most.
pub fn band_refused(band: &str) -> Option<String> {
    if band.chars().any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}')) {
        Some("`band` is one line, without a control character".to_owned())
    } else if band.trim().is_empty() {
        Some("`band` is empty: `band_place = \"hidden\"` hides the band".to_owned())
    } else if band.chars().count() > MAX_BAND {
        Some(format!("`band` has more than {MAX_BAND} characters"))
    } else {
        None
    }
}

/// What a mask says that it cannot, and where in the manifest.
pub type MaskProblem = (Code, String, Option<Span>);

impl Mask {
    /// The mask of a manifest that reads as TOML: `None` of the table when it has none.
    pub fn read(source: &str, names: &mut Interner) -> (Mask, Vec<MaskProblem>) {
        let mut mask = Mask::default();
        let mut problems = Vec::new();
        let Ok(root) = DeTable::parse(source) else { return (mask, problems) };
        let lines = Lines::of(source);
        let Some((key, value)) = root.get_ref().iter().find(|(key, _)| key.get_ref().as_ref() == "mask") else { return (mask, problems) };
        let DeValue::Table(table) = value.get_ref() else {
            let message = format!("`mask` must be a table, and is {}; the blocks keep their defaults (11.3)", value.get_ref().type_str());
            problems.push((Code::Lc0107, message, Some(lines.span(key.span()))));
            return (mask, problems);
        };
        for (key, value) in table {
            let field = key.get_ref().as_ref();
            let at = Some(lines.span(key.span()));
            let mut refuse = |why: String| problems.push((Code::Lc0107, format!("{why}; the field keeps its default (11.3)"), at));
            let text = value.get_ref().as_str();
            match field {
                "band" => match text.map(|band| (band, band_refused(band))) {
                    None => refuse(format!("`band` must be a string, and is {}", value.get_ref().type_str())),
                    Some((_, Some(why))) => refuse(why),
                    Some((band, None)) => mask.band = Some(band.to_owned()),
                },
                "band_place" => match text.and_then(BandPlace::from_word) {
                    Some(place) => mask.band_place = Some(place),
                    None => refuse(format!("`band_place` is `inside`, `above` or `hidden`, and is {}", shown(value.get_ref()))),
                },
                "color" | "fill" | "stroke" => match (ColourRole::from_word(field), text.and_then(Rgb::parse)) {
                    (Some(role), Some(colour)) => *mask.colours.slot(role) = Some(colour),
                    _ => refuse(format!("`{field}` is a colour `\"#RRGGBB\"`, and is {}", shown(value.get_ref()))),
                },
                "outline" => match text {
                    None => refuse(format!("`outline` must be a string, and is {}", value.get_ref().type_str())),
                    Some(items) => {
                        let raw = (source.get(value.span()).unwrap_or_default(), value.span().start);
                        let placed = Placed::of(&lines, raw, key.span());
                        match labcraft_format::read_outline_mapped(items, names) {
                            // On the line of the item that does not read.
                            Err((why, span)) => {
                                let message =
                                    format!("the `outline` does not read as an `outline` section: {why}; the field keeps its default (11.3)");
                                problems.push((Code::Lc0107, message, placed.span(span)));
                            }
                            Ok((outline, map)) => mask.outline = MaskOutline { outline, map, placed }.judged(names, &mut problems),
                        }
                    }
                },
                _ => {
                    let message = format!(
                        "the mask has no field `{}`: it knows `band`, `band_place`, `color`, `fill`, `stroke` and `outline`, and ignores it (11.3)",
                        cut_text(field)
                    );
                    problems.push((Code::Lc0107, message, at));
                }
            }
        }
        // In the order of the manifest, whatever the order in which the table gives its keys.
        problems.sort_by_key(|(_, _, span)| span.map(|span| (span.start.line, span.start.column)));
        (mask, problems)
    }

    /// The outline of a function (11.4): its own section, else that of the mask.
    pub fn outline_of<'a>(&'a self, own: Option<&'a Outline>) -> Option<&'a Outline> {
        own.or(self.outline.as_ref().map(|mask| &mask.outline))
    }

    /// Says nothing of the blocks.
    pub fn says_nothing_of_blocks(&self) -> bool {
        self.band_place.is_none() && self.colours == Colours::default() && self.outline.is_none()
    }
}

impl MaskOutline {
    /// Its items judged as those of a section, each problem on its line of the manifest; `None` when one is an error of
    /// a section (`LC5016`), which the mask refuses (`LC0107`): the field keeps its default.
    fn judged(self, names: &Interner, problems: &mut Vec<MaskProblem>) -> Option<MaskOutline> {
        let (_, found) = crate::outline::read(&self.outline, names);
        let mut refused = false;
        for (code, message, id) in found {
            if code == Code::Lc5016 {
                refused = true;
                problems.push((Code::Lc0107, format!("{message}; the mask keeps no outline"), self.span(id)));
            } else {
                problems.push((code, message, self.span(id)));
            }
        }
        (!refused).then_some(self)
    }
}

/// A value as a diagnostic quotes it.
fn shown(value: &DeValue<'_>) -> String {
    match value.as_str() {
        Some(text) => format!("`{}`", cut_text(text)),
        None => value.type_str().to_owned(),
    }
}

/// Where each line of a text starts, to turn a byte offset into a line and a column of code points.
struct Lines<'t> {
    text: &'t str,
    starts: Vec<usize>,
}

impl<'t> Lines<'t> {
    fn of(text: &'t str) -> Lines<'t> {
        let starts = std::iter::once(0).chain(text.match_indices('\n').map(|(at, _)| at + 1)).collect();
        Lines { text, starts }
    }

    fn position(&self, offset: usize) -> Position {
        let line = self.starts.partition_point(|start| *start <= offset).max(1);
        let start = self.starts.get(line - 1).copied().unwrap_or(0);
        let column = self.text.get(start..offset.min(self.text.len())).map_or(0, |before| before.chars().count());
        Position { line: u32::try_from(line).unwrap_or(u32::MAX), column: u32::try_from(column + 1).unwrap_or(u32::MAX) }
    }

    fn span(&self, range: std::ops::Range<usize>) -> Span {
        Span { start: self.position(range.start), end: self.position(range.end) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(source: &str) -> (Mask, Vec<(Code, u32, u32)>) {
        let (mask, problems) = Mask::read(source, &mut Interner::new());
        (mask, problems.into_iter().map(|(code, _, span)| (code, span.map_or(0, |s| s.start.line), span.map_or(0, |s| s.start.column))).collect())
    }

    #[test]
    fn each_field_is_read() {
        let (mask, problems) =
            read("[mask]\nband = \"DMM\"\nband_place = \"above\"\ncolor = \"#2f6F9F\"\nfill = \"#000000\"\nstroke = \"#FFFFFF\"\n");
        assert_eq!(problems, []);
        assert_eq!(mask.band.as_deref(), Some("DMM"));
        assert_eq!(mask.band_place, Some(BandPlace::Above));
        assert_eq!(mask.colours.color, Some(Rgb([0x2F, 0x6F, 0x9F])));
        assert_eq!(mask.colours.fill, Some(Rgb([0, 0, 0])));
        assert_eq!(mask.colours.stroke, Some(Rgb([255, 255, 255])));
        assert!(mask.outline.is_none());
        assert!(!mask.says_nothing_of_blocks());
    }

    #[test]
    fn a_manifest_without_a_mask_or_that_is_not_toml_says_nothing() {
        for source in ["", "name = \"x\"\n", "nonsense = = [", "[mask\nband = 1"] {
            let (mask, problems) = read(source);
            assert_eq!(problems, [], "{source}");
            assert_eq!(mask, Mask::default(), "{source}");
            assert!(mask.says_nothing_of_blocks());
        }
    }

    #[test]
    fn a_refused_value_is_a_warning_on_the_line_and_column_1_of_its_key() {
        // Lines counted from 1: `[mask]` is 1, so the keys are on 2, 3, 4, 5 and 6.
        let (mask, problems) = read("[mask]\nband = \"\"\nband_place = \"left\"\ncolor = \"#12345\"\nfill = 3\nstroke = \"red\"\n");
        assert_eq!(problems, [(Code::Lc0107, 2, 1), (Code::Lc0107, 3, 1), (Code::Lc0107, 4, 1), (Code::Lc0107, 5, 1), (Code::Lc0107, 6, 1)]);
        assert!(mask.says_nothing_of_blocks() && mask.band.is_none(), "every field keeps its default");
    }

    #[test]
    fn a_band_that_is_too_long_or_holds_a_line_break_is_refused() {
        let long = "x".repeat(65);
        let exact = "y".repeat(64);
        let (mask, problems) = read(&format!("[mask]\nband = \"{long}\"\n"));
        assert_eq!((problems, mask.band), (vec![(Code::Lc0107, 2, 1)], None));
        let (mask, problems) = read(&format!("[mask]\nband = \"{exact}\"\n"));
        assert_eq!((problems, mask.band), (vec![], Some(exact)));
        // 64 code points that are more than 64 bytes are accepted: the limit counts characters.
        let accents = "é".repeat(64);
        let (mask, problems) = read(&format!("[mask]\nband = \"{accents}\"\n"));
        assert_eq!((problems, mask.band), (vec![], Some(accents)));
        let (mask, problems) = read("[mask]\nband = \"\"\"a\nb\"\"\"\n");
        assert_eq!((problems, mask.band), (vec![(Code::Lc0107, 2, 1)], None));
        let (mask, problems) = read("[mask]\nband = \"   \"\n");
        assert_eq!((problems, mask.band), (vec![(Code::Lc0107, 2, 1)], None));
    }

    #[test]
    fn the_column_of_a_key_counts_code_points_not_bytes() {
        // An inline table: `mask = { band = "éé", ` is 22 code points (24 bytes) before `nonsense`, at column 23.
        let (_, problems) = read("mask = { band = \"éé\", nonsense = 1 }\n");
        assert_eq!(problems, [(Code::Lc0107, 1, 23)]);
    }

    #[test]
    fn an_unknown_field_is_a_warning_and_the_known_ones_still_read() {
        let (mask, mut problems) = read("[mask]\nband = \"A\"\nnonsense = 1\ncolour = \"#000000\"\n");
        problems.sort_by_key(|problem| problem.1);
        assert_eq!(problems, [(Code::Lc0107, 3, 1), (Code::Lc0107, 4, 1)]);
        assert_eq!(mask.band.as_deref(), Some("A"));
        assert!(mask.colours.color.is_none());
    }

    #[test]
    fn a_mask_that_is_not_a_table_is_a_warning_on_its_key() {
        let (mask, problems) = read("name = \"x\"\nmask = 1\n");
        assert_eq!(problems, [(Code::Lc0107, 2, 1)]);
        assert!(mask.says_nothing_of_blocks());
    }

    #[test]
    fn an_outline_that_does_not_read_is_a_warning_on_its_place_and_keeps_none() {
        let (mask, problems) = read("[mask]\nband = \"A\"\noutline = \"template zz\"\n");
        // An unknown template is an error of the section: said on its item, at column 12 (`outline = "` is 11 characters).
        assert_eq!(problems, [(Code::Lc0107, 3, 12)]);
        assert!(mask.outline.is_none());
        let (mask, problems) = read("[mask]\noutline = 5\n");
        assert_eq!(problems, [(Code::Lc0107, 2, 1)]);
        assert!(mask.outline.is_none());
        // Items that do not read: on the place of the first diagnostic, the start of the items here (column 12).
        let (mask, problems) = read("[mask]\noutline = \"not an item at all\"\n");
        assert_eq!(problems, [(Code::Lc0107, 2, 12)]);
        assert!(mask.outline.is_none());
        // On its line in a multi-line string: the second item, line 4 of the manifest.
        let (_, problems) = read("[mask]\noutline = '''\ntemplate d\n  nonsense here\n'''\n");
        assert_eq!(problems.iter().map(|problem| problem.1).collect::<Vec<_>>(), [4]);
    }

    #[test]
    fn an_outline_that_is_an_error_as_a_section_is_a_warning_on_its_item_and_keeps_none() {
        // Lines of the manifest: 1 `[mask]`, 2 `outline = '''`, 3 and 4 the items, 5 `'''`. Two `path` items are an error
        // (11.4), said on the second item, line 4.
        let source = "[mask]\noutline = '''\npath (0, 0) (w, 0) (w, h)\npath (0, 0) (w, 0) (w, h)\n'''\n";
        let (mask, problems) = read(source);
        assert_eq!(problems.len(), 1, "{problems:?}");
        let (code, line, _) = problems[0];
        assert_eq!((code, line), (Code::Lc0107, 4));
        assert!(mask.outline.is_none());
    }

    #[test]
    fn an_outline_in_a_literal_string_keeps_its_lines_of_the_manifest() {
        // The first item is on line 4 after `band` (2) and `outline = '''` (3). A clean outline: said nothing, kept.
        let source =
            "[mask]\nband = \"A\"\noutline = '''\npath (0, 0) (w, 0) round 4 (w, h) (0, h)\nright terminal { path (0, -2) (4, 0) (0, 2) }\n'''\n";
        let (mask, problems) = read(source);
        assert_eq!(problems, []);
        let outline = mask.outline.as_ref();
        assert!(outline.is_some());
        // Its items map onto lines 4 and 5.
        let spans: Vec<u32> = outline.iter().flat_map(|o| o.outline.items.iter()).filter_map(|item| o_span(outline, item.id)).collect();
        assert_eq!(spans, [4, 5]);
    }

    fn o_span(outline: Option<&MaskOutline>, id: Id) -> Option<u32> {
        outline?.span(id).map(|span| span.start.line)
    }

    #[test]
    fn a_guard_of_the_outline_is_not_judged_by_the_mask_alone() {
        // A guard that depends on the terminals (LC5017) is judged for each function, not by `read`.
        let (mask, problems) = read("[mask]\noutline = '''\npath (0, 0) (w, zz) (w, h) (0, h)\n'''\n");
        assert_eq!(problems, []);
        assert!(mask.outline.is_some());
    }
}
