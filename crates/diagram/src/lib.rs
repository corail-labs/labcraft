//! L5: the drawing of a function as the visual notation says (`docs/visual-notation.md`; `docs/architecture.md`,
//! 11.3), computed once for the canvas and the SVG alike. It reads what `check` resolved and never resolves again.
//! Headless: no egui here.
//!
//! So far, the first three steps of 11.3: the graph of the function with its wires, the size of each node, compact or
//! standard, and placement, with the SVG of the frames and the nodes. The wires are drawn once they are routed (step 4
//! of M0v).
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
mod marks;
mod names;
mod place;
mod route;
mod shape;
mod size;
mod svg;
pub mod theme;

pub use marks::{MAX_LINES, mark, undrawn};
pub use shape::{BodyDrawn, Drawing, Group, IconDrawing, IconItem, IconShape, Mark, Paint, Pattern, Placed, Run, Shape, Token, WireOf, WireStyle};
pub use svg::{FONT_FAMILY, SCHEMA, Theme, svg};

use graph::{Base, Builder, Graph, Kind, StructureKind};
use size::{ANY_TEXT, GRID, LABEL_LINE, LARGE, MEDIUM, SMALL, Sizer, Text};

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

/// The drawing of the function of `checked.files[file]`.
pub fn draw(checked: &Checked, file: usize, names: &Interner, view: &ViewState, limits: &DiagramLimits) -> Drawn {
    let Some(checked_file) = checked.files.get(file) else { return Drawn::NotDrawn(Vec::new()) };
    if let labcraft_graph::Decl::Extern(external) = &checked_file.file.decl {
        // A native function has a frame and no body.
        let builder = Builder::new(checked, checked_file, names);
        return Drawn::Drawing(drawing(&builder.empty(external.name, &external.signature), names, view));
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
        Ok(graph) => Drawn::Drawing(drawing(&graph, names, view)),
        Err(graph::TooLarge(counts)) => too_large(format!(
            "the function is too large to draw: more than {max_terminals} terminals, counted on its first {} nodes, for at most five \
             times {} (--max-diagram-nodes)",
            counts.nodes, limits.max_nodes
        )),
    }
}

/// A data wire between two nodes of one body, by where its reader reads it: where the figure of stability puts a
/// neutral node (notation 2.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DataWire {
    /// The path that reads the value.
    pub read: labcraft_graph::Id,
    /// The line of the reader.
    pub statement: labcraft_graph::Id,
}

/// The data wires between two nodes of one body of the function of `checked.files[file]`; none for a function that
/// `LC7002` refuses.
pub fn data_wires(checked: &Checked, file: usize, names: &Interner, limits: &DiagramLimits) -> Vec<DataWire> {
    let Some(checked_file) = checked.files.get(file) else { return Vec::new() };
    let Some(function) = graph::function_of(checked_file) else { return Vec::new() };
    let orders = graph::orders_of(checked_file, function, names);
    if graph::count_nodes(function, &orders, &checked_file.resolved) > limits.max_nodes {
        return Vec::new();
    }
    let mut builder = Builder::new(checked, checked_file, names);
    let Ok(graph) = builder.graph(function, &orders, limits.max_nodes.saturating_mul(5)) else { return Vec::new() };
    let data =
        |family: graph::Family| family.dimensions > 0 || matches!(family.base, Base::Number | Base::Bool | Base::Text | Base::Enum | Base::Struct);
    graph
        .bodies
        .iter()
        .flat_map(|body| body.wires.iter())
        .filter_map(|wire| match (wire.from, wire.to, wire.read) {
            (graph::End::Node { .. }, graph::End::Node { node, .. }, Some(read)) if data(wire.family) => {
                Some(DataWire { read, statement: graph.nodes.get(node)?.statement })
            }
            _ => None,
        })
        .collect()
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
        Kind::Structure(structure) => structure.header.clone(),
    };
    labcraft_types::cut_text(&text).into_owned()
}

