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
}

/// A drawing: what it draws, in the order of drawing, and its extent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Drawing {
    /// `data-function`.
    pub function: String,
    pub width: i64,
    pub height: i64,
    /// Shapes that stand for nothing of the program: the captions of the bodies until they are placed.
    pub loose: Vec<Shape>,
    pub groups: Vec<Group>,
    /// The icons that the shapes show, each once.
    pub icons: Vec<IconDrawing>,
}

/// The token of a run of text, from its ink (notation 2.3).
pub fn token_of(ink: Ink, text: Token) -> Token {
    match ink {
        Ink::Text => text,
        Ink::Invalid => "mark.invalid",
    }
}
