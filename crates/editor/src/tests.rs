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
    assert!(shown.contains(&"fn main".to_owned()), "{shown:?}");
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
    // The host shows it: two nodes more than the arm of a single `yield`, primitives whose icons are meshes of their own.
    let shown = Shown { arms: [(structure, 1)].into(), ..Shown::default() };
    let (shapes, _) = frame(&ctx, &mut canvas, &shown, input(0.4));
    assert_eq!(
        count(&shapes, |shape| matches!(shape, Painted::Mesh(_))),
        first_count + 6,
        "two nodes, the icons of their primitives and their two wires"
    );
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
            reach: (0, 0, 0),
            key: "a".into(),
            x: 1000,
            y: 0,
            width: 0,
            height: 0,
            column: 0,
            rank: 0,
            body: 0,
            holds: Vec::new(),
            name: "a".into(),
            line: 0,
            at: None,
            view: String::new(),
            over: 0,
            least: (0, 0),
            size: None,
            callee: String::new(),
            held: Vec::new(),
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
    assert!(near.contains(&"x".to_owned()) && near.contains(&"fn main".to_owned()), "{near:?}");
    let middle = texts(&at(0.5, 10.0));
    // `n` names only a terminal of the function; `x` and `y` also name wires, which show.
    assert!(near.contains(&"n".to_owned()) && !middle.contains(&"n".to_owned()) && middle.contains(&"fn main".to_owned()), "{middle:?}");
    let far = at(0.3, 20.0);
    let far_texts = texts(&far);
    assert!(far_texts.contains(&"fn main".to_owned()) && far_texts.contains(&"match n".to_owned()), "{far_texts:?}");
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
        // Its square is the whole block of 40, which has no band (notation 3.2).
        assert!(largest > 0.0 && largest <= 64.0 * 40.0 / 32.0 * zoom + 0.01, "{largest}");
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

/// A frame with these events, at 100 % from the corner of the drawing, the host showing `shown`: what the canvas painted,
/// the iteration it asks for and what its gestures ask for.
fn frame_shown(
    ctx: &egui::Context,
    canvas: &mut Canvas,
    time: f64,
    events: Vec<egui::Event>,
    shown: &Shown,
) -> (Vec<ClippedShape>, Option<(String, u64)>, Vec<Intent>) {
    let mut raw = input(time);
    raw.events = events;
    run_frame(ctx, canvas, raw, shown)
}

/// A frame of this input, the host showing `shown`.
fn run_frame(ctx: &egui::Context, canvas: &mut Canvas, raw: RawInput, shown: &Shown) -> (Vec<ClippedShape>, Option<(String, u64)>, Vec<Intent>) {
    let mut iteration = None;
    let mut intents = Vec::new();
    let mut output = ctx.run_ui(raw, |ui| {
        let response = canvas.show(ui, Theme::Light, shown);
        iteration = response.iteration;
        intents = response.intents;
    });
    let shapes = std::mem::take(&mut output.shapes);
    output.drop_without_applying_deltas();
    (shapes, iteration, intents)
}

/// A frame with these events, at 100 % from the corner of the drawing: what the canvas painted and the iteration it asks for.
fn frame_with(ctx: &egui::Context, canvas: &mut Canvas, time: f64, events: Vec<egui::Event>) -> (Vec<ClippedShape>, Option<(String, u64)>) {
    let (shapes, iteration, _) = frame_shown(ctx, canvas, time, events, &Shown::default());
    (shapes, iteration)
}

/// A press and a release of the primary button at `at`, with `modifiers`, over two frames from `time`, the host showing
/// `shown`: the iteration and what the gestures ask for.
fn click_with(
    ctx: &egui::Context,
    canvas: &mut Canvas,
    at: Pos2,
    time: f64,
    modifiers: egui::Modifiers,
    shown: &Shown,
) -> (Option<(String, u64)>, Vec<Intent>) {
    // egui reads the modifiers that the input says, not those of the event.
    let frame_at = |canvas: &mut Canvas, time: f64, pressed: bool| {
        let mut raw = input(time);
        raw.events = vec![
            egui::Event::ModifiersChanged(modifiers),
            egui::Event::PointerMoved(at),
            egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers },
        ];
        run_frame(ctx, canvas, raw, shown)
    };
    let (_, first, mut intents) = frame_at(canvas, time, true);
    let (_, second, more) = frame_at(canvas, time + 0.05, false);
    intents.extend(more);
    let mut raw = input(time + 0.1);
    raw.events = vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)];
    intents.extend(run_frame(ctx, canvas, raw, shown).2);
    (first.or(second), intents)
}

/// A click at `at`, over two frames, from `time`: what the second asks for.
fn click(ctx: &egui::Context, canvas: &mut Canvas, at: Pos2, time: f64) -> Option<(String, u64)> {
    click_with(ctx, canvas, at, time, egui::Modifiers::NONE, &Shown::default()).0
}

fn middle((left, top, right, bottom): (i64, i64, i64, i64)) -> Pos2 {
    Pos2::new((left + right) as f32 / 200.0, (top + bottom) as f32 / 200.0)
}

fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
}

/// The centre of a node, in units, at 100 % from the corner of the drawing.
fn centre_of(drawing: &Drawing, name: &str) -> Pos2 {
    let placed = drawing.placed.iter().find(|placed| placed.name == name).unwrap();
    Pos2::new((placed.x + placed.width / 2) as f32 / 100.0, (placed.y + placed.height / 2) as f32 / 100.0)
}

fn selecting(nodes: &[&str], wires: &[String], terminals: &[&str]) -> Intent {
    Intent::Select(Selection {
        nodes: nodes.iter().map(|name| (*name).to_owned()).collect(),
        wires: wires.to_vec(),
        terminals: terminals.iter().map(|name| (*name).to_owned()).collect(),
    })
}

fn showing(intent: &Intent) -> Shown {
    let Intent::Select(selection) = intent else { panic!("a selection expected, got {intent:?}") };
    Shown { selection: selection.clone(), ..Shown::default() }
}

#[test]
fn a_selector_steps_its_iterations_and_a_click_asks_for_a_selection_that_the_host_shows() {
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
    // The values of the last come back: forward from it asks nothing, and a click on a selector selects nothing.
    canvas.set_values(Some(overlay.clone()), Some((run_of_count(&drawn).1, labcraft_diagram::Choice::default())));
    let (iteration, intents) = click_with(&ctx, &mut canvas, middle(selector.forward), 0.4, egui::Modifiers::NONE, &Shown::default());
    assert_eq!((iteration, intents), (None, Vec::new()));
    // A click on `done` asks for it; the host shows it, and its input `m`, 3 at the last iteration, shows once more than
    // its bubble alone.
    let at = centre_of(&drawn, "done");
    let away = Pos2::new(1.0, 550.0);
    let (unselected, _) = frame_with(&ctx, &mut canvas, 0.6, vec![egui::Event::PointerMoved(away)]);
    let (_, intents) = click_with(&ctx, &mut canvas, at, 0.8, egui::Modifiers::NONE, &Shown::default());
    assert_eq!(intents, vec![selecting(&["done"], &[], &[])]);
    let selected = showing(&intents[0]);
    let (painted, _, _) = frame_shown(&ctx, &mut canvas, 1.0, vec![egui::Event::PointerMoved(away)], &selected);
    let threes = |shapes: &[ClippedShape]| texts(shapes).iter().filter(|text| *text == "3").count();
    assert_eq!(threes(&painted), threes(&unselected) + 1, "{:?}", texts(&painted));
    // The click gave the canvas the focus: Escape goes up one level, to the loop that holds `done`.
    let (_, _, intents) = frame_shown(&ctx, &mut canvas, 1.2, vec![key(egui::Key::Escape, egui::Modifiers::NONE)], &selected);
    assert_eq!(intents, vec![selecting(&["n"], &[], &[])]);
    // A click on the wire `m` asks for each of its wires, and pins its values over the 3 iterations while the host shows
    // it selected: a curve from 1 to 3, by hand.
    let m = drawn.groups.iter().position(|group| group.wire.as_ref().is_some_and(|wire| wire.value == "m")).unwrap();
    let wire = drawn.groups[m].wire.as_ref().unwrap();
    assert_eq!(wire.named, ["m -> done.a", "m -> n.next.i"], "`m` also reaches `next i`, named `n.next.i` (commands.md 9.2)");
    let path = &wire.readers[0].1;
    let ((x0, y0), (x1, y1)) = (path[0], path[1]);
    let on_wire = Pos2::new((x0 + x1) as f32 / 200.0, (y0 + y1) as f32 / 200.0);
    let (before, _) = frame_with(&ctx, &mut canvas, 1.4, vec![egui::Event::PointerMoved(away)]);
    assert!(!texts(&before).iter().any(|text| text == "3 iterations"));
    let (_, intents) = click_with(&ctx, &mut canvas, on_wire, 1.6, egui::Modifiers::NONE, &Shown::default());
    assert_eq!(intents, vec![selecting(&[], &wire.named, &[])]);
    let selected = showing(&intents[0]);
    let (pinned, _, _) = frame_shown(&ctx, &mut canvas, 1.8, vec![egui::Event::PointerMoved(away)], &selected);
    let painted = texts(&pinned);
    assert!(["1", "3", "3 iterations"].iter().all(|wanted| painted.iter().any(|text| text == wanted)), "{painted:?}");
    // Escape at the top selects nothing, and the values are pinned no more.
    let (_, _, intents) = frame_shown(&ctx, &mut canvas, 2.0, vec![key(egui::Key::Escape, egui::Modifiers::NONE)], &selected);
    assert_eq!(intents, vec![selecting(&[], &[], &[])]);
    let (unpinned, _, _) = frame_shown(&ctx, &mut canvas, 2.2, vec![egui::Event::PointerMoved(away)], &Shown::default());
    assert!(!texts(&unpinned).iter().any(|text| text == "3 iterations"));
}

/// Three nodes in a row and two inputs: what a click, Shift, a rectangle and the keys select.
const ROW: &str =
    "labcraft 1\n\nuse core\n\nfn main(x: f64, k: f64) -> (y: f64) {\n    a = core.mul(x, k)\n    b = core.add(a, 1.0)\n    y = core.neg(b)\n}\n";

