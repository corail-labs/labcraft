//! L7: the canvas that a Craft app embeds to show the drawings of `diagram` (`docs/architecture.md`, 11.4 and 12.6;
//! D-27). It depends on `diagram` and egui only: no engine, no file, no window.
//!
//! It paints, and decides nothing of the notation: every position, text and colour comes from the [`Drawing`]. What a
//! gesture asks for comes back in [`CanvasResponse`], as an [`Intent`] or a choice of the view, and the host makes it a
//! command (`view.select`, `view.set_state`). The selection is the host's ([`Shown::selection`]): the canvas finds it
//! on the drawing and paints it.
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

use std::collections::HashMap;
use std::sync::Arc;

use egui::{Color32, PointerButton, Pos2, Rect, Response, Sense, Ui, Vec2};
use labcraft_diagram::style;
use labcraft_diagram::{COARSE_NODE, Choice, Drawing, Overlay, Series, Shape, TabKind, Trace};

mod blockdrag;
mod blockview;
mod clip;
mod edge;
mod fields;
mod fonts;
mod frames;
pub mod insertion;
pub(crate) mod moving;
pub(crate) mod paint;
mod parts;
mod pins;
mod quick;
pub(crate) mod rerouting;
pub(crate) mod select;
pub(crate) mod series;
mod signature;
mod sizing;
pub(crate) mod view;
pub(crate) mod wiring;

pub use blockview::{BlockIntent, BlockView, MAX_ENLARGED, enlarged_zoom, swatch};
pub use fields::FieldKind;
pub use fonts::{FAMILY, HACK, family, has_family, install_fonts};
pub use frames::{InFrame, STRUCTURES, Seen};
pub use parts::PartAsked;
pub use quick::{Dragged, Quick, Source};
pub use select::{Selection, Step};
pub use signature::{default_is_cut, frame_default, frame_height, frame_terminal, own_name};
pub use view::{GLIDE_SECONDS, MAX_ZOOM, MIN_ZOOM};

use moving::Moving;
use paint::{Over, Painted, Painter};
use rerouting::Rerouting;
use select::{Found, Hit, Index};
use sizing::{Grip, Resizing};
use view::{Glide, Grid, Viewport};
use wiring::{Retract, Rules, Wiring};

/// The colours of the theme tokens (notation 8): those of the style in force in `diagram`, light or dark.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Theme {
    #[default]
    Light,
    Dark,
}

impl Theme {
    /// The colour of a token; an unknown token is magenta, which a test of every drawing keeps from happening.
    pub fn color(self, token: &str) -> Color32 {
        let Some(rgb) = style::color(token, self == Theme::Dark) else { return Color32::from_rgb(255, 0, 255) };
        let [_, red, green, blue] = rgb.to_be_bytes();
        Color32::from_rgb(red, green, blue)
    }

    /// The colour of a token, or of a colour that a file writes (spec 11.2) in this theme.
    pub fn colour(self, colour: labcraft_diagram::Colour) -> Color32 {
        let Some(rgb) = colour.value(self == Theme::Dark) else { return Color32::from_rgb(255, 0, 255) };
        let [_, red, green, blue] = rgb.to_be_bytes();
        Color32::from_rgb(red, green, blue)
    }
}

/// What the host shows of a drawing without drawing it again (architecture 11.4, 12.6): the arm shown of each structure
/// in tabs, by the key of the structure, the first when it is not said or when the structure has fewer arms now; the
/// selection of the window; and how a drag on the empty canvas acts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shown {
    pub arms: HashMap<String, usize>,
    /// The selection of the window (`view.select`), as the host holds it.
    pub selection: Selection,
    /// The setting "drag on the empty canvas pans" (notation 2.6): a drag on the empty canvas pans, and Shift with it
    /// selects. Off by default: a drag selects with a rectangle.
    pub drag_pans: bool,
    /// The setting "mark the fixed nodes" (notation 2.4): a pin on each node that `layout` fixes. On in the IDE by
    /// default; the host says it.
    pub mark_fixed: bool,
    /// The setting "replace only with Alt" (notation 2.4): a wire dropped on an input that holds something replaces it
    /// only with Alt down. Off by default.
    pub replace_with_alt: bool,
    /// The setting "accept it" (notation 2.4): a wire of the wrong type is written, and `check` gives `LC3001`. Off by
    /// default.
    pub accept_mismatch: bool,
    /// The modifier of a wire moved by hand (notation 2.6): Shift+Alt and a drag on a segment instead of Alt, for a
    /// desktop that keeps Alt and a drag to move windows. Off by default.
    pub route_with_shift_alt: bool,
    /// The setting "inserting into a wire" (notation 2.6): a function of the panel `Palette` dropped on a wire that it
    /// can take goes into it. On in the IDE by default; the host says it.
    pub into_wire: bool,
}

impl Shown {
    pub fn arm_of(&self, structure: &str) -> usize {
        self.arms.get(structure).copied().unwrap_or(0)
    }
}

