//! The `icon` section (spec 11.1, R-14): a grid written `32`, numbers that are finite, a radius or a size above zero, `path` strings in
//! the SVG path syntax, `accent` tokens of the theme (`LC5008`), and shapes that stay in the grid (`LC5009`). What a shape draws counts,
//! stroke aside: a curve or an arc by its real extent, not by its control points, and a text by its box in the font of the notation
//! (`types::text`): as wide as its cells as a drawing shows them, from the top of its capitals to its descent.

use labcraft_graph::{Icon, IconItem, IconShape, Numeral, NumeralKind, Point};
use labcraft_types::{Code, Interner, cut_text, text};

use super::super::read_escapes;
use super::{Sections, numeral_text};

const GRID: f64 = 32.0;

/// The size of a `text` that does not write one (11.1).
const TEXT_SIZE: f64 = 10.0;

/// What rounding may add to a shape that touches an edge.
const SLACK: f64 = 1e-9;

/// The tokens of the theme that `accent` can name: those of the visual notation, section 8, whose parts are names. A test compares them.
pub(super) const TOKENS: &[&str] = &[
    "badge.fill",
    "badge.text",
    "band.fill",
    "band.text",
    "canvas.fill",
    "canvas.grid",
    "coercion.fill",
    "frame.divider",
    "frame.stroke",
    "ink.faint",
    "ink.muted",
    "mark.coercion",
    "mark.invalid",
    "node.active",
    "node.divider",
    "node.fill",
    "node.stroke",
    "node.title",
    "pill.fill",
    "pill.stroke",
    "pill.text",
    "probe.fill",
    "probe.stroke",
    "select.stroke",
    "tab.fill",
    "wire.data",
    "wire.error",
    "wire.order",
    "wire.resource",
];

/// The most shapes of an icon *(proposed)*: with [`MAX_PATH_SEGMENTS`] for its paths in all, what drawing an icon costs
/// stays within the bounds of scenario 4, from whatever library it comes; beyond, its calls show the label (11.1).
pub const MAX_ICON_SHAPES: usize = 256;

/// The most bytes of the `path` strings of an icon together *(proposed)*: an SVG writes them as they are, at each call, so
/// a few segments of long numbers or of spaces must not cost what 4 MiB do. 4 096 segments of 16 bytes (11.1).
pub const MAX_ICON_PATH_BYTES: usize = 65_536;

pub(super) fn judge(sections: &mut Sections<'_, '_>, icon: &Icon) {
    let grid = sections.text(icon.grid);
    if grid != "32" {
        sections.say(Code::Lc5008, format!("`icon {grid}`: the grid of an icon is written `32` in v0 (11.1)"), icon.id, Vec::new());
    }
    if icon.items.len() > MAX_ICON_SHAPES {
        let message = format!("an icon of {} shapes: at most {MAX_ICON_SHAPES}, or its calls show the label (11.1)", icon.items.len());
        sections.say(Code::Lc5008, message, icon.id, Vec::new());
        // Its shapes are not judged one by one: the icon is not drawn, and judging them could cost far more than drawing.
        return;
    }
    let bytes = path_bytes(icon);
    if bytes > MAX_ICON_PATH_BYTES {
        let message = format!("paths of {bytes} bytes in all: at most {MAX_ICON_PATH_BYTES}, or the calls show the label (11.1)");
        sections.say(Code::Lc5008, message, icon.id, Vec::new());
        // Its paths are not read: the icon is not drawn.
        return;
    }
    let segments = segments_in_all(icon);
    if segments > MAX_PATH_SEGMENTS {
        let message = format!("paths of {segments} segments in all: at most {MAX_PATH_SEGMENTS}, or the calls show the label (11.1)");
        sections.say(Code::Lc5008, message, icon.id, Vec::new());
    }
    for item in &icon.items {
        judge_item(sections, item);
    }
}

