//! The icons of the shell: SVG files of Lucide (`assets/icons/lucide/`), tinted when they are drawn. Not glyphs: a glyph
//! that no installed face has would stop egui, and the web has no fonts of the system to fall back on. A host calls
//! [`install`] once on its context before the first icon is drawn; without it no icon shows.

use std::sync::OnceLock;

use craft_ui::icons::{SvgIconSet, SvgStyle};
use egui::{Color32, Rect, Response, Sense, Vec2};

/// An icon of the shell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Home,
    Undo,
    Redo,
    Close,
    Minimize,
    Maximize,
    /// Two squares, one behind the other: the window is maximized, a click restores it.
    Restore,
    /// The views of a tab (ide.md 3): two nodes and a wire.
    Diagram,
    /// Lines of text.
    Text,
    /// A box with its terminals.
    Block,
    /// Two columns: the text beside the diagram.
    Split,
    /// A front panel: a window with a knob and a gauge.
    Panel,
    /// Four corners: fit the diagram to the view.
    Fit,
    /// A frame with an arrow leaving it at its top right: move the tab into a window of its own.
    Detach,
    /// A frame with an arrow coming into it: move the window back among the tabs.
    Dock,
    /// A window with a column on its right: the inspector, beside the canvas.
    Inspector,
    /// A window with a column of lines on its left: the palette of functions, beside the canvas.
    Palette,
    /// A triangle pointing right: run the function.
    Run,
    /// A square: stop the run.
    Stop,
    /// Two bars: pause the run.
    Pause,
    /// A disc: a breakpoint on a node.
    Breakpoint,
    /// An eye: watch a value.
    Watch,
    /// A line of a signal: record the values of a run.
    Record,
    /// Two windows side by side: every function in a window of its own.
    DetachedMode,
    /// A sun: the light theme.
    Sun,
    /// A crescent moon: the dark theme.
    Moon,
    /// A folder, in a tree of files.
    Folder,
    /// A file of code, in a tree of files.
    File,
}

/// A button showing an icon that can be on, as one of a group (the views of a tab): its hover text names it; the one on
/// has the background of the selection and a line of the accent under it.
pub fn toggle(ui: &mut egui::Ui, icon: Icon, on: bool, enabled: bool, hover: &str) -> Response {
    let size = Vec2::new(ui.spacing().interact_size.y + 4.0, ui.spacing().interact_size.y);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    if ui.is_rect_visible(rect) {
        let visuals = ui.visuals();
        if on {
            ui.painter().rect_filled(rect, visuals.widgets.active.corner_radius, visuals.selection.bg_fill);
            let line = egui::Rect::from_min_max(egui::pos2(rect.left() + 5.0, rect.bottom() - 2.0), egui::pos2(rect.right() - 5.0, rect.bottom()));
            ui.painter().rect_filled(line, 1.0, visuals.hyperlink_color);
        } else if response.hovered() && enabled {
            ui.painter().rect_filled(rect, visuals.widgets.hovered.corner_radius, visuals.widgets.hovered.weak_bg_fill);
        }
        let colour = if !enabled {
            visuals.weak_text_color().gamma_multiply(0.6)
        } else if on {
            visuals.strong_text_color()
        } else {
            visuals.weak_text_color()
        };
        paint(ui, Rect::from_center_size(rect.center(), Vec2::splat(16.0)), icon, colour);
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, enabled, on, hover));
    response.on_hover_text(hover)
}

/// A button showing an icon, with its hover text; greyed when not enabled.
pub fn button(ui: &mut egui::Ui, icon: Icon, enabled: bool, hover: &str) -> Response {
    let size = Vec2::splat(ui.spacing().interact_size.y);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        if response.hovered() && enabled {
            ui.painter().rect_filled(rect, visuals.corner_radius, visuals.weak_bg_fill);
        }
        let colour = if enabled { visuals.fg_stroke.color } else { ui.visuals().weak_text_color() };
        paint(ui, Rect::from_center_size(rect.center(), Vec2::splat(16.0)), icon, colour);
    }
    response.on_hover_text(hover)
}

/// The button of an entry to come (ide.md 6): greyed as a button that is not enabled, yet a click opens the card that
/// says what it will be.
pub fn planned(ui: &mut egui::Ui, icon: Icon, hover: &str) -> Response {
    let size = Vec2::splat(ui.spacing().interact_size.y);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        if response.hovered() {
            ui.painter().rect_filled(rect, visuals.corner_radius, visuals.weak_bg_fill);
        }
        let colour = ui.visuals().weak_text_color().gamma_multiply(0.6);
        paint(ui, Rect::from_center_size(rect.center(), Vec2::splat(16.0)), icon, colour);
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, hover));
    response.on_hover_text(hover)
}

/// An icon before the label of a line (a tree of files): 14 points, in `colour`, the height of a line.
pub fn leading(ui: &mut egui::Ui, icon: Icon, colour: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(16.0, ui.spacing().interact_size.y), Sense::hover());
    if ui.is_rect_visible(rect) {
        paint(ui, Rect::from_center_size(rect.center(), Vec2::splat(14.0)), icon, colour);
    }
}

