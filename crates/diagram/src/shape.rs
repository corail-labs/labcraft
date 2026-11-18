//! What a drawing is made of (`docs/architecture.md`, 11.3): shapes with the name of their theme token, in integer
//! hundredths of a unit of the canvas. The canvas and the SVG paint only these.

use crate::style;
use labcraft_types::text::Ink;

/// A drawing as data, for the web viewer (feature `serde`).
#[cfg(feature = "serde")]
#[path = "serial.rs"]
mod serial;

/// A token of the theme (notation 8), by its name: `node.fill`.
pub type Token = &'static str;

/// How a shape is painted: a fill, an outline and its width, each optional.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Paint {
    #[cfg_attr(feature = "serde", serde(with = "crate::shape::serial::option_token"))]
    pub fill: Option<Token>,
    #[cfg_attr(feature = "serde", serde(with = "crate::shape::serial::option_token"))]
    pub stroke: Option<Token>,
    /// In hundredths of a unit.
    pub width: i64,
    /// The outline dashed: a value of a trace that no longer matches its files (notation 6.1).
    pub dashed: bool,
    /// From 0 to 100: a veil over what a run did not reach, an arm it did not take (notation 6.1). 100 elsewhere.
    pub opacity: u8,
}

impl Paint {
    pub const fn fill(token: Token) -> Paint {
        Paint { fill: Some(token), stroke: None, width: 0, dashed: false, opacity: 100 }
    }

    pub const fn stroke(token: Token, width: i64) -> Paint {
        Paint { fill: None, stroke: Some(token), width, dashed: false, opacity: 100 }
    }

    pub const fn both(fill: Token, stroke: Token, width: i64) -> Paint {
        Paint { fill: Some(fill), stroke: Some(stroke), width, dashed: false, opacity: 100 }
    }

    pub const fn dashed(self) -> Paint {
        Paint { dashed: true, ..self }
    }

    pub const fn veiled(self, opacity: u8) -> Paint {
        Paint { opacity, ..self }
    }
}

/// From which zoom a text is shown on the canvas (notation 7). The SVG shows every text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Detail {
    /// A title of a frame, an arm or a tab: at every zoom.
    Title,
    /// From 40 %.
    #[default]
    Label,
    /// The name of a terminal, its unit: from 75 %.
    Name,
}

impl Detail {
    /// Whether a text of this detail is shown at `zoom` (1 for 100 %).
    pub fn shown_at(self, zoom: f32) -> bool {
        match self {
            Detail::Title => true,
            Detail::Label => zoom >= style::factor(style::coarse_zoom()),
            Detail::Name => zoom >= style::factor(style::names_zoom()),
        }
    }
}

/// A node below the coarse zoom (notation 7, `zoom.coarse` of [`crate::style`]).
pub const COARSE_NODE: Paint = Paint::both("node.fill", "node.stroke", 100);

/// A fill under a text, that hides what passes under it and is shown and hidden with it (notation 5): the name of a
/// terminal of a frame, over the wire that leaves the terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Ground {
    #[cfg_attr(feature = "serde", serde(with = "crate::shape::serial::token"))]
    pub token: Token,
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
}

/// A run of a text in one token.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Run {
    #[cfg_attr(feature = "serde", serde(with = "crate::shape::serial::token"))]
    pub token: Token,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Shape {
    Rect {
        x: i64,
        y: i64,
        width: i64,
        height: i64,
        radius: i64,
        paint: Paint,
    },
    Circle {
        x: i64,
        y: i64,
        radius: i64,
        paint: Paint,
    },
    /// A closed polygon, or an open line when `closed` is false.
    Poly {
        points: Vec<(i64, i64)>,
        closed: bool,
        paint: Paint,
    },
    /// A text in logical order, isolated, left to right (notation 2.3): `x` its start, `y` its baseline, `size` in
    /// hundredths, `width` what its cells take.
    Text {
        x: i64,
        y: i64,
        size: u32,
        width: i64,
        runs: Vec<Run>,
        /// Not in the SVG.
        detail: Detail,
        /// Drawn just before the text.
        ground: Option<Ground>,
    },
    /// The icon of a function (spec 11.1), drawn in a square of `side` and cut to its grid: `icon` is its index in
    /// `Drawing::icons`, where each icon is once, however many calls show it.
    Icon {
        x: i64,
        y: i64,
        side: i64,
        icon: usize,
    },
    /// A wire: an orthogonal polyline, its corners rounded (notation 2), drawn as its family says (notation 4.1).
    Wire {
        points: Vec<(i64, i64)>,
        style: WireStyle,
    },
}

