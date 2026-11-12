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

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

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
pub mod style;
mod svg;
pub mod theme;

pub use labcraft_check::PathSegment;
pub use marks::{MAX_LINES, mark, undrawn};
pub use shape::{
    ArmOf, BodyDrawn, CHEVRON_STROKE, COARSE_NODE, Detail, Drawing, Ground, Group, Hint, IconDrawing, IconDrawn, IconItem, IconShape, Mark, Paint,
    Pattern, Placed, Run, Shape, Tab, TabKind, TerminalsOf, Token, WireOf, WireStyle, chevrons,
};
pub use svg::{FONT_FAMILY, SCHEMA, Theme, svg};

use graph::{Base, Builder, Graph, Kind, StructureKind};
use size::{Sizer, Text};

/// What placement depends on, of the view (notation 2.1): the preference of the IDE that shows every node in standard
/// view (notation 3), and the view state that the file does not hold, which a viewer changes (commands.md 6.5,
/// `view.set_state`).
///
/// The default is the view state of every exported image (cli.md 7.2): constructions as their block, every arm
/// unfolded.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ViewState {
    pub standard_everywhere: bool,
    /// A construction whose fields are all constants drawn folded, as a small node (notation 3.1): the default of the
    /// viewer, never of an export.
    pub fold_constructions: bool,
    /// The arms of each `match` and `handle` in tabs, one shown at a time (notation 5.2): the default of the viewer,
    /// never of an export. Which arm shows is not placement: the canvas chooses it from [`Drawing::arms`].
    pub tabs: bool,
    /// The structures drawn unfolded all the same, by the key that follows them ([`Placed::key`]).
    pub unfolded: BTreeSet<String>,
    /// The nodes drawn in standard view besides those that their file puts in it, by their key.
    pub standard: BTreeSet<String>,
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
#[expect(clippy::large_enum_variant, reason = "one per function drawn, and most are drawings")]
pub enum Drawn {
    Drawing(Drawing),
    /// `LC7002`, or a file that declares no function.
    NotDrawn(Vec<Diagnostic>),
}

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
        Ok(mut graph) => {
            for node in &mut graph.nodes {
                node.standard |= view.standard.contains(&node.stable);
            }
            let mut drawn = drawing(&graph, names, view);
            for warning in &mut drawn.warnings {
                warning.file.clone_from(&checked_file.shown);
            }
            Drawn::Drawing(drawn)
        }
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

/// The band of each node (notation 3.2): the last segment of its library, cut to the width of its block; the full path
/// when two libraries of the drawing would read the same at that width; cut in the middle when they still do. A width
/// is that of a compact block or of an oblong, whose label is cut: few widths, each worked out once.
fn bands(graph: &Graph<'_>) -> Vec<Option<ShownText>> {
    let library_of = |kind: &Kind<'_>| match kind {
        Kind::Call { library, glyph: None, .. } | Kind::Construction { library, .. } | Kind::Variant { library, .. } => library.clone(),
        _ => None,
    };
    let libraries: Vec<Option<String>> = graph.nodes.iter().map(|node| library_of(&node.kind)).collect();
    let mut distinct: Vec<&String> = libraries.iter().flatten().collect();
    distinct.sort();
    distinct.dedup();
    let cells: Vec<u64> = graph
        .nodes
        .iter()
        .zip(&libraries)
        .map(|(node, library)| if library.is_some() { crate::size::band_cells(crate::size::band_room(node)) } else { 0 })
        .collect();
    let mut at: BTreeMap<u64, BTreeMap<&String, String>> = BTreeMap::new();
    for (library, &cells) in libraries.iter().zip(&cells) {
        if library.is_some() && !at.contains_key(&cells) {
            at.insert(cells, bands_at(&distinct, cells));
        }
    }
    libraries
        .iter()
        .zip(&cells)
        .map(|(library, cells)| library.as_ref().and_then(|library| at.get(cells)?.get(library)).map(|text| ShownText::plain(text)))
        .collect()
}