#[test]
fn a_click_shift_a_rectangle_and_the_keys_ask_for_their_selections() {
    let ctx = context();
    install_fonts(&ctx);
    let mut canvas = Canvas::new();
    let drawn = drawing(ROW, &ViewState::default());
    canvas.set_drawing(drawn.clone(), false);
    canvas.set_viewport(1.0, Vec2::ZERO);
    frame_with(&ctx, &mut canvas, 0.0, Vec::new());
    let none = Shown::default();
    // A click on `a`, then Shift and a click on `b`: both; Ctrl and a click on `a` again: `b` alone.
    let (_, intents) = click_with(&ctx, &mut canvas, centre_of(&drawn, "a"), 0.2, egui::Modifiers::NONE, &none);
    assert_eq!(intents, vec![selecting(&["a"], &[], &[])]);
    let a = showing(&intents[0]);
    let (_, intents) = click_with(&ctx, &mut canvas, centre_of(&drawn, "b"), 0.4, egui::Modifiers::SHIFT, &a);
    assert_eq!(intents, vec![selecting(&["a", "b"], &[], &[])]);
    let both = showing(&intents[0]);
    let (_, intents) = click_with(&ctx, &mut canvas, centre_of(&drawn, "a"), 0.6, egui::Modifiers::COMMAND, &both);
    assert_eq!(intents, vec![selecting(&["b"], &[], &[])]);
    // A click on the empty canvas selects nothing.
    let (_, intents) = click_with(&ctx, &mut canvas, Pos2::new(1.0, 550.0), 0.8, egui::Modifiers::NONE, &both);
    assert_eq!(intents, vec![selecting(&[], &[], &[])]);
    // A click on the terminal of the input `k`.
    let k = drawn.hints.iter().find(|hint| hint.selects.as_deref() == Some("input k")).unwrap();
    let on_k = Pos2::new((k.x + k.width / 2) as f32 / 100.0, (k.y + k.height / 2) as f32 / 100.0);
    let (_, intents) = click_with(&ctx, &mut canvas, on_k, 1.0, egui::Modifiers::NONE, &none);
    assert_eq!(intents, vec![selecting(&[], &[], &["input k"])]);
    // A rectangle from the empty corner over `a` and `b`, not `y`: the two of them.
    let (b, y) = (drawn.placed.iter().find(|p| p.name == "b").unwrap(), drawn.placed.iter().find(|p| p.name == "y").unwrap());
    assert!(b.x + b.width < y.x, "`y` is right of `b`");
    let corner = Pos2::new(1.0, 1.0);
    let end = Pos2::new((b.x + b.width / 2) as f32 / 100.0, 540.0);
    let pointer =
        |at, pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    frame_shown(&ctx, &mut canvas, 1.2, vec![egui::Event::PointerMoved(corner), pointer(corner, true)], &none);
    frame_shown(&ctx, &mut canvas, 1.3, vec![egui::Event::PointerMoved(corner + Vec2::new(40.0, 40.0))], &none);
    frame_shown(&ctx, &mut canvas, 1.4, vec![egui::Event::PointerMoved(end)], &none);
    let (_, _, intents) = frame_shown(&ctx, &mut canvas, 1.5, vec![egui::Event::PointerMoved(end), pointer(end, false)], &none);
    assert_eq!(intents, vec![selecting(&["a", "b"], &[], &[])]);
    // The keys, with the focus that the press gave: Tab from `a` to `b`, then to `y`, then to the output; the arrow Left
    // from `b` to `a`.
    let a = showing(&selecting(&["a"], &[], &[]));
    let (_, _, intents) = frame_shown(&ctx, &mut canvas, 1.6, vec![key(egui::Key::Tab, egui::Modifiers::NONE)], &a);
    assert_eq!(intents, vec![selecting(&["b"], &[], &[])]);
    let y = showing(&selecting(&["y"], &[], &[]));
    let (_, _, intents) = frame_shown(&ctx, &mut canvas, 1.7, vec![key(egui::Key::Tab, egui::Modifiers::NONE)], &y);
    assert_eq!(intents, vec![selecting(&[], &[], &["output y"])]);
    let (_, _, intents) = frame_shown(&ctx, &mut canvas, 1.8, vec![key(egui::Key::Tab, egui::Modifiers::SHIFT)], &a);
    assert_eq!(intents, vec![selecting(&[], &[], &["input k"])], "before `a`: the inputs, in the order of the text");
    let b = showing(&selecting(&["b"], &[], &[]));
    let (_, _, intents) = frame_shown(&ctx, &mut canvas, 1.9, vec![key(egui::Key::ArrowLeft, egui::Modifiers::NONE)], &b);
    assert_eq!(intents, vec![selecting(&["a"], &[], &[])]);
    // What feeds `b` and what it feeds; every node; one level then all.
    assert_eq!(canvas.step_selection(Step::Upstream { every: false }, &b).unwrap().nodes, ["a", "b"]);
    assert_eq!(canvas.step_selection(Step::Downstream { every: false }, &a).unwrap().nodes, ["a", "b"]);
    assert_eq!(canvas.step_selection(Step::Downstream { every: true }, &a).unwrap().nodes, ["a", "b", "y"]);
    assert_eq!(canvas.step_selection(Step::All, &none).unwrap().nodes, ["a", "b", "y"]);
    // From a wire: what feeds it is the node it leaves, what it feeds the node it reaches.
    let wire = drawn.groups.iter().find_map(|group| group.wire.as_ref().filter(|wire| wire.value == "a")).unwrap();
    let on_a = showing(&selecting(&[], &wire.named, &[]));
    assert_eq!(canvas.step_selection(Step::Upstream { every: false }, &on_a).unwrap().nodes, ["a"]);
    assert_eq!(canvas.step_selection(Step::Downstream { every: false }, &on_a).unwrap().nodes, ["b"]);
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
    let selection = Selection { nodes: vec![drawing.placed[node].name.clone()], ..Selection::default() };
    let shown = Shown { selection, ..Shown::default() };
    frame_shown(ctx, canvas, time, vec![egui::Event::PointerMoved(Pos2::new(1.0, 550.0))], &shown).0
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

/// A zoom that the host moves counts as moving, as a gesture's (11.3): the frame asks for the one where it settles, and
/// its texts are laid out again there. A zoom set, or a pan, does not.
#[test]
fn a_zoom_moved_by_the_host_settles_as_a_gesture() {
    let ctx = context();
    install_fonts(&ctx);
    let mut canvas = Canvas::new();
    canvas.set_drawing(drawing(ARMS, &ViewState::default()), false);
    let repaint_after = |time: f64, canvas: &mut Canvas| {
        let output = ctx.run_ui(input(time), |ui| {
            canvas.show(ui, Theme::Light, &Shown::default());
        });
        let delay = output.viewport_output.get(&egui::ViewportId::ROOT).map(|viewport| viewport.repaint_delay);
        output.drop_without_applying_deltas();
        delay
    };
    canvas.set_viewport(1.0, Vec2::ZERO);
    // The fonts, the first frames.
    for time in [0.0, 0.1, 0.2] {
        repaint_after(time, &mut canvas);
    }
    // The frame after a change, which may ask for one more at once.
    let after = |time: f64, canvas: &mut Canvas| {
        repaint_after(time, canvas);
        repaint_after(time + 0.05, canvas).unwrap_or_default()
    };
    let never = std::time::Duration::from_secs(3600);
    canvas.set_viewport(0.9, Vec2::ZERO);
    let set = after(1.0, &mut canvas);
    assert!(set > never, "{set:?}");
    canvas.move_viewport(0.8, Vec2::new(10.0, 0.0));
    let moved = after(2.0, &mut canvas);
    assert!(moved > std::time::Duration::ZERO && moved <= std::time::Duration::from_millis(70), "{moved:?}");
    // Settled, then panned only.
    after(2.5, &mut canvas);
    canvas.move_viewport(0.8, Vec2::new(20.0, 0.0));
    let panned = after(3.0, &mut canvas);
    assert!(panned > never, "{panned:?}");
    assert_eq!(canvas.viewport(), (0.8, Vec2::new(20.0, 0.0)));
}

/// `a` is read three times; the line between `a` and `y` has no name.
const UNNAMED: &str = "labcraft 1\n\nuse core\n\nfn main(x: f64) -> (y: f64, z: f64) {\n    a = core.mul(x, 2.0)\n    _ = core.add(a, 1.0)\n    \
                       y = core.neg(a)\n    z = core.mul(a, 3.0)\n}\n";

#[test]
fn a_node_with_no_name_is_named_by_its_line_and_a_click_on_a_wire_asks_for_each_of_its_wires() {
    let ctx = context();
    install_fonts(&ctx);
    let mut canvas = Canvas::new();
    let drawn = drawing(UNNAMED, &ViewState::default());
    canvas.set_drawing(drawn.clone(), false);
    canvas.set_viewport(1.0, Vec2::ZERO);
    frame_with(&ctx, &mut canvas, 0.0, Vec::new());
    // The line `_ = …`, line 7 of the text by hand, has no name: a command names it `@7` (commands.md 9.2).
    assert_eq!(drawn.placed.iter().filter(|placed| placed.name == "@7").count(), 1);
    // The focus from a click on `a`, then Tab: that node, in the order of the text.
    let (_, intents) = click_with(&ctx, &mut canvas, centre_of(&drawn, "a"), 0.2, egui::Modifiers::NONE, &Shown::default());
    let a = showing(&intents[0]);
    let (_, _, intents) = frame_shown(&ctx, &mut canvas, 0.4, vec![key(egui::Key::Tab, egui::Modifiers::NONE)], &a);
    assert_eq!(intents, vec![selecting(&["@7"], &[], &[])]);
    // The wire `a`, read three times: its three wires, as a command names each (commands.md 9.2).
    let group = drawn.groups.iter().position(|group| group.wire.as_ref().is_some_and(|wire| wire.value == "a")).unwrap();
    let wire = drawn.groups[group].wire.as_ref().unwrap();
    assert_eq!(wire.named, ["a -> @7.a", "a -> y.x", "a -> z.a"], "by hand: `add(a, b)`, `neg(x)`, `mul(a, b)`");
    let path = &wire.readers[0].1;
    let on_wire = Pos2::new((path[0].0 + path[1].0) as f32 / 200.0, (path[0].1 + path[1].1) as f32 / 200.0);
    let (_, intents) = click_with(&ctx, &mut canvas, on_wire, 0.6, egui::Modifiers::NONE, &Shown::default());
    assert_eq!(intents, vec![selecting(&[], &wire.named, &[])]);
}

#[test]
fn with_the_setting_a_drag_on_the_empty_canvas_pans_and_shift_with_it_selects() {
    let ctx = context();
    install_fonts(&ctx);
    let mut canvas = Canvas::new();
    let drawn = drawing(ROW, &ViewState::default());
    canvas.set_drawing(drawn.clone(), false);
    canvas.set_viewport(1.0, Vec2::ZERO);
    let pans = Shown { drag_pans: true, ..Shown::default() };
    frame_shown(&ctx, &mut canvas, 0.0, Vec::new(), &pans);
    let drag = |canvas: &mut Canvas, time: f64, modifiers: egui::Modifiers| {
        let (from, to) = (Pos2::new(1.0, 1.0), Pos2::new(1200.0, 540.0));
        let button = |at, pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers };
        let mut intents = Vec::new();
        let steps = [
            vec![egui::Event::ModifiersChanged(modifiers), egui::Event::PointerMoved(from), button(from, true)],
            vec![egui::Event::PointerMoved(from + Vec2::new(40.0, 40.0))],
            vec![egui::Event::PointerMoved(to)],
            vec![egui::Event::PointerMoved(to), button(to, false)],
            vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)],
        ];
        for (at, events) in steps.into_iter().enumerate() {
            intents.extend(frame_shown(&ctx, canvas, time + at as f64 * 0.1, events, &pans).2);
        }
        intents
    };
    // A plain drag pans, and selects nothing.
    let before = canvas.viewport().1;
    assert_eq!(drag(&mut canvas, 0.2, egui::Modifiers::NONE), Vec::new());
    assert_ne!(canvas.viewport().1, before, "the drag panned");
    // A drag from a node does not pan: only the empty canvas does (notation 2.6).
    canvas.set_viewport(1.0, Vec2::ZERO);
    let on_b = centre_of(&drawn, "b");
    let button =
        |at, pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    for (at, events) in [
        vec![egui::Event::PointerMoved(on_b), button(on_b, true)],
        vec![egui::Event::PointerMoved(on_b + Vec2::new(40.0, 40.0))],
        vec![egui::Event::PointerMoved(on_b + Vec2::new(80.0, 80.0)), button(on_b + Vec2::new(80.0, 80.0), false)],
    ]
    .into_iter()
    .enumerate()
    {
        frame_shown(&ctx, &mut canvas, 0.6 + at as f64 * 0.1, events, &pans);
    }
    assert_eq!(canvas.viewport().1, Vec2::ZERO, "a drag from a node never pans");
    // It moves the node instead: the move asked for is put back, as when the host says it failed.
    canvas.cancel_move();
    // Shift and a drag: a rectangle, over the three nodes, from where the drawing is now.
    canvas.set_viewport(1.0, Vec2::ZERO);
    assert_eq!(drag(&mut canvas, 1.0, egui::Modifiers::SHIFT), vec![selecting(&["a", "b", "y"], &[], &[])]);
}

/// A drag of the primary button from `from` to `to` over frames from `time`, the host showing `shown`: what it asks for.
fn drag_with(ctx: &egui::Context, canvas: &mut Canvas, (from, to): (Pos2, Pos2), time: f64, shown: &Shown) -> Vec<Intent> {
    let button =
        |at, pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    let mut intents = Vec::new();
    let steps = [
        vec![egui::Event::PointerMoved(from), button(from, true)],
        vec![egui::Event::PointerMoved(from + (to - from) / 2.0)],
        vec![egui::Event::PointerMoved(to)],
        vec![egui::Event::PointerMoved(to), button(to, false)],
    ];
    for (at, events) in steps.into_iter().enumerate() {
        intents.extend(frame_shown(ctx, canvas, time + at as f64 * 0.1, events, shown).2);
    }
    intents
}

fn moving_of(nodes: &[&str], by: (i64, i64)) -> Intent {
    Intent::Move { nodes: nodes.iter().map(|name| (*name).to_owned()).collect(), by }
}

#[test]
fn a_drag_on_a_node_selects_it_and_asks_to_move_it_by_whole_steps_of_the_grid() {
    let ctx = context();
    install_fonts(&ctx);
    let mut canvas = Canvas::new();
    let drawn = drawing(ROW, &ViewState::default());
    canvas.set_drawing(drawn.clone(), false);
    canvas.set_viewport(1.0, Vec2::ZERO);
    let none = Shown::default();
    frame_shown(&ctx, &mut canvas, 0.0, Vec::new(), &none);
    // 43 across and 21 down, at 100 %: 40 and 24, the nearest steps of 8 (notation 2.6).
    let on_b = centre_of(&drawn, "b");
    let intents = drag_with(&ctx, &mut canvas, (on_b, on_b + Vec2::new(43.0, 21.0)), 0.2, &none);
    assert_eq!(intents, vec![selecting(&["b"], &[], &[]), moving_of(&["b"], (40, 24))]);
    // The canvas keeps `b` where it went until the new drawing comes; a failure puts it back.
    canvas.cancel_move();
    // A drag from a node of the selection moves the selection of its frame.
    let both = showing(&selecting(&["a", "b"], &[], &[]));
    let on_a = centre_of(&drawn, "a");
    let intents = drag_with(&ctx, &mut canvas, (on_a, on_a + Vec2::new(-16.0, 64.0)), 1.0, &both);
    assert_eq!(intents, vec![moving_of(&["a", "b"], (-16, 64))]);
    canvas.cancel_move();
    // Back where it started, it asks for nothing.
    let button =
        |at, pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    let back = on_a + Vec2::new(2.0, -3.0);
    let mut intents = Vec::new();
    for (at, events) in [
        vec![egui::Event::PointerMoved(on_a), button(on_a, true)],
        vec![egui::Event::PointerMoved(on_a + Vec2::new(40.0, 0.0))],
        vec![egui::Event::PointerMoved(back), button(back, false)],
    ]
    .into_iter()
    .enumerate()
    {
        intents.extend(frame_shown(&ctx, &mut canvas, 2.0 + at as f64 * 0.1, events, &both).2);
    }
    assert_eq!(intents, Vec::new());
}