/// How a wire is drawn (notation 4.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct WireStyle {
    #[cfg_attr(feature = "serde", serde(with = "crate::shape::serial::token"))]
    pub token: Token,
    /// In hundredths of a unit.
    pub width: i64,
    pub pattern: Pattern,
    /// A struct is sheathed: its core, in the fill of the frame it runs in.
    #[cfg_attr(feature = "serde", serde(with = "crate::shape::serial::option_token"))]
    pub core: Option<Token>,
    /// An array has a cross-tick per dimension every 8 units, within the width of the wire.
    pub ticks: u8,
    /// What a cut through the wire shows: the fill of the frame it runs in. Nothing is drawn outside the width of a
    /// wire: its pattern is cut into it (notation 4.1).
    #[cfg_attr(feature = "serde", serde(with = "crate::shape::serial::token"))]
    pub ground: Token,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Pattern {
    Solid,
    /// An error: a tape, chevrons cut into it every 8 units, in the direction of the flow.
    Chevrons,
    /// An order wire, thin and dotted.
    Dotted,
    /// A function reference.
    DashDot,
}

impl Pattern {
    /// The dashes of the stroke, alternately a dash and a gap, in hundredths of a unit; none for a solid stroke.
    pub fn dashes(self) -> &'static [i64] {
        match self {
            Pattern::Dotted => &[150, 300],
            Pattern::DashDot => &[600, 300, 150, 300],
            Pattern::Solid | Pattern::Chevrons => &[],
        }
    }
}

impl WireStyle {
    /// The cross-ticks of an array, one per dimension every 8 units: alternately a dash and a gap, in hundredths of a
    /// unit; none when the wire has no tick.
    pub fn tick_dashes(&self) -> &'static [i64] {
        match self.ticks {
            0 => &[],
            1 => &[100, 700],
            2 => &[100, 200, 100, 400],
            _ => &[100, 200, 100, 200, 100, 100],
        }
    }
}

/// How wide the stroke of a chevron cut into an error wire is, in hundredths of a unit.
pub const CHEVRON_STROKE: i64 = 90;

/// A point of an orthogonal polyline once its corners are rounded: the ends and straight points as they are, a corner as
/// a quadratic curve from `from` to `to` that bends toward `corner`, of radius `style::wire_radius` at most.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corner {
    Point((i64, i64)),
    Round { from: (i64, i64), corner: (i64, i64), to: (i64, i64) },
}

/// The points of an orthogonal polyline with its corners rounded: one `Corner` per point (notation 4.1).
pub fn corners(points: &[(i64, i64)]) -> Vec<Corner> {
    points
        .iter()
        .enumerate()
        .map(|(index, &(x, y))| {
            let (Some(&(px, py)), Some(&(nx, ny))) = (index.checked_sub(1).and_then(|before| points.get(before)), points.get(index + 1)) else {
                return Corner::Point((x, y));
            };
            let before = (x - px).abs() + (y - py).abs();
            let after = (nx - x).abs() + (ny - y).abs();
            let radius = style::wire_radius().min(before / 2).min(after / 2);
            Corner::Round {
                from: (x - (x - px).signum() * radius, y - (y - py).signum() * radius),
                corner: (x, y),
                to: (x + (nx - x).signum() * radius, y + (ny - y).signum() * radius),
            }
        })
        .collect()
}