/// The drawing of a graph: each node in its view, placed (notation 2.1), inside the frames of its bodies.
fn drawing(graph: &Graph<'_>, names: &Interner, view: &ViewState) -> Drawing {
    let sizer = Sizer { names, standard_everywhere: view.standard_everywhere };
    let bands = bands(graph);
    let mut drawing = Drawing { function: graph.function.clone(), ..Drawing::default() };
    // Each icon once in the drawing, however many calls show it.
    let mut icons: HashMap<*const labcraft_graph::Icon, usize> = HashMap::new();
    let mut sized: Vec<size::Sized> = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            let icon = match &node.kind {
                Kind::Call { glyph: None, icon: Some(icon), .. } if !(view.standard_everywhere || node.standard) => {
                    Some(*icons.entry(std::ptr::from_ref(*icon)).or_insert_with(|| {
                        drawing.icons.push(sizer.icon(icon));
                        drawing.icons.len() - 1
                    }))
                }
                _ => None,
            };
            sizer.size(node, bands.get(index).and_then(Option::as_ref), icon)
        })
        .collect();
    // The pill of a wire stands left of its reader, as the pill of an inline constant does (notation 3.2).
    for (&(body, wire), pill) in &graph.pills {
        let Some(graph::End::Node { node, .. }) = graph.bodies.get(body).and_then(|of| of.wires.get(wire)).map(|wire| wire.to) else { continue };
        if let Some(found) = sized.get_mut(node) {
            found.left = found.left.max(pill_reach(pill));
        }
    }
    let titles = titles(graph);
    let placement = place::place(graph, &mut sized, &titles, (MARGIN, MARGIN));
    // The function's frame, then each node, a structure with its frame before what it holds.
    if let (Some(body), Some(frame)) = (graph.bodies.first(), placement.frames.first()) {
        let mut shapes = frame_shapes(frame, &Text::new(&body.caption, ANY_TEXT, LARGE), None, false, "frame.stroke");
        frame_terminals(body, frame, true, &mut shapes);
        drawing.loose = shapes;
    }
    // The wires of each body, drawn over its frame and under its nodes.
    let mut wires: Vec<Vec<Group>> = (0..graph.bodies.len()).map(|body| wire_groups(graph, body, &placement, &sized)).collect();
    if let Some(first) = wires.first_mut() {
        drawing.groups.append(first);
    }
    for (index, (node, sized)) in graph.nodes.iter().zip(sized).enumerate() {
        let Some(Some(at)) = placement.nodes.get(index) else { continue };
        let mut shapes: Vec<Shape> = sized.shapes.into_iter().map(|shape| moved(shape, at.x, at.y)).collect();
        let mut arms = Vec::new();
        if let Kind::Structure(structure) = &node.kind {
            // The frame, then the wires in the margins of a `match`.
            let margins = std::mem::take(&mut shapes);
            (shapes, arms) = structure_shapes(graph, node, structure, &placement, at, (sized.width, &sized.inputs, &sized.outputs));
            shapes.extend(margins);
            shapes.extend(sized_label(node, at));
        }
        let (width, height) = (sized.width, sized.height);
        drawing.placed.push(Placed {
            key: node.stable.clone(),
            x: at.x,
            y: at.y,
            width,
            height,
            column: at.column,
            rank: at.rank,
            body: node.body,
            holds: node.bodies.clone(),
        });
        drawing.groups.push(Group { node: node.key.clone(), title: title(&node.kind), shapes, arm: None, wire: None });
        drawing.groups.extend(arms.into_iter().map(|(arm, shapes)| Group {
            node: String::new(),
            title: String::new(),
            shapes,
            arm: Some(arm),
            wire: None,
        }));
        for &body in &node.bodies {
            if let Some(list) = wires.get_mut(body) {
                drawing.groups.append(list);
            }
        }
    }
    // The extent: everything, and the margin around it (cli.md 7.2).
    let (mut right, mut bottom) = (0i64, 0i64);
    let (mut left, mut top) = (i64::MAX, i64::MAX);
    for shape in drawing.loose.iter().chain(drawing.groups.iter().flat_map(|group| group.shapes.iter())) {
        let (x0, y0, x1, y1) = extent(shape);
        (left, top, right, bottom) = (left.min(x0), top.min(y0), right.max(x1), bottom.max(y1));
    }
    if left == i64::MAX {
        (left, top) = (MARGIN, MARGIN);
    }
    // A node that the file places left of the frame or above it moves the whole drawing.
    let (dx, dy) = ((MARGIN - left).max(0), (MARGIN - top).max(0));
    if dx > 0 || dy > 0 {
        drawing.loose = std::mem::take(&mut drawing.loose).into_iter().map(|shape| moved(shape, dx, dy)).collect();
        for group in &mut drawing.groups {
            group.shapes = std::mem::take(&mut group.shapes).into_iter().map(|shape| moved(shape, dx, dy)).collect();
            for (_, points) in group.wire.iter_mut().flat_map(|wire| wire.readers.iter_mut()) {
                points.iter_mut().for_each(|point| *point = (point.0 + dx, point.1 + dy));
            }
        }
        for placed in &mut drawing.placed {
            (placed.x, placed.y) = (placed.x + dx, placed.y + dy);
        }
    }
    drawing.bodies = placement.frames.iter().map(|frame| BodyDrawn { axis: frame.axis.map(|axis| axis + dy), from_file: frame.from_file }).collect();
    drawing.width = right + dx + MARGIN;
    drawing.height = bottom + dy + MARGIN;
    drawing
}