/// The band of each library at `cells` (notation 3.2).
fn bands_at<'a>(distinct: &[&'a String], cells: u64) -> BTreeMap<&'a String, String> {
    let short = |library: &str| labcraft_types::text::cut(library.rsplit('.').next().unwrap_or(library), cells).into_owned();
    let mut drawn: BTreeMap<&String, String> = distinct.iter().map(|&library| (library, short(library))).collect();
    for pass in 0..2 {
        let mut readers: BTreeMap<String, Vec<&String>> = BTreeMap::new();
        for (library, text) in &drawn {
            readers.entry(text.clone()).or_default().push(library);
        }
        for same in readers.values().filter(|libraries| libraries.len() > 1) {
            for &library in same {
                let text = if pass == 0 { labcraft_types::text::cut(library, cells).into_owned() } else { cut_middle(library, cells).into_owned() };
                drawn.insert(library, text);
            }
        }
    }
    drawn
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
    let mut drawing = Drawing { function: graph.function.clone(), style: style::current(), ..Drawing::default() };
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
    let titles = titles(graph, view);
    let placement = place::place(graph, &mut sized, &titles, (style::margin(), style::margin()));
    drawing.warnings = overlapping(graph, &sized, &placement);
    // The arms in tabs, outermost first, and the innermost arm in tabs that holds each body.
    let mut inside: Vec<Option<usize>> = vec![None; graph.bodies.len()];
    // The rank of each body among the arms of its structure.
    let mut rank: Vec<usize> = vec![0; graph.bodies.len()];
    for node in &graph.nodes {
        for (arm, &body) in node.bodies.iter().enumerate() {
            if let Some(slot) = rank.get_mut(body) {
                *slot = arm;
            }
        }
    }
    for (index, body) in graph.bodies.iter().enumerate() {
        let Some(owner) = body.owner else { continue };
        let Some(found) = graph.nodes.get(owner) else { continue };
        let around = inside.get(found.body).copied().flatten();
        let held = if titles.tabs.contains(&owner) {
            let arm = rank.get(index).copied().unwrap_or(0);
            drawing.arms.push(ArmOf { structure: found.stable.clone(), arm, within: around });
            Some(drawing.arms.len() - 1)
        } else {
            around
        };
        if let Some(slot) = inside.get_mut(index) {
            *slot = held;
        }
    }
    let within = |body: usize| inside.get(body).copied().flatten();
    // The function's frame, then each node, a structure with its frame before what it holds.
    if let (Some(body), Some(frame)) = (graph.bodies.first(), placement.frames.first()) {
        drawing.loose = frame_shapes(frame, &Text::new(&body.caption, crate::size::any_text(), crate::size::large()), None, false, "frame.stroke");
    }
    // The wires of each body, drawn over its frame and under its nodes.
    let mut wires: Vec<Vec<Group>> = (0..graph.bodies.len())
        .map(|body| {
            let mut groups = wire_groups(graph, body, &placement, &sized);
            groups.iter_mut().for_each(|group| group.within = within(body));
            groups
        })
        .collect();
    // The terminals of the frame of a body and their names, over its wires.
    let edges = |body: usize, placed: Option<usize>, hints: &mut Vec<Hint>| -> Option<Group> {
        let (Some(of), Some(frame)) = (graph.bodies.get(body), placement.frames.get(body)) else { return None };
        hints.extend(frame_hints(graph, body, frame, placed, within(body)));
        let shapes = frame_terminals(of, frame);
        let arm = of
            .owner
            .and_then(|owner| graph.nodes.get(owner))
            .filter(
                |owner| matches!(&owner.kind, Kind::Structure(structure) if matches!(structure.kind, StructureKind::Match | StructureKind::Handle)),
            )
            .map(|_| of.pattern.clone());
        (!shapes.is_empty()).then(|| Group {
            node: String::new(),
            title: String::new(),
            shapes,
            arm: None,
            wire: None,
            placed,
            within: within(body),
            terminals: Some(TerminalsOf { arm }),
        })
    };
    if let Some(first) = wires.first_mut() {
        drawing.groups.append(first);
    }
    drawing.groups.extend(edges(0, None, &mut drawing.hints));
    // The conversions on the wires into nodes, by the input they reach, for its tooltip (notation 4.2): once for all.
    let mut conversions: HashMap<(usize, usize), &str> = HashMap::new();
    for (&(body, wire), pill) in &graph.pills {
        if let (Some(conversion), Some(graph::End::Node { node, terminal })) =
            (&pill.conversion, graph.bodies.get(body).and_then(|of| of.wires.get(wire)).map(|wire| wire.to))
        {
            conversions.insert((node, terminal), conversion);
        }
    }
    for (index, (node, sized)) in graph.nodes.iter().zip(sized).enumerate() {
        let Some(Some(at)) = placement.nodes.get(index) else { continue };
        let mut shapes: Vec<Shape> = sized.shapes.into_iter().map(|shape| moved(shape, at.x, at.y)).collect();
        let mut arms = Vec::new();
        let mut tabs = Vec::new();
        if let Kind::Structure(structure) = &node.kind {
            // The frame, then the wires in the margins of a `match`.
            let margins = std::mem::take(&mut shapes);
            let in_tabs = titles.tabs.contains(&index);
            (shapes, arms, tabs) = structure_shapes(graph, node, structure, &placement, at, (sized.width, &sized.inputs, &sized.outputs), in_tabs);
            shapes.extend(margins);
            shapes.extend(sized_label(node, at));
            // In tabs, each arm draws its own margins.
            for ((_, arm), margins) in arms.iter_mut().zip(sized.arms) {
                arm.extend(margins.into_iter().map(|shape| moved(shape, at.x, at.y)));
            }
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
        let placed = Some(drawing.placed.len() - 1);
        let half = style::terminal() / 2;
        let first_hint = drawing.hints.len();
        for (terminal, &(dx, dy)) in node.inputs.iter().zip(&sized.inputs).chain(node.outputs.iter().zip(&sized.outputs)) {
            drawing.hints.push(hint_at(terminal, (at.x + dx, at.y + dy), half, placed, within(node.body)));
        }
        // The tooltip of an input says the whole conversion that the pill of its wire shows in part (notation 4.2).
        for terminal in 0..node.inputs.len().min(sized.inputs.len()) {
            if let (Some(conversion), Some(hint)) = (conversions.get(&(index, terminal)), drawing.hints.get_mut(first_hint + terminal)) {
                hint.text.push_str("\nconverted: ");
                hint.text.push_str(conversion);
            }
        }
        drawing.groups.push(Group {
            node: node.key.clone(),
            title: title(&node.kind),
            shapes,
            arm: None,
            wire: None,
            placed,
            within: within(node.body),
            terminals: None,
        });
        for ((arm, shapes), &body) in arms.into_iter().zip(&node.bodies) {
            drawing.groups.push(Group {
                node: String::new(),
                title: String::new(),
                shapes,
                arm: Some(arm),
                wire: None,
                placed,
                within: within(body),
                terminals: None,
            });
        }
        for found in tabs {
            let arm_of = |rank: usize| node.bodies.get(rank).and_then(|&body| within(body));
            let Some(arm) = arm_of(found.arm) else { continue };
            let when = match found.when {
                Some(rank) => match arm_of(rank) {
                    Some(when) => Some(when),
                    None => continue,
                },
                None => None,
            };
            let (x, y, width, height) = found.at;
            let (shown, other, kind, caption) = (found.shown, found.other, found.kind, found.caption);
            drawing.tabs.push(Tab { arm, structure: drawing.placed.len() - 1, x, y, width, height, shown, other, when, kind, caption });
        }
        for &body in &node.bodies {
            if let Some(list) = wires.get_mut(body) {
                drawing.groups.append(list);
            }
            drawing.groups.extend(edges(body, placed, &mut drawing.hints));
        }
    }
    // The extent: everything, and the margin around it (cli.md 7.2). The frame of the function counts, though it is not
    // drawn: the drawing keeps its place on the grid.
    let (mut right, mut bottom) = (0i64, 0i64);
    let (mut left, mut top) = (i64::MAX, i64::MAX);
    let function_frame = placement.frames.first().map(|frame| Shape::Rect {
        x: frame.x,
        y: frame.y,
        width: frame.width,
        height: frame.height,
        radius: 0,
        paint: Paint::stroke("frame.stroke", 0),
    });
    for shape in drawing.loose.iter().chain(function_frame.iter()).chain(drawing.groups.iter().flat_map(|group| group.shapes.iter())) {
        let (x0, y0, x1, y1) = extent(shape);
        (left, top, right, bottom) = (left.min(x0), top.min(y0), right.max(x1), bottom.max(y1));
    }
    if left == i64::MAX {
        (left, top) = (style::margin(), style::margin());
    }
    // A node that the file places left of the frame or above it moves the whole drawing.
    let (dx, dy) = ((style::margin() - left).max(0), (style::margin() - top).max(0));
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
        for hint in &mut drawing.hints {
            (hint.x, hint.y) = (hint.x + dx, hint.y + dy);
        }
        for tab in &mut drawing.tabs {
            (tab.x, tab.y) = (tab.x + dx, tab.y + dy);
            tab.shown = std::mem::take(&mut tab.shown).into_iter().map(|shape| moved(shape, dx, dy)).collect();
            tab.other = std::mem::take(&mut tab.other).into_iter().map(|shape| moved(shape, dx, dy)).collect();
        }
    }
    drawing.bodies = placement
        .frames
        .iter()
        .map(|frame| BodyDrawn {
            axis: frame.axis.map(|axis| axis + dy),
            from_file: frame.from_file,
            frame: (frame.x + dx, frame.y + dy, frame.width, frame.height),
        })
        .collect();
    drawing.width = right + dx + style::margin();
    drawing.height = bottom + dy + style::margin();
    drawing
}

