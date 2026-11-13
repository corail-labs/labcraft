//! The values of a wire over the iterations of its loop (notation 6.1): a small curve for a real number, a list
//! otherwise, cut after its first 1,000 values with a note. In the tooltip of the wire hovered, and pinned when the wire
//! is selected, where a click on a point or a line chooses that iteration.

use egui::{Pos2, Rect, Sense, Stroke, Ui, Vec2};
use labcraft_diagram::Series;
use labcraft_diagram::values::SERIES;
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
    let shown = series.values.len() as u64;
    let picked = if series.curve { curve(ui, series, theme, pick) } else { list(ui, series, pick) };
    let note = if series.count > shown {
        format!("The first {} of {} iterations", group(SERIES), group(series.count))
    } else {
        format!("{} iteration{}", group(series.count), if series.count == 1 { "" } else { "s" })
    };
    ui.label(egui::RichText::new(note).small().weak());
    picked
}

/// The finite number of a value of a series.
fn finite(value: &Option<(String, Option<f64>)>) -> Option<f64> {
    value.as_ref().and_then(|(_, number)| *number).filter(|number| number.is_finite())
}

/// The lowest and the highest finite values; none when there is none.
fn bounds(series: &Series) -> Option<(f64, f64)> {
    series.values.iter().filter_map(finite).fold(None, |found, value| match found {
        None => Some((value, value)),
        Some((low, high)) => Some((f64::min(low, value), f64::max(high, value))),
    })
}

/// The points of a curve in `area`: the finite values, from the lowest at the bottom to the highest at the top, each at
/// the place of its iteration; none for an iteration that gives no finite value.
pub fn points(series: &Series, area: Rect) -> Vec<Option<Pos2>> {
    let Some((low, high)) = bounds(series) else { return vec![None; series.values.len()] };
    // In halves, so that the span of values near the largest float stays finite; none between two that halve alike.
    let span = high / 2.0 - low / 2.0;
    let last = series.values.len().saturating_sub(1).max(1) as f32;
    series
        .values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let number = finite(value)?;
            // A constant series lies in the middle.
            let height = if span > 0.0 { ((number / 2.0 - low / 2.0) / span) as f32 } else { 0.5 };
            Some(Pos2::new(area.left() + area.width() * index as f32 / last, area.bottom() - area.height() * height))
        })
        .collect()
}

/// The iteration under `x` in a curve drawn in `area`.
pub fn iteration_at(series: &Series, area: Rect, x: f32) -> u64 {
    let last = series.values.len().saturating_sub(1);
    let at = ((x - area.left()) / area.width().max(1.0) * last as f32).round();
    (at.max(0.0) as u64).min(last as u64)
}

/// The text of the first value equal to `number`, as its bubble writes it: the bounds of a curve.
fn text_of(series: &Series, number: f64) -> &str {
    series.values.iter().find_map(|value| value.as_ref().filter(|_| finite(value) == Some(number)).map(|(text, _)| text.as_str())).unwrap_or_default()
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
    let placed = points(series, area.shrink(3.0));
    // One line through each run of neighbours that have a value; a value alone is a dot.
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
    if let Some(Some(point)) = usize::try_from(series.shown).ok().and_then(|shown| placed.get(shown)) {
        painter.circle(*point, 4.0, theme.color("probe.fill"), Stroke::new(1.5, theme.color("select.stroke")));
    }
    if let Some((low, _)) = bounds {
        ui.label(egui::RichText::new(text_of(series, low)).small().monospace());
    }
    // The value under the pointer, and the iteration a click chooses.
    let mut picked = None;
    if let Some(at) = response.hover_pos() {
        let iteration = iteration_at(series, area.shrink(3.0), at.x);
        let text = usize::try_from(iteration)
            .ok()
            .and_then(|index| series.values.get(index))
            .and_then(|value| value.as_ref().map(|(text, _)| text.clone()))
            .unwrap_or_else(|| "no value".to_owned());
        painter.vline(at.x, area.y_range(), Stroke::new(1.0, theme.color("ink.faint").gamma_multiply(0.6)));
        ui.label(egui::RichText::new(format!("{}: {text}", iteration + 1)).small().monospace());
        if pick && response.clicked() {
            picked = Some(iteration);
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
    egui::ScrollArea::vertical().max_height(LIST_HEIGHT).auto_shrink([false, true]).show_rows(ui, row, series.values.len(), |ui, rows| {
        for (index, value) in series.values.iter().enumerate().skip(rows.start).take(rows.len()) {
            let iteration = index as u64;
            let text = value.as_ref().map_or("no value", |(text, _)| text.as_str());
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
        Series {
            key: "n".to_owned(),
            count: values.len() as u64,
            shown: 0,
            values: values.iter().map(|value| value.map(|number| (number.to_string(), Some(number)))).collect(),
            curve: true,
            unit: String::new(),
        }
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

    #[test]
    fn a_click_on_a_curve_chooses_the_nearest_iteration() {
        let area = Rect::from_min_size(Pos2::ZERO, Vec2::new(100.0, 10.0));
        let five = series(&[Some(0.0); 5]);
        assert_eq!([0.0, 12.0, 13.0, 99.0, 150.0, -20.0].map(|x| iteration_at(&five, area, x)), [0, 0, 1, 4, 4, 0]);
    }
}