/// The chevrons cut into an error wire along its polyline: each three points, pointing in the direction of the flow,
/// 1.8 long and 2.8 across, so that with their stroke they stay inside a wire of 4.
pub fn chevrons(points: &[(i64, i64)]) -> Vec<[(i64, i64); 3]> {
    let mut found = Vec::new();
    let mut next = style::chevron_step() / 2;
    let mut walked = 0i64;
    for pair in points.windows(2) {
        let (Some(&(x0, y0)), Some(&(x1, y1))) = (pair.first(), pair.get(1)) else { continue };
        let (dx, dy) = ((x1 - x0).signum(), (y1 - y0).signum());
        let length = (x1 - x0).abs() + (y1 - y0).abs();
        // Not on a corner, where the wire bends.
        while next <= walked + length - style::wire_radius() {
            let along = next - walked;
            if along >= style::wire_radius() {
                let (x, y) = (x0 + dx * along, y0 + dy * along);
                let (back, side) = (90, 140);
                found.push([
                    (x - dx * back - dy * side, y - dy * back + dx * side),
                    (x + dx * back, y + dy * back),
                    (x - dx * back + dy * side, y - dy * back - dx * side),
                ]);
            }
            next += style::chevron_step();
        }
        walked += length;
        next = next.max(walked + style::wire_radius());
    }
    found
}

/// An icon as the SVG draws it: its grid and its items, in the numbers that the file writes.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct IconDrawing {
    pub grid: String,
    pub items: Vec<IconItem>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct IconItem {
    pub shape: IconShape,
    /// The same shape in numbers of its grid, a path in segments: what a canvas draws (decided at step 2 of M0v).
    pub drawn: IconDrawn,
    pub fill: bool,
    #[cfg_attr(feature = "serde", serde(with = "crate::shape::serial::token"))]
    pub token: Token,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum IconShape {
    Rect {
        x: String,
        y: String,
        width: String,
        height: String,
        radius: Option<String>,
    },
    Circle {
        x: String,
        y: String,
        radius: String,
    },
    Line {
        x1: String,
        y1: String,
        x2: String,
        y2: String,
    },
    Path {
        data: String,
    },
    /// Centred on `x`, its baseline at `y`.
    Text {
        x: String,
        y: String,
        size: String,
        runs: Vec<Run>,
    },
}

/// An item of an icon in numbers of its grid. A text keeps its runs in [`IconShape::Text`].
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum IconDrawn {
    Rect {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        radius: f64,
    },
    Circle {
        x: f64,
        y: f64,
        radius: f64,
    },
    Line {
        from: (f64, f64),
        to: (f64, f64),
    },
    Path(#[cfg_attr(feature = "serde", serde(with = "crate::shape::serial::segments"))] Vec<labcraft_check::PathSegment>),
    /// Centred on `x`, its baseline at `y`.
    Text {
        x: f64,
        y: f64,
        size: f64,
    },
}

impl Eq for IconDrawn {}

/// What a group of shapes stands for, in the forms of the reports (`docs/cli.md`, 7.2).
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Group {
    /// `data-node`: the name of the line, `#close`, `@12`.
    pub node: String,
    /// The `<title>`: the full name of what it calls, as its tooltip.
    pub title: String,
    pub shapes: Vec<Shape>,
    /// `data-arm`: a branch of a `match` or a `handle`, as its tab writes it, instead of a node.
    pub arm: Option<String>,
    /// A wire, instead of a node.
    pub wire: Option<WireOf>,
    /// The node it draws, or the structure whose arm it draws, by its index in [`Drawing::placed`]: what glides with
    /// it (notation 2.1). Not in the SVG.
    pub placed: Option<usize>,
    /// The innermost arm shown in tabs that holds it, by its index in [`Drawing::arms`]: it shows only when that arm and
    /// every arm around it are shown (notation 5.2). Not in the SVG.
    pub within: Option<usize>,
    /// The terminals of the frame of a body and their names, drawn over its wires (notation 5), instead of a node: with
    /// the title of its arm when the body is one, for its `data-arm`.
    pub terminals: Option<TerminalsOf>,
}

/// What a group of the terminals of a frame belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TerminalsOf {
    /// The title of the arm, when the frame is that of an arm of a `match` or a `handle`.
    pub arm: Option<String>,
}

impl Group {
    /// Whether it draws a node, a structure included: neither a wire, nor an arm, nor the terminals of a frame. The groups
    /// of the nodes come in the order of [`Drawing::placed`].
    pub fn is_node(&self) -> bool {
        self.wire.is_none() && self.arm.is_none() && self.terminals.is_none()
    }
}

/// An arm of a `match` or a `handle` drawn in tabs (notation 5.2): its arms lie on one another, in one frame as large
/// as the largest, and the canvas shows the one that the view state chooses.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ArmOf {
    /// The structure, by the key that follows it across a change of the file ([`Placed::key`]).
    pub structure: String,
    /// Its rank among the arms of its structure, in the order of the text.
    pub arm: usize,
    /// The arm in tabs that holds its structure, by its index in [`Drawing::arms`].
    pub within: Option<usize>,
}

