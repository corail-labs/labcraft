//! The look of the shell: colour tokens, radii, spacing and type, for a dark and a light theme. Widgets read
//! [`Tokens::of`] rather than colours of their own, and [`install`] gives egui the same values, so that its widgets and
//! the docking of `craft-ui` follow. Only defaults: the user changes them (ide.md 10).

use egui::{Color32, CornerRadius, FontFamily, FontId, Shadow, Stroke, TextStyle, Theme, Vec2};

/// A look: the same layout, other colours.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Look {
    /// Neutral graphite, indigo accent.
    #[default]
    Studio,
    /// Blue slate, teal accent.
    Lab,
}

impl Look {
    /// The name of the look, as the settings keep it.
    pub fn name(self) -> &'static str {
        match self {
            Look::Studio => "studio",
            Look::Lab => "lab",
        }
    }

    /// The look named `name`: `studio` or `lab`.
    pub fn named(name: &str) -> Option<Look> {
        match name.trim() {
            "studio" => Some(Look::Studio),
            "lab" => Some(Look::Lab),
            _ => None,
        }
    }
}

/// The colours of a theme.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    pub dark: bool,
    /// The top bar, the status bar, the headers of the panels.
    pub chrome: Color32,
    /// The panels.
    pub base: Color32,
    /// Under the content: the canvas around the drawing, the fields.
    pub sunken: Color32,
    /// Above the panels: buttons, cards, menus, windows.
    pub raised: Color32,
    pub border: Color32,
    pub hover: Color32,
    pub text: Color32,
    pub text_weak: Color32,
    pub accent: Color32,
    /// The background of what is selected.
    pub accent_soft: Color32,
    /// Text on the accent.
    pub on_accent: Color32,
    pub danger: Color32,
    pub warn: Color32,
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

impl Tokens {
    /// The tokens of `look` in the dark or light theme.
    pub const fn new(look: Look, dark: bool) -> Tokens {
        match (look, dark) {
            (Look::Studio, true) => Tokens {
                dark,
                chrome: rgb(0x18191c),
                base: rgb(0x1e1f23),
                sunken: rgb(0x141518),
                raised: rgb(0x2a2b31),
                border: rgb(0x303238),
                hover: rgb(0x2e3036),
                text: rgb(0xe4e5e9),
                text_weak: rgb(0x9a9ca5),
                accent: rgb(0x7c8cff),
                accent_soft: rgb(0x2c3260),
                on_accent: rgb(0xffffff),
                danger: rgb(0xf0645a),
                warn: rgb(0xe0a23a),
            },
            (Look::Studio, false) => Tokens {
                dark,
                chrome: rgb(0xf1f1f4),
                base: rgb(0xf8f8fa),
                sunken: rgb(0xffffff),
                raised: rgb(0xffffff),
                border: rgb(0xdedfe4),
                hover: rgb(0xe8e8ee),
                text: rgb(0x1c1d22),
                text_weak: rgb(0x6a6d76),
                accent: rgb(0x4f5bd5),
                accent_soft: rgb(0xdfe2fb),
                on_accent: rgb(0xffffff),
                danger: rgb(0xc8382e),
                warn: rgb(0xa86a00),
            },
            (Look::Lab, true) => Tokens {
                dark,
                chrome: rgb(0x131920),
                base: rgb(0x18202a),
                sunken: rgb(0x10151b),
                raised: rgb(0x222c38),
                border: rgb(0x2a3542),
                hover: rgb(0x26313e),
                text: rgb(0xdce5ee),
                text_weak: rgb(0x8796a8),
                accent: rgb(0x2fbfae),
                accent_soft: rgb(0x173f3e),
                on_accent: rgb(0x06201d),
                danger: rgb(0xf0645a),
                warn: rgb(0xe0a23a),
            },
            (Look::Lab, false) => Tokens {
                dark,
                chrome: rgb(0xeaeff4),
                base: rgb(0xf4f7fa),
                sunken: rgb(0xffffff),
                raised: rgb(0xffffff),
                border: rgb(0xd3dbe4),
                hover: rgb(0xdfe6ee),
                text: rgb(0x16202b),
                text_weak: rgb(0x5b6878),
                accent: rgb(0x0d8f82),
                accent_soft: rgb(0xd3efeb),
                on_accent: rgb(0xffffff),
                danger: rgb(0xc8382e),
                warn: rgb(0xa86a00),
            },
        }
    }

    /// The tokens of the theme `ctx` shows.
    pub fn of(ctx: &egui::Context) -> Tokens {
        let look = ctx.data(|data| data.get_temp::<Look>(egui::Id::unique("kit-shell-look"))).unwrap_or_default();
        Tokens::new(look, ctx.theme() == Theme::Dark)
    }
}