/// The label of a structure, above its frame (notation 3.1).
fn sized_label(node: &graph::Node<'_>, at: &place::NodeAt) -> Option<Shape> {
    let label = node.label.as_ref()?;
    let text = Text::new(&format!("{label}:"), ANY_TEXT, MEDIUM);
    Some(text.at(at.x, at.y - place::LABEL_ABOVE - size::line_height(MEDIUM) / 2, "ink.muted"))
}

/// The widths that titles need: the name of the function, a header and its badge, the patterns of an arm.
fn titles(graph: &Graph<'_>) -> place::Titles {
    let mut titles = place::Titles { bodies: vec![0; graph.bodies.len()], structures: HashMap::new() };
    let header_width = |structure: &graph::Structure| {
        let header = Text::new(&structure.header, ANY_TEXT, LARGE).width;
        let badge = structure.badge.map_or(0, |badge| GRID + Text::new(badge, ANY_TEXT, MEDIUM).width + 2 * size::PADDING);
        2 * place::LABEL_START + header + badge
    };
    for (index, body) in graph.bodies.iter().enumerate() {
        let width = match body.owner.and_then(|owner| graph.nodes.get(owner)).map(|owner| &owner.kind) {
            None => 2 * place::LABEL_START + Text::new(&body.caption, ANY_TEXT, LARGE).width,
            Some(Kind::Structure(structure)) if matches!(structure.kind, StructureKind::Loop | StructureKind::For) => header_width(structure),
            Some(_) => 2 * place::LABEL_START + Text::new(&body.caption, ANY_TEXT, MEDIUM).width,
        };
        if let Some(slot) = titles.bodies.get_mut(index) {
            *slot = width;
        }
    }
    for (index, node) in graph.nodes.iter().enumerate() {
        if let Kind::Structure(structure) = &node.kind
            && matches!(structure.kind, StructureKind::Match | StructureKind::Handle)
        {
            titles.structures.insert(index, header_width(structure));
        }
    }
    titles
}

/// A frame: its box, its title in its header, a badge.
fn frame_shapes(frame: &place::FrameAt, title: &Text, badge: Option<&str>, double: bool, stroke: Token) -> Vec<Shape> {
    let level = (frame.depth + 1).min(4);
    let fill = match level {
        1 => "frame.fill.1",
        2 => "frame.fill.2",
        3 => "frame.fill.3",
        _ => "frame.fill.4",
    };
    let mut shapes = vec![Shape::Rect {
        x: frame.x,
        y: frame.y,
        width: frame.width,
        height: frame.height,
        radius: size::RADIUS,
        paint: Paint::both(fill, stroke, size::OUTLINE),
    }];
    if double {
        shapes.push(Shape::Rect {
            x: frame.x + 300,
            y: frame.y + 300,
            width: frame.width - 600,
            height: frame.height - 600,
            radius: size::RADIUS,
            paint: Paint::stroke(stroke, size::OUTLINE),
        });
    }
    let middle = frame.y + frame.header / 2;
    shapes.push(title.at(frame.x + place::LABEL_START, middle, "node.title"));
    if let Some(badge) = badge {
        let text = Text::new(badge, ANY_TEXT, MEDIUM);
        let x = frame.x + place::LABEL_START + title.width + GRID;
        shapes.push(Shape::Rect {
            x,
            y: middle - GRID,
            width: text.width + 2 * size::PADDING,
            height: 2 * GRID,
            radius: 300,
            paint: Paint::fill("badge.fill"),
        });
        shapes.push(text.at(x + size::PADDING, middle, "badge.text"));
    }
    shapes
}

