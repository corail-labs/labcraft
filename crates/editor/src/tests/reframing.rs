//! Nodes dragged over another frame (notation 2.4, "Moving nodes into another frame"; notation 2.5, "A frame that would
//! take a drag"; #208). What is expected is worked out by hand from those rules and commands.md 6.7 (`node.reframe`): the
//! frame is the deepest one under the pointer, the header and the edges of a structure being the frame around it; the
//! first node lands on the grid of 8 of that frame, counted from its origin; Shift keeps the nodes in their frame, and
//! the setting "drag between frames" on `stay` swaps the two.

use super::*;

const ARM: &str = "labcraft 1

use core

fn main(x: f64, on: bool) -> (y: f64, z: f64) {
    o = core.mul(x, 2.0)
    y = match on {
        true => {
            s = core.add(x, o)
            yield s
        }
        false => yield x
    }
    z = core.add(y, o)
}
";

/// A drag from `from` to `to`, with `modifiers` held, over frames from `time`, the host showing `shown`: what the canvas
/// painted while the pointer was at `to`, before the release, and what the drag asks for.
fn drag_held(
    ctx: &egui::Context,
    canvas: &mut Canvas,
    (from, to): (Pos2, Pos2),
    time: f64,
    modifiers: egui::Modifiers,
    shown: &Shown,
) -> (Vec<ClippedShape>, Vec<Intent>) {
    let button = |at, pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers };
    let mut intents = Vec::new();
    let mut over = Vec::new();
    let steps = [
        vec![egui::Event::ModifiersChanged(modifiers), egui::Event::PointerMoved(from), button(from, true)],
        vec![egui::Event::PointerMoved(from + (to - from) / 2.0)],
        vec![egui::Event::PointerMoved(to)],
        vec![egui::Event::PointerMoved(to), button(to, false)],
        vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)],
    ];
    for (at, events) in steps.into_iter().enumerate() {
        let (shapes, _, asked) = frame_shown(ctx, canvas, time + at as f64 * 0.1, events, shown);
        if at == 2 {
            over = shapes;
        }
        intents.extend(asked);
    }
    (over, intents)
}

/// The body of the arm `arm` of the structure named `structure`.
fn arm_body(drawing: &Drawing, structure: &str, arm: &str) -> usize {
    let owner = drawing.placed.iter().position(|placed| placed.name == structure).unwrap();
    drawing.bodies.iter().position(|body| body.owner == Some(owner) && body.arm == arm).unwrap()
}

/// The centre of the frame of a body, in units.
fn body_centre(drawing: &Drawing, body: usize) -> Pos2 {
    let (x, y, width, height) = drawing.bodies[body].frame;
    Pos2::new((x + width / 2) as f32 / 100.0, (y + height / 2) as f32 / 100.0)
}

/// Where the drop puts the node `name` dragged by `delta` into `body`, in units from the origin of that body: its corner
/// there, rounded to units, plus the drag in whole steps of 8, on the nearest point of the grid of 8 (the larger on a tie).
fn landing(drawing: &Drawing, name: &str, body: usize, delta: Vec2) -> (i64, i64) {
    let placed = drawing.placed.iter().find(|placed| placed.name == name).unwrap();
    let origin = drawing.bodies[body].origin;
    let unit = |hundredths: i64| (hundredths + 50).div_euclid(100);
    let step = |value: f32| ((value / 8.0).round() * 8.0) as i64;
    let grid = |value: i64| (value + 4).div_euclid(8) * 8;
    (grid(unit(placed.x - origin.0) + step(delta.x)), grid(unit(placed.y + placed.over - origin.1) + step(delta.y)))
}

fn reframe_of(nodes: &[&str], inside: Option<&str>, arm: Option<&str>, at: (i64, i64)) -> Intent {
    Intent::Reframe {
        nodes: nodes.iter().map(|name| (*name).to_owned()).collect(),
        within: InFrame { inside: inside.map(str::to_owned), arm: arm.map(str::to_owned) },
        at,
        through: Vec::new(),
    }
}

