//! The values of the notation in one place (`docs/visual-notation.md`, 10): sizes, gaps and colours. The drawing reads
//! them here, never as constants of its own, so that a viewer can try other values on a drawing that it draws again
//! (`labcraft --style`, `docs/cli.md`, 8).
//!
//! The values in force are those of the thread, for the time of [`scope`]: the defaults everywhere else, so every export,
//! snapshot and check of the repository draws with the defaults. A file of values is a tool to tune the notation, not a
//! preference of the user: the same file must place its nodes the same way on every screen (notation 2.1).

use std::cell::RefCell;
use std::fmt::Write as _;
use std::sync::Arc;

use crate::theme::TOKENS;

/// What a value counts, and how a file writes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Hundredths of a unit; a file writes units, with at most two decimals.
    Length,
    /// Cells of a text, as `types::text` counts them: two for a wide character.
    Count,
    /// Percent of the zoom.
    Percent,
}

/// A value of the notation: where a file writes it, and what it may be.
#[derive(Clone, Copy, Debug)]
pub struct Field {
    pub section: &'static str,
    pub key: &'static str,
    pub kind: Kind,
    pub default: i64,
    pub min: i64,
    pub max: i64,
    /// The section of the notation that says what it is.
    pub notation: &'static str,
    pub doc: &'static str,
    get: fn(&Numbers) -> i64,
    set: fn(&mut Numbers, i64),
}

impl Field {
    pub fn get(&self, numbers: &Numbers) -> i64 {
        (self.get)(numbers)
    }
}

macro_rules! numbers {
    ($($field:ident: $section:literal . $key:literal = $default:literal, $kind:ident, $min:literal ..= $max:literal, $notation:literal, $doc:literal;)*) => {
        /// The numbers of the notation, each in the unit of its [`Kind`].
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct Numbers {
            $(#[doc = $doc] pub $field: i64,)*
        }

        impl Numbers {
            pub const DEFAULT: Numbers = Numbers { $($field: $default,)* };
        }

        /// Every value, in the order of a file of values.
        pub const FIELDS: &[Field] = &[$(Field {
            section: $section,
            key: $key,
            kind: Kind::$kind,
            default: $default,
            min: $min,
            max: $max,
            notation: $notation,
            doc: $doc,
            get: |numbers| numbers.$field,
            set: |numbers, value| numbers.$field = value,
        },)*];

        $(#[doc = $doc] pub fn $field() -> i64 {
            CURRENT.with_borrow(|style| style.numbers.$field)
        })*
    };
}

