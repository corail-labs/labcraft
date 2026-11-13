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

/// How wide the stroke of a chevron cut into an error wire is, in hundredths of a unit.
pub const CHEVRON_STROKE: i64 = 90;

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
}

/// The token of a run of text, from its ink (notation 2.3).
pub fn token_of(ink: Ink, text: Token) -> Token {
    match ink {
        Ink::Text => text,
        Ink::Invalid => "mark.invalid",
    }
}
