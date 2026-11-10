//! From the shapes of a drawing to what egui paints: a mesh for the shapes of each group, tessellated once per drawing
//! and per step of zoom and then only moved and scaled, the galleys of its texts, its icons.

use std::collections::HashMap;
use std::sync::Arc;

use egui::epaint::text::{LayoutJob, TextFormat, TextWrapping};
use egui::epaint::{
    CircleShape, CornerRadius, Mesh, PathShape, PathStroke, RectShape, Shape as Painted2, Stroke, StrokeKind, TessellationOptions, Tessellator,
    TextShape,
};
use egui::{Color32, FontId, Galley, Pos2, Rect, Vec2};
use labcraft_diagram::{
    CHEVRON_STEP, Detail, Drawing, IconDrawing, IconDrawn, IconShape, Paint, PathSegment, Pattern, Run, Shape, WIRE_RADIUS, WireStyle, chevrons,
};

use crate::Theme;
use crate::fonts::family;

/// What a cache is good for: one drawing, one theme, one step of zoom, one density of pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub generation: u64,
    pub theme: Theme,
    pub step: u32,
    pub pixels: u32,
}

/// The state of the fonts that galleys depend on: a galley kept across a reset of the atlas would show other glyphs.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Stamp {
    size: [usize; 2],
    faces: usize,
    fill: f32,
}

impl Stamp {
    fn of(ctx: &egui::Context) -> Stamp {
        ctx.fonts(|fonts| Stamp { size: fonts.font_image_size(), faces: fonts.definitions().font_data.len(), fill: fonts.font_atlas_fill_ratio() })
    }

    /// Whether the galleys made under `self` still hold under `now`: the atlas only grows between two resets.
    fn holds(self, now: Stamp) -> bool {
        self.size == now.size && self.faces == now.faces && now.fill >= self.fill
    }
}

/// The caches of the canvas.
#[derive(Default)]
pub struct Painter {
    key: Option<Key>,
    stamp: Option<Stamp>,
    loose: Option<Arc<Painted>>,
    groups: HashMap<usize, Arc<Painted>>,
    tabs: HashMap<(usize, bool), Arc<Painted>>,
    marks: HashMap<usize, Arc<Painted>>,
}

impl Painter {
    /// At each frame: the caches go when what they were made for changed.
    pub fn begin(&mut self, ctx: &egui::Context, key: Key) {
        let stamp = Stamp::of(ctx);
        let holds = self.stamp.is_some_and(|before| before.holds(stamp));
        if self.key != Some(key) || !holds {
            self.loose = None;
            self.groups.clear();
            self.tabs.clear();
            self.marks.clear();
            self.key = Some(key);
        }
        self.stamp = Some(stamp);
    }

    pub fn loose(&mut self, ctx: &egui::Context, drawing: &Drawing, theme: Theme, step: f32) -> Option<Arc<Painted>> {
        if self.loose.is_none() {
            self.loose = Some(Arc::new(Painted::of(ctx, drawing, &drawing.loose, theme, step)));
        }
        self.loose.clone()
    }

    pub fn group(&mut self, ctx: &egui::Context, drawing: &Drawing, index: usize, theme: Theme, step: f32) -> Option<Arc<Painted>> {
        let group = drawing.groups.get(index)?;
        Some(self.groups.entry(index).or_insert_with(|| Arc::new(Painted::of(ctx, drawing, &group.shapes, theme, step))).clone())
    }

    pub fn tab(&mut self, ctx: &egui::Context, drawing: &Drawing, index: usize, shown: bool, theme: Theme, step: f32) -> Option<Arc<Painted>> {
        let tab = drawing.tabs.get(index)?;
        let shapes = if shown { &tab.shown } else { &tab.other };
        Some(self.tabs.entry((index, shown)).or_insert_with(|| Arc::new(Painted::of(ctx, drawing, shapes, theme, step))).clone())
    }

    pub fn mark(&mut self, ctx: &egui::Context, drawing: &Drawing, index: usize, theme: Theme, step: f32) -> Option<Arc<Painted>> {
        let mark = drawing.marks.get(index)?;
        Some(self.marks.entry(index).or_insert_with(|| Arc::new(Painted::of(ctx, drawing, &mark.shapes, theme, step))).clone())
    }
}