/// Why the calls to a function show its label instead of its icon (spec 11.1): what `judge` says with `LC5008`, a grid,
/// a number, a path, an accent, or more than [`MAX_ICON_SHAPES`] shapes, [`MAX_ICON_PATH_BYTES`] bytes or
/// [`MAX_PATH_SEGMENTS`] segments of path in all. `None` when the icon is drawn. A library that is not checked is never judged: its icons are, by this.
pub fn icon_refused(icon: &Icon, names: &Interner) -> Option<String> {
    if names.resolve(icon.grid) != "32" {
        return Some("its grid is not written `32`".to_owned());
    }
    if icon.items.len() > MAX_ICON_SHAPES {
        return Some(format!("more than {MAX_ICON_SHAPES} shapes"));
    }
    if path_bytes(icon) > MAX_ICON_PATH_BYTES {
        return Some(format!("more than {MAX_ICON_PATH_BYTES} bytes of path"));
    }
    if segments_in_all(icon) > MAX_PATH_SEGMENTS {
        return Some(format!("more than {MAX_PATH_SEGMENTS} segments of path"));
    }
    icon.items.iter().find_map(|item| {
        if let Some(accent) = &item.accent {
            let token: Vec<&str> = accent.segments().map(|name| names.resolve(name)).collect();
            if !TOKENS.contains(&token.join(".").as_str()) {
                return Some("an accent that is not a token of the theme".to_owned());
            }
        }
        shape_extent(names, &item.shape).err()
    })
}

/// The bytes of all the `path` strings of an icon.
fn path_bytes(icon: &Icon) -> usize {
    icon.items
        .iter()
        .map(|item| match &item.shape {
            IconShape::Path { data } => data.len(),
            _ => 0,
        })
        .fold(0usize, usize::saturating_add)
}

/// The segments of all the paths of an icon, valid or not; it stops counting past the bound.
fn segments_in_all(icon: &Icon) -> usize {
    let mut total = 0usize;
    for item in &icon.items {
        if let IconShape::Path { data } = &item.shape {
            total = total.saturating_add(read_path(data, false).map_or(0, |pen| pen.count));
            if total > MAX_PATH_SEGMENTS {
                break;
            }
        }
    }
    total
}

fn judge_item(sections: &mut Sections<'_, '_>, item: &IconItem) {
    if let Some(accent) = &item.accent {
        let token: Vec<&str> = accent.segments().map(|name| sections.names().resolve(name)).collect();
        let token = token.join(".");
        if !TOKENS.contains(&token.as_str()) {
            let token = cut_text(&token).into_owned();
            let message = format!("`accent {token}`: an accent is a token of the theme, of the visual notation (section 8), such as `wire.error`");
            sections.say(Code::Lc5008, message, accent.id, Vec::new());
        }
    }
    let extent = match shape_extent(sections.names(), &item.shape) {
        Ok(extent) => extent,
        Err(message) => {
            sections.say(Code::Lc5008, message, item.id, Vec::new());
            return;
        }
    };
    if let Some(extent) = extent
        && (extent.min_x < -SLACK || extent.min_y < -SLACK || extent.max_x > GRID + SLACK || extent.max_y > GRID + SLACK)
    {
        let (x0, y0, x1, y1) = (shown(extent.min_x), shown(extent.min_y), shown(extent.max_x), shown(extent.max_y));
        let message = format!("this shape leaves the grid of 32: it draws from ({x0}, {y0}) to ({x1}, {y1}) (11.1)");
        sections.say(Code::Lc5009, message, item.id, Vec::new());
    }
}