/// What a gesture asks for (architecture 12.6): the host makes it a command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    /// `view.select`: this selection, the whole of it.
    Select(Selection),
    /// `node.move`: these nodes of one frame, as a command names them, by `[dx, dy]` in units (notation 2.4). The canvas
    /// shows them there until the next drawing, or until the host calls [`Canvas::cancel_move`].
    Move { nodes: Vec<String>, by: (i64, i64) },
    /// `layout.arrange` on these nodes: "Remove the fixed position" (notation 2.4).
    Arrange(Selection),
    /// `wire.connect` from an output to an input, as a command names them (commands.md 9.2): a wire dropped, or wired
    /// with Enter (notation 2.6). `replace` when the input holds something that it replaces (notation 2.4).
    /// `route`: a branch drawn from a wire, with `wire.route` after it in one `commands.batch`, its junction first
    /// (notation 2.4): the wire as `wire.route` names it and its points in units of its frame.
    /// `arm`: the arm of a `yield`, by its patterns (commands.md 9.2).
    /// `through`: how each loop that the wire leaves takes the value out (`carry`, `collect`), from the inside out, as
    /// `wire.connect` takes it; empty when it leaves none.
    Wire {
        from: String,
        to: String,
        arm: Option<String>,
        replace: bool,
        accept_mismatch: bool,
        route: Option<(String, Vec<(i64, i64)>)>,
        through: Vec<&'static str>,
    },
    /// `wire.route` for each of these wires, in one `commands.batch`: a segment moved by hand (notation 2.4).
    Route(Vec<(String, Vec<(i64, i64)>)>),
    /// `frame.resize`: the frame of the structure named, or of the function with none, to `[w, h]` in units from its
    /// origin, by its right edge, its bottom edge or its corner (notation 2.6, spec 11); with no size, "Remove the fixed
    /// size": its entry goes, and its size is automatic again (notation 2.4).
    Resize { structure: Option<String>, size: Option<(i64, i64)> },
    /// Ctrl and a drag took the end of a wire to another terminal (notation 2.6): these `wire.disconnect`, then these
    /// `wire.connect` (`from`, `to`, `replace`, `accept_mismatch`), in one `commands.batch`; each with the arm of a
    /// `yield` that it reaches.
    Rewire { disconnect: Vec<(String, Option<String>)>, connect: Vec<(String, String, bool, bool, Option<String>)> },
    /// Quick insertion (notation 2.6): a wire dropped on the empty canvas; the host opens the list where it says, and
    /// Space opens it too ([`Canvas::quick_insert`]).
    QuickInsert(Quick),
    /// A function of the panel `Palette` dropped on the canvas (notation 2.5): `node.add` with `at`, in units of the frame
    /// of the function, or `into` the wire it was dropped on.
    /// `within`: the frame under the drop, an arm or a loop body (commands.md 6.7).
    Drop { call: String, at: (i64, i64), within: InFrame, into: Option<String> },
    /// What the status bar says, and a screen reader announces (notation 2.6): why a drop wrote nothing.
    Say(String),
    /// "Surround with ▸" of the context menu of a node (notation 2.6): the host surrounds `nodes` (the node right-clicked,
    /// or the selection that holds it) with a structure of this kind (`match`, `handle`, `for`, `loop`), and first asks for
    /// its value where it needs one.
    Surround { kind: &'static str, nodes: Vec<String> },
    /// A field in place written (notation 2.6): the host sends its command, and calls [`Canvas::close_field`] when it is
    /// done or [`Canvas::field_refused`] with the message of a refusal.
    Field { kind: FieldKind, text: String },
    /// `structure.remove_part` (notation 2.6): an arm by its patterns, `carry x`, `in x` or `collect y`.
    RemovePart { structure: String, part: String },
    /// `structure.set_passage` (notation 2.6): a part `carry x`, `collect y` or `last y` of a loop becomes a `to`, `carry`,
    /// `collect` or `last`, from the context menu of its terminal.
    SetPassage { structure: String, part: String, to: &'static str },
    /// `structure.set_option`: `continue_on_error` of a `loop`, `parallel` of a `for`.
    SetOption { structure: String, option: &'static str, on: bool },
    /// "Add clause…" of a `for`: the host opens the list of the values it can go over (notation 2.6); with `over`, a
    /// wire of an array dropped on its left edge, `structure.add_clause` at once.
    AddClause { structure: String, over: Option<String> },
    /// `structure.add_collect`, or `structure.add_last` with `last`: a wire from inside a loop dropped on its inner right
    /// edge, Alt held for a `last` (notation 2.6).
    AddCollect { structure: String, value: String, last: bool },
    /// "Make an input of the function", "Make an output of the function" (notation 2.4): `signature.add` of `terminal`
    /// (`input gain`) of type `ty`, then `wire.connect` from `from` to `to`, in one `commands.batch`.
    MakeTerminal { terminal: String, ty: String, from: String, to: String },
    /// `signature.remove`: "Remove" of the menu of a terminal of the frame of the function.
    RemoveTerminal { terminal: String },
    /// `terminal.move`: a terminal of the frame of the function (`input x`) to the height `y`, in units from the origin of
    /// the frame (spec 11), by a drag along its edge or a run of Ctrl and an arrow (notation 2.4). The canvas shows it there
    /// until the next drawing, or until the host calls [`Canvas::cancel_move`].
    MoveTerminal { terminal: String, y: i64 },
    /// "Type…" of the menu of a terminal of the frame of the function: the host opens the list of types, and
    /// `signature.set_type` with the one chosen (decision of the owner, 2026-11-19).
    PickType { terminal: String },
    /// `block.set_terminal`: Alt and a drag moved a terminal of a block (`input x`) onto a place of a side of its block
    /// (notation 2.4), in the file of the function called.
    BlockTerminal { file: String, terminal: String, side: &'static str, place: u32 },
    /// `block.reset`: "Reset the terminals of this block" of the context menu of a node (notation 2.4).
    ResetBlock { file: String },
}

/// A gesture in progress: the canvas's own state, which the host follows (the gesture lock comes with step 8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gesture {
    /// A rectangle of selection is being drawn.
    Rectangle,
    /// Nodes are being moved, by a drag or by a run of presses of Ctrl and an arrow.
    Move,
    /// A wire is being drawn, by a drag or from the keyboard.
    Wire,
    /// A segment of a wire is being moved (notation 2.4).
    Route,
    /// A frame is being resized by its edge (notation 2.6).
    Resize,
}

/// What a frame of the canvas gives back to the host.
pub struct CanvasResponse {
    pub response: egui::Response,
    /// A tab clicked: the key of its structure and the rank of its arm, for `view.set_state`.
    pub tab: Option<(String, usize)>,
    /// An iteration chosen, by a selector or on the values of a wire pinned: the key of the loop and the iteration, from
    /// 0, for `view.set_state` (notation 6.1).
    pub iteration: Option<(String, u64)>,
    /// The node under the pointer, by its index in [`Drawing::placed`].
    pub hovered: Option<usize>,
    /// What the gestures of this frame ask for, in their order.
    pub intents: Vec<Intent>,
    /// The gesture under way, if any.
    pub gesture: Option<Gesture>,
}

/// A drag of the primary button, from where it started.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Drag {
    /// A rectangle of selection, in units, adding to the selection or not.
    Rectangle { from: Pos2, to: Pos2, adding: bool },
    /// The setting "drag on the empty canvas pans" is on.
    Pan,
    /// From a node, a wire or a terminal: moving and wiring come with steps 2b and 3.
    Held,
}

/// The canvas: the drawing shown, its viewport, its caches, the glide.
pub struct Canvas {
    drawing: Option<Arc<Drawing>>,
    /// Changes with each drawing and each overlay, so that the caches know theirs.
    generation: u64,
    viewport: Viewport,
    grid: Grid,
    /// What the drawing names, for the selection.
    index: Index,
    glide: Option<Glide>,
    reduced_motion: bool,
    painter: Painter,
    /// The list of the arms of a selector, open: its structure, by its index in [`Drawing::placed`], and the corner it
    /// opens from, in units, so that it follows a pan and a zoom (notation 5.2).
    list: Option<(usize, Pos2)>,
    /// The values of a run over the drawing, and whether they show (notation 6.1).
    values: Option<Values>,
    values_shown: bool,
    /// The trace of those values and the iterations chosen: the values of a wire over the iterations of its loop come
    /// from it, worked out when the wire is first hovered.
    run: Option<(Arc<Trace>, Choice)>,
    /// The series of each wire hovered or selected, by its group: none for a wire outside every loop.
    series: HashMap<usize, Option<Arc<Series>>>,
    /// The selection of the host found on the drawing, and the selection it was found for.
    found: Found,
    found_for: Option<Selection>,
    /// The selection asked for last, until the host's changes: what Shift, Ctrl or a rectangle adds to, so that a second
    /// click before the host answers keeps the first.
    asked_found: Option<Found>,
    /// The wire clicked last, by its group, and where, in units: its values stay pinned there while it is selected
    /// (notation 6.1; decisions of the owner, 2026-11-13).
    pinned: Option<(usize, Pos2)>,
    /// The drag under way.
    drag: Option<Drag>,
    /// A number being typed into the selector of a loop.
    typing: Option<Typing>,
    /// What the wheel turned over a selector and did not make a step yet, in points.
    wheel: f32,
    /// The iteration asked for last, until the values for it come: the next step goes from it, and the same is not asked
    /// twice.
    asked: Option<(String, u64)>,
    /// A move under way, or asked of the host and not drawn yet.
    moving: Option<Moving>,
    /// The fixed node that a right-click was on, for its menu (notation 2.4).
    context: Option<usize>,
    /// A wire being drawn, or asked of the host and not drawn yet.
    wiring: Option<Wiring>,
    /// The terminal that has the focus, by its index in [`Drawing::ports`]: Enter on a node goes to its terminals
    /// (notation 2.6).
    port_focus: Option<usize>,
    /// The settings that a drop reads, from the host ([`Shown`]).
    rules: Rules,
    /// The old wire that a new one replaces, retracting (notation 2.4): asked with the wire, started by the new drawing.
    retract: Option<Retract>,
    /// A segment of a wire being moved, or asked of the host and not drawn yet.
    rerouting: Option<rerouting::Rerouting>,
    /// A frame resized by its edge (notation 2.6).
    resizing: Option<Resizing>,
    /// A terminal of the frame of the function moved along its edge, or asked of the host and not drawn yet (notation 2.4).
    sliding: Option<edge::Sliding>,
    /// A terminal of a block moved by Alt and a drag (notation 2.4).
    block_drag: Option<blockdrag::BlockDrag>,
    /// The wire routed by hand that a right-click was on, by its group, for its menu (notation 2.4).
    wire_context: Option<usize>,
    /// The room of the last frame, on the screen: where quick insertion opens.
    rect: Rect,
    /// The field in place open (notation 2.6).
    field: Option<fields::Field>,
    /// A field closed: the canvas takes the keyboard back at its next frame.
    refocus: bool,
    /// The arm whose tab a right-click was on, by its rank, and the terminal it was on: the parts of a structure in its
    /// menu (notation 2.6).
    context_arm: Option<usize>,
    context_port: Option<usize>,
    /// The frame of the function right-clicked by its edge: its menu (notation 2.4), with "Remove the fixed size" when
    /// `layout` fixes its size.
    frame_context: bool,
    /// The terminal of the frame of the function right-clicked, by its index in [`Drawing::ports`], for its menu.
    terminal_context: Option<usize>,
    /// Where the last right-click was, in units: where a field of its menu opens.
    context_at: Option<Pos2>,
    /// What the command palette asked of the canvas, for the next frame to give back.
    queued: Vec<Intent>,
}

/// The wires of a value that the file routes (`via`), as `layout.arrange` names them in `wires`.
fn routed(drawing: &Drawing, group: usize) -> Vec<String> {
    let Some(wire) = drawing.groups.get(group).and_then(|group| group.wire.as_ref()) else { return Vec::new() };
    wire.entries.iter().zip(&wire.vias).filter(|(_, via)| **via).filter_map(|(entry, _)| entry.clone()).collect()
}

/// The number typed into the selector of a loop: the loop, its iterations, the text, the corner of the field in units.
struct Typing {
    key: String,
    count: u64,
    text: String,
    /// The iteration shown, from 1, which the empty field shows faintly.
    hint: String,
    at: Pos2,
    focused: bool,
}

/// The wheel turned this far over a selector makes one step *(proposed)*: a notch of a mouse.
const WHEEL_STEP: f32 = 40.0;
/// A wire is hit within this many units of its route *(proposed)*, or 5 points on the screen when that is more
/// (architecture 12.4).
const WIRE_REACH: f32 = 4.0;
/// How high the header of a structure is, in units, as a press reads it (`select::inner_rect`): that of its first body,
/// from the top of its box.
fn header_of(drawing: &Drawing, placed: usize) -> f32 {
    let Some(found) = drawing.placed.get(placed) else { return 0.0 };
    let body = found.holds.first().and_then(|&body| Some((select::body_rect(drawing, body)?, drawing.bodies.get(body)?.header)));
    body.map_or(0.0, |(area, header)| area.min.y - found.y as f32 / 100.0 + header as f32 / 100.0)
}

/// The corner of a frame that resizes it (notation 2.6): three short diagonals inside its bottom-right corner.
fn paint_corner(painter: &egui::Painter, corner: Pos2, zoom: f32, color: Color32) {
    let stroke = egui::Stroke::new((zoom).clamp(0.75, 1.5), color);
    for reach in [3.0, 6.0, 9.0] {
        let reach = reach * zoom;
        let inset = 2.0 * zoom;
        painter.line_segment([corner + Vec2::new(-reach - inset, -inset), corner + Vec2::new(-inset, -reach - inset)], stroke);
    }
}

/// A plain drag that starts within this many units of a wire grabs it for a branch (notation 2.5), or 5 points on the
/// screen when that is more.
const BRANCH_REACH: f32 = 4.0;
/// The ghost of a function dragged from the panel `Palette`, in units *(proposed)*: the size of a small block.
const GHOST: (f32, f32) = (96.0, 40.0);

/// The values of a run over a drawing, with the box of each of their parts, in units, to find what the viewport shows and
/// what the pointer is over.
struct Values {
    overlay: Arc<Overlay>,
    under: Vec<Rect>,
    banner: Rect,
    /// Nothing for a bubble that has no place: its value is read on hover of its wire.
    bubbles: Vec<Rect>,
    selectors: Vec<Rect>,
}

impl Values {
    fn of(overlay: Arc<Overlay>) -> Values {
        let under = overlay.under.iter().map(|layer| boxed(&layer.shapes).unwrap_or(Rect::NOTHING)).collect();
        let banner = boxed(&overlay.banner).unwrap_or(Rect::NOTHING);
        let bubbles = overlay.bubbles.iter().map(|bubble| boxed(&bubble.layer.shapes).unwrap_or(Rect::NOTHING)).collect();
        let selectors = overlay.selectors.iter().map(|selector| boxed(&selector.layer.shapes).unwrap_or(Rect::NOTHING)).collect();
        Values { overlay, under, banner, bubbles, selectors }
    }

    /// The selector shown under `unit`.
    fn selector_at(&self, unit: Pos2, visible: &dyn Fn(Option<usize>) -> bool) -> Option<&labcraft_diagram::Selector> {
        self.overlay
            .selectors
            .iter()
            .zip(&self.selectors)
            .find(|(selector, area)| visible(selector.layer.within) && area.contains(unit))
            .map(|(selector, _)| selector)
    }
}

/// The box of a shape, in units: that of `diagram`, the one box of a shape (architecture 12.4).
pub(crate) fn extent(shape: &Shape) -> Rect {
    let (left, top, right, bottom) = labcraft_diagram::extent(shape);
    Rect::from_min_max(Pos2::new(left as f32, top as f32) / 100.0, Pos2::new(right as f32, bottom as f32) / 100.0)
}

/// The box of shapes, in units; none for no shape.
fn boxed(shapes: &[Shape]) -> Option<Rect> {
    shapes.iter().map(extent).reduce(|a, b| a.union(b))
}

/// Whether two boxes share some area: boxes that only touch do not.
fn overlaps(a: &Rect, b: &Rect) -> bool {
    a.min.x < b.max.x && b.min.x < a.max.x && a.min.y < b.max.y && b.min.y < a.max.y
}

/// What a frame works with, read before the input and the painting.
struct Frame {
    rect: Rect,
    drawing: Arc<Drawing>,
    theme: Theme,
    ctx: egui::Context,
    now: f64,
    /// Whether each arm in tabs is shown, by its index in [`Drawing::arms`].
    arms: Vec<bool>,
    /// The values shown, while they show.
    values: Option<Arc<Overlay>>,
    /// Where each node stands from its place while nodes glide or move, by its index in [`Drawing::placed`].
    offsets: Option<Vec<Vec2>>,
    /// The offsets are those of a move, not of a glide: the wires of the preview show, and the selection.
    moving: bool,
    /// The setting "mark the fixed nodes" (notation 2.4).
    mark_fixed: bool,
    /// The groups that the viewport shows, in the order of drawing.
    groups: Vec<usize>,
    /// The point under the pointer, in units.
    at: Option<Pos2>,
    hovered: Option<usize>,
    wire_under: Option<usize>,
    /// The edge or the header of a frame under the pointer, and what a press there takes (notation 2.6).
    grip: Option<(sizing::Target, Grip)>,
}

impl Frame {
    fn visible(&self, within: Option<usize>) -> bool {
        within.is_none_or(|arm| self.arms.get(arm).copied().unwrap_or(false))
    }

    /// Whether a tab shows: its structure is shown, and a part of a selector with the arm it stands for.
    fn tab_shown(&self, tab: &labcraft_diagram::Tab) -> bool {
        self.drawing.arms.get(tab.arm).is_some_and(|arm| self.visible(arm.within))
            && tab.when.is_none_or(|when| self.arms.get(when).copied().unwrap_or(false))
    }

    fn offset_of(&self, placed: Option<usize>) -> Vec2 {
        match (&self.offsets, placed) {
            (Some(offsets), Some(placed)) => offsets.get(placed).copied().unwrap_or(Vec2::ZERO),
            _ => Vec2::ZERO,
        }
    }
}

/// What the input of a frame asks for and decides, which the painting reads.
#[derive(Default)]
struct Input {
    tab: Option<(String, usize)>,
    iteration: Option<(String, u64)>,
    intents: Vec<Intent>,
    /// The hint under the pointer, its text, while nothing else says more.
    hint: Option<String>,
}

impl Default for Canvas {
    fn default() -> Self {
        Canvas::new()
    }
}

impl Canvas {
    pub fn new() -> Canvas {
        Canvas {
            drawing: None,
            generation: 0,
            viewport: Viewport::default(),
            grid: Grid::default(),
            index: Index::default(),
            glide: None,
            reduced_motion: false,
            list: None,
            painter: Painter::default(),
            values: None,
            values_shown: true,
            run: None,
            series: HashMap::new(),
            found: Found::default(),
            found_for: None,
            asked_found: None,
            pinned: None,
            drag: None,
            typing: None,
            wheel: 0.0,
            asked: None,
            moving: None,
            context: None,
            wiring: None,
            port_focus: None,
            rules: Rules::default(),
            retract: None,
            rerouting: None,
            resizing: None,
            sliding: None,
            block_drag: None,
            wire_context: None,
            rect: Rect::NOTHING,
            field: None,
            refocus: false,
            context_arm: None,
            context_port: None,
            frame_context: false,
            terminal_context: None,
            context_at: None,
            queued: Vec::new(),
        }
    }

    /// The values of a run over the drawing shown (notation 6.1), made for it, with the trace they come from and the
    /// iterations chosen; none to show none. A new drawing comes without them: its values are those of its own trace.
    pub fn set_values(&mut self, overlay: Option<Arc<Overlay>>, run: Option<(Arc<Trace>, Choice)>) {
        self.values = overlay.map(Values::of);
        self.run = run;
        self.series.clear();
        self.asked = None;
        // Only what is painted of the values goes: an iteration that moves draws nothing again.
        self.painter.clear_values();
    }

    /// Hides the values of the run, or shows them again: a state of the view, which places nothing (notation 6.1).
    pub fn set_values_shown(&mut self, shown: bool) {
        self.values_shown = shown;
    }

    pub fn values_shown(&self) -> bool {
        self.values_shown
    }

    /// Shows another drawing, keeping the viewport (scenario 6, point 3). With `glide`, which the host asks for when the
    /// file of the function shown changed (notation 2.1), each node goes from its old place to its new one, found by
    /// its key, unless the system asks for reduced motion; not for another function. The selection, which the host
    /// names, is found again on it.
    pub fn set_drawing(&mut self, drawing: Arc<Drawing>, glide: bool) {
        // The nodes of a move asked for are where it put them: they do not glide there (architecture 12.4).
        // A move under way goes on over the new drawing, its nodes found again by their keys; one asked for ends.
        let (moved, going) = match self.moving.take() {
            Some(moving) if moving.state == moving::State::Asked => {
                (Some((moving.preview.carried.into_iter().collect::<Vec<usize>>(), moving.by)), None)
            }
            Some(moving) => (None, self.drawing.as_ref().and_then(|old| moving.again(old, &drawing))),
            None => (None, None),
        };
        self.glide = match (&self.drawing, glide && !self.reduced_motion) {
            (Some(old), true) => match &moved {
                Some((nodes, by)) => Glide::after_move(old, &drawing, nodes, (by.0 * 100, by.1 * 100)),
                None => Glide::between(old, &drawing),
            },
            _ => None,
        };
        // The viewport follows the frame of the function, when the same function is drawn again: what the file places
        // above or left of it moves the whole drawing, not what the canvas shows (spec 11).
        if glide && let Some(old) = &self.drawing {
            let (dx, dy) = view::shift_of_frame(old, &drawing);
            self.viewport.origin += Vec2::new(dx as f32, dy as f32) / 100.0;
        }
        self.context = None;
        self.wire_context = None;
        // A size asked for is in the new drawing; a resize under way ends, its frame drawn anew. So does a terminal moved.
        self.resizing = None;
        self.sliding = match self.sliding.take() {
            Some(sliding) if sliding.state != edge::State::Asked => sliding.again(&drawing),
            _ => None,
        };
        // A route asked for is in the new drawing. A segment being moved goes on over the new drawing when its wire is
        // drawn there as it was; otherwise it ends.
        self.rerouting = match (self.rerouting.take(), &self.drawing) {
            (Some(rerouting), Some(old)) => rerouting.again(old, &drawing),
            _ => None,
        };
        // A wire asked for is in the new drawing, and the old wire it replaced retracts now. A wire under way goes on over
        // the new one, from its terminal found again by its name; one that Ctrl took ends.
        self.wiring =
            self.wiring.take().filter(|wiring| wiring.state != wiring::State::Asked && wiring.taken == wiring::Taken::Nothing).and_then(|wiring| {
                let old = self.drawing.as_ref()?.ports.get(wiring.from)?;
                let from = drawing.ports.iter().position(|port| port.input == old.input && port.name == old.name)?;
                let mut again = Wiring::new(&drawing, from, wiring.at, wiring.state)?;
                again.over = None;
                Some(again)
            });
        if let Some(retract) = &mut self.retract {
            retract.drawn = true;
        }
        if self.reduced_motion {
            self.retract = None;
        }
        // The terminal of the focus, found again by its name and its node.
        self.port_focus = self.port_focus.and_then(|at| {
            let old = self.drawing.as_ref()?.ports.get(at)?;
            let key = old.placed.and_then(|placed| self.drawing.as_ref()?.placed.get(placed)).map(|placed| placed.key.clone());
            let same_node = |port: &&labcraft_diagram::Port| {
                port.input == old.input && port.placed.and_then(|placed| drawing.placed.get(placed)).map(|placed| placed.key.clone()) == key
            };
            // By the name a command gives it, else by its name in its node.
            drawing
                .ports
                .iter()
                .position(|port| same_node(&port) && !old.name.is_empty() && port.name == old.name)
                .or_else(|| drawing.ports.iter().position(|port| same_node(&port) && port.caption == old.caption))
        });
        self.grid = Grid::of(&drawing);
        self.index = Index::of(&drawing);
        // A list open on the drawing before: its structure may be another one now.
        self.list = None;
        self.typing = None;
        self.found_for = None;
        self.asked_found = None;
        self.pinned = None;
        self.drag = None;
        self.generation = self.generation.wrapping_add(1);
        self.drawing = Some(drawing);
        self.set_values(None, None);
        self.moving = going;
    }

    pub fn drawing(&self) -> Option<&Arc<Drawing>> {
        self.drawing.as_ref()
    }

    /// No glide: what the system asks for (`prefers-reduced-motion` in a browser), or a preference of the viewer.
    pub fn set_reduced_motion(&mut self, reduced: bool) {
        self.reduced_motion = reduced;
        if reduced {
            self.glide = None;
        }
    }

    /// The zoom, 1 for 100 %, and the point of the drawing at the top-left corner of the canvas, in units.
    pub fn viewport(&self) -> (f32, Vec2) {
        (self.viewport.zoom, self.viewport.origin)
    }

    pub fn set_viewport(&mut self, zoom: f32, origin: Vec2) {
        self.viewport.zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        self.viewport.origin = origin;
        self.viewport.fitted = true;
    }

    /// As `set_viewport`, but a new zoom counts as moving, as a gesture's does: its shapes are tessellated at a step of
    /// it, and its texts laid out again once it has not changed for 120 ms (11.3). For a pan and a zoom that a host
    /// drives, as the motion that `labcraft` times (`LABCRAFT_FRAMES`).
    pub fn move_viewport(&mut self, zoom: f32, origin: Vec2) {
        let zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        self.viewport.zoom_moved |= zoom != self.viewport.zoom;
        self.set_viewport(zoom, origin);
    }

    /// The whole drawing in `size`, at 100 % at most (notation 7), at the next frame.
    pub fn fit(&mut self) {
        self.viewport.fitted = false;
    }

    /// Whether a glide is under way.
    pub fn gliding(&self) -> bool {
        self.glide.is_some()
    }

    /// The move asked for failed: its nodes go back to their places.
    /// A move under way (a drag, a run of keys) is not the one that failed, and goes on.
    pub fn cancel_move(&mut self) {
        if self.moving.as_ref().is_some_and(|moving| moving.state == moving::State::Asked) {
            self.moving = None;
        }
        if self.resizing.as_ref().is_some_and(|resizing| resizing.state == sizing::State::Asked) {
            self.resizing = None;
        }
        if self.sliding.as_ref().is_some_and(|sliding| sliding.state == edge::State::Asked) {
            self.sliding = None;
        }
    }

    /// The wire asked for failed: it goes, and the old wire it would have replaced stays.
    pub fn cancel_wire(&mut self) {
        if self.wiring.as_ref().is_some_and(|wiring| wiring.state == wiring::State::Asked) {
            self.wiring = None;
        }
        if self.rerouting.as_ref().is_some_and(|rerouting| rerouting.state == rerouting::State::Asked) {
            self.rerouting = None;
        }
        self.retract = None;
    }

    /// W on the terminal that has the focus starts a wire from it (notation 2.6): Tab then goes through the terminals that
    /// would take it, Enter wires it, Escape cancels. Nothing without a terminal in focus.
    pub fn start_wire(&mut self, shown: &Shown) -> bool {
        let (Some(drawing), Some(from)) = (self.drawing.clone(), self.port_focus) else { return false };
        let Some(port) = drawing.ports.get(from) else { return false };
        let Some(mut wiring) = Wiring::new(&drawing, from, wiring::centre(port), wiring::State::Keys) else { return false };
        let arms = arms_shown(&drawing, shown);
        let visible = |within: Option<usize>| within.is_none_or(|arm| arms.get(arm).copied().unwrap_or(false));
        wiring.over = wiring.takers(&drawing, rules_of(shown, false), &visible).first().copied();
        self.wiring = Some(wiring);
        true
    }

    /// The terminal that has the focus, as a command names it, and whether it is an input.
    pub fn port_focus(&self) -> Option<(String, bool)> {
        let port = self.drawing.as_ref()?.ports.get(self.port_focus?)?;
        Some((port.name.clone(), port.input))
    }

    /// Ends a run of presses of Ctrl and an arrow now (notation 2.6: another key ends it): the move it asks for, if any.
    /// The host calls it before it runs a key of its own, so that the move comes before what the key does.
    pub fn end_run(&mut self) -> Option<Intent> {
        if let Some(sliding) = self.sliding.as_mut().filter(|sliding| matches!(sliding.state, edge::State::Keys { .. })) {
            if !sliding.changed() {
                self.sliding = None;
                return None;
            }
            sliding.state = edge::State::Asked;
            return Some(Intent::MoveTerminal { terminal: sliding.terminal.clone(), y: sliding.y });
        }
        let moving = self.moving.as_mut().filter(|moving| matches!(moving.state, moving::State::Keys { .. }))?;
        moving.state = moving::State::Asked;
        let (by, nodes) = (moving.by, moving.nodes.clone());
        let drawing = self.drawing.clone()?;
        self.move_intent(&drawing, &nodes, by)
    }

    /// Moves these nodes (by their index in [`Drawing::placed`]) by `by` in units, as a drag would, for a host that drives
    /// the canvas itself: the scripted drag of `LABCRAFT_FRAMES` (scenario 6 of M1). [`Canvas::drop_held`] ends it.
    pub fn hold_move(&mut self, nodes: &[usize], by: (i64, i64)) {
        let Some(drawing) = self.drawing.clone() else { return };
        let moving = match &mut self.moving {
            Some(moving) if moving.state == moving::State::Held && moving.nodes == nodes => moving,
            _ => self.moving.insert(Moving::new(&drawing, nodes.to_vec(), moving::State::Held)),
        };
        moving.set_by(&drawing, by);
    }

    /// Ends a move that the host holds: the move it asks for, as at the drop of a drag.
    pub fn drop_held(&mut self) -> Option<Intent> {
        let moving = self.moving.as_mut().filter(|moving| moving.state == moving::State::Held)?;
        moving.state = moving::State::Asked;
        let (by, nodes) = (moving.by, moving.nodes.clone());
        let drawing = self.drawing.clone()?;
        self.move_intent(&drawing, &nodes, by)
    }

    /// `node.move` for these nodes by `by`; none for a move by nothing, which leaves the nodes where they are.
    fn move_intent(&mut self, drawing: &Drawing, nodes: &[usize], by: (i64, i64)) -> Option<Intent> {
        let names: Vec<String> =
            nodes.iter().filter_map(|&at| drawing.placed.get(at)).map(|placed| placed.name.clone()).filter(|name| !name.is_empty()).collect();
        if by == (0, 0) || names.is_empty() {
            self.moving = None;
            return None;
        }
        Some(Intent::Move { nodes: names, by })
    }

    /// The selection that a key of notation 2.6 asks for (Ctrl+A, Alt+Left, Alt+Right…), from the selection of `shown`;
    /// none without a drawing. The host binds the keys (`craft-keymap`) and makes it `view.select`.
    pub fn step_selection(&self, step: Step, shown: &Shown) -> Option<Selection> {
        let drawing = self.drawing.as_ref()?;
        let arms = arms_shown(drawing, shown);
        let visible = |within: Option<usize>| within.is_none_or(|arm| arms.get(arm).copied().unwrap_or(false));
        let found = self.index.find(&shown.selection);
        Some(self.index.name(drawing, &select::step(drawing, &self.index, &found, step, &visible)))
    }

    /// Paints the drawing in all the room that `ui` gives, and reads the pointer and the keys (notation 2.6): a click or
    /// a rectangle selects, the middle button or the wheel pans, the wheel with Ctrl or a pinch zooms, a click on a tab
    /// shows its arm; with the focus, Tab, the arrows, Enter and Escape move the selection.
    pub fn show(&mut self, ui: &mut Ui, theme: Theme, shown: &Shown) -> CanvasResponse {
        // With the values its drawing was drawn with (notation 10).
        let style = self.drawing.as_ref().map(|drawing| drawing.style.clone()).unwrap_or_default();
        style::scope(&style, || self.show_in_style(ui, theme, shown))
    }

    fn show_in_style(&mut self, ui: &mut Ui, theme: Theme, shown: &Shown) -> CanvasResponse {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        self.rect = rect;
        ui.painter_at(rect).rect_filled(rect, 0.0, theme.color("canvas.fill"));
        let Some(drawing) = self.drawing.clone() else {
            return CanvasResponse { response, tab: None, iteration: None, hovered: None, intents: Vec::new(), gesture: None };
        };
        if self.found_for.as_ref() != Some(&shown.selection) {
            self.found = self.index.find(&shown.selection);
            // The terminal in focus is one of the node selected: another selection takes it away.
            self.port_focus = self
                .port_focus
                .filter(|&at| drawing.ports.get(at).and_then(|port| port.placed).is_some_and(|placed| self.found.nodes.contains(&placed)));
            self.found_for = Some(shown.selection.clone());
            self.asked_found = None;
            // A wire that a command names is pinned while the host holds it; one that it cannot name, while nothing
            // else is selected.
            self.pinned = self.pinned.filter(|(group, _)| {
                let named = drawing.groups.get(*group).and_then(|found| found.wire.as_ref()).is_some_and(|wire| !wire.named.is_empty());
                if named { self.found.groups.contains(group) } else { self.found.is_empty() }
            });
        }
        self.rules = rules_of(shown, false);
        let mut input = Input::default();
        let frame = self.begin(ui, &response, (rect, theme), drawing, shown, &mut input);
        // The keyboard comes back to the canvas when a field closes (notation 2.6).
        if std::mem::take(&mut self.refocus) {
            response.request_focus();
        }
        self.input(ui, &response, &frame, shown, &mut input);
        let response = self.paint(ui, response, &frame, theme, &input);
        self.describe(ui, &response, &frame);
        // Never the same iteration twice while its values are on their way.
        let iteration = input.iteration.filter(|asked| self.asked.as_ref() != Some(asked));
        if iteration.is_some() {
            self.asked.clone_from(&iteration);
        }
        let gesture = if matches!(self.drag, Some(Drag::Rectangle { .. })) {
            Some(Gesture::Rectangle)
        } else if self.moving.as_ref().is_some_and(|moving| !matches!(moving.state, moving::State::Asked)) {
            Some(Gesture::Move)
        } else if self.wiring.as_ref().is_some_and(|wiring| wiring.state != wiring::State::Asked) {
            Some(Gesture::Wire)
        } else if self.rerouting.as_ref().is_some_and(|rerouting| rerouting.state == rerouting::State::Dragging) {
            Some(Gesture::Route)
        } else if self.resizing.as_ref().is_some_and(|resizing| resizing.state == sizing::State::Dragging) {
            Some(Gesture::Resize)
        } else if self.sliding.as_ref().is_some_and(|sliding| sliding.state != edge::State::Asked) || self.block_drag.is_some() {
            Some(Gesture::Move)
        } else {
            None
        };
        CanvasResponse { response, tab: input.tab, iteration, hovered: frame.hovered, intents: input.intents, gesture }
    }

    /// What the frame works with: the viewport moved by the pointer, the glide, what the viewport shows and what the
    /// pointer is over.
    fn begin(
        &mut self,
        ui: &Ui,
        response: &Response,
        (rect, theme): (Rect, Theme),
        drawing: Arc<Drawing>,
        shown: &Shown,
        input: &mut Input,
    ) -> Frame {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|input| input.time);
        let values = self.values.as_ref().filter(|_| self.values_shown).map(|values| values.overlay.clone());
        let room = values.as_ref().map_or((0.0, 0.0), |values| (values.top as f32 / 100.0, values.width as f32 / 100.0));
        let arms = arms_shown(&drawing, shown);
        let visible_arm = |within: Option<usize>| within.is_none_or(|arm| arms.get(arm).copied().unwrap_or(false));
        // The selectors show with the values, from `zoom.coarse` on (notation 7).
        let selectors_shown = values.is_some() && self.viewport.zoom >= style::factor(style::coarse_zoom());
        let unit_of = |viewport: &Viewport, at: Pos2| (viewport.origin + (at - rect.min) / viewport.zoom).to_pos2();
        // The wheel over a selector steps its iterations, down forward, and does not scroll (decisions of the owner,
        // 2026-11-13). Not while nodes glide, when no selector shows.
        let under_wheel = response.hover_pos().filter(|_| selectors_shown && self.glide.is_none()).and_then(|at| {
            let selector = self.values.as_ref()?.selector_at(unit_of(&self.viewport, at), &visible_arm)?;
            Some((selector.key.clone(), self.shown_of(selector), selector.count))
        });
        match &under_wheel {
            Some((key, shown, count)) if ctx.input(|input| input.zoom_delta()) == 1.0 => {
                self.wheel += ctx.input(|input| input.smooth_scroll_delta.y);
                let steps = (self.wheel / WHEEL_STEP).trunc();
                self.wheel -= steps * WHEEL_STEP;
                let next = (i128::from(*shown) - steps as i128).clamp(0, i128::from(count.saturating_sub(1)));
                if steps != 0.0 && next != i128::from(*shown) {
                    input.iteration = u64::try_from(next).ok().map(|next| (key.clone(), next));
                }
            }
            _ => self.wheel = 0.0,
        }
        // The middle button pans, and the primary one when the setting says so (notation 2.6).
        let pan = response.dragged_by(PointerButton::Middle) || (response.dragged_by(PointerButton::Primary) && self.drag == Some(Drag::Pan));
        self.viewport.read_input(&ctx, response, rect, &drawing, room, under_wheel.is_some(), pan);
        // A frame when the zoom has settled, to lay its texts out again at the zoom itself.
        if let Some(left) = self.viewport.settles_in(now) {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(left));
        }
        // A move asked for whose drawing does not come (the file no longer draws) gives its nodes back after a while.
        if let Some(asked) = self.moving.as_mut().filter(|moving| moving.state == moving::State::Asked) {
            let since = *asked.asked_at.get_or_insert(now);
            if now - since > moving::ASKED_SECONDS {
                self.moving = None;
            } else {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(moving::ASKED_SECONDS));
            }
        }
        if let Some(asked) = self.rerouting.as_mut().filter(|rerouting| rerouting.state == rerouting::State::Asked) {
            let since = *asked.asked_at.get_or_insert(now);
            if now - since > wiring::ASKED_SECONDS {
                self.rerouting = None;
            } else {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(wiring::ASKED_SECONDS));
            }
        }
        if let Some(asked) = self.wiring.as_mut().filter(|wiring| wiring.state == wiring::State::Asked) {
            let since = *asked.asked_at.get_or_insert(now);
            if now - since > wiring::ASKED_SECONDS {
                self.wiring = None;
            } else {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(wiring::ASKED_SECONDS));
            }
        }
        // The retract of an old wire starts at the first frame of the new drawing.
        if let Some(retract) = self.retract.as_mut().filter(|retract| retract.drawn) {
            let since = *retract.since.get_or_insert(now);
            if now - since > wiring::RETRACT_SECONDS {
                self.retract = None;
            }
            ctx.request_repaint();
        }
        let moving = self.moving.is_some();
        if moving {
            self.glide = None;
        }
        let offsets = match &mut self.glide {
            _ if moving => self.moving.as_ref().map(|moving| moving.offsets(drawing.placed.len())),
            Some(glide) => {
                let done = glide.advance(now);
                ctx.request_repaint();
                if done {
                    self.glide = None;
                    None
                } else {
                    self.glide.as_ref().map(Glide::offsets)
                }
            }
            None => None,
        };
        let zoom = self.viewport.zoom;
        let seen = Rect::from_min_max(self.viewport.origin.to_pos2(), (self.viewport.origin + rect.size() / zoom).to_pos2());
        // While nodes glide, those whose way crosses the viewport too.
        let mut groups = self.grid.seen(seen);
        if offsets.is_some()
            && let Some(glide) = &self.glide
        {
            groups.extend(self.grid.gliding(glide.from(), seen));
            groups.sort_unstable();
            groups.dedup();
        }
        // What is under the pointer: a node, and a wire within its reach; no wire while nodes glide, as wires do not
        // show.
        let at = response.hover_pos().map(|at| unit_of(&self.viewport, at));
        let hovered = at.and_then(|unit| select::node_at(&drawing, &self.index, unit, &visible_arm));
        let reach = (5.0 / zoom).max(WIRE_REACH);
        let wire_under = at.filter(|_| offsets.is_none()).and_then(|unit| select::wire_at(&drawing, &groups, unit, reach, &visible_arm));
        // While nodes move, what moves shows wherever it started.
        if let Some(moving) = &self.moving {
            groups.extend(moving.preview.wires.iter().map(|(group, _)| *group));
            for &placed in &moving.preview.carried {
                groups.extend_from_slice(self.grid.groups_of(placed));
            }
            groups.extend(
                drawing
                    .groups
                    .iter()
                    .enumerate()
                    .filter(|(_, group)| group.wire.as_ref().is_some_and(|wire| moving.preview.bodies.contains(&wire.body)))
                    .map(|(index, _)| index),
            );
            groups.sort_unstable();
            groups.dedup();
        }
        let mark_fixed = shown.mark_fixed;
        let grip = at.filter(|_| offsets.is_none()).and_then(|unit| grip_under(&drawing, &self.index, &groups, unit, zoom, &visible_arm));
        Frame { rect, drawing, theme, ctx, now, arms, values, offsets, moving, mark_fixed, groups, at, hovered, wire_under, grip }
    }

    /// The input of a frame: the tabs, the selectors and their fields, the selection by the pointer and the keys.
    fn input(&mut self, ui: &mut Ui, response: &Response, frame: &Frame, shown: &Shown, input: &mut Input) {
        input.intents.append(&mut self.queued);
        let (ctx, drawing, rect) = (&frame.ctx, &frame.drawing, frame.rect);
        let zoom = self.viewport.zoom;
        let origin = self.viewport.origin;
        let to_screen = |unit: Vec2| rect.min + (unit - origin) * zoom;
        let unit_of = |at: Pos2| (origin + (at - rect.min) / zoom).to_pos2();
        let visible = |within: Option<usize>| frame.visible(within);
        let coarse = zoom < style::factor(style::coarse_zoom());
        let pointer = response.interact_pointer_pos().filter(|_| response.clicked());
        let clicked_unit = pointer.map(unit_of);
        self.end_run_when_due(frame, input);
        self.fixed_menu(response, frame, shown, input);
        // A click on a tab shows its arm; on the title of a selector, opens the list of its arms. A double-click on a tab
        // opens the field of its patterns (notation 2.6).
        let mut opened = false;
        let double = response.interact_pointer_pos().filter(|_| response.double_clicked()).map(unit_of);
        let mut patterns = None;
        for found in &drawing.tabs {
            let structure_shown = drawing.arms.get(found.arm).is_some_and(|arm| visible(arm.within));
            let with_its_arm = found.when.is_none_or(|when| frame.arms.get(when).copied().unwrap_or(false));
            if !structure_shown || !with_its_arm {
                continue;
            }
            let unit = Rect::from_min_size(
                Pos2::new(found.x as f32 / 100.0, found.y as f32 / 100.0) + frame.offset_of(Some(found.structure)),
                Vec2::new(found.width as f32, found.height as f32) / 100.0,
            );
            if found.kind == TabKind::Tab
                && double.is_some_and(|at| unit.contains(at))
                && let Some(arm) = drawing.arms.get(found.arm)
                && let Some(node) = drawing.placed.get(found.structure)
                && let Some(body) = node.holds.get(arm.arm).and_then(|&body| drawing.bodies.get(body))
                && !node.name.is_empty()
            {
                patterns = Some((found.structure, FieldKind::Patterns { structure: node.name.clone(), arm: body.arm.clone() }, body.arm.clone()));
            }
            if clicked_unit.is_some_and(|at| unit.contains(at))
                && let Some(arm) = drawing.arms.get(found.arm)
            {
                if found.kind == TabKind::Name {
                    self.list = Some((found.structure, Pos2::new(unit.min.x, unit.max.y)));
                    opened = true;
                } else {
                    input.tab = Some((arm.structure.clone(), arm.arm));
                }
            }
        }
        // The list of the arms of a selector: one line per arm, the one shown selected; a choice, or a click elsewhere,
        // closes it.
        if let Some((structure, anchor)) = self.list {
            let corner = to_screen(anchor.to_vec2() + frame.offset_of(Some(structure)));
            let entries: Vec<(&str, usize, bool)> = drawing
                .tabs
                .iter()
                .filter(|found| found.kind == TabKind::Name && found.structure == structure)
                .map(|found| (found.caption.as_str(), found.arm, frame.arms.get(found.arm).copied().unwrap_or(false)))
                .collect();
            let mut chosen = None;
            let area = egui::Area::new(egui::Id::unique("labcraft-arms-list")).order(egui::Order::Foreground).fixed_pos(corner).show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                        for &(caption, arm, is_shown) in &entries {
                            if ui.selectable_label(is_shown, caption).clicked() {
                                chosen = Some(arm);
                            }
                        }
                    });
                });
            });
            if let Some(arm) = chosen.and_then(|arm| drawing.arms.get(arm)) {
                input.tab = Some((arm.structure.clone(), arm.arm));
                self.list = None;
            } else {
                let clicked_at = ctx.input(|input| input.pointer.any_click().then(|| input.pointer.interact_pos()).flatten());
                let elsewhere = clicked_at.is_some_and(|at| !area.response.rect.contains(at));
                if entries.is_empty() || (elsewhere && !opened) {
                    self.list = None;
                }
            }
        }
        // A click on a selector: a step back or forward, or the number, which opens a field to type one.
        let shown_values = self.values.as_ref().filter(|_| frame.values.is_some() && frame.offsets.is_none());
        let mut consumed = input.tab.is_some() || opened;
        let at_selector = clicked_unit.zip(shown_values.filter(|_| !coarse && !consumed)).and_then(|(unit, found)| {
            let selector = found.selector_at(unit, &visible)?;
            let shown = self.shown_of(selector);
            let inside = |(left, top, right, bottom): (i64, i64, i64, i64)| {
                Rect::from_min_max(Pos2::new(left as f32, top as f32) / 100.0, Pos2::new(right as f32, bottom as f32) / 100.0).contains(unit)
            };
            Some((selector.clone(), shown, inside(selector.back), inside(selector.forward)))
        });
        if let Some((selector, shown, back, forward)) = at_selector {
            consumed = true;
            if back {
                input.iteration = shown.checked_sub(1).map(|back| (selector.key.clone(), back));
            } else if forward {
                input.iteration = Some(shown + 1).filter(|next| *next < selector.count).map(|next| (selector.key.clone(), next));
            } else {
                let (left, _, _, bottom) = selector.number;
                let at = Pos2::new(left as f32, bottom as f32) / 100.0;
                let hint = (shown + 1).to_string();
                self.typing = Some(Typing { key: selector.key.clone(), count: selector.count, text: String::new(), hint, at, focused: false });
            }
        }
        if let Some((placed, kind, text)) = patterns {
            consumed |= self.edit_part(placed, kind, text);
        }
        // A click on the `+` after the tabs of a structure: "Add arm…" (notation 2.6).
        let tab_shown = |tab: &labcraft_diagram::Tab| frame.tab_shown(tab);
        let plus = clicked_unit.filter(|_| !consumed).and_then(|at| {
            parts::plus_tabs(drawing, &|placed| frame.offset_of(Some(placed)), &tab_shown).into_iter().find(|(_, area)| area.contains(at))
        });
        if let Some((placed, _)) = plus
            && let Some(node) = drawing.placed.get(placed)
        {
            consumed |= self.edit_part(placed, FieldKind::AddArm { structure: node.name.clone() }, String::new());
        }
        // A double-click on what a field in place edits opens it (notation 2.6).
        if response.double_clicked()
            && !consumed
            && let Some(at) = response.interact_pointer_pos()
            && self.field_at(frame, unit_of(at))
        {
            consumed = true;
        }
        self.select_by_pointer(response, frame, shown, clicked_unit.filter(|_| !consumed), input);
        self.select_by_keys(ui, response, frame, input);
        // The number typed into a selector: Enter chooses that iteration, from 1 as the selector counts; Escape, or a click
        // elsewhere, closes the field.
        if let Some(typing) = &mut self.typing {
            let corner = to_screen(typing.at.to_vec2());
            let mut done = false;
            egui::Area::new(egui::Id::unique("labcraft-iteration-field")).order(egui::Order::Foreground).fixed_pos(corner).show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    let field = ui.add(egui::TextEdit::singleline(&mut typing.text).desired_width(64.0).hint_text(typing.hint.as_str()));
                    if !typing.focused {
                        field.request_focus();
                        typing.focused = true;
                    }
                    if field.lost_focus() {
                        if ui.input(|input| input.key_pressed(egui::Key::Enter))
                            && let Ok(number) = typing.text.trim().replace(',', "").parse::<u64>()
                        {
                            input.iteration = Some((typing.key.clone(), number.clamp(1, typing.count.max(1)) - 1));
                        }
                        done = true;
                    }
                });
            });
            if done {
                self.typing = None;
            }
        }
        self.show_field(frame, input);
        self.pinned_values(frame, input);
        self.palette_drop(response, frame, shown, input);
        // The tooltip of a terminal under the pointer: its name, its exact type, the default of an input that no argument
        // wires (notation 3.2, 4.1).
        input.hint = frame.at.filter(|_| self.wiring.is_none()).and_then(|unit| {
            drawing.hints.iter().rev().filter(|hint| visible(hint.within)).find_map(|hint| {
                let corner = Pos2::new(hint.x as f32 / 100.0, hint.y as f32 / 100.0) + frame.offset_of(hint.placed);
                Rect::from_min_size(corner, Vec2::new(hint.width as f32, hint.height as f32) / 100.0).contains(unit).then(|| hint.text.clone())
            })
        });
        // The pin of a fixed node says the entry it comes from (notation 2.4, 2.5).
        if input.hint.is_none() && shown.mark_fixed && !coarse {
            input.hint = frame.at.and_then(|unit| pins::under(drawing, frame, &self.grid, unit)).map(|placed| pins::tooltip(drawing, placed));
        }
        if input.hint.is_none() && shown.mark_fixed && !coarse && frame.offsets.is_none() {
            input.hint = frame.at.and_then(|unit| pins::terminal_under(drawing, unit)).map(|port| pins::terminal_tooltip(drawing, port));
        }
        // And the corner of a frame of fixed size.
        if input.hint.is_none() && shown.mark_fixed && !coarse && frame.offsets.is_none() {
            let fixed = sizing::fixed(drawing, &|placed| self.grid.shown(placed, &|within| frame.visible(within)));
            input.hint = frame.at.and_then(|unit| sizing::corner_under(&fixed, unit)).map(|target| sizing::tooltip(drawing, target));
        }
    }

    /// A function dragged from the panel `Palette` (notation 2.5): its ghost follows the pointer, outlined dashed in
    /// `select.stroke`, on the grid; over a wire it can take, that wire shows in `select.stroke`, and the drop inserts the
    /// node into it; elsewhere the node falls where the ghost is.
    fn palette_drop(&mut self, response: &Response, frame: &Frame, shown: &Shown, input: &mut Input) {
        let Some(dragged) = egui::DragAndDrop::payload::<Dragged>(&frame.ctx) else { return };
        let rect = frame.rect;
        let Some(pointer) = frame.ctx.pointer_latest_pos().filter(|at| rect.contains(*at)) else { return };
        let (zoom, origin) = (self.viewport.zoom, self.viewport.origin);
        let unit = (origin + (pointer - rect.min) / zoom).to_pos2();
        let reach = (5.0 / zoom).max(WIRE_REACH);
        let visible = |within: Option<usize>| frame.visible(within);
        let wire_under = select::wire_at(&frame.drawing, &frame.groups, unit, reach, &visible);
        let into = self.dragged_over(&dragged, unit, wire_under, shown.into_wire);
        // The frame under the pointer, where the ghost is drawn and the node falls.
        let (within, at) = self.frame_under(&frame.drawing, &visible, unit);
        if response.dnd_release_payload::<Dragged>().is_some() {
            let call = dragged.catalog.entries.get(dragged.entry).map(|entry| entry.call.clone()).unwrap_or_default();
            input.intents.push(Intent::Drop { call, at, within, into: into.map(|(_, name)| name) });
            return;
        }
        let to_screen = |unit: Vec2| rect.min + (unit - origin) * zoom;
        let painter =
            frame.ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("labcraft-palette-ghost"))).with_clip_rect(rect);
        let stroke = egui::Stroke::new(2.0, frame.theme.color("select.stroke"));
        if let Some(wire) = into.and_then(|(group, _)| frame.drawing.groups.get(group)).and_then(|group| group.wire.as_ref()) {
            for (_, path) in &wire.readers {
                let points: Vec<Pos2> = path.iter().map(|&(x, y)| to_screen(Vec2::new(x as f32, y as f32) / 100.0)).collect();
                painter.add(egui::Shape::line(points, stroke));
            }
        }
        // The ghost: a block of the size of a small node, on the grid, with what it calls.
        let corner = to_screen(
            Vec2::new(at.0 as f32, at.1 as f32)
                + frame.drawing.bodies.first().map_or(Vec2::ZERO, |body| Vec2::new(body.origin.0 as f32, body.origin.1 as f32) / 100.0),
        );
        let ghost = Rect::from_min_size(corner, Vec2::new(GHOST.0, GHOST.1) * zoom);
        painter.add(egui::Shape::dashed_line(
            &[ghost.left_top(), ghost.right_top(), ghost.right_bottom(), ghost.left_bottom(), ghost.left_top()],
            stroke,
            6.0,
            4.0,
        ));
        let call = dragged.catalog.entries.get(dragged.entry).map(|entry| entry.call.as_str()).unwrap_or_default();
        painter.text(ghost.center(), egui::Align2::CENTER_CENTER, call, egui::FontId::proportional((12.0 * zoom).clamp(9.0, 18.0)), stroke.color);
    }

    /// A run of presses of Ctrl and an arrow ends a second after the last press, or at a press of the pointer or of
    /// another key (notation 2.6): then it is the move it asks for.
    fn end_run_when_due(&mut self, frame: &Frame, input: &mut Input) {
        let last = match (self.moving.as_ref().map(|moving| moving.state), self.sliding.as_ref().map(|sliding| sliding.state)) {
            (Some(moving::State::Keys { last }), _) | (_, Some(edge::State::Keys { last })) => last,
            _ => return,
        };
        let other_key = frame.ctx.input(|input| {
            input.events.iter().any(|event| {
                matches!(event, egui::Event::Key { key, pressed: true, modifiers, .. } if !(is_arrow(*key) && modifiers.command && !modifiers.alt))
            })
        });
        let pressed = frame.ctx.input(|input| input.pointer.any_pressed());
        let left = moving::RUN_SECONDS - (frame.now - last);
        if other_key || pressed || left <= 0.0 {
            input.intents.extend(self.end_run());
        } else {
            frame.ctx.request_repaint_after(std::time::Duration::from_secs_f64(left));
        }
    }

    /// A right-click on a fixed node opens its menu: "Remove the fixed position"; on a wire routed by hand, "Arrange this
    /// wire" (notation 2.4).
    fn fixed_menu(&mut self, response: &Response, frame: &Frame, shown: &Shown, input: &mut Input) {
        if response.secondary_clicked() {
            let unit_of = |at: Pos2| (self.viewport.origin + (at - frame.rect.min) / self.viewport.zoom).to_pos2();
            let visible = |within: Option<usize>| frame.visible(within);
            let reach = (5.0 / self.viewport.zoom).max(WIRE_REACH);
            // A wire routed by hand within reach has its menu, over a node too; otherwise the node under the pointer.
            self.wire_context = frame.ctx.input(|input| input.pointer.interact_pos()).map(unit_of).and_then(|at| {
                select::wire_at(&frame.drawing, &frame.groups, at, reach, &visible).filter(|&group| !routed(&frame.drawing, group).is_empty())
            });
            self.context = frame.hovered.filter(|_| self.wire_context.is_none());
            // The tab and the terminal under the pointer, for the parts of a structure (notation 2.6).
            let at = frame.ctx.input(|input| input.pointer.interact_pos()).map(unit_of);
            self.context_at = at;
            let port = at.and_then(|at| port_at(&frame.drawing, at, (6.0 / self.viewport.zoom).max(5.0), &visible));
            self.terminal_context = port
                .filter(|_| self.wire_context.is_none() && self.context.is_none())
                .filter(|&port| frame.drawing.ports.get(port).and_then(signature::frame_terminal).is_some());
            // The edge of the frame of the function: its resize grip, or its left or right edge (notation 2.4).
            let on_edge = matches!(frame.grip, Some((None, _)))
                || at.is_some_and(|at| signature::function_edge(&frame.drawing, at, (6.0 / self.viewport.zoom).max(4.0)).is_some());
            self.frame_context = self.wire_context.is_none() && self.context.is_none() && self.terminal_context.is_none() && on_edge;
            self.context_arm = at.and_then(|at| {
                frame.drawing.tabs.iter().find_map(|tab| {
                    let corner = Pos2::new(tab.x as f32, tab.y as f32) / 100.0 + frame.offset_of(Some(tab.structure));
                    let area = Rect::from_min_size(corner, Vec2::new(tab.width as f32, tab.height as f32) / 100.0);
                    (tab.kind == TabKind::Tab && area.contains(at)).then(|| frame.drawing.arms.get(tab.arm).map(|arm| arm.arm)).flatten()
                })
            });
            self.context_port = at.and_then(|at| port_at(&frame.drawing, at, (6.0 / self.viewport.zoom).max(5.0), &visible));
        }
        if let Some(group) = self.wire_context {
            let wires = routed(&frame.drawing, group);
            let mut asked = false;
            response.context_menu(|ui| {
                if ui.button("Arrange this wire").clicked() {
                    asked = true;
                    ui.close();
                }
            });
            if asked {
                input.intents.push(Intent::Arrange(Selection { wires, ..Selection::default() }));
                self.wire_context = None;
            }
            return;
        }
        if let Some(port) = self.terminal_context {
            self.terminal_menu(response, frame, port, input);
            return;
        }
        if self.frame_context {
            let sized = frame.drawing.bodies.first().is_some_and(|body| body.size.is_some());
            let (mut asked, mut adding) = (false, None);
            response.context_menu(|ui| {
                if ui.button("Add input").clicked() {
                    adding = Some(true);
                    ui.close();
                }
                if ui.button("Add output").clicked() {
                    adding = Some(false);
                    ui.close();
                }
                if sized && ui.button("Remove the fixed size").clicked() {
                    asked = true;
                    ui.close();
                }
            });
            if asked {
                input.intents.push(Intent::Resize { structure: None, size: None });
                self.frame_context = false;
            }
            if let Some(side) = adding {
                self.add_terminal(&frame.drawing, side);
                self.frame_context = false;
            }
            return;
        }
        let Some(placed) = self.context else { return };
        let (name, fixed) =
            frame.drawing.placed.get(placed).map(|found| (found.name.clone(), found.at.is_some() && !found.name.is_empty())).unwrap_or_default();
        let sized = frame.drawing.placed.get(placed).is_some_and(|found| found.size.is_some() && !found.name.is_empty());
        let mut unsize = false;
        // The block of a function of the project: its terminals can go back to their defaults (notation 2.4).
        let callee = frame.drawing.placed.get(placed).map(|found| found.callee.clone()).unwrap_or_default();
        let mut reset = false;
        let (mut arrange, mut label, mut surround) = (false, false, None);
        let shown_arm = |index: usize| frame.arms.get(index).copied().unwrap_or(false);
        let (context_arm, context_port) = (self.context_arm, self.context_port);
        let mut part = None;
        let mut make = false;
        response.context_menu(|ui| {
            if ui.button("Label…").clicked() {
                label = true;
                ui.close();
            }
            if fixed && ui.button("Remove the fixed position").clicked() {
                arrange = true;
                ui.close();
            }
            if sized && ui.button("Remove the fixed size").clicked() {
                unsize = true;
                ui.close();
            }
            if !callee.is_empty() && ui.button("Reset the terminals of this block").clicked() {
                reset = true;
                ui.close();
            }
            ui.menu_button("Surround with", |ui| {
                for kind in STRUCTURES {
                    if ui.button(kind).clicked() {
                        surround = Some(kind);
                        ui.close();
                    }
                }
            });
            // Built while the menu shows, not on each frame of the canvas.
            if let Some(parts) = parts::PartsMenu::of(&frame.drawing, placed, context_arm, context_port, &shown_arm) {
                part = parts.ui(ui);
            }
            // A terminal of the node that holds nothing (notation 2.4).
            if let Some((made, ty)) = context_port
                .filter(|&port| frame.drawing.ports.get(port).is_some_and(|found| found.placed == Some(placed)))
                .and_then(|port| signature::makeable(&frame.drawing, port))
            {
                let text = if made { "Make an input of the function" } else { "Make an output of the function" };
                let button = ui.add_enabled(ty.is_some(), egui::Button::new(text));
                let button = if ty.is_none() {
                    button.on_disabled_hover_text("Its type is not known: add the terminal from the edge of the frame")
                } else {
                    button
                };
                if button.clicked() {
                    make = true;
                    ui.close();
                }
            }
        });
        if make && let Some(intent) = context_port.and_then(|port| signature::make(&frame.drawing, port)) {
            input.intents.push(intent);
            self.context = None;
        }
        match part {
            Some(parts::Chosen::Field(kind, text)) => {
                self.edit_part(placed, kind, text);
                self.context = None;
            }
            Some(parts::Chosen::Intent(intent)) => {
                input.intents.push(intent);
                self.context = None;
            }
            None => {}
        }
        if let Some(kind) = surround {
            // The node right-clicked, or the selection when it holds that node.
            let nodes = if name.is_empty() || shown.selection.nodes.contains(&name) { shown.selection.nodes.clone() } else { vec![name.clone()] };
            input.intents.push(Intent::Surround { kind, nodes });
            self.context = None;
        }
        if unsize {
            input.intents.push(Intent::Resize { structure: Some(name.clone()), size: None });
            self.context = None;
        }
        if reset {
            input.intents.push(Intent::ResetBlock { file: callee });
            self.context = None;
        }
        if arrange {
            input.intents.push(Intent::Arrange(Selection { nodes: vec![name], ..Selection::default() }));
            self.context = None;
        }
        if label {
            self.edit_label(placed);
            self.context = None;
        }
    }

    /// The menu of a terminal of the frame of the function (notation 2.4): "Rename", "Type…", "Default…" for an input,
    /// "Remove", and "Add input", "Add output".
    fn terminal_menu(&mut self, response: &Response, frame: &Frame, port: usize, input: &mut Input) {
        let Some(found) = frame.drawing.ports.get(port) else { return };
        let Some(terminal) = signature::frame_terminal(found) else { return };
        let is_input = !found.input;
        let fixed = found.fixed;
        let caption = found.caption.clone();
        let mut chosen: Option<&'static str> = None;
        response.context_menu(|ui| {
            if fixed && ui.button("Remove the fixed position").clicked() {
                chosen = Some("unfix");
                ui.close();
            }
            for (entry, shown) in [("rename", true), ("type", true), ("default", is_input), ("remove", true)] {
                let label = match entry {
                    "rename" => "Rename",
                    "type" => "Type…",
                    "default" => "Default…",
                    _ => "Remove",
                };
                if shown && ui.button(label).clicked() {
                    chosen = Some(entry);
                    ui.close();
                }
            }
            ui.separator();
            if ui.button("Add input").clicked() {
                chosen = Some("add input");
                ui.close();
            }
            if ui.button("Add output").clicked() {
                chosen = Some("add output");
                ui.close();
            }
        });
        let Some(chosen) = chosen else { return };
        self.terminal_context = None;
        let at = signature::field_by(&frame.drawing, port);
        match chosen {
            "rename" => {
                self.edit_terminal(FieldKind::TerminalName { terminal }, caption, at);
            }
            "type" => input.intents.push(Intent::PickType { terminal }),
            "default" => {
                let default = signature::default_at(&frame.drawing, port);
                if signature::default_is_cut(&default) {
                    input.intents.push(Intent::Say(signature::CUT_DEFAULT.to_owned()));
                } else {
                    self.edit_terminal(FieldKind::TerminalDefault { terminal }, default, at);
                }
            }
            "remove" => input.intents.push(Intent::RemoveTerminal { terminal }),
            "unfix" => input.intents.push(Intent::Arrange(Selection { terminals: vec![terminal], ..Selection::default() })),
            "add input" => self.add_terminal(&frame.drawing, true),
            _ => self.add_terminal(&frame.drawing, false),
        }
    }

    /// "Add input", "Add output" (notation 2.4): the field of a new terminal, on its edge, where the menu was opened.
    fn add_terminal(&mut self, drawing: &Drawing, input: bool) {
        let y = self.context_at.map_or(0.0, |at| at.y);
        let at = signature::edge_field_at(drawing, input, y);
        self.edit_terminal(FieldKind::AddTerminal { input }, String::new(), at);
    }

    /// A click selects what it is on, Shift or Ctrl with it adds or takes out; a drag on the empty canvas draws a
    /// rectangle that selects the nodes it touches in its frame (notation 2.6).
    fn select_by_pointer(&mut self, response: &Response, frame: &Frame, shown: &Shown, clicked: Option<Pos2>, input: &mut Input) {
        let (ctx, drawing, rect) = (&frame.ctx, &frame.drawing, frame.rect);
        let (zoom, origin) = (self.viewport.zoom, self.viewport.origin);
        let unit_of = |at: Pos2| (origin + (at - rect.min) / zoom).to_pos2();
        let visible = |within: Option<usize>| frame.visible(within);
        let modifiers = ctx.input(|input| input.modifiers);
        // A press takes the keys of the canvas, for those of notation 2.6.
        if response.clicked() || response.drag_started() {
            response.request_focus();
        }
        let reach = (5.0 / zoom).max(WIRE_REACH);
        let rules = Rules { alt: modifiers.alt, ..self.rules };
        if response.drag_started_by(PointerButton::Primary) && frame.offsets.is_none() {
            let from = ctx.input(|input| input.pointer.press_origin()).map(unit_of);
            // A drag from a terminal draws a wire; Ctrl with it takes the end of its wire (notation 2.6). A terminal of the
            // frame of the function moves along its edge (notation 2.4). Alt with it moves a terminal of a block, from
            // step 6 of M1.
            let port = from.and_then(|from| port_at(drawing, from, (6.0 / zoom).max(5.0), &visible));
            if let (Some(from), Some(port)) = (from, port) {
                let of_the_function = drawing.ports.get(port).is_some_and(|found| found.frame && found.placed.is_none());
                if of_the_function {
                    self.sliding = edge::Sliding::new(drawing, port, edge::State::Dragging, from.y);
                    self.port_focus = None;
                    self.drag = Some(Drag::Held);
                    return;
                }
                // The same setting as for a wire (notation 2.6): Shift and Alt when Alt alone is the desktop's.
                if modifiers.alt && (modifiers.shift || !shown.route_with_shift_alt) {
                    match blockdrag::BlockDrag::new(drawing, port, from) {
                        Ok(moving) => self.block_drag = Some(moving),
                        Err(why) if !why.is_empty() => input.intents.push(Intent::Say(why)),
                        Err(_) => {}
                    }
                    self.port_focus = None;
                    self.drag = Some(Drag::Held);
                    return;
                }
                self.wiring =
                    if modifiers.command { Wiring::taking(drawing, port, from) } else { Wiring::new(drawing, port, from, wiring::State::Dragging) };
                if self.wiring.is_some() {
                    self.port_focus = None;
                    self.drag = Some(Drag::Held);
                }
            }
        }
        // A drag on the right edge, the bottom edge or the corner of a structure or of the frame of the function resizes it
        // (notation 2.6).
        if response.drag_started_by(PointerButton::Primary)
            && frame.offsets.is_none()
            && self.wiring.is_none()
            && self.drag.is_none()
            && let Some(from) = ctx.input(|input| input.pointer.press_origin()).map(unit_of)
            && let Some((target, grip)) = grip_under(drawing, &self.index, &frame.groups, from, zoom, &visible)
            && let Some(resizing) = Resizing::new(drawing, target, grip, from)
        {
            self.resizing = Some(resizing);
            self.port_focus = None;
            self.drag = Some(Drag::Held);
        }
        if let Some(sliding) = self.sliding.as_mut().filter(|sliding| sliding.state == edge::State::Dragging) {
            if let Some(at) = response.interact_pointer_pos() {
                sliding.set_at(unit_of(at).y);
                ctx.request_repaint();
            }
            if response.drag_stopped_by(PointerButton::Primary) {
                if sliding.changed() {
                    sliding.state = edge::State::Asked;
                    input.intents.push(Intent::MoveTerminal { terminal: sliding.terminal.clone(), y: sliding.y });
                } else {
                    self.sliding = None;
                }
                self.drag = None;
            }
            return;
        }
        if let Some(moving) = self.block_drag.as_mut() {
            if let Some(at) = response.interact_pointer_pos() {
                moving.set_at(drawing, unit_of(at));
                ctx.request_repaint();
            }
            if response.drag_stopped_by(PointerButton::Primary) {
                if moving.changed() {
                    let (side, place) = moving.to;
                    input.intents.push(Intent::BlockTerminal {
                        file: moving.file.clone(),
                        terminal: moving.terminal.clone(),
                        side: side.word(),
                        place,
                    });
                }
                self.block_drag = None;
                self.drag = None;
            }
            return;
        }
        if let Some(resizing) = self.resizing.as_mut().filter(|resizing| resizing.state == sizing::State::Dragging) {
            if let Some(at) = response.interact_pointer_pos() {
                resizing.set_at(unit_of(at));
                ctx.request_repaint();
            }
            if response.drag_stopped_by(PointerButton::Primary) {
                let structure = match resizing.target {
                    None => Some(None),
                    Some(placed) => drawing.placed.get(placed).map(|found| found.name.clone()).filter(|name| !name.is_empty()).map(Some),
                };
                match structure.filter(|_| resizing.changed()) {
                    Some(structure) => {
                        resizing.state = sizing::State::Asked;
                        input.intents.push(Intent::Resize { structure, size: Some(resizing.size) });
                    }
                    None => self.resizing = None,
                }
                self.drag = None;
            }
            return;
        }
        // A drag on a wire: with the modifier of the setting, its segment moves (`wire.route`); without, a branch starts at
        // a junction on it (notation 2.6).
        if response.drag_started_by(PointerButton::Primary)
            && frame.offsets.is_none()
            && self.wiring.is_none()
            && self.drag.is_none()
            && let Some(from) = ctx.input(|input| input.pointer.press_origin()).map(unit_of)
            && let Some(Hit::Wire(group)) = select::hit(drawing, &self.index, &frame.groups, from, reach, &visible)
        {
            let reroute = modifiers.alt && (modifiers.shift || !shown.route_with_shift_alt);
            if reroute {
                self.rerouting = Rerouting::grab(drawing, group, from, reach);
            } else {
                self.wiring = Wiring::branch(drawing, group, from, (5.0 / zoom).max(BRANCH_REACH));
            }
            if self.rerouting.is_some() || self.wiring.is_some() {
                self.port_focus = None;
                self.drag = Some(Drag::Held);
            }
        }
        if let Some(rerouting) = self.rerouting.as_mut().filter(|rerouting| rerouting.state == rerouting::State::Dragging) {
            if let Some(at) = response.interact_pointer_pos() {
                rerouting.set_at(drawing, unit_of(at));
                ctx.request_repaint();
            }
            if response.drag_stopped_by(PointerButton::Primary) {
                let routes: Vec<(String, Vec<(i64, i64)>)> = rerouting.routes(drawing).into_iter().map(|(name, _, via)| (name, via)).collect();
                if routes.is_empty() {
                    self.rerouting = None;
                } else {
                    rerouting.state = rerouting::State::Asked;
                    input.intents.push(Intent::Route(routes));
                }
                self.drag = None;
            }
            return;
        }
        if let Some(wiring) = self.wiring.as_mut().filter(|wiring| wiring.state == wiring::State::Dragging)
            && let Some(at) = response.interact_pointer_pos()
        {
            wiring.at = unit_of(at);
            wiring.find_over(drawing, (10.0 / zoom).max(wiring::SNAP), &visible);
            ctx.request_repaint();
        }
        if response.drag_stopped_by(PointerButton::Primary) && self.wiring.as_ref().is_some_and(|wiring| wiring.state == wiring::State::Dragging) {
            self.drop_wire(drawing, rules, input, &visible);
            self.drag = None;
            return;
        }
        if self.wiring.as_ref().is_some_and(|wiring| wiring.state == wiring::State::Dragging) {
            return;
        }
        if response.drag_started_by(PointerButton::Primary) {
            let from = ctx.input(|input| input.pointer.press_origin()).map(unit_of);
            let hit = from.and_then(|from| select::hit(drawing, &self.index, &frame.groups, from, reach, &visible));
            // A drag on a node moves it, or the selection that holds it in its frame (notation 2.6).
            if let (Some(from), Some(Hit::Node(placed))) = (from, hit)
                && frame.offsets.is_none()
                && drawing.placed.get(placed).is_some_and(|found| !found.name.is_empty())
            {
                let nodes = self.grabbed(drawing, placed, input);
                self.moving = Some(Moving::new(drawing, nodes, moving::State::Dragging { from }));
            }
            self.drag = match (from, hit) {
                (Some(_), Some(_)) => Some(Drag::Held),
                // On the empty canvas: with the setting, a drag pans and Shift with it selects, only Ctrl then adding.
                (Some(_), None) if shown.drag_pans && !modifiers.shift => Some(Drag::Pan),
                (Some(from), None) if frame.offsets.is_none() => {
                    Some(Drag::Rectangle { from, to: from, adding: modifiers.command || (modifiers.shift && !shown.drag_pans) })
                }
                _ => None,
            };
        }
        if let Some(Drag::Rectangle { to, .. }) = &mut self.drag
            && let Some(at) = response.interact_pointer_pos()
        {
            *to = unit_of(at);
        }
        if let Some(moving) = &mut self.moving
            && let moving::State::Dragging { from } = moving.state
            && let Some(at) = response.interact_pointer_pos()
        {
            moving.set_by(drawing, moving::snapped(unit_of(at) - from));
            ctx.request_repaint();
        }
        if response.drag_stopped_by(PointerButton::Primary)
            && let Some(moving) = self.moving.as_mut().filter(|moving| matches!(moving.state, moving::State::Dragging { .. }))
        {
            moving.state = moving::State::Asked;
            let (nodes, by) = (moving.nodes.clone(), moving.by);
            let asked = self.move_intent(drawing, &nodes, by);
            input.intents.extend(asked);
        }
        if response.drag_stopped_by(PointerButton::Primary) {
            if let Some(Drag::Rectangle { from, to, adding }) = self.drag.take() {
                let mut found = if adding { self.asked_found.clone().unwrap_or_else(|| self.found.clone()) } else { Found::default() };
                found.nodes.extend(select::rectangle(drawing, &self.index, from, to, &visible));
                if !adding {
                    self.pinned = None;
                }
                input.intents.push(Intent::Select(self.index.name(drawing, &found)));
                self.asked_found = Some(found);
            }
            self.drag = None;
        }
        let Some(at) = clicked else { return };
        let adding = modifiers.shift || modifiers.command;
        let hit = select::hit(drawing, &self.index, &frame.groups, at, reach, &visible);
        let base = self.asked_found.clone().unwrap_or_else(|| self.found.clone());
        let found = select::click(&base, hit, adding);
        self.pinned = match hit {
            Some(Hit::Wire(group)) if select::holds(&found, Hit::Wire(group)) => Some((group, at)),
            _ if adding => self.pinned.filter(|(group, _)| found.groups.contains(group)),
            _ => None,
        };
        input.intents.push(Intent::Select(self.index.name(drawing, &found)));
        self.asked_found = Some(found);
    }

    /// The drop of the wire being drawn: on a terminal that takes it, the wire; on one that refuses it, nothing, and why;
    /// on the empty canvas, quick insertion (notation 2.6).
    fn drop_wire(&mut self, drawing: &Drawing, rules: Rules, input: &mut Input, visible: &dyn Fn(Option<usize>) -> bool) {
        let Some(mut wiring) = self.wiring.take() else { return };
        let Some(over) = wiring.over else {
            // On the edge of a loop: a `collect` (a `last` with Alt), or a clause of a `for` (notation 2.6).
            let reach = (6.0 / self.viewport.zoom).max(4.0);
            // From a terminal of a node that holds nothing, on the edge of the frame of the function of its side: an input
            // or an output of the function (notation 2.4).
            if wiring.taken == wiring::Taken::Nothing
                && wiring.branch.is_none()
                && let Some((input_side, _)) = signature::makeable(drawing, wiring.from)
                && signature::function_edge(drawing, wiring.at, reach) == Some(input_side)
            {
                match signature::make(drawing, wiring.from) {
                    Some(intent) => input.intents.push(intent),
                    None => input.intents.push(Intent::Say("Its type is not known: add the terminal from the edge of the frame".to_owned())),
                }
                return;
            }
            if wiring.taken == wiring::Taken::Nothing
                && wiring.branch.is_none()
                && let Some(port) = drawing.ports.get(wiring.from)
                && let Some(intent) = parts::edge_drop(drawing, port, wiring.at, reach, rules.alt, visible)
            {
                input.intents.push(intent);
                return;
            }
            // Ctrl and a drag that ends nowhere leaves the wire as it was.
            if wiring.taken == wiring::Taken::Nothing
                && let Some(port) = drawing.ports.get(wiring.from)
                && let Some(wanted) = insertion::Wanted::of_port(drawing, wiring.from)
            {
                let replace = port.input && matches!(port.holds, labcraft_diagram::Holds::Wire | labcraft_diagram::Holds::Constant(_));
                let arm = (!port.arm.is_empty()).then(|| port.arm.clone());
                let source = Source::Terminal { name: port.name.clone(), arm, wanted, replace };
                let screen = self.rect.min + (wiring.at.to_vec2() - self.viewport.origin) * self.viewport.zoom;
                let (within, at) = self.frame_under(drawing, visible, wiring.at);
                input.intents.push(Intent::QuickInsert(Quick { at: Some(at), within, screen, source }));
            }
            return;
        };
        match wiring.drop_on(drawing, over, rules) {
            Some(wiring::Drop::Wire { replace, accept_mismatch, through }) => {
                let Some(intent) = wiring.intent(drawing, over, replace, accept_mismatch, through) else { return };
                // The old wire retracts, visibly (notation 2.4).
                let target = if drawing.ports.get(over).is_some_and(|port| port.input) { Some(over) } else { Some(wiring.from) };
                self.retract = target.filter(|_| replace && !self.reduced_motion).and_then(|target| {
                    drawing.ports.get(target).filter(|port| port.holds == labcraft_diagram::Holds::Wire)?;
                    Retract::of(drawing, target)
                });
                input.intents.push(intent);
                wiring.state = wiring::State::Asked;
                self.wiring = Some(wiring);
            }
            Some(wiring::Drop::Refused(refusal)) => input.intents.push(Intent::Say(refusal.message)),
            None => {}
        }
    }

    /// The nodes that a drag from `placed` moves: the selection in the frame of `placed` when it holds it, otherwise
    /// `placed` alone, which the drag selects (decision of the owner, 2026-11-17).
    fn grabbed(&mut self, drawing: &Drawing, placed: usize, input: &mut Input) -> Vec<usize> {
        let base = self.asked_found.clone().unwrap_or_else(|| self.found.clone());
        let body = drawing.placed.get(placed).map(|found| found.body);
        // A node that no command names cannot be moved: it is left where it is.
        if base.nodes.contains(&placed) {
            return base
                .nodes
                .iter()
                .copied()
                .filter(|&at| drawing.placed.get(at).is_some_and(|found| Some(found.body) == body && !found.name.is_empty()))
                .collect();
        }
        let found = Found::one(Hit::Node(placed));
        self.pinned = None;
        input.intents.push(Intent::Select(self.index.name(drawing, &found)));
        self.asked_found = Some(found);
        vec![placed]
    }

    /// Ctrl and an arrow moves the selection by a step of the grid, by 64 with Shift; the presses that follow one another
    /// on the same nodes are one move (notation 2.6). The nodes are those of the selection in the frame of the first.
    fn nudge(&mut self, drawing: &Drawing, step: (i64, i64), now: f64, input: &mut Input) {
        if self.moving.as_ref().is_some_and(|moving| moving.state == moving::State::Asked) {
            return;
        }
        let base = self.asked_found.clone().unwrap_or_else(|| self.found.clone());
        let movable: Vec<usize> =
            base.nodes.iter().copied().filter(|&at| drawing.placed.get(at).is_some_and(|found| !found.name.is_empty())).collect();
        let Some(body) = movable.first().and_then(|&first| drawing.placed.get(first)).map(|found| found.body) else { return };
        let nodes: Vec<usize> = movable.into_iter().filter(|&at| drawing.placed.get(at).is_some_and(|found| found.body == body)).collect();
        // A run on other nodes ends first, with its move.
        if self.moving.as_ref().is_some_and(|moving| moving.nodes != nodes) {
            input.intents.extend(self.end_run());
            return;
        }
        let moving = match &mut self.moving {
            Some(moving) if moving.nodes == nodes => moving,
            _ => self.moving.insert(Moving::new(drawing, nodes, moving::State::Keys { last: now })),
        };
        moving.state = moving::State::Keys { last: now };
        let by = (moving.by.0 + step.0, moving.by.1 + step.1);
        moving.set_by(drawing, by);
    }

    /// The terminal of the frame of the function that the selection holds alone, by its port.
    fn selected_frame_terminal(&self, drawing: &Drawing) -> Option<usize> {
        let base = self.asked_found.as_ref().unwrap_or(&self.found);
        if !base.nodes.is_empty() || !base.groups.is_empty() || base.terminals.len() != 1 {
            return None;
        }
        let name = base.terminals.first().and_then(|&hint| drawing.hints.get(hint)).and_then(|hint| hint.selects.clone())?;
        drawing.ports.iter().position(|port| signature::frame_terminal(port).as_deref() == Some(name.as_str()))
    }

    /// Ctrl and Up or Down on a terminal of the frame of the function: to the next free place that way, the presses that
    /// follow one another one move (notation 2.4, 2.6).
    fn slide(&mut self, drawing: &Drawing, port: usize, by: i64, now: f64, input: &mut Input) {
        if self.sliding.as_ref().is_some_and(|sliding| sliding.state == edge::State::Asked) {
            return;
        }
        if self.sliding.as_ref().is_some_and(|sliding| sliding.port != port) {
            input.intents.extend(self.end_run());
            self.sliding = None;
        }
        let sliding = match &mut self.sliding {
            Some(sliding) => sliding,
            None => match edge::Sliding::new(drawing, port, edge::State::Keys { last: now }, 0.0) {
                Some(sliding) => self.sliding.insert(sliding),
                None => return,
            },
        };
        sliding.state = edge::State::Keys { last: now };
        sliding.step(by);
    }

    /// With the focus: Tab and Shift+Tab, the arrows, Enter and Escape move the selection, which is the focus (notation
    /// 2.6). Tab past the last one, or Shift+Tab before the first, leaves the canvas.
    fn select_by_keys(&mut self, ui: &mut Ui, response: &Response, frame: &Frame, input: &mut Input) {
        if !response.has_focus() || self.typing.is_some() {
            return;
        }
        let id = response.id;
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(id, egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true });
        });
        let drawing = frame.drawing.clone();
        let drawing = &drawing;
        let visible = |within: Option<usize>| frame.visible(within);
        let asked = input.intents.len();
        self.keys_of_focus(ui, input, drawing, &visible);
        // The canvas scrolls to keep the focus in view (notation 2.6).
        if input.intents.len() > asked
            && let Some(Intent::Select(selection)) = input.intents.last()
            && let Some(area) = select::focus_rect(drawing, &self.index.find(selection))
        {
            let zoom = self.viewport.zoom;
            let seen = Rect::from_min_size(self.viewport.origin.to_pos2(), frame.rect.size() / zoom);
            if !seen.contains_rect(area) {
                self.viewport.origin = (area.center() - seen.size() / 2.0).to_vec2();
            }
        }
    }

    /// The keys of the focus, each a selection asked for.
    fn keys_of_focus(&mut self, ui: &mut Ui, input: &mut Input, drawing: &Drawing, visible: &dyn Fn(Option<usize>) -> bool) {
        if self.wiring_keys(ui, input, drawing, visible) || self.port_keys(ui, input, drawing) {
            return;
        }
        // Exactly these modifiers: Alt or Shift with an arrow is a key of the host (Alt+Left, notation 2.6).
        let pressed = |key: egui::Key, modifiers: egui::Modifiers| {
            ui.input_mut(|input| {
                let at = input.events.iter().position(|event| {
                    matches!(event, egui::Event::Key { key: found, pressed: true, modifiers: with, .. } if *found == key && *with == modifiers)
                });
                at.map(|at| input.events.remove(at)).is_some()
            })
        };
        if pressed(egui::Key::Escape, egui::Modifiers::NONE) {
            // A gesture first, then a list open, then up one level, then nothing.
            if self.wiring.as_ref().is_some_and(|wiring| wiring.state == wiring::State::Dragging) {
                self.wiring = None;
                self.drag = Some(Drag::Held);
            } else if self.rerouting.as_ref().is_some_and(|rerouting| rerouting.state == rerouting::State::Dragging) {
                self.rerouting = None;
                self.drag = Some(Drag::Held);
            } else if self.moving.as_ref().is_some_and(|moving| matches!(moving.state, moving::State::Dragging { .. })) {
                self.moving = None;
                self.drag = Some(Drag::Held);
            } else if self.resizing.as_ref().is_some_and(|resizing| resizing.state == sizing::State::Dragging) {
                self.resizing = None;
                self.drag = Some(Drag::Held);
            } else if self.sliding.as_ref().is_some_and(|sliding| sliding.state == edge::State::Dragging) {
                self.sliding = None;
                self.drag = Some(Drag::Held);
            } else if self.block_drag.is_some() {
                self.block_drag = None;
                self.drag = Some(Drag::Held);
            } else if matches!(self.drag, Some(Drag::Rectangle { .. })) {
                self.drag = Some(Drag::Held);
            } else if self.list.is_some() {
                self.list = None;
            } else {
                self.pinned = None;
                input.intents.push(Intent::Select(self.index.name(drawing, &select::up(drawing, &self.index, &self.found))));
            }
            return;
        }
        let tab = if pressed(egui::Key::Tab, egui::Modifiers::NONE) {
            Some(true)
        } else if pressed(egui::Key::Tab, egui::Modifiers::SHIFT) {
            Some(false)
        } else {
            None
        };
        if let Some(forward) = tab {
            // Tab leaves the terminals for the next node (notation 2.6).
            self.port_focus = None;
            match select::tab(drawing, &self.index, &self.found, forward, &visible) {
                Some(found) => input.intents.push(Intent::Select(self.index.name(drawing, &found))),
                None => {
                    let direction = if forward { egui::FocusDirection::Next } else { egui::FocusDirection::Previous };
                    ui.memory_mut(|memory| memory.move_focus(direction));
                }
            }
            return;
        }
        // Ctrl and an arrow, Shift with them for 64: Ctrl is the command key of macOS too.
        let moves = [(egui::Key::ArrowLeft, (-1, 0)), (egui::Key::ArrowRight, (1, 0)), (egui::Key::ArrowUp, (0, -1)), (egui::Key::ArrowDown, (0, 1))];
        for (key, (dx, dy)) in moves {
            let found = ui.input_mut(|input| {
                let at = input.events.iter().position(|event| {
                    matches!(event, egui::Event::Key { key: found, pressed: true, modifiers, .. } if *found == key && modifiers.command && !modifiers.alt)
                });
                at.and_then(|at| match input.events.remove(at) {
                    egui::Event::Key { modifiers, .. } => Some(modifiers.shift),
                    _ => None,
                })
            });
            if let Some(big) = found {
                let size = if big { 64 } else { moving::GRID as i64 };
                let now = ui.input(|input| input.time);
                // A terminal of the frame of the function selected alone moves along its edge (notation 2.4).
                if let Some(port) = self.selected_frame_terminal(drawing) {
                    if dy != 0 {
                        self.slide(drawing, port, dy * size, now, input);
                    }
                    return;
                }
                self.nudge(drawing, (dx * size, dy * size), now, input);
                return;
            }
        }
        let arrows = [
            (egui::Key::ArrowLeft, Vec2::new(-1.0, 0.0)),
            (egui::Key::ArrowRight, Vec2::X),
            (egui::Key::ArrowUp, Vec2::new(0.0, -1.0)),
            (egui::Key::ArrowDown, Vec2::Y),
        ];
        for (key, direction) in arrows {
            if pressed(key, egui::Modifiers::NONE) {
                if let Some(found) = select::arrow(drawing, &self.index, &self.found, direction, &visible) {
                    input.intents.push(Intent::Select(self.index.name(drawing, &found)));
                }
                return;
            }
        }
        if pressed(egui::Key::Enter, egui::Modifiers::NONE) {
            if let Some(found) = select::enter(drawing, &self.index, &self.found, &visible) {
                input.intents.push(Intent::Select(self.index.name(drawing, &found)));
            } else if let (Some(&placed), 1) = (self.found.nodes.first(), self.found.nodes.len()) {
                // A node that is not a structure: its terminals, the inputs first (notation 2.6).
                self.port_focus = node_ports(drawing, placed).first().copied();
            }
        }
    }

    /// The keys of a wire drawn from the keyboard (notation 2.6): Tab and Shift+Tab go through the terminals that would
    /// take it, Enter wires it, Escape cancels, Space opens quick insertion. Whether a key was used.
    fn wiring_keys(&mut self, ui: &mut Ui, input: &mut Input, drawing: &Drawing, visible: &dyn Fn(Option<usize>) -> bool) -> bool {
        if !self.wiring.as_ref().is_some_and(|wiring| wiring.state == wiring::State::Keys) {
            return false;
        }
        let alt = ui.input(|input| input.modifiers.alt);
        let rules = Rules { alt, ..self.rules };
        let take = |key: egui::Key, shift: bool| {
            ui.input_mut(|input| {
                let at = input.events.iter().position(|event| {
                    matches!(event, egui::Event::Key { key: found, pressed: true, modifiers, .. } if *found == key && modifiers.shift == shift && !modifiers.command)
                });
                at.map(|at| input.events.remove(at)).is_some()
            })
        };
        if take(egui::Key::Escape, false) {
            self.wiring = None;
            return true;
        }
        if take(egui::Key::Enter, false) {
            if self.wiring.as_ref().is_some_and(|wiring| wiring.over.is_none()) {
                input.intents.push(Intent::Say("No terminal takes this wire".to_owned()));
            } else {
                self.drop_wire(drawing, rules, input, visible);
            }
            return true;
        }
        let forward = if take(egui::Key::Tab, false) {
            Some(true)
        } else if take(egui::Key::Tab, true) {
            Some(false)
        } else {
            None
        };
        if let (Some(forward), Some(wiring)) = (forward, self.wiring.as_mut()) {
            let takers = wiring.takers(drawing, rules, visible);
            let at = wiring.over.and_then(|over| takers.iter().position(|&found| found == over));
            let next = match (at, forward) {
                (Some(at), true) => takers.get(at + 1).or_else(|| takers.first()),
                (Some(at), false) => at.checked_sub(1).and_then(|before| takers.get(before)).or_else(|| takers.last()),
                (None, true) => takers.first(),
                (None, false) => takers.last(),
            };
            wiring.over = next.copied();
            if let Some(port) = wiring.over.and_then(|over| drawing.ports.get(over)) {
                wiring.at = wiring::centre(port);
            }
            return true;
        }
        false
    }

    /// The keys of a terminal in focus (notation 2.6): Up and Down along its side, Left and Right to the other side,
    /// Enter on a wired one selects its wire, Escape goes back to its node. Whether a key was used.
    fn port_keys(&mut self, ui: &mut Ui, input: &mut Input, drawing: &Drawing) -> bool {
        let Some(focus) = self.port_focus else { return false };
        let Some(port) = drawing.ports.get(focus) else {
            self.port_focus = None;
            return false;
        };
        let pressed = |key: egui::Key| {
            ui.input_mut(|input| {
                let at = input.events.iter().position(
                    |event| matches!(event, egui::Event::Key { key: found, pressed: true, modifiers, .. } if *found == key && modifiers.is_none()),
                );
                at.map(|at| input.events.remove(at)).is_some()
            })
        };
        if pressed(egui::Key::Escape) {
            self.port_focus = None;
            return true;
        }
        if pressed(egui::Key::Enter) {
            let wired = port.holds == labcraft_diagram::Holds::Wire || !port.input;
            match (wired, port.group) {
                (true, Some(group)) => {
                    let found = select::Found::one(Hit::Wire(group));
                    self.port_focus = None;
                    input.intents.push(Intent::Select(self.index.name(drawing, &found)));
                }
                _ => input.intents.push(Intent::Say("Constants, defaults and holes are edited from M1 step 4".to_owned())),
            }
            return true;
        }
        let Some(placed) = port.placed else { return false };
        let ports = node_ports(drawing, placed);
        let side: Vec<usize> = ports.iter().copied().filter(|&at| drawing.ports.get(at).is_some_and(|found| found.input == port.input)).collect();
        let other: Vec<usize> = ports.iter().copied().filter(|&at| drawing.ports.get(at).is_some_and(|found| found.input != port.input)).collect();
        let at = side.iter().position(|&found| found == focus).unwrap_or(0);
        let next = if pressed(egui::Key::ArrowDown) {
            side.get(at + 1).copied()
        } else if pressed(egui::Key::ArrowUp) {
            at.checked_sub(1).and_then(|before| side.get(before)).copied()
        } else if pressed(egui::Key::ArrowLeft) || pressed(egui::Key::ArrowRight) {
            // The nearest in height on the other side.
            other.iter().copied().min_by_key(|&found| drawing.ports.get(found).map_or(i64::MAX, |found| (found.y - port.y).abs()))
        } else {
            return false;
        };
        if let Some(next) = next {
            self.port_focus = Some(next);
        }
        true
    }

    /// The values of the wire clicked last, over the iterations of its loop, pinned where it was clicked while it is
    /// selected and that place is in sight (notation 6.1).
    fn pinned_values(&mut self, frame: &Frame, input: &mut Input) {
        let Some((group, unit)) = self.pinned else { return };
        let coarse = self.viewport.zoom < style::factor(style::coarse_zoom());
        let seen = Rect::from_min_max(self.viewport.origin.to_pos2(), (self.viewport.origin + frame.rect.size() / self.viewport.zoom).to_pos2());
        let readable = frame.values.is_some() && !coarse && frame.offsets.is_none();
        let in_sight = frame.drawing.groups.get(group).is_some_and(|found| frame.visible(found.within)) && seen.contains(unit);
        if !readable || !in_sight {
            return;
        }
        let Some(title) = self.named(frame, group) else { return };
        let series = self.series_of(frame, group);
        let corner = frame.rect.min + (unit.to_vec2() - self.viewport.origin) * self.viewport.zoom + Vec2::new(12.0, 12.0);
        let theme = frame.theme;
        egui::Area::new(egui::Id::unique("labcraft-pinned-values")).order(egui::Order::Foreground).fixed_pos(corner).show(&frame.ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| match &series {
                Some(series) => {
                    if let Some(chosen) = series::show(ui, &title, series, theme, true) {
                        input.iteration = Some((series.key.clone(), chosen));
                    }
                }
                None => {
                    ui.label(&title);
                }
            });
        });
    }

    /// The series of a wire over the iterations of its loop, worked out once.
    fn series_of(&mut self, frame: &Frame, group: usize) -> Option<Arc<Series>> {
        let (trace, choice) = self.run.as_ref()?;
        let drawing = &frame.drawing;
        self.series.entry(group).or_insert_with(|| labcraft_diagram::series(drawing, trace, choice, group).map(Arc::new)).clone()
    }

    /// The name of a wire, its exact type and, while the values show, the value of the run.
    fn named(&self, frame: &Frame, group: usize) -> Option<String> {
        let wire = frame.drawing.groups.get(group)?.wire.as_ref()?;
        let value = labcraft_types::cut_text(&wire.value);
        let named = if wire.ty.is_empty() { value.into_owned() } else { format!("{value}: {}", wire.ty) };
        let coarse = self.viewport.zoom < style::factor(style::coarse_zoom());
        let run = self
            .values
            .as_ref()
            .filter(|_| frame.values.is_some() && !coarse && frame.offsets.is_none())
            .and_then(|values| values.overlay.wires.get(&group))
            .map(|valued| valued.title.clone());
        Some(match run {
            Some(run) => format!("{named}\n{run}"),
            None => named,
        })
    }

    /// The painting of a frame, which decides nothing: the drawing, the values, the selection and the gesture under
    /// way, then the tooltips.
    fn paint(&mut self, ui: &Ui, response: Response, frame: &Frame, theme: Theme, input: &Input) -> Response {
        let (ctx, drawing, rect) = (&frame.ctx, &frame.drawing, frame.rect);
        let painter = ui.painter_at(rect);
        let zoom = self.viewport.zoom;
        let origin = self.viewport.origin;
        let to_screen = |unit: Vec2| rect.min + (unit - origin) * zoom;
        let seen = Rect::from_min_max(origin.to_pos2(), (origin + rect.size() / zoom).to_pos2());
        let visible = |within: Option<usize>| frame.visible(within);
        // The texts wait for the fonts of the notation: a host that never installed them, or set its fonts again.
        let texts = has_family(ctx);
        if !texts {
            install_fonts(ctx);
            ctx.request_repaint();
        }
        let step = self.viewport.step(frame.now);
        let key = paint::Key { generation: self.generation, theme, step: step.to_bits(), pixels: ctx.pixels_per_point().to_bits() };
        self.painter.begin(ctx, key);
        let coarse = zoom < style::factor(style::coarse_zoom());
        let put = |painted: &Painted, offset: Vec2| painted.paint(&painter, to_screen(offset), zoom / step, zoom, texts);
        // The title of the function, then each group in the order of drawing, then the tabs, then the marks.
        if let Some(painted) = self.painter.loose(ctx, drawing, theme, step) {
            put(&painted, Vec2::ZERO);
        }
        for &index in &frame.groups {
            let Some(group) = drawing.groups.get(index) else { continue };
            if !visible(group.within) {
                continue;
            }
            let mut offset = frame.offset_of(group.placed);
            if let Some(wire) = &group.wire
                && frame.offsets.is_some()
            {
                // The new wires show at the end of the glide.
                let Some(moving) = self.moving.as_mut().filter(|_| frame.moving) else { continue };
                // A wire to a node moved: its preview (architecture 12.4); one inside a structure moved: with it.
                if let Some(&at) = moving.rerouted.get(&index) {
                    let shapes = moving.preview.wires.get(at).map(|(_, shapes)| shapes.as_slice()).unwrap_or_default();
                    let painted = moving.painted.entry(index).or_insert_with(|| Arc::new(Painted::of(ctx, drawing, shapes, theme, step))).clone();
                    put(&painted, Vec2::ZERO);
                    continue;
                }
                offset = moving.body_offset(wire.body);
            }
            let placed = group.placed.and_then(|placed| drawing.placed.get(placed));
            if coarse
                && group.arm.is_none()
                && let Some(placed) = placed.filter(|placed| placed.holds.is_empty())
            {
                // A node as a plain rectangle (notation 7).
                let corner = to_screen(offset + Vec2::new(placed.x as f32, placed.y as f32) / 100.0);
                let size = Vec2::new(placed.width as f32, placed.height as f32) / 100.0 * zoom;
                painter.add(paint::coarse_rect(Rect::from_min_size(corner, size), COARSE_NODE, theme, zoom));
                continue;
            }
            if let Some(painted) = self.painter.group(ctx, drawing, index, theme, step) {
                put(&painted, offset);
            }
        }
        // The pin of each fixed node shown (notation 2.4, 2.5), at the real scale.
        if frame.mark_fixed && !coarse {
            for &index in &frame.groups {
                let Some(placed) = drawing.groups.get(index).filter(|group| group.is_node() && visible(group.within)).and_then(|group| group.placed)
                else {
                    continue;
                };
                if let Some(area) = pins::area(drawing, placed, frame.offset_of(Some(placed))) {
                    pins::paint(&painter, Rect::from_min_max(to_screen(area.min.to_vec2()), to_screen(area.max.to_vec2())), theme, zoom);
                }
            }
            // The corner of each frame of fixed size, as the pin of a fixed node (notation 2.4), and the pin of each fixed
            // terminal of the frame of the function; not while nodes move.
            if frame.offsets.is_none() {
                for port in 0..drawing.ports.len() {
                    if let Some(area) = pins::terminal_area(drawing, port) {
                        pins::paint(&painter, Rect::from_min_max(to_screen(area.min.to_vec2()), to_screen(area.max.to_vec2())), theme, zoom);
                    }
                }
                for (_, area) in sizing::fixed(drawing, &|placed| self.grid.shown(placed, &visible)) {
                    paint_corner(&painter, to_screen(area.max.to_vec2()), zoom, theme.color("ink.muted"));
                }
            }
        }
        for (index, found) in drawing.tabs.iter().enumerate() {
            let structure_shown = drawing.arms.get(found.arm).is_some_and(|arm| visible(arm.within));
            // A part of a selector shows with the arm it stands for.
            let with_its_arm = found.when.is_none_or(|when| frame.arms.get(when).copied().unwrap_or(false));
            if !structure_shown || !with_its_arm {
                continue;
            }
            let offset = frame.offset_of(Some(found.structure));
            let unit = Rect::from_min_size(
                Pos2::new(found.x as f32 / 100.0, found.y as f32 / 100.0) + offset,
                Vec2::new(found.width as f32, found.height as f32) / 100.0,
            );
            if !unit.intersects(seen) {
                continue;
            }
            let is_shown = frame.arms.get(found.arm).copied().unwrap_or(false);
            if let Some(painted) = self.painter.tab(ctx, drawing, index, is_shown, theme, step) {
                put(&painted, offset);
            }
        }
        // The `+` after the tabs, in the editor only (notation 2.6): outlined as a tab not shown, its mark muted.
        if !coarse {
            let tab_shown = |tab: &labcraft_diagram::Tab| frame.tab_shown(tab);
            for (_, unit) in parts::plus_tabs(drawing, &|placed| frame.offset_of(Some(placed)), &tab_shown) {
                if !unit.intersects(seen) {
                    continue;
                }
                let area = Rect::from_min_max(to_screen(unit.min.to_vec2()), to_screen(unit.max.to_vec2()));
                let stroke = egui::Stroke::new(1.0, theme.color("frame.divider"));
                painter.rect_stroke(area, 3.0 * zoom, stroke, egui::StrokeKind::Inside);
                let font = egui::FontId::proportional((area.height() * 0.7).max(6.0));
                painter.text(area.center(), egui::Align2::CENTER_CENTER, "+", font, theme.color("ink.muted"));
            }
        }
        // The values of the run, but while nodes glide (notation 6.1): the veils, hatching, outlines and counters under the
        // marks, the banner and the bubbles over everything; the bubbles from `zoom.values` on, and on hover down to
        // `zoom.coarse` (notation 7).
        let shown_values = self.values.as_ref().filter(|_| frame.values.is_some() && frame.offsets.is_none());
        // A counter that a selector stands in for, when the header has room for only one.
        let replaced: Vec<usize> = shown_values
            .filter(|_| !coarse)
            .map(|found| {
                found.overlay.selectors.iter().filter(|selector| visible(selector.layer.within)).filter_map(|selector| selector.replaces).collect()
            })
            .unwrap_or_default();
        if let Some(Values { overlay, under, .. }) = shown_values {
            for (index, layer) in overlay.under.iter().enumerate() {
                if visible(layer.within)
                    && !replaced.contains(&index)
                    && under.get(index).is_some_and(|area| area.intersects(seen))
                    && let Some(painted) = self.painter.over(ctx, drawing, overlay, Over::Under(index), theme, step)
                {
                    put(&painted, Vec2::ZERO);
                }
            }
        }
        // The selection (notation 2.5): the halo of each wire, the outline of each node, a ring around each terminal.
        let selected_nodes: Vec<usize> = self.found.nodes.iter().copied().filter(|&placed| self.grid.shown(placed, &visible)).collect();
        if frame.offsets.is_none() {
            let mut wires: Vec<usize> = self.found.groups.iter().copied().collect();
            wires.extend(self.pinned.map(|(group, _)| group).filter(|group| !wires.contains(group)));
            for group in wires {
                if drawing.groups.get(group).is_some_and(|found| visible(found.within)) {
                    let painted =
                        self.painter.made(ctx, drawing, Over::SelectedWire(group), theme, step, || labcraft_diagram::selected_wire(drawing, group));
                    put(&painted, Vec2::ZERO);
                }
            }
            for &placed in &selected_nodes {
                let painted =
                    self.painter.made(ctx, drawing, Over::SelectedNode(placed), theme, step, || labcraft_diagram::selected_node(drawing, placed));
                put(&painted, Vec2::ZERO);
            }
            let ring = egui::Stroke::new((style::mark_width() as f32 / 100.0 * zoom).max(1.0), theme.color("select.stroke"));
            for hint in self.found.terminals.iter().filter_map(|&at| drawing.hints.get(at)) {
                let centre = Vec2::new((hint.x + hint.width / 2) as f32, (hint.y + hint.height / 2) as f32) / 100.0;
                painter.circle_stroke(to_screen(centre), hint.width.min(hint.height) as f32 / 200.0 * zoom, ring);
            }
            for index in 0..drawing.marks.len() {
                if let Some(painted) = self.painter.mark(ctx, drawing, index, theme, step) {
                    put(&painted, Vec2::ZERO);
                }
            }
        } else if frame.moving {
            // The nodes selected go with the move.
            for &placed in &selected_nodes {
                let painted =
                    self.painter.made(ctx, drawing, Over::SelectedNode(placed), theme, step, || labcraft_diagram::selected_node(drawing, placed));
                put(&painted, frame.offset_of(Some(placed)));
            }
        }
        // `zoom.values` may be set below `zoom.coarse`, under which no value shows (notation 7).
        let bubbles_drawn = !coarse && zoom >= style::factor(style::values_zoom());
        // The bubbles that a value at an input covers, by their index: neither drawn nor read on hover.
        let mut hidden = std::collections::BTreeSet::new();
        if let Some(Values { overlay, banner, bubbles, selectors, .. }) = shown_values {
            if banner.intersects(seen)
                && let Some(painted) = self.painter.over(ctx, drawing, overlay, Over::Banner, theme, step)
            {
                put(&painted, Vec2::ZERO);
            }
            for (index, selector) in overlay.selectors.iter().enumerate() {
                if !coarse
                    && visible(selector.layer.within)
                    && selectors.get(index).is_some_and(|area| area.intersects(seen))
                    && let Some(painted) = self.painter.over(ctx, drawing, overlay, Over::Selector(index), theme, step)
                {
                    put(&painted, Vec2::ZERO);
                }
            }
            // The values at the inputs of the node hovered and of the node selected alone: the bubble of the same wire
            // that one covers gives the same value and is not drawn (notation 6.1; decision of the owner, 2026-11-13).
            let alone = match selected_nodes.as_slice() {
                [only] => Some(*only),
                _ => None,
            };
            let nodes: std::collections::BTreeSet<usize> =
                if coarse { std::collections::BTreeSet::new() } else { [frame.hovered, alone].into_iter().flatten().collect() };
            for &placed in &nodes {
                let boxes = self.painter.input_boxes(placed, || {
                    labcraft_diagram::input_boxes(drawing, overlay, placed)
                        .into_iter()
                        .map(|(group, (left, top, right, bottom))| {
                            let area = Rect::from_min_max(Pos2::new(left as f32, top as f32) / 100.0, Pos2::new(right as f32, bottom as f32) / 100.0);
                            (group, area)
                        })
                        .collect()
                });
                for (index, (bubble, area)) in overlay.bubbles.iter().zip(bubbles).enumerate() {
                    if boxes.iter().any(|(group, input)| bubble.group == Some(*group) && overlaps(input, area)) {
                        hidden.insert(index);
                    }
                }
            }
            for (index, bubble) in overlay.bubbles.iter().enumerate() {
                if bubbles_drawn
                    && visible(bubble.layer.within)
                    && !hidden.contains(&index)
                    && bubbles.get(index).is_some_and(|area| area.intersects(seen))
                    && let Some(painted) = self.painter.over(ctx, drawing, overlay, Over::Bubble(index), theme, step)
                {
                    put(&painted, Vec2::ZERO);
                }
            }
            // The values at the inputs, over everything (notation 6.1).
            for placed in nodes {
                let painted =
                    self.painter.made(ctx, drawing, Over::Inputs(placed), theme, step, || labcraft_diagram::inputs_of(drawing, overlay, placed));
                put(&painted, Vec2::ZERO);
            }
        }
        if frame.offsets.is_none() {
            self.paint_grips(&painter, frame, &selected_nodes, theme, &to_screen);
        }
        self.paint_wiring(&painter, frame, theme, &to_screen);
        // The rectangle of selection under way, in `select.stroke`.
        if let Some(Drag::Rectangle { from, to, .. }) = self.drag {
            let area = Rect::from_two_pos(to_screen(from.to_vec2()), to_screen(to.to_vec2()));
            let stroke = theme.color("select.stroke");
            painter.rect(area, 0.0, stroke.gamma_multiply(0.12), egui::Stroke::new(1.0, stroke), egui::StrokeKind::Inside);
            ctx.request_repaint();
        }
        // The tooltip under the pointer: a bubble, its value whole (notation 6.1); a terminal (input); else the node, the
        // full name of what it calls (notation 3.2); else a wire, its name, its exact type, its value, and its values
        // over the iterations of its loop.
        let readable = shown_values.filter(|_| !coarse);
        let bubble = || {
            let (unit, readable) = (frame.at?, readable?);
            readable.overlay.bubbles.iter().zip(&readable.bubbles).enumerate().rev().find_map(|(index, (bubble, area))| {
                (bubbles_drawn && !hidden.contains(&index) && visible(bubble.layer.within) && area.contains(unit)).then(|| bubble.title.clone())
            })
        };
        let title = || {
            frame
                .hovered
                .and_then(|placed| self.grid.group_of(placed))
                .and_then(|group| drawing.groups.get(group))
                .map(|group| group.title.clone())
                .filter(|title| !title.is_empty())
        };
        let text = bubble().or_else(|| input.hint.clone()).or_else(title);
        let pinned = self.pinned.filter(|_| readable.is_some()).map(|(group, _)| group);
        match (text, frame.wire_under) {
            (Some(text), _) => response.on_hover_text_at_pointer(text),
            // The values pinned already show those of the wire selected.
            (None, Some(group)) if pinned == Some(group) => response,
            (None, Some(group)) => {
                let series = if readable.is_some() { self.series_of(frame, group) } else { None };
                match (self.named(frame, group), series) {
                    (Some(title), Some(series)) => response.on_hover_ui_at_pointer(|ui| {
                        series::show(ui, &title, &series, theme, false);
                    }),
                    (Some(title), None) => response.on_hover_text_at_pointer(title),
                    (None, _) => response,
                }
            }
            (None, None) => response,
        }
    }

    /// Where a frame is taken (notation 2.6, decision of the owner, 2026-11-18): on a structure hovered by its header or its
    /// edges, or selected, a grip at the start of its header and a corner at the bottom right; the edge under the
    /// pointer lit, 6 units wide. The box where a resize goes, while it goes.
    fn paint_grips(&self, painter: &egui::Painter, frame: &Frame, selected: &[usize], theme: Theme, to_screen: &dyn Fn(Vec2) -> Pos2) {
        let (drawing, zoom) = (&frame.drawing, self.viewport.zoom);
        let screen = |area: Rect| Rect::from_min_max(to_screen(area.min.to_vec2()), to_screen(area.max.to_vec2()));
        let select = theme.color("select.stroke");
        let muted = theme.color("ink.muted");
        // A terminal of a block moved by Alt and a drag: the free places of its block lit, the one it goes to ringed, and a
        // note that every call changes (notation 2.4).
        if let Some(moving) = &self.block_drag {
            for place in moving.free(drawing) {
                if let Some(at) = blockdrag::place_at(drawing, moving.placed, place) {
                    painter.circle_filled(to_screen(at.to_vec2()), (2.0 * zoom).max(2.0), select.gamma_multiply(0.35));
                }
            }
            if let Some(at) = blockdrag::place_at(drawing, moving.placed, moving.to) {
                painter.circle(to_screen(at.to_vec2()), (4.0 * zoom).max(4.0), select.gamma_multiply(0.25), egui::Stroke::new(1.5, select));
            }
            let at = to_screen(moving.at.to_vec2()) + Vec2::new(12.0, 12.0);
            painter.text(at, egui::Align2::LEFT_TOP, &moving.note, egui::FontId::proportional(12.0), muted);
            frame.ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        // Where a terminal of the frame goes, while it moves: a mark across its edge, its place on the grid.
        if let Some(sliding) = &self.sliding {
            let at = to_screen(sliding.at().to_vec2());
            let reach = 8.0 * zoom;
            painter.line_segment([at - Vec2::new(reach, 0.0), at + Vec2::new(reach, 0.0)], egui::Stroke::new(2.0, select));
            painter.circle(at, (3.0 * zoom).max(3.0), select.gamma_multiply(0.25), egui::Stroke::new(1.5, select));
            if sliding.state == edge::State::Dragging {
                frame.ctx.set_cursor_icon(egui::CursorIcon::ResizeVertical);
            }
        }
        if let Some(resizing) = &self.resizing {
            let area = screen(resizing.area());
            painter.rect(
                area,
                style::radius() as f32 / 100.0 * zoom,
                select.gamma_multiply(0.06),
                egui::Stroke::new(1.5, select),
                egui::StrokeKind::Inside,
            );
            frame.ctx.set_cursor_icon(resizing.grip.cursor(true));
            return;
        }
        let hovered = frame.grip.and_then(|(target, _)| target);
        let mut shown: Vec<usize> =
            selected.iter().copied().filter(|&placed| drawing.placed.get(placed).is_some_and(|found| !found.holds.is_empty())).collect();
        shown.extend(hovered.filter(|placed| !shown.contains(placed)));
        let dot = (0.8 * zoom).max(0.75);
        for placed in shown {
            let Some(area) = select::node_rect(drawing, placed) else { continue };
            // Two columns of three dots, in the header before its mark.
            let middle = area.min.y + header_of(drawing, placed) / 2.0;
            for column in [2.5, 5.5] {
                for row in [-3.0, 0.0, 3.0] {
                    painter.circle_filled(to_screen(Vec2::new(area.min.x + column, middle + row)), dot, muted);
                }
            }
            paint_corner(painter, to_screen(area.max.to_vec2()), zoom, muted);
        }
        let Some((target, grip)) = frame.grip.filter(|_| self.drag.is_none()) else { return };
        let area = match target {
            Some(placed) => select::node_rect(drawing, placed),
            None => select::body_rect(drawing, 0),
        };
        let Some(area) = area else { return };
        let lit = select.gamma_multiply(0.18);
        let band = sizing::EDGE_BAND;
        let right = Rect::from_min_max(Pos2::new(area.max.x - band, area.min.y), area.max);
        let bottom = Rect::from_min_max(Pos2::new(area.min.x, area.max.y - band), area.max);
        match grip {
            Grip::Move => {
                let left = Rect::from_min_max(area.min, Pos2::new(area.min.x + band, area.max.y));
                let header = target.map_or(0.0, |placed| header_of(drawing, placed)).max(sizing::EDGE_BAND);
                let top = Rect::from_min_max(area.min, Pos2::new(area.max.x, area.min.y + header));
                painter.rect_filled(screen(left), 0.0, lit);
                painter.rect_filled(screen(top), 0.0, lit);
            }
            Grip::Right => {
                painter.rect_filled(screen(right), 0.0, lit);
            }
            Grip::Bottom => {
                painter.rect_filled(screen(bottom), 0.0, lit);
            }
            Grip::Corner => {
                painter.rect_filled(screen(right), 0.0, lit);
                painter.rect_filled(screen(bottom), 0.0, lit);
            }
        }
        if target.is_none() {
            paint_corner(painter, to_screen(area.max.to_vec2()), zoom, muted);
        }
        frame.ctx.set_cursor_icon(grip.cursor(false));
    }

    /// The wire being drawn (notation 2.5): a ring on each terminal that would take it, the wire in the style of its
    /// family, snapped to the terminal it would reach on a square route, or dashed in `mark.invalid` over one that refuses
    /// it, with the reason where it would be dropped; the terminal that has the focus, and an old wire retracting.
    fn paint_wiring(&self, painter: &egui::Painter, frame: &Frame, theme: Theme, to_screen: &dyn Fn(Vec2) -> Pos2) {
        let (drawing, zoom) = (&frame.drawing, self.viewport.zoom);
        let screen = |at: Pos2| to_screen(at.to_vec2());
        let select = theme.color("select.stroke");
        let ring = egui::Stroke::new((style::mark_width() as f32 / 100.0 * zoom).max(1.0), select);
        if let Some(retract) = &self.retract
            && let Some(since) = retract.since
        {
            let part = retract.part(((frame.now - since) / wiring::RETRACT_SECONDS) as f32);
            let stroke = egui::Stroke::new((retract.width * zoom).max(1.0), theme.color(retract.token));
            painter.add(egui::Shape::line(part.into_iter().map(screen).collect(), stroke));
            frame.ctx.request_repaint();
        }
        if let Some(port) = self.port_focus.and_then(|focus| drawing.ports.get(focus)).filter(|_| self.wiring.is_none()) {
            painter.circle_stroke(screen(wiring::centre(port)), 6.0 * zoom, ring);
        }
        if let Some(rerouting) = &self.rerouting {
            // The route it will have, in the colour of what is being placed; the old one stays until the drawing comes.
            let stroke = egui::Stroke::new((2.0 * zoom).max(1.5), select);
            for (_, path, _) in rerouting.routes(drawing) {
                painter.add(egui::Shape::line(path.into_iter().map(|point| screen(wiring::unit(point))).collect(), stroke));
            }
            if rerouting.state == rerouting::State::Dragging {
                frame.ctx.request_repaint();
            }
        }
        let Some(wiring) = &self.wiring else { return };
        let rules = Rules { alt: frame.ctx.input(|input| input.modifiers.alt), ..self.rules };
        let visible = |within: Option<usize>| frame.visible(within);
        for at in wiring.takers(drawing, rules, &visible) {
            if let Some(port) = drawing.ports.get(at) {
                painter.circle_stroke(screen(wiring::centre(port)), 5.0 * zoom, ring);
            }
        }
        let dropped = wiring.over.and_then(|over| wiring.drop_on(drawing, over, rules));
        // Out of loops, the mark of what each makes, beside the pointer (notation 2.6).
        let passing = match &dropped {
            Some(wiring::Drop::Wire { through, .. }) if !through.is_empty() => Some(wiring::passage_marks(through)),
            _ => None,
        };
        let refused = match dropped {
            Some(wiring::Drop::Refused(refusal)) => Some(refusal),
            _ => None,
        };
        let Some(start) = drawing.ports.get(wiring.from) else { return };
        let width = (start.style.width as f32 / 100.0 * zoom).max(1.5);
        for path in wiring.paths(drawing) {
            let points: Vec<Pos2> = path.into_iter().map(screen).collect();
            if refused.is_some() {
                let stroke = egui::Stroke::new(width, theme.color("mark.invalid"));
                painter.extend(egui::Shape::dashed_line(&points, stroke, 6.0 * zoom, 4.0 * zoom));
            } else {
                painter.add(egui::Shape::line(points, egui::Stroke::new(width, theme.color(start.style.token))));
            }
        }
        if let Some(refusal) = refused {
            let corner = screen(wiring::reason_at(wiring.at));
            let galley = painter.layout(refusal.message, egui::FontId::proportional(13.0), theme.color("mark.invalid"), 360.0);
            let area = Rect::from_min_size(corner, galley.size()).expand(4.0);
            painter.rect(area, 3.0, theme.color("canvas.fill"), egui::Stroke::new(1.0, theme.color("mark.invalid")), egui::StrokeKind::Inside);
            painter.galley(corner, galley, theme.color("mark.invalid"));
        }
        if let Some(marks) = passing {
            let corner = screen(wiring::reason_at(wiring.at));
            let ink = theme.color(start.style.token);
            let galley = painter.layout(marks, egui::FontId::proportional(13.0), ink, 360.0);
            let area = Rect::from_min_size(corner, galley.size()).expand(4.0);
            painter.rect(area, 3.0, theme.color("canvas.fill"), egui::Stroke::new(1.0, ink), egui::StrokeKind::Inside);
            painter.galley(corner, galley, ink);
        }
        if wiring.state == wiring::State::Dragging {
            frame.ctx.request_repaint();
        }
    }

    /// The canvas in the tree of AccessKit (notation 2.6): a list of what can be selected, each node and terminal of the
    /// frame of the function an element with its name, selected or not. Built only while a reader of the tree asks.
    fn describe(&self, ui: &mut Ui, response: &Response, frame: &Frame) {
        let drawing = &frame.drawing;
        let label = format!("Diagram of {}", drawing.function);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ListBox, true, label.clone()));
        let active = frame.ctx.accesskit_node_builder(response.id, |_| ()).is_some();
        if !active {
            return;
        }
        let (zoom, origin, rect) = (self.viewport.zoom, self.viewport.origin, frame.rect);
        let screen =
            |area: Rect| Rect::from_min_max(rect.min + (area.min.to_vec2() - origin) * zoom, rect.min + (area.max.to_vec2() - origin) * zoom);
        let mut element = |salt: (&str, usize), area: Rect, name: String, selected: bool| {
            let child = ui.new_child(egui::UiBuilder::new().id_salt(salt).max_rect(screen(area)).accessibility_parent(response.id));
            frame.ctx.accesskit_node_builder(child.unique_id(), |node| {
                node.set_role(egui::accesskit::Role::ListBoxOption);
                node.set_label(name);
                node.set_selected(selected);
                let area = screen(area);
                node.set_bounds(egui::accesskit::Rect { x0: area.min.x.into(), y0: area.min.y.into(), x1: area.max.x.into(), y1: area.max.y.into() });
            });
        };
        for (at, hint) in drawing.hints.iter().enumerate() {
            let Some(name) = &hint.selects else { continue };
            let ty = hint.text.lines().next().and_then(|line| line.split_once(": ")).map(|(_, ty)| format!(", {ty}")).unwrap_or_default();
            let area = Rect::from_min_size(Pos2::new(hint.x as f32, hint.y as f32) / 100.0, Vec2::new(hint.width as f32, hint.height as f32) / 100.0);
            element(("terminal", at), area, format!("{name}{ty}"), self.found.terminals.contains(&at));
        }
        for (at, placed) in drawing.placed.iter().enumerate() {
            if !self.grid.shown(at, &|within| frame.visible(within)) {
                continue;
            }
            let Some(area) = select::node_rect(drawing, at) else { continue };
            let title = self.grid.group_of(at).and_then(|group| drawing.groups.get(group)).map(|group| group.title.as_str()).unwrap_or_default();
            let name = match (placed.name.as_str(), title) {
                ("", title) => title.to_owned(),
                (name, "") => name.to_owned(),
                (name, title) => format!("{name}, {title}"),
            };
            element(("node", at), area, name, self.found.nodes.contains(&at));
        }
        // The terminals of the nodes shown: "input high of verdict, f64[V], constant 240 V" (notation 2.6).
        for (at, port) in drawing.ports.iter().enumerate() {
            let Some(placed) = port.placed.filter(|&placed| !port.frame && self.grid.shown(placed, &|within| frame.visible(within))) else {
                continue;
            };
            let node = drawing.placed.get(placed).map(|node| node.name.as_str()).filter(|name| !name.is_empty()).unwrap_or("its node");
            element(
                ("port", at),
                Rect::from_center_size(wiring::centre(port), Vec2::splat(8.0)),
                describe_port(drawing, port, node),
                self.port_focus == Some(at),
            );
        }
    }

    /// The iteration a selector shows, or the one asked for last when its values have not come yet.
    fn shown_of(&self, selector: &labcraft_diagram::Selector) -> u64 {
        match &self.asked {
            Some((key, asked)) if *key == selector.key => *asked,
            _ => selector.shown,
        }
    }
}