/// A text laid out at the step: the top-left of its galley, from the corner of the drawing, in points of the step.
struct Text {
    at: Pos2,
    galley: Arc<Galley>,
    detail: Detail,
}

/// An icon: its square, in units, and what it draws, tessellated at the step from the corner of its square.
struct Icon {
    area: Rect,
    mesh: Arc<Mesh>,
    texts: Vec<Text>,
}

/// The shapes of a group, ready to paint: a mesh in points of the step from the corner of the drawing, its texts, its
/// icons.
pub struct Painted {
    mesh: Arc<Mesh>,
    texts: Vec<Text>,
    icons: Vec<Icon>,
}

impl Painted {
    fn of(ctx: &egui::Context, drawing: &Drawing, shapes: &[Shape], theme: Theme, step: f32) -> Painted {
        let size = ctx.fonts(|fonts| fonts.font_image_size());
        let discs = Vec::new();
        let options = ctx.tessellation_options(TessellationOptions::clone);
        let mut tessellator = Tessellator::new(ctx.pixels_per_point(), options, size, discs);
        let mut mesh = Mesh::default();
        let mut texts = Vec::new();
        let mut icons = Vec::new();
        for shape in shapes {
            match shape {
                Shape::Text { x, y, size, width, runs, detail } => {
                    if let Some(text) = text(ctx, (*x, *y), *size, *width, runs, *detail, theme, step) {
                        texts.push(text);
                    }
                }
                Shape::Icon { x, y, side, icon } => {
                    if let Some(found) = drawing.icons.get(*icon) {
                        let area = Rect::from_min_size(Pos2::new(*x as f32 / 100.0, *y as f32 / 100.0), Vec2::splat(*side as f32 / 100.0));
                        icons.push(icon_of(ctx, &mut tessellator, area, found, theme, step));
                    }
                }
                other => {
                    for painted in geometry(other, theme, step) {
                        tessellator.tessellate_shape(painted, &mut mesh);
                    }
                }
            }
        }
        Painted { mesh: Arc::new(mesh), texts, icons }
    }

    /// Paints at `origin`, the corner of the drawing on the screen, scaled by `scale` from the step.
    pub fn paint(&self, painter: &egui::Painter, origin: Pos2, scale: f32, zoom: f32, texts: bool) {
        add_mesh(painter, &self.mesh, origin, scale);
        if texts {
            for text in self.texts.iter().filter(|text| text.detail.shown_at(zoom)) {
                painter.add(TextShape::new(origin + text.at.to_vec2() * scale, text.galley.clone(), Color32::PLACEHOLDER));
            }
        }
        // Each icon cut to its square (spec 11.1).
        for icon in &self.icons {
            let corner = origin + icon.area.min.to_vec2() * zoom;
            let clipped = painter.with_clip_rect(Rect::from_min_size(corner, icon.area.size() * zoom).intersect(painter.clip_rect()));
            add_mesh(&clipped, &icon.mesh, corner, scale);
            if texts {
                for text in &icon.texts {
                    clipped.add(TextShape::new(corner + text.at.to_vec2() * scale, text.galley.clone(), Color32::PLACEHOLDER));
                }
            }
        }
    }
}

/// A mesh of the step, moved to `origin` and scaled.
fn add_mesh(painter: &egui::Painter, mesh: &Mesh, origin: Pos2, scale: f32) {
    if mesh.is_empty() {
        return;
    }
    let mut mesh = mesh.clone();
    for vertex in &mut mesh.vertices {
        vertex.pos = origin + vertex.pos.to_vec2() * scale;
    }
    painter.add(Painted2::mesh(mesh));
}

/// A node as a plain rectangle (notation 7).
pub fn coarse_rect(rect: Rect, paint: Paint, theme: Theme, zoom: f32) -> Painted2 {
    let fill = paint.fill.map_or(Color32::TRANSPARENT, |token| theme.color(token));
    let stroke = paint.stroke.map_or(Stroke::NONE, |token| Stroke::new(paint.width as f32 / 100.0 * zoom, theme.color(token)));
    Painted2::Rect(RectShape::new(rect, CornerRadius::ZERO, fill, stroke, StrokeKind::Middle))
}