/// A number for a message: two decimals at most.
fn shown(value: f64) -> String {
    // A text of a finite size can still draw past every number: `size 1e308`.
    if value.is_infinite() {
        return if value > 0.0 { "\u{221E}".to_owned() } else { "-\u{221E}".to_owned() };
    }
    let text = format!("{value:.2}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" { "0".to_owned() } else { text.to_owned() }
}

/// What a shape draws, or what is wrong with it. `None` for what draws nothing, such as an empty path.
fn shape_extent(names: &Interner, shape: &IconShape) -> Result<Option<Extent>, String> {
    let number = |numeral: Numeral| -> Result<f64, String> {
        number_of(numeral, names).ok_or_else(|| {
            let text = numeral_text(numeral, names);
            format!("`{text}` is not a number of the grid: an icon takes finite numbers (11.1)")
        })
    };
    let point = |point: &Point| -> Result<(f64, f64), String> { Ok((number(point.x)?, number(point.y)?)) };
    let positive = |numeral: Numeral, what: &str| -> Result<f64, String> {
        let value = number(numeral)?;
        if value > 0.0 {
            Ok(value)
        } else {
            let text = numeral_text(numeral, names);
            Err(format!("a {what} of `{text}`: a {what} is above zero (11.1)"))
        }
    };
    let mut extent = Extent::empty();
    match shape {
        IconShape::Rect { from, to, radius } => {
            if let Some(radius) = radius {
                positive(*radius, "radius")?;
            }
            extent.add(point(from)?);
            extent.add(point(to)?);
        }
        IconShape::Circle { center, radius } => {
            let (x, y) = point(center)?;
            let radius = positive(*radius, "radius")?;
            extent.add((x - radius, y - radius));
            extent.add((x + radius, y + radius));
        }
        IconShape::Line { from, to } => {
            let (from, to) = (point(from)?, point(to)?);
            // Of no length, it draws nothing, as a path that stays where it is.
            if from == to {
                return Ok(None);
            }
            extent.add(from);
            extent.add(to);
        }
        IconShape::Text { text, at, size } => {
            let size = match size {
                Some(size) => positive(*size, "size")?,
                None => TEXT_SIZE,
            };
            let (x, y) = point(at)?;
            // `at` is the middle of the baseline. The numbers are those of `types::text`, in floating point here, since a
            // size of an icon is a decimal of any precision.
            let cells = text::cells(&text::shown(&read_escapes(text)));
            if cells == 0 {
                return Ok(None);
            }
            let em = size / f64::from(text::UNITS_PER_EM);
            let half = cells as f64 * f64::from(text::ADVANCE) * em / 2.0;
            extent.add((x - half, y - f64::from(text::CAP_HEIGHT) * em));
            extent.add((x + half, y + f64::from(text::DESCENT) * em));
        }
        IconShape::Path { data } => return path_extent(data),
    }
    Ok(Some(extent))
}

fn number_of(numeral: Numeral, names: &Interner) -> Option<f64> {
    let text = match numeral.kind {
        NumeralKind::Int(text) | NumeralKind::Float(text) => names.resolve(text),
        NumeralKind::Inf | NumeralKind::Nan => return None,
    };
    let digits: String = text.chars().filter(|c| *c != '_').collect();
    let value = digits.parse::<f64>().ok().filter(|value| value.is_finite())?;
    Some(if numeral.negative { -value } else { value })
}

/// The box around what a shape draws.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Extent {
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
}

impl Extent {
    fn empty() -> Self {
        Extent { min_x: f64::INFINITY, min_y: f64::INFINITY, max_x: f64::NEG_INFINITY, max_y: f64::NEG_INFINITY }
    }

    fn is_empty(&self) -> bool {
        self.min_x > self.max_x
    }

    fn add(&mut self, (x, y): (f64, f64)) {
        self.min_x = self.min_x.min(x);
        self.min_y = self.min_y.min(y);
        self.max_x = self.max_x.max(x);
        self.max_y = self.max_y.max(y);
    }
}

/// A segment of a `path` of an icon, in absolute coordinates of its grid: what a canvas draws (architecture 11.4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathSegment {
    Move((f64, f64)),
    Line((f64, f64)),
    Cubic((f64, f64), (f64, f64), (f64, f64)),
    Quadratic((f64, f64), (f64, f64)),
    /// An arc in its center form (SVG 1.1, appendix F.6.5): its center, its radii, its rotation in radians, the angle
    /// where it starts and how far it turns, negative the other way.
    Arc {
        center: (f64, f64),
        radii: (f64, f64),
        rotation: f64,
        start: f64,
        delta: f64,
    },
    Close,
}

/// The most segments of the paths of an icon in all *(proposed)*: an icon is 32 units of grid, and more draws no more;
/// beyond, its calls show the label (11.1). A canvas keeps at most as many of one path.
pub const MAX_PATH_SEGMENTS: usize = 4096;

/// The segments of a `path` string (spec 11.1), at most [`MAX_PATH_SEGMENTS`], or why it is not in the SVG path syntax
/// (`LC5008`).
pub fn path_segments(data: &str) -> Result<Vec<PathSegment>, String> {
    Ok(read_path(data, true)?.segments)
}