/// The tab of an arm in the header of its structure: where a click chooses it, and how it is drawn, shown or not.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Tab {
    /// The arm it shows when clicked, by its index in [`Drawing::arms`].
    pub arm: usize,
    /// Its structure, by its index in [`Drawing::placed`]: it glides with it.
    pub structure: usize,
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
    /// Painted when its arm is shown, and otherwise `other`.
    pub shown: Vec<Shape>,
    pub other: Vec<Shape>,
    /// Shown only while this arm is, by its index in [`Drawing::arms`]: the parts of a selector.
    pub when: Option<usize>,
    pub kind: TabKind,
    /// The title of its arm, as the list of a selector writes it.
    pub caption: String,
}

/// A tab of a row, or a part of the selector that stands for the row when its tabs do not hold in the width of the
/// frame (notation 5.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TabKind {
    Tab,
    /// `‹` or `›`: to the arm before or after.
    Step,
    /// The title of the arm shown: a click lists the arms.
    Name,
}

/// A reader of a wire: the form of the terminal it reaches, if it has one, and the path from the source to it.
pub type Reader = (Option<String>, Vec<(i64, i64)>);

/// What a wire stands for (`docs/cli.md`, 7.2): `data-wire`, `data-from`, `data-to`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct WireOf {
    /// The body that holds it, by its index in the graph. Not in the SVG.
    pub body: usize,
    /// Whether it carries an error. Not in the SVG.
    pub error: bool,
    /// Whether the file gives it waypoints (`via`), which it keeps whatever the rules say. Not in the SVG.
    pub via: bool,
    pub value: String,
    /// The terminal it leaves, when that has a form in the reports (`docs/cli.md`, 4.3).
    pub from: Option<String>,
    /// The terminals it reaches that have one.
    pub to: Vec<String>,
    /// Each reader: the form of the terminal it reaches, if it has one, and the whole path from the source to it, for
    /// the mark of a diagnostic that names it (`docs/cli.md`, 7.2). Not in the SVG.
    pub readers: Vec<Reader>,
    /// Its exact type as the language writes it, for its tooltip (notation 4.1). Not in the SVG.
    pub ty: String,
    /// The whole conversions that its pills show in part (`f32→f64`, notation 4.2), in the order of its readers: its
    /// `data-conversion` (`docs/cli.md`, 7.2).
    pub conversions: Vec<String>,
    /// Where its value comes from in a trace, by its index in [`crate::values::TracePlan::sources`]; none for an order
    /// wire. Not in the SVG.
    pub source: Option<u32>,
    /// Whether its value starts in this body, where its bubble shows: not a value that enters by a notch, which shows
    /// outside (notation 6.1). Not in the SVG.
    pub starts: bool,
    /// Its wires as an editing command names them (commands.md 9.2), `value -> node.input`, `value -> output`, one per
    /// reader that a command can name: a click selects them all, the whole value (notation 2.5). A reader that no
    /// command names yet (a node with no name; the `next`, `until`, `collect` or `yield` of a structure) has none. Not in
    /// the SVG.
    pub named: Vec<String>,
    /// The node it leaves and the nodes it reaches, by [`Placed::key`]: for what feeds a node and what it feeds
    /// (notation 2.6, Alt+Left and Alt+Right). None for a terminal of a frame. Not in the SVG.
    pub from_node: Option<String>,
    pub to_nodes: Vec<String>,
    /// By reader, in the order of `readers`: the wire as `wire.route` and a `wire` entry of `layout` name it (spec 11),
    /// which also names the wires into a structure and those to its `next`, `until`, `collect` or `yield`; none for a
    /// value with no name. Not in the SVG.
    #[cfg_attr(feature = "serde", serde(default))]
    pub entries: Vec<Option<String>>,
    /// By reader: whether the file gives that wire waypoints (`via`). Not in the SVG.
    #[cfg_attr(feature = "serde", serde(default))]
    pub vias: Vec<bool>,
}