/// The terminals of the frame of a body on its edges, their names inside (notation 2, 5).
fn frame_terminals(body: &graph::BodyOf, frame: &place::FrameAt, named: bool, shapes: &mut Vec<Shape>) {
    for (side, terminals, heights) in [(false, &body.left, &frame.left), (true, &body.right, &frame.right)] {
        let x = if side { frame.x + frame.width } else { frame.x };
        for (terminal, &height) in terminals.iter().zip(heights) {
            let y = frame.y + height;
            shapes.extend(edge_terminal(terminal, x, y));
            if !named {
                continue;
            }
            let label = place::TerminalLabel::of(terminal);
            let width = label.width();
            let mut at = if side { x - place::LABEL_START - width } else { x + place::LABEL_START };
            shapes.push(label.name.at(at, y, "node.title"));
            at += label.name.width;
            if let Some(unit) = &label.unit {
                shapes.push(unit.at(at + size::PADDING, y, "ink.muted"));
                at += size::PADDING + unit.width;
            }
            if let Some(pill) = &label.pill {
                shapes.extend(size::pill(pill, at + GRID, y));
            }
        }
    }
}

/// A terminal of a frame: a register is a box across the edge, a notch a bar, the others the shape of their family.
fn edge_terminal(terminal: &graph::FrameTerminal, x: i64, y: i64) -> Vec<Shape> {
    let family = terminal.terminal.family;
    match terminal.edge {
        graph::Edge::Register => {
            let token = size::wire_token(family);
            let mark = Text::new("\u{21BB}", 1, SMALL);
            vec![
                Shape::Rect { x: x - 700, y: y - 700, width: 1400, height: 1400, radius: 0, paint: Paint::both("node.fill", token, 150) },
                mark.centred(x, y, "node.title"),
            ]
        }
        graph::Edge::Notch => {
            vec![Shape::Rect { x: x - 200, y: y - 700, width: 400, height: 1400, radius: 0, paint: Paint::fill(size::wire_token(family)) }]
        }
        _ => size::terminal(family, x, y, 100, false),
    }
}

/// The width of a block and where its inputs and outputs are.
type Terminals<'a> = (i64, &'a [(i64, i64)], &'a [(i64, i64)]);