fn point(x: i64, y: i64, step: f32) -> Pos2 {
    Pos2::new(x as f32 / 100.0 * step, y as f32 / 100.0 * step)
}

fn length(value: i64, step: f32) -> f32 {
    value as f32 / 100.0 * step
}

fn fill_and_stroke(paint: &Paint, theme: Theme, step: f32) -> (Color32, Stroke) {
    let fill = paint.fill.map_or(Color32::TRANSPARENT, |token| theme.color(token));
    let stroke = paint.stroke.map_or(Stroke::NONE, |token| Stroke::new(length(paint.width, step), theme.color(token)));
    (fill, stroke)
}

/// The shapes of epaint for a shape that is not a text or an icon, as the SVG draws it (`diagram::svg`).
fn geometry(shape: &Shape, theme: Theme, step: f32) -> Vec<Painted2> {
    match shape {
        Shape::Rect { x, y, width, height, radius, paint } => {
            let (fill, stroke) = fill_and_stroke(paint, theme, step);
            let rect = Rect::from_min_size(point(*x, *y, step), Vec2::new(length(*width, step), length(*height, step)));
            vec![Painted2::Rect(RectShape::new(rect, CornerRadius::from(length(*radius, step)), fill, stroke, StrokeKind::Middle))]
        }
        Shape::Circle { x, y, radius, paint } => {
            let (fill, stroke) = fill_and_stroke(paint, theme, step);
            vec![Painted2::Circle(CircleShape { center: point(*x, *y, step), radius: length(*radius, step), fill, stroke })]
        }
        Shape::Poly { points, closed, paint } => {
            let (fill, stroke) = fill_and_stroke(paint, theme, step);
            let points = points.iter().map(|&(x, y)| point(x, y, step)).collect();
            vec![Painted2::Path(PathShape { points, closed: *closed, fill, stroke: PathStroke::from(stroke) })]
        }
        Shape::Wire { points, style } => wire(points, style, theme, step),
        Shape::Text { .. } | Shape::Icon { .. } => Vec::new(),
    }
}

/// The points of an orthogonal polyline with its corners rounded as the SVG rounds them: a quadratic curve of radius
/// 6 at most, through the corner.
fn rounded(points: &[(i64, i64)], step: f32) -> Vec<Pos2> {
    let mut out = Vec::with_capacity(points.len() * 4);
    for (index, &(x, y)) in points.iter().enumerate() {
        let (Some(&(px, py)), Some(&(nx, ny))) = (index.checked_sub(1).and_then(|before| points.get(before)), points.get(index + 1)) else {
            out.push(point(x, y, step));
            continue;
        };
        let before = (x - px).abs() + (y - py).abs();
        let after = (nx - x).abs() + (ny - y).abs();
        let radius = WIRE_RADIUS.min(before / 2).min(after / 2);
        let a = point(x - (x - px).signum() * radius, y - (y - py).signum() * radius, step);
        let c = point(x, y, step);
        let b = point(x + (nx - x).signum() * radius, y + (ny - y).signum() * radius, step);
        const PARTS: usize = 6;
        for part in 0..=PARTS {
            let t = part as f32 / PARTS as f32;
            let u = 1.0 - t;
            out.push(Pos2::new(u * u * a.x + 2.0 * u * t * c.x + t * t * b.x, u * u * a.y + 2.0 * u * t * c.y + t * t * b.y));
        }
    }
    out
}

