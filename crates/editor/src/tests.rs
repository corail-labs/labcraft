//! The canvas without a window: a context run by hand, its output read back.

use std::borrow::Cow;
use std::sync::Arc;

use egui::epaint::text::{FontData, FontDefinitions};
use egui::epaint::{ClippedShape, Shape as Painted};
use egui::{FontFamily, Pos2, RawInput, Rect, Vec2};
use labcraft_check::{Content, Found, Libraries, LibrarySource, Origin, SourceFile, check};
use labcraft_diagram::{DiagramLimits, Drawing, Drawn, ViewState, draw};
use labcraft_types::{Interner, Limits};

use super::*;

struct Nothing;

impl Libraries for Nothing {
    fn find(&self, _: &str, _: Origin) -> Found {
        Found::NotFound
    }
}

const ARMS: &str = "labcraft 1

use core

fn main(x: f64, n: i64) -> (y: f64) {
    y = match n {
        0 => yield x
        1 => {
            a = core.add(x, 1.0)
            b = core.mul(a, 2.0)
            yield b
        }
        _ => yield 0.0
    }
}
";

fn drawing(text: &str, view: &ViewState) -> Arc<Drawing> {
    drawing_of(&[("main.lcf", text)], view)
}

/// The drawing of `main.lcf` among `files`.
fn drawing_of(files: &[(&str, &str)], view: &ViewState) -> Arc<Drawing> {
    let mut names = Interner::new();
    let mut root = LibrarySource::new(String::new(), String::new(), Origin::Project);
    for (shown, text) in files {
        root.files.push(SourceFile { shown: (*shown).to_owned(), content: Content::Bytes(Cow::Owned(text.as_bytes().to_vec())) });
    }
    let checked = check(root, &Nothing, &mut names, &Limits::default());
    let main = checked.files.iter().position(|file| file.shown == "main.lcf").unwrap();
    let Drawn::Drawing(drawing) = draw(&checked, main, &names, view, &DiagramLimits::default()) else { panic!("not drawn") };
    Arc::new(drawing)
}

/// A context whose own families have a face, as a host gives them, since egui is built without its default fonts:
/// Hack for all, which a tooltip needs.
fn context() -> egui::Context {
    let ctx = egui::Context::default();
    let mut fonts = FontDefinitions::empty();
    fonts.font_data.insert("hack".to_owned(), Arc::new(FontData::from_static(include_bytes!("../../../assets/fonts/Hack-Regular.ttf"))));
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts.families.insert(family, vec!["hack".to_owned()]);
    }
    ctx.set_fonts(fonts);
    ctx
}

fn input(time: f64) -> RawInput {
    RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1340.0, 560.0))), time: Some(time), ..RawInput::default() }
}

fn frame(ctx: &egui::Context, canvas: &mut Canvas, shown: &Shown, input: RawInput) -> (Vec<ClippedShape>, Option<(String, usize)>) {
    let mut tab = None;
    let mut output = ctx.run_ui(input, |ui| tab = canvas.show(ui, Theme::Light, shown).tab);
    let shapes = std::mem::take(&mut output.shapes);
    output.drop_without_applying_deltas();
    (shapes, tab)
}

fn count(shapes: &[ClippedShape], what: fn(&Painted) -> bool) -> usize {
    shapes.iter().filter(|clipped| what(&clipped.shape)).count()
}

fn is_text(shape: &Painted) -> bool {
    matches!(shape, Painted::Text(_))
}

