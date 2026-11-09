//! The rules of routing (`docs/visual-notation.md`, 2.2), checked on a drawing by a checker of their own, not by
//! comparing with an output (`docs/architecture.md`, 11.6). It reads only the shapes of the drawing:
//!
//! - no wire crosses a node, a label, a pill, a band or a wire name of its body, but its own pill and its own name;
//! - parallel wires of two values keep 8 units apart: no two share a segment, but the last one of two values that reach
//!   the same terminal;
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
    /// The terminal it reaches, for the last segment of a wire.
    reaches: Option<(i64, i64)>,
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
                // A structure: the texts of its frame. Those of a plain node are its own, kept from the wires of its
                // body by its boxes; a wire of another body meets them only where the positions of the file make the
                // node overlap a frame, and what the file writes wins (notation 2.1).
                let frame = group
                    .shapes
                    .iter()
                    .any(|shape| matches!(shape, Shape::Rect { paint, .. } if paint.fill.is_some_and(|fill| fill.starts_with("frame.fill"))));
                if let Some(placed) = nodes.next()
                    && frame
                {
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
                                // The way into its terminal: its last segment, and the one before when the last comes
                                // down onto a top edge (an `after:` of a compact block).
                                let end = points.last().copied();
                                let from_above = match points.len().checked_sub(2).and_then(|at| points.get(at)) {
                                    Some(&(x, y)) => end.is_some_and(|end| end.0 == x && end.1 > y),
                                    None => false,
                                };
                                for (index, pair) in points.windows(2).enumerate() {
                                    let (Some(&a), Some(&b)) = (pair.first(), pair.get(1)) else { continue };
                                    let a = if index == 0 { toward(a, b, TERMINAL) } else { a };
                                    let into = index + 1 == last || (from_above && index + 2 == last);
                                    let reaches = if into { end } else { None };
                                    let b = if index + 1 == last { toward(b, a, TERMINAL) } else { b };
                                    segments.push(Segment { a, b, net, reaches });
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
    // Two values that reach one terminal share their way into it: a value bound twice, which `check` refuses, or an input
    // that reads several values (notation 2.2).
    if one.reaches.is_some() && one.reaches == other.reaches {
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

    /// A library with a file that does not read: `check` types nothing, `t` is unknown, so no wire is an error and there
    /// is no axis. Found by the fuzzing: a jog took a height 4 from a value that crosses the channel straight.
    #[test]
    fn an_untyped_library_keeps_the_rules() {
        const FILES: [(&str, &str); 2] = [
            ("swap.lcf", include_str!("../routing-cases/swap_untyped.lcf")),
            (
                "broken.lcf",
                "labcraft 1

us
",
            ),
        ];
        let mut root = LibrarySource::new(String::new(), String::new(), Origin::Project);
        for (shown, text) in FILES {
            root.files.push(SourceFile { shown: shown.to_owned(), content: Content::Bytes(Cow::Borrowed(text.as_bytes())) });
        }
        let mut names = Interner::new();
        let checked = check(root, &Nothing, &mut names, &Limits::default());
        let mut faults = Vec::new();
        for index in 0..checked.files.len() {
            for standard_everywhere in [false, true] {
                let view = ViewState { standard_everywhere, ..ViewState::default() };
                if let Drawn::Drawing(drawing) = draw(&checked, index, &names, &view, &DiagramLimits::default()) {
                    faults.extend(super::faults(&drawing));
                }
            }
        }
        assert!(
            faults.is_empty(),
            "{}",
            faults.join(
                "
"
            )
        );
    }

    /// A call that reads the many outputs of another in the reverse order: a crossing of every value with every other in
    /// one channel, cycles everywhere, whose jogs find their heights far from the middle (found in review, at 90 values).
    #[test]
    fn many_values_read_in_reverse_keep_the_rules() {
        let count = 120;
        let params: Vec<String> = (0..count).map(|k| format!("x{k}: f64")).collect();
        let outputs: Vec<String> = (0..count).map(|k| format!("y{k}: f64")).collect();
        let body: String = (0..count)
            .map(|k| {
                format!(
                    "    y{k} = core.neg(x{k})
"
                )
            })
            .collect();
        let r = format!(
            "labcraft 1

use core

fn r({}) -> ({}) {{
{body}}}
",
            params.join(", "),
            outputs.join(", ")
        );
        let firsts: Vec<String> = (0..count).map(|k| format!("a{k}")).collect();
        let seconds: Vec<String> = (0..count).map(|k| format!("b{k}")).collect();
        let reversed: Vec<String> = (0..count).rev().map(|k| format!("a{k}")).collect();
        let inputs: Vec<String> = (0..count).map(|_| "x".to_owned()).collect();
        let main = format!(
            "labcraft 1

fn main(x: f64) -> () {{
    {} = r({})
    {} = r({})
}}
",
            firsts.join(", "),
            inputs.join(", "),
            seconds.join(", "),
            reversed.join(", ")
        );
        let mut root = LibrarySource::new(String::new(), String::new(), Origin::Project);
        for (shown, text) in [("r.lcf", r), ("main.lcf", main)] {
            root.files.push(SourceFile { shown: shown.to_owned(), content: Content::Bytes(Cow::Owned(text.into_bytes())) });
        }
        let mut names = Interner::new();
        let checked = check(root, &Nothing, &mut names, &Limits::default());
        let mut faults = Vec::new();
        for index in 0..checked.files.len() {
            for standard_everywhere in [false, true] {
                let view = ViewState { standard_everywhere, ..ViewState::default() };
                let Drawn::Drawing(drawing) = draw(&checked, index, &names, &view, &DiagramLimits::default()) else { panic!() };
                faults.extend(super::faults(&drawing));
            }
        }
        assert!(
            faults.is_empty(),
            "{}",
            faults.join(
                "
"
            )
        );
    }

    /// Every case of the corpora of acceptance (`tests/acceptance/hostile`, `type-errors`), checked as a folder and each
    /// function drawn in both views: what the fuzzing of `diagram` starts from.
    #[test]
    fn the_corpora_of_acceptance_keep_the_rules() {
        let mut faults = Vec::new();
        for case in crate::HOSTILE.iter().chain(crate::TYPE_ERRORS) {
            let mut root = LibrarySource::new(String::new(), String::new(), Origin::Project);
            for file in case.files {
                let shown = file.path.rsplit('/').next().unwrap_or(file.path).to_owned();
                root.files.push(SourceFile { shown, content: Content::Bytes(Cow::Borrowed(file.bytes)) });
            }
            let mut names = Interner::new();
            let checked = check(root, &Nothing, &mut names, &Limits::default());
            for index in 0..checked.files.len() {
                for standard_everywhere in [false, true] {
                    let view = ViewState { standard_everywhere, ..ViewState::default() };
                    if let Drawn::Drawing(drawing) = draw(&checked, index, &names, &view, &DiagramLimits::default()) {
                        faults.extend(super::faults(&drawing).into_iter().map(|fault| format!("{}: {fault}", case.name)));
                    }
                }
            }
        }
        assert!(
            faults.is_empty(),
            "{}",
            faults.join(
                "
"
            )
        );
    }

    /// Programs that the review of M0v step 4 built to break the rules (`tests/harness/routing-cases/`): two values
    /// swapped between two calls, a cycle of three constraints in a channel, one in a loop, an order wire from a label,
    /// an `after:` on the top edge of a labelled block; and six that the fuzzing found: a node that reads its own error,
    /// read on to the right too, an `err` input whose type is unknown, which is then no error, an output bound twice, a constant too many beside a wire on a circle, two order wires into one `after:` from above, a value read twice before it, by a cycle; one of the review: a track over a channel where a jog would go; and one that the fuzzing found on `main`: a loop that the file places over a node that it places too. Each is drawn in both views and checked, whatever `check` says of it (a cycle is an error
    /// there, and still a drawing).
    #[test]
    fn the_hard_cases_keep_the_rules() {
        const CASES: [(&str, &str); 15] = [
            ("s.lcf", include_str!("../routing-cases/s.lcf")),
            ("t.lcf", include_str!("../routing-cases/t.lcf")),
            ("swap.lcf", include_str!("../routing-cases/swap.lcf")),
            ("cycle_of_three.lcf", include_str!("../routing-cases/cycle_of_three.lcf")),
            ("cycle_in_loop.lcf", include_str!("../routing-cases/cycle_in_loop.lcf")),
            ("order_from_label.lcf", include_str!("../routing-cases/order_from_label.lcf")),
            ("after_on_labelled.lcf", include_str!("../routing-cases/after_on_labelled.lcf")),
            ("self_loop.lcf", include_str!("../routing-cases/self_loop.lcf")),
            ("unknown_error.lcf", include_str!("../routing-cases/unknown_error.lcf")),
            ("one_output_twice.lcf", include_str!("../routing-cases/one_output_twice.lcf")),
            ("pill_beside_a_wire.lcf", include_str!("../routing-cases/pill_beside_a_wire.lcf")),
            ("two_orders_into_one.lcf", include_str!("../routing-cases/two_orders_into_one.lcf")),
            ("back_twice.lcf", include_str!("../routing-cases/back_twice.lcf")),
            ("track_over_a_jog.lcf", include_str!("../routing-cases/track_over_a_jog.lcf")),
            ("projectile.lcf", include_str!("../routing-cases/loop_placed_left.lcf")),
        ];
        let mut root = LibrarySource::new(String::new(), String::new(), Origin::Project);
        for (shown, text) in CASES {
            root.files.push(SourceFile { shown: shown.to_owned(), content: Content::Bytes(Cow::Borrowed(text.as_bytes())) });
        }
        let mut names = Interner::new();
        let checked = check(root, &Nothing, &mut names, &Limits::default());
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
