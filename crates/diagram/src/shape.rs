//! What a drawing is made of (`docs/architecture.md`, 11.3): shapes with the name of their theme token, in integer
//! hundredths of a unit of the canvas. The canvas and the SVG paint only these.

use labcraft_types::text::Ink;

/// A token of the theme (notation 8), by its name: `node.fill`.
pub type Token = &'static str;

/// How a shape is painted: a fill, an outline and its width, each optional.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Paint {
    pub fill: Option<Token>,
    pub stroke: Option<Token>,
    /// In hundredths of a unit.
    pub width: i64,
}

impl Paint {
    pub const fn fill(token: Token) -> Paint {
        Paint { fill: Some(token), stroke: None, width: 0 }
    }

    pub const fn stroke(token: Token, width: i64) -> Paint {
        Paint { fill: None, stroke: Some(token), width }
    }

    pub const fn both(fill: Token, stroke: Token, width: i64) -> Paint {
        Paint { fill: Some(fill), stroke: Some(stroke), width }
    }
}

/// A run of a text in one token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub token: Token,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
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
pub struct WireStyle {
    pub token: Token,
    /// In hundredths of a unit.
    pub width: i64,
    pub pattern: Pattern,
    /// A struct is sheathed: its core, in the fill of the frame it runs in.
    pub core: Option<Token>,
    /// An array has a cross-tick per dimension every 8 units.
    pub ticks: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pattern {
    Solid,
    /// An error: chevrons every 24 units, in the direction of the flow.
    Chevrons,
    /// An order wire, thin and dotted.
    Dotted,
    /// A function reference.
    DashDot,
}

/// The radius of the corners of a wire (notation 2).
pub const WIRE_RADIUS: i64 = 600;
/// Between two chevrons of an error wire, and from its start to the first.
pub const CHEVRON_STEP: i64 = 2400;

/// The chevrons of an error wire along its polyline: each three points, pointing in the direction of the flow.
pub fn chevrons(points: &[(i64, i64)]) -> Vec<[(i64, i64); 3]> {
    let mut found = Vec::new();
    let mut next = CHEVRON_STEP / 2;
    let mut walked = 0i64;
    for pair in points.windows(2) {
        let (Some(&(x0, y0)), Some(&(x1, y1))) = (pair.first(), pair.get(1)) else { continue };
        let (dx, dy) = ((x1 - x0).signum(), (y1 - y0).signum());
        let length = (x1 - x0).abs() + (y1 - y0).abs();
        // Not on a corner, where the wire bends.
        while next <= walked + length - WIRE_RADIUS {
            let along = next - walked;
            if along >= WIRE_RADIUS {
                let (x, y) = (x0 + dx * along, y0 + dy * along);
                let (back, side) = (200, 300);
                found.push([
                    (x - dx * back - dy * side, y - dy * back + dx * side),
                    (x + dx * back, y + dy * back),
                    (x - dx * back + dy * side, y - dy * back - dx * side),
                ]);
            }
            next += CHEVRON_STEP;
        }
        walked += length;
        next = next.max(walked + WIRE_RADIUS);
    }
    found
}

/// An icon as the SVG draws it: its grid and its items, in the numbers that the file writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IconDrawing {
    pub grid: String,
    pub items: Vec<IconItem>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IconItem {
    pub shape: IconShape,
    pub fill: bool,
    pub token: Token,
}

#[derive(Clone, Debug, PartialEq, Eq)]
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

/// What a group of shapes stands for, in the forms of the reports (`docs/cli.md`, 7.2).
#[derive(Clone, Debug, PartialEq, Eq)]
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
}

/// What a wire stands for (`docs/cli.md`, 7.2): `data-wire`, `data-from`, `data-to`.
#[derive(Clone, Debug, PartialEq, Eq)]
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
}

/// A drawing: what it draws, in the order of drawing, and its extent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
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
}

/// A body drawn: the height of its main error chain, and whether its file places a node of it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BodyDrawn {
    pub axis: Option<i64>,
    pub from_file: bool,
}

/// A node placed: the top-left corner of its block, its size, and its place in the columns of its body.
#[derive(Clone, Debug, PartialEq, Eq)]
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
}

/// The token of a run of text, from its ink (notation 2.3).
pub fn token_of(ink: Ink, text: Token) -> Token {
    match ink {
        Ink::Text => text,
        Ink::Invalid => "mark.invalid",
    }
}