numbers! {
    grid: "canvas"."grid" = 800, Length, 400..=1600, "2", "The grid of the canvas: positions and sizes round up to it.";
    margin: "canvas"."margin" = 1600, Length, 0..=6400, "2", "Around what an image shows (`docs/cli.md`, 7.2).";

    large: "text"."large" = 1000, Length, 600..=2400, "2.3", "Titles of structures and of standard nodes, names of constants.";
    label: "text"."label" = 900, Length, 600..=2400, "3.2", "The name on a compact block, the label of an oblong.";
    medium: "text"."medium" = 1000, Length, 600..=2400, "2.3", "Labels, pills, names of wires.";
    small: "text"."small" = 900, Length, 600..=2400, "2.3", "Units of terminals, bands, marks.";
    any_text: "text"."any_text" = 40, Count, 4..=160, "2.3", "A text that no other rule cuts.";
    label_line: "text"."label_line" = 10, Count, 4..=48, "3.2", "A line of the label of an oblong, the pill of a wire.";
    compact_constant: "text"."compact_constant" = 8, Count, 2..=32, "3.2", "An inline constant in compact view.";

    terminal: "node"."terminal" = 600, Length, 400..=4000, "4.1", "The shape of a terminal.";
    pill_height: "node"."pill_height" = 1400, Length, 800..=7200, "3.2", "A pill of an inline constant.";
    band_height: "node"."band_height" = 1200, Length, 600..=4800, "3.2", "The band of a library above a block.";
    padding: "node"."padding" = 400, Length, 0..=1600, "3.2", "Inside a pill, on each side.";
    label_padding: "node"."label_padding" = 200, Length, 0..=3600, "3.2", "Inside an oblong label, on each side.";
    square: "node"."square" = 4000, Length, 1600..=16000, "3.2", "The width of a compact block, and its least height.";
    compact_pitch: "node"."pitch" = 800, Length, 800..=4800, "3.2", "Between two lines of terminals of a compact block.";
    circle: "node"."circle" = 2800, Length, 1200..=11200, "3.2", "The circle of a primitive.";
    icon: "node"."icon" = 2800, Length, 1200..=11200, "3.2", "An icon in a block.";
    outline: "node"."outline" = 100, Length, 0..=400, "3", "The outline of a node.";
    radius: "node"."radius" = 200, Length, 0..=1600, "3", "The corners of a node.";

    standard_header: "standard"."header" = 2400, Length, 1200..=9600, "3.1", "The title of a node in standard view.";
    first_line: "standard"."first_line" = 4000, Length, 1600..=16000, "3.1", "From its top to its first line of terminals.";
    pitch: "standard"."pitch" = 2400, Length, 1200..=9600, "3.1", "Between two lines of terminals.";
    last_line: "standard"."last_line" = 1600, Length, 0..=6400, "3.1", "Below its last line.";
    side: "standard"."side" = 1200, Length, 0..=4800, "3.1", "Between a name of a terminal and the edge.";
    between: "standard"."between" = 2400, Length, 0..=9600, "3.1", "Between the names of an input and an output on a line.";
    standard_width: "standard"."width" = 11200, Length, 4000..=44800, "3.1", "The least width of a node in standard view.";

    stack: "place"."stack" = 2400, Length, 0..=9600, "2.1", "Between nodes stacked in a column.";
    off_axis: "place"."off_axis" = 4000, Length, 0..=16000, "2.1", "Between the error chain and the nodes above it.";
    slot: "place"."slot" = 2400, Length, 1200..=9600, "2.1", "Between two terminals of a frame, and above the error chain.";
    label_start: "place"."label_start" = 1200, Length, 0..=4800, "2.1", "From a terminal of a frame to its label.";
    label_above: "place"."label_above" = 400, Length, 0..=1600, "2.1", "Between the label of a structure and its frame.";

    frame_header: "frame"."header" = 3200, Length, 1600..=12800, "5", "The title of a structure.";
    frame_pad: "frame"."pad" = 1600, Length, 0..=6400, "5", "Inside a frame, on each side.";
    arm_header: "frame"."arm_header" = 2400, Length, 0..=9600, "5.2", "The title of an arm of a `match` or `handle` unfolded.";
    arm_gap: "frame"."arm_gap" = 1600, Length, 0..=6400, "5.2", "Between two arms unfolded.";

    tab_row: "tabs"."row" = 2400, Length, 1200..=9600, "5.2", "The row of tabs of a `match` or `handle` in tabs.";
    tab_height: "tabs"."height" = 2000, Length, 1200..=8000, "5.2", "A tab.";
    tab_gap: "tabs"."gap" = 400, Length, 0..=1600, "5.2", "Between two tabs.";
    tab_arm_header: "tabs"."arm_header" = 800, Length, 0..=3200, "5.2", "Under the row of tabs, above the arm shown.";

    wire_gap: "wire"."gap" = 800, Length, 400..=3200, "2.2", "Between two wires side by side.";
    clear: "wire"."clear" = 1600, Length, 0..=6400, "2.2", "Between a wire and a node it passes.";
    min_channel: "wire"."min_channel" = 1600, Length, 800..=6400, "2.2", "The narrowest channel between two columns.";
    track_room: "wire"."track_room" = 2400, Length, 0..=9600, "4", "Above the highest track: room for the name of its wire.";
    wire_radius: "wire"."radius" = 600, Length, 0..=2400, "2", "The corners of a wire.";
    chevron_step: "wire"."chevron_step" = 800, Length, 400..=9600, "4.1", "Between two chevrons cut into an error wire.";
    name_above: "wire"."name_above" = 300, Length, 0..=1200, "4", "Between a wire and its name.";
    name_side: "wire"."name_side" = 400, Length, 0..=1600, "4", "On each side of the name of a wire.";
    width_data: "wire"."width_data" = 150, Length, 50..=600, "4.1", "A wire of a number, a boolean, a text or an enum.";
    width_struct: "wire"."width_struct" = 400, Length, 300..=1600, "4.1", "A wire of a struct, with a core of the fill under it.";
    width_session: "wire"."width_session" = 250, Length, 50..=1000, "4.1", "A wire of a session.";
    width_error: "wire"."width_error" = 400, Length, 400..=1600, "4.1", "A wire of an error, a tape with its chevrons cut into it.";
    width_order: "wire"."width_order" = 100, Length, 50..=400, "4.1", "A wire of order (`after:`).";
    width_function: "wire"."width_function" = 150, Length, 50..=600, "4.1", "A wire of a reference to a function.";

    mark_line: "marks"."line_pitch" = 2000, Length, 1000..=8000, "4.2", "Between the lines of a function that is not drawn.";
    mark_around: "marks"."around" = 300, Length, 0..=1200, "4.2", "Between a node marked and its mark.";
    mark_width: "marks"."width" = 200, Length, 50..=800, "4.2", "The stroke of a mark.";

    coarse_zoom: "zoom"."coarse" = 40, Percent, 10..=100, "7", "Below it, a node is a plain rectangle.";
    names_zoom: "zoom"."names" = 75, Percent, 10..=200, "7", "Below it, no name of a terminal.";
}