/// What a path draws, from the text of its `path` string (SVG path syntax, `M L H V C S Q T A Z`, absolute or relative).
fn path_extent(data: &str) -> Result<Option<Extent>, String> {
    let pen = read_path(data, false)?;
    Ok(if pen.extent.is_empty() { None } else { Some(pen.extent) })
}

/// `record`: whether the pen keeps its segments, which only a drawing needs.
fn read_path(data: &str, record: bool) -> Result<Pen, String> {
    let mut reader = Reader { bytes: data.as_bytes(), at: 0 };
    let mut pen = Pen { record, ..Pen::default() };
    reader.skip_space();
    let mut command = match reader.peek() {
        None => return Ok(pen),
        Some(b'M' | b'm') => reader.command()?,
        Some(_) => return Err(reader.mistake("a path starts with `M` or `m`")),
    };
    loop {
        let relative = command.is_ascii_lowercase();
        match command.to_ascii_uppercase() {
            b'Z' => pen.close(),
            upper => {
                let mut first = true;
                loop {
                    pen.step(upper, relative, first, &mut reader)?;
                    first = false;
                    if !reader.more_numbers()? {
                        break;
                    }
                }
            }
        }
        reader.skip_space();
        match reader.peek() {
            None => break,
            Some(_) => command = reader.command()?,
        }
    }
    Ok(pen)
}

struct Reader<'s> {
    bytes: &'s [u8],
    at: usize,
}

impl Reader<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn mistake(&self, what: &str) -> String {
        let place = self.at + 1;
        format!("this `path` is not in the SVG path syntax: at character {place}, {what} (11.1)")
    }

    fn skip_space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r' | b'\x0C')) {
            self.at += 1;
        }
    }

    fn command(&mut self) -> Result<u8, String> {
        match self.peek() {
            Some(byte) if b"MmLlHhVvCcSsQqTtAaZz".contains(&byte) => {
                self.at += 1;
                Ok(byte)
            }
            _ => Err(self.mistake("a command is expected: `M L H V C S Q T A Z`, or their lowercase")),
        }
    }

    /// Between two numbers: spaces, at most one comma. Whether another number follows: a comma must lead to one.
    fn more_numbers(&mut self) -> Result<bool, String> {
        self.skip_space();
        let comma = self.peek() == Some(b',');
        if comma {
            self.at += 1;
            self.skip_space();
        }
        let number = matches!(self.peek(), Some(b'0'..=b'9' | b'.' | b'+' | b'-'));
        if comma && !number {
            return Err(self.mistake("a number is expected after the comma"));
        }
        Ok(number)
    }

    fn separator(&mut self) {
        self.skip_space();
        if self.peek() == Some(b',') {
            self.at += 1;
            self.skip_space();
        }
    }

    fn number(&mut self) -> Result<f64, String> {
        self.skip_space();
        let start = self.at;
        if matches!(self.peek(), Some(b'+' | b'-')) {
            self.at += 1;
        }
        let digits = self.digits();
        let mut decimals = 0;
        if self.peek() == Some(b'.') {
            self.at += 1;
            decimals = self.digits();
        }
        if digits + decimals == 0 {
            self.at = start;
            return Err(self.mistake("a number is expected"));
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            let before = self.at;
            self.at += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.at += 1;
            }
            if self.digits() == 0 {
                self.at = before;
                return Err(self.mistake("an exponent needs digits"));
            }
        }
        let text = std::str::from_utf8(self.bytes.get(start..self.at).unwrap_or_default()).unwrap_or_default();
        match text.parse::<f64>() {
            Ok(value) if value.is_finite() => Ok(value),
            _ => {
                self.at = start;
                Err(self.mistake("this number is too large"))
            }
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.at;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.at += 1;
        }
        self.at - start
    }

    fn flag(&mut self) -> Result<bool, String> {
        self.separator();
        match self.peek() {
            Some(b'0') => {
                self.at += 1;
                Ok(false)
            }
            Some(b'1') => {
                self.at += 1;
                Ok(true)
            }
            _ => Err(self.mistake("an arc flag is `0` or `1`")),
        }
    }

    fn pair(&mut self) -> Result<(f64, f64), String> {
        let x = self.number()?;
        self.separator();
        let y = self.number()?;
        Ok((x, y))
    }
}