/// The label of a structure, above its frame (notation 3.1).
fn sized_label(node: &graph::Node<'_>, at: &place::NodeAt) -> Option<Shape> {
    let label = node.label.as_ref()?;
    let text = Text::new(&format!("{label}:"), crate::size::any_text(), crate::size::medium());
    Some(text.at(at.x, at.y - style::label_above() - size::line_height(crate::size::medium()) / 2, "ink.muted"))
}

/// The widths that titles need: the name of the function, a header and its badge, the patterns of an arm, the tabs of a
/// structure in tabs; and which structures are in tabs.
fn titles(graph: &Graph<'_>, view: &ViewState) -> place::Titles {
    let mut titles = place::Titles { bodies: vec![0; graph.bodies.len()], structures: HashMap::new(), tabs: HashSet::new() };
    if view.tabs {
        for (index, node) in graph.nodes.iter().enumerate() {
            if let Kind::Structure(structure) = &node.kind
                && matches!(structure.kind, StructureKind::Match | StructureKind::Handle)
                && !view.unfolded.contains(&node.stable)
            {
                titles.tabs.insert(index);
            }
        }
    }
    let header_width = |structure: &graph::Structure| {
        let header = Text::new(&structure.header, crate::size::any_text(), crate::size::large()).width;
        let badge = structure
            .badge
            .map_or(0, |badge| style::grid() + Text::new(badge, crate::size::any_text(), crate::size::medium()).width + 2 * style::padding());
        2 * style::label_start() + header + badge
    };
    for (index, body) in graph.bodies.iter().enumerate() {
        let width = match body.owner.and_then(|owner| graph.nodes.get(owner)).map(|owner| &owner.kind) {
            // In tabs, an arm has no title: its tab names it.
            _ if body.owner.is_some_and(|owner| titles.tabs.contains(&owner)) => 0,
            None => 2 * style::label_start() + Text::new(&body.caption, crate::size::any_text(), crate::size::large()).width,
            Some(Kind::Structure(structure)) if matches!(structure.kind, StructureKind::Loop | StructureKind::For) => header_width(structure),
            Some(_) => 2 * style::label_start() + Text::new(&body.caption, crate::size::any_text(), crate::size::medium()).width,
        };
        if let Some(slot) = titles.bodies.get_mut(index) {
            *slot = width;
        }
    }
    for (index, node) in graph.nodes.iter().enumerate() {
        if let Kind::Structure(structure) = &node.kind
            && matches!(structure.kind, StructureKind::Match | StructureKind::Handle)
        {
            let mut width = header_width(structure);
            // The tabs widen the frame no more than a selector needs: when they do not hold, it stands for them (notation
            // 5.2).
            if titles.tabs.contains(&index) {
                let row: i64 =
                    node.bodies.iter().filter_map(|&body| graph.bodies.get(body)).map(|body| tab_width(&body.caption) + style::tab_gap()).sum();
                width = width.max(2 * style::label_start() + (row - style::tab_gap()).min(selector_least()));
            }
            titles.structures.insert(index, width);
        }
    }
    titles
}

/// The least width of a selector, from the values of the notation: its two steps, as wide as a tab is high, a gap on
/// each side of its title, and a title that holds 13 cells, 6 of the arm and 7 of ` ▾ 1/5`, and its padding: with the
/// defaults 2 × 20 + 2 × 4 + 78.27 (13 × 1233 × 10 / 2048) + 2 × 4 = 134.27.
fn selector_least() -> i64 {
    let cells = labcraft_types::text::width(13, crate::size::medium()).unwrap_or(0);
    2 * style::tab_height() + 2 * style::tab_gap() + cells + 2 * style::padding()
}

