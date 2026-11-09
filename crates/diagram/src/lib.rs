//! L5: the drawing of a function as the visual notation says (`docs/visual-notation.md`; `docs/architecture.md`,
//! 11.3), computed once for the canvas and the SVG alike. It reads what `check` resolved and never resolves again.
//! Headless: no egui here.
//!
//! So far, the first two steps of 11.3: the graph of the function and the size of each node, compact or standard,
//! with the SVG of the boxes. Until placement comes (step 3 of M0v), each body is drawn as a row of its nodes in
//! canonical order, and a structure is drawn by its bodies only.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable,
    clippy::indexing_slicing,
    clippy::string_slice
)]
#![forbid(unsafe_code)]

use std::collections::{BTreeMap, HashMap};

use labcraft_check::Checked;
use labcraft_types::text::{ShownText, cut_middle};
use labcraft_types::{Code, Diagnostic, Interner};

mod graph;
mod shape;
mod size;
mod svg;
pub mod theme;

pub use shape::{Drawing, Group, IconDrawing, IconItem, IconShape, Paint, Run, Shape, Token};
pub use svg::{FONT_FAMILY, SCHEMA, Theme, svg};

use graph::{Builder, Graph, Kind};
use size::{ANY_TEXT, LABEL_LINE, MEDIUM, Sizer, Text};

/// What placement depends on, of the view (notation 2.1). So far, the preference of the IDE that shows every node in
/// standard view (notation 3); the view of each node is in its file.
///
/// The default is the view state of every exported image (cli.md 7.2): constructions as their block.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ViewState {
    pub standard_everywhere: bool,
    /// A construction whose fields are all constants drawn folded, as a small node (notation 3.1): the default of the
    /// viewer, never of an export.
    pub fold_constructions: bool,
}

/// `--max-diagram-nodes` (`docs/cli.md`, 7.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiagramLimits {
    pub max_nodes: u64,
}

impl DiagramLimits {
    /// The default; above it, the bounds of scenario 4 are not promised.
    pub const DEFAULT_NODES: u64 = 20_000;
    /// The most that `--max-diagram-nodes` takes *(proposed)*.
    pub const MAX_NODES: u64 = 100_000;
}