#[test]
fn escape_cancels_a_drag() {
    let ctx = context();
    install_fonts(&ctx);
    let mut canvas = Canvas::new();
    let drawn = drawing(ROW, &ViewState::default());
    canvas.set_drawing(drawn.clone(), false);
    canvas.set_viewport(1.0, Vec2::ZERO);
    let b = showing(&selecting(&["b"], &[], &[]));
    frame_shown(&ctx, &mut canvas, 0.0, Vec::new(), &b);
    let on_b = centre_of(&drawn, "b");
    let button =
        |at, pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    let far = on_b + Vec2::new(80.0, 0.0);
    let mut intents = Vec::new();
    for (at, events) in [
        vec![egui::Event::PointerMoved(on_b), button(on_b, true)],
        vec![egui::Event::PointerMoved(on_b + Vec2::new(40.0, 0.0))],
        vec![egui::Event::PointerMoved(far), key(egui::Key::Escape, egui::Modifiers::NONE)],
        vec![egui::Event::PointerMoved(far), button(far, false)],
    ]
    .into_iter()
    .enumerate()
    {
        intents.extend(frame_shown(&ctx, &mut canvas, 0.2 + at as f64 * 0.1, events, &b).2);
    }
    assert!(!intents.iter().any(|intent| matches!(intent, Intent::Move { .. })), "{intents:?}");
}

#[test]
fn a_run_of_ctrl_and_the_arrows_is_one_move() {
    let ctx = context();
    install_fonts(&ctx);
    let mut canvas = Canvas::new();
    let drawn = drawing(ROW, &ViewState::default());
    canvas.set_drawing(drawn.clone(), false);
    canvas.set_viewport(1.0, Vec2::ZERO);
    let b = showing(&selecting(&["b"], &[], &[]));
    // The focus comes with a click.
    frame_shown(&ctx, &mut canvas, 0.0, Vec::new(), &b);
    click_with(&ctx, &mut canvas, centre_of(&drawn, "b"), 0.1, egui::Modifiers::NONE, &b);
    let ctrl = egui::Modifiers::COMMAND;
    let ctrl_shift = egui::Modifiers { shift: true, ..egui::Modifiers::COMMAND };
    let (_, _, intents) = frame_shown(&ctx, &mut canvas, 0.5, vec![key(egui::Key::ArrowRight, ctrl)], &b);
    assert_eq!(intents, Vec::new(), "a run asks for nothing while it goes on");
    let (_, _, intents) = frame_shown(&ctx, &mut canvas, 0.9, vec![key(egui::Key::ArrowDown, ctrl_shift)], &b);
    assert_eq!(intents, Vec::new());
    // A second after the last press, the run is one move: 8 right, 64 down.
    let (_, _, intents) = frame_shown(&ctx, &mut canvas, 1.5, Vec::new(), &b);
    assert_eq!(intents, Vec::new());
    let (_, _, intents) = frame_shown(&ctx, &mut canvas, 1.95, Vec::new(), &b);
    assert_eq!(intents, vec![moving_of(&["b"], (8, 64))]);
    canvas.cancel_move();
    // Another key ends a run at once, and so does the host before a key of its own.
    frame_shown(&ctx, &mut canvas, 3.0, vec![key(egui::Key::ArrowLeft, ctrl)], &b);
    let (_, _, intents) = frame_shown(&ctx, &mut canvas, 3.1, vec![key(egui::Key::Tab, egui::Modifiers::NONE)], &b);
    assert_eq!(intents.first(), Some(&moving_of(&["b"], (-8, 0))));
    canvas.cancel_move();
    frame_shown(&ctx, &mut canvas, 4.0, vec![key(egui::Key::ArrowUp, ctrl)], &b);
    assert_eq!(canvas.end_run(), Some(moving_of(&["b"], (0, -8))));
    assert_eq!(canvas.end_run(), None);
}

/// `b` fixed by the file.
const FIXED: &str = "labcraft 1\n\nuse core\n\nfn main(x: f64, k: f64) -> (y: f64) {\n    a = core.mul(x, k)\n    b = core.add(a, 1.0)\n    y = core.neg(b)\n}\n\nlayout {\n    node b at (400, 200)\n}\n";

#[test]
fn a_fixed_node_shows_its_pin_under_the_setting() {
    let ctx = context();
    install_fonts(&ctx);
    let mut canvas = Canvas::new();
    let drawn = drawing(FIXED, &ViewState::default());
    assert!(drawn.placed.iter().any(|placed| placed.name == "b" && placed.at == Some((40_000, 20_000))));
    canvas.set_drawing(drawn.clone(), false);
    canvas.set_viewport(1.0, Vec2::ZERO);
    let lines = |shapes: &[ClippedShape]| shapes.iter().filter(|shape| matches!(shape.shape, egui::Shape::Path(_))).count();
    let marked = Shown { mark_fixed: true, ..Shown::default() };
    let (with, _, _) = frame_shown(&ctx, &mut canvas, 0.0, Vec::new(), &marked);
    let (without, _, _) = frame_shown(&ctx, &mut canvas, 0.1, Vec::new(), &Shown::default());
    assert!(lines(&with) > lines(&without), "the pin is drawn with the setting only");
    assert_eq!(
        crate::pins::tooltip(&drawn, drawn.placed.iter().position(|placed| placed.name == "b").unwrap()),
        "Position fixed (`node b at (400, 200)` in `layout`). Right-click › Remove the fixed position"
    );
}

#[test]
fn the_nodes_moved_do_not_glide_to_the_new_drawing() {
    let before = drawing(ROW, &ViewState::default());
    let b = before.placed.iter().position(|placed| placed.name == "b").unwrap();
    let (x, y) = (before.placed[b].x, before.placed[b].y);
    // The drawing after the move, its node where the move put it: 40 right of where it was.
    let mut after = (*before).clone();
    after.placed[b].x = x + 4_000;
    after.placed[b].y = y;
    let glide = crate::view::Glide::after_move(&before, &after, &[b], (4_000, 0));
    assert!(glide.is_none(), "nothing glides: the node is where the drag left it");
    assert!(crate::view::Glide::between(&before, &after).is_some());
}

#[test]
fn a_drawing_that_comes_during_a_drag_keeps_it() {
    let ctx = context();
    install_fonts(&ctx);
    let mut canvas = Canvas::new();
    let drawn = drawing(ROW, &ViewState::default());
    canvas.set_drawing(drawn.clone(), false);
    canvas.set_viewport(1.0, Vec2::ZERO);
    let b = showing(&selecting(&["b"], &[], &[]));
    frame_shown(&ctx, &mut canvas, 0.0, Vec::new(), &b);
    let on_b = centre_of(&drawn, "b");
    let button =
        |at, pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    let to = on_b + Vec2::new(48.0, 0.0);
    frame_shown(&ctx, &mut canvas, 0.1, vec![egui::Event::PointerMoved(on_b), button(on_b, true)], &b);
    frame_shown(&ctx, &mut canvas, 0.2, vec![egui::Event::PointerMoved(on_b + Vec2::new(24.0, 0.0))], &b);
    // Another file changed: a new drawing of the same function, its nodes where they were.
    canvas.set_drawing(Arc::new((*drawn).clone()), true);
    frame_shown(&ctx, &mut canvas, 0.3, vec![egui::Event::PointerMoved(to)], &b);
    let (_, _, intents) = frame_shown(&ctx, &mut canvas, 0.4, vec![egui::Event::PointerMoved(to), button(to, false)], &b);
    assert_eq!(intents, vec![moving_of(&["b"], (48, 0))]);
}

/// Two nodes in a row, a value of another family beside them: the wiring of notation 2.4 to 2.6.
const WIRED: &str =
    "labcraft 1\n\nuse core\n\nfn main(x: f64, flag: bool) -> (y: f64) {\n    a = core.add(x, 1.0)\n    b = core.mul(a, 2.0)\n    y = b\n}\n";

/// The centre of the port that a command names `name`, an input or an output, in units at 100 % from the corner.
fn port_of(drawing: &Drawing, name: &str, input: bool) -> Pos2 {
    let port = drawing.ports.iter().find(|port| port.name == name && port.input == input).unwrap_or_else(|| panic!("no port {name}"));
    Pos2::new(port.x as f32, port.y as f32) / 100.0
}

fn wiring_of(from: &str, to: &str, replace: bool) -> Intent {
    Intent::Wire { from: from.to_owned(), to: to.to_owned(), arm: None, replace, accept_mismatch: false, route: None, through: Vec::new() }
}

/// A drag of the primary button from `from` to `to`, with `modifiers` held, over frames from `time`: what it asks for.
fn drag_holding(
    ctx: &egui::Context,
    canvas: &mut Canvas,
    (from, to): (Pos2, Pos2),
    time: f64,
    modifiers: egui::Modifiers,
    shown: &Shown,
) -> Vec<Intent> {
    let button = |at, pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers };
    let held = egui::Event::ModifiersChanged(modifiers);
    let mut intents = Vec::new();
    let steps = [
        vec![held.clone(), egui::Event::PointerMoved(from), button(from, true)],
        vec![held.clone(), egui::Event::PointerMoved(from + (to - from) / 2.0)],
        vec![held.clone(), egui::Event::PointerMoved(to)],
        vec![held, egui::Event::PointerMoved(to), button(to, false)],
    ];
    for (at, events) in steps.into_iter().enumerate() {
        intents.extend(frame_shown(ctx, canvas, time + at as f64 * 0.1, events, shown).2);
    }
    intents.extend(frame_shown(ctx, canvas, time + 0.5, vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)], shown).2);
    intents
}

/// A drag of the primary button from `from` to `to`, Alt pressed before the drop, over frames from `time`.
fn drag_then_alt(ctx: &egui::Context, canvas: &mut Canvas, (from, to): (Pos2, Pos2), time: f64, shown: &Shown) -> Vec<Intent> {
    let button = |at, pressed, modifiers| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers };
    let none = egui::Modifiers::NONE;
    let mut intents = Vec::new();
    let steps = [
        vec![egui::Event::PointerMoved(from), button(from, true, none)],
        vec![egui::Event::PointerMoved(from + (to - from) / 2.0)],
        vec![egui::Event::ModifiersChanged(egui::Modifiers::ALT), egui::Event::PointerMoved(to)],
        vec![egui::Event::ModifiersChanged(egui::Modifiers::ALT), egui::Event::PointerMoved(to), button(to, false, egui::Modifiers::ALT)],
        vec![egui::Event::ModifiersChanged(none)],
    ];
    for (at, events) in steps.into_iter().enumerate() {
        intents.extend(frame_shown(ctx, canvas, time + at as f64 * 0.1, events, shown).2);
    }
    intents
}

/// A drag from an output to an input asks for `wire.connect`, `replace` when the input holds something; from an input,
/// back to an output; a drop near the input snaps to it (notation 2.5, 2.6).
#[test]
fn a_wire_dragged_from_a_terminal_to_one_that_takes_it_asks_to_connect_them() {
    let ctx = context();
    install_fonts(&ctx);
    let mut canvas = Canvas::new();
    let drawn = drawing(WIRED, &ViewState::default());
    canvas.set_drawing(drawn.clone(), false);
    canvas.set_viewport(1.0, Vec2::ZERO);
    let none = Shown::default();
    frame_shown(&ctx, &mut canvas, 0.0, Vec::new(), &none);
    // To the constant `2.0` of `b`, which the wire replaces; dropped 5 units off it, it snaps.
    let (from, to) = (port_of(&drawn, "a", false), port_of(&drawn, "b.b", true));
    let intents = drag_with(&ctx, &mut canvas, (from, to + Vec2::new(5.0, 3.0)), 0.2, &none);
    assert_eq!(intents, vec![wiring_of("a", "b.b", true)]);
    canvas.cancel_wire();
    // From the input `a.b` back to the input `x` of the function.
    let intents = drag_with(&ctx, &mut canvas, (port_of(&drawn, "a.b", true), port_of(&drawn, "x", false)), 1.0, &none);
    assert_eq!(intents, vec![wiring_of("x", "a.b", true)]);
    canvas.cancel_wire();
    // To the output `y` of the function, which `b` gives already: it replaces that wire.
    let intents = drag_with(&ctx, &mut canvas, (from, port_of(&drawn, "output y", true)), 2.0, &none);
    assert_eq!(intents, vec![wiring_of("a", "output y", true)]);
}