/// A wire, as its family draws it (notation 4.1): a stroke, the core of a sheath, the ticks of an array, the chevrons of
/// an error; dotted or dashed as the SVG.
fn wire(points: &[(i64, i64)], style: &WireStyle, theme: Theme, step: f32) -> Vec<Painted2> {
    let path = rounded(points, step);
    let color = theme.color(style.token);
    let stroke = Stroke::new(length(style.width, step), color);
    let unit = step;
    let mut out = Vec::new();
    match style.pattern {
        Pattern::Dotted => Painted2::dashed_line_many(&path, stroke, 1.5 * unit, 3.0 * unit, &mut out),
        Pattern::DashDot => {
            Painted2::dashed_line_many_with_offset(&path, stroke, &[6.0 * unit, 1.5 * unit], &[3.0 * unit, 3.0 * unit], 0.0, &mut out)
        }
        Pattern::Solid | Pattern::Chevrons => out.push(Painted2::line(path.clone(), stroke)),
    }
    if let Some(core) = style.core {
        out.push(Painted2::line(path.clone(), Stroke::new(length(style.width - 240, step), theme.color(core))));
    }
    if style.ticks > 0 {
        // A tick is a dash of a wide stroke: 6 across, 1 along, one per dimension, every 8.
        let (dashes, gaps): (&[f32], &[f32]) = match style.ticks {
            1 => (&[1.0], &[7.0]),
            2 => (&[1.0, 1.0], &[2.0, 4.0]),
            _ => (&[1.0, 1.0, 1.0], &[2.0, 2.0, 1.0]),
        };
        let dashes: Vec<f32> = dashes.iter().map(|dash| dash * unit).collect();
        let gaps: Vec<f32> = gaps.iter().map(|gap| gap * unit).collect();
        Painted2::dashed_line_many_with_offset(&path, Stroke::new(6.0 * unit, color), &dashes, &gaps, 0.0, &mut out);
    }
    if style.pattern == Pattern::Chevrons && CHEVRON_STEP > 0 {
        let thin = Stroke::new(1.5 * unit, color);
        for [a, b, c] in chevrons(points) {
            out.push(Painted2::line(vec![point(a.0, a.1, step), point(b.0, b.1, step), point(c.0, c.1, step)], thin));
        }
    }
    out
}

/// A text in the family of the notation, its runs in their tokens, left to right in logical order: epaint applies
/// UAX #9 to it (architecture 11.4). A text that a fallback font makes wider than its cells is drawn smaller, whole.
#[expect(clippy::too_many_arguments, reason = "the fields of a text shape")]
fn text(ctx: &egui::Context, (x, y): (i64, i64), size: u32, width: i64, runs: &[Run], detail: Detail, theme: Theme, step: f32) -> Option<Text> {
    if runs.iter().all(|run| run.text.is_empty()) {
        return None;
    }
    let points = size as f32 / 100.0 * step;
    let job = |points: f32| {
        let mut job = LayoutJob { wrap: TextWrapping::no_max_width(), break_on_newline: false, ..LayoutJob::default() };
        job.direction = egui::epaint::text::TextDirection::LeftToRight;
        for run in runs {
            job.append(&run.text, 0.0, TextFormat { font_id: FontId::new(points, family()), color: theme.color(run.token), ..TextFormat::default() });
        }
        job
    };
    let mut galley = ctx.fonts_mut(|fonts| fonts.layout_job(job(points)));
    let cells = length(width, step);
    let drawn = galley.size().x;
    if cells > 0.0 && drawn > cells * 1.02 {
        let smaller = points * cells / drawn;
        galley = ctx.fonts_mut(|fonts| fonts.layout_job(job(smaller)));
    }
    let baseline = galley.rows.first().and_then(|row| Some(row.pos.y + row.row.glyphs.first()?.pos.y)).unwrap_or(points * 0.93);
    Some(Text { at: point(x, y, step) - Vec2::new(0.0, baseline), galley, detail })
}

/// The size of a text of an icon, at most twice its grid: a larger text shows nothing more in its square, and a text
/// of size 100,000 would take the font rasteriser seconds, or make it panic.
const MAX_ICON_TEXT: f64 = 2.0;
/// A filled path of more points is filled as epaint fills, convex, rather than cut into triangles one ear at a time.
const MAX_EAR_POINTS: usize = 512;

