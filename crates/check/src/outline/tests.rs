//! Each expected value is worked out by hand from spec 11.2 and 11.4, in hundredths of a unit.

use labcraft_graph::{BlockSide, Outline};
use labcraft_types::{Code, Interner};

use super::{At, Drawn, Frame, Problem, Read, draw, read};

/// The section of a file of one function, read.
fn section(items: &str) -> (Outline, Interner) {
    let mut names = Interner::new();
    match labcraft_format::read_outline(items, &mut names) {
        Ok(outline) => (outline, names),
        Err(message) => panic!("{items}: {message}"),
    }
}

fn read_of(items: &str) -> (Read, Vec<Problem>) {
    let (outline, names) = section(items);
    read(&outline, &names)
}

fn codes(problems: &[Problem]) -> Vec<Code> {
    problems.iter().map(|(code, _, _)| *code).collect()
}

const U: i64 = 100;

/// `fn split(x: f64[V]) -> (a, b, c)` with `top 5` and `left middle x` (spec 11.4): 48 × 48, `x` at the middle place 3
/// (24), `a`, `b`, `c` on the right at 8, 16 and 24.
fn split<'t>() -> Frame<'t> {
    let at = |side, along: i64, name, output| At { side, along: along * U, name, output, anchor: true };
    Frame {
        width: 48 * U,
        height: 48 * U,
        terminals: vec![
            at(BlockSide::Left, 24, "x", false),
            at(BlockSide::Right, 8, "a", true),
            at(BlockSide::Right, 16, "b", true),
            at(BlockSide::Right, 24, "c", true),
        ],
        chamfer: 8 * U,
    }
}

fn drawn(items: &str, frame: &Frame<'_>) -> Drawn {
    let (read, problems) = read_of(items);
    assert!(problems.is_empty(), "{items}: {problems:?}");
    match draw(&read, frame) {
        Ok(Some(drawn)) => drawn,
        other => panic!("{items}: {other:?}"),
    }
}

fn refused(items: &str, frame: &Frame<'_>) -> Vec<Code> {
    let (read, problems) = read_of(items);
    assert!(problems.is_empty(), "{items}: {problems:?}");
    match draw(&read, frame) {
        Err(problems) => codes(&problems),
        other => panic!("{items}: {other:?}"),
    }
}

fn units(points: &[(i64, i64)]) -> Vec<(i64, i64)> {
    points.iter().map(|(x, y)| (x / U, y / U)).collect()
}

#[test]
fn the_funnel_of_the_spec_is_its_body_with_a_piece_at_each_right_terminal() {
    let found = drawn(
        "    path (16, 0) (w - 8, 0) (w - 8, h) (16, h) (4, x + 2) (4, x - 2)\n    right terminal { path (0, -2) (8, -2) (8, 2) (0, 2) }",
        &split(),
    );
    // The right edge runs down from (40, 0) to (40, 48): the pieces at 8, 16 and 24 in that order, each 8 outward.
    assert_eq!(
        units(&found.points),
        [
            (16, 0),
            (40, 0),
            (40, 6),
            (48, 6),
            (48, 10),
            (40, 10),
            (40, 14),
            (48, 14),
            (48, 18),
            (40, 18),
            (40, 22),
            (48, 22),
            (48, 26),
            (40, 26),
            (40, 48),
            (16, 48),
            (4, 26),
            (4, 22),
        ]
    );
    // `x` starts on the tip, 4 inside the box; `a`, `b`, `c` at the end of their pieces, on the box.
    assert_eq!(units(&found.starts), [(4, 24), (48, 8), (48, 16), (48, 24)]);
    assert_eq!((found.left, found.right, found.top), (0, 0, 0));
}

#[test]
fn the_d_cuts_its_right_corners_by_the_chamfer() {
    let found = drawn("    template d", &split());
    assert_eq!(units(&found.points), [(0, 0), (40, 0), (48, 8), (48, 40), (40, 48), (0, 48)]);
    assert_eq!(units(&found.starts), [(0, 24), (48, 8), (48, 16), (48, 24)]);
}