/// A colour, `0xRRGGBB`.
pub type Rgb = u32;

/// The values of the notation: its numbers, and the colours of each token of [`TOKENS`], in its order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Style {
    pub numbers: Numbers,
    /// Light, dark.
    pub colors: Vec<(Rgb, Rgb)>,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            numbers: Numbers::DEFAULT,
            colors: TOKENS.iter().map(|token| (hex(token.light).unwrap_or(0), hex(token.dark).unwrap_or(0))).collect(),
        }
    }
}

thread_local! {
    static CURRENT: RefCell<Arc<Style>> = RefCell::new(Arc::new(Style::default()));
}

/// Runs `work` with `style` in force on this thread, then puts back the values that were.
pub fn scope<R>(style: &Arc<Style>, work: impl FnOnce() -> R) -> R {
    struct Restore(Option<Arc<Style>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            if let Some(before) = self.0.take() {
                CURRENT.with_borrow_mut(|current| *current = before);
            }
        }
    }
    let before = CURRENT.with_borrow_mut(|current| std::mem::replace(current, style.clone()));
    let _restore = Restore(Some(before));
    work()
}

/// The values in force on this thread.
pub fn current() -> Arc<Style> {
    CURRENT.with_borrow(Arc::clone)
}

/// The colour of a token in force, light or dark; `None` for a token that [`TOKENS`] does not name.
pub fn color(token: &str, dark: bool) -> Option<Rgb> {
    let index = TOKENS.iter().position(|value| value.name == token)?;
    CURRENT.with_borrow(|style| style.colors.get(index).map(|(light, night)| if dark { *night } else { *light }))
}

/// A zoom of [`Kind::Percent`] as a factor: 40 gives 0.4.
pub fn factor(percent: i64) -> f32 {
    let percent = i16::try_from(percent.clamp(0, 10_000)).unwrap_or(i16::MAX);
    f32::from(percent) / 100.0
}

/// `#RRGGBB`.
pub fn hex(text: &str) -> Option<Rgb> {
    let digits = text.strip_prefix('#')?;
    if digits.len() != 6 || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(digits, 16).ok()
}

/// `#RRGGBB`, in capitals as notation 8 writes them.
pub fn written(rgb: Rgb) -> String {
    format!("#{:06X}", rgb & 0x00FF_FFFF)
}

/// The most a file of values may hold.
pub const MAX_FILE: usize = 1 << 20;