/// The tabs of a `match` or a `handle` in tabs (notation 5.2), in the row under its header, from `x` at `y`, in the width
/// `room`: a tab per arm when they hold; else a selector, `‹ State.Init ▾ 1/5 ›`, its steps to the arm before and after,
/// its title a list of the arms, each part shown with the arm it stands for.
fn tab_row(captions: &[&str], (x, y): (i64, i64), room: i64, stroke: Token) -> Vec<TabShapes> {
    let height = style::tab_height();
    let middle = y + height / 2;
    let rect = |x: i64, width: i64, paint| Shape::Rect { x, y, width, height, radius: 300, paint };
    let row: i64 = captions.iter().map(|caption| tab_width(caption) + style::tab_gap()).sum::<i64>() - style::tab_gap();
    let mut tabs = Vec::new();
    if row <= room {
        let mut tab_x = x;
        for (rank, caption) in captions.iter().enumerate() {
            let text = Text::new(caption, crate::size::any_text(), crate::size::medium());
            let width = tab_width(caption);
            // The arm shown filled *(proposed)*.
            let shown = vec![
                rect(tab_x, width, Paint::both("tab.fill", stroke, style::outline())),
                text.shown_from(tab_x + style::padding(), middle, "node.title", shape::Detail::Title),
            ];
            let other = vec![
                rect(tab_x, width, Paint::stroke("frame.divider", style::outline())),
                text.shown_from(tab_x + style::padding(), middle, "ink.muted", shape::Detail::Title),
            ];
            let caption = labcraft_types::cut_text(caption).into_owned();
            tabs.push(TabShapes { at: (tab_x, y, width, height), shown, other, arm: rank, when: None, kind: TabKind::Tab, caption });
            tab_x += width + style::tab_gap();
        }
        return tabs;
    }
    let count = captions.len();
    let step = height;
    let name_x = x + step + style::tab_gap();
    let name_width = (room - 2 * (step + style::tab_gap())).max(step);
    let next_x = name_x + name_width + style::tab_gap();
    let glyph = |text: &str, at: i64| Text::new(text, 1, crate::size::medium()).centred(at + step / 2, middle, "node.title");
    for (rank, caption) in captions.iter().enumerate() {
        // `▾ 2/9` after the title, which is cut to what is left.
        let count_text = format!(" \u{25BE} {}/{count}", rank + 1);
        let tail = Text::new(&count_text, crate::size::any_text(), crate::size::medium());
        let cell = labcraft_types::text::width(1, crate::size::medium()).unwrap_or(1).max(1);
        let cells = u64::try_from((name_width - 2 * style::padding() - tail.width).max(0) / cell).unwrap_or(0);
        let title = Text::new(caption, cells, crate::size::medium());
        let shown = vec![
            rect(name_x, name_width, Paint::both("tab.fill", stroke, style::outline())),
            title.shown_from(name_x + style::padding(), middle, "node.title", shape::Detail::Title),
            tail.shown_from(name_x + style::padding() + title.width, middle, "ink.muted", shape::Detail::Title),
        ];
        let caption = labcraft_types::cut_text(caption).into_owned();
        tabs.push(TabShapes {
            at: (name_x, y, name_width, height),
            shown,
            other: Vec::new(),
            arm: rank,
            when: Some(rank),
            kind: TabKind::Name,
            caption: caption.clone(),
        });
        // Its steps: to the arm before and after, round.
        for (target, at, text) in [((rank + count - 1) % count, x, "\u{2039}"), ((rank + 1) % count, next_x, "\u{203A}")] {
            let shapes = vec![rect(at, step, Paint::stroke("frame.divider", style::outline())), glyph(text, at)];
            let caption = captions.get(target).map(|caption| labcraft_types::cut_text(caption).into_owned()).unwrap_or_default();
            tabs.push(TabShapes {
                at: (at, y, step, height),
                shown: shapes.clone(),
                other: shapes,
                arm: target,
                when: Some(rank),
                kind: TabKind::Step,
                caption,
            });
        }
    }
    tabs
}

/// The width of the tab of an arm.
fn tab_width(caption: &str) -> i64 {
    Text::new(caption, crate::size::any_text(), crate::size::medium()).width + 2 * style::padding()
}

/// An arm: its title, and its shapes.
type ArmShapes = (String, Vec<Shape>);

/// A tab or a part of a selector: where it is, its shapes shown and not, the rank of the arm it shows, the rank of the
/// arm while which it shows, its kind and the title of its arm.
struct TabShapes {
    at: (i64, i64, i64, i64),
    shown: Vec<Shape>,
    other: Vec<Shape>,
    arm: usize,
    when: Option<usize>,
    kind: TabKind,
    caption: String,
}

/// A frame: its box, its title in its header, a badge.
fn frame_shapes(frame: &place::FrameAt, title: &Text, badge: Option<&str>, double: bool, stroke: Token) -> Vec<Shape> {
    let mut shapes = Vec::new();
    // The frame of the function has no fill: it is drawn on the canvas, and the first structure takes the first level
    // (notation 5, 8).
    let paint =
        if frame.depth > 0 { Paint::both(frame_fill(frame.depth), stroke, style::outline()) } else { Paint::stroke(stroke, style::outline()) };
    shapes.push(Shape::Rect { x: frame.x, y: frame.y, width: frame.width, height: frame.height, radius: style::radius(), paint });
    if double {
        shapes.push(Shape::Rect {
            x: frame.x + 300,
            y: frame.y + 300,
            width: frame.width - 600,
            height: frame.height - 600,
            radius: style::radius(),
            paint: Paint::stroke(stroke, style::outline()),
        });
    }
    let middle = frame.y + frame.header / 2;
    shapes.push(title.shown_from(frame.x + style::label_start(), middle, "node.title", shape::Detail::Title));
    if let Some(badge) = badge {
        let text = Text::new(badge, crate::size::any_text(), crate::size::medium());
        let x = frame.x + style::label_start() + title.width + style::grid();
        shapes.push(Shape::Rect {
            x,
            y: middle - style::grid(),
            width: text.width + 2 * style::padding(),
            height: 2 * style::grid(),
            radius: 300,
            paint: Paint::fill("badge.fill"),
        });
        shapes.push(text.at(x + style::padding(), middle, "badge.text"));
    }
    shapes
}

