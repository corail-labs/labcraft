//! The values of a wire over the iterations of its loop (notation 6.1): a small curve for a real number, over the whole
//! run, a list otherwise, of at most 1,000 iterations in a row around the one shown, with a note. In the tooltip of the
//! wire hovered, and pinned when the wire is selected, where a click on a point or a line chooses that iteration.

use egui::{Pos2, Rect, Sense, Stroke, Ui, Vec2};
use labcraft_diagram::{Sample, Series};
use labcraft_types::text::grouped as group;

use crate::Theme;

/// The size of a curve, in points *(proposed)*.
const CURVE: Vec2 = Vec2::new(240.0, 72.0);
/// The most lines of a list shown at once; the others scroll *(proposed)*.
const LIST_HEIGHT: f32 = 200.0;

/// Shows `series` under `title`; with `pick`, a click on a point of the curve or a line of the list gives that iteration,
/// from 0.
pub fn show(ui: &mut Ui, title: &str, series: &Series, theme: Theme, pick: bool) -> Option<u64> {
    ui.label(title);
    let picked = if series.curve { curve(ui, series, theme, pick) } else { list(ui, series, pick) };
    ui.label(egui::RichText::new(note(series)).small().weak());
    picked
}

/// What the note under a series says: the iterations, and which of them were read when not all.
pub fn note(series: &Series) -> String {
    let read = series.values.len() as u64;
    let plural = if series.count == 1 { "" } else { "s" };
    match (series.values.first(), series.values.last()) {
        (Some(_), Some(_)) if read < series.count && series.curve => {
            format!("{} of {} iterations, spread evenly", group(read), group(series.count))
        }
        (Some(first), Some(last)) if read < series.count => {
            format!("Iterations {}\u{2013}{} of {}", group(first.iteration + 1), group(last.iteration + 1), group(series.count))
        }
        _ => format!("{} iteration{plural}", group(series.count)),
    }
}

/// The finite number of a sample.
fn finite(sample: &Sample) -> Option<f64> {
    sample.value.as_ref().and_then(|(_, number)| *number).filter(|number| number.is_finite())
}

/// The lowest and the highest finite values; none when there is none.
fn bounds(series: &Series) -> Option<(f64, f64)> {
    series.values.iter().filter_map(finite).fold(None, |found, value| match found {
        None => Some((value, value)),
        Some((low, high)) => Some((f64::min(low, value), f64::max(high, value))),
    })
}

/// The place of `iteration` across a curve of `area`: the first at the left, the last of the run at the right.
fn x_of(series: &Series, area: Rect, iteration: u64) -> f32 {
    let last = series.count.saturating_sub(1).max(1) as f64;
    area.left() + area.width() * (iteration as f64 / last) as f32
}

/// The points of a curve in `area`, one for each sample: the finite values, from the lowest at the bottom to the highest
/// at the top, each at the place of its iteration in the run; none for a sample that gives no finite value.
pub fn points(series: &Series, area: Rect) -> Vec<Option<Pos2>> {
    let Some((low, high)) = bounds(series) else { return vec![None; series.values.len()] };
    // In halves, so that the span of values near the largest float stays finite; none between two that halve alike.
    let span = high / 2.0 - low / 2.0;
    series
        .values
        .iter()
        .map(|sample| {
            let number = finite(sample)?;
            // A constant series lies in the middle.
            let height = if span > 0.0 { ((number / 2.0 - low / 2.0) / span) as f32 } else { 0.5 };
            Some(Pos2::new(x_of(series, area, sample.iteration), area.bottom() - area.height() * height))
        })
        .collect()
}

/// The sample nearest `x` in a curve drawn in `area`, by its index in the series; none in a series without a sample.
pub fn sample_at(series: &Series, area: Rect, x: f32) -> Option<usize> {
    let last = series.count.saturating_sub(1);
    let at = ((x - area.left()) / area.width().max(1.0) * last as f32).round();
    let iteration = (at.max(0.0) as u64).min(last);
    let after = series.values.partition_point(|sample| sample.iteration < iteration);
    let before = after.checked_sub(1);
    let distance = |index: usize| series.values.get(index).map(|sample| sample.iteration.abs_diff(iteration));
    match (before.and_then(|index| distance(index).map(|d| (index, d))), distance(after).map(|d| (after, d))) {
        (Some((index, low)), Some((_, high))) if low <= high => Some(index),
        (_, Some((index, _))) | (Some((index, _)), None) => Some(index),
        (None, None) => None,
    }
}