/// An icon tessellated at the step, from the corner of its square: its items in the numbers of its grid, a stroke of 1.5.
fn icon_of(ctx: &egui::Context, tessellator: &mut Tessellator, area: Rect, icon: &IconDrawing, theme: Theme, step: f32) -> Icon {
    let mut mesh = Mesh::default();
    let mut texts = Vec::new();
    let grid: f64 = icon.grid.parse().unwrap_or(32.0);
    let side = area.width() * step;
    if grid.is_nan() || grid <= 0.0 || side.is_nan() || side <= 0.0 {
        return Icon { area, mesh: Arc::new(mesh), texts };
    }
    let k = side / grid as f32;
    let at = |(x, y): (f64, f64)| Pos2::new(x as f32 * k, y as f32 * k);
    for item in &icon.items {
        let color = theme.color(item.token);
        let stroke = Stroke::new(1.5 * k, color);
        let fill = if item.fill { color } else { Color32::TRANSPARENT };
        let shapes: Vec<Painted2> = match &item.drawn {
            IconDrawn::Rect { x, y, width, height, radius } => {
                let rect = Rect::from_min_size(at((*x, *y)), Vec2::new(*width as f32, *height as f32) * k);
                vec![Painted2::Rect(RectShape::new(rect, CornerRadius::from(*radius as f32 * k), fill, stroke, StrokeKind::Middle))]
            }
            IconDrawn::Circle { x, y, radius } => {
                vec![Painted2::Circle(CircleShape { center: at((*x, *y)), radius: *radius as f32 * k, fill, stroke })]
            }
            IconDrawn::Line { from, to } => vec![Painted2::line_segment([at(*from), at(*to)], stroke)],
            IconDrawn::Path(segments) => {
                let mut shapes = Vec::new();
                for (points, closed) in subpaths(segments) {
                    let points: Vec<Pos2> = points.into_iter().map(at).collect();
                    // As the SVG fills: every subpath closed for its fill, convex or not; the stroke as written.
                    if item.fill {
                        fill_polygon(&points, color, tessellator, &mut mesh);
                    }
                    shapes.push(Painted2::Path(PathShape { points, closed, fill: Color32::TRANSPARENT, stroke: PathStroke::from(stroke) }));
                }
                shapes
            }
            IconDrawn::Text { x, y, size } => {
                let IconShape::Text { runs, .. } = &item.shape else { continue };
                let points = (size.clamp(0.0, MAX_ICON_TEXT * grid) as f32) * k;
                if points.is_nan() || points <= 0.0 {
                    continue;
                }
                let mut job = LayoutJob { wrap: TextWrapping::no_max_width(), break_on_newline: false, ..LayoutJob::default() };
                job.direction = egui::epaint::text::TextDirection::LeftToRight;
                for run in runs {
                    job.append(
                        &run.text,
                        0.0,
                        TextFormat { font_id: FontId::new(points, family()), color: theme.color(run.token), ..TextFormat::default() },
                    );
                }
                let galley = ctx.fonts_mut(|fonts| fonts.layout_job(job));
                let baseline = galley.rows.first().and_then(|row| Some(row.pos.y + row.row.glyphs.first()?.pos.y)).unwrap_or(0.0);
                let corner = at((*x, *y)) - Vec2::new(galley.size().x / 2.0, baseline);
                texts.push(Text { at: corner, galley, detail: Detail::Label });
                Vec::new()
            }
        };
        for shape in shapes {
            tessellator.tessellate_shape(shape, &mut mesh);
        }
    }
    Icon { area, mesh: Arc::new(mesh), texts }
}

