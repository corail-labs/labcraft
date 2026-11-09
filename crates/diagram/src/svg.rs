//! The SVG of a drawing (`docs/cli.md`, 7.2): the same bytes on every platform and in WASM, numbers in their shortest
//! form with at most two decimals and no exponent, attributes in a fixed order, colours as CSS variables of the tokens.

use std::fmt::Write as _;

use crate::shape::{Drawing, IconItem, IconShape, Paint, Run, Shape};
use crate::theme::{TOKENS, variable};

/// The schema of the attributes, stable under this name (`docs/cli.md`, 4.4).
pub const SCHEMA: &str = "labcraft-diagram/1";
/// Notation 2.3: the font named once, on the root, not embedded.
pub const FONT_FAMILY: &str = "Hack, DejaVu Sans Mono, Menlo, monospace";
/// The stroke of an icon, in units of its grid: chosen by the IDE (spec 11.1).
const ICON_STROKE: &str = "1.5";

/// Which theme an SVG holds: both, under `prefers-color-scheme`, or one (`--theme`, cli.md 7.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    Both,
    Light,
    Dark,
}

/// `value / 10^scale` in its shortest decimal form, without an exponent.
pub fn decimal(value: i128, scale: u32) -> String {
    let negative = value < 0;
    let digits = value.unsigned_abs().to_string();
    let scale = scale as usize;
    let (whole, fraction) = if digits.len() > scale {
        let (whole, fraction) = digits.split_at(digits.len() - scale);
        (whole.to_owned(), fraction.to_owned())
    } else {
        ("0".to_owned(), format!("{digits:0>scale$}"))
    };
    let fraction = fraction.trim_end_matches('0');
    let sign = if negative && (whole != "0" || !fraction.is_empty()) { "-" } else { "" };
    if fraction.is_empty() { format!("{sign}{whole}") } else { format!("{sign}{whole}.{fraction}") }
}

/// A length in hundredths of a unit.
fn n(value: i64) -> String {
    decimal(i128::from(value), 2)
}

/// Text for XML: `<`, `>`, `&` and quotes escaped; what XML 1.0 cannot hold, even escaped, as its code `⟨U+0000⟩`.
pub fn escape(text: &str, out: &mut String) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            c if u32::from(c) < 0x20 || c == '\u{FFFE}' || c == '\u{FFFF}' => {
                out.push_str(&labcraft_types::text::code(c));
            }
            c => out.push(c),
        }
    }
}

fn attribute(name: &str, value: &str, out: &mut String) {
    out.push(' ');
    out.push_str(name);
    out.push_str("=\"");
    escape(value, out);
    out.push('"');
}

fn style(paint: &Paint) -> String {
    let fill = paint.fill.map_or_else(|| "none".to_owned(), |token| format!("var({})", variable(token)));
    match paint.stroke {
        Some(stroke) => format!("fill: {fill}; stroke: var({}); stroke-width: {}", variable(stroke), n(paint.width)),
        None => format!("fill: {fill}"),
    }
}

fn values(theme: impl Fn(&crate::theme::TokenValue) -> &'static str) -> String {
    TOKENS.iter().map(|token| format!("{}: {};", variable(token.name), theme(token))).collect::<Vec<_>>().join(" ")
}

/// The SVG of a drawing.
pub fn svg(drawing: &Drawing, theme: Theme) -> String {
    let mut out = String::new();
    let (width, height) = (n(drawing.width), n(drawing.height));
    out.push_str("<svg xmlns=\"http://www.w3.org/2000/svg\"");
    attribute("viewBox", &format!("0 0 {width} {height}"), &mut out);
    attribute("width", &width, &mut out);
    attribute("height", &height, &mut out);
    attribute("font-family", FONT_FAMILY, &mut out);
    attribute("data-schema", SCHEMA, &mut out);
    attribute("data-function", &drawing.function, &mut out);
    out.push_str(">\n<style>\n");
    match theme {
        Theme::Both => {
            let _ = writeln!(out, "svg {{ {} }}", values(|token| token.light));
            let _ = writeln!(out, "@media (prefers-color-scheme: dark) {{ svg {{ {} }} }}", values(|token| token.dark));
        }
        Theme::Light => {
            let _ = writeln!(out, "svg {{ {} }}", values(|token| token.light));
        }
        Theme::Dark => {
            let _ = writeln!(out, "svg {{ {} }}", values(|token| token.dark));
        }
    }
    out.push_str("text { direction: ltr; unicode-bidi: isolate; white-space: pre; }\n</style>\n");
    let _ = writeln!(out, "<rect width=\"{width}\" height=\"{height}\" style=\"fill: var(--canvas-fill)\"/>");
    let icons = icons(drawing, &mut out);
    for shape in &drawing.loose {
        write_shape(shape, &icons, &mut out);
    }
    for group in &drawing.groups {
        out.push_str("<g");
        attribute("data-node", &group.node, &mut out);
        out.push_str("><title>");
        escape(&group.title, &mut out);
        out.push_str("</title>\n");
        for shape in &group.shapes {
            write_shape(shape, &icons, &mut out);
        }
        out.push_str("</g>\n");
    }
    out.push_str("</svg>\n");
    out
}

fn runs(runs: &[Run], out: &mut String) {
    match runs {
        [one] => {
            let _ = write!(out, " style=\"fill: var({})\">", variable(one.token));
            escape(&one.text, out);
        }
        _ => {
            out.push('>');
            for run in runs {
                let _ = write!(out, "<tspan style=\"fill: var({})\">", variable(run.token));
                escape(&run.text, out);
                out.push_str("</tspan>");
            }
        }
    }
}