fn texts(shapes: &[ClippedShape]) -> Vec<String> {
    shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            Painted::Text(text) => Some(text.galley.job.text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_theme_gives_the_colours_of_notation_8() {
    assert_eq!(Theme::Light.color("wire.error"), Color32::from_rgb(0xAD, 0x3A, 0x0B));
    assert_eq!(Theme::Dark.color("wire.error"), Color32::from_rgb(0xFF, 0x8F, 0x5E));
    assert_eq!(Theme::Light.color("no.such"), Color32::from_rgb(255, 0, 255));
}

/// A host that never installed the fonts of the notation: the first frame paints no text and installs them, the next
/// paints its texts.
#[test]
fn the_canvas_installs_its_font_and_paints_without_text_until_then() {
    let ctx = context();
    let mut canvas = Canvas::new();
    canvas.set_drawing(drawing(ARMS, &ViewState::default()), true);
    let (first, _) = frame(&ctx, &mut canvas, &Shown::default(), input(0.0));
    assert_eq!(count(&first, is_text), 0);
    assert!(count(&first, |shape| matches!(shape, Painted::Mesh(_))) > 0);
    let (second, _) = frame(&ctx, &mut canvas, &Shown::default(), input(0.1));
    assert!(has_family(&ctx));
    let shown = texts(&second);
    assert!(shown.contains(&"main".to_owned()), "{shown:?}");
}

/// Unfolded, the three arms each have their title; in tabs, the first is shown, the tab of another chooses it.
#[test]
fn a_click_on_a_tab_asks_for_its_arm() {
    let ctx = context();
    install_fonts(&ctx);
    let mut canvas = Canvas::new();
    let tabs = drawing(ARMS, &ViewState { tabs: true, ..ViewState::default() });
    canvas.set_drawing(tabs.clone(), true);
    canvas.set_viewport(1.0, Vec2::ZERO);
    let (shapes, _) = frame(&ctx, &mut canvas, &Shown::default(), input(0.0));
    let (shapes, _) = if count(&shapes, is_text) == 0 { frame(&ctx, &mut canvas, &Shown::default(), input(0.05)) } else { (shapes, None) };
    // The nodes of the second arm do not show while the first is.
    let second =
        tabs.placed.iter().filter(|placed| placed.body == tabs.placed.iter().find(|p| !p.holds.is_empty()).map_or(0, |p| p.holds[1])).count();
    assert_eq!(second, 2);
    let first_count = count(&shapes, |shape| matches!(shape, Painted::Mesh(_)));
    // A click in the middle of the second tab.
    let tab = &tabs.tabs[1];
    let at = Pos2::new((tab.x + tab.width / 2) as f32 / 100.0, (tab.y + tab.height / 2) as f32 / 100.0);
    let press = |pressed: bool, time: f64| {
        let mut raw = input(time);
        raw.events = vec![
            egui::Event::PointerMoved(at),
            egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE },
        ];
        raw
    };
    frame(&ctx, &mut canvas, &Shown::default(), press(true, 0.2));
    let (_, clicked) = frame(&ctx, &mut canvas, &Shown::default(), press(false, 0.25));
    let structure = tabs.arms[1].structure.clone();
    assert_eq!(clicked, Some((structure.clone(), 1)));
    // The host shows it: two nodes more than the arm of a single `yield`.
    let shown = Shown { arms: [(structure, 1)].into() };
    let (shapes, _) = frame(&ctx, &mut canvas, &shown, input(0.4));
    assert_eq!(count(&shapes, |shape| matches!(shape, Painted::Mesh(_))), first_count + 4, "two nodes and their two wires");
}

#[test]
fn a_new_drawing_glides_then_shows_its_wires() {
    let ctx = context();
    install_fonts(&ctx);
    let mut canvas = Canvas::new();
    let before = "labcraft 1\n\nuse core\n\nfn main(x: f64) -> (y: f64) {\n    y = core.add(x, 1.0)\n}\n";
    let after = "labcraft 1\n\nuse core\n\nfn main(x: f64) -> (y: f64) {\n    a = core.mul(x, 2.0)\n    y = core.add(a, 1.0)\n}\n";
    canvas.set_drawing(drawing(before, &ViewState::default()), true);
    frame(&ctx, &mut canvas, &Shown::default(), input(0.0));
    let meshes = |shapes: &[ClippedShape]| count(shapes, |shape| matches!(shape, Painted::Mesh(_)));
    let after = drawing(after, &ViewState::default());
    let wires = after.groups.iter().filter(|group| group.wire.is_some()).count();
    assert!(wires >= 2);
    canvas.set_drawing(after.clone(), true);
    assert!(canvas.gliding());
    let (start, _) = frame(&ctx, &mut canvas, &Shown::default(), input(1.0));
    assert!(canvas.gliding());
    frame(&ctx, &mut canvas, &Shown::default(), input(1.0 + GLIDE_SECONDS / 2.0));
    assert!(canvas.gliding());
    let (end, _) = frame(&ctx, &mut canvas, &Shown::default(), input(1.0 + GLIDE_SECONDS));
    assert!(!canvas.gliding());
    // The wires show at the end, and only then.
    assert_eq!(meshes(&end), meshes(&start) + wires);
    // No glide for another function, which the host says, nor when the system asks for reduced motion.
    canvas.set_drawing(drawing(before, &ViewState::default()), false);
    assert!(!canvas.gliding());
    canvas.set_drawing(after, true);
    assert!(canvas.gliding());
    canvas.set_reduced_motion(true);
    assert!(!canvas.gliding());
    canvas.set_drawing(drawing(before, &ViewState::default()), true);
    assert!(!canvas.gliding());
}

#[test]
fn the_glide_eases_from_the_old_place() {
    let old = Drawing {
        placed: vec![labcraft_diagram::Placed {
            key: "a".into(),
            x: 1000,
            y: 0,
            width: 0,
            height: 0,
            column: 0,
            rank: 0,
            body: 0,
            holds: Vec::new(),
        }],
        ..Drawing::default()
    };
    let mut new = old.clone();
    new.placed[0].x = 3000;
    let mut glide = view::Glide::between(&old, &new).unwrap();
    assert!(!glide.advance(10.0));
    assert_eq!(glide.offsets(), [Vec2::new(-20.0, 0.0)]);
    assert!(!glide.advance(10.0 + GLIDE_SECONDS / 2.0));
    assert_eq!(glide.offsets(), [Vec2::new(-20.0 * 0.125, 0.0)]);
    assert!(glide.advance(10.0 + GLIDE_SECONDS));
    assert_eq!(glide.offsets(), [Vec2::ZERO]);
    assert!(view::Glide::between(&old, &old).is_none());
}

/// Zoomed in on a corner, the canvas paints only what it shows.
#[test]
fn only_what_the_viewport_shows_is_painted() {
    let ctx = context();
    install_fonts(&ctx);
    let mut text = String::from("labcraft 1\n\nuse core\n\nfn main(x: f64) -> (y: f64) {\n    a0 = core.add(x, 1.0)\n");
    for index in 1..200 {
        text.push_str(&format!("    a{index} = core.add(a{}, 1.0)\n", index - 1));
    }
    text.push_str("    y = a199\n}\n");
    let mut canvas = Canvas::new();
    canvas.set_drawing(drawing(&text, &ViewState::default()), true);
    canvas.set_viewport(0.1, Vec2::ZERO);
    let (whole, _) = frame(&ctx, &mut canvas, &Shown::default(), input(0.0));
    canvas.set_viewport(2.0, Vec2::ZERO);
    let (corner, _) = frame(&ctx, &mut canvas, &Shown::default(), input(1.0));
    let meshes = |shapes: &[ClippedShape]| count(shapes, |shape| matches!(shape, Painted::Mesh(_)));
    assert!(meshes(&corner) * 5 < meshes(&whole), "{} {}", meshes(&corner), meshes(&whole));
    // A node more at the start of the chain moves every node: while they glide, still only those whose way crosses
    // the viewport.
    let moved = text.replace(
        "    a0 = core.add(x, 1.0)
",
        "    b = core.neg(x)
    a0 = core.add(b, 1.0)
",
    );
    canvas.set_drawing(drawing(&moved, &ViewState::default()), true);
    let (gliding, _) = frame(&ctx, &mut canvas, &Shown::default(), input(2.0));
    assert!(canvas.gliding());
    assert!(meshes(&gliding) * 5 < meshes(&whole), "{} {}", meshes(&gliding), meshes(&whole));
}

/// Notation 7: no name of a terminal below 75 %; below 40 %, nodes as plain rectangles and no texts but titles.
#[test]
fn the_zoom_decides_what_shows() {
    let ctx = context();
    install_fonts(&ctx);
    let mut canvas = Canvas::new();
    canvas.set_drawing(drawing(ARMS, &ViewState::default()), true);
    let mut at = |zoom: f32, time: f64| {
        canvas.set_viewport(zoom, Vec2::ZERO);
        frame(&ctx, &mut canvas, &Shown::default(), input(time));
        let (shapes, _) = frame(&ctx, &mut canvas, &Shown::default(), input(time + 1.0));
        shapes
    };
    let near = texts(&at(1.0, 0.0));
    assert!(near.contains(&"x".to_owned()) && near.contains(&"main".to_owned()), "{near:?}");
    let middle = texts(&at(0.5, 10.0));
    // `n` names only a terminal of the function; `x` and `y` also name wires, which show.
    assert!(near.contains(&"n".to_owned()) && !middle.contains(&"n".to_owned()) && middle.contains(&"main".to_owned()), "{middle:?}");
    let far = at(0.3, 20.0);
    let far_texts = texts(&far);
    assert!(far_texts.contains(&"main".to_owned()) && far_texts.contains(&"match n".to_owned()), "{far_texts:?}");
    assert!(!far_texts.iter().any(|text| text == "add" || text == "+"), "{far_texts:?}");
    // The background, and each node that is not a structure as a plain rectangle.
    let drawn = drawing(ARMS, &ViewState::default());
    let nodes = drawn.placed.iter().filter(|placed| placed.holds.is_empty()).count();
    assert!(nodes >= 2);
    assert_eq!(count(&far, |shape| matches!(shape, Painted::Rect(_))), 1 + nodes);
    assert_eq!(count(&at(1.0, 30.0), |shape| matches!(shape, Painted::Rect(_))), 1);
}

/// An icon that `check` only warns about (`LC5009`) draws within its square, quickly: a text of size 100,000, at the
/// largest zoom, and a filled path left open, which the SVG fills as if closed.
#[test]
#[expect(clippy::disallowed_methods, reason = "a native test, never built for WASM, where the clock traps")]
fn a_hostile_icon_is_drawn_quickly_and_cut_to_its_square() {
    let f = "labcraft 1\n\nfn f(x: f64) -> (y: f64) {\n    y = x\n}\n\nicon 32 {\n    text \"W\" at (16, 30) size 100000\n    path \"M4 4 L28 4 L16 28\" fill\n    path \"M4 28 L16 20 L28 28 L16 4 Z\" fill\n}\n";
    let main = "labcraft 1\n\nfn main(x: f64) -> (y: f64) {\n    y = f(x)\n}\n";
    let drawn = drawing_of(&[("main.lcf", main), ("f.lcf", f)], &ViewState::default());
    assert_eq!(drawn.icons.len(), 1);
    let ctx = context();
    install_fonts(&ctx);
    let mut canvas = Canvas::new();
    canvas.set_drawing(drawn, false);
    for (zoom, time) in [(1.0, 0.0), (MAX_ZOOM, 1.0)] {
        canvas.set_viewport(zoom, Vec2::ZERO);
        let begun = std::time::Instant::now();
        let (shapes, _) = frame(&ctx, &mut canvas, &Shown::default(), input(time));
        let (shapes, _) = if texts(&shapes).is_empty() { frame(&ctx, &mut canvas, &Shown::default(), input(time + 0.5)) } else { (shapes, None) };
        assert!(begun.elapsed().as_secs_f64() < 2.0, "{:?}", begun.elapsed());
        // The text of the icon at twice its grid at most.
        let largest = shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                Painted::Text(text) if text.galley.job.text == "W" => text.galley.job.sections.first().map(|section| section.format.font_id.size),
                _ => None,
            })
            .fold(0.0f32, f32::max);
        assert!(largest > 0.0 && largest <= 64.0 * 28.0 / 32.0 * zoom + 0.01, "{largest}");
    }
}