/// Reads a file of values: a key left out keeps its default; a key unknown, a value of the wrong kind or out of its range
/// is refused, with the reason. The values of a file are tried by a viewer only.
pub fn read(text: &str) -> Result<Style, String> {
    if text.len() > MAX_FILE {
        return Err(format!("a file of values holds at most {MAX_FILE} bytes"));
    }
    let table: toml::Table = toml::from_str(text).map_err(|error| error.message().to_owned())?;
    let mut style = Style::default();
    let mut problems = Vec::new();
    for (section, value) in &table {
        let Some(entries) = value.as_table() else {
            problems.push(format!("`{}` is outside a section", cut(section)));
            continue;
        };
        if section == "colors" {
            colors(entries, &mut style, &mut problems);
            continue;
        }
        if !FIELDS.iter().any(|field| field.section == section) {
            problems.push(format!("no section `[{}]`", cut(section)));
            continue;
        }
        for (key, value) in entries {
            let Some(field) = FIELDS.iter().find(|field| field.section == section && field.key == key) else {
                problems.push(format!("no value `{section}.{}`", cut(key)));
                continue;
            };
            match number(field, value) {
                Ok(number) => (field.set)(&mut style.numbers, number),
                Err(problem) => problems.push(format!("`{section}.{key}`: {problem}")),
            }
        }
    }
    if style.numbers.coarse_zoom > style.numbers.names_zoom {
        problems.push("`zoom.coarse` is above `zoom.names`".to_owned());
    }
    if problems.is_empty() { Ok(style) } else { Err(said(&problems)) }
}

/// How many problems a report names.
const NAMED: usize = 8;

/// The problems of a file, the first [`NAMED`] of them.
fn said(problems: &[String]) -> String {
    let mut text = problems.iter().take(NAMED).cloned().collect::<Vec<_>>().join("; ");
    if problems.len() > NAMED {
        let _ = write!(text, "; and {} more", problems.len() - NAMED);
    }
    text
}

/// A name of the file in a report: its first 40 characters, a control or a mark of direction as U+FFFD, so that a name
/// cannot break the line of a notice or turn it around.
fn cut(name: &str) -> String {
    let shown = |c: char| {
        let direction = matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}');
        if c.is_control() || direction { '\u{FFFD}' } else { c }
    };
    let mut text: String = name.chars().take(40).map(shown).collect();
    if name.chars().nth(40).is_some() {
        text.push('…');
    }
    text
}

fn number(field: &Field, value: &toml::Value) -> Result<i64, String> {
    let (number, written) = match (field.kind, value) {
        (Kind::Length, toml::Value::Integer(units)) => (units.checked_mul(100), units.to_string()),
        (Kind::Length, toml::Value::Float(units)) => {
            let hundredths = units * 100.0;
            if !hundredths.is_finite() || (hundredths - hundredths.round()).abs() > 1e-6 {
                return Err(format!("{units}: units with at most two decimals"));
            }
            #[expect(clippy::cast_possible_truncation, reason = "finite, and within the range checked below")]
            let rounded = hundredths.round().clamp(-1e15, 1e15) as i64;
            (Some(rounded), units.to_string())
        }
        (Kind::Count | Kind::Percent, toml::Value::Integer(count)) => (Some(*count), count.to_string()),
        (Kind::Length, _) => return Err("a number of units".to_owned()),
        (Kind::Count, _) => return Err("a whole number of cells".to_owned()),
        (Kind::Percent, _) => return Err("a whole number of percent".to_owned()),
    };
    match number {
        Some(number) if (field.min..=field.max).contains(&number) => Ok(number),
        _ => Err(format!("{written} is not between {} and {}", shown(field, field.min), shown(field, field.max))),
    }
}

