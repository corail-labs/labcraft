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

/// The canvas paints a drawing with the values it was drawn with, and only there: the colours of the thread are the
/// defaults again after.
#[test]
fn the_canvas_paints_with_the_values_of_its_drawing() {
    let background = |shapes: &[ClippedShape]| {
        shapes.iter().find_map(|clipped| match &clipped.shape {
            Painted::Rect(rect) => Some(rect.fill),
            _ => None,
        })
    };
    let ctx = context();
    let mut canvas = Canvas::new();
    canvas.set_drawing(drawing(ARMS, &ViewState::default()), false);
    let (before, _) = frame(&ctx, &mut canvas, &Shown::default(), input(0.0));
    assert_eq!(background(&before), Some(Color32::from_rgb(0xF7, 0xF7, 0xF5)));
    let mut style = labcraft_diagram::style::Style::default();
    let fill = labcraft_diagram::theme::TOKENS.iter().position(|token| token.name == "canvas.fill").unwrap();
    style.colors[fill].0 = 0x102030;
    let mut other = (*drawing(ARMS, &ViewState::default())).clone();
    other.style = Arc::new(style);
    canvas.set_drawing(Arc::new(other), false);
    let (after, _) = frame(&ctx, &mut canvas, &Shown::default(), input(0.1));
    assert_eq!(background(&after), Some(Color32::from_rgb(0x10, 0x20, 0x30)));
    assert_eq!(Theme::Light.color("canvas.fill"), Color32::from_rgb(0xF7, 0xF7, 0xF5));
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
    // At 100 %, the background and the ground of each name of a terminal of a frame (notation 5), which the far zoom
    // hides with the name.
    let grounds = drawn
        .groups
        .iter()
        .flat_map(|group| group.shapes.iter())
        .filter(|shape| matches!(shape, labcraft_diagram::Shape::Text { ground: Some(_), .. }))
        .count();
    assert!(grounds >= 3, "{grounds}");
    assert_eq!(count(&at(1.0, 30.0), |shape| matches!(shape, Painted::Rect(_))), 1 + grounds);
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

const DOUBLE: &str = "labcraft 1\n\nuse core\n\nfn main(x: f64) -> (y: f64) {\n    y = core.mul(x, 2.0)\n}\n";

/// The values of a run of `main` with `x` in and `y` out, over `drawing`, as `diagram` works them out.
fn values_of(drawing: &Drawing, x: f64, y: f64) -> Arc<Overlay> {
    let trace = format!(
        "{{\"schema\":\"labcraft-trace/1\",\"run\":\"r1\",\"function\":\"main\",\"mode\":\"deterministic\",\"files\":[]}}\n\
         {{\"seq\":1,\"t_ms\":0.0,\"event\":\"call\",\"frame\":0,\"function\":\"main\",\"in\":[{{\"name\":\"x\",\"type\":\"f64\",\"value\":{x:?}}}],\"types\":{{\"y\":\"f64\"}}}}\n\
         {{\"seq\":2,\"t_ms\":0.0,\"event\":\"ran\",\"frame\":0,\"node\":\"y\",\"out\":{{\"y\":{y:?}}}}}\n\
         {{\"seq\":3,\"t_ms\":0.0,\"event\":\"end\",\"status\":\"ok\"}}\n"
    );
    let mut reader = labcraft_trace::Reader::new(labcraft_diagram::expected(drawing), labcraft_trace::ReadLimits::default());
    reader.feed(trace.as_bytes());
    let read = reader.finish();
    Arc::new(labcraft_diagram::values(drawing, &read, &labcraft_diagram::Judged::default(), &labcraft_diagram::Choice::default()))
}

#[test]
fn the_values_of_a_run_show_over_their_drawing_and_hide_with_their_button() {
    let ctx = context();
    let mut canvas = Canvas::new();
    let drawn = drawing(DOUBLE, &ViewState::default());
    canvas.set_drawing(drawn.clone(), false);
    let shown = Shown::default();
    // The first frame installs the fonts.
    frame(&ctx, &mut canvas, &shown, input(0.0));
    let (none, _) = frame(&ctx, &mut canvas, &shown, input(0.2));
    assert!(!texts(&none).iter().any(|text| text == "21.5"), "{:?}", texts(&none));
    canvas.set_values(Some(values_of(&drawn, 10.75, 21.5)), None);
    let (with, _) = frame(&ctx, &mut canvas, &shown, input(0.4));
    let painted = texts(&with);
    assert!(painted.iter().any(|text| text == "21.5") && painted.iter().any(|text| text == "10.75"), "{painted:?}");
    // The button hides them, and shows them again.
    canvas.set_values_shown(false);
    let (hidden, _) = frame(&ctx, &mut canvas, &shown, input(0.6));
    assert!(!texts(&hidden).iter().any(|text| text == "21.5"));
    canvas.set_values_shown(true);
    // Below `zoom.values`, no bubble (notation 7).
    canvas.set_viewport(0.45, Vec2::ZERO);
    // A frame for the zoom to settle.
    frame(&ctx, &mut canvas, &shown, input(0.8));
    let (far, _) = frame(&ctx, &mut canvas, &shown, input(1.0));
    assert!(!texts(&far).iter().any(|text| text == "21.5"), "{:?}", texts(&far));
    canvas.set_viewport(1.0, Vec2::ZERO);
    frame(&ctx, &mut canvas, &shown, input(1.2));
    let (near, _) = frame(&ctx, &mut canvas, &shown, input(1.4));
    assert!(texts(&near).iter().any(|text| text == "21.5"));
    // A new drawing comes without the values of the one before.
    canvas.set_drawing(drawing(DOUBLE, &ViewState::default()), false);
    let (after, _) = frame(&ctx, &mut canvas, &shown, input(1.6));
    assert!(!texts(&after).iter().any(|text| text == "21.5"));
}

const COUNT: &str = "labcraft 1\n\nuse core\n\nfn main() -> (n: f64) {\n    n = loop carry i = 0.0 {\n        m = core.add(i, 1.0)\n        \
                     done = core.ge(m, 3.0)\n        next i = m\n        until done\n    }\n}\n";

/// A run of `COUNT` by hand: 3 iterations, `m` 1, 2, 3, `done` true at the last.
fn run_of_count(drawing: &Drawing) -> (Arc<Overlay>, Arc<labcraft_diagram::Trace>) {
    let mut lines = vec![
        r#"{"schema":"labcraft-trace/1","run":"r1","function":"main","mode":"deterministic","files":[]}"#.to_owned(),
        r#"{"seq":1,"t_ms":0.0,"event":"call","frame":0,"function":"main","in":[],"types":{"n":"f64","m":"f64","done":"bool"}}"#.to_owned(),
    ];
    for k in 0..3u32 {
        let m = f64::from(k + 1);
        lines
            .push(format!(r#"{{"seq":{},"t_ms":0.0,"event":"ran","frame":0,"node":"m","loops":["n"],"iter":[{k}],"out":{{"m":{m:?}}}}}"#, 2 + 2 * k));
        lines.push(format!(
            r#"{{"seq":{},"t_ms":0.0,"event":"ran","frame":0,"node":"done","loops":["n"],"iter":[{k}],"out":{{"done":{}}}}}"#,
            3 + 2 * k,
            k == 2
        ));
    }
    lines.push(r#"{"seq":8,"t_ms":0.0,"event":"ran","frame":0,"node":"n","out":{"n":3.0}}"#.to_owned());
    lines.push(r#"{"seq":9,"t_ms":0.0,"event":"end","status":"ok"}"#.to_owned());
    let text = lines.join("\n") + "\n";
    let mut reader = labcraft_trace::Reader::new(labcraft_diagram::expected(drawing), labcraft_trace::ReadLimits::default());
    reader.feed(text.as_bytes());
    let trace = Arc::new(reader.finish());
    assert_eq!(trace.ignored, 0);
    let overlay = labcraft_diagram::values(drawing, &trace, &labcraft_diagram::Judged::default(), &labcraft_diagram::Choice::default());
    (Arc::new(overlay), trace)
}

/// A frame with these events, at 100 % from the corner of the drawing: what the canvas painted and the iteration it asks for.
fn frame_with(ctx: &egui::Context, canvas: &mut Canvas, time: f64, events: Vec<egui::Event>) -> (Vec<ClippedShape>, Option<(String, u64)>) {
    let mut raw = input(time);
    raw.events = events;
    let mut iteration = None;
    let mut output = ctx.run_ui(raw, |ui| iteration = canvas.show(ui, Theme::Light, &Shown::default()).iteration);
    let shapes = std::mem::take(&mut output.shapes);
    output.drop_without_applying_deltas();
    (shapes, iteration)
}

/// A click at `at`, over two frames, from `time`: what the second asks for.
fn click(ctx: &egui::Context, canvas: &mut Canvas, at: Pos2, time: f64) -> Option<(String, u64)> {
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    let (_, first) = frame_with(ctx, canvas, time, vec![egui::Event::PointerMoved(at), button(true)]);
    let (_, second) = frame_with(ctx, canvas, time + 0.05, vec![egui::Event::PointerMoved(at), button(false)]);
    first.or(second)
}

fn middle((left, top, right, bottom): (i64, i64, i64, i64)) -> Pos2 {
    Pos2::new((left + right) as f32 / 200.0, (top + bottom) as f32 / 200.0)
}

#[test]
fn a_selector_steps_its_iterations_and_a_click_selects_a_node_until_escape() {
    let ctx = context();
    install_fonts(&ctx);
    let mut canvas = Canvas::new();
    let drawn = drawing(COUNT, &ViewState::default());
    canvas.set_drawing(drawn.clone(), false);
    canvas.set_viewport(1.0, Vec2::ZERO);
    let (overlay, trace) = run_of_count(&drawn);
    canvas.set_values(Some(overlay.clone()), Some((trace, labcraft_diagram::Choice::default())));
    frame_with(&ctx, &mut canvas, 0.0, Vec::new());
    let (shapes, _) = frame_with(&ctx, &mut canvas, 0.1, Vec::new());
    let selector = overlay.selectors.first().unwrap().clone();
    assert_eq!((selector.count, selector.shown), (3, 2));
    assert!(texts(&shapes).iter().any(|text| text == "3/3"), "{:?}", texts(&shapes));
    // Back from the last: the second, from 0; forward from the last: nothing.
    assert_eq!(click(&ctx, &mut canvas, middle(selector.back), 0.2), Some((selector.key.clone(), 1)));
    // Until the values of the second come, a step goes from it.
    assert_eq!(click(&ctx, &mut canvas, middle(selector.forward), 0.3), Some((selector.key.clone(), 2)));
    // The values of the last come back: forward from it asks nothing.
    canvas.set_values(Some(overlay.clone()), Some((run_of_count(&drawn).1, labcraft_diagram::Choice::default())));
    assert_eq!(click(&ctx, &mut canvas, middle(selector.forward), 0.4), None);
    assert!(canvas.selected_node().is_none(), "a click on a selector selects nothing");
    // A click on `done` selects it; its input `m`, 3 at the last iteration, shows once more than its bubble alone.
    let done = drawn.placed.iter().find(|placed| placed.key.ends_with("done")).unwrap();
    let at = Pos2::new((done.x + done.width / 2) as f32 / 100.0, (done.y + done.height / 2) as f32 / 100.0);
    let away = Pos2::new(1.0, 550.0);
    let (unselected, _) = frame_with(&ctx, &mut canvas, 0.6, vec![egui::Event::PointerMoved(away)]);
    click(&ctx, &mut canvas, at, 0.8);
    assert_eq!(canvas.selected_node(), Some(done.key.as_str()));
    let (selected, _) = frame_with(&ctx, &mut canvas, 1.0, vec![egui::Event::PointerMoved(away)]);
    let threes = |shapes: &[ClippedShape]| texts(shapes).iter().filter(|text| *text == "3").count();
    assert_eq!(threes(&selected), threes(&unselected) + 1, "{:?}", texts(&selected));
    // Escape selects nothing.
    let escape = egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    frame_with(&ctx, &mut canvas, 1.2, vec![escape.clone()]);
    assert!(canvas.selected_node().is_none());
    // A click on the wire `m` selects it and pins its values over the 3 iterations: a curve from 1 to 3, by hand.
    let m = drawn.groups.iter().position(|group| group.wire.as_ref().is_some_and(|wire| wire.value == "m")).unwrap();
    let path = &drawn.groups[m].wire.as_ref().unwrap().readers[0].1;
    let ((x0, y0), (x1, y1)) = (path[0], path[1]);
    let on_wire = Pos2::new((x0 + x1) as f32 / 200.0, (y0 + y1) as f32 / 200.0);
    let (before, _) = frame_with(&ctx, &mut canvas, 1.4, vec![egui::Event::PointerMoved(away)]);
    assert!(!texts(&before).iter().any(|text| text == "3 iterations"));
    click(&ctx, &mut canvas, on_wire, 1.6);
    assert_eq!(canvas.selected_wire(), Some(m));
    let (pinned, _) = frame_with(&ctx, &mut canvas, 1.8, vec![egui::Event::PointerMoved(away)]);
    let painted = texts(&pinned);
    assert!(["1", "3", "3 iterations"].iter().all(|wanted| painted.iter().any(|text| text == wanted)), "{painted:?}");
    frame_with(&ctx, &mut canvas, 2.0, vec![escape]);
    assert_eq!(canvas.selected_wire(), None);
}

/// Two nodes close together: `a` leaves `mul` and enters `add` a few units further.
const CHAIN: &str = "labcraft 1\n\nuse core\n\nfn main(x: f64) -> (y: f64) {\n    a = core.mul(x, 2.0)\n    y = core.add(a, 1.0)\n}\n";

/// The values of a run of `main` over `drawing`, from its input `x` and the outputs of its nodes, each named after its
/// value, all `f64`.
fn run_of(drawing: &Drawing, x: f64, outputs: &[(&str, f64)]) -> Arc<Overlay> {
    let types: Vec<String> = outputs.iter().map(|(name, _)| format!("\"{name}\":\"f64\"")).collect();
    let mut lines = vec![
        r#"{"schema":"labcraft-trace/1","run":"r1","function":"main","mode":"deterministic","files":[]}"#.to_owned(),
        format!(
            r#"{{"seq":1,"t_ms":0.0,"event":"call","frame":0,"function":"main","in":[{{"name":"x","type":"f64","value":{x:?}}}],"types":{{{}}}}}"#,
            types.join(",")
        ),
    ];
    for (index, (name, value)) in outputs.iter().enumerate() {
        lines.push(format!(r#"{{"seq":{},"t_ms":0.0,"event":"ran","frame":0,"node":"{name}","out":{{"{name}":{value:?}}}}}"#, index + 2));
    }
    lines.push(format!(r#"{{"seq":{},"t_ms":0.0,"event":"end","status":"ok"}}"#, outputs.len() + 2));
    let text = lines.join("\n") + "\n";
    let mut reader = labcraft_trace::Reader::new(labcraft_diagram::expected(drawing), labcraft_trace::ReadLimits::default());
    reader.feed(text.as_bytes());
    let read = reader.finish();
    assert_eq!(read.ignored, 0);
    Arc::new(labcraft_diagram::values(drawing, &read, &labcraft_diagram::Judged::default(), &labcraft_diagram::Choice::default()))
}

/// The box of a bubble, and of each value at the inputs of a node with the group of its wire, in units.
fn boxes(drawing: &Drawing, overlay: &Overlay, wire: &str, node: usize) -> (Rect, Vec<(usize, Rect)>) {
    let bubble = overlay.bubbles.iter().find(|bubble| bubble.wire == wire).unwrap();
    let rect = |(l, t, r, b): (i64, i64, i64, i64)| Rect::from_min_max(Pos2::new(l as f32, t as f32) / 100.0, Pos2::new(r as f32, b as f32) / 100.0);
    let inputs = labcraft_diagram::input_boxes(drawing, overlay, node).into_iter().map(|(group, area)| (group, rect(area))).collect();
    (crate::boxed(&bubble.layer.shapes).unwrap(), inputs)
}

/// Where each text `text` is painted.
fn painted_at(shapes: &[ClippedShape], text: &str) -> Vec<Pos2> {
    shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            Painted::Text(shape) if shape.galley.job.text == text => Some(shape.pos),
            _ => None,
        })
        .collect()
}

/// A click in the middle of a placed node, then a frame with the pointer away: what it paints.
fn select_and_paint(ctx: &egui::Context, canvas: &mut Canvas, drawing: &Drawing, node: usize, time: f64) -> Vec<ClippedShape> {
    let block = &drawing.placed[node];
    click(ctx, canvas, Pos2::new((block.x + block.width / 2) as f32 / 100.0, (block.y + block.height / 2) as f32 / 100.0), time);
    frame_with(ctx, canvas, time + 0.4, vec![egui::Event::PointerMoved(Pos2::new(1.0, 550.0))]).0
}

/// A node selected shows the value at its input; the bubble of the same wire that it covers, the same value, is not
/// drawn (notation 6.1; decision of the owner, 2026-11-13).
#[test]
fn the_value_at_an_input_hides_the_bubble_of_its_wire_that_it_covers() {
    let ctx = context();
    install_fonts(&ctx);
    let mut canvas = Canvas::new();
    let drawn = drawing(CHAIN, &ViewState::default());
    canvas.set_drawing(drawn.clone(), false);
    canvas.set_viewport(1.0, Vec2::ZERO);
    // By hand: `x` 1.25, `a` 2.5, `y` 3.5.
    let overlay = run_of(&drawn, 1.25, &[("a", 2.5), ("y", 3.5)]);
    canvas.set_values(Some(overlay.clone()), None);
    let add = drawn.placed.iter().position(|placed| placed.key.ends_with('y')).unwrap();
    // The case holds: the value at the input of `add` covers the bubble of `a`, its own wire.
    let (bubble, inputs) = boxes(&drawn, &overlay, "a", add);
    let a = drawn.groups.iter().position(|group| group.wire.as_ref().is_some_and(|wire| wire.value == "a")).unwrap();
    assert!(inputs.iter().any(|(group, input)| *group == a && crate::overlaps(input, &bubble)), "{inputs:?} {bubble:?}");
    frame_with(&ctx, &mut canvas, 0.0, Vec::new());
    let (unselected, _) = frame_with(&ctx, &mut canvas, 0.2, vec![egui::Event::PointerMoved(Pos2::new(1.0, 550.0))]);
    let in_bubble = painted_at(&unselected, "2.5");
    assert_eq!(in_bubble.len(), 1, "{:?}", texts(&unselected));
    // Selected: one `2.5` still, and no longer where the bubble wrote it: at the input.
    let at_input = painted_at(&select_and_paint(&ctx, &mut canvas, &drawn, add, 0.4), "2.5");
    assert_eq!(at_input.len(), 1, "the value at the input in place of the bubble");
    assert_ne!(at_input, in_bubble, "the bubble hidden, not the value at the input");
}

/// The bubble of another wire stays, though a value at an input covers it: it gives another value.
const CROSSED: &str = "labcraft 1\n\nuse core\n\nfn main(x: f64) -> (y: f64, z: f64) {\n    a = core.mul(x, 2.0)\n    b = core.add(a, 1.0)\n    \
                       c = core.add(b, 1.0)\n    y = core.add(c, a)\n    z = core.mul(a, 2.0)\n}\n";

#[test]
fn the_bubble_of_another_wire_stays_under_a_value_at_an_input() {
    let ctx = context();
    install_fonts(&ctx);
    let mut canvas = Canvas::new();
    let drawn = drawing(CROSSED, &ViewState::default());
    canvas.set_drawing(drawn.clone(), false);
    canvas.set_viewport(1.0, Vec2::ZERO);
    // By hand: `x` 10.125, `a` 20.25, `b` 21.25, `c` 22.25, `y` 42.5, `z` 40.5.
    let overlay = run_of(&drawn, 10.125, &[("a", 20.25), ("b", 21.25), ("c", 22.25), ("y", 42.5), ("z", 40.5)]);
    canvas.set_values(Some(overlay.clone()), None);
    // A node whose values at its inputs cover the bubble of `z`, which none of them carries.
    let z = drawn.groups.iter().position(|group| group.wire.as_ref().is_some_and(|wire| wire.value == "z")).unwrap();
    let node = (0..drawn.placed.len())
        .find(|&node| {
            let (bubble, inputs) = boxes(&drawn, &overlay, "z", node);
            inputs.iter().any(|(group, input)| *group != z && crate::overlaps(input, &bubble))
        })
        .expect("the case holds: a value at an input covers the bubble of `z`");
    frame_with(&ctx, &mut canvas, 0.0, Vec::new());
    let selected = select_and_paint(&ctx, &mut canvas, &drawn, node, 0.2);
    assert_eq!(painted_at(&selected, "40.5").len(), 1, "{:?}", texts(&selected));
}

/// The wheel turned up over a selector steps back, and never scrolls the canvas; down from the last, nothing.
#[test]
fn the_wheel_over_a_selector_steps_its_iterations_and_does_not_scroll() {
    let ctx = context();
    install_fonts(&ctx);
    let mut canvas = Canvas::new();
    let drawn = drawing(COUNT, &ViewState::default());
    canvas.set_drawing(drawn.clone(), false);
    canvas.set_viewport(1.0, Vec2::ZERO);
    let (overlay, trace) = run_of_count(&drawn);
    canvas.set_values(Some(overlay.clone()), Some((trace, labcraft_diagram::Choice::default())));
    let selector = overlay.selectors.first().unwrap().clone();
    let at = middle(selector.number);
    let wheel = |y: f32| egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: Vec2::new(0.0, y),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
        modifiers: egui::Modifiers::NONE,
    };
    let turn = |canvas: &mut Canvas, y: f32, from: f64| {
        let mut asked = Vec::new();
        let (_, first) = frame_with(&ctx, canvas, from, vec![egui::Event::PointerMoved(at), wheel(y)]);
        asked.extend(first);
        // The smoothing of the wheel spreads it over the frames that follow.
        for frame in 1..40 {
            let (_, next) = frame_with(&ctx, canvas, from + f64::from(frame) * 0.02, vec![egui::Event::PointerMoved(at)]);
            asked.extend(next);
        }
        asked
    };
    frame_with(&ctx, &mut canvas, 0.0, vec![egui::Event::PointerMoved(at)]);
    let before = canvas.viewport();
    assert!(turn(&mut canvas, -60.0, 0.1).is_empty(), "down from the last: nothing");
    let back = turn(&mut canvas, 60.0, 2.0);
    assert_eq!(back.first(), Some(&(selector.key.clone(), 1)), "{back:?}");
    assert_eq!(canvas.viewport(), before, "the canvas did not scroll");
}

/// A number typed into the selector replaces the one shown: the field opens empty, `5` then Enter asks for the fifth.
#[test]
fn a_number_typed_into_a_selector_chooses_that_iteration() {
    let ctx = context();
    install_fonts(&ctx);
    let mut canvas = Canvas::new();
    let drawn = drawing(COUNT, &ViewState::default());
    canvas.set_drawing(drawn.clone(), false);
    canvas.set_viewport(1.0, Vec2::ZERO);
    let (overlay, trace) = run_of_count(&drawn);
    canvas.set_values(Some(overlay.clone()), Some((trace, labcraft_diagram::Choice::default())));
    frame_with(&ctx, &mut canvas, 0.0, Vec::new());
    let selector = overlay.selectors.first().unwrap().clone();
    assert_eq!(click(&ctx, &mut canvas, middle(selector.number), 0.1), None, "the field opens");
    frame_with(&ctx, &mut canvas, 0.2, Vec::new());
    let (_, typed) = frame_with(&ctx, &mut canvas, 0.3, vec![egui::Event::Text("2".to_owned())]);
    assert_eq!(typed, None);
    let enter = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    let (_, asked) = frame_with(&ctx, &mut canvas, 0.4, vec![enter]);
    assert_eq!(asked, Some((selector.key.clone(), 1)), "the second, from 0");
}

/// A list of 1,000 iterations opens on the one shown, here the 901st, counted from 1 (notation 6.1).
#[test]
fn a_list_of_values_opens_on_the_iteration_shown() {
    let ctx = context();
    let values = (0..1_000u64).map(|iteration| labcraft_diagram::Sample { iteration, value: Some((format!("v{iteration}"), None)) }).collect();
    let series = labcraft_diagram::Series { key: "n".to_owned(), count: 1_000, shown: 900, values, curve: false, unit: String::new() };
    let mut painted = Vec::new();
    for time in [0.0, 0.1] {
        let mut output = ctx.run_ui(input(time), |ui| {
            crate::series::show(ui, "n", &series, Theme::Light, false);
        });
        painted = texts(&std::mem::take(&mut output.shapes));
        output.drop_without_applying_deltas();
    }
    assert!(painted.iter().any(|text| text.contains("901  v900")), "{painted:?}");
    assert!(!painted.iter().any(|text| text.contains("  1  v0")), "{painted:?}");
}
