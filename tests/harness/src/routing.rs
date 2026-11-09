//! The rules of routing (`docs/visual-notation.md`, 2.2), checked on a drawing by a checker of their own, not by
//! comparing with an output (`docs/architecture.md`, 11.6). It reads only the shapes of the drawing:
//!
//! - no wire crosses a node, a label, a pill, a band or a wire name of its body, but its own pill and its own name;
//! - parallel wires of two values keep 8 units apart: no two share a segment;
//! - nothing but error wires below the main chain of a body, which no other wire crosses.
//!
//! A body whose file places a node is not checked, nor a wire that the file gives waypoints: there the rules hold only
//! as far as they can (notation 2.1). The titles and the names of the terminals of the frames count as labels.

use labcraft_diagram::{Drawing, Group, Shape};
use labcraft_types::text::{ASCENT, DESCENT, scale};

/// Between parallel wires (notation 2.2).
const PARALLEL: i64 = 800;
/// How far a wire may run into the box of the node it reaches: a terminal of a circle is inside its square (notation
/// 3.2), and a terminal is 10 across.
const TERMINAL: i64 = 600;

#[derive(Clone, Copy, Debug)]
struct Segment {
    a: (i64, i64),
    b: (i64, i64),
    net: usize,
}

/// A box, its edges excluded.
#[derive(Clone, Copy, Debug)]
struct Box {
    x0: i64,
    y0: i64,
    x1: i64,
    y1: i64,
    /// The net it belongs to: a pill or a name of a wire.
    net: Option<usize>,
    what: &'static str,
    /// Whether a wire on its edge crosses it too: the block of a node.
    edges: bool,
}

impl Segment {
    fn horizontal(&self) -> bool {
        self.a.1 == self.b.1
    }

    fn crosses(&self, area: &Box) -> bool {
        let (x0, x1) = (self.a.0.min(self.b.0), self.a.0.max(self.b.0));
        let (y0, y1) = (self.a.1.min(self.b.1), self.a.1.max(self.b.1));
        if area.edges {
            return x0 <= area.x1 && x1 >= area.x0 && y0 <= area.y1 && y1 >= area.y0;
        }
        x0 < area.x1 && x1 > area.x0 && y0 < area.y1 && y1 > area.y0
    }
}