fn colors(entries: &toml::Table, style: &mut Style, problems: &mut Vec<String>) {
    for (theme, value) in entries {
        let dark = match theme.as_str() {
            "light" => false,
            "dark" => true,
            _ => {
                problems.push(format!("no colours `{}`: `[colors.light]` or `[colors.dark]`", cut(theme)));
                continue;
            }
        };
        let Some(tokens) = value.as_table() else {
            problems.push(format!("`colors.{theme}` is a table"));
            continue;
        };
        let mut flat = Vec::new();
        flatten("", tokens, &mut flat);
        let mut given = std::collections::HashSet::new();
        for (token, value) in flat {
            if !given.insert(token.clone()) {
                problems.push(format!("`{}` is given twice in `colors.{theme}`", cut(&token)));
                continue;
            }
            let Some(index) = TOKENS.iter().position(|known| known.name == token) else {
                problems.push(format!("no token `{}`", cut(&token)));
                continue;
            };
            let Some(rgb) = value.as_str().and_then(hex) else {
                problems.push(format!("`{token}`: a colour `#RRGGBB`"));
                continue;
            };
            if let Some(pair) = style.colors.get_mut(index) {
                if dark { pair.1 = rgb } else { pair.0 = rgb }
            }
        }
    }
}

/// `canvas.fill = "…"` without quotes is a table `canvas` in TOML: the key of the token all the same.
fn flatten<'t>(prefix: &str, table: &'t toml::Table, flat: &mut Vec<(String, &'t toml::Value)>) {
    for (key, value) in table {
        let name = if prefix.is_empty() { key.clone() } else { format!("{prefix}.{key}") };
        match value.as_table() {
            Some(inner) if prefix.matches('.').count() < 4 => flatten(&name, inner, flat),
            _ => flat.push((name, value)),
        }
    }
}

/// A value as a file writes it: `24`, `2.5`, `40`.
pub fn shown(field: &Field, value: i64) -> String {
    match field.kind {
        Kind::Length if value % 100 == 0 => (value / 100).to_string(),
        Kind::Length => {
            let text = format!("{}.{:02}", value / 100, (value % 100).abs());
            text.trim_end_matches('0').to_owned()
        }
        Kind::Count | Kind::Percent => value.to_string(),
    }
}

/// A file of values that holds `style`, each value with what it is: what `labcraft --style` writes when its file does not
/// exist yet.
pub fn file_of(style: &Style) -> String {
    let mut text = String::from(
        "# The values of the visual notation (docs/visual-notation.md, 10), read again by `labcraft --style` each time this\n\
         # file is saved. Each value is written commented, at its default: remove the `#` of a line to try another.\n\
         # Lengths in units (the grid is 8), with at most two decimals; text cuts in cells; zooms in percent. A value\n\
         # left out keeps its default, so a default that changes in the code is not held here. This file tunes the\n\
         # notation: it is not a preference.\n",
    );
    let mut section = "";
    for field in FIELDS {
        if field.section != section {
            section = field.section;
            let _ = write!(text, "\n[{section}]\n");
        }
        let _ = writeln!(text, "# {} (notation {}, {} to {})", field.doc, field.notation, shown(field, field.min), shown(field, field.max));
        let _ = writeln!(text, "# {} = {}", field.key, shown(field, field.get(&style.numbers)));
    }
    for (theme, dark) in [("light", false), ("dark", true)] {
        let _ = write!(text, "\n[colors.{theme}]\n");
        for (token, (light, night)) in TOKENS.iter().zip(&style.colors) {
            let _ = writeln!(text, "# \"{}\" = \"{}\"", token.name, written(if dark { *night } else { *light }));
        }
    }
    text
}

