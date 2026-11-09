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

#[cfg(test)]
mod tests {
    use super::*;

    fn value(name: &str, dark: bool) -> &'static str {
        let found = TOKENS.iter().find(|token| token.name == name).unwrap_or_else(|| panic!("{name}"));
        if dark { found.dark } else { found.light }
    }

    /// WCAG 2 relative luminance and contrast ratio, from their definitions.
    fn luminance(hex: &str) -> f64 {
        let channel = |at: usize| {
            let byte = u8::from_str_radix(hex.get(at..at + 2).unwrap(), 16).unwrap();
            let c = f64::from(byte) / 255.0;
            if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5)
    }

    fn contrast(a: &str, b: &str) -> f64 {
        let (a, b) = (luminance(a), luminance(b));
        let (light, dark) = if a > b { (a, b) } else { (b, a) };
        (light + 0.05) / (dark + 0.05)
    }

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

    /// Notation 8: every wire, mark and text token at 4.5:1 on the canvas and every frame level, every text on its own fill
    /// at 4.5:1, every outline at 3:1, in both themes.
    #[test]
    fn the_contrasts_of_notation_8_hold() {
        let backgrounds = ["canvas.fill", "frame.fill.1", "frame.fill.2", "frame.fill.3", "frame.fill.4"];
        let inks = [
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
        let outlines = ["node.stroke", "frame.stroke", "pill.stroke", "probe.stroke", "node.active"];
        let own = [
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
        for dark in [false, true] {
            for background in backgrounds {
                for ink in inks {
                    let ratio = contrast(value(ink, dark), value(background, dark));
                    assert!(ratio >= 4.5, "{ink} on {background}, dark {dark}: {ratio:.2}");
                }
                for outline in outlines {
                    let ratio = contrast(value(outline, dark), value(background, dark));
                    assert!(ratio >= 3.0, "{outline} on {background}, dark {dark}: {ratio:.2}");
                }
            }
            for (ink, fill) in own {
                let ratio = contrast(value(ink, dark), value(fill, dark));
                assert!(ratio >= 4.5, "{ink} on {fill}, dark {dark}: {ratio:.2}");
            }
        }
    }

    #[test]
    fn a_token_is_a_css_variable() {
        assert_eq!(variable("wire.error"), "--wire-error");
        assert_eq!(variable("frame.fill.1"), "--frame-fill-1");
    }
}