/// A wire that the input would refuse writes nothing, and says why (notation 2.5): a value of another family, a cycle,
/// an input that holds something while "replace only with Alt" is on and Alt is up; dropped on the empty canvas, it asks
/// for quick insertion (notation 2.6).
#[test]
fn a_wire_that_the_input_refuses_writes_nothing_and_says_why() {
    let ctx = context();
    install_fonts(&ctx);
    let mut canvas = Canvas::new();
    let drawn = drawing(WIRED, &ViewState::default());
    canvas.set_drawing(drawn.clone(), false);
    canvas.set_viewport(1.0, Vec2::ZERO);
    let none = Shown::default();
    frame_shown(&ctx, &mut canvas, 0.0, Vec::new(), &none);
    let said = |intents: Vec<Intent>| match intents.as_slice() {
        [Intent::Say(text)] => text.clone(),
        other => panic!("a reason expected, got {other:?}"),
    };
    // `core.mul` takes numbers (spec A.1): from its input back to `flag`.
    let text = said(drag_with(&ctx, &mut canvas, (port_of(&drawn, "b.b", true), port_of(&drawn, "flag", false)), 0.2, &none));
    assert_eq!(text, "`bool` where `a number` is expected");
    // `b` reads `a`: a wire from `b` back into `a` closes a cycle.
    let text = said(drag_with(&ctx, &mut canvas, (port_of(&drawn, "b", false), port_of(&drawn, "a.b", true)), 1.0, &none));
    assert!(text.contains("cycle"), "{text}");
    // With "replace only with Alt", the constant stays unless Alt is down.
    let alt = Shown { replace_with_alt: true, ..Shown::default() };
    let (from, to) = (port_of(&drawn, "a", false), port_of(&drawn, "b.b", true));
    let text = said(drag_with(&ctx, &mut canvas, (from, to), 2.0, &alt));
    assert_eq!(text, "`b` already holds `2.0`: hold Alt to replace it");
    // Alt read at the drop: held when the drag starts, it would move the terminal (notation 2.4).
    let intents = drag_then_alt(&ctx, &mut canvas, (from, to), 3.0, &alt);
    assert_eq!(intents, vec![wiring_of("a", "b.b", true)]);
    canvas.cancel_wire();
    // With "accept it", a wire of the wrong type is written.
    let accept = Shown { accept_mismatch: true, ..Shown::default() };
    let intents = drag_with(&ctx, &mut canvas, (to, port_of(&drawn, "flag", false)), 4.0, &accept);
    assert_eq!(
        intents,
        vec![Intent::Wire {
            from: "flag".to_owned(),
            to: "b.b".to_owned(),
            arm: None,
            replace: true,
            accept_mismatch: true,
            route: None,
            through: Vec::new()
        }]
    );
    canvas.cancel_wire();
    // On the empty canvas, far from every terminal.
    let intents = drag_with(&ctx, &mut canvas, (from, from + Vec2::new(0.0, 160.0)), 5.0, &none);
    assert!(
        matches!(intents.as_slice(), [Intent::QuickInsert(Quick { source: Source::Terminal { name, replace: false, .. }, at: Some(_), .. })] if name == "a"),
        "{intents:?}"
    );
}

/// Ctrl and a drag on a wired input takes the end of its wire to another input: the old wire goes, the new one comes, in
/// one batch (notation 2.6).
#[test]
fn ctrl_and_a_drag_on_a_wired_input_takes_its_wire_to_another_input() {
    let ctx = context();
    install_fonts(&ctx);
    let mut canvas = Canvas::new();
    let drawn = drawing(WIRED, &ViewState::default());
    canvas.set_drawing(drawn.clone(), false);
    canvas.set_viewport(1.0, Vec2::ZERO);
    let none = Shown::default();
    frame_shown(&ctx, &mut canvas, 0.0, Vec::new(), &none);
    let ends = (port_of(&drawn, "b.a", true), port_of(&drawn, "b.b", true));
    let intents = drag_holding(&ctx, &mut canvas, ends, 0.2, egui::Modifiers::COMMAND, &none);
    assert_eq!(
        intents,
        vec![Intent::Rewire {
            disconnect: vec![("a -> b.a".to_owned(), None)],
            connect: vec![("a".to_owned(), "b.b".to_owned(), true, false, None)]
        }]
    );
}

/// Enter on a node goes to its terminals, the inputs first; W starts a wire from the one in focus, Tab goes through the
/// terminals that would take it, in the order of the text, and Enter wires it (notation 2.6).
#[test]
fn the_keyboard_draws_a_wire_from_a_terminal() {
    let ctx = context();
    install_fonts(&ctx);
    let mut canvas = Canvas::new();
    let drawn = drawing(WIRED, &ViewState::default());
    canvas.set_drawing(drawn.clone(), false);
    canvas.set_viewport(1.0, Vec2::ZERO);
    let b = showing(&selecting(&["b"], &[], &[]));
    // The focus comes with a click on `b`.
    frame_shown(&ctx, &mut canvas, 0.0, Vec::new(), &b);
    click_with(&ctx, &mut canvas, centre_of(&drawn, "b"), 0.05, egui::Modifiers::NONE, &b);
    frame_shown(&ctx, &mut canvas, 0.2, vec![key(egui::Key::Enter, egui::Modifiers::NONE)], &b);
    assert_eq!(canvas.port_focus(), Some(("b.a".to_owned(), true)));
    frame_shown(&ctx, &mut canvas, 0.3, vec![key(egui::Key::ArrowDown, egui::Modifiers::NONE)], &b);
    assert_eq!(canvas.port_focus(), Some(("b.b".to_owned(), true)));
    // W: the wire starts at the first output that takes it, `a`; Tab goes to the input `x` of the function, past `flag`,
    // a bool, and `b`, which would close a cycle.
    assert!(canvas.start_wire(&b));
    frame_shown(&ctx, &mut canvas, 0.4, vec![key(egui::Key::Tab, egui::Modifiers::NONE)], &b);
    let (_, _, intents) = frame_shown(&ctx, &mut canvas, 0.5, vec![key(egui::Key::Enter, egui::Modifiers::NONE)], &b);
    assert_eq!(intents, vec![wiring_of("x", "b.b", true)]);
}

/// The terminal in focus belongs to the node selected: another selection takes the focus away, and Space on a terminal
/// asks for quick insertion (notation 2.6). A drag on a terminal of the frame of the function moves it along its edge
/// and draws no wire; "accept it" from a wired input replaces what it holds.
#[test]
fn the_focus_of_a_terminal_follows_the_selection_and_a_drag_on_the_frame_draws_no_wire() {
    let ctx = context();
    install_fonts(&ctx);
    let mut canvas = Canvas::new();
    let drawn = drawing(WIRED, &ViewState::default());
    canvas.set_drawing(drawn.clone(), false);
    canvas.set_viewport(1.0, Vec2::ZERO);
    let b = showing(&selecting(&["b"], &[], &[]));
    frame_shown(&ctx, &mut canvas, 0.0, Vec::new(), &b);
    click_with(&ctx, &mut canvas, centre_of(&drawn, "b"), 0.05, egui::Modifiers::NONE, &b);
    frame_shown(&ctx, &mut canvas, 0.2, vec![key(egui::Key::Enter, egui::Modifiers::NONE)], &b);
    assert_eq!(canvas.port_focus(), Some(("b.a".to_owned(), true)));
    // Space is a key of the host (`ide.quick_insert`), which asks the canvas.
    let quick = canvas.quick_insert(&b, true, None);
    assert!(matches!(&quick, Some(Quick { source: Source::Terminal { name, .. }, .. }) if name == "b.a"), "{quick:?}");
    let a = showing(&selecting(&["a"], &[], &[]));
    frame_shown(&ctx, &mut canvas, 0.4, Vec::new(), &a);
    assert_eq!(canvas.port_focus(), None);
    // From `x`, a terminal of the frame: no wire; it moves along its edge, up by 48 to the place on the grid of 8 nearest
    // there, from the origin of the frame (notation 2.4, spec 11). `flag` is below it: that place is free.
    let x = port_of(&drawn, "x", false);
    let origin = drawn.bodies[0].origin.1 as f32 / 100.0;
    let intents = drag_with(&ctx, &mut canvas, (x, x - Vec2::new(0.0, 48.0)), 1.0, &a);
    let expected = ((x.y - origin - 48.0) / 8.0).round() as i64 * 8;
    assert_eq!(intents, vec![Intent::MoveTerminal { terminal: "input x".to_owned(), y: expected }]);
    // From the input `b.b`, which holds `2.0`, back to `flag`, a bool, with "accept it": written, replacing the constant.
    let accept = Shown { accept_mismatch: true, ..Shown::default() };
    let intents = drag_with(&ctx, &mut canvas, (port_of(&drawn, "b.b", true), port_of(&drawn, "flag", false)), 2.0, &accept);
    assert_eq!(
        intents,
        vec![Intent::Wire {
            from: "flag".to_owned(),
            to: "b.b".to_owned(),
            arm: None,
            replace: true,
            accept_mismatch: true,
            route: None,
            through: Vec::new()
        }]
    );
}

/// A drawing that comes while a wire is drawn keeps it, from its terminal found again (as a drag of nodes); Enter on a
/// wired output selects its wire, its whole value (notation 2.6).
#[test]
fn a_drawing_that_comes_while_a_wire_is_drawn_keeps_it() {
    let ctx = context();
    install_fonts(&ctx);
    let mut canvas = Canvas::new();
    let drawn = drawing(WIRED, &ViewState::default());
    canvas.set_drawing(drawn.clone(), false);
    canvas.set_viewport(1.0, Vec2::ZERO);
    let none = Shown::default();
    frame_shown(&ctx, &mut canvas, 0.0, Vec::new(), &none);
    let (from, to) = (port_of(&drawn, "a", false), port_of(&drawn, "b.b", true));
    let button =
        |at, pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    let mut intents = Vec::new();
    intents.extend(frame_shown(&ctx, &mut canvas, 0.1, vec![egui::Event::PointerMoved(from), button(from, true)], &none).2);
    intents.extend(frame_shown(&ctx, &mut canvas, 0.2, vec![egui::Event::PointerMoved(from + (to - from) / 2.0)], &none).2);
    canvas.set_drawing(drawing(WIRED, &ViewState::default()), true);
    intents.extend(frame_shown(&ctx, &mut canvas, 0.3, vec![egui::Event::PointerMoved(to)], &none).2);
    intents.extend(frame_shown(&ctx, &mut canvas, 0.4, vec![egui::Event::PointerMoved(to), button(to, false)], &none).2);
    assert_eq!(intents, vec![wiring_of("a", "b.b", true)]);
    canvas.cancel_wire();
    // The output of `a` in focus: Enter selects its wire.
    let a = showing(&selecting(&["a"], &[], &[]));
    frame_shown(&ctx, &mut canvas, 1.0, Vec::new(), &a);
    click_with(&ctx, &mut canvas, centre_of(&drawn, "a"), 1.05, egui::Modifiers::NONE, &a);
    frame_shown(&ctx, &mut canvas, 1.2, vec![key(egui::Key::Enter, egui::Modifiers::NONE)], &a);
    frame_shown(&ctx, &mut canvas, 1.3, vec![key(egui::Key::ArrowRight, egui::Modifiers::NONE)], &a);
    assert_eq!(canvas.port_focus(), Some(("a".to_owned(), false)));
    let (_, _, intents) = frame_shown(&ctx, &mut canvas, 1.4, vec![key(egui::Key::Enter, egui::Modifiers::NONE)], &a);
    assert!(matches!(intents.as_slice(), [Intent::Select(selection)] if selection.wires.contains(&"a -> b.a".to_owned())), "{intents:?}");
}

/// The path of the first reader of the wire of `value`, in hundredths, and the origin of its frame.
fn path_of(drawing: &Drawing, value: &str) -> (Vec<(i64, i64)>, (i64, i64)) {
    let wire = drawing.groups.iter().find_map(|group| group.wire.as_ref().filter(|wire| wire.value == value)).unwrap();
    (wire.readers[0].1.clone(), drawing.bodies[wire.body].origin)
}

fn units_of((x, y): (i64, i64)) -> Pos2 {
    Pos2::new(x as f32 / 100.0, y as f32 / 100.0)
}

/// Alt and a drag on a segment moves it across, on the grid of 8 of its frame, and the drop asks for `wire.route` with
/// the points that give that route: a jog 16 from each terminal it leaves (notation 2.4, 2.6).
#[test]
fn alt_and_a_drag_on_a_segment_routes_the_wire() {
    let ctx = context();
    install_fonts(&ctx);
    let mut canvas = Canvas::new();
    let drawn = drawing(CHAIN, &ViewState::default());
    canvas.set_drawing(drawn.clone(), false);
    canvas.set_viewport(1.0, Vec2::ZERO);
    let none = Shown::default();
    frame_shown(&ctx, &mut canvas, 0.0, Vec::new(), &none);
    let (path, (ox, oy)) = path_of(&drawn, "a");
    // `a` rises in the channel to `y`: its last segment, into `y`, moves.
    assert_eq!(path.len(), 4, "{path:?}");
    let (p1, p2, p3) = (path[1], path[2], path[3]);
    let on = units_of(((p2.0 + p3.0) / 2, p2.1));
    let intents = drag_holding(&ctx, &mut canvas, (on, on + Vec2::new(5.0, 21.0)), 0.2, egui::Modifiers::ALT, &none);
    // Down by 21, to the nearest line of the grid of 8 that starts at the origin of the frame; the vertical before it
    // stretches, and the segment jogs 16 before `y`, which it reaches at its height.
    let moved = oy + ((p2.1 + 2_100 - oy) as f64 / 800.0).round() as i64 * 800;
    let unit = |hundredths: i64| (hundredths + 50).div_euclid(100);
    let via = vec![(unit(p1.0 - ox), unit(moved - oy)), (unit(p3.0 - 1_600 - ox), unit(p3.1 - oy))];
    assert_eq!(intents.last(), Some(&Intent::Route(vec![("a -> y.a".to_owned(), via)])), "{intents:?}");
    // With the setting, Alt alone draws a branch instead, and Shift+Alt moves the segment.
    let shifted = Shown { route_with_shift_alt: true, ..Shown::default() };
    canvas.cancel_wire();
    let intents = drag_holding(&ctx, &mut canvas, (on, on + Vec2::new(0.0, 21.0)), 2.0, egui::Modifiers::ALT | egui::Modifiers::SHIFT, &shifted);
    assert!(intents.iter().any(|intent| matches!(intent, Intent::Route(_))), "{intents:?}");
}