/// A polygon that is not convex is filled as the SVG fills it: its triangles cover its area, no more.
#[test]
fn a_concave_polygon_is_cut_into_its_ears() {
    // An arrow head of 4 points, its notch at (16, 20). Its area, by the shoelace formula worked by hand:
    // (80 - 448) + (448 - 560) + (112 - 448) + (448 - 16) = -384, half of it 192.
    let points = [Pos2::new(4.0, 28.0), Pos2::new(16.0, 20.0), Pos2::new(28.0, 28.0), Pos2::new(16.0, 4.0)];
    let mut mesh = egui::epaint::Mesh::default();
    let mut tessellator = egui::epaint::Tessellator::new(1.0, egui::epaint::TessellationOptions::default(), [1, 1], Vec::new());
    paint::fill_polygon(&points, Color32::WHITE, &mut tessellator, &mut mesh);
    let area: f32 = mesh
        .indices
        .chunks(3)
        .map(|triangle| {
            let [a, b, c] = [0, 1, 2].map(|corner| mesh.vertices[triangle[corner] as usize].pos);
            ((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)).abs() / 2.0
        })
        .sum();
    assert_eq!(area, 192.0);
}

#[test]
fn the_first_frame_fits_the_drawing_at_100_percent_at_most() {
    let ctx = context();
    let mut canvas = Canvas::new();
    let drawn = drawing(ARMS, &ViewState::default());
    canvas.set_drawing(drawn.clone(), true);
    frame(&ctx, &mut canvas, &Shown::default(), input(0.0));
    let (zoom, origin) = canvas.viewport();
    assert!(zoom <= 1.0);
    // Smaller than the canvas: centred.
    let size = Vec2::new(drawn.width as f32, drawn.height as f32) / 100.0;
    assert!((origin + (Vec2::new(1340.0, 560.0) / zoom - size) / 2.0).length() < 0.01, "{origin:?}");
}

/// A path of an icon: its curves in points, closed where it closes.
#[test]
fn a_path_is_flattened_into_its_subpaths() {
    let segments = labcraft_check::path_segments("M7 21 A9 9 0 0 1 25 21 M0 0 L4 0 L4 4 Z").unwrap();
    let parts = paint::subpaths(&segments);
    assert_eq!(parts.len(), 2);
    let (arc, closed) = &parts[0];
    assert!(!closed);
    // Every point of the arc 9 from its center, the top reached.
    assert!(arc.iter().all(|&(x, y)| (((x - 16.0).powi(2) + (y - 21.0).powi(2)).sqrt() - 9.0).abs() < 1e-9));
    assert!(arc.iter().any(|&(x, y)| (x - 16.0).abs() < 1e-9 && (y - 12.0).abs() < 1e-9));
    assert_eq!(parts[1], (vec![(0.0, 0.0), (4.0, 0.0), (4.0, 4.0)], true));
}