fn setup(text: &str) -> (egui::Context, Canvas, Arc<Drawing>) {
    let ctx = context();
    install_fonts(&ctx);
    let mut canvas = Canvas::new();
    let drawn = drawing(text, &ViewState::default());
    canvas.set_drawing(drawn.clone(), false);
    canvas.set_viewport(1.0, Vec2::ZERO);
    frame_shown(&ctx, &mut canvas, 0.0, Vec::new(), &Shown::default());
    (ctx, canvas, drawn)
}

/// Case B of the sheet: `o`, dropped in the arm `true` of `y`, goes into it, on its grid; with Shift it stays in its frame,
/// moved; with the setting on `stay`, the reverse.
#[test]
fn a_node_dropped_over_an_arm_goes_into_it_and_shift_keeps_it_in_its_frame() {
    let (ctx, mut canvas, drawn) = setup(ARM);
    let body = arm_body(&drawn, "y", "true");
    let (from, to) = (centre_of(&drawn, "o"), body_centre(&drawn, body));
    let into = reframe_of(&["o"], Some("y"), Some("true"), landing(&drawn, "o", body, to - from));
    let none = Shown::default();
    let (over, intents) = drag_held(&ctx, &mut canvas, (from, to), 0.2, egui::Modifiers::NONE, &none);
    assert_eq!(intents, vec![selecting(&["o"], &[], &[]), into.clone()]);
    // While the pointer is over it, the arm is outlined, and no reason shows.
    assert!(count(&over, |shape| matches!(shape, Painted::Rect(rect) if rect.stroke.width > 0.0)) > 0);
    canvas.cancel_move();
    let selected = showing(&selecting(&["o"], &[], &[]));
    let (_, intents) = drag_held(&ctx, &mut canvas, (from, to), 1.0, egui::Modifiers::SHIFT, &selected);
    assert!(matches!(intents.as_slice(), [Intent::Move { nodes, .. }] if nodes == &["o"]), "{intents:?}");
    canvas.cancel_move();
    let stay = Shown { stay_between_frames: true, ..selected.clone() };
    let (_, intents) = drag_held(&ctx, &mut canvas, (from, to), 2.0, egui::Modifiers::NONE, &stay);
    assert!(matches!(intents.as_slice(), [Intent::Move { .. }]), "{intents:?}");
    canvas.cancel_move();
    let (_, intents) = drag_held(&ctx, &mut canvas, (from, to), 3.0, egui::Modifiers::SHIFT, &stay);
    assert_eq!(intents, vec![into]);
}

/// The header of a structure belongs to the frame around it: `o` dropped there stays in its frame, moved.
#[test]
fn the_header_of_a_structure_is_the_frame_around_it() {
    let (ctx, mut canvas, drawn) = setup(ARM);
    let structure = drawn.placed.iter().find(|placed| placed.name == "y").unwrap();
    let from = centre_of(&drawn, "o");
    let header = Pos2::new((structure.x + structure.width / 2) as f32 / 100.0, structure.y as f32 / 100.0 + 2.0);
    let (_, intents) = drag_held(&ctx, &mut canvas, (from, header), 0.2, egui::Modifiers::NONE, &Shown::default());
    assert!(matches!(intents.as_slice(), [_, Intent::Move { nodes, .. }] if nodes == &["o"]), "{intents:?}");
}