/// How a screen reader names a terminal of a node (notation 2.6): its side, its name, its node, its type and what it
/// holds.
fn describe_port(drawing: &Drawing, port: &labcraft_diagram::Port, node: &str) -> String {
    use labcraft_diagram::Holds;
    let side = if port.input { "input" } else { "output" };
    let mut name = if port.caption.is_empty() { format!("{side} of {node}") } else { format!("{side} {} of {node}", port.caption) };
    if let Some(ty) = port.ty.and_then(|ty| drawing.types.get(ty)) {
        name.push_str(", ");
        name.push_str(ty);
    }
    let holds = match &port.holds {
        Holds::Wire if port.input => format!("wired from {}", port.value),
        Holds::Constant(constant) => format!("constant {constant}"),
        Holds::Default(default) => format!("default {default}"),
        Holds::Hole | Holds::Missing => "required input, not wired".to_owned(),
        Holds::Wire | Holds::Nothing => String::new(),
    };
    if !holds.is_empty() {
        name.push_str(", ");
        name.push_str(&holds);
    }
    name
}

/// The settings of `shown` that a drop reads, with Alt.
fn rules_of(shown: &Shown, alt: bool) -> Rules {
    Rules { replace_with_alt: shown.replace_with_alt, accept_mismatch: shown.accept_mismatch, alt }
}