/// The text of the first value equal to `number`, as its bubble writes it: the bounds of a curve.
fn text_of(series: &Series, number: f64) -> &str {
    series
        .values
        .iter()
        .find_map(|sample| sample.value.as_ref().filter(|_| finite(sample) == Some(number)).map(|(text, _)| text.as_str()))
        .unwrap_or_default()
}

fn curve(ui: &mut Ui, series: &Series, theme: Theme, pick: bool) -> Option<u64> {
    let bounds = bounds(series);
    if let Some((_, high)) = bounds {
        ui.label(egui::RichText::new(text_of(series, high)).small().monospace());
    }
    let sense = if pick { Sense::click() } else { Sense::hover() };
    let (area, response) = ui.allocate_exact_size(CURVE, sense);
    let painter = ui.painter_at(area.expand(4.0));
    painter.rect_stroke(area, 0.0, Stroke::new(1.0, theme.color("node.divider")), egui::StrokeKind::Inside);
    let line = Stroke::new(1.5, theme.color("wire.data"));
    let inner = area.shrink(3.0);
    let placed = points(series, inner);
    // One line through each run of neighbours that have a value; a value alone is a dot. Many samples to a pixel draw
    // the lowest and the highest of that pixel, the line going up and down between them.
    for run in placed.split(Option::is_none).filter(|run| !run.is_empty()) {
        let run: Vec<Pos2> = run.iter().flatten().copied().collect();
        match run.as_slice() {
            [alone] => {
                painter.circle_filled(*alone, 1.5, line.color);
            }
            _ => {
                painter.add(egui::Shape::line(run, line));
            }
        }
    }
    // The iteration shown, marked.
    let shown = series.values.iter().position(|sample| sample.iteration == series.shown);
    if let Some(Some(point)) = shown.and_then(|index| placed.get(index)) {
        painter.circle(*point, 4.0, theme.color("probe.fill"), Stroke::new(1.5, theme.color("select.stroke")));
    }
    if let Some((low, _)) = bounds {
        ui.label(egui::RichText::new(text_of(series, low)).small().monospace());
    }
    // The value under the pointer, and the iteration a click chooses.
    let mut picked = None;
    if let Some(at) = response.hover_pos()
        && let Some(sample) = sample_at(series, inner, at.x).and_then(|index| series.values.get(index))
    {
        let text = sample.value.as_ref().map_or("no value", |(text, _)| text.as_str());
        let x = x_of(series, inner, sample.iteration);
        painter.vline(x, area.y_range(), Stroke::new(1.0, theme.color("ink.faint").gamma_multiply(0.6)));
        ui.label(egui::RichText::new(format!("{}: {text}", group(sample.iteration + 1))).small().monospace());
        if pick && response.clicked() {
            picked = Some(sample.iteration);
        }
    }
    picked
}

fn list(ui: &mut Ui, series: &Series, pick: bool) -> Option<u64> {
    let mut picked = None;
    // Only the lines in sight are laid out.
    // The height of a row without the spacing, which `show_rows` adds: a line, or a line that can be picked.
    let text = ui.text_style_height(&egui::TextStyle::Monospace);
    let row = if pick { (text + 2.0 * ui.spacing().button_padding.y).max(ui.spacing().interact_size.y) } else { text };
    let mut area = egui::ScrollArea::vertical().max_height(LIST_HEIGHT).auto_shrink([false, true]);
    // The line of the iteration shown in the middle, when it first shows and when it changes; the developer scrolls
    // freely in between.
    let seen = ui.scope_id().with(("series", series.key.as_str()));
    if ui.data(|data| data.get_temp::<u64>(seen)) != Some(series.shown) {
        ui.data_mut(|data| data.insert_temp(seen, series.shown));
        let line = row + ui.spacing().item_spacing.y;
        let index = series.values.partition_point(|sample| sample.iteration < series.shown) as f32;
        area = area.vertical_scroll_offset((index * line - (LIST_HEIGHT - line) / 2.0).max(0.0));
    }
    area.show_rows(ui, row, series.values.len(), |ui, rows| {
        for sample in series.values.iter().skip(rows.start).take(rows.len()) {
            let iteration = sample.iteration;
            let text = sample.value.as_ref().map_or("no value", |(text, _)| text.as_str());
            let line = egui::RichText::new(format!("{:>5}  {text}", iteration + 1)).monospace();
            let response = if pick { ui.selectable_label(iteration == series.shown, line) } else { ui.label(line) };
            if iteration == series.shown && !pick {
                response.clone().highlight();
            }
            if pick && response.clicked() {
                picked = Some(iteration);
            }
        }
    });
    picked
}