/// What a style breaks of notation 8: each pair of tokens below its contrast, in either theme.
pub fn contrast_problems(style: &Style) -> Vec<String> {
    let get = |name: &str, dark: bool| {
        let index = TOKENS.iter().position(|token| token.name == name)?;
        style.colors.get(index).map(|(light, night)| if dark { *night } else { *light })
    };
    let mut problems = Vec::new();
    for dark in [false, true] {
        for (ink, background, least) in crate::theme::contrasts() {
            let (Some(a), Some(b)) = (get(ink, dark), get(background, dark)) else { continue };
            let ratio = crate::theme::contrast(a, b);
            if ratio < least {
                let theme = if dark { "dark" } else { "light" };
                problems.push(format!("`{ink}` on `{background}` ({theme}): {ratio:.2}:1, below {least}:1"));
            }
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_of_the_defaults_reads_as_the_defaults() {
        let text = file_of(&Style::default());
        assert_eq!(read(&text), Ok(Style::default()));
        assert!(text.contains("\n[place]\n"), "{text}");
        assert!(text.contains("\n# Between nodes stacked in a column. (notation 2.1, 0 to 96)\n# stack = 24\n"), "{text}");
        assert!(text.contains("\n# width_data = 1.5\n"), "{text}");
        assert!(text.contains("\n# \"wire.error\" = \"#AD3A0B\"\n"), "{text}");
        // Every value is commented: a file of other values says the same, a default changed in the code is not held.
        assert!(text.lines().all(|line| line.is_empty() || line.starts_with('#') || line.starts_with('[')), "{text}");
        let wide = Style { numbers: Numbers { stack: 4800, ..Numbers::DEFAULT }, ..Style::default() };
        assert_eq!(read(&file_of(&wide)), Ok(Style::default()));
        // A line opened is a value tried.
        assert_eq!(read(&text.replace("# stack = 24", "stack = 30")).map(|style| style.numbers.stack), Ok(3000));
    }

    #[test]
    fn a_value_left_out_keeps_its_default() {
        let style = read("[place]\nstack = 30\n\n[wire]\nwidth_error = 4.25\n\n[zoom]\nnames = 60\n").unwrap();
        assert_eq!(style.numbers.stack, 3000);
        assert_eq!(style.numbers.width_error, 425);
        assert_eq!(style.numbers.names_zoom, 60);
        assert_eq!(style.numbers.grid, 800);
        assert_eq!(style.colors, Style::default().colors);
        assert_eq!(read(""), Ok(Style::default()));
    }

    #[test]
    fn a_token_is_written_quoted_or_dotted() {
        let quoted = read("[colors.dark]\n\"wire.error\" = \"#102030\"\n").unwrap();
        let dotted = read("[colors.dark]\nwire.error = \"#102030\"\n").unwrap();
        assert_eq!(quoted, dotted);
        let index = TOKENS.iter().position(|token| token.name == "wire.error").unwrap();
        assert_eq!(quoted.colors[index], (0xAD3A0B, 0x102030));
        let deep = read("[colors.light]\nframe.fill.3 = \"#ABCDEF\"\n").unwrap();
        let index = TOKENS.iter().position(|token| token.name == "frame.fill.3").unwrap();
        assert_eq!(deep.colors[index].0, 0xABCDEF);
    }

    #[test]
    fn what_is_wrong_in_a_file_is_said_and_nothing_of_it_is_taken() {
        let said = |text: &str| read(text).unwrap_err();
        assert_eq!(said("[place]\nstak = 30\n"), "no value `place.stak`");
        assert_eq!(said("[plac]\nstack = 30\n"), "no section `[plac]`");
        assert_eq!(said("[place]\nstack = 2.125\n"), "`place.stack`: 2.125: units with at most two decimals");
        assert_eq!(said("[place]\nstack = 500\n"), "`place.stack`: 500 is not between 0 and 96");
        assert_eq!(said("[canvas]\ngrid = 0\n"), "`canvas.grid`: 0 is not between 4 and 16");
        assert_eq!(said("[place]\nstack = \"24\"\n"), "`place.stack`: a number of units");
        assert_eq!(said("[text]\nany_text = 4.5\n"), "`text.any_text`: a whole number of cells");
        assert_eq!(said("[place]\nstack = 9223372036854775807\n"), "`place.stack`: 9223372036854775807 is not between 0 and 96");
        assert_eq!(said("[place]\nstack = inf\n"), "`place.stack`: inf: units with at most two decimals");
        assert_eq!(said("[colors.light]\n\"wire.eror\" = \"#000000\"\n"), "no token `wire.eror`");
        assert_eq!(said("[colors.light]\n\"wire.error\" = \"red\"\n"), "`wire.error`: a colour `#RRGGBB`");
        assert_eq!(said("[colors.dim]\n"), "no colours `dim`: `[colors.light]` or `[colors.dark]`");
        assert_eq!(said("stack = 30\n"), "`stack` is outside a section");
        assert_eq!(said("[place]\nstack = 30\nslot = -1\n[x]\n"), "`place.slot`: -1 is not between 12 and 96; no section `[x]`");
        assert_eq!(said("[zoom]\ncoarse = 80\n"), "`zoom.coarse` is above `zoom.names`");
        assert_eq!(said("[colors.dark]\n\"wire.error\" = \"#102030\"\nwire.error = \"#203040\"\n"), "`wire.error` is given twice in `colors.dark`");
        // A report names 8 problems, each name cut at 40 characters.
        let mut many = String::from(
            "[place]
",
        );
        for n in 0..20 {
            many.push_str(&format!(
                "k{n:02}{} = 1
",
                "x".repeat(100)
            ));
        }
        let report = said(&many);
        assert!(report.ends_with("; and 12 more"), "{report}");
        assert_eq!(report.matches("no value").count(), 8);
        assert!(report.len() < 8 * 80, "{report}");
        assert!(report.starts_with(&format!("no value `place.k00{}…`", "x".repeat(37))), "{report}");
        assert_eq!(said("[place]\n\"a\\nb\\u202Ec\" = 1\n"), "no value `place.a\u{FFFD}b\u{FFFD}c`");
        assert!(read("[place\n").is_err());
        assert!(read(&" ".repeat(MAX_FILE + 1)).is_err());
    }

    #[test]
    fn a_scope_holds_on_its_thread_only_and_ends_with_its_work() {
        let wide = Arc::new(Style { numbers: Numbers { stack: 4800, ..Numbers::DEFAULT }, ..Style::default() });
        assert_eq!(stack(), 2400);
        let inside = scope(&wide, || {
            let other = std::thread::spawn(stack).join().unwrap();
            (stack(), other, scope(&Arc::new(Style::default()), stack), stack())
        });
        assert_eq!(inside, (4800, 2400, 2400, 4800));
        assert_eq!(stack(), 2400);
        let _ = std::panic::catch_unwind(|| scope(&wide, || panic!("in the work")));
        assert_eq!(stack(), 2400);
    }

    #[test]
    fn a_colour_below_the_contrast_of_notation_8_is_said() {
        let mut style = Style::default();
        let index = TOKENS.iter().position(|token| token.name == "wire.error").unwrap();
        style.colors[index].0 = 0xF0F0F0;
        let problems = contrast_problems(&style);
        assert!(problems.contains(&"`wire.error` on `canvas.fill` (light): 1.06:1, below 4.5:1".to_owned()), "{problems:?}");
        assert!(problems.iter().all(|problem| problem.contains("(light)")), "{problems:?}");
    }

    /// Notation 10 lists every value with its default and its section: another value is a reviewed change of both.
    #[test]
    fn the_values_are_those_of_the_notation() {
        const NOTATION: &str = include_str!("../../../docs/visual-notation.md");
        let section = NOTATION.split("## 10. The values in one place").nth(1).unwrap();
        let rows: Vec<String> = section.lines().filter(|line| line.starts_with("| `")).map(str::to_owned).collect();
        let unit = |kind: Kind| match kind {
            Kind::Length => "units",
            Kind::Count => "cells",
            Kind::Percent => "%",
        };
        let ours: Vec<String> = FIELDS
            .iter()
            .map(|field| {
                format!(
                    "| `{}.{}` | {} {} | {} to {} | {} | {} |",
                    field.section,
                    field.key,
                    shown(field, field.default),
                    unit(field.kind),
                    shown(field, field.min),
                    shown(field, field.max),
                    field.notation,
                    field.doc
                )
            })
            .collect();
        assert_eq!(ours, rows);
    }

    #[test]
    fn the_keys_are_unique_and_the_defaults_within_their_range() {
        for (at, field) in FIELDS.iter().enumerate() {
            assert!((field.min..=field.max).contains(&field.default), "{}.{}", field.section, field.key);
            assert!(!FIELDS[..at].iter().any(|other| other.section == field.section && other.key == field.key), "{}.{}", field.section, field.key);
        }
        assert_eq!(factor(40), 0.4);
        assert_eq!(factor(75), 0.75);
        assert_eq!(shown(&FIELDS[0], 850), "8.5");
        assert_eq!(shown(&FIELDS[0], 805), "8.05");
    }
}