/// The terminals of the frame of a body on its edges, their names inside (notation 2, 5). They are drawn over the wires
/// of the body, which leave a terminal from its centre: the terminal covers the end of its wire, and the name with its
/// unit lies on a ground of the fill of the frame, from the terminal to where the wire was routed from, so that the
/// wire shows when the name does not (notation 7).
fn frame_terminals(body: &graph::BodyOf, frame: &place::FrameAt) -> Vec<Shape> {
    let mut shapes = Vec::new();
    let height = size::line_height(crate::size::medium());
    for (side, terminals, heights) in [(false, &body.left, &frame.left), (true, &body.right, &frame.right)] {
        let x = if side { frame.x + frame.width } else { frame.x };
        for (terminal, &at_height) in terminals.iter().zip(heights) {
            let y = frame.y + at_height;
            shapes.extend(edge_terminal(terminal, x, y));
            let label = place::TerminalLabel::of(terminal);
            let width = label.width();
            let start = if side { x - style::label_start() - width } else { x + style::label_start() };
            let mut at = start + label.name.width;
            if let Some(unit) = &label.unit {
                at += style::padding() + unit.width;
            }
            // From the terminal to the pill, or to where the route starts; on the right, from where it ends. It stops a
            // outline short of the terminal, so that it never covers its outline (a register's box).
            let reach = edge_reach(terminal, Some(side)) + style::outline();
            let (from, to) = match (side, &label.pill) {
                (false, Some(_)) => (x + reach, at + style::grid()),
                (false, None) => (x + reach, start + width + style::padding()),
                (true, Some(_)) => (start - style::padding(), at + style::grid()),
                (true, None) => (start - style::padding(), x - reach),
            };
            let ground = Ground { token: frame_fill(frame.depth), x: from, y: y - height / 2, width: (to - from).max(0), height };
            let mut name = label.name.shown_from(start, y, "node.title", shape::Detail::Name);
            if let Shape::Text { ground: slot, .. } = &mut name {
                *slot = Some(ground);
            }
            shapes.push(name);
            if let Some(unit) = &label.unit {
                shapes.push(unit.shown_from(start + label.name.width + style::padding(), y, "ink.muted", shape::Detail::Name));
            }
            if let Some(pill) = &label.pill {
                shapes.extend(size::pill(pill, at + style::grid(), y));
            }
        }
    }
    shapes
}

/// How far a terminal of a frame reaches from its edge (`edge_terminal`): inside the frame for a terminal on its left
/// (`right` false) or on its right, or either way with `None`. Measured on its shapes, so that the brackets of an array
/// (notation 4.1) are inside it, and the box of a range, which grows outside, is not.
fn edge_reach(terminal: &graph::FrameTerminal, right: Option<bool>) -> i64 {
    let reach = |(x0, _, x1, _): (i64, i64, i64, i64)| match right {
        Some(false) => x1,
        Some(true) => -x0,
        None => x1.max(-x0),
    };
    edge_terminal(terminal, 0, 0).iter().map(extent).map(reach).max().unwrap_or(0)
}

