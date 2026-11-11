//! What a diagnostic shows in an SVG (`docs/cli.md`, 7.1 and 7.2): the marks of its errors on a function that is drawn,
//! and, for one that is not, its frame with its diagnostics in place of the drawing.

use labcraft_types::{Diagnostic, MAX_TEXT, Severity, text};

use crate::shape::{Drawing, Mark, Paint, Run, Shape, WireStyle, token_of};
use crate::size::Text;
use crate::style;

/// The diagnostics that a function that is not drawn lists, at most *(proposed)*.
pub const MAX_LINES: usize = 32;

/// A function that is not drawn: its frame with its name, and its diagnostics, one per line, as the first line of a
/// report writes them (`docs/cli.md`, 4.1), each cut at 200 code points, at most [`MAX_LINES`], then how many are left
/// out of `total`. The place is in `ink.muted`, the severity and the code in `mark.invalid` for an error and in
/// `node.title` for a warning, the message in `node.title`. Each line carries its code.
pub fn undrawn(function: &str, diagnostics: &[Diagnostic], total: u64) -> Drawing {
    let title = Text::new(function, crate::size::any_text(), crate::size::large());
    let (x, y) = (style::margin(), style::margin());
    let mut lines: Vec<(String, Shape)> = Vec::new();
    for (index, diagnostic) in diagnostics.iter().take(MAX_LINES).enumerate() {
        let middle = y + style::frame_header() + style::grid() + style::mark_line() * index as i64;
        lines.push((diagnostic.code().as_str().to_owned(), line(diagnostic, x + style::label_start(), middle)));
    }
    let left_out = total.saturating_sub(lines.len() as u64);
    let mut loose = Vec::new();
    let mut widest = title.width;
    for (_, shape) in &lines {
        if let Shape::Text { width, .. } = shape {
            widest = widest.max(*width);
        }
    }
    let more = (left_out > 0).then(|| {
        let text = Text::new(&format!("{left_out} more"), u64::MAX, crate::size::medium());
        widest = widest.max(text.width);
        text.at(x + style::label_start(), y + style::frame_header() + style::grid() + style::mark_line() * lines.len() as i64, "ink.muted")
    });
    let rows = lines.len() as i64 + i64::from(more.is_some());
    let width = widest + 2 * style::label_start();
    let height = style::frame_header() + style::mark_line() * rows.max(1) + style::grid();
    loose.push(Shape::Rect { x, y, width, height, radius: style::radius(), paint: Paint::both("frame.fill.1", "frame.stroke", style::outline()) });
    loose.push(title.at(x + style::label_start(), y + style::frame_header() / 2, "node.title"));
    loose.extend(more);
    Drawing {
        function: function.to_owned(),
        width: x + width + style::margin(),
        height: y + height + style::margin(),
        loose,
        marks: lines.into_iter().map(|(code, shape)| Mark { code, shapes: vec![shape] }).collect(),
        undrawn: true,
        style: style::current(),
        ..Drawing::default()
    }
}

/// One line of a diagnostic, in three parts: its place, its severity and code, its message.
fn line(diagnostic: &Diagnostic, x: i64, middle: i64) -> Shape {
    let place = match diagnostic.span {
        Some(span) => format!("{}:{}:{}: ", diagnostic.file, span.start.line, span.start.column),
        None => format!("{}: ", diagnostic.file),
    };
    let (severity, token) = match diagnostic.severity() {
        Severity::Error => ("error", "mark.invalid"),
        Severity::Warning => ("warning", "node.title"),
    };
    let parts =
        [(place, "ink.muted"), (format!("{severity}[{}]", diagnostic.code().as_str()), token), (format!(": {}", diagnostic.message), "node.title")];
    // Cut at 200 code points as a report cuts (`docs/cli.md`, 4.2): 199 of them, and `…`.
    let total: usize = parts.iter().map(|(part, _)| part.chars().count()).sum();
    let mut budget = if total > MAX_TEXT { MAX_TEXT - 1 } else { usize::MAX };
    let mut runs = Vec::new();
    let mut cells = 0u64;
    for (part, token) in parts {
        if budget == 0 {
            break;
        }
        let kept: String = part.chars().take(budget).collect();
        budget = budget.saturating_sub(kept.chars().count());
        let cut = budget == 0 && total > MAX_TEXT;
        let kept = if cut { format!("{kept}\u{2026}") } else { kept };
        let shown = text::show(&kept, u64::MAX);
        cells = cells.saturating_add(shown.cells);
        runs.extend(shown.runs.into_iter().map(|(ink, text)| Run { token: token_of(ink, token), text }));
    }
    let width = text::width(cells, crate::size::medium()).unwrap_or(i64::MAX / 4);
    let cap = text::scale(u64::from(text::CAP_HEIGHT), crate::size::medium()).unwrap_or(0);
    Shape::Text { x, y: middle + (cap + 1) / 2, size: crate::size::medium(), width, runs, detail: crate::Detail::Title }
}