/// The SVG files of Lucide that the icons are (ISC, `assets/icons/lucide/`, `ATTRIBUTION.md`), by their name.
const LUCIDE: &[(&str, &[u8])] = &[
    ("activity", include_bytes!("../../../assets/icons/lucide/activity.svg")),
    ("app-window", include_bytes!("../../../assets/icons/lucide/app-window.svg")),
    ("circle-dot", include_bytes!("../../../assets/icons/lucide/circle-dot.svg")),
    ("columns-2", include_bytes!("../../../assets/icons/lucide/columns-2.svg")),
    ("copy", include_bytes!("../../../assets/icons/lucide/copy.svg")),
    ("eye", include_bytes!("../../../assets/icons/lucide/eye.svg")),
    ("file-code", include_bytes!("../../../assets/icons/lucide/file-code.svg")),
    ("folder", include_bytes!("../../../assets/icons/lucide/folder.svg")),
    ("gauge", include_bytes!("../../../assets/icons/lucide/gauge.svg")),
    ("house", include_bytes!("../../../assets/icons/lucide/house.svg")),
    ("minus", include_bytes!("../../../assets/icons/lucide/minus.svg")),
    ("moon", include_bytes!("../../../assets/icons/lucide/moon.svg")),
    ("panel-left", include_bytes!("../../../assets/icons/lucide/panel-left.svg")),
    ("panel-right", include_bytes!("../../../assets/icons/lucide/panel-right.svg")),
    ("pause", include_bytes!("../../../assets/icons/lucide/pause.svg")),
    ("play", include_bytes!("../../../assets/icons/lucide/play.svg")),
    ("redo-2", include_bytes!("../../../assets/icons/lucide/redo-2.svg")),
    ("scan", include_bytes!("../../../assets/icons/lucide/scan.svg")),
    ("square", include_bytes!("../../../assets/icons/lucide/square.svg")),
    ("square-arrow-down-left", include_bytes!("../../../assets/icons/lucide/square-arrow-down-left.svg")),
    ("square-arrow-out-up-right", include_bytes!("../../../assets/icons/lucide/square-arrow-out-up-right.svg")),
    ("square-function", include_bytes!("../../../assets/icons/lucide/square-function.svg")),
    ("sun", include_bytes!("../../../assets/icons/lucide/sun.svg")),
    ("text", include_bytes!("../../../assets/icons/lucide/text.svg")),
    ("undo-2", include_bytes!("../../../assets/icons/lucide/undo-2.svg")),
    ("workflow", include_bytes!("../../../assets/icons/lucide/workflow.svg")),
    ("x", include_bytes!("../../../assets/icons/lucide/x.svg")),
];

impl Icon {
    /// The name of its file of Lucide.
    pub fn lucide(self) -> &'static str {
        match self {
            Icon::Home => "house",
            Icon::Undo => "undo-2",
            Icon::Redo => "redo-2",
            Icon::Close => "x",
            Icon::Minimize => "minus",
            Icon::Maximize => "square",
            Icon::Restore => "copy",
            Icon::Diagram => "workflow",
            Icon::Text => "text",
            Icon::Block => "square-function",
            Icon::Split => "columns-2",
            Icon::Panel => "gauge",
            Icon::Fit => "scan",
            Icon::Detach => "square-arrow-out-up-right",
            Icon::Dock => "square-arrow-down-left",
            Icon::Inspector => "panel-right",
            Icon::Palette => "panel-left",
            Icon::Run => "play",
            Icon::Stop => "square",
            Icon::Pause => "pause",
            Icon::Breakpoint => "circle-dot",
            Icon::Watch => "eye",
            Icon::Record => "activity",
            Icon::DetachedMode => "app-window",
            Icon::Sun => "sun",
            Icon::Moon => "moon",
            Icon::Folder => "folder",
            Icon::File => "file-code",
        }
    }
}

/// The icons, prepared once: white, so that a tint gives each its colour, with a stroke a little thinner than Lucide's 2.
fn icons() -> &'static SvgIconSet {
    static ICONS: OnceLock<SvgIconSet> = OnceLock::new();
    ICONS.get_or_init(|| SvgIconSet::new(LUCIDE, SvgStyle { current_color: "white", stroke_width: Some("1.75") }, "bytes://kit-shell/lucide/", "x"))
}

/// Installs the loader of the SVG files of the icons on the context: once, before the first icon is drawn.
pub fn install(ctx: &egui::Context) {
    egui_extras::install_image_loaders(ctx);
}

/// Paints `icon` within `rect`, in `colour`.
pub fn paint(ui: &egui::Ui, rect: Rect, icon: Icon, colour: Color32) {
    egui::Image::new(icons().source(icon.lucide())).tint(colour).fit_to_exact_size(rect.size()).paint_at(ui, rect);
}
