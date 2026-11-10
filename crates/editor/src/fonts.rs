//! The font of the notation (notation 2.3): Hack, embedded, first in a family of its own, where the host adds its
//! fallbacks after it (architecture 11.5).

use egui::FontFamily;
use egui::epaint::text::{FontData, FontInsert, FontPriority, InsertFontFamily};

/// The name of the family of the notation in the fonts of the host.
pub const FAMILY: &str = "labcraft-notation";

/// The name of the face of Hack in the fonts of the host.
const HACK_NAME: &str = "labcraft-hack";

/// Hack 3.003 Regular, whose SHA-256 a test of `types` checks (architecture 11.6).
static HACK: &[u8] = include_bytes!("../../../assets/fonts/Hack-Regular.ttf");

pub fn family() -> FontFamily {
    FontFamily::Name(FAMILY.into())
}

/// Adds Hack, first, to the family of the notation; it takes effect at the next pass. A host that sets its fonts
/// calls it after them, or adds the family itself, Hack first.
pub fn install_fonts(ctx: &egui::Context) {
    ctx.add_font(FontInsert {
        name: HACK_NAME.to_owned(),
        data: FontData::from_static(HACK),
        families: vec![InsertFontFamily { family: family(), priority: FontPriority::Highest }],
    });
}

/// Whether the fonts of this pass have the family of the notation: epaint panics on a family with no face.
pub fn has_family(ctx: &egui::Context) -> bool {
    ctx.fonts(|fonts| fonts.definitions().families.get(&family()).is_some_and(|faces| !faces.is_empty()))
}