#[test]
fn template_block_and_a_refused_section_are_the_ordinary_block() {
    let (block, problems) = read_of("    template block");
    assert!(problems.is_empty());
    assert_eq!(draw(&block, &split()), Ok(None));
}

#[test]
fn a_section_that_cannot_be_read_as_one_is_an_error() {
    for items in [
        "    path (0, 0) (w, 0) (w, h)\n    path (0, 0) (w, 0) (0, h)",
        "    template d\n    template d",
        "    template d\n    path (0, 0) (w, 0) (w, h)",
        "    template d\n    right terminal { path (0, -2) (8, 0) (0, 2) }",
        "    template e",
        "    right terminal { path (0, -2) (8, 0) (0, 2) }",
        "    path (0, 0) (w, 0) (w, h)\n    right terminal { path (0, -2) (8, 0) (0, 2) }\n    right terminal { path (0, -2) (4, 0) (0, 2) }",
        "    path (0, 0) (w, 0) round 0 (w, h)",
        "    path (0, 0) (w, 0) round 17 (w, h)",
        "    path (0, 0) (1025, 0) (w, h)",
        "    path (0, 0) (w - 1025, 0) (w, h)",
    ] {
        let (read, problems) = read_of(items);
        assert_eq!(read, Read::Block, "{items}");
        assert_eq!(codes(&problems), [Code::Lc5016], "{items}");
    }
    // 65 corners, 17 points in a piece.
    let many: String = (0..65).map(|index| format!(" ({}, 0)", index % 40)).collect();
    assert_eq!(codes(&read_of(&format!("    path{many}")).1), [Code::Lc5016]);
    let points: String = (0..17).map(|index| format!(" ({}, {})", if index == 0 || index == 16 { 0 } else { 1 }, index / 5 - 2)).collect();
    assert_eq!(codes(&read_of(&format!("    path (0, 0) (w, 0) (w, h)\n    right terminal {{ path{points} }}")).1), [Code::Lc5016]);
    // The bounds hold: 1024, 16 and 64 corners read.
    assert!(read_of("    path (0, 0) (1024, 0) round 16 (w, h)").1.is_empty());
}

#[test]
fn a_piece_outside_its_bounds_is_a_warning_at_every_size() {
    for points in [
        // Its first or last point off the edge.
        "(1, -2) (8, 0) (0, 2)",
        "(0, -2) (8, 0) (-1, 2)",
        // Past 8 outward or inward, past 4 along.
        "(0, -2) (9, 0) (0, 2)",
        "(0, -2) (-9, 0) (0, 2)",
        "(0, -5) (8, 0) (0, 2)",
        // Back along.
        "(0, 2) (8, 0) (0, 3)",
    ] {
        let (read, problems) = read_of(&format!("    path (0, 0) (w, 0) (w, h) (0, h)\n    right terminal {{ path {points} }}"));
        assert_eq!(read, Read::Block, "{points}");
        assert_eq!(codes(&problems), [Code::Lc5017], "{points}");
    }
}