/// A terminal of a frame: a register is a box across the edge, a notch a bar, the others the shape of their family.
fn edge_terminal(terminal: &graph::FrameTerminal, x: i64, y: i64) -> Vec<Shape> {
    let family = terminal.terminal.family;
    match terminal.edge {
        graph::Edge::Register => {
            let token = size::wire_token(family);
            let mark = Text::new("\u{21BB}", 1, crate::size::small());
            vec![
                Shape::Rect { x: x - 700, y: y - 700, width: 1400, height: 1400, radius: 0, paint: Paint::both("node.fill", token, 150) },
                mark.centred(x, y, "node.title"),
            ]
        }
        graph::Edge::Notch => {
            vec![Shape::Rect { x: x - 200, y: y - 700, width: 400, height: 1400, radius: 0, paint: Paint::fill(size::wire_token(family)) }]
        }
        // `x in values`: the array enters by a bracket outside the edge, an element leaves inside, in the shape of its
        // family (5.1, 4.1).
        graph::Edge::In => {
            let token = size::wire_token(family);
            vec![Shape::Poly {
                points: vec![(x - 500, y - 600), (x - 250, y - 600), (x - 250, y + 600), (x - 500, y + 600)],
                closed: false,
                paint: Paint::stroke(token, 150),
            }]
            .into_iter()
            .chain(size::terminal(family, x + 300, y, 100, false))
            .collect()
        }
        // `collect`: an element enters, the array leaves: the shape of its family in brackets, across the edge (5.1).
        graph::Edge::Collect => {
            let token = size::wire_token(family);
            let bracket = |side: i64| Shape::Poly {
                points: vec![(x + side * 250, y - 600), (x + side * 500, y - 600), (x + side * 500, y + 600), (x + side * 250, y + 600)],
                closed: false,
                paint: Paint::stroke(token, 150),
            };
            vec![bracket(-1), bracket(1)].into_iter().chain(size::terminal(family, x, y, 100, false)).collect()
        }
        // `i in 0..n`: a box across the edge with the start of the range, which the end enters (5.1); rounded and thinner
        // than the box of a register, which it must not look like. It reaches inside as far as a register, and grows
        // outside with its text, so that the name inside keeps its place.
        graph::Edge::Range => {
            let text = Text::new(terminal.start.as_deref().unwrap_or(".."), crate::size::any_text(), crate::size::small());
            let width = (text.width + 2 * 300).max(1400);
            vec![
                Shape::Rect {
                    x: x + 700 - width,
                    y: y - 700,
                    width,
                    height: 1400,
                    radius: 300,
                    paint: Paint::both("node.fill", size::wire_token(family), style::outline()),
                },
                text.centred(x + 700 - width / 2, y, "node.title"),
            ]
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
    in_tabs: bool,
) -> (Vec<Shape>, Vec<ArmShapes>, Vec<TabShapes>) {
    let title = Text::new(&structure.header, crate::size::any_text(), crate::size::large());
    let stroke = if structure.kind == StructureKind::Handle { "wire.error" } else { "frame.stroke" };
    match structure.kind {
        StructureKind::Loop | StructureKind::For => {
            let Some(frame) = node.bodies.first().and_then(|index| placement.frames.get(*index)) else {
                return (Vec::new(), Vec::new(), Vec::new());
            };
            let mut shapes = frame_shapes(frame, &title, structure.badge, structure.parallel, stroke);
            // The inputs follow the clauses, as the left of the frame does: the start of a range is in its box, not on a pill.
            let left = node.bodies.first().and_then(|index| graph.bodies.get(*index)).map_or(&[][..], |body| &body.left[..]);
            for (index, (input, &(_, y))) in node.inputs.iter().zip(inputs).enumerate() {
                if left.get(index).is_some_and(|terminal| terminal.edge == graph::Edge::Range) {
                    continue;
                }
                if let Some(constant) = &input.constant {
                    let text = Text::new(constant, crate::size::compact_constant(), crate::size::medium());
                    shapes.extend(size::pill(&text, at.x - place::constant_reach(constant), at.y + y));
                }
            }
            (shapes, Vec::new(), Vec::new())
        }
        StructureKind::Match | StructureKind::Handle => {
            let depth = node.bodies.first().and_then(|index| placement.frames.get(*index)).map_or(1, |frame| frame.depth.saturating_sub(1));
            // Below its lowest arm, which is the last unfolded.
            let height = node
                .bodies
                .iter()
                .filter_map(|index| placement.frames.get(*index))
                .map(|frame| frame.y + frame.height - at.y + style::frame_pad())
                .max()
                .unwrap_or(style::frame_header() + style::frame_pad());
            let outer = place::FrameAt { x: at.x, y: at.y, width, height, header: style::frame_header(), depth, ..place::FrameAt::default() };
            let mut shapes = frame_shapes(&outer, &title, structure.badge, false, stroke);
            // The selector, then the notches of the outer values that the arms read.
            for (index, (input, &(x, y))) in node.inputs.iter().zip(inputs).enumerate() {
                let edge = if index == 0 { graph::Edge::Input } else { graph::Edge::Notch };
                shapes.extend(edge_terminal(&graph::FrameTerminal { terminal: input.clone(), edge, start: None }, at.x + x, at.y + y));
            }
            for (output, &(x, y)) in node.outputs.iter().zip(outputs) {
                shapes.extend(size::terminal(output.family, at.x + x, at.y + y, 100, output.ignored));
            }
            let mut arms = Vec::new();
            let tabs = if in_tabs {
                let captions: Vec<&str> = node.bodies.iter().filter_map(|&index| graph.bodies.get(index)).map(|body| body.caption.as_str()).collect();
                tab_row(&captions, (at.x + style::label_start(), at.y + style::frame_header()), width - 2 * style::label_start(), stroke)
            } else {
                Vec::new()
            };
            for &index in &node.bodies {
                let (Some(body), Some(frame)) = (graph.bodies.get(index), placement.frames.get(index)) else { continue };
                let mut shapes = Vec::new();
                let caption = Text::new(&body.caption, crate::size::any_text(), crate::size::medium());
                let fill = frame_fill(frame.depth);
                shapes.push(Shape::Rect {
                    x: frame.x,
                    y: frame.y,
                    width: frame.width,
                    height: frame.height,
                    radius: style::radius(),
                    paint: Paint::both(fill, stroke, style::outline()),
                });
                if !in_tabs {
                    shapes.push(caption.shown_from(frame.x + style::label_start(), frame.y + frame.header / 2, "ink.muted", shape::Detail::Title));
                }
                arms.push((body.pattern.clone(), shapes));
            }
            (shapes, arms, tabs)
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
        Shape::Text { x, y, size, width, runs, detail, ground } => Shape::Text {
            x: x + dx,
            y: y + dy,
            size,
            width,
            runs,
            detail,
            ground: ground.map(|ground| Ground { x: ground.x + dx, y: ground.y + dy, ..ground }),
        },
        Shape::Icon { x, y, side, icon } => Shape::Icon { x: x + dx, y: y + dy, side, icon },
        Shape::Wire { points, style } => Shape::Wire { points: points.into_iter().map(|(x, y)| (x + dx, y + dy)).collect(), style },
    }
}

/// The exact type of what a wire carries: that of the terminal it leaves.
fn source_type(graph: &Graph<'_>, of: &graph::BodyOf, from: &graph::End) -> String {
    match *from {
        graph::End::Node { node, terminal } => {
            graph.nodes.get(node).and_then(|node| node.outputs.get(terminal)).map(|terminal| terminal.ty_text.clone())
        }
        graph::End::Left(index) => of.left.get(index).map(|terminal| terminal.terminal.ty_text.clone()),
        _ => None,
    }
    .unwrap_or_default()
}

/// What the tooltip of a terminal says (notation 3.2, 4.1): `timeout: duration`, and `default: 2 s` on a second line for
/// an input that no argument wires; each part cut at 200 code points (cli.md 7.2).
fn hint_text(terminal: &graph::Terminal) -> String {
    let mut text = labcraft_types::cut_text(&terminal.name).into_owned();
    if !terminal.ty_text.is_empty() {
        text.push_str(": ");
        text.push_str(&terminal.ty_text);
    }
    if let Some(default) = &terminal.default {
        text.push_str("\ndefault: ");
        text.push_str(&labcraft_types::cut_text(default));
    }
    text
}

/// A tooltip around a terminal at (`x`, `y`): at least 8 across, so that a small terminal is easy to reach.
fn hint_at(terminal: &graph::Terminal, (x, y): (i64, i64), half: i64, placed: Option<usize>, within: Option<usize>) -> Hint {
    let half = half.max(400);
    Hint { x: x - half, y: y - half, width: 2 * half, height: 2 * half, text: hint_text(terminal), placed, within }
}

/// The tooltips of the terminals of the frame of a body.
/// The tooltips of the terminals of the frame of a body: a terminal on the right that a converted value reaches (an
/// output, a `next`) says the whole conversion, as an input of a node does (notation 4.2). Their area reaches inside the
/// frame as far as the terminal does, so that the box of a range, which grows outside, never covers the body.
fn frame_hints(graph: &Graph<'_>, body: usize, frame: &place::FrameAt, placed: Option<usize>, within: Option<usize>) -> Vec<Hint> {
    let mut hints = Vec::new();
    let Some(of) = graph.bodies.get(body) else { return hints };
    let mut converted: HashMap<usize, &str> = HashMap::new();
    for (index, wire) in of.wires.iter().enumerate() {
        if let (graph::End::Right(terminal), Some(conversion)) =
            (wire.to, graph.pills.get(&(body, index)).and_then(|pill| pill.conversion.as_deref()))
        {
            converted.insert(terminal, conversion);
        }
    }
    for (side, terminals, heights) in [(false, &of.left, &frame.left), (true, &of.right, &frame.right)] {
        let x = if side { frame.x + frame.width } else { frame.x };
        for (index, (terminal, &height)) in terminals.iter().zip(heights).enumerate() {
            let mut hint = hint_at(&terminal.terminal, (x, frame.y + height), edge_reach(terminal, Some(side)).max(700), placed, within);
            if side && let Some(conversion) = converted.get(&index) {
                hint.text.push_str("\nconverted: ");
                hint.text.push_str(conversion);
            }
            hints.push(hint);
        }
    }
    hints
}

/// The branches of a wire as drawn: one that leaves a terminal of the frame, or reaches one, goes on to its centre,
/// under its name (`frame_terminals`); the route stops at the name, so that the name never moves a channel.
fn to_frame_terminals(of: &graph::BodyOf, frame: Option<&place::FrameAt>, routed: &route::Routed) -> Vec<Vec<(i64, i64)>> {
    let Some(frame) = frame else { return routed.branches.clone() };
    let from_left = routed.wires.first().and_then(|&wire| of.wires.get(wire)).is_some_and(|wire| matches!(wire.from, graph::End::Left(_)));
    routed
        .wires
        .iter()
        .zip(&routed.branches)
        .map(|(&wire, branch)| {
            // The end of the route moves to the terminal when its segment is level, which it is when it leaves or reaches a
            // side; otherwise a segment is added. Nothing else is simplified: the waypoints of a `via` stay (spec 11).
            let mut points = branch.clone();
            let level = |a: Option<&(i64, i64)>, b: Option<&(i64, i64)>| matches!((a, b), (Some(a), Some(b)) if a.1 == b.1);
            if from_left && let Some(&(_, y)) = points.first() {
                if level(points.first(), points.get(1))
                    && let Some(first) = points.first_mut()
                {
                    *first = (frame.x, y);
                } else {
                    points.insert(0, (frame.x, y));
                }
            }
            if of.wires.get(wire).is_some_and(|wire| matches!(wire.to, graph::End::Right(_)))
                && let Some(&(_, y)) = points.last()
            {
                let last = points.len().saturating_sub(1);
                if last > 0
                    && level(points.get(last - 1), points.get(last))
                    && let Some(end) = points.last_mut()
                {
                    *end = (frame.x + frame.width, y);
                } else {
                    points.push((frame.x + frame.width, y));
                }
            }
            points
        })
        .collect()
}

/// The fill of a frame at a depth of nesting (notation 8): the function has none, it is on the canvas.
fn frame_fill(depth: u32) -> Token {
    match depth.min(4) {
        0 => "canvas.fill",
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
        // The terminals of the frames of a structure reach past its edges.
        for &inner in graph.nodes.get(node).map_or(&[][..], |found| &found.bodies[..]) {
            if let (Some(inner_of), Some(frame)) = (graph.bodies.get(inner), placement.frames.get(inner)) {
                frame_obstacles(inner_of, frame, false, &mut obstacles);
            }
        }
    }
    // The terminals of its own frame and their names inside.
    if let Some(frame) = placement.frames.get(body) {
        frame_obstacles(of, frame, true, &mut obstacles);
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
    // The pill of each wire, just before its reader, where the reader keeps it: past the pill of a neighbour when it
    // steps outward (notation 3.2).
    let mut pills: HashMap<usize, Vec<Shape>> = HashMap::new();
    for routed in routes {
        for (&wire, branch) in routed.wires.iter().zip(&routed.branches) {
            let (Some(pill), Some(&(x, y))) = (graph.pills.get(&(body, wire)), branch.last()) else { continue };
            let text = Text::new(&pill.text, crate::size::label_line(), crate::size::medium());
            let kept = match of.wires.get(wire).map(|wire| wire.to) {
                Some(graph::End::Node { node, terminal }) => sized.get(node).and_then(|found| found.reaches.get(terminal)).copied().flatten(),
                _ => None,
            };
            let start = x - kept.unwrap_or_else(|| pill_reach(pill));
            obstacles.rect(start, y - style::pill_height() / 2, start + size::pill_width(&text), y + style::pill_height() / 2);
            pills.entry(wire).or_default().extend(size::wire_pill(&text, start, y, pill.conversion.is_some()));
        }
    }
    routes
        .iter()
        .filter_map(|routed| {
            let first = of.wires.get(*routed.wires.first()?)?;
            // Each branch as its wire is drawn: a value read as an order (`after:`) and as a value shares its trunk.
            let style = size::wire_style(first.family, core);
            let branches = to_frame_terminals(of, placement.frames.get(body), routed);
            // The trunk once: each branch from where it leaves those before it.
            let mut shapes: Vec<Shape> = routed
                .wires
                .iter()
                .zip(route::parts(&branches))
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
            // A wire that leaves a terminal of the frame already named the same has no name at its start, nor one that
            // leaves a constant node, which shows its name (notation 4, B1). Nor on the last segment of a branch that ends
            // at a terminal of the frame named the same, `next s` for `s`.
            let named_at_start = match first.from {
                graph::End::Left(index) => of.left.get(index).is_some_and(|terminal| terminal.terminal.name == value),
                graph::End::Node { node, .. } => graph.nodes.get(node).is_some_and(|node| matches!(node.kind, graph::Kind::Constant { .. })),
                _ => false,
            };
            let named_at_end: Vec<bool> = routed
                .wires
                .iter()
                .map(|&wire| match of.wires.get(wire).map(|wire| wire.to) {
                    Some(graph::End::Right(index)) => of.right.get(index).is_some_and(|terminal| {
                        let name = &terminal.terminal.name;
                        *name == value || (terminal.edge == graph::Edge::Register && name.strip_prefix("next ") == Some(value.as_str()))
                    }),
                    _ => false,
                })
                .collect();
            shapes.extend(wire_names(&value, &routed.branches, (named_at_start, &named_at_end[..]), &mut obstacles));
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
                .zip(&branches)
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
                wire: Some(WireOf {
                    body,
                    error,
                    via: routed.via,
                    value,
                    from,
                    to,
                    readers,
                    ty: source_type(graph, of, &first.from),
                    conversions: routed
                        .wires
                        .iter()
                        .filter_map(|&wire| graph.pills.get(&(body, wire)).and_then(|pill| pill.conversion.clone()))
                        .collect(),
                }),
                placed: None,
                within: None,
                terminals: None,
            })
        })
        .collect()
}

/// `LC7005` for each node placed by the file that overlaps one before it, from the left then from the top (notation
/// 2.1), at its entry of the `layout` section, naming that one: the drawing keeps both where the file puts them.
fn overlapping(graph: &Graph<'_>, sized: &[size::Sized], placement: &place::Placement) -> Vec<Diagnostic> {
    place::overlaps(graph, sized, placement)
        .into_iter()
        .filter_map(|(first, second)| {
            let (Some(a), Some(b)) = (graph.nodes.get(first), graph.nodes.get(second)) else { return None };
            let message =
                format!("node `{}` overlaps node `{}`: the `layout` section places both, and the drawing keeps where it puts them", b.key, a.key);
            let mut diagnostic = Diagnostic::new(Code::Lc7005, message, String::new());
            if let Some(span) = b.placed_by.or(a.placed_by) {
                diagnostic = diagnostic.with_span(span);
            }
            diagnostic.function = Some(graph.function.clone());
            Some(diagnostic)
        })
        .collect()
}

/// How far the pill of a wire reaches left of the terminal it leads to.
fn pill_reach(pill: &graph::Pill) -> i64 {
    style::terminal() / 2 + style::grid() + size::pill_width(&Text::new(&pill.text, crate::size::label_line(), crate::size::medium()))
}

/// The terminals of a frame as obstacles to the names of wires: each terminal, which reaches past the edge, and, when
/// `named`, its name inside with its unit and pill (notation 4, 5).
fn frame_obstacles(of: &graph::BodyOf, frame: &place::FrameAt, named: bool, obstacles: &mut names::Obstacles) {
    // A register box is 14 across, centred on the edge (`edge_terminal`); the other terminals are smaller.
    let reach = style::terminal().max(1400) / 2;
    let height = size::line_height(crate::size::medium());
    for (side, terminals, heights) in [(false, &of.left, &frame.left), (true, &of.right, &frame.right)] {
        let x = if side { frame.x + frame.width } else { frame.x };
        for (terminal, &at) in terminals.iter().zip(heights) {
            let y = frame.y + at;
            obstacles.rect(x - reach, y - reach, x + reach, y + reach);
            if named {
                let width = place::TerminalLabel::of(terminal).width();
                let start = if side { x - style::label_start() - width } else { x + style::label_start() };
                obstacles.rect(start, y - height / 2, start + width, y + height / 2);
            }
        }
    }
}

/// Above a wire, its name (notation 4): on its first horizontal segment, 3 units above it, when the whole name fits there
/// with 4 on each side and covers nothing, else on the next horizontal of that branch where it does; none at the start
/// when the wire leaves a terminal that shows the same name (`named_at_start`), and none on the last segment of a branch
/// that ends at one (`named_at_end`, by branch). And again halfway along a long horizontal of the wire, past the segment
/// that holds the first name, by the same rule.
fn wire_names(
    value: &str,
    branches: &[Vec<(i64, i64)>],
    (named_at_start, named_at_end): (bool, &[bool]),
    obstacles: &mut names::Obstacles,
) -> Vec<Shape> {
    let (above, side) = (style::name_above(), style::name_side());
    let text = Text::new(value, crate::size::any_text(), crate::size::medium());
    if text.width == 0 {
        return Vec::new();
    }
    let height = size::line_height(crate::size::medium());
    let across = |a: (i64, i64), b: (i64, i64)| (a.1 == b.1 && a.0 != b.0).then_some((a.0.min(b.0), a.0.max(b.0), a.1));
    // The segments of a branch that may hold its name: all but the last when its reader shows the same name.
    let open = |number: usize, branch: &[(i64, i64)]| {
        let shown = named_at_end.get(number).copied().unwrap_or(false);
        branch.len().saturating_sub(if shown { 2 } else { 1 })
    };
    let cap = labcraft_types::text::scale(u64::from(labcraft_types::text::CAP_HEIGHT), crate::size::medium()).unwrap_or(0);
    let mut shapes = Vec::new();
    // Writes the name on `[start, end]` at `y` when it fits there and covers nothing.
    let mut put = |start: i64, end: i64, y: i64, centred: bool, shapes: &mut Vec<Shape>| -> bool {
        if end - start < text.width {
            return false;
        }
        let x = if centred { start + (end - start - text.width) / 2 } else { start };
        let (top, bottom) = (y - above - height, y - above);
        if !obstacles.free(x, top, x + text.width, bottom) {
            return false;
        }
        obstacles.rect(x, top, x + text.width, bottom);
        // Its baseline 3 above the wire, its line above that, as the extent of a text counts it.
        shapes.push(text.at(x, bottom - (cap + 1) / 2, "ink.muted"));
        true
    };
    // The first name: the first horizontal of the first branch where it fits, the segment that holds it kept.
    let mut first = None;
    if !named_at_start && let Some(branch) = branches.first() {
        for (index, pair) in branch.windows(2).take(open(0, branch)).enumerate() {
            let (Some(&a), Some(&b)) = (pair.first(), pair.get(1)) else { continue };
            let Some((x0, x1, y)) = across(a, b) else { continue };
            if put(x0 + side, x1 - side, y, false, &mut shapes) {
                first = Some(index);
                break;
            }
        }
    }
    // The longest horizontal of the net past the first segment and past the first name, when it is long: three times
    // what the name needs.
    // The segments of the first branch up to the one of the first name: the other branches repeat them as their trunk.
    let past = first.unwrap_or(0);
    let before: HashSet<((i64, i64), (i64, i64))> = branches
        .first()
        .map(|branch| branch.windows(2).take(past + 1).filter_map(|pair| Some((*pair.first()?, *pair.get(1)?))).collect())
        .unwrap_or_default();
    let longest = branches
        .iter()
        .enumerate()
        .flat_map(|(number, branch)| {
            let before = &before;
            branch
                .windows(2)
                .take(open(number, branch))
                .skip(1)
                .filter_map(|pair| Some((*pair.first()?, *pair.get(1)?)))
                .filter(move |segment| !before.contains(segment))
                .filter_map(|(a, b)| across(a, b))
        })
        .max_by_key(|&(x0, x1, y)| (x1 - x0, -y, -x0));
    if let Some((x0, x1, y)) = longest
        && x1 - x0 >= 3 * (text.width + 2 * side)
    {
        put(x0 + side, x1 - side, y, true, &mut shapes);
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