/// Where the path stands, and what it has drawn.
#[derive(Default)]
struct Pen {
    at: (f64, f64),
    start: (f64, f64),
    /// The last control point of a cubic or a quadratic curve, which `S` or `T` reflects.
    cubic: Option<(f64, f64)>,
    quadratic: Option<(f64, f64)>,
    extent: Extent,
    record: bool,
    segments: Vec<PathSegment>,
    /// Every segment drawn, kept or not.
    count: usize,
}

impl Default for Extent {
    fn default() -> Self {
        Extent::empty()
    }
}

impl Pen {
    /// A segment kept, within the bound; a close after a close adds nothing.
    fn push(&mut self, segment: PathSegment) {
        self.count = self.count.saturating_add(1);
        if !self.record || self.segments.len() >= MAX_PATH_SEGMENTS || (segment == PathSegment::Close && self.segments.last() == Some(&segment)) {
            return;
        }
        self.segments.push(segment);
    }

    fn close(&mut self) {
        // Back where it started, it draws nothing: `M40 40 Z` is no shape.
        if self.at != self.start {
            self.line(self.start);
        }
        self.push(PathSegment::Close);
        self.at = self.start;
        self.cubic = None;
        self.quadratic = None;
    }

    /// A segment of no length draws nothing, stroke aside, as a path that closes where it started.
    fn line(&mut self, to: (f64, f64)) {
        if self.at != to {
            self.extent.add(self.at);
            self.extent.add(to);
        }
    }

    /// One set of arguments of a command.
    fn step(&mut self, command: u8, relative: bool, first: bool, reader: &mut Reader<'_>) -> Result<(), String> {
        let (ox, oy) = if relative { self.at } else { (0.0, 0.0) };
        let shift = |(x, y): (f64, f64)| (x + ox, y + oy);
        let (mut cubic, mut quadratic) = (None, None);
        match command {
            b'M' if first => {
                self.at = shift(reader.pair()?);
                self.start = self.at;
                self.push(PathSegment::Move(self.at));
            }
            b'M' | b'L' => {
                let to = shift(reader.pair()?);
                self.line(to);
                self.push(PathSegment::Line(to));
                self.at = to;
            }
            b'H' => {
                let x = reader.number()? + ox;
                let to = (x, self.at.1);
                self.line(to);
                self.push(PathSegment::Line(to));
                self.at = to;
            }
            b'V' => {
                let y = reader.number()? + oy;
                let to = (self.at.0, y);
                self.line(to);
                self.push(PathSegment::Line(to));
                self.at = to;
            }
            b'C' | b'S' => {
                let first_control = if command == b'C' {
                    let control = shift(reader.pair()?);
                    reader.separator();
                    control
                } else {
                    reflect(self.cubic, self.at)
                };
                let second = shift(reader.pair()?);
                reader.separator();
                let to = shift(reader.pair()?);
                self.cubic_curve(first_control, second, to);
                self.push(PathSegment::Cubic(first_control, second, to));
                cubic = Some(second);
                self.at = to;
            }
            b'Q' | b'T' => {
                let control = if command == b'Q' {
                    let control = shift(reader.pair()?);
                    reader.separator();
                    control
                } else {
                    reflect(self.quadratic, self.at)
                };
                let to = shift(reader.pair()?);
                self.quadratic_curve(control, to);
                self.push(PathSegment::Quadratic(control, to));
                quadratic = Some(control);
                self.at = to;
            }
            b'A' => {
                let rx = reader.number()?;
                reader.separator();
                let ry = reader.number()?;
                reader.separator();
                let rotation = reader.number()?;
                let large = reader.flag()?;
                let sweep = reader.flag()?;
                reader.separator();
                let to = shift(reader.pair()?);
                self.arc(rx, ry, rotation, large, sweep, to);
                self.at = to;
            }
            _ => return Err(reader.mistake("a command is expected")),
        }
        self.cubic = cubic;
        self.quadratic = quadratic;
        Ok(())
    }