#[test]
fn the_guards_at_a_size() {
    for items in [
        // A corner outside the box 48 × 48.
        "    path (0, 0) (w + 1, 0) (w, h) (0, h)",
        "    path (0, 0) (w, 0) (w, 49) (0, h)",
        // An anchor that names no terminal it can: `a` is on the right, so it gives a height, not a first coordinate;
        // `zz` names none.
        "    path (0, 0) (a, 0) (w, h) (0, h)",
        "    path (0, 0) (w, zz) (w, h) (0, h)",
        // A body that crosses itself: a bow tie.
        "    path (0, 0) (w, h) (w, 0) (0, h)",
        // The right side has a piece and a terminal, but its edge is slanted: no straight edge 4 on each side of `a`.
        "    path (0, 0) (40, 0) (w, h) (0, h)\n    right terminal { path (0, -2) (8, 0) (0, 2) }",
        // A straight edge that stops 2 below `a` (at 8): not 4 on each side of it.
        "    path (0, 0) (w, 6) (w, h) (0, h)\n    right terminal { path (0, -2) (8, 0) (0, 2) }",
        // The left edge is 8 inside the box: the outline does not meet `x` within 4 of it.
        "    path (8, 0) (w, 0) (w, h) (8, h)",
        // A piece that reaches inward past the body: the left edge of a block 8 wide at the right.
        "    path (40, 0) (w, 0) (w, h) (40, h)\n    right terminal { path (0, -2) (-8, 0) (0, 2) }\n    left terminal { path (0, -2) (8, 0) (0, 2) }",
    ] {
        assert_eq!(refused(items, &split()), [Code::Lc5017], "{items}");
    }
}

#[test]
fn a_variadic_value_or_w_named_as_a_terminal_is_no_anchor() {
    let mut frame = split();
    if let Some(at) = frame.terminals.first_mut() {
        at.anchor = false;
    }
    assert_eq!(refused("    path (0, 0) (w, 0) (w, h) (0, h) (0, x)", &frame), [Code::Lc5017]);
    // `w` is always the width, even with a terminal of that name.
    let mut frame = split();
    if let Some(at) = frame.terminals.get_mut(1) {
        at.name = "w";
    }
    let found = drawn("    path (0, 0) (w, 0) (w, h) (0, h)", &frame);
    assert_eq!(units(&found.points), [(0, 0), (48, 0), (48, 48), (0, 48)]);
}

#[test]
fn two_pieces_at_places_8_apart_may_touch() {
    // Each piece spans 4 above and 4 below its place: at 8 and 16 they meet at 12, which is no crossing.
    let found = drawn("    path (0, 0) (w, 0) (w, h) (0, h)\n    right terminal { path (0, -4) (4, 0) (0, 4) }", &split());
    assert_eq!(units(&found.points), [(0, 0), (48, 0), (48, 4), (52, 8), (48, 12), (52, 16), (48, 20), (52, 24), (48, 28), (48, 48), (0, 48)]);
    assert_eq!(found.right, 4 * U);
}

#[test]
fn a_rounded_corner_is_an_arc_and_its_radius_is_reduced_to_half_the_shorter_edge() {
    let found = drawn("    path (0, 0) (w, 0) round 4 (w, h) (0, h)", &split());
    // The corner (48, 0) becomes an arc from (44, 0) to (48, 4).
    assert_eq!(found.points.get(1), Some(&(44 * U, 0)));
    assert!(found.points.contains(&(48 * U, 4 * U)), "{:?}", units(&found.points));
    // Through its middle, at 45° from its centre (44, 4): 4 × √2 / 2 = 2.83 from it on each axis.
    assert!(found.points.contains(&(4683, 117)), "{:?}", found.points);
    // On a block 6 wide, a radius of 16 is reduced to 3, half of 6.
    let narrow = Frame { width: 6 * U, terminals: Vec::new(), ..split() };
    let found = drawn("    path (0, 0) (w, 0) round 16 (w, h) (0, h)", &narrow);
    assert_eq!(found.points.get(1), Some(&(3 * U, 0)));
    assert!(found.points.contains(&(6 * U, 3 * U)), "{:?}", units(&found.points));
}

#[test]
fn an_inward_piece_moves_the_start_of_its_terminal_in() {
    // A notch of 4 on the left edge at `x`: `x` starts at its bottom, 4 inside.
    let found = drawn("    path (0, 0) (w, 0) (w, h) (0, h)\n    left terminal { path (0, -4) (-4, 0) (0, 4) }", &split());
    assert_eq!(found.starts.first(), Some(&(4 * U, 24 * U)));
    assert_eq!(found.left, 0);
}