/// The marks of the errors of `diagnostics` on a drawing (`docs/cli.md`, 7.1): each wire at fault drawn again in
/// `mark.invalid` (notation 4.2), each node at fault outlined in it, and the frame of the function for an error that
/// names neither. Each mark carries its code. A warning has no mark: the report lists it.
pub fn mark(drawing: &mut Drawing, diagnostics: &[Diagnostic]) {
    let mut marks = Vec::new();
    for diagnostic in diagnostics.iter().filter(|diagnostic| diagnostic.severity() == Severity::Error) {
        let mut shapes = Vec::new();
        for wire in &diagnostic.wires {
            match &wire.value {
                // The path of the reader it names, from the source; else the first one, from its source.
                Some(value) => {
                    for group in &drawing.groups {
                        let Some(of) = &group.wire else { continue };
                        if of.value != *value || (wire.to.is_none() && wire.from.is_some() && wire.from != of.from) {
                            continue;
                        }
                        let path = match &wire.to {
                            Some(to) => of.readers.iter().find(|(form, _)| form.as_ref() == Some(to)),
                            None => of.readers.first(),
                        };
                        if let Some((_, points)) = path {
                            shapes.push(Shape::Wire { points: points.clone(), style: invalid(group) });
                        }
                    }
                }
                // A terminal alone: the node that has it.
                None => {
                    let terminal = wire.to.as_ref().or(wire.from.as_ref());
                    let node = terminal.and_then(|terminal| terminal.rsplit_once('.')).map(|(node, _)| node);
                    shapes.extend(node.and_then(|node| outline(drawing, node)));
                }
            }
        }
        if shapes.is_empty() {
            let named = diagnostic.node.as_ref().and_then(|node| node.name.clone());
            let node = named.or_else(|| diagnostic.span.map(|span| format!("@{}", span.start.line)));
            shapes.extend(node.and_then(|node| outline(drawing, &node)));
        }
        if shapes.is_empty() {
            shapes.extend(drawing.loose.iter().find_map(|shape| match shape {
                Shape::Rect { x, y, width, height, radius, .. } => Some(Shape::Rect {
                    x: *x,
                    y: *y,
                    width: *width,
                    height: *height,
                    radius: *radius,
                    paint: Paint::stroke("mark.invalid", style::mark_width()),
                }),
                _ => None,
            }));
        }
        marks.push(Mark { code: diagnostic.code().as_str().to_owned(), shapes });
    }
    drawing.marks.extend(marks);
}

/// The style of a wire of this group in `mark.invalid`, its pattern kept.
fn invalid(group: &crate::shape::Group) -> WireStyle {
    let style = group.shapes.iter().find_map(|shape| match shape {
        Shape::Wire { style, .. } => Some(*style),
        _ => None,
    });
    let (width, pattern) =
        style.map_or((style::mark_width(), crate::shape::Pattern::Solid), |style| (style.width.max(style::mark_width()), style.pattern));
    let ground = style.map_or("canvas.fill", |style| style.ground);
    WireStyle { token: "mark.invalid", width, pattern, core: None, ticks: 0, ground }
}

/// An outline around the block of the node of that name, if the drawing has it.
fn outline(drawing: &Drawing, node: &str) -> Option<Shape> {
    let nodes = drawing.groups.iter().filter(|group| group.wire.is_none() && group.arm.is_none());
    let placed = nodes.zip(&drawing.placed).find(|(group, _)| group.node == node).map(|(_, placed)| placed)?;
    Some(Shape::Rect {
        x: placed.x - style::mark_around(),
        y: placed.y - style::mark_around(),
        width: placed.width + 2 * style::mark_around(),
        height: placed.height + 2 * style::mark_around(),
        radius: style::radius() + style::mark_around(),
        paint: Paint::stroke("mark.invalid", style::mark_width()),
    })
}

#[cfg(test)]
mod tests {
    use labcraft_types::{Code, Position, Span};

    use super::*;

    fn at(line: u32, column: u32) -> Span {
        Span { start: Position { line, column }, end: Position { line, column: column + 1 } }
    }

    fn texts(shape: &Shape) -> Vec<(&str, &str)> {
        match shape {
            Shape::Text { runs, .. } => runs.iter().map(|run| (run.token, run.text.as_str())).collect(),
            _ => Vec::new(),
        }
    }

    #[test]
    fn a_line_reads_as_the_first_line_of_a_report() {
        let diagnostic = Diagnostic::new(Code::Lc2001, "unknown value `x`", "broken.lcf").with_span(at(4, 9));
        let shape = line(&diagnostic, 0, 0);
        assert_eq!(texts(&shape), [("ink.muted", "broken.lcf:4:9: "), ("mark.invalid", "error[LC2001]"), ("node.title", ": unknown value `x`")]);
        let warning = Diagnostic::new(Code::Lc4101, "m", "a.lcf");
        assert_eq!(texts(&line(&warning, 0, 0)), [("ink.muted", "a.lcf: "), ("node.title", "warning[LC4101]"), ("node.title", ": m")]);
    }

    #[test]
    fn a_long_line_is_cut_at_200_code_points() {
        let diagnostic = Diagnostic::new(Code::Lc2001, "é".repeat(300), "a.lcf");
        let shape = line(&diagnostic, 0, 0);
        let whole: String = texts(&shape).iter().map(|(_, text)| *text).collect();
        assert_eq!(whole.chars().count(), 200);
        assert!(whole.ends_with('\u{2026}') && whole.starts_with("a.lcf: error[LC2001]: é"));
    }

    #[test]
    fn a_function_that_is_not_drawn_lists_32_diagnostics_and_how_many_are_left_out() {
        let diagnostics: Vec<Diagnostic> = (1..=40).map(|line| Diagnostic::new(Code::Lc1001, "m", "a.lcf").with_span(at(line, 1))).collect();
        let drawing = undrawn("a", &diagnostics, 41);
        assert!(drawing.undrawn);
        assert_eq!(drawing.marks.len(), MAX_LINES);
        assert!(drawing.marks.iter().all(|mark| mark.code == "LC1001"));
        let more: Vec<_> = drawing.loose.iter().flat_map(texts).collect();
        assert!(more.contains(&("ink.muted", "9 more")), "{more:?}");
        assert!(more.contains(&("node.title", "a")));
    }
}