    fn cubic_curve(&mut self, c1: (f64, f64), c2: (f64, f64), to: (f64, f64)) {
        let from = self.at;
        self.line(to);
        let mut times = Vec::with_capacity(4);
        for (p0, p1, p2, p3) in [(from.0, c1.0, c2.0, to.0), (from.1, c1.1, c2.1, to.1)] {
            // The derivative over 3: a t² + b t + c.
            let a = -p0 + 3.0 * p1 - 3.0 * p2 + p3;
            let b = 2.0 * (p0 - 2.0 * p1 + p2);
            let c = p1 - p0;
            times.extend(roots(a, b, c));
        }
        for t in times.into_iter().filter(|t| *t > 0.0 && *t < 1.0) {
            let u = 1.0 - t;
            let at = |p0: f64, p1: f64, p2: f64, p3: f64| u * u * u * p0 + 3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t * p3;
            self.extent.add((at(from.0, c1.0, c2.0, to.0), at(from.1, c1.1, c2.1, to.1)));
        }
    }

    fn quadratic_curve(&mut self, control: (f64, f64), to: (f64, f64)) {
        let from = self.at;
        self.line(to);
        for (p0, p1, p2) in [(from.0, control.0, to.0), (from.1, control.1, to.1)] {
            let denominator = p0 - 2.0 * p1 + p2;
            if denominator.abs() < f64::EPSILON {
                continue;
            }
            let t = (p0 - p1) / denominator;
            if t > 0.0 && t < 1.0 {
                let u = 1.0 - t;
                let at = |a: f64, b: f64, c: f64| u * u * a + 2.0 * u * t * b + t * t * c;
                self.extent.add((at(from.0, control.0, to.0), at(from.1, control.1, to.1)));
            }
        }
    }

    /// An arc in the endpoint form of SVG, through its center form (SVG 1.1, appendix F.6.5), for the extremes it reaches.
    fn arc(&mut self, rx: f64, ry: f64, rotation: f64, large: bool, sweep: bool, to: (f64, f64)) {
        let from = self.at;
        self.line(to);
        let (mut rx, mut ry) = (rx.abs(), ry.abs());
        if from == to {
            // It draws nothing, and still counts in the bound of an icon, as every arc that the text writes (11.1).
            self.count = self.count.saturating_add(1);
            return;
        }
        // An arc with a radius of zero is a line (SVG 1.1, F.6.6).
        if rx == 0.0 || ry == 0.0 {
            self.push(PathSegment::Line(to));
            return;
        }
        let phi = rotation.to_radians();
        let (sin, cos) = phi.sin_cos();
        let (dx, dy) = ((from.0 - to.0) / 2.0, (from.1 - to.1) / 2.0);
        let (x1, y1) = (cos * dx + sin * dy, -sin * dx + cos * dy);
        let lambda = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
        // Radii too small for the chord grow to it (F.6.6), written so that radii of 1e-300, whose `lambda` overflows,
        // still give the half ellipse that they mean.
        if lambda > 1.0 || lambda.is_nan() {
            let ratio = rx / ry;
            (rx, ry) = ((x1 * x1 + (y1 * ratio) * (y1 * ratio)).sqrt(), ((x1 / ratio) * (x1 / ratio) + y1 * y1).sqrt());
        }
        let numerator = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
        let denominator = rx * rx * y1 * y1 + ry * ry * x1 * x1;
        let mut coefficient = if denominator > 0.0 { (numerator / denominator).max(0.0).sqrt() } else { 0.0 };
        if large == sweep {
            coefficient = -coefficient;
        }
        let (cx1, cy1) = (coefficient * rx * y1 / ry, -coefficient * ry * x1 / rx);
        let (cx, cy) = (cos * cx1 - sin * cy1 + (from.0 + to.0) / 2.0, sin * cx1 + cos * cy1 + (from.1 + to.1) / 2.0);
        let angle = |ux: f64, uy: f64| uy.atan2(ux);
        let start = angle((x1 - cx1) / rx, (y1 - cy1) / ry);
        let end = angle((-x1 - cx1) / rx, (-y1 - cy1) / ry);
        let tau = std::f64::consts::TAU;
        let mut delta = (end - start).rem_euclid(tau);
        if !sweep && delta > 0.0 {
            delta -= tau;
        }
        if ![cx, cy, rx, ry, start, delta].iter().all(|value| value.is_finite()) {
            // Numbers that no grid of 32 holds: the extent says it with `LC5009`, a canvas draws the chord.
            self.push(PathSegment::Line(to));
            return;
        }
        self.push(PathSegment::Arc { center: (cx, cy), radii: (rx, ry), rotation: phi, start, delta });
        let point = |theta: f64| {
            let (s, c) = theta.sin_cos();
            (cx + rx * cos * c - ry * sin * s, cy + rx * sin * c + ry * cos * s)
        };
        let x_extreme = (-ry * sin).atan2(rx * cos);
        let y_extreme = (ry * cos).atan2(rx * sin);
        for theta in [x_extreme, x_extreme + std::f64::consts::PI, y_extreme, y_extreme + std::f64::consts::PI] {
            let reached = if delta >= 0.0 { (theta - start).rem_euclid(tau) <= delta } else { (start - theta).rem_euclid(tau) <= -delta };
            if reached {
                self.extent.add(point(theta));
            }
        }
    }
}