/// A structure: its frame with its header, the terminals of its bodies; the selector, the notches and the outputs of a
/// `match` on its own edges, and its arms unfolded, each apart, with its title.
fn structure_shapes(
    graph: &Graph<'_>,
    node: &graph::Node<'_>,
    structure: &graph::Structure,
    placement: &place::Placement,
    at: &place::NodeAt,
    (width, inputs, outputs): Terminals<'_>,
) -> (Vec<Shape>, Vec<(String, Vec<Shape>)>) {
    let title = Text::new(&structure.header, ANY_TEXT, LARGE);
    let stroke = if structure.kind == StructureKind::Handle { "wire.error" } else { "frame.stroke" };
    match structure.kind {
        StructureKind::Loop | StructureKind::For => {
            let Some((&index, frame)) = node.bodies.first().and_then(|index| Some((index, placement.frames.get(*index)?))) else {
                return (Vec::new(), Vec::new());
            };
            let mut shapes = frame_shapes(frame, &title, structure.badge, structure.parallel, stroke);
            if let Some(body) = graph.bodies.get(index) {
                frame_terminals(body, frame, true, &mut shapes);
            }
            for (input, &(_, y)) in node.inputs.iter().zip(inputs) {
                if let Some(constant) = &input.constant {
                    let text = Text::new(constant, size::COMPACT_CONSTANT, MEDIUM);
                    shapes.extend(size::pill(&text, at.x - place::constant_reach(constant), at.y + y));
                }
            }
            (shapes, Vec::new())
        }
        StructureKind::Match | StructureKind::Handle => {
            let depth = node.bodies.first().and_then(|index| placement.frames.get(*index)).map_or(1, |frame| frame.depth.saturating_sub(1));
            let height = node
                .bodies
                .last()
                .and_then(|index| placement.frames.get(*index))
                .map_or(place::HEADER + place::PAD, |frame| frame.y + frame.height - at.y + place::PAD);
            let outer = place::FrameAt { x: at.x, y: at.y, width, height, header: place::HEADER, depth, ..place::FrameAt::default() };
            let mut shapes = frame_shapes(&outer, &title, structure.badge, false, stroke);
            // The selector, then the notches of the outer values that the arms read.
            for (index, (input, &(x, y))) in node.inputs.iter().zip(inputs).enumerate() {
                let edge = if index == 0 { graph::Edge::Input } else { graph::Edge::Notch };
                shapes.extend(edge_terminal(&graph::FrameTerminal { terminal: input.clone(), edge }, at.x + x, at.y + y));
            }
            for (output, &(x, y)) in node.outputs.iter().zip(outputs) {
                shapes.extend(size::terminal(output.family, at.x + x, at.y + y, 100, output.ignored));
            }
            let mut arms = Vec::new();
            for &index in &node.bodies {
                let (Some(body), Some(frame)) = (graph.bodies.get(index), placement.frames.get(index)) else { continue };
                let mut shapes = Vec::new();
                let caption = Text::new(&body.caption, ANY_TEXT, MEDIUM);
                let level = (frame.depth + 1).min(4);
                let fill = match level {
                    1 => "frame.fill.1",
                    2 => "frame.fill.2",
                    3 => "frame.fill.3",
                    _ => "frame.fill.4",
                };
                shapes.push(Shape::Rect {
                    x: frame.x,
                    y: frame.y,
                    width: frame.width,
                    height: frame.height,
                    radius: size::RADIUS,
                    paint: Paint::both(fill, stroke, size::OUTLINE),
                });
                shapes.push(caption.at(frame.x + place::LABEL_START, frame.y + frame.header / 2, "ink.muted"));
                frame_terminals(body, frame, true, &mut shapes);
                arms.push((body.caption.clone(), shapes));
            }
            (shapes, arms)
        }
    }
}

/// The box that a shape covers.
fn extent(shape: &Shape) -> (i64, i64, i64, i64) {
    match shape {
        Shape::Rect { x, y, width, height, .. } => (*x, *y, x + width, y + height),
        Shape::Circle { x, y, radius, .. } => (x - radius, y - radius, x + radius, y + radius),
        Shape::Poly { points, .. } => {
            points.iter().fold((i64::MAX, i64::MAX, i64::MIN, i64::MIN), |(a, b, c, d), &(x, y)| (a.min(x), b.min(y), c.max(x), d.max(y)))
        }
        Shape::Text { x, y, size, width, .. } => (*x, y - size::line_height(*size), x + width, *y),
        Shape::Icon { x, y, side, .. } => (*x, *y, x + side, y + side),
        Shape::Wire { points, style } => {
            let half = style.width.max(600) / 2;
            let (a, b, c, d) =
                points.iter().fold((i64::MAX, i64::MAX, i64::MIN, i64::MIN), |(a, b, c, d), &(x, y)| (a.min(x), b.min(y), c.max(x), d.max(y)));
            (a - half, b - half, c + half, d + half)
        }
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
        Shape::Wire { points, style } => Shape::Wire { points: points.into_iter().map(|(x, y)| (x + dx, y + dy)).collect(), style },
    }
}

/// The fill of a frame at a depth of nesting (notation 8).
fn frame_fill(depth: u32) -> Token {
    match (depth + 1).min(4) {
        1 => "frame.fill.1",
        2 => "frame.fill.2",
        3 => "frame.fill.3",
        _ => "frame.fill.4",
    }
}