/// What the rules find wrong in a drawing, each said once.
pub fn faults(drawing: &Drawing) -> Vec<String> {
    let mut found = Vec::new();
    let bodies = drawing.bodies.len();
    // The texts of the frames: the function's, those of each structure inside its block, those of each arm.
    let mut frames: Vec<Box> = Vec::new();
    let mut texts = |shapes: &[Shape], inside: Option<&labcraft_diagram::Placed>| {
        for shape in shapes {
            if let Shape::Text { x, y, size, width, .. } = shape {
                let area = text_box(*x, *y, *size, *width, None, "a label of a frame");
                if inside.is_none_or(|placed| {
                    area.x0 >= placed.x && area.x1 <= placed.x + placed.width && area.y0 >= placed.y && area.y1 <= placed.y + placed.height
                }) {
                    frames.push(area);
                }
            }
        }
    };
    texts(&drawing.loose, None);
    let mut nodes = drawing.placed.iter();
    for group in &drawing.groups {
        match (&group.wire, &group.arm) {
            (Some(_), _) => {}
            (None, Some(_)) => texts(&group.shapes, None),
            (None, None) => {
                if let Some(placed) = nodes.next() {
                    texts(&group.shapes, Some(placed));
                }
            }
        }
    }
    for body in 0..bodies {
        if drawing.bodies.get(body).is_none_or(|of| of.from_file) {
            continue;
        }
        let axis = drawing.bodies.get(body).and_then(|of| of.axis);
        let mut segments = Vec::new();
        let mut boxes = Vec::new();
        let mut errors = Vec::new();
        // The groups of the nodes come in the order of `placed`.
        let mut placed = drawing.placed.iter();
        for (net, group) in drawing.groups.iter().enumerate() {
            match &group.wire {
                Some(wire) if wire.body == body && !wire.via => {
                    errors.push((net, wire.error, wire.value.clone()));
                    for shape in &group.shapes {
                        match shape {
                            Shape::Wire { points, .. } => {
                                // Its ends stop short of the terminals they reach.
                                let last = points.len().saturating_sub(1);
                                for (index, pair) in points.windows(2).enumerate() {
                                    let (Some(&a), Some(&b)) = (pair.first(), pair.get(1)) else { continue };
                                    let a = if index == 0 { toward(a, b, TERMINAL) } else { a };
                                    let b = if index + 1 == last { toward(b, a, TERMINAL) } else { b };
                                    segments.push(Segment { a, b, net });
                                }
                            }
                            Shape::Rect { x, y, width, height, .. } => {
                                boxes.push(Box { x0: *x, y0: *y, x1: x + width, y1: y + height, net: Some(net), what: "a pill", edges: false })
                            }
                            Shape::Text { x, y, size, width, .. } => boxes.push(text_box(*x, *y, *size, *width, Some(net), "a wire name")),
                            _ => {}
                        }
                    }
                }
                Some(_) => {}
                None if group.arm.is_some() => {}
                None => {
                    if let Some(node) = placed.next()
                        && node.body == body
                    {
                        boxes.extend(node_boxes(node, group));
                    }
                }
            }
        }
        let value = |net: usize| errors.iter().find(|(index, ..)| *index == net).map_or("?", |(_, _, value)| value.as_str());
        for segment in &segments {
            for area in boxes.iter().chain(&frames) {
                if area.net != Some(segment.net) && segment.crosses(area) {
                    found.push(format!("{}: the wire `{}` crosses {}", drawing.function, value(segment.net), area.what));
                    break;
                }
            }
        }
        for (index, one) in segments.iter().enumerate() {
            for other in segments.iter().skip(index + 1) {
                if one.net != other.net && too_close(one, other) {
                    found.push(format!(
                        "{}: the wires `{}` and `{}` run closer than 8, {:?}-{:?} and {:?}-{:?}",
                        drawing.function,
                        value(one.net),
                        value(other.net),
                        one.a,
                        one.b,
                        other.a,
                        other.b
                    ));
                }
            }
        }
        if let Some(axis) = axis {
            for segment in &segments {
                let error = errors.iter().any(|(net, error, _)| *net == segment.net && *error);
                if !error && segment.a.1.max(segment.b.1) > axis {
                    found.push(format!("{}: the wire `{}` goes below the main chain", drawing.function, value(segment.net)));
                }
            }
        }
    }
    found.sort();
    found.dedup();
    found
}

/// `from` moved `by` toward `to`, no further than `to`.
fn toward(from: (i64, i64), to: (i64, i64), by: i64) -> (i64, i64) {
    let length = (to.0 - from.0).abs() + (to.1 - from.1).abs();
    let by = by.min(length);
    (from.0 + (to.0 - from.0).signum() * by, from.1 + (to.1 - from.1).signum() * by)
}

fn text_box(x: i64, y: i64, size: u32, width: i64, net: Option<usize>, what: &'static str) -> Box {
    let height = scale(u64::from(ASCENT + DESCENT), size).unwrap_or(0);
    Box { x0: x, y0: y - height, x1: x + width, y1: y, net, what, edges: false }
}

/// What a node of the body shows that no wire may cross: its block, its texts outside it (a label, a band), its pills
/// and its band. The frame of a structure is its block: what it holds is in other bodies.
fn node_boxes(placed: &labcraft_diagram::Placed, group: &Group) -> Vec<Box> {
    let block = Box { x0: placed.x, y0: placed.y, x1: placed.x + placed.width, y1: placed.y + placed.height, net: None, what: "a node", edges: true };
    let mut boxes = vec![block];
    let inside =
        |x0: i64, y0: i64, x1: i64, y1: i64| x0 >= placed.x && x1 <= placed.x + placed.width && y0 >= placed.y && y1 <= placed.y + placed.height;
    for shape in &group.shapes {
        match shape {
            Shape::Text { x, y, size, width, .. } => {
                let area = text_box(*x, *y, *size, *width, None, "a label");
                if !inside(area.x0, area.y0, area.x1, area.y1) {
                    boxes.push(area);
                }
            }
            // A pill of an inline constant, a band, a badge: not a terminal, such as the box of a register.
            Shape::Rect { x, y, width, height, paint, .. }
                if !inside(*x, *y, x + width, y + height) && matches!(paint.fill, Some("pill.fill" | "band.fill" | "badge.fill")) =>
            {
                boxes.push(Box { x0: *x, y0: *y, x1: x + width, y1: y + height, net: None, what: "a pill or a band", edges: false })
            }
            _ => {}
        }
    }
    boxes
}