/// The control point that `S` or `T` takes: the last one reflected about the current point, or the current point itself.
fn reflect(control: Option<(f64, f64)>, at: (f64, f64)) -> (f64, f64) {
    control.map_or(at, |(x, y)| (2.0 * at.0 - x, 2.0 * at.1 - y))
}

/// The real roots of a t² + b t + c.
fn roots(a: f64, b: f64, c: f64) -> Vec<f64> {
    if a.abs() < 1e-12 {
        return if b.abs() < 1e-12 { Vec::new() } else { vec![-c / b] };
    }
    let discriminant = b * b - 4.0 * a * c;
    if discriminant < 0.0 {
        return Vec::new();
    }
    let root = discriminant.sqrt();
    vec![(-b + root) / (2.0 * a), (-b - root) / (2.0 * a)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extent(data: &str) -> (f64, f64, f64, f64) {
        let extent = path_extent(data).ok().flatten().unwrap_or_default();
        let round = |value: f64| (value * 1000.0).round() / 1000.0;
        (round(extent.min_x), round(extent.min_y), round(extent.max_x), round(extent.max_y))
    }

    #[test]
    fn the_arc_of_check_voltage_reaches_its_top() {
        // A half circle of radius 9 around (16, 21), above its chord.
        assert_eq!(extent("M7 21 A9 9 0 0 1 25 21"), (7.0, 12.0, 25.0, 21.0));
    }

    #[test]
    fn the_other_sweep_goes_below() {
        assert_eq!(extent("M7 21 A9 9 0 0 0 25 21"), (7.0, 21.0, 25.0, 30.0));
    }

    #[test]
    fn a_curve_counts_by_what_it_draws_not_by_its_control_points() {
        // Control points at y = -8: the curve reaches y = -5, halfway.
        assert_eq!(extent("M0 4 C0 -8 32 -8 32 4"), (0.0, -5.0, 32.0, 4.0));
        assert_eq!(extent("M0 10 Q16 -10 32 10"), (0.0, 0.0, 32.0, 10.0));
    }

    #[test]
    fn relative_commands_and_repeats_move_from_where_the_pen_is() {
        assert_eq!(extent("m2 2 l4 0 0 4 h-4 z"), (2.0, 2.0, 6.0, 6.0));
        assert_eq!(extent("M1,1 2,2 3,3"), (1.0, 1.0, 3.0, 3.0));
        assert_eq!(extent("M1-1"), extent("M 1 -1"));
        assert_eq!(extent("M0 0L.5.5"), (0.0, 0.0, 0.5, 0.5));
        assert_eq!(extent("M0 0 a1 1 0 00 2 0"), extent("M0 0 a1,1,0,0,0,2,0"));
    }

    #[test]
    fn s_and_t_reflect_the_last_control_point() {
        assert_eq!(extent("M0 10 Q5 0 10 10 T20 10"), (0.0, 5.0, 20.0, 15.0));
        assert_eq!(extent("M0 0 C0 0 0 0 0 0 S10 10 10 0"), extent("M0 0 C0 0 0 0 0 0 C0 0 10 10 10 0"));
    }

    #[test]
    fn what_is_not_the_syntax_is_refused() {
        for data in ["L1 1", "M", "M1", "M1 1 X", "M1 1,", "M1 1 A1 1 0 2 0 5 5", "M1e 1", "M1 1 L", "M1 1 é", "M1e999 1"] {
            assert!(path_extent(data).is_err(), "{data}");
        }
    }

    #[test]
    fn the_segments_are_absolute() {
        // Worked by hand: `m2 2 l4 0 0 4` from (2, 2), then (6, 2), (6, 6); `h-4` to (2, 6); `z` back.
        assert_eq!(
            path_segments("m2 2 l4 0 0 4 h-4 z"),
            Ok(vec![
                PathSegment::Move((2.0, 2.0)),
                PathSegment::Line((6.0, 2.0)),
                PathSegment::Line((6.0, 6.0)),
                PathSegment::Line((2.0, 6.0)),
                PathSegment::Close
            ])
        );
        // `S` reflects (0, 0) about (0, 0): its first control point is (0, 0).
        assert_eq!(
            path_segments("M0 0 S10 10 10 0").map(|segments| segments.get(1).copied()),
            Ok(Some(PathSegment::Cubic((0.0, 0.0), (10.0, 10.0), (10.0, 0.0))))
        );
        // A half circle of radius 9 around (16, 21), from (7, 21) through its top: from angle pi, a half turn forward.
        let Ok(arc) = path_segments("M7 21 A9 9 0 0 1 25 21") else { panic!("the arc reads") };
        let Some(&PathSegment::Arc { center, radii, start, delta, .. }) = arc.get(1) else { panic!("an arc: {arc:?}") };
        let round = |value: f64| (value * 1000.0).round() / 1000.0;
        assert_eq!(
            (round(center.0), round(center.1), radii, round(start.abs()), round(delta)),
            (16.0, 21.0, (9.0, 9.0), round(std::f64::consts::PI), round(std::f64::consts::PI))
        );
        assert_eq!(path_segments("M0 0 A0 5 0 0 1 4 0"), Ok(vec![PathSegment::Move((0.0, 0.0)), PathSegment::Line((4.0, 0.0))]));
    }

    #[test]
    fn an_arc_with_tiny_radii_is_the_half_ellipse_on_its_chord() {
        // From (0, 30) to (32, 30), the radii grown to 16: a half circle around (16, 30), below it with the sweep 0.
        assert_eq!(extent("M0 30 A1e-300 1e-300 0 0 0 32 30"), (0.0, 30.0, 32.0, 46.0));
        assert_eq!(extent("M0 30 A1 1 0 0 0 32 30"), (0.0, 30.0, 32.0, 46.0));
        let Ok(segments) = path_segments("M-1e308 0 A1 1 0 0 1 1e308 0") else { panic!("it reads") };
        assert!(segments.iter().all(|segment| !matches!(segment, PathSegment::Arc { .. })), "{segments:?}");
    }

    #[test]
    fn the_segments_kept_are_bounded_and_a_check_keeps_none() {
        let long = format!("M0 0{}", " L1 1".repeat(MAX_PATH_SEGMENTS * 2));
        assert_eq!(path_segments(&long).map(|segments| segments.len()), Ok(MAX_PATH_SEGMENTS));
        assert_eq!(path_segments("M0 0 L1 1 z z z").map(|segments| segments.len()), Ok(3));
        assert!(read_path(&long, false).is_ok_and(|pen| pen.segments.is_empty()));
    }

    #[test]
    fn an_empty_path_draws_nothing() {
        assert_eq!(path_extent("  "), Ok(None));
        assert_eq!(path_extent("M5 5"), Ok(None));
    }

    /// The tokens of the visual notation, section 8, whose parts are names.
    #[test]
    fn the_tokens_are_those_of_the_notation() {
        let notation = include_str!("../../../../../../docs/visual-notation.md");
        let section = notation.split("## 8. Theme tokens").nth(1).and_then(|rest| rest.split("\n- ").next()).unwrap_or_default();
        let mut written: Vec<&str> = section
            .split('`')
            .skip(1)
            .step_by(2)
            .filter(|token| token.contains('.') && token.split('.').all(|part| part.starts_with(|c: char| c.is_ascii_lowercase())))
            .collect();
        written.sort_unstable();
        written.dedup();
        assert_eq!(written, TOKENS);
    }
}