#[test]
fn a_piece_that_crosses_itself_is_outside_its_bounds() {
    // A point that goes back along its row, and a spike that comes back on itself.
    for points in ["(0, -4) (4, 0) (8, 0) (2, 0) (0, 4)", "(0, 0) (8, 0) (0, 0)"] {
        let (read, problems) = read_of(&format!("    path (0, 0) (w, 0) (w, h) (0, h)\n    right terminal {{ path {points} }}"));
        assert_eq!(read, Read::Block, "{points}");
        assert_eq!(codes(&problems), [Code::Lc5017], "{points}");
    }
}

#[test]
fn two_pieces_in_a_row_that_fold_back_on_each_other_cross() {
    // `a` at 8 and `b` at 16 on the right edge (48): the first piece ends at (48, 12) after running back from (56, 12),
    // and the second starts there and runs out to (56, 12) again: they share the segment from 48 to 56 at 12.
    let refused = refused("    path (0, 0) (w, 0) (w, h) (0, h)\n    right terminal { path (0, -4) (8, -4) (8, 4) (0, 4) }", &split());
    assert_eq!(refused, [Code::Lc5017]);
}

#[test]
fn the_pieces_of_the_left_and_the_right_sides_in_one_row_are_judged_together() {
    // A block 8 wide, from 12 to 20, whose two pieces at the height 24 reach across it: the left one from 12 out to 4 and
    // in to 19, the right one from 20 out to 28 and in to 13. Each is in its bounds, and they cross.
    let at = |side, along: i64, name, output| At { side, along: along * U, name, output, anchor: true };
    let frame = Frame {
        width: 48 * U,
        height: 48 * U,
        terminals: vec![at(BlockSide::Left, 24, "x", false), at(BlockSide::Right, 24, "y", true)],
        chamfer: 8 * U,
    };
    let refused = refused(
        "    path (12, 0) (20, 0) (20, h) (12, h)\n    left terminal { path (0, -4) (8, -4) (8, 0) (-7, 0) (-7, 4) (0, 4) }\n    right terminal { path (0, -4) (8, -4) (8, 0) (-7, 0) (-7, 4) (0, 4) }",
        &frame,
    );
    assert_eq!(refused, [Code::Lc5017]);
}

#[test]
fn a_piece_sits_only_on_an_edge_that_faces_its_side() {
    // The left edge runs slanted from (0, 48) up to (20, 0): no vertical edge faces left at `x` (24), and the right edge,
    // which faces right, is not taken for it.
    let (read, problems) = read_of("    path (20, 0) (w, 0) (w, h) (0, h)\n    left terminal { path (0, -2) (4, 0) (0, 2) }");
    assert!(problems.is_empty());
    match draw(&read, &split()) {
        Err(problems) => {
            assert_eq!(codes(&problems), [Code::Lc5017]);
            assert!(problems.iter().all(|(_, message, _)| message.contains("no straight edge")), "{problems:?}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_anchor_is_a_terminal_of_its_axis_the_input_first() {
    // `x` is an input on the top side and an output on the right side: as a height, it is the output (at 16).
    let at = |side, along: i64, output| At { side, along: along * U, name: "x", output, anchor: true };
    let frame =
        Frame { width: 48 * U, height: 48 * U, terminals: vec![at(BlockSide::Top, 24, false), at(BlockSide::Right, 16, true)], chamfer: 8 * U };
    let found = drawn("    path (0, 0) (w, 0) (w, x) (w, h) (0, h)", &frame);
    assert!(found.points.contains(&(48 * U, 16 * U)), "{:?}", units(&found.points));
    // As a first coordinate, the input on the top side (at 24).
    let found = drawn("    path (0, 0) (x, 0) (w, 0) (w, h) (0, h)", &frame);
    assert!(found.points.contains(&(24 * U, 0)), "{:?}", units(&found.points));
}