/// The edge or the header of a frame under `at`, in units, at `zoom` (notation 2.6): none where a press takes a terminal or
/// a wire instead, so that the pointer shows what the press does.
fn grip_under(
    drawing: &Drawing,
    index: &Index,
    groups: &[usize],
    at: Pos2,
    zoom: f32,
    visible: &dyn Fn(Option<usize>) -> bool,
) -> Option<(sizing::Target, Grip)> {
    if port_at(drawing, at, (6.0 / zoom).max(5.0), visible).is_some()
        || select::wire_at(drawing, groups, at, (5.0 / zoom).max(WIRE_REACH), visible).is_some()
    {
        return None;
    }
    sizing::grip_at(drawing, index, at, (6.0 / zoom).max(4.0), visible)
}

/// The terminal nearest `at` within `reach` units, shown, that a command wires.
fn port_at(drawing: &Drawing, at: Pos2, reach: f32, visible: &dyn Fn(Option<usize>) -> bool) -> Option<usize> {
    drawing
        .ports
        .iter()
        .enumerate()
        .filter(|(_, port)| !port.name.is_empty() && visible(port.within))
        .map(|(index, port)| (index, wiring::centre(port).distance(at)))
        .filter(|(_, distance)| *distance <= reach)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(index, _)| index)
}