/// Two segments of two values that run along each other closer than 8, or cross on one line.
fn too_close(one: &Segment, other: &Segment) -> bool {
    if one.horizontal() != other.horizontal() {
        return false;
    }
    let span = |segment: &Segment| -> ((i64, i64), i64) {
        if segment.horizontal() {
            ((segment.a.0.min(segment.b.0), segment.a.0.max(segment.b.0)), segment.a.1)
        } else {
            ((segment.a.1.min(segment.b.1), segment.a.1.max(segment.b.1)), segment.a.0)
        }
    };
    let ((a0, a1), a) = span(one);
    let ((b0, b1), b) = span(other);
    (a - b).abs() < PARALLEL && a0 < b1 && b0 < a1
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use labcraft_check::{Content, Found, Libraries, LibrarySource, Origin, SourceFile, check};
    use labcraft_diagram::{DiagramLimits, Drawn, ViewState, draw};
    use labcraft_types::{Interner, Limits};

    struct Nothing;

    impl Libraries for Nothing {
        fn find(&self, _: &str, _: Origin) -> Found {
            Found::NotFound
        }
    }

    /// Programs that the review of M0v step 4 built to break the rules (`tests/harness/routing-cases/`): two values
    /// swapped between two calls, a cycle of three constraints in a channel, one in a loop, an order wire from a label,
    /// an `after:` on the top edge of a labelled block. Each is drawn in both views and checked.
    #[test]
    fn the_hard_cases_keep_the_rules() {
        const CASES: [(&str, &str); 7] = [
            ("s.lcf", include_str!("../routing-cases/s.lcf")),
            ("t.lcf", include_str!("../routing-cases/t.lcf")),
            ("swap.lcf", include_str!("../routing-cases/swap.lcf")),
            ("cycle_of_three.lcf", include_str!("../routing-cases/cycle_of_three.lcf")),
            ("cycle_in_loop.lcf", include_str!("../routing-cases/cycle_in_loop.lcf")),
            ("order_from_label.lcf", include_str!("../routing-cases/order_from_label.lcf")),
            ("after_on_labelled.lcf", include_str!("../routing-cases/after_on_labelled.lcf")),
        ];
        let mut root = LibrarySource::new(String::new(), String::new(), Origin::Project);
        for (shown, text) in CASES {
            root.files.push(SourceFile { shown: shown.to_owned(), content: Content::Bytes(Cow::Borrowed(text.as_bytes())) });
        }
        let mut names = Interner::new();
        let checked = check(root, &Nothing, &mut names, &Limits::default());
        // Warnings are what the generator left (an error wired nowhere); none is an error.
        let errors: Vec<_> = checked.report.diagnostics().iter().filter(|found| found.severity() == labcraft_types::Severity::Error).collect();
        assert!(errors.is_empty(), "{errors:?}");
        let mut faults = Vec::new();
        for (index, file) in checked.files.iter().enumerate() {
            for standard_everywhere in [false, true] {
                let view = ViewState { standard_everywhere, ..ViewState::default() };
                let Drawn::Drawing(drawing) = draw(&checked, index, &names, &view, &DiagramLimits::default()) else { panic!("{}", file.shown) };
                faults.extend(super::faults(&drawing));
            }
        }
        assert!(faults.is_empty(), "{}", faults.join("\n"));
    }
}