/// The wires of a body, one group per value: its branches, each from the source to one reader, the trunk drawn by each,
/// and its junction points (notation 2.2, 4).
fn wire_groups(graph: &Graph<'_>, body: usize, placement: &place::Placement, sized: &[size::Sized]) -> Vec<Group> {
    let (Some(of), Some(routes)) = (graph.bodies.get(body), placement.routes.get(body)) else { return Vec::new() };
    let core = frame_fill(placement.frames.get(body).map_or(0, |frame| frame.depth));
    // What a name must not cover: the nodes of the body, its wires, the pills.
    let mut obstacles = names::Obstacles::default();
    for &node in &of.nodes {
        let (Some(Some(at)), Some(found)) = (placement.nodes.get(node), sized.get(node)) else { continue };
        obstacles.rect(at.x - found.left, at.y - found.above, at.x + found.width + found.right, at.y + found.height);
    }
    for routed in routes {
        for branch in &routed.branches {
            for pair in branch.windows(2) {
                if let (Some(&a), Some(&b)) = (pair.first(), pair.get(1)) {
                    obstacles.segment(a, b);
                }
            }
        }
    }
    // The number of each `collect` and each `yield` on the right, among those of its kind.
    let mut counted = (0usize, 0usize);
    let ordinals: Vec<usize> = of
        .right
        .iter()
        .map(|terminal| match terminal.edge {
            graph::Edge::Collect => {
                counted.0 += 1;
                counted.0 - 1
            }
            graph::Edge::Yield => {
                counted.1 += 1;
                counted.1 - 1
            }
            _ => 0,
        })
        .collect();
    // The pill of each wire, just before its reader.
    let mut pills: HashMap<usize, Vec<Shape>> = HashMap::new();
    for routed in routes {
        for (&wire, branch) in routed.wires.iter().zip(&routed.branches) {
            let (Some(pill), Some(&(x, y))) = (graph.pills.get(&(body, wire)), branch.last()) else { continue };
            let text = Text::new(&pill.text, size::LABEL_LINE, MEDIUM);
            let start = x - pill_reach(pill);
            obstacles.rect(start, y - size::PILL_HEIGHT / 2, start + size::pill_width(&text), y + size::PILL_HEIGHT / 2);
            pills.entry(wire).or_default().extend(size::wire_pill(&text, start, y, pill.conversion));
        }
    }
    routes
        .iter()
        .filter_map(|routed| {
            let first = of.wires.get(*routed.wires.first()?)?;
            // Each branch as its wire is drawn: a value read as an order (`after:`) and as a value shares its trunk.
            let style = size::wire_style(first.family, core);
            // The trunk once: each branch from where it leaves those before it.
            let mut shapes: Vec<Shape> = routed
                .wires
                .iter()
                .zip(route::parts(&routed.branches))
                .filter(|(_, points)| points.len() > 1)
                .map(|(&wire, points)| {
                    let style = of.wires.get(wire).map_or(style, |wire| size::wire_style(wire.family, core));
                    Shape::Wire { points, style }
                })
                .collect();
            shapes.extend(routed.junctions.iter().map(|&(x, y)| Shape::Circle { x, y, radius: 300, paint: Paint::fill(style.token) }));
            for wire in &routed.wires {
                shapes.extend(pills.remove(wire).unwrap_or_default());
            }
            let value = first.name.split('.').next().unwrap_or_default().to_owned();
            shapes.extend(wire_names(&value, &routed.branches, &mut obstacles));
            let from = terminal_form(graph, of, body, &first.from, true, &ordinals);
            let mut to: Vec<String> = routed
                .wires
                .iter()
                .filter_map(|&wire| of.wires.get(wire))
                .filter_map(|wire| terminal_form(graph, of, body, &wire.to, false, &ordinals))
                .collect();
            to.dedup();
            let readers = routed
                .wires
                .iter()
                .zip(&routed.branches)
                .map(|(&wire, branch)| {
                    let form = of.wires.get(wire).and_then(|wire| terminal_form(graph, of, body, &wire.to, false, &ordinals));
                    (form, branch.clone())
                })
                .collect();
            let error = routed.wires.iter().filter_map(|&wire| of.wires.get(wire)).any(|wire| wire.family == graph::Family::ERROR);
            Some(Group {
                node: String::new(),
                title: String::new(),
                shapes,
                arm: None,
                wire: Some(WireOf { body, error, via: routed.via, value, from, to, readers }),
            })
        })
        .collect()
}