/// Each icon once, as a `<symbol>` that the nodes `<use>`, so that an SVG holds an icon once however many calls show
/// it. Its id is a hash of what it draws, so that two SVGs inlined in one page share a symbol when they draw the same
/// icon; two different icons of one SVG whose hashes meet are told apart by a suffix.
fn icons(drawing: &Drawing, out: &mut String) -> Vec<String> {
    if drawing.icons.is_empty() {
        return Vec::new();
    }
    out.push_str("<defs>\n");
    let mut ids = Vec::new();
    let mut written: std::collections::HashMap<u64, Vec<String>> = std::collections::HashMap::new();
    for icon in &drawing.icons {
        let mut body = String::new();
        attribute("viewBox", &format!("0 0 {} {}", icon.grid, icon.grid), &mut body);
        body.push_str(" overflow=\"hidden\">\n");
        for item in &icon.items {
            icon_item(item, &mut body);
        }
        let hash = fnv(body.as_bytes());
        let bodies = written.entry(hash).or_default();
        // Two functions may draw the same icon: one symbol for both.
        let (index, new) = match bodies.iter().position(|other| *other == body) {
            Some(index) => (index, false),
            None => (bodies.len(), true),
        };
        let id = if index == 0 { format!("icon-{hash:016x}") } else { format!("icon-{hash:016x}-{index}") };
        if new {
            let _ = writeln!(out, "<symbol id=\"{id}\"{body}</symbol>");
            bodies.push(body);
        }
        ids.push(id);
    }
    out.push_str("</defs>\n");
    ids
}

/// FNV-1a on 64 bits: the same on every platform and in WASM.
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3))
}

fn write_shape(shape: &Shape, icons: &[String], out: &mut String) {
    match shape {
        Shape::Rect { x, y, width, height, radius, paint } => {
            let _ = write!(out, "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"", n(*x), n(*y), n(*width), n(*height));
            if *radius > 0 {
                let _ = write!(out, " rx=\"{}\"", n(*radius));
            }
            let _ = writeln!(out, " style=\"{}\"/>", style(paint));
        }
        Shape::Circle { x, y, radius, paint } => {
            let _ = writeln!(out, "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" style=\"{}\"/>", n(*x), n(*y), n(*radius), style(paint));
        }
        Shape::Poly { points, closed, paint } => {
            let points: Vec<String> = points.iter().map(|(x, y)| format!("{},{}", n(*x), n(*y))).collect();
            let element = if *closed { "polygon" } else { "polyline" };
            let _ = writeln!(out, "<{element} points=\"{}\" style=\"{}\"/>", points.join(" "), style(paint));
        }
        Shape::Text { x, y, size, runs: text, .. } => {
            if text.is_empty() {
                return;
            }
            let _ = write!(out, "<text x=\"{}\" y=\"{}\" font-size=\"{}\"", n(*x), n(*y), decimal(i128::from(*size), 2));
            runs(text, out);
            out.push_str("</text>\n");
        }
        Shape::Icon { x, y, side, icon } => {
            let Some(id) = icons.get(*icon) else { return };
            let _ = writeln!(out, "<use href=\"#{id}\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/>", n(*x), n(*y), n(*side), n(*side));
        }
    }
}

fn icon_item(item: &IconItem, out: &mut String) {
    let token = variable(item.token);
    let paint = if item.fill {
        format!("fill: var({token}); stroke: var({token}); stroke-width: {ICON_STROKE}")
    } else {
        format!("fill: none; stroke: var({token}); stroke-width: {ICON_STROKE}")
    };
    match &item.shape {
        IconShape::Rect { x, y, width, height, radius } => {
            out.push_str("<rect");
            attribute("x", x, out);
            attribute("y", y, out);
            attribute("width", width, out);
            attribute("height", height, out);
            if let Some(radius) = radius {
                attribute("rx", radius, out);
            }
            attribute("style", &paint, out);
            out.push_str("/>\n");
        }
        IconShape::Circle { x, y, radius } => {
            out.push_str("<circle");
            attribute("cx", x, out);
            attribute("cy", y, out);
            attribute("r", radius, out);
            attribute("style", &paint, out);
            out.push_str("/>\n");
        }
        IconShape::Line { x1, y1, x2, y2 } => {
            out.push_str("<line");
            attribute("x1", x1, out);
            attribute("y1", y1, out);
            attribute("x2", x2, out);
            attribute("y2", y2, out);
            attribute("style", &paint, out);
            out.push_str("/>\n");
        }
        IconShape::Path { data } => {
            out.push_str("<path");
            attribute("d", data, out);
            attribute("style", &paint, out);
            out.push_str("/>\n");
        }
        IconShape::Text { x, y, size, runs: text } => {
            out.push_str("<text");
            attribute("x", x, out);
            attribute("y", y, out);
            attribute("font-size", size, out);
            out.push_str(" text-anchor=\"middle\"");
            runs(text, out);
            out.push_str("</text>\n");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_number_is_written_in_its_shortest_form() {
        assert_eq!(n(0), "0");
        assert_eq!(n(722), "7.22");
        assert_eq!(n(1200), "12");
        assert_eq!(n(1250), "12.5");
        assert_eq!(n(-5), "-0.05");
        assert_eq!(n(-500), "-5");
        assert_eq!(n(i64::MAX), "92233720368547758.07");
        assert_eq!(decimal(15, 1), "1.5");
        assert_eq!(decimal(7, 0), "7");
    }

    #[test]
    fn xml_holds_every_text() {
        let mut out = String::new();
        escape("a<b>&\"'\u{0}\u{FFFF}\t", &mut out);
        assert_eq!(out, "a&lt;b&gt;&amp;&quot;&apos;\u{27E8}U+0000\u{27E9}\u{27E8}U+FFFF\u{27E9}&#9;");
    }
}