/// Case D of the sheet: `z` in the arm would read `y`, the output of its own structure. While the pointer is over the
/// arm the reason shows beside it; the drop writes nothing, the node goes back, and the status bar says why.
#[test]
fn a_frame_that_refuses_the_nodes_says_why_and_the_drop_writes_nothing() {
    let (ctx, mut canvas, drawn) = setup(ARM);
    let body = arm_body(&drawn, "y", "true");
    let (from, to) = (centre_of(&drawn, "z"), body_centre(&drawn, body));
    let (over, intents) = drag_held(&ctx, &mut canvas, (from, to), 0.2, egui::Modifiers::NONE, &Shown::default());
    let said: Vec<&String> = intents.iter().filter_map(|intent| if let Intent::Say(text) = intent { Some(text) } else { None }).collect();
    let [reason] = said.as_slice() else { panic!("{intents:?}") };
    assert!(reason.contains("cycle"), "{reason}");
    assert!(!intents.iter().any(|intent| matches!(intent, Intent::Move { .. } | Intent::Reframe { .. })), "{intents:?}");
    assert!(texts(&over).iter().any(|text| text.contains("cycle")), "{:?}", texts(&over));
    assert!(canvas.moving.is_none(), "the node goes back");
}

/// Case C of the sheet: `p`, bound to the output `p` of the function, dropped into the `for`, leaves it: an `f64` read
/// where an `f64` is expected comes out as its last value (notation 2.5), and Alt takes a `collect`.
#[test]
fn a_value_that_leaves_a_loop_goes_out_as_the_type_of_its_reader_chooses() {
    let main = "labcraft 1

use core

fn main(xs: [f64], g: f64) -> (t: [f64], p: f64) {
    p = core.mul(g, 2.0)
    t = for x in xs {
        s = core.mul(x, g)
        collect s
    }
}
";
    let (ctx, mut canvas, drawn) = setup(main);
    let owner = drawn.placed.iter().position(|placed| placed.name == "t").unwrap();
    let body = drawn.bodies.iter().position(|body| body.owner == Some(owner)).unwrap();
    let (from, to) = (centre_of(&drawn, "p"), body_centre(&drawn, body));
    let at = landing(&drawn, "p", body, to - from);
    let (over, intents) = drag_held(&ctx, &mut canvas, (from, to), 0.2, egui::Modifiers::NONE, &Shown::default());
    let wanted = Intent::Reframe {
        nodes: vec!["p".to_owned()],
        within: InFrame { inside: Some("t".to_owned()), arm: None },
        at,
        through: vec![("p".to_owned(), vec!["last"])],
    };
    assert_eq!(intents, vec![selecting(&["p"], &[], &[]), wanted]);
    assert!(texts(&over).iter().any(|text| text.contains("last value")), "{:?}", texts(&over));
    canvas.cancel_move();
    let selected = showing(&selecting(&["p"], &[], &[]));
    let (_, intents) = drag_held(&ctx, &mut canvas, (from, to), 1.0, egui::Modifiers::ALT, &selected);
    assert!(matches!(intents.as_slice(), [Intent::Reframe { through, .. }] if through == &[("p".to_owned(), vec!["collect"])]), "{intents:?}");
}

/// Shift pressed only at the drop, the drag started without it, keeps the node in its frame (notation 2.4: "Shift held at
/// the drop").
#[test]
fn shift_pressed_at_the_drop_keeps_the_node_in_its_frame() {
    let (ctx, mut canvas, drawn) = setup(ARM);
    let body = arm_body(&drawn, "y", "true");
    let (from, to) = (centre_of(&drawn, "o"), body_centre(&drawn, body));
    let none = Shown::default();
    let shift = egui::Modifiers::SHIFT;
    let button = |at, pressed, modifiers| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers };
    let steps = [
        vec![egui::Event::PointerMoved(from), button(from, true, egui::Modifiers::NONE)],
        vec![egui::Event::PointerMoved(from + (to - from) / 2.0)],
        vec![egui::Event::PointerMoved(to)],
        vec![egui::Event::ModifiersChanged(shift), egui::Event::PointerMoved(to)],
        vec![egui::Event::PointerMoved(to), button(to, false, shift)],
        vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)],
    ];
    let mut intents = Vec::new();
    for (at, events) in steps.into_iter().enumerate() {
        intents.extend(frame_shown(&ctx, &mut canvas, 0.2 + at as f64 * 0.1, events, &none).2);
    }
    assert!(matches!(intents.as_slice(), [_, Intent::Move { nodes, .. }] if nodes == &["o"]), "{intents:?}");
}