/// A plain drag on a wire draws a branch of its value from a junction on the grid of 8 on the wire; dropped on an
/// input of its frame, it asks for the wire and its junction in one step (notation 2.4, 2.5).
#[test]
fn a_drag_on_a_wire_draws_a_branch_from_a_junction() {
    let ctx = context();
    install_fonts(&ctx);
    let mut canvas = Canvas::new();
    let drawn = drawing(CHAIN, &ViewState::default());
    canvas.set_drawing(drawn.clone(), false);
    canvas.set_viewport(1.0, Vec2::ZERO);
    let none = Shown::default();
    frame_shown(&ctx, &mut canvas, 0.0, Vec::new(), &none);
    let (path, (ox, oy)) = path_of(&drawn, "a");
    let ((sx, sy), (rx, _)) = (path[0], path[1]);
    let press = (sx + (rx - sx) / 2 + 300, sy + 200);
    // The junction: on the wire, at the line of the grid of 8 of the frame nearest the press.
    let jx = ox + ((press.0 - ox) as f64 / 800.0).round() as i64 * 800;
    let input = drawn.ports.iter().find(|port| port.name == "y.b").unwrap();
    let target = units_of((input.x, input.y));
    let intents = drag_with(&ctx, &mut canvas, (units_of(press), target), 0.2, &none);
    let unit = |hundredths: i64| (hundredths + 50).div_euclid(100);
    let wired = Intent::Wire {
        from: "a".to_owned(),
        to: "y.b".to_owned(),
        arm: None,
        replace: true,
        accept_mismatch: false,
        route: Some(("a -> y.b".to_owned(), vec![(unit(jx - ox), unit(sy - oy))])),
        through: Vec::new(),
    };
    assert_eq!(intents.last(), Some(&wired), "{intents:?}");
}

const FORK: &str = "labcraft 1\n\nuse core\n\nfn main(x: f64) -> (y: f64, z: f64) {\n    a = core.mul(x, 2.0)\n    y = core.add(a, 1.0)\n    z = core.add(a, 2.0)\n}\n";

/// A segment that two wires of a value share moves for both: the one dragged gets the route, and the other, which ran
/// along it, becomes its branch, its junction where it leaves it, moved with the segment (decision of the owner,
/// 2026-11-17). Escape before the drop asks for nothing.
#[test]
fn a_segment_shared_moves_for_every_wire_along_it() {
    let ctx = context();
    install_fonts(&ctx);
    let mut canvas = Canvas::new();
    let drawn = drawing(FORK, &ViewState::default());
    canvas.set_drawing(drawn.clone(), false);
    canvas.set_viewport(1.0, Vec2::ZERO);
    let none = Shown::default();
    frame_shown(&ctx, &mut canvas, 0.0, Vec::new(), &none);
    let wire = drawn.groups.iter().find_map(|group| group.wire.as_ref().filter(|wire| wire.value == "a")).unwrap();
    let (ox, oy) = drawn.bodies[wire.body].origin;
    let paths: Vec<&Vec<(i64, i64)>> = wire.readers.iter().map(|(_, path)| path).collect();
    // Both leave `a` across, then down the same vertical of the channel, each off it at the height of its reader.
    assert_eq!((paths[0].len(), paths[1].len()), (4, 4), "{paths:?}");
    assert_eq!((paths[0][1], paths[0][0]), (paths[1][1], paths[1][0]), "{paths:?}");
    let longer = if (paths[0][2].1 - paths[0][1].1).abs() >= (paths[1][2].1 - paths[1][1].1).abs() { 0 } else { 1 };
    let (dragged, other) = (paths[longer], paths[1 - longer]);
    let cx = dragged[1].0;
    let on = units_of((cx, (dragged[1].1 + other[2].1) / 2));
    // Escape before the drop: nothing.
    let escaped = {
        let button =
            |at, pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::ALT };
        let mut intents = Vec::new();
        for (at, events) in [
            vec![egui::Event::ModifiersChanged(egui::Modifiers::ALT), egui::Event::PointerMoved(on), button(on, true)],
            vec![egui::Event::PointerMoved(on + Vec2::new(10.0, 0.0))],
            vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
            vec![egui::Event::PointerMoved(on + Vec2::new(12.0, 0.0)), button(on + Vec2::new(12.0, 0.0), false)],
            vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)],
        ]
        .into_iter()
        .enumerate()
        {
            intents.extend(frame_shown(&ctx, &mut canvas, 0.1 + at as f64 * 0.1, events, &none).2);
        }
        intents
    };
    assert!(!escaped.iter().any(|intent| matches!(intent, Intent::Route(_))), "{escaped:?}");
    let intents = drag_holding(&ctx, &mut canvas, (on, on + Vec2::new(10.0, 0.0)), 1.0, egui::Modifiers::ALT, &none);
    let Some(Intent::Route(routes)) = intents.last() else { panic!("{intents:?}") };
    let names: Vec<&str> = routes.iter().map(|(name, _)| name.as_str()).collect();
    let (first, second) = if longer == 0 { ("a -> y.a", "a -> z.a") } else { ("a -> z.a", "a -> y.a") };
    assert_eq!(names, [first, second]);
    // Right by 10, to the nearest line of the grid of 8 of the frame; the other leaves the vertical moved, at its height.
    let moved = ox + ((cx + 1_000 - ox) as f64 / 800.0).round() as i64 * 800;
    let unit = |hundredths: i64| (hundredths + 50).div_euclid(100);
    assert_eq!(routes[1].1.first(), Some(&(unit(moved - ox), unit(other[2].1 - oy))), "{routes:?}");
    // With the setting, Alt alone draws a branch instead.
    canvas.cancel_wire();
    let shifted = Shown { route_with_shift_alt: true, ..Shown::default() };
    let intents = drag_holding(&ctx, &mut canvas, (on, on + Vec2::new(10.0, 0.0)), 3.0, egui::Modifiers::ALT, &shifted);
    assert!(!intents.iter().any(|intent| matches!(intent, Intent::Route(_))), "{intents:?}");
}

/// A new drawing that draws the wire as it was does not end a segment being moved; one that draws it elsewhere does.
#[test]
fn a_segment_being_moved_goes_on_over_a_new_drawing() {
    let drawn = drawing(CHAIN, &ViewState::default());
    let (path, _) = path_of(&drawn, "a");
    let group = drawn.groups.iter().position(|group| group.wire.as_ref().is_some_and(|wire| wire.value == "a")).unwrap();
    let on = units_of(((path[2].0 + path[3].0) / 2, path[2].1));
    let mut moving = rerouting::Rerouting::grab(&drawn, group, on, 1.0).unwrap();
    moving.by = 800;
    let again = moving.again(&drawn, &drawing(CHAIN, &ViewState::default())).unwrap();
    assert_eq!((again.segment, again.by), (moving.segment, 800));
    let other = drawing(&CHAIN.replace("core.add(a, 1.0)", "core.add(1.0, a)"), &ViewState::default());
    assert!(moving.again(&drawn, &other).is_none());
}