/// What the tooltip says over an area of the canvas: a terminal, its name, its exact type, and the default of an input
/// that no argument wires (notation 3.2, 4.1). Not in the SVG.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Hint {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
    pub text: String,
    /// The node it belongs to, or the structure whose frame holds it, by its index in [`Drawing::placed`]: it glides with
    /// it.
    pub placed: Option<usize>,
    /// As [`Group::within`].
    pub within: Option<usize>,
    /// A terminal of the frame of the function, as `view.select` names it, `input x` or `output x`: a click on it selects
    /// it (notation 2.6). None for a terminal of a node or of the frame of a structure.
    pub selects: Option<String>,
}

/// A terminal where a wire starts or ends, as the wiring of the editor needs it (notation 2.5, 2.6): the canvas judges a
/// wire on these before `wire.connect` judges it again (architecture 12.6). Not in the SVG.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Port {
    /// Where its wire meets it, in hundredths.
    pub x: i64,
    pub y: i64,
    /// The node it belongs to, or the structure whose frame holds it, by its index in [`Drawing::placed`]; none for a
    /// terminal of the frame of the function.
    pub placed: Option<usize>,
    /// As [`Group::within`].
    pub within: Option<usize>,
    /// The body where its wire lies, by its index in the graph.
    pub body: usize,
    /// Whether it takes a wire: an input of a node, its `after:`, an output of the function. Otherwise a wire starts
    /// there: an output of a node, an input of the function, a value that a frame gives its body.
    pub input: bool,
    /// The `after:` of a node (R-15): it takes an order wire from any value.
    pub order: bool,
    /// A terminal of a frame: of the function, or of the body of a structure.
    #[cfg_attr(feature = "serde", serde(default))]
    pub frame: bool,
    /// A notch: an outer value read inside a structure, whose body ([`Port::body`]) is the one that binds it.
    #[cfg_attr(feature = "serde", serde(default))]
    pub notch: bool,
    /// Its family, for an input that takes one family whatever its unit (a parameter of a primitive) and for the
    /// value that a wire from it carries; empty otherwise.
    #[cfg_attr(feature = "serde", serde(default))]
    pub family: String,
    /// Why no wire reaches this input yet, when none does (the template of `string.format`, spec 5.4); empty otherwise.
    #[cfg_attr(feature = "serde", serde(default))]
    pub never: String,
    /// As `wire.connect` names it (commands.md 9.2): its `to` for an input (`sq.b`, `msg.parts.2`, `n.after`,
    /// `output y`, the selector `m`, the carry `total.sum`, `n.next.x`, the `until` of `n`, a `collect` or a `yield` by
    /// the output it gives), its `from` for an output (a value, `dmm`, or `node.output`). Empty for what no command wires.
    pub name: String,
    /// The arm of a `yield`, by its patterns as the text writes them (`true`, `_`): what `wire.connect` takes as `arm`.
    /// Empty for another terminal.
    #[cfg_attr(feature = "serde", serde(default))]
    pub arm: String,
    /// Its name in its node (`high`), or in the signature for a terminal of the frame (`limit`); empty for an input
    /// without a name.
    pub caption: String,
    /// Its type, by its index in [`Drawing::types`]; none when `check` does not know it, which takes anything.
    pub ty: Option<usize>,
    /// The constant it holds as the file writes it, escapes kept: what a field in place opens with (notation 2.6). Empty
    /// for another input, or a constant above `LITERAL_BYTES`.
    #[cfg_attr(feature = "serde", serde(default))]
    pub literal: String,
    /// What it holds.
    pub holds: Holds,
    /// The value it gives, or that its wire carries; empty otherwise.
    pub value: String,
    /// The wire at it, by its group in [`Drawing::groups`]: the wire of its value for an output.
    pub group: Option<usize>,
    /// How a wire from it is drawn, in the style of its family (notation 4.1).
    pub style: WireStyle,
    /// The part of a structure that this terminal of its frame stands for, as `structure.remove_part` names it
    /// (`carry x`, `in x`, `collect y`); empty for another terminal.
    #[cfg_attr(feature = "serde", serde(default))]
    pub part: String,
    /// For a value that a frame gives its body (a carry, an `in` clause, an input of the function): whether a wire
    /// leaves it, that is, whether the body reads it. False for another terminal.
    #[cfg_attr(feature = "serde", serde(default))]
    pub read: bool,
}