/// Radii, in points.
pub const RADIUS: u8 = 6;
/// The height of the top bar, the title bar of the window (ide.md 2).
pub const TOP_BAR: f32 = 36.0;

/// Gives egui the look, in both themes; [`Tokens::of`] gives it back.
pub fn install(ctx: &egui::Context, look: Look) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::unique("kit-shell-look"), look));
    for theme in [Theme::Dark, Theme::Light] {
        let tokens = Tokens::new(look, theme == Theme::Dark);
        ctx.style_mut_of(theme, |style| apply(style, &tokens));
    }
}

fn apply(style: &mut egui::Style, t: &Tokens) {
    let radius = CornerRadius::same(RADIUS);
    let mut visuals = if t.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    visuals.panel_fill = t.base;
    visuals.window_fill = t.raised;
    visuals.window_stroke = Stroke::new(1.0, t.border);
    visuals.window_corner_radius = CornerRadius::same(RADIUS + 2);
    visuals.menu_corner_radius = radius;
    visuals.window_shadow = Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(if t.dark { 110 } else { 40 }) };
    visuals.popup_shadow = Shadow { offset: [0, 4], blur: 12, spread: 0, color: Color32::from_black_alpha(if t.dark { 90 } else { 30 }) };
    visuals.extreme_bg_color = t.sunken;
    visuals.faint_bg_color = t.hover;
    visuals.code_bg_color = t.raised;
    visuals.hyperlink_color = t.accent;
    visuals.warn_fg_color = t.warn;
    visuals.error_fg_color = t.danger;
    visuals.selection.bg_fill = t.accent_soft;
    visuals.selection.stroke = Stroke::new(1.0, t.text);
    visuals.indent_has_left_vline = false;
    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_fill = t.base;
    widgets.noninteractive.weak_bg_fill = t.base;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, t.border);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, t.text);
    widgets.inactive.bg_fill = t.raised;
    widgets.inactive.weak_bg_fill = t.raised;
    // A line around a field, a check box or a button at rest: in the light theme, `raised` is near the panel behind it, and
    // a check box without it is not seen.
    widgets.inactive.bg_stroke = Stroke::new(1.0, t.border);
    widgets.inactive.fg_stroke = Stroke::new(1.0, t.text);
    widgets.hovered.bg_fill = t.hover;
    widgets.hovered.weak_bg_fill = t.hover;
    widgets.hovered.bg_stroke = Stroke::new(1.0, t.border);
    widgets.hovered.fg_stroke = Stroke::new(1.5, t.text);
    widgets.hovered.expansion = 0.0;
    widgets.active.bg_fill = t.accent_soft;
    widgets.active.weak_bg_fill = t.accent_soft;
    widgets.active.bg_stroke = Stroke::new(1.0, t.accent);
    widgets.active.fg_stroke = Stroke::new(1.5, t.text);
    widgets.active.expansion = 0.0;
    widgets.open.bg_fill = t.hover;
    widgets.open.weak_bg_fill = t.hover;
    widgets.open.bg_stroke = Stroke::new(1.0, t.border);
    widgets.open.fg_stroke = Stroke::new(1.0, t.text);
    for state in [&mut widgets.noninteractive, &mut widgets.inactive, &mut widgets.hovered, &mut widgets.active, &mut widgets.open] {
        state.corner_radius = radius;
    }
    visuals.weak_text_color = Some(t.text_weak);
    style.visuals = visuals;

    let spacing = &mut style.spacing;
    spacing.item_spacing = Vec2::new(8.0, 6.0);
    spacing.button_padding = Vec2::new(8.0, 4.0);
    spacing.interact_size.y = 24.0;
    spacing.menu_margin = egui::Margin::same(6);
    spacing.window_margin = egui::Margin::same(12);
    spacing.indent = 16.0;

    style.text_styles = [
        (TextStyle::Small, FontId::new(11.5, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(13.5, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(13.5, FontFamily::Proportional)),
        (TextStyle::Heading, FontId::new(17.0, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace)),
    ]
    .into();
}

/// A frame for a bar of the window (top, status), in the colour of the chrome; the panel draws the line on the side of
/// the content. No margin on the right: the buttons of the window, when the app draws them, reach its corner.
pub fn bar_frame(t: &Tokens) -> egui::Frame {
    egui::Frame::new().fill(t.chrome).inner_margin(egui::Margin { left: 8, right: 0, top: 0, bottom: 0 })
}