impl Default for DiagramLimits {
    fn default() -> Self {
        DiagramLimits { max_nodes: Self::DEFAULT_NODES }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Drawn {
    Drawing(Drawing),
    /// `LC7002`, or a file that declares no function.
    NotDrawn(Vec<Diagnostic>),
}

/// The margin around what is drawn: 16 units (`docs/cli.md`, 7.2).
const MARGIN: i64 = 1600;
/// Between two nodes of a row, and between two rows.
const GAP: i64 = 1600;
const ROWS: i64 = 2400;

/// The drawing of the function of `checked.files[file]`.
pub fn draw(checked: &Checked, file: usize, names: &Interner, view: &ViewState, limits: &DiagramLimits) -> Drawn {
    let Some(checked_file) = checked.files.get(file) else { return Drawn::NotDrawn(Vec::new()) };
    if let labcraft_graph::Decl::Extern(external) = &checked_file.file.decl {
        // A native function has a frame and no body.
        let builder = Builder::new(checked, checked_file, names);
        return Drawn::Drawing(rows(&builder.empty(external.name), names, view));
    }
    let Some(function) = graph::function_of(checked_file) else { return Drawn::NotDrawn(Vec::new()) };
    let orders = graph::orders_of(checked_file, function, names);
    let too_large = |message: String| Drawn::NotDrawn(vec![Diagnostic::new(Code::Lc7002, message, checked_file.shown.clone())]);
    let nodes = graph::count_nodes(function, &orders, &checked_file.resolved);
    if nodes > limits.max_nodes {
        return too_large(format!("the function is too large to draw: {nodes} nodes, for at most {} (--max-diagram-nodes)", limits.max_nodes));
    }
    let max_terminals = limits.max_nodes.saturating_mul(5);
    let mut builder = Builder::new(checked, checked_file, names).folding(view.fold_constructions);
    match builder.graph(function, &orders, max_terminals) {
        Ok(graph) => Drawn::Drawing(rows(&graph, names, view)),
        Err(graph::TooLarge(counts)) => too_large(format!(
            "the function is too large to draw: more than {max_terminals} terminals, counted on its first {} nodes, for at most five \
             times {} (--max-diagram-nodes)",
            counts.nodes, limits.max_nodes
        )),
    }
}

/// The nodes that `LC7002` counts of the function of `checked.files[file]`, for the report of `diagram` (cli.md 7.5).
pub fn count_nodes(checked: &Checked, file: usize, names: &Interner) -> Option<u64> {
    let checked_file = checked.files.get(file)?;
    let function = graph::function_of(checked_file)?;
    Some(graph::count_nodes(function, &graph::orders_of(checked_file, function, names), &checked_file.resolved))
}

/// The band of each node (notation 3.2): the last segment of its library, cut at 12 cells; the full path when two
/// libraries of the drawing would read the same; cut in the middle when they still do.
fn bands(graph: &Graph<'_>) -> Vec<Option<ShownText>> {
    let library_of = |kind: &Kind<'_>| match kind {
        Kind::Call { library, glyph: None, .. } | Kind::Construction { library, .. } | Kind::Variant { library, .. } => library.clone(),
        _ => None,
    };
    let libraries: Vec<Option<String>> = graph.nodes.iter().map(|node| library_of(&node.kind)).collect();
    let mut distinct: Vec<&String> = libraries.iter().flatten().collect();
    distinct.sort();
    distinct.dedup();
    let short = |library: &str| labcraft_types::text::cut(library.rsplit('.').next().unwrap_or(library), LABEL_LINE).into_owned();
    let mut drawn: BTreeMap<&String, String> = distinct.iter().map(|&library| (library, short(library))).collect();
    for pass in 0..2 {
        let mut readers: BTreeMap<String, Vec<&String>> = BTreeMap::new();
        for (library, text) in &drawn {
            readers.entry(text.clone()).or_default().push(library);
        }
        for same in readers.values().filter(|libraries| libraries.len() > 1) {
            for &library in same {
                let text = if pass == 0 {
                    labcraft_types::text::cut(library, LABEL_LINE).into_owned()
                } else {
                    cut_middle(library, LABEL_LINE).into_owned()
                };
                drawn.insert(library, text);
            }
        }
    }
    libraries.iter().map(|library| library.as_ref().and_then(|library| drawn.get(library)).map(|text| ShownText::plain(text))).collect()
}

/// What the tooltip of a node says (notation 3.2), cut at 200 code points (cli.md 7.2).
fn title(kind: &Kind<'_>) -> String {
    let text = match kind {
        Kind::Call { qualified, .. } => qualified.clone(),
        Kind::Construction { ty, library } => library.as_ref().map_or_else(|| ty.clone(), |library| format!("{library}.{ty}")),
        Kind::Variant { ty, variant, library } => match library {
            Some(library) => format!("{library}.{ty}.{variant}"),
            None => format!("{ty}.{variant}"),
        },
        Kind::Ui { operation, element } => format!("ui.{operation} {element}"),
        Kind::Constant { name, value } => format!("{name} = {value}"),
        Kind::Structure => String::new(),
    };
    labcraft_types::cut_text(&text).into_owned()
}

/// Step 2 of M0v only: each body as a row, under its caption, its nodes in canonical order.
fn rows(graph: &Graph<'_>, names: &Interner, view: &ViewState) -> Drawing {
    let sizer = Sizer { names, standard_everywhere: view.standard_everywhere };
    let bands = bands(graph);
    let mut drawing = Drawing { function: graph.function.clone(), ..Drawing::default() };
    // Each icon once in the drawing, however many calls show it.
    let mut icons: HashMap<*const labcraft_graph::Icon, usize> = HashMap::new();
    let mut y = MARGIN;
    let mut right = MARGIN;
    for body in &graph.bodies {
        let caption = match body.arm {
            None => Text::new(&body.caption, ANY_TEXT, MEDIUM),
            Some(number) => {
                let suffix = format!(", arm {number}");
                let room = ANY_TEXT.saturating_sub(labcraft_types::text::cells(&suffix));
                Text::measured(labcraft_types::text::show(&body.caption, room).then(&suffix), MEDIUM)
            }
        };
        let line = labcraft_types::text::scale(u64::from(labcraft_types::text::ASCENT + labcraft_types::text::DESCENT), MEDIUM).unwrap_or(0);
        drawing.loose.push(caption_shape(&caption, MARGIN, y + line / 2));
        right = right.max(MARGIN + caption.width);
        y += line + GAP;
        let sized: Vec<(usize, size::Sized)> = body
            .nodes
            .iter()
            .filter_map(|&index| {
                let node = graph.nodes.get(index)?;
                if matches!(node.kind, Kind::Structure) {
                    return None;
                }
                let icon = match &node.kind {
                    Kind::Call { glyph: None, icon: Some(icon), .. } if !(view.standard_everywhere || node.standard) => {
                        Some(*icons.entry(std::ptr::from_ref(*icon)).or_insert_with(|| {
                            drawing.icons.push(sizer.icon(icon));
                            drawing.icons.len() - 1
                        }))
                    }
                    _ => None,
                };
                Some((index, sizer.size(node, bands.get(index).and_then(Option::as_ref), icon)))
            })
            .collect();
        let above = sized.iter().map(|(_, sized)| sized.above).max().unwrap_or(0);
        let tallest = sized.iter().map(|(_, sized)| sized.height).max().unwrap_or(0);
        let top = y + above;
        let mut x = MARGIN;
        for (index, sized) in sized {
            let Some(node) = graph.nodes.get(index) else { continue };
            x += sized.left;
            let shapes = sized.shapes.into_iter().map(|shape| moved(shape, x, top)).collect();
            drawing.groups.push(Group { node: node.key.clone(), title: title(&node.kind), shapes });
            x += sized.width + sized.right + GAP;
            right = right.max(x - GAP);
        }
        y = top + tallest + ROWS;
    }
    drawing.width = right + MARGIN;
    drawing.height = y - ROWS + MARGIN;
    drawing
}

fn caption_shape(caption: &Text, x: i64, middle: i64) -> Shape {
    let cap = labcraft_types::text::scale(u64::from(labcraft_types::text::CAP_HEIGHT), caption.size).unwrap_or(0);
    Shape::Text {
        x,
        y: middle + (cap + 1) / 2,
        size: caption.size,
        width: caption.width,
        runs: caption.shown.runs.iter().map(|(ink, text)| Run { token: shape::token_of(*ink, "ink.muted"), text: text.clone() }).collect(),
    }
}

/// A shape moved by `(dx, dy)`.
fn moved(shape: Shape, dx: i64, dy: i64) -> Shape {
    match shape {
        Shape::Rect { x, y, width, height, radius, paint } => Shape::Rect { x: x + dx, y: y + dy, width, height, radius, paint },
        Shape::Circle { x, y, radius, paint } => Shape::Circle { x: x + dx, y: y + dy, radius, paint },
        Shape::Poly { points, closed, paint } => Shape::Poly { points: points.into_iter().map(|(x, y)| (x + dx, y + dy)).collect(), closed, paint },
        Shape::Text { x, y, size, width, runs } => Shape::Text { x: x + dx, y: y + dy, size, width, runs },
        Shape::Icon { x, y, side, icon } => Shape::Icon { x: x + dx, y: y + dy, side, icon },
    }
}

#[cfg(test)]
mod tests;