/// What a terminal holds (notation 2.5): an input is wired, given a constant, left to its default, a hole, or a required
/// input that nothing reaches.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Holds {
    /// An output, or an input that takes nothing yet and needs nothing (`after:`).
    Nothing,
    /// A wire, of the value [`Port::value`].
    Wire,
    /// A constant, as the text writes it.
    Constant(String),
    /// Its default, as the text writes it.
    Default(String),
    /// A hole, `?` (spec 5.1).
    Hole,
    /// A required input that the call leaves out (`LC2101`).
    Missing,
}

/// A drawing: what it draws, in the order of drawing, and its extent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Drawing {
    /// `data-function`.
    pub function: String,
    pub width: i64,
    pub height: i64,
    /// The frame of the function and its terminals.
    pub loose: Vec<Shape>,
    pub groups: Vec<Group>,
    /// The icons that the shapes show, each once.
    pub icons: Vec<IconDrawing>,
    /// Where each node is, by the key that follows it across a change of the file (notation 2.1): for the glide of
    /// the canvas, and the figure of stability. Not in the SVG.
    pub placed: Vec<Placed>,
    /// What the checker of the rules of routing needs of each body (notation 2.2). Not in the SVG.
    pub bodies: Vec<BodyDrawn>,
    /// The marks of diagnostics, drawn over everything else, each in a `<g data-code>` (`docs/cli.md`, 7.2).
    pub marks: Vec<Mark>,
    /// A function that is not drawn: its frame and its diagnostics only, `data-drawn="false"` (`docs/cli.md`, 7.2).
    pub undrawn: bool,
    /// The arms drawn in tabs, and their tabs; none in an export, which unfolds every arm (notation 5.2). Not in the SVG.
    pub arms: Vec<ArmOf>,
    pub tabs: Vec<Tab>,
    /// The tooltips of the terminals. Not in the SVG.
    pub hints: Vec<Hint>,
    /// The terminals where a wire starts or ends, for the wiring of the editor. Not in the SVG.
    #[cfg_attr(feature = "serde", serde(default))]
    pub ports: Vec<Port>,
    /// The types of the ports, as the language writes them, and what the rule of fit reads of each (spec 3.3, 3.4), by
    /// [`Port::ty`]. Not in the SVG.
    #[cfg_attr(feature = "serde", serde(default))]
    pub types: Vec<String>,
    #[cfg_attr(feature = "serde", serde(default))]
    pub type_keys: Vec<labcraft_types::FitKey>,
    /// The values of the notation it was drawn with (notation 10), which the SVG and the canvas paint it with.
    pub style: std::sync::Arc<crate::style::Style>,
    /// What drawing found to say about the file, its warnings (`LC7005`), with no file name: the caller gives it. Not
    /// in the SVG.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub warnings: Vec<labcraft_types::Diagnostic>,
    /// What a trace needs of it: its nodes for the reader, where each wire takes its value from (notation 6.1). Not in
    /// the SVG, nor sent to the web viewer, which shows no trace in M0 (architecture 11.5).
    #[cfg_attr(feature = "serde", serde(skip))]
    pub trace: crate::values::TracePlan,
}

impl Drawing {
    /// Where a node is in its frame, in hundredths, as its `node … at` entry would write it (spec 11): what a move by
    /// `[dx, dy]` of a node placed automatically starts from (architecture 12.3).
    pub fn in_frame(&self, placed: usize) -> Option<(i64, i64)> {
        let node = self.placed.get(placed)?;
        let body = self.bodies.get(node.body)?;
        Some((node.x - body.origin.0, node.y + node.over - body.origin.1))
    }
}

/// The mark of a diagnostic: what shows where it is, and its code.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Mark {
    pub code: String,
    pub shapes: Vec<Shape>,
}