#[cfg(test)]
mod tests {
    use super::*;

    fn series(values: &[Option<f64>]) -> Series {
        let values: Vec<Sample> = values
            .iter()
            .enumerate()
            .map(|(at, value)| Sample { iteration: at as u64, value: value.map(|number| (number.to_string(), Some(number))) })
            .collect();
        Series { key: "n".to_owned(), count: values.len() as u64, shown: 0, values, curve: true, unit: String::new() }
    }

    /// By hand: three values 0, 10, 5 over 100 points of width and 10 of height, the lowest at the bottom.
    #[test]
    fn a_curve_goes_from_the_lowest_value_at_the_bottom_to_the_highest_at_the_top() {
        let area = Rect::from_min_size(Pos2::ZERO, Vec2::new(100.0, 10.0));
        let placed = points(&series(&[Some(0.0), Some(10.0), Some(5.0)]), area);
        assert_eq!(placed, [Some(Pos2::new(0.0, 10.0)), Some(Pos2::new(50.0, 0.0)), Some(Pos2::new(100.0, 5.0))]);
        // A gap where an iteration gives no value, and none for what is not finite.
        let placed = points(&series(&[Some(1.0), None, Some(f64::NAN), Some(3.0)]), area);
        assert_eq!(placed.iter().map(Option::is_some).collect::<Vec<_>>(), [true, false, false, true]);
        // Two values that halve alike: in the middle, never NaN.
        assert_eq!(points(&series(&[Some(0.0), Some(5e-324)]), area), [Some(Pos2::new(0.0, 5.0)), Some(Pos2::new(100.0, 5.0))]);
        // Values near the largest float: the highest at the top, not lost.
        let wide = points(&series(&[Some(-f64::MAX), Some(f64::MAX)]), area);
        assert_eq!(wide, [Some(Pos2::new(0.0, 10.0)), Some(Pos2::new(100.0, 0.0))]);
        // A constant series in the middle.
        assert_eq!(points(&series(&[Some(2.0), Some(2.0)]), area), [Some(Pos2::new(0.0, 5.0)), Some(Pos2::new(100.0, 5.0))]);
    }

    /// By hand: samples at iterations 0, 50 and 100 of a run of 101 sit at 0, 50 and 100 of a width of 100.
    #[test]
    fn a_curve_of_samples_places_each_at_its_iteration_in_the_run() {
        let area = Rect::from_min_size(Pos2::ZERO, Vec2::new(100.0, 10.0));
        let mut spread = series(&[Some(0.0), Some(10.0), Some(5.0)]);
        spread.count = 101;
        for (sample, iteration) in spread.values.iter_mut().zip([0, 50, 100]) {
            sample.iteration = iteration;
        }
        assert_eq!(points(&spread, area), [Some(Pos2::new(0.0, 10.0)), Some(Pos2::new(50.0, 0.0)), Some(Pos2::new(100.0, 5.0))]);
        // The pointer at 30 is iteration 30, nearer 50 than 0; at 20, nearer 0; past the end, the last.
        assert_eq!([30.0, 20.0, 25.0, 150.0, -5.0].map(|x| sample_at(&spread, area, x)), [Some(1), Some(0), Some(0), Some(2), Some(0)]);
        assert_eq!(note(&spread), "3 of 101 iterations, spread evenly");
    }

    #[test]
    fn a_click_on_a_curve_chooses_the_nearest_iteration() {
        let area = Rect::from_min_size(Pos2::ZERO, Vec2::new(100.0, 10.0));
        let five = series(&[Some(0.0); 5]);
        assert_eq!([0.0, 12.0, 13.0, 99.0, 150.0, -20.0].map(|x| sample_at(&five, area, x)), [0, 0, 1, 4, 4, 0].map(Some));
        assert_eq!(note(&five), "5 iterations");
        assert_eq!(sample_at(&series(&[]), area, 10.0), None);
    }

    /// By hand: a list of iterations 1,001 to 2,000 of 5,000 says so, counted from 1.
    #[test]
    fn a_list_cut_says_which_iterations_it_shows() {
        let mut list = series(&[None, None]);
        list.curve = false;
        list.count = 5_000;
        list.values[0].iteration = 1_000;
        list.values[1].iteration = 1_999;
        assert_eq!(note(&list), "Iterations 1,001\u{2013}2,000 of 5,000");
    }
}