/// The fill of a polygon, as the SVG fills it (non-zero for a simple polygon): cut into triangles one ear at a time, or,
/// above [`MAX_EAR_POINTS`], filled as epaint fills a convex polygon.
pub(crate) fn fill_polygon(points: &[Pos2], color: Color32, tessellator: &mut Tessellator, mesh: &mut Mesh) {
    if points.len() < 3 {
        return;
    }
    if points.len() > MAX_EAR_POINTS {
        let shape = PathShape { points: points.to_vec(), closed: true, fill: color, stroke: PathStroke::NONE };
        tessellator.tessellate_shape(Painted2::Path(shape), mesh);
        return;
    }
    let base = u32::try_from(mesh.vertices.len()).unwrap_or(u32::MAX);
    for point in points {
        mesh.colored_vertex(*point, color);
    }
    let area: f32 = points.iter().zip(points.iter().cycle().skip(1)).map(|(a, b)| a.x * b.y - b.x * a.y).sum();
    let mut left: Vec<usize> = (0..points.len()).collect();
    if area < 0.0 {
        left.reverse();
    }
    let at = |index: usize| points.get(index).copied().unwrap_or(Pos2::ZERO);
    let cross = |a: Pos2, b: Pos2, c: Pos2| (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
    let mut guard = 0;
    while left.len() > 3 && guard < points.len() * points.len() {
        guard += 1;
        let count = left.len();
        let mut cut = None;
        for corner in 0..count {
            let (Some(&a), Some(&b), Some(&c)) = (left.get((corner + count - 1) % count), left.get(corner), left.get((corner + 1) % count)) else {
                continue;
            };
            let (pa, pb, pc) = (at(a), at(b), at(c));
            if cross(pa, pb, pc) <= 0.0 {
                continue;
            }
            let inside = left.iter().any(|&other| {
                if other == a || other == b || other == c {
                    return false;
                }
                let p = at(other);
                cross(pa, pb, p) > 0.0 && cross(pb, pc, p) > 0.0 && cross(pc, pa, p) > 0.0
            });
            if !inside {
                cut = Some((corner, [a, b, c]));
                break;
            }
        }
        // A polygon that crosses itself has no ear left: the rest is cut as a fan.
        let Some((corner, [a, b, c])) = cut else { break };
        add_triangle(mesh, base, [a, b, c]);
        left.remove(corner);
    }
    if let Some((&first, rest)) = left.split_first() {
        for pair in rest.windows(2) {
            if let (Some(&b), Some(&c)) = (pair.first(), pair.get(1)) {
                add_triangle(mesh, base, [first, b, c]);
            }
        }
    }
}

fn add_triangle(mesh: &mut Mesh, base: u32, corners: [usize; 3]) {
    let [a, b, c] = corners.map(|corner| base.saturating_add(u32::try_from(corner).unwrap_or(u32::MAX)));
    mesh.add_triangle(a, b, c);
}

/// The subpaths of a path, each its points, its curves flattened, and whether it closes.
pub(crate) fn subpaths(segments: &[PathSegment]) -> Vec<(Vec<(f64, f64)>, bool)> {
    const PARTS: usize = 12;
    let mut out: Vec<(Vec<(f64, f64)>, bool)> = Vec::new();
    let mut current: Vec<(f64, f64)> = Vec::new();
    let mut start = (0.0, 0.0);
    let mut at = (0.0, 0.0);
    let flush = |current: &mut Vec<(f64, f64)>, out: &mut Vec<(Vec<(f64, f64)>, bool)>, closed: bool| {
        if current.len() > 1 {
            out.push((std::mem::take(current), closed));
        } else {
            current.clear();
        }
    };
    for segment in segments {
        match *segment {
            PathSegment::Move(to) => {
                flush(&mut current, &mut out, false);
                current.push(to);
                (start, at) = (to, to);
            }
            PathSegment::Line(to) => {
                if current.is_empty() {
                    current.push(at);
                }
                current.push(to);
                at = to;
            }
            PathSegment::Cubic(c1, c2, to) => {
                if current.is_empty() {
                    current.push(at);
                }
                let from = at;
                for part in 1..=PARTS {
                    let t = part as f64 / PARTS as f64;
                    let u = 1.0 - t;
                    let mix = |a: f64, b: f64, c: f64, d: f64| u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t * t * t * d;
                    current.push((mix(from.0, c1.0, c2.0, to.0), mix(from.1, c1.1, c2.1, to.1)));
                }
                at = to;
            }
            PathSegment::Quadratic(control, to) => {
                if current.is_empty() {
                    current.push(at);
                }
                let from = at;
                for part in 1..=PARTS {
                    let t = part as f64 / PARTS as f64;
                    let u = 1.0 - t;
                    let mix = |a: f64, b: f64, c: f64| u * u * a + 2.0 * u * t * b + t * t * c;
                    current.push((mix(from.0, control.0, to.0), mix(from.1, control.1, to.1)));
                }
                at = to;
            }
            PathSegment::Arc { center, radii, rotation, start: angle, delta } => {
                if current.is_empty() {
                    current.push(at);
                }
                let (sin, cos) = rotation.sin_cos();
                let parts = ((delta.abs() / std::f64::consts::FRAC_PI_2).ceil() as usize).max(1) * PARTS / 2;
                for part in 1..=parts {
                    let theta = angle + delta * part as f64 / parts as f64;
                    let (s, c) = theta.sin_cos();
                    current.push((center.0 + radii.0 * cos * c - radii.1 * sin * s, center.1 + radii.0 * sin * c + radii.1 * cos * s));
                }
                at = current.last().copied().unwrap_or(at);
            }
            PathSegment::Close => {
                flush(&mut current, &mut out, true);
                at = start;
            }
        }
    }
    flush(&mut current, &mut out, false);
    out
}