/// The terminals of a node, its inputs then its outputs, each side from the top, by their index in [`Drawing::ports`].
fn node_ports(drawing: &Drawing, placed: usize) -> Vec<usize> {
    let mut ports: Vec<usize> =
        (0..drawing.ports.len()).filter(|&at| drawing.ports.get(at).is_some_and(|port| port.placed == Some(placed) && !port.frame)).collect();
    ports.sort_by_key(|&at| drawing.ports.get(at).map_or((true, 0), |port| (!port.input, port.y)));
    ports
}

/// An arrow key.
fn is_arrow(key: egui::Key) -> bool {
    matches!(key, egui::Key::ArrowLeft | egui::Key::ArrowRight | egui::Key::ArrowUp | egui::Key::ArrowDown)
}

pub(crate) fn arms_shown(drawing: &Drawing, shown: &Shown) -> Vec<bool> {
    let counts: HashMap<&str, usize> = drawing.arms.iter().fold(HashMap::new(), |mut counts, arm| {
        *counts.entry(arm.structure.as_str()).or_insert(0) += 1;
        counts
    });
    let mut result: Vec<bool> = Vec::with_capacity(drawing.arms.len());
    for arm in &drawing.arms {
        let count = counts.get(arm.structure.as_str()).copied().unwrap_or(1);
        let chosen = shown.arm_of(&arm.structure);
        let chosen = if chosen < count { chosen } else { 0 };
        // An arm comes after the arm that holds its structure.
        let around = arm.within.is_none_or(|within| result.get(within).copied().unwrap_or(false));
        result.push(around && arm.arm == chosen);
    }
    result
}

#[cfg(test)]
mod tests;
