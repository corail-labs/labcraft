//! The colours of a block that its file writes, `color`, `fill` and `stroke` (spec 11.2), and what the theme derives
//! from them (notation 8): the band of `color` as written, its lightness moved only when no ink holds 4.5:1 on it; the
//! tint of the body, `node.fill` mixed with `color`; the ink of a text or an icon on a colour that a file writes.

use labcraft_format::block::Colours;
use labcraft_graph::Rgb as Written;

use crate::shape::{Colour, Token};
use crate::style::{self, Rgb};
use crate::theme::{contrast, linear};

/// The two inks, the same in both themes (notation 8): their tokens.
pub const INK_LIGHT: Token = "ink.light";
pub const INK_DARK: Token = "ink.dark";

/// What a text must hold on its fill (notation 8).
const TEXT_CONTRAST: f64 = 4.5;

/// The share of `color` in the tint of the body, in percent *(proposed)*.
const TINT: u32 = 16;

/// How a block is painted: neutral, or as its file colours it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Inks {
    /// The fill of the body.
    pub body: Colour,
    /// The outline.
    pub stroke: Colour,
    /// The fill of the band.
    pub band: Colour,
    /// The text of the band.
    pub band_text: Token,
    /// The label, the icon and the unit of a primitive in the centre.
    pub ink: Token,
}

impl Default for Inks {
    fn default() -> Inks {
        Inks {
            body: Colour::Token("node.fill"),
            stroke: Colour::Token("node.stroke"),
            band: Colour::Token("band.fill"),
            band_text: "band.text",
            ink: "node.title",
        }
    }
}

impl Inks {
    /// The inks of a block whose file writes `colours`, from the style in force.
    pub fn of(colours: &Colours) -> Inks {
        let mut inks = Inks::default();
        if *colours == Colours::default() {
            return inks;
        }
        if let Some(color) = colours.color.map(Written::value) {
            let band = band(color);
            inks.band = Colour::Written(band, band);
            inks.band_text = best_ink(band);
            let tint = |dark: bool| {
                let (Some(fill), Some(title)) = (style::color("node.fill", dark), style::color("node.title", dark)) else { return color };
                tint(fill, color, title)
            };
            inks.body = Colour::Written(tint(false), tint(true));
        }
        if let Some(fill) = colours.fill.map(Written::value) {
            inks.body = Colour::Written(fill, fill);
            inks.ink = best_ink(fill);
        }
        if let Some(stroke) = colours.stroke.map(Written::value) {
            inks.stroke = Colour::Written(stroke, stroke);
        }
        inks
    }
}

/// The values of the two inks in force, light then dark: the same in both themes.
fn ink_values() -> (Rgb, Rgb) {
    (style::color(INK_LIGHT, false).unwrap_or_default(), style::color(INK_DARK, false).unwrap_or_default())
}

/// The ink with the more contrast on `background`, the light one on a tie (notation 8), and that contrast.
fn best((light, dark): (Rgb, Rgb), background: Rgb) -> (Token, f64) {
    let (on_light, on_dark) = (contrast(light, background), contrast(dark, background));
    if on_light >= on_dark { (INK_LIGHT, on_light) } else { (INK_DARK, on_dark) }
}

/// The ink with the more contrast on `background`, the light one on a tie (notation 8).
pub fn best_ink(background: Rgb) -> Token {
    best(ink_values(), background).0
}

/// The band of `color` (notation 8, rule F): the colour as written; when no ink holds 4.5:1 on it, its lightness (OKLCH)
/// moves toward the nearer end, by steps of 0.01, its chroma and hue kept, each channel rounded and held within 0 to 255.
pub fn band(color: Rgb) -> Rgb {
    let inks = ink_values();
    let holds = |background: Rgb| best(inks, background).1 >= TEXT_CONTRAST;
    if holds(color) {
        return color;
    }
    let (lightness, a, b) = oklab(color);
    let up = lightness >= 0.5;
    let mut step = 1;
    loop {
        let moved = lightness + if up { 0.01 } else { -0.01 } * f64::from(step);
        if !(0.0..=1.0).contains(&moved) {
            return if up { 0xFF_FFFF } else { 0 };
        }
        let found = from_oklab(moved, a, b);
        if holds(found) {
            return found;
        }
        step += 1;
    }
}

/// The tint of the body: `fill` mixed with `color` at 16 %, channel by channel, the share lowered by 1 % while `title`
/// holds less than 4.5:1 on it, down to none (notation 8).
pub fn tint(fill: Rgb, color: Rgb, title: Rgb) -> Rgb {
    (0..=TINT).rev().map(|share| mix(fill, color, share)).find(|&tint| contrast(title, tint) >= TEXT_CONTRAST).unwrap_or(fill)
}

