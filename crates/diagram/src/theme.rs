//! The theme tokens of notation 8, light and dark, as data: the SVG writes them as CSS variables, the canvas maps each to
//! a colour, and the tests read them for the contrasts of notation 8.

/// A token and its values, `#RRGGBB`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TokenValue {
    pub name: &'static str,
    pub light: &'static str,
    pub dark: &'static str,
}

const fn token(name: &'static str, light: &'static str, dark: &'static str) -> TokenValue {
    TokenValue { name, light, dark }
}

/// In the order of the table of notation 8.
pub const TOKENS: &[TokenValue] = &[
    token("canvas.fill", "#F7F7F5", "#16181C"),
    token("canvas.grid", "#E6E6E2", "#23262B"),
    token("node.fill", "#FFFFFF", "#20242A"),
    token("node.stroke", "#777D88", "#7B8390"),
    token("node.title", "#1F2328", "#ECEFF3"),
    token("node.active", "#15803D", "#4ADE80"),
    token("node.divider", "#E1E3E6", "#343A43"),
    token("frame.fill.1", "#F0F1F3", "#1C1F24"),
    token("frame.fill.2", "#E9EBEE", "#22262C"),
    token("frame.fill.3", "#E2E5E9", "#282D34"),
    token("frame.fill.4", "#DBDFE4", "#2E343C"),
    token("frame.stroke", "#777D88", "#7B8390"),
    token("frame.divider", "#C9CDD2", "#434A55"),
    token("wire.data", "#3D4148", "#C5CBD3"),
    token("wire.resource", "#1A63A4", "#5BA8F0"),
    token("wire.error", "#AD3A0B", "#FF8F5E"),
    token("wire.order", "#5C6168", "#A3ABB6"),
    token("mark.coercion", "#752FEC", "#B197FC"),
    token("mark.invalid", "#BD1E1E", "#FF6E6E"),
    token("coercion.fill", "#F1EAFE", "#2E2646"),
    token("ink.muted", "#5D6168", "#A3ABB6"),
    token("ink.faint", "#5D6168", "#979EAA"),
    token("pill.fill", "#F0F1F3", "#2B3038"),
    token("pill.stroke", "#777D88", "#7B8390"),
    token("pill.text", "#3D4148", "#C5CBD3"),
    token("probe.fill", "#FFF7D6", "#3A3419"),
    token("probe.stroke", "#777D88", "#7B8390"),
    token("badge.fill", "#1F2328", "#ECEFF3"),
    token("badge.text", "#FFFFFF", "#16181C"),
    token("tab.fill", "#E1E4E8", "#2B3038"),
    token("band.fill", "#E1E4E8", "#2B3038"),
    token("band.text", "#1F2328", "#ECEFF3"),
];

/// The name of the CSS variable of a token: `wire.error` gives `--wire-error` (notation 8).
pub fn variable(token: &str) -> String {
    format!("--{}", token.replace('.', "-"))
}

/// The contrasts of notation 8: every wire, mark and text token at 4.5:1 on the canvas and every frame level, every text
/// on its own fill at 4.5:1, every outline at 3:1; each as (ink, background, least ratio), for both themes.
pub fn contrasts() -> Vec<(&'static str, &'static str, f64)> {
    const BACKGROUNDS: [&str; 5] = ["canvas.fill", "frame.fill.1", "frame.fill.2", "frame.fill.3", "frame.fill.4"];
    const INKS: [&str; 10] = [
        "wire.data",
        "wire.resource",
        "wire.error",
        "wire.order",
        "mark.coercion",
        "mark.invalid",
        "node.title",
        "ink.muted",
        "ink.faint",
        "pill.text",
    ];
    const OUTLINES: [&str; 5] = ["node.stroke", "frame.stroke", "pill.stroke", "probe.stroke", "node.active"];
    const OWN: [(&str, &str); 11] = [
        ("node.title", "node.fill"),
        ("ink.muted", "node.fill"),
        ("ink.faint", "node.fill"),
        ("pill.text", "pill.fill"),
        ("badge.text", "badge.fill"),
        ("band.text", "band.fill"),
        ("node.title", "tab.fill"),
        ("node.title", "probe.fill"),
        ("ink.muted", "probe.fill"),
        ("wire.error", "probe.fill"),
        ("mark.invalid", "probe.fill"),
    ];
    let mut pairs = Vec::new();
    for background in BACKGROUNDS {
        pairs.extend(INKS.iter().map(|ink| (*ink, background, 4.5)));
        pairs.extend(OUTLINES.iter().map(|outline| (*outline, background, 3.0)));
    }
    pairs.extend(OWN.iter().map(|(ink, fill)| (*ink, *fill, 4.5)));
    pairs
}

/// The WCAG 2 contrast ratio of two colours `0xRRGGBB`, from its definition.
pub fn contrast(a: crate::style::Rgb, b: crate::style::Rgb) -> f64 {
    let luminance = |rgb: crate::style::Rgb| {
        let channel = |shift: u32| {
            let c = f64::from((rgb >> shift) & 0xFF) / 255.0;
            if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
    };
    let (a, b) = (luminance(a), luminance(b));
    let (light, dark) = if a > b { (a, b) } else { (b, a) };
    (light + 0.05) / (dark + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table is that of notation 8, read from the document: another value is a reviewed change of both.
    #[test]
    fn the_tokens_are_those_of_the_notation() {
        const NOTATION: &str = include_str!("../../../docs/visual-notation.md");
        let section = NOTATION.split("## 8. Theme tokens").nth(1).unwrap().split("## 9.").next().unwrap();
        let rows: Vec<(String, String, String)> = section
            .lines()
            .filter(|line| line.starts_with("| `"))
            .map(|line| {
                let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
                (cells[0].trim_matches('`').to_owned(), cells[1].trim_matches('`').to_owned(), cells[2].trim_matches('`').to_owned())
            })
            .collect();
        let ours: Vec<(String, String, String)> =
            TOKENS.iter().map(|token| (token.name.to_owned(), token.light.to_owned(), token.dark.to_owned())).collect();
        assert_eq!(ours, rows);
    }

    /// Notation 8, in both themes, on the defaults.
    #[test]
    fn the_contrasts_of_notation_8_hold() {
        assert_eq!(crate::style::contrast_problems(&crate::style::Style::default()), Vec::<String>::new());
        assert_eq!(contrasts().len(), 5 * 15 + 11);
    }

    /// The ratio of WCAG 2, by hand: white on black 21:1, a colour on itself 1:1, `#777777` on white 4.48:1.
    #[test]
    fn a_contrast_is_that_of_wcag() {
        assert!((contrast(0xFFFFFF, 0x000000) - 21.0).abs() < 1e-9);
        assert!((contrast(0x3D4148, 0x3D4148) - 1.0).abs() < 1e-9);
        assert!((contrast(0x777777, 0xFFFFFF) - 4.478).abs() < 0.001);
    }

    #[test]
    fn a_token_is_a_css_variable() {
        assert_eq!(variable("wire.error"), "--wire-error");
        assert_eq!(variable("frame.fill.1"), "--frame-fill-1");
    }
}