/// A body drawn: the height of its main error chain, whether its file places a node of it, and its frame (`x`, `y`,
/// width, height), which the function has without drawing it (notation 5).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BodyDrawn {
    pub axis: Option<i64>,
    pub from_file: bool,
    pub frame: (i64, i64, i64, i64),
    /// The height of the header at the top of its frame, in hundredths: the title of a loop or a `for`, the patterns of
    /// an arm, which belong to the structure and not to its body (notation 2.6).
    #[cfg_attr(feature = "serde", serde(default))]
    pub header: i64,
    /// For the frame of the function, the size its content needs from its origin, whatever `frame size` (spec 11): the
    /// least that a resize writes; in hundredths, none for another frame.
    #[cfg_attr(feature = "serde", serde(default))]
    pub least: Option<(i64, i64)>,
    /// For the frame of the function, the size that `frame size` fixes (spec 11), in hundredths: what the editor marks
    /// (notation 2.4); none otherwise.
    #[cfg_attr(feature = "serde", serde(default))]
    pub size: Option<(i64, i64)>,
    /// The origin of the coordinates of `layout` in its frame (spec 11), in the coordinates of the drawing.
    #[cfg_attr(feature = "serde", serde(default))]
    pub origin: (i64, i64),
    /// The structure that holds it, by its index in [`Drawing::placed`]; none for the body of the function.
    #[cfg_attr(feature = "serde", serde(default))]
    pub owner: Option<usize>,
    /// Whether it is the body of a `loop` or a `for`, out of which a value goes only by a `collect` or a carry.
    #[cfg_attr(feature = "serde", serde(default))]
    pub looping: bool,
    /// Whether it is the body of a `for`, whose default way out is a `collect` (notation 2.6).
    #[cfg_attr(feature = "serde", serde(default))]
    pub each: bool,
    /// Whether it is the body of a `for parallel`, which takes no carry (spec 6.2).
    #[cfg_attr(feature = "serde", serde(default))]
    pub parallel: bool,
    /// Whether it is the body of a `loop continue_on_error` (spec 6.3).
    #[cfg_attr(feature = "serde", serde(default))]
    pub continue_on_error: bool,
    /// For an arm, its patterns as the text writes them and a command names the arm (`true`, `_`, `1, 3..=5`); empty for
    /// another body.
    #[cfg_attr(feature = "serde", serde(default))]
    pub arm: String,
}

/// A node placed: the top-left corner of its block, its size, and its place in the columns of its body.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Placed {
    pub key: String,
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
    pub column: u32,
    pub rank: u32,
    /// The body that holds it, by its index in the graph.
    pub body: usize,
    /// The bodies of a structure, by their index in the graph: one, or one per arm, in the order of its arms. Empty for
    /// another node.
    pub holds: Vec<usize>,
    /// As an editing command names it (commands.md 9.2), for the selection of the window: its label or the name of its
    /// line, `#close`, `dmm`; empty for a node that has neither (a call statement without a label), which a command
    /// cannot name.
    pub name: String,
    /// The line of the file where it starts, from 1, which the `Text` view highlights when it is selected (notation
    /// 2.5); 0 when the file does not say.
    pub line: u32,
    /// Where its `node … at` entry fixes it, in hundredths, from the top-left corner of its frame (spec 11); none for a
    /// node placed automatically.
    pub at: Option<(i64, i64)>,
    /// The view that its entry of `layout` writes, `standard` or `compact` (spec 11); empty when it writes none.
    #[cfg_attr(feature = "serde", serde(default))]
    pub view: String,
    /// How far the frame of a structure rises above the corner that its entry places (its box starts that much higher):
    /// a loop, or a `match` or a `handle` in tabs, whose body grows upward (`Sized::over`); in hundredths, 0 otherwise.
    #[cfg_attr(feature = "serde", serde(default))]
    pub over: i64,
    /// For a structure, the size its content needs from the corner its entry places (its box less `over`), whatever its
    /// `frame … size` entry (spec 11): the least that a resize writes; in hundredths, (0, 0) for another node.
    #[cfg_attr(feature = "serde", serde(default))]
    pub least: (i64, i64),
    /// For a structure, the size that its `frame … size` entry fixes (spec 11), in hundredths: what the editor marks
    /// (notation 2.4); none for a structure of automatic size and for another node.
    #[cfg_attr(feature = "serde", serde(default))]
    pub size: Option<(i64, i64)>,
}

/// The token of a run of text, from its ink (notation 2.3).
pub fn token_of(ink: Ink, text: Token) -> Token {
    match ink {
        Ink::Text => text,
        Ink::Invalid => "mark.invalid",
    }
}