/// `round(f + (c − f) × share / 100)` for each channel.
fn mix(fill: Rgb, color: Rgb, share: u32) -> Rgb {
    let channel = |shift: u32| {
        let (f, c) = (i64::from((fill >> shift) & 0xFF), i64::from((color >> shift) & 0xFF));
        // Between `f` and `c`, so never below 0 nor above 255: half up, on integers.
        let scaled = f * 100 + (c - f) * i64::from(share);
        u32::try_from((scaled + 50) / 100).unwrap_or(0)
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

fn encoded(value: f64) -> u32 {
    let c = if value <= 0.003_130_8 { value * 12.92 } else { 1.055 * value.powf(1.0 / 2.4) - 0.055 };
    // Within 0 to 255 after rounding: a cast of a float saturates.
    (c.clamp(0.0, 1.0) * 255.0).round() as u32
}

/// OKLab of an sRGB colour (Björn Ottosson, 2020): lightness and the two axes; chroma and hue are those of `(a, b)`.
fn oklab(color: Rgb) -> (f64, f64, f64) {
    let (r, g, b) = (linear(color >> 16), linear(color >> 8), linear(color));
    let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
    let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
    let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
    (
        0.210_454_255_3 * l + 0.793_617_785 * m - 0.004_072_046_8 * s,
        1.977_998_495_1 * l - 2.428_592_205 * m + 0.450_593_709_9 * s,
        0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766 * s,
    )
}

fn from_oklab(lightness: f64, a: f64, b: f64) -> Rgb {
    let l = (lightness + 0.396_337_777_4 * a + 0.215_803_757_3 * b).powi(3);
    let m = (lightness - 0.105_561_345_8 * a - 0.063_854_172_8 * b).powi(3);
    let s = (lightness - 0.089_484_177_5 * a - 1.291_485_548 * b).powi(3);
    let r = 4.076_741_662_1 * l - 3.307_711_591_3 * m + 0.230_969_929_2 * s;
    let g = -1.268_438_004_6 * l + 2.609_757_401_1 * m - 0.341_319_396_5 * s;
    let bl = -0.004_196_086_3 * l - 0.703_418_614_7 * m + 1.707_614_701 * s;
    (encoded(r) << 16) | (encoded(g) << 8) | encoded(bl)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Worked out by hand: white has 4.76:1 on `#64748B` and `#1F2328` 3.32:1 (luminances 0.1706 and 0.0164), so the light ink (notation 8).
    #[test]
    fn the_ink_of_the_primitives_is_the_light_one() {
        assert_eq!(best_ink(0x64_748B), INK_LIGHT);
        assert_eq!(best_ink(0xFF_F7D6), INK_DARK);
    }

    /// A colour on which an ink holds is kept as written; a mid grey, `#808080` (white 3.95:1, `#1F2328` 4.00:1), is
    /// moved until one does, and it stays a grey.
    #[test]
    fn a_band_is_the_colour_as_written_unless_no_ink_holds() {
        assert_eq!(band(0x2F_6F9F), 0x2F_6F9F);
        let moved = band(0x80_8080);
        assert_ne!(moved, 0x80_8080);
        assert!(best(ink_values(), moved).1 >= TEXT_CONTRAST);
        let [r, g, b] = [moved >> 16, (moved >> 8) & 0xFF, moved & 0xFF];
        assert!(r.abs_diff(g) <= 1 && g.abs_diff(b) <= 1, "{moved:06X}");
    }

    /// `#FFFFFF` mixed with `#2F6F9F` at 16 %: 255 − 208 × 0.16 = 221.72 → 222, 255 − 144 × 0.16 = 231.96 → 232,
    /// 255 − 96 × 0.16 = 239.64 → 240.
    #[test]
    fn the_tint_mixes_sixteen_percent_channel_by_channel() {
        assert_eq!(tint(0xFF_FFFF, 0x2F_6F9F, 0x1F_2328), 0xDE_E8F0);
        // A black body: the title, black too, never holds, so no tint.
        assert_eq!(tint(0, 0xFF_0000, 0), 0);
    }

    #[test]
    fn oklab_comes_back_to_the_colour() {
        for color in [0x00_0000, 0xFF_FFFF, 0x2F_6F9F, 0x64_748B, 0xBE_185D] {
            let (l, a, b) = oklab(color);
            assert_eq!(from_oklab(l, a, b), color, "{color:06X}");
        }
    }
}