/// How far the pill of a wire reaches left of the terminal it leads to.
fn pill_reach(pill: &graph::Pill) -> i64 {
    size::TERMINAL / 2 + GRID + size::pill_width(&Text::new(&pill.text, size::LABEL_LINE, MEDIUM))
}

/// Above a wire, its name (notation 4): on its first horizontal segment, 3 units above it, when the whole name fits there
/// with 4 on each side and covers nothing; and again halfway along a long horizontal of the wire, by the same rule.
fn wire_names(value: &str, branches: &[Vec<(i64, i64)>], obstacles: &mut names::Obstacles) -> Vec<Shape> {
    const ABOVE: i64 = 300;
    const SIDE: i64 = 400;
    let text = Text::new(value, ANY_TEXT, MEDIUM);
    if text.width == 0 {
        return Vec::new();
    }
    let height = size::line_height(MEDIUM);
    let across = |a: (i64, i64), b: (i64, i64)| (a.1 == b.1 && a.0 != b.0).then_some((a.0.min(b.0), a.0.max(b.0), a.1));
    let mut candidates = Vec::new();
    if let Some((x0, x1, y)) = branches.first().and_then(|branch| across(*branch.first()?, *branch.get(1)?)) {
        candidates.push((x0 + SIDE, x1 - SIDE, y, false));
    }
    // The longest horizontal of the net past its first segment, when it is long: three times what the name needs.
    let longest = branches
        .iter()
        .flat_map(|branch| branch.windows(2).skip(1).filter_map(|pair| across(*pair.first()?, *pair.get(1)?)))
        .max_by_key(|&(x0, x1, y)| (x1 - x0, -y, -x0));
    if let Some((x0, x1, y)) = longest
        && x1 - x0 >= 3 * (text.width + 2 * SIDE)
    {
        candidates.push((x0 + SIDE, x1 - SIDE, y, true));
    }
    let mut shapes = Vec::new();
    for (start, end, y, centred) in candidates {
        if end - start < text.width {
            continue;
        }
        let x = if centred { start + (end - start - text.width) / 2 } else { start };
        let (top, bottom) = (y - ABOVE - height, y - ABOVE);
        if !obstacles.free(x, top, x + text.width, bottom) {
            continue;
        }
        obstacles.rect(x, top, x + text.width, bottom);
        // Its baseline 3 above the wire, its line above that, as the extent of a text counts it.
        let cap = labcraft_types::text::scale(u64::from(labcraft_types::text::CAP_HEIGHT), MEDIUM).unwrap_or(0);
        shapes.push(text.at(x, bottom - (cap + 1) / 2, "ink.muted"));
    }
    shapes
}

/// A terminal as the reports write it (`docs/cli.md`, 4.3), when it has a form there.
fn terminal_form(graph: &Graph<'_>, of: &graph::BodyOf, body: usize, end: &graph::End, start: bool, ordinals: &[usize]) -> Option<String> {
    match *end {
        graph::End::Node { node, terminal } => {
            let found = graph.nodes.get(node)?;
            // The selector of a `match` or a `handle` has no form.
            if !start
                && terminal == 0
                && matches!(&found.kind, Kind::Structure(structure) if matches!(structure.kind, StructureKind::Match | StructureKind::Handle))
            {
                return None;
            }
            let terminals = if start { &found.outputs } else { &found.inputs };
            Some(format!("{}.{}", found.key, terminals.get(terminal)?.name))
        }
        graph::End::Order(node) => (!start).then(|| graph.nodes.get(node).map(|found| format!("{}.after", found.key))).flatten(),
        graph::End::Left(index) => (body == 0).then(|| of.left.get(index).map(|terminal| format!("fn.{}", terminal.terminal.name))).flatten(),
        graph::End::Right(index) => {
            let terminal = of.right.get(index)?;
            match terminal.edge {
                graph::Edge::Output => Some(format!("fn.{}", terminal.terminal.name)),
                graph::Edge::Register => Some(format!("next.{}", terminal.terminal.name.trim_start_matches("next "))),
                graph::Edge::Until => Some("until".to_owned()),
                graph::Edge::Collect => Some(format!("collect.{}", ordinals.get(index)?)),
                graph::Edge::Yield => Some(format!("yield.{}", ordinals.get(index)?)),
                _ => None,
            }
        }
    }
}

#[cfg(test)]
mod tests;