/// The fields in place of M1 step 4b (notation 2.6): what a double-click finds is the text that the drawing writes.
#[test]
fn a_field_in_place_opens_on_what_the_drawing_writes() {
    let text = "labcraft 1

use core
use string

fn main(x: f64) -> (y: f64, t: string) {
    sum = core.add(x, 1.5)
    t = string.format(\"{v}\", v: sum)
    y = sum
}
";
    let drawn = drawing(text, &ViewState::default());
    let placed = |name: &str| drawn.placed.iter().position(|found| found.name == name).unwrap();
    let texts: Vec<String> = fields::texts_of(&drawn, placed("sum")).into_iter().map(|written| written.text).collect();
    assert!(texts.iter().any(|text| text == "1.5"), "the pill of the constant: {texts:?}");
    let mut canvas = Canvas::new();
    canvas.set_drawing(drawn.clone(), false);
    assert!(canvas.edit_template("t"), "the template of a `string.format`");
    let field = canvas.field.as_ref().unwrap();
    // Between its quotes, which the host writes back.
    assert_eq!((&field.kind, field.text.as_str()), (&FieldKind::Template { node: "t".to_owned() }, "{v}"));
    canvas.field_refused("no");
    canvas.close_field();
    assert!(!canvas.field_open());
    assert!(!canvas.edit_template("sum"), "not a `string.format`");
}

/// A field opens on the constant as the file writes it, escapes kept, not on what the pill shows (notation 2.6).
#[test]
fn a_field_opens_on_the_text_of_the_file() {
    let text = "labcraft 1\n\nuse string\n\nfn main(x: f64) -> (t: string) {\n    t = string.format(\"say \\\"{v}\\\"\", v: x)\n}\n";
    let drawn = drawing(text, &ViewState::default());
    let mut canvas = Canvas::new();
    canvas.set_drawing(drawn, false);
    assert!(canvas.edit_template("t"));
    assert_eq!(canvas.field.as_ref().map(|field| field.text.as_str()), Some("say \\\"{v}\\\""));
}

/// Two pills that read the same: a double-click on one edits its own input (notation 2.6).
#[test]
fn a_pill_double_clicked_edits_its_own_input() {
    let text = "labcraft 1\n\nuse core\n\nfn main(x: f64) -> (y: f64) {\n    sum = core.add(2.0, 2.0)\n    y = core.mul(x, sum)\n}\n";
    let drawn = drawing(text, &ViewState::default());
    let placed = drawn.placed.iter().position(|found| found.name == "sum").unwrap();
    let pills: Vec<Rect> = fields::texts_of(&drawn, placed).into_iter().filter(|written| written.text == "2.0").map(|written| written.rect).collect();
    assert_eq!(pills.len(), 2, "two pills");
    let port = |name: &str| drawn.ports.iter().find(|port| port.name == name).map(|port| Pos2::new(port.x as f32, port.y as f32) / 100.0).unwrap();
    let nearest = |to: Pos2| *pills.iter().min_by(|a, b| a.right_center().distance(to).total_cmp(&b.right_center().distance(to))).unwrap();
    for to in ["sum.a", "sum.b"] {
        let pill = nearest(port(to));
        let (kind, shown, _) = fields::input_under(&drawn, placed, pill.center(), Vec2::ZERO).unwrap();
        assert_eq!((kind, shown.as_str()), (FieldKind::Constant { to: to.to_owned(), held: true }, "2.0"));
    }
    // F2 on the node selected renames its value; Delete in focus takes the constant out; a field waiting for its answer
    // lets no other open.
    let mut canvas = Canvas::new();
    canvas.set_drawing(drawn.clone(), false);
    canvas.found = select::Found::one(select::Hit::Node(placed));
    assert!(canvas.edit_name());
    assert_eq!(canvas.field.as_ref().map(|field| field.kind.clone()), Some(FieldKind::Rename { value: "sum".to_owned() }));
    canvas.close_field();
    canvas.port_focus = drawn.ports.iter().position(|found| found.name == "sum.b");
    assert_eq!(canvas.constant_in_focus().as_deref(), Some("sum.b"));
    assert_eq!(fields::unquoted("\"{v} V\""), "{v} V");
}

/// Copy counts from the top-left corner of the nodes selected, as their `node … at` entries write it (spec 11), so that
/// Duplicate falls 16 to the right and 16 below; a paste without the pointer on the canvas falls in the view, on the grid
/// of 8 (notation 2.6; decision of the owner, 2026-11-17).
#[test]
fn copy_counts_from_the_corner_and_a_paste_falls_on_the_grid() {
    let fixed = format!("{WIRED}\nlayout {{\n    node a at (40, 80)\n    node b at (200, 120)\n}}\n");
    let ctx = context();
    install_fonts(&ctx);
    let mut canvas = Canvas::new();
    canvas.set_drawing(drawing(&fixed, &ViewState::default()), false);
    canvas.set_viewport(1.0, Vec2::ZERO);
    let both = showing(&selecting(&["b", "a"], &[], &[]));
    frame_shown(&ctx, &mut canvas, 0.0, Vec::new(), &both);
    assert_eq!(canvas.selection_corner(&both.selection), Some((40, 80)));
    assert_eq!(canvas.selection_corner(&Selection::default()), None);
    let (_, at) = canvas.paste_at(&Shown::default(), None).unwrap();
    assert_eq!((at.0 % 8, at.1 % 8), (0, 0), "{at:?}");
    // A pointer off the canvas (on a menu) is no pointer.
    assert_eq!(canvas.paste_at(&Shown::default(), Some(Pos2::new(-5000.0, -5000.0))), Some((crate::InFrame::default(), at)));
}

/// An `error` out of a loop is a carry by default, so that a `loop` stops on it (spec 6.3), Alt a `last`; a `for
/// parallel` has no carry, so a `last` (decision of the owner, 2026-11-18).
#[test]
fn an_error_out_of_a_loop_is_a_carry_by_default() {
    for (head, default, other) in [("loop {", "carry", "last"), ("for x in xs {", "carry", "last"), ("for parallel x in xs {", "last", "collect")] {
        let text = format!(
            "labcraft 1\n\nuse time\n\nfn main(xs: [f64], err: error = none) -> (out: error) {{\n    _ = {head}\n        e = time.wait(1 s, err: err)\n{}    }}\n    out = err\n}}\n",
            if head.starts_with("loop") { "        until true\n" } else { "" }
        );
        let shown = self::drawing(&text, &ViewState::default());
        let from = shown.ports.iter().position(|port| !port.input && port.name == "e").unwrap();
        let verdicts = labcraft_diagram::wiring(&shown, from);
        let at = shown.ports.iter().position(|port| port.input && port.name == "output out").unwrap();
        let Some(Some(labcraft_diagram::Verdict::Takes { exits, .. })) = verdicts.get(at) else { panic!("{head}: {:?}", verdicts.get(at)) };
        assert_eq!(wiring::passages(&shown, &shown.ports[from], &shown.ports[at], exits, false), vec![default], "{head}");
        assert_eq!(wiring::passages(&shown, &shown.ports[from], &shown.ports[at], exits, true), vec![other], "{head}");
    }
    assert_eq!(wiring::passage_marks(&["carry"]), "↻ carry");
}

/// A `last` is a terminal of its own on the inner right edge, `•]` (notation 5.1): the part it stands for is `last y`, as
/// `structure.remove_part` names it, and the wire to it ends at the output it gives (commands.md 9.2).
#[test]
fn a_last_is_a_part_of_its_loop() {
    let text = "labcraft 1\n\nuse core\n\nfn main(xs: [f64]) -> (y: [f64], z: f64) {\n    all, top = for v in xs {\n        w = core.abs(v)\n        collect w\n        last w\n    }\n    y = all\n    z = top\n}\n";
    let drawing = drawing(text, &ViewState::default());
    let parts: Vec<(&str, &str)> =
        drawing.ports.iter().filter(|port| port.frame && !port.part.is_empty()).map(|port| (port.part.as_str(), port.name.as_str())).collect();
    assert!(parts.contains(&("collect all", "all")), "{parts:?}");
    assert!(parts.contains(&("last top", "top")), "{parts:?}");
}

/// A wire out of a loop takes the value out as the type of its input says (notation 2.5, decisions of the owner,
/// 2026-11-18): the value's type, a `last`; an array of it, a `collect`; neither, the default of the loop, a `collect`
/// for a `for` and a `last` for a `loop`; Alt takes the other one, in a `for parallel` too.
#[test]
fn a_wire_out_of_a_loop_chooses_its_passage_by_the_type_of_its_input() {
    let text = "labcraft 1\n\nuse core\n\nfn main(xs: [f64]) -> (y: f64, z: [f64], k: bool) {\n    _ = for v in xs {\n        w = core.abs(v)\n    }\n    y = 0.0\n    z = xs\n    k = true\n}\n";
    let drawing = drawing(text, &ViewState::default());
    let from = drawing.ports.iter().position(|port| !port.input && port.name == "w").unwrap();
    let verdicts = labcraft_diagram::wiring(&drawing, from);
    let passage = |to: &str, alt: bool| {
        let at = drawing.ports.iter().position(|port| port.input && port.name == to).unwrap();
        let Some(Some(labcraft_diagram::Verdict::Takes { exits, .. })) = verdicts.get(at) else { panic!("{to}: {:?}", verdicts.get(at)) };
        wiring::passages(&drawing, &drawing.ports[from], &drawing.ports[at], exits, alt)
    };
    assert_eq!(passage("output y", false), vec!["last"]);
    assert_eq!(passage("output z", false), vec!["collect"]);
    assert_eq!(passage("output k", false), vec!["collect"]);
    // The type then judged is that of the passage: an `[f64]` is no `bool`, an `f64` is an `f64`.
    let port = |name: &str| drawing.ports.iter().find(|port| port.input && port.name == name).unwrap();
    assert!(labcraft_diagram::passage_refusal(&drawing, &drawing.ports[from], port("output k"), 1).is_some());
    assert!(labcraft_diagram::passage_refusal(&drawing, &drawing.ports[from], port("output y"), 0).is_none());
    assert!(labcraft_diagram::passage_refusal(&drawing, &drawing.ports[from], port("output z"), 1).is_none());
    // Alt takes the other one; the gesture makes no carry (decision of the owner, 2026-11-18).
    assert_eq!(passage("output y", true), vec!["collect"]);
    assert_eq!(passage("output z", true), vec!["last"]);
    assert_eq!(wiring::passage_marks(&["last", "collect"]), "•] last value  ·  [•] collect");
    // The default of a `loop` is a `last`, and of a `for parallel` as of any `for`, a `collect`, Alt the other one.
    for (head, default, other) in [("loop {", "last", "collect"), ("for parallel v in xs {", "collect", "last")] {
        let text = format!(
            "labcraft 1\n\nuse core\n\nfn main(xs: [f64]) -> (k: bool) {{\n    _ = {head}\n        w = core.abs(1.0)\n{}    }}\n    k = true\n}}\n",
            if head.starts_with("loop") { "        until true\n" } else { "" }
        );
        let shown = self::drawing(&text, &ViewState::default());
        let from = shown.ports.iter().position(|port| !port.input && port.name == "w").unwrap();
        let verdicts = labcraft_diagram::wiring(&shown, from);
        let at = shown.ports.iter().position(|port| port.input && port.name == "output k").unwrap();
        let Some(Some(labcraft_diagram::Verdict::Takes { exits, .. })) = verdicts.get(at) else { panic!("{head}: {:?}", verdicts.get(at)) };
        assert_eq!(wiring::passages(&shown, &shown.ports[from], &shown.ports[at], exits, false), vec![default], "{head}");
        assert_eq!(wiring::passages(&shown, &shown.ports[from], &shown.ports[at], exits, true), vec![other], "{head}");
    }
}

/// The terminal of a part offers the other two kinds (notation 2.6, #215): no carry in a `for parallel`, and neither a
/// `collect` nor a `last` for a carry that the body reads, which the terminal inside knows by the wire that leaves it.
#[test]
fn a_part_offers_the_other_kinds_of_passage() {
    use crate::parts::passages_offered;
    assert_eq!(passages_offered("carry a", false, false), vec![("collect", true), ("last", true)]);
    assert_eq!(passages_offered("carry a", false, true), vec![("collect", false), ("last", false)]);
    assert_eq!(passages_offered("collect b", false, false), vec![("carry", true), ("last", true)]);
    assert_eq!(passages_offered("last b", true, false), vec![("collect", true)]);
    assert!(passages_offered("in v", false, false).is_empty());
    let read = |text: &str| {
        let drawing = drawing(text, &ViewState::default());
        let port = drawing.ports.iter().find(|port| port.frame && !port.input && port.part == "carry a").unwrap();
        port.read
    };
    let reads = "labcraft 1\n\nuse core\n\nfn main(xs: [f64]) -> (y: f64) {\n    a = for v in xs, carry a = 0.0 {\n        next a = core.add(a, v)\n    }\n    y = a\n}\n";
    let ignores = "labcraft 1\n\nuse core\n\nfn main(xs: [f64]) -> (y: f64) {\n    a = for v in xs, carry a = 0.0 {\n        next a = core.abs(v)\n    }\n    y = a\n}\n";
    assert!(read(reads));
    assert!(!read(ignores));
}

/// A carry of an `error` stops a `loop` on the first error (spec 6.3): the entry that removes or adds that stop says so
/// (decision of the owner, 2026-11-18), and the terminal of such a carry has the type `error`.
#[test]
fn a_part_that_changes_the_stop_on_error_says_so() {
    use crate::parts::passage_warning;
    assert!(passage_warning("carry", "last", true).is_some_and(|text| text.contains("no longer stops")));
    assert!(passage_warning("last", "carry", true).is_some_and(|text| text.contains("will stop")));
    assert_eq!(passage_warning("collect", "last", true), None);
    assert_eq!(passage_warning("carry", "collect", false), None);
    let text = "labcraft 1\n\nfn main(e: error) -> (y: error) {\n    err = loop carry err = none {\n        next err = e\n        until true\n    }\n    y = err\n}\n";
    let drawing = drawing(text, &ViewState::default());
    let port = drawing.ports.iter().find(|port| port.frame && !port.input && port.part == "carry err").unwrap();
    assert_eq!(port.ty.and_then(|ty| drawing.types.get(ty)).map(String::as_str), Some("error"));
}

/// Points of a structure, in units: on its header (just under its top edge, in its middle), on its left edge (just above
/// its bottom edge, where no terminal is), on its right edge (in its middle) and on its bottom-right corner.
fn edges_of(drawing: &Drawing, name: &str) -> [Pos2; 4] {
    let placed = drawing.placed.iter().find(|placed| placed.name == name).unwrap();
    let (x, y, width, height) = (placed.x as f32 / 100.0, placed.y as f32 / 100.0, placed.width as f32 / 100.0, placed.height as f32 / 100.0);
    [
        Pos2::new(x + width / 2.0, y + 3.0),
        Pos2::new(x + 2.0, y + height - 10.0),
        Pos2::new(x + width - 2.0, y + height / 2.0),
        Pos2::new(x + width - 2.0, y + height - 2.0),
    ]
}

/// A named `for` with a terminal on each edge, and an unnamed `loop` with none (issue #214).
const STRUCTURES: [&str; 2] = [
    "labcraft 1\n\nuse core\n\nfn main(xs: [f64]) -> (y: [f64]) {\n    ys = for v in xs {\n        w = core.abs(v)\n        collect w\n    }\n    y = ys\n}\n",
    "labcraft 1\n\nfn main() -> (y: f64) {\n    _ = loop {\n        until true\n    }\n    y = 0.0\n}\n",
];

#[test]
fn a_structure_moves_when_dragged_by_its_header_or_its_left_edge() {
    for text in STRUCTURES {
        let ctx = context();
        install_fonts(&ctx);
        let mut canvas = Canvas::new();
        let drawn = drawing(text, &ViewState::default());
        let name = drawn.placed.iter().find(|placed| !placed.holds.is_empty()).map(|placed| placed.name.clone()).unwrap();
        canvas.set_drawing(drawn.clone(), false);
        canvas.set_viewport(1.0, Vec2::ZERO);
        let none = Shown::default();
        frame_shown(&ctx, &mut canvas, 0.0, Vec::new(), &none);
        let [header, left, ..] = edges_of(&drawn, &name);
        for (rank, from) in [header, left].into_iter().enumerate() {
            let intents = drag_with(&ctx, &mut canvas, (from, from + Vec2::new(40.0, 24.0)), 1.0 + rank as f64, &none);
            assert!(intents.contains(&moving_of(&[name.as_str()], (40, 24))), "{name} from {from:?}: {intents:?}");
            canvas.cancel_move();
        }
    }
}

#[test]
fn the_right_edge_the_bottom_edge_or_the_corner_resizes_a_structure_on_the_grid() {
    for text in STRUCTURES {
        let ctx = context();
        install_fonts(&ctx);
        let mut canvas = Canvas::new();
        let drawn = drawing(text, &ViewState::default());
        let placed = drawn.placed.iter().find(|placed| !placed.holds.is_empty()).unwrap();
        let name = placed.name.clone();
        // Its size from the corner that its entry places, in units, as the drawing gives it; an edge moves by whole steps
        // of the grid of 8, as a node does (30 is 32, 41 is 40).
        let (width, height) = (placed.width / 100, (placed.height - placed.over) / 100);
        let on_grid = |from: i64, by: i64| from + (by as f32 / 8.0).round() as i64 * 8;
        canvas.set_drawing(drawn.clone(), false);
        canvas.set_viewport(1.0, Vec2::ZERO);
        let none = Shown::default();
        frame_shown(&ctx, &mut canvas, 0.0, Vec::new(), &none);
        let [_, _, right, corner] = edges_of(&drawn, &name);
        let resize = |size| Intent::Resize { structure: Some(name.clone()), size: Some(size) };
        let intents = drag_with(&ctx, &mut canvas, (right, right + Vec2::new(30.0, 50.0)), 1.0, &none);
        assert_eq!(intents, vec![resize((on_grid(width, 30), height))], "{name}");
        canvas.cancel_move();
        let intents = drag_with(&ctx, &mut canvas, (corner, corner + Vec2::new(30.0, 41.0)), 2.0, &none);
        assert_eq!(intents, vec![resize((on_grid(width, 30), on_grid(height, 41)))], "{name}");
        canvas.cancel_move();
        // Never under what it holds: pulled far in, it stops at its least size.
        let least = (placed.least.0 + 99) / 100;
        let intents = drag_with(&ctx, &mut canvas, (right, right - Vec2::new(500.0, 0.0)), 3.0, &none);
        let expected = if least == width { Vec::new() } else { vec![resize((least, height))] };
        assert_eq!(intents, expected, "{name}");
    }
}

#[test]
fn the_corner_of_the_frame_of_the_function_resizes_it() {
    let ctx = context();
    install_fonts(&ctx);
    let mut canvas = Canvas::new();
    let drawn = drawing(STRUCTURES[0], &ViewState::default());
    canvas.set_drawing(drawn.clone(), false);
    canvas.set_viewport(1.0, Vec2::ZERO);
    let none = Shown::default();
    frame_shown(&ctx, &mut canvas, 0.0, Vec::new(), &none);
    let body = drawn.bodies.first().unwrap();
    let (x, y, width, height) = body.frame;
    let corner = Pos2::new((x + width) as f32 / 100.0 + 2.0, (y + height) as f32 / 100.0 + 2.0);
    let size = ((x + width - body.origin.0) / 100, (y + height - body.origin.1) / 100);
    let on_grid = |from: i64, by: i64| from + (by as f32 / 8.0).round() as i64 * 8;
    let intents = drag_with(&ctx, &mut canvas, (corner, corner + Vec2::new(64.0, 40.0)), 1.0, &none);
    assert_eq!(intents, vec![Intent::Resize { structure: None, size: Some((on_grid(size.0, 64), on_grid(size.1, 40))) }]);
}

#[test]
fn a_terminal_on_the_edge_of_a_structure_draws_a_wire_and_never_resizes() {
    let ctx = context();
    install_fonts(&ctx);
    let mut canvas = Canvas::new();
    let drawn = drawing(STRUCTURES[0], &ViewState::default());
    canvas.set_drawing(drawn.clone(), false);
    canvas.set_viewport(1.0, Vec2::ZERO);
    let none = Shown::default();
    frame_shown(&ctx, &mut canvas, 0.0, Vec::new(), &none);
    // The output `ys` of the `for` straddles its right edge, in the band that resizes it: the terminal is what a press
    // takes (notation 2.6).
    let from = port_of(&drawn, "ys", false);
    let intents = drag_with(&ctx, &mut canvas, (from, from + Vec2::new(0.0, 120.0)), 1.0, &none);
    assert!(!intents.iter().any(|intent| matches!(intent, Intent::Resize { .. })), "{intents:?}");
}

#[test]
fn a_frame_of_fixed_size_shows_its_corner_under_the_setting() {
    let ctx = context();
    install_fonts(&ctx);
    let mut canvas = Canvas::new();
    // The `for` of [`STRUCTURES`] and the function, each with a size larger than their content.
    let text = format!("{}\nlayout {{\n    frame size (640, 400)\n    frame ys size (320, 200)\n}}\n", STRUCTURES[0]);
    let drawn = drawing(&text, &ViewState::default());
    let ys = drawn.placed.iter().position(|placed| placed.name == "ys").unwrap();
    assert_eq!(drawn.placed.get(ys).and_then(|placed| placed.size), Some((32_000, 20_000)));
    assert_eq!(drawn.bodies.first().and_then(|body| body.size), Some((64_000, 40_000)));
    canvas.set_drawing(drawn.clone(), false);
    canvas.set_viewport(1.0, Vec2::ZERO);
    // Three diagonals for each corner, as segments: those of the two frames with the setting only.
    let segments = |shapes: &[ClippedShape]| shapes.iter().filter(|shape| matches!(shape.shape, egui::Shape::LineSegment { .. })).count();
    let marked = Shown { mark_fixed: true, ..Shown::default() };
    let (with, _, _) = frame_shown(&ctx, &mut canvas, 0.0, Vec::new(), &marked);
    let (without, _, _) = frame_shown(&ctx, &mut canvas, 0.1, Vec::new(), &Shown::default());
    assert_eq!(segments(&with), segments(&without) + 6);
    assert_eq!(crate::sizing::tooltip(&drawn, Some(ys)), "Size fixed (`frame ys size (320, 200)` in `layout`). Right-click › Remove the fixed size");
    assert_eq!(crate::sizing::tooltip(&drawn, None), "Size fixed (`frame size (640, 400)` in `layout`). Right-click › Remove the fixed size");
}

// The signature on the canvas (notation 2.4; decisions of the owner, 2026-11-19). The intents expected are written by
// hand from the rule: a terminal of a node that holds nothing becomes a terminal of the function of its name and type.

const SCALED: &str = "labcraft 1\n\nfn main(low: f64[V] = 220 V) -> (y: f64[V]) {\n    y = low\n    r = scale(?)\n}\n";
const SCALE: &str = "labcraft 1\n\nfn scale(v: f64[V], k: f64 = 2) -> (r: f64) {\n    r = 1.0\n}\n";

fn scaled() -> Arc<Drawing> {
    drawing_of(&[("main.lcf", SCALED), ("scale.lcf", SCALE)], &ViewState::default())
}

fn port_named(drawing: &Drawing, name: &str, input: bool) -> usize {
    drawing.ports.iter().position(|port| port.name == name && port.input == input && !port.frame).unwrap_or_else(|| panic!("no port {name}"))
}

#[test]
fn a_terminal_of_a_node_that_holds_nothing_becomes_one_of_the_function() {
    let drawing = scaled();
    let hole = port_named(&drawing, "r.v", true);
    assert_eq!(
        signature::make(&drawing, hole),
        Some(Intent::MakeTerminal { terminal: "input v".to_owned(), ty: "f64[V]".to_owned(), from: "v".to_owned(), to: "r.v".to_owned() })
    );
    // The output `r`, read by nothing, keeps its own name.
    let output = port_named(&drawing, "r", false);
    assert_eq!(
        signature::make(&drawing, output),
        Some(Intent::MakeTerminal { terminal: "output r".to_owned(), ty: "f64".to_owned(), from: "r".to_owned(), to: "output r".to_owned() })
    );
    // `k` holds its default: it holds nothing, and its name is free.
    assert!(signature::make(&drawing, port_named(&drawing, "r.k", true)).is_some());
}

#[test]
fn a_name_that_the_drawing_shows_is_numbered() {
    let text = "labcraft 1\n\nfn main(v: f64[V]) -> (y: f64[V]) {\n    y = v\n    r = scale(?)\n}\n";
    let drawing = drawing_of(&[("main.lcf", text), ("scale.lcf", SCALE)], &ViewState::default());
    let hole = port_named(&drawing, "r.v", true);
    assert!(matches!(signature::make(&drawing, hole), Some(Intent::MakeTerminal { terminal, from, .. }) if terminal == "input v2" && from == "v2"));
}

#[test]
fn a_wired_terminal_is_not_made_one_of_the_function() {
    let drawing = scaled();
    // `y` takes the wire of `low`.
    let text = "labcraft 1\n\nfn main(low: f64[V]) -> (y: f64) {\n    y = scale(low)\n}\n";
    let wired = drawing_of(&[("main.lcf", text), ("scale.lcf", SCALE)], &ViewState::default());
    assert_eq!(signature::makeable(&wired, port_named(&wired, "y.v", true)), None);
    // A terminal of the frame is one already.
    let frame = drawing.ports.iter().position(|port| port.frame && port.placed.is_none()).unwrap();
    assert_eq!(signature::makeable(&drawing, frame), None);
}

#[test]
fn the_name_and_the_default_of_a_terminal_of_the_frame_open_their_fields() {
    let drawing = scaled();
    let text_at = |wanted: &str| {
        drawing
            .groups
            .iter()
            .filter(|group| group.terminals.is_some() && group.placed.is_none())
            .flat_map(|group| &group.shapes)
            .find_map(|shape| match shape {
                labcraft_diagram::Shape::Text { x, y, size, runs, .. } if runs.iter().map(|run| run.text.as_str()).collect::<String>() == wanted => {
                    Some(Pos2::new(*x as f32 / 100.0 + 2.0, *y as f32 / 100.0 - *size as f32 / 300.0))
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("no text {wanted}"))
    };
    let name = signature::text_under(&drawing, text_at("low")).map(|(kind, text, _)| (kind, text));
    assert_eq!(name, Some((FieldKind::TerminalName { terminal: "input low".to_owned() }, "low".to_owned())));
    let default = signature::text_under(&drawing, text_at("◂ 220 V")).map(|(kind, text, _)| (kind, text));
    assert_eq!(default, Some((FieldKind::TerminalDefault { terminal: "input low".to_owned() }, "220 V".to_owned())));
    assert_eq!(signature::frame_default(&drawing, "input low"), "220 V");
}

/// The title of the frame, `fn` and the name (notation 2): a double-click on it, or F2 with nothing selected, renames
/// the function (decision of the owner, 2026-11-19).
#[test]
fn the_title_of_the_frame_opens_the_name_of_the_function() {
    let drawing = drawing(WIRED, &ViewState::default());
    let (name, at) = signature::title(&drawing).unwrap();
    assert_eq!(name, "main");
    assert_eq!(signature::title_under(&drawing, at + Vec2::new(4.0, 2.0)).map(|(name, _)| name), Some("main".to_owned()));
    assert_eq!(signature::title_under(&drawing, at + Vec2::new(0.0, 200.0)), None);
    let mut canvas = Canvas::new();
    canvas.set_drawing(drawing, false);
    assert!(canvas.edit_name());
    assert_eq!(canvas.field.as_ref().map(|field| (field.kind.clone(), field.text.clone())), Some((FieldKind::FunctionName, "main".to_owned())));
}

#[test]
fn the_edges_of_the_frame_of_the_function_are_its_sides() {
    let drawing = scaled();
    let (x, y, width, height) = drawing.bodies[0].frame;
    let (left, right, middle) = (x as f32 / 100.0, (x + width) as f32 / 100.0, (y + height / 2) as f32 / 100.0);
    assert_eq!(signature::function_edge(&drawing, Pos2::new(left + 1.0, middle), 4.0), Some(true));
    assert_eq!(signature::function_edge(&drawing, Pos2::new(right - 1.0, middle), 4.0), Some(false));
    assert_eq!(signature::function_edge(&drawing, Pos2::new((left + right) / 2.0, middle), 4.0), None);
}

/// Ctrl and Up on a terminal of the frame of the function selected alone: to the next place on the grid of 8 that way,
/// the presses that follow one another one `terminal.move` (notation 2.4, 2.6). `x` is the first input: nothing above it.
#[test]
fn a_run_of_ctrl_and_up_moves_a_terminal_of_the_frame_along_its_edge() {
    let ctx = context();
    install_fonts(&ctx);
    let mut canvas = Canvas::new();
    let drawn = drawing(WIRED, &ViewState::default());
    canvas.set_drawing(drawn.clone(), false);
    canvas.set_viewport(1.0, Vec2::ZERO);
    let x = showing(&selecting(&[], &[], &["input x"]));
    frame_shown(&ctx, &mut canvas, 0.0, Vec::new(), &x);
    // The focus comes with a click on it, which selects it.
    let clicked = click_with(&ctx, &mut canvas, port_of(&drawn, "x", false), 0.1, egui::Modifiers::NONE, &x);
    assert!(clicked.1.contains(&selecting(&[], &[], &["input x"])), "{clicked:?}");
    let ctrl = egui::Modifiers::COMMAND;
    let (_, _, intents) = frame_shown(&ctx, &mut canvas, 0.5, vec![key(egui::Key::ArrowUp, ctrl)], &x);
    assert_eq!(intents, Vec::new(), "a run asks for nothing while it goes on");
    // Left and Right do not take a terminal off its edge.
    frame_shown(&ctx, &mut canvas, 0.6, vec![key(egui::Key::ArrowLeft, ctrl)], &x);
    frame_shown(&ctx, &mut canvas, 0.7, vec![key(egui::Key::ArrowUp, ctrl)], &x);
    let port = port_of(&drawn, "x", false);
    let origin = drawn.bodies[0].origin.1 as f32 / 100.0;
    let start = (port.y - origin).round() as i64;
    let once = ((start - 8) as f32 / 8.0).round() as i64 * 8;
    assert_eq!(canvas.end_run(), Some(Intent::MoveTerminal { terminal: "input x".to_owned(), y: once - 8 }));
    assert_eq!(canvas.end_run(), None);
}

/// `x` fixed by the file.
const TERMINAL_FIXED: &str = "labcraft 1\n\nuse core\n\nfn main(x: f64, flag: bool) -> (y: f64) {\n    a = core.add(x, 1.0)\n    b = core.mul(a, 2.0)\n    y = b\n}\n\nlayout {\n    input x at 200\n}\n";

#[test]
fn a_fixed_terminal_of_the_frame_shows_its_pin_under_the_setting() {
    let ctx = context();
    install_fonts(&ctx);
    let mut canvas = Canvas::new();
    let drawn = drawing(TERMINAL_FIXED, &ViewState::default());
    let fixed: Vec<&str> = drawn.ports.iter().filter(|port| port.fixed).map(|port| port.caption.as_str()).collect();
    assert_eq!(fixed, ["x"]);
    canvas.set_drawing(drawn.clone(), false);
    canvas.set_viewport(1.0, Vec2::ZERO);
    let lines = |shapes: &[ClippedShape]| shapes.iter().filter(|shape| matches!(shape.shape, egui::Shape::Path(_))).count();
    let (with, _, _) = frame_shown(&ctx, &mut canvas, 0.0, Vec::new(), &Shown { mark_fixed: true, ..Shown::default() });
    let (without, _, _) = frame_shown(&ctx, &mut canvas, 0.1, Vec::new(), &Shown::default());
    assert!(lines(&with) > lines(&without), "{} {}", lines(&with), lines(&without));
}

/// A new drawing during a drag of a terminal of the frame does not end it: the drop asks for its move, as for a node.
#[test]
fn a_terminal_dragged_goes_on_over_a_new_drawing() {
    let ctx = context();
    install_fonts(&ctx);
    let mut canvas = Canvas::new();
    let drawn = drawing(WIRED, &ViewState::default());
    canvas.set_drawing(drawn.clone(), false);
    canvas.set_viewport(1.0, Vec2::ZERO);
    let shown = Shown::default();
    let x = port_of(&drawn, "x", false);
    let to = x - Vec2::new(0.0, 48.0);
    let button =
        |at, pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    frame_shown(&ctx, &mut canvas, 0.0, Vec::new(), &shown);
    frame_shown(&ctx, &mut canvas, 0.1, vec![egui::Event::PointerMoved(x), button(x, true)], &shown);
    frame_shown(&ctx, &mut canvas, 0.2, vec![egui::Event::PointerMoved(x - Vec2::new(0.0, 20.0))], &shown);
    canvas.set_drawing(drawn.clone(), false);
    frame_shown(&ctx, &mut canvas, 0.3, vec![egui::Event::PointerMoved(to)], &shown);
    let (_, _, intents) = frame_shown(&ctx, &mut canvas, 0.4, vec![egui::Event::PointerMoved(to), button(to, false)], &shown);
    let origin = drawn.bodies[0].origin.1 as f32 / 100.0;
    let expected = ((x.y - origin - 48.0) / 8.0).round() as i64 * 8;
    assert_eq!(intents, vec![Intent::MoveTerminal { terminal: "input x".to_owned(), y: expected }]);
}

// Alt and a drag on a terminal of a block (notation 2.4, spec 11.2): the places are 8 apart, a block of 40 has 4 on each
// side (40 / 8 - 1), the first place of the top side is `after:`'s.

const BLOCK_CALLEE: &str = "labcraft 1\n\nfn g(x: f64, y: f64, z: f64) -> (n: i64) {\n    n = 1\n}\n";
const BLOCK_MAIN: &str = "labcraft 1\n\nfn main(a: f64) -> (r: i64) {\n    r = g(a, a, a)\n}\n";

/// The drawing, the block of the call to `g` and a drag of its input `x`.
fn block_drag() -> (Arc<Drawing>, usize, blockdrag::BlockDrag, Pos2) {
    let drawing = drawing_of(&[("main.lcf", BLOCK_MAIN), ("g.lcf", BLOCK_CALLEE)], &ViewState::default());
    let placed = drawing.placed.iter().position(|node| !node.callee.is_empty()).expect("the call is a block of the project");
    let port = drawing.ports.iter().position(|port| port.placed == Some(placed) && port.caption == "x" && port.input).expect("x");
    let origin = Pos2::new(drawing.placed[placed].x as f32 / 100.0, drawing.placed[placed].y as f32 / 100.0);
    let drag = blockdrag::BlockDrag::new(&drawing, port, origin).expect("a block of the project");
    (drawing, placed, drag, origin)
}

#[test]
fn a_terminal_goes_to_the_nearest_side_and_place_of_the_pointer() {
    let (drawing, _, mut drag, origin) = block_drag();
    // `x` starts on the left, place 1 (the first free place from the top).
    assert_eq!(drag.from, (blockdrag::Side::Left, 1));
    assert!(!drag.changed());
    let mut to = |dx: f32, dy: f32| {
        drag.set_at(&drawing, origin + Vec2::new(dx, dy));
        drag.to
    };
    // The block is 40 by 40. 1 from the left edge, 17 down: 17 / 8 = 2.1, place 2.
    assert_eq!(to(1.0, 17.0), (blockdrag::Side::Left, 2));
    // 1 from the right edge (x = 39), 30 down: 30 / 8 = 3.75, place 4.
    assert_eq!(to(39.0, 30.0), (blockdrag::Side::Right, 4));
    // 1 below the top edge, 25 along: 25 / 8 = 3.1, place 3.
    assert_eq!(to(25.0, 1.0), (blockdrag::Side::Top, 3));
    // Never the first place of the top side, which is `after:`'s: 2 along is place 0, which the top side takes as place 2.
    assert_eq!(to(2.0, 0.0), (blockdrag::Side::Top, 2));
    assert_eq!(to(9.0, 0.0), (blockdrag::Side::Top, 2));
    // Past the last place of a side: the last. 4 places: 32.
    assert_eq!(to(1.0, 100.0), (blockdrag::Side::Left, 4));
    assert_eq!(to(39.0, 100.0), (blockdrag::Side::Right, 4));
    assert_eq!(to(100.0, 1.0), (blockdrag::Side::Top, 4));
    assert!(drag.changed());
}

#[test]
fn a_side_without_a_free_place_is_never_offered() {
    // `top 1`: the only place of the top side is `after:`'s (spec 11.2), so a pointer on the top edge goes to the nearest
    // side that has one. The block is 16 wide and, its left side keeping the 4 places of a small block, 40 high.
    let callee = format!("{BLOCK_CALLEE}\nterminals {{\n    top 1\n}}\n");
    let drawing = drawing_of(&[("main.lcf", BLOCK_MAIN), ("g.lcf", &callee)], &ViewState::default());
    let placed = drawing.placed.iter().position(|node| !node.callee.is_empty()).expect("the call");
    let port = drawing.ports.iter().position(|port| port.placed == Some(placed) && port.caption == "x" && port.input).expect("x");
    let origin = Pos2::new(drawing.placed[placed].x as f32 / 100.0, drawing.placed[placed].y as f32 / 100.0);
    let mut drag = blockdrag::BlockDrag::new(&drawing, port, origin).expect("a block of the project");
    // On the top edge, 12 along: nearer the right edge (4 away) than the left (12): right, place 0 taken as 1.
    drag.set_at(&drawing, origin + Vec2::new(12.0, 0.0));
    assert_eq!(drag.to, (blockdrag::Side::Right, 1));
    drag.set_at(&drawing, origin + Vec2::new(3.0, 0.0));
    assert_eq!(drag.to, (blockdrag::Side::Left, 1));
}

#[test]
fn the_free_places_are_all_but_its_own_and_the_first_of_the_top() {
    let (drawing, _, drag, _) = block_drag();
    // A place held by default is free (commands.md 9.4): only the place of `x` itself, left 1, is left out, and top 1, which
    // is `after:`'s. The top side lists places 2 to 4, then each place of the left and the right in turn.
    use blockdrag::Side::{Left, Right, Top};
    assert_eq!(drag.free(&drawing), [(Top, 2), (Top, 3), (Top, 4), (Right, 1), (Left, 2), (Right, 2), (Left, 3), (Right, 3), (Left, 4), (Right, 4)]);
}

// The block of the view `Block` (ide.md 3): a drag on a terminal moves it to a place, a drag of an edge changes the size,
// in places of 8 (spec 11.2). The block is shown in the rectangle `VIEW`, at the largest whole zoom that holds it.

const VIEW: Rect = Rect { min: Pos2::new(0.0, 0.0), max: Pos2::new(600.0, 400.0) };

/// The block of `g`, as the view shows it, in a drawing that holds it.
fn own_block(sections: &str) -> Arc<Drawing> {
    let callee = format!("{BLOCK_CALLEE}\nterminals {{\n{sections}}}\n");
    let mut names = Interner::new();
    let mut root = LibrarySource::new(String::new(), String::new(), Origin::Project);
    root.files.push(SourceFile { shown: "g.lcf".to_owned(), content: Content::Bytes(Cow::Owned(callee.into_bytes())) });
    let checked = check(root, &Nothing, &mut names, &Limits::default());
    let own = labcraft_diagram::own_block(&checked, 0, &names, &DiagramLimits::default()).expect("the block");
    Arc::new(Drawing { block: Some(Box::new(own)), ..Drawing::default() })
}

/// Where a point of the block, in units, is on the screen.
fn on_view(drawing: &Drawing, unit: Pos2) -> Pos2 {
    let own = own_of(drawing);
    let zoom = enlarged_zoom(&own.drawing, VIEW.size());
    let size = Vec2::new(own.drawing.width as f32, own.drawing.height as f32) / 100.0 * zoom;
    VIEW.center() - size / 2.0 + unit.to_vec2() * zoom
}

/// The box of the block, in units.
fn block_box(drawing: &Drawing) -> Rect {
    let placed = &own_of(drawing).drawing.placed[0];
    Rect::from_min_size(Pos2::new(placed.x as f32, placed.y as f32) / 100.0, Vec2::new(placed.width as f32, placed.height as f32) / 100.0)
}

fn own_of(drawing: &Drawing) -> &labcraft_diagram::OwnBlock {
    drawing.block.as_deref().expect("the block")
}

/// A drag from `from` to `to`, on the screen, over a few frames: what the view asks for.
fn drag_block(own: &Arc<Drawing>, editable: bool, from: Pos2, to: Pos2) -> Vec<BlockIntent> {
    let ctx = context();
    install_fonts(&ctx);
    let mut view = BlockView::new();
    let button =
        |at, pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    let mut asked = Vec::new();
    let steps = [
        vec![],
        vec![egui::Event::PointerMoved(from), button(from, true)],
        vec![egui::Event::PointerMoved(from + (to - from) / 2.0)],
        vec![egui::Event::PointerMoved(to)],
        vec![egui::Event::PointerMoved(to), button(to, false)],
    ];
    for (step, events) in steps.into_iter().enumerate() {
        let mut raw = input(step as f64 * 0.1);
        raw.events = events;
        let output = ctx.run_ui(raw, |ui| {
            ui.scope_builder(egui::UiBuilder::new().max_rect(VIEW), |ui| asked.extend(view.show(ui, own, Theme::Light, editable)));
        });
        output.drop_without_applying_deltas();
    }
    asked
}

#[test]
fn a_drag_of_the_right_edge_of_the_block_writes_its_top_side() {
    let own = own_block("    top 5\n");
    let block = block_box(&own);
    assert_eq!(block.width(), 48.0);
    // From the middle of the right edge, 16 units right: 64 from the left edge, 64 / 8 - 1 = 7 places.
    let from = on_view(&own, Pos2::new(block.max.x, block.center().y));
    let to = on_view(&own, Pos2::new(block.max.x + 16.0, block.center().y));
    assert_eq!(drag_block(&own, true, from, to), [BlockIntent::Size { top: Some(7), left: None }]);
}

#[test]
fn a_drag_of_the_bottom_edge_stops_at_the_least_size_that_holds_the_lines() {
    // `left 4 z` needs 4 places on the left: dragged up to 8 below the top, the side keeps 4.
    let own = own_block("    left 6\n    left 4 z\n");
    let block = block_box(&own);
    assert_eq!((own_of(&own).left, own_of(&own).least.1), (6, 4));
    let from = on_view(&own, Pos2::new(block.center().x, block.max.y));
    let to = on_view(&own, Pos2::new(block.center().x, block.min.y + 8.0));
    assert_eq!(drag_block(&own, true, from, to), [BlockIntent::Size { top: None, left: Some(4) }]);
}

#[test]
fn a_terminal_dragged_on_the_block_goes_to_a_place_of_a_side() {
    let own = own_block("    top 5\n");
    let block = block_box(&own);
    let x = own_of(&own).drawing.ports.iter().find(|port| port.caption == "x" && port.input).expect("x");
    let from = on_view(&own, Pos2::new(x.x as f32, x.y as f32) / 100.0);
    // On the right edge, 16 down: place 2 of the right side.
    let to = on_view(&own, Pos2::new(block.max.x, block.min.y + 16.0));
    assert_eq!(drag_block(&own, true, from, to), [BlockIntent::Terminal { terminal: "input x".to_owned(), side: "right", place: 2 }]);
}

#[test]
fn a_block_that_the_commands_cannot_change_says_why_and_asks_for_nothing() {
    let mut drawing = (*own_block("    top 5\n")).clone();
    if let Some(own) = drawing.block.as_mut() {
        own.fixed = "kept".to_owned();
    }
    let own = Arc::new(drawing);
    let block = block_box(&own);
    let from = on_view(&own, Pos2::new(block.max.x, block.center().y));
    let to = on_view(&own, Pos2::new(block.max.x + 16.0, block.center().y));
    assert_eq!(drag_block(&own, false, from, to), [BlockIntent::Say("kept".to_owned())]);
}
