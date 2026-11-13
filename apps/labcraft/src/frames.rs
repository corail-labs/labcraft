//! The time of each frame, and a motion to time it over (`docs/architecture.md`, 11.6: 60 frames/s with 1,000 nodes and
//! 2,000 wires, measured by hand before the tag). Only when `LABCRAFT_FRAMES` is set (`docs/cli.md`, section 8):
//! `cargo xtask frames` opens the viewer so, on a function of 1,000 nodes and 2,000 wires.
//!
//! With it, the window paints without pause, so that the time between two frames is what a frame costs, or the period
//! of the display when it costs less. A corner of the canvas shows the last 2 seconds; stderr gets the motion, then the
//! rest until the window closes.

use std::collections::VecDeque;

use egui::Vec2;

/// The variable that turns the timing on, whatever its value.
pub const VARIABLE: &str = "LABCRAFT_FRAMES";
/// A frame at 60 frames/s.
pub const BUDGET: f64 = 1.0 / 60.0;
/// A frame later than this missed a refresh of a display at 60 Hz.
const LATE: f64 = 1.5 * BUDGET;
/// What the corner shows.
const RECENT: f64 = 2.0;
/// The histograms of a phase, by tenths of a millisecond, up to a second; a longer frame counts in the last.
const BUCKETS: usize = 10_000;
/// The motion starts this long after the first drawing, once it has been tessellated and fitted.
pub const MOTION_DELAY: f64 = 1.0;

/// Milliseconds: the median, the 95th percentile and the longest.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Spread {
    pub p50: f64,
    pub p95: f64,
    pub max: f64,
}

/// Frames over some time.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stats {
    pub frames: usize,
    pub seconds: f64,
    /// Between two frames.
    pub interval: Spread,
    /// What eframe counts of a frame, the vsync not counted: none when it said nothing.
    pub cpu: Option<Spread>,
    /// Frames later than 1.5 times the budget.
    pub late: usize,
}

impl Stats {
    pub fn per_second(&self) -> f64 {
        if self.seconds > 0.0 { self.frames as f64 / self.seconds } else { 0.0 }
    }

    /// 60 frames/s: no fewer than 59 a second, for the jitter of the display, and no frame later than 1.5 budgets in
    /// more than 1 % of them. On a display at 60 Hz: on a faster one, a frame between 1/60 s and 1.5/60 s missed 60
    /// frames/s without being late.
    pub fn holds_60(&self) -> bool {
        self.frames > 0 && self.per_second() >= 59.0 && self.late * 100 <= self.frames
    }

    /// One line, for the corner and for stderr.
    pub fn line(&self) -> String {
        let spread = |spread: &Spread| format!("p50 {:.1} p95 {:.1} max {:.1} ms", spread.p50, spread.p95, spread.max);
        let cpu = self.cpu.as_ref().map_or("not given".to_owned(), spread);
        format!(
            "{:.1} frames/s ({}): frame {}, cpu {cpu}, {} late of {} in {:.1} s",
            self.per_second(),
            if self.holds_60() { "holds 60" } else { "below 60" },
            spread(&self.interval),
            self.late,
            self.frames,
            self.seconds
        )
    }
}

#[derive(Clone, Copy)]
struct Sample {
    at: f64,
    interval: f64,
    cpu: Option<f64>,
}

/// The frames of a phase, in bounded memory however long it lasts.
struct Phase {
    intervals: Vec<u32>,
    cpus: Vec<u32>,
    frames: usize,
    cpu_frames: usize,
    seconds: f64,
    late: usize,
    longest: f64,
    longest_cpu: f64,
}

impl Phase {
    fn new() -> Phase {
        Phase { intervals: vec![0; BUCKETS], cpus: vec![0; BUCKETS], frames: 0, cpu_frames: 0, seconds: 0.0, late: 0, longest: 0.0, longest_cpu: 0.0 }
    }

    fn add(&mut self, sample: Sample) {
        bump(&mut self.intervals, sample.interval);
        self.frames += 1;
        self.seconds += sample.interval;
        self.late += usize::from(sample.interval > LATE);
        self.longest = self.longest.max(sample.interval);
        if let Some(cpu) = sample.cpu {
            bump(&mut self.cpus, cpu);
            self.cpu_frames += 1;
            self.longest_cpu = self.longest_cpu.max(cpu);
        }
    }

    fn stats(&self) -> Stats {
        let spread = |buckets: &[u32], count: usize, longest: f64| Spread {
            p50: quantile(buckets, count, 0.50),
            p95: quantile(buckets, count, 0.95),
            max: longest * 1000.0,
        };
        Stats {
            frames: self.frames,
            seconds: self.seconds,
            interval: spread(&self.intervals, self.frames, self.longest),
            cpu: (self.cpu_frames > 0).then(|| spread(&self.cpus, self.cpu_frames, self.longest_cpu)),
            late: self.late,
        }
    }
}

fn bump(buckets: &mut [u32], seconds: f64) {
    let bucket = ((seconds.max(0.0) * 10_000.0) as usize).min(BUCKETS - 1);
    if let Some(count) = buckets.get_mut(bucket) {
        *count = count.saturating_add(1);
    }
}

/// The upper edge of the bucket that holds the quantile, in milliseconds.
fn quantile(buckets: &[u32], count: usize, quantile: f64) -> f64 {
    let rank = ((count as f64 * quantile).ceil() as usize).max(1);
    let mut seen = 0usize;
    for (bucket, &n) in buckets.iter().enumerate() {
        seen += n as usize;
        if seen >= rank {
            return (bucket + 1) as f64 / 10.0;
        }
    }
    0.0
}

/// The time of the frames, from `tick` at each one.
pub struct Frames {
    last: Option<f64>,
    recent: VecDeque<Sample>,
    phase: Phase,
    /// A part of the phase: one move of the motion.
    part: Phase,
}

impl Default for Frames {
    fn default() -> Self {
        Frames::new()
    }
}

impl Frames {
    pub fn new() -> Frames {
        Frames { last: None, recent: VecDeque::new(), phase: Phase::new(), part: Phase::new() }
    }

    /// A frame at `now`, in seconds, and the CPU time eframe gives for the frame before. A second pass of the same frame,
    /// which egui runs when a widget asks for it with the same time, is not another frame.
    pub fn tick(&mut self, now: f64, cpu: Option<f32>) {
        if self.last == Some(now) {
            return;
        }
        if let Some(last) = self.last.replace(now) {
            let sample = Sample { at: now, interval: (now - last).max(0.0), cpu: cpu.map(f64::from) };
            self.phase.add(sample);
            self.part.add(sample);
            self.recent.push_back(sample);
        }
        while self.recent.front().is_some_and(|sample| sample.at < now - RECENT) {
            self.recent.pop_front();
        }
    }

    /// The last 2 seconds, each frame, at every frame: their percentiles exact, by the rank of the frame, not by a
    /// bucket of a histogram.
    pub fn recent(&self) -> Stats {
        let spread = |mut values: Vec<f64>| {
            values.sort_by(f64::total_cmp);
            let at = |quantile: f64| {
                let rank = ((values.len() as f64 * quantile).ceil() as usize).max(1);
                values.get(rank - 1).copied().unwrap_or(0.0) * 1000.0
            };
            Spread { p50: at(0.50), p95: at(0.95), max: values.last().copied().unwrap_or(0.0) * 1000.0 }
        };
        let cpus: Vec<f64> = self.recent.iter().filter_map(|sample| sample.cpu).collect();
        Stats {
            frames: self.recent.len(),
            seconds: self.recent.iter().map(|sample| sample.interval).sum(),
            interval: spread(self.recent.iter().map(|sample| sample.interval).collect()),
            cpu: (!cpus.is_empty()).then(|| spread(cpus)),
            late: self.recent.iter().filter(|sample| sample.interval > LATE).count(),
        }
    }

    /// The phase so far.
    pub fn phase(&self) -> Stats {
        self.phase.stats()
    }

    /// Ends the phase and starts another, and a part with it: what the one ended counted.
    pub fn next_phase(&mut self) -> Stats {
        self.part = Phase::new();
        std::mem::replace(&mut self.phase, Phase::new()).stats()
    }

    /// Ends a part of the phase and starts another: what the one ended counted.
    pub fn next_part(&mut self) -> Stats {
        std::mem::replace(&mut self.part, Phase::new()).stats()
    }
}

/// Where the motion goes: the zoom that fits the drawing, at 100 % at most, or a zoom.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Zoom {
    Fit,
    /// Just above `zoom.coarse`: the lowest zoom where every node shows whole, the most of them the viewport holds.
    AboveCoarse,
    At(f32),
}

/// Seconds from the start, the zoom, the left of the viewport, from the left of the drawing (0) to its right (1), and
/// what the move that ends there does.
const KEYS: [(f64, Zoom, f32, &str); 8] = [
    (0.0, Zoom::Fit, 0.0, ""),
    (3.0, Zoom::Fit, 1.0, "pan at the fit"),
    (5.0, Zoom::At(1.0), 1.0, "zoom in to 100 %"),
    (7.0, Zoom::At(1.0), 0.95, "pan at 100 %"),
    (8.0, Zoom::AboveCoarse, 0.9, "zoom out to zoom.coarse"),
    (10.0, Zoom::AboveCoarse, 0.7, "pan at zoom.coarse"),
    (11.0, Zoom::Fit, 0.6, "zoom out to the fit"),
    (12.0, Zoom::Fit, 0.0, "pan back at the fit"),
];

/// How long the motion lasts.
pub fn motion_seconds() -> f64 {
    KEYS.last().map_or(0.0, |key| key.0)
}

/// The move of the motion at `t` seconds from its start, as its number and what it does; nothing outside the motion.
pub fn move_at(t: f64) -> Option<(usize, &'static str)> {
    if !(0.0..=motion_seconds()).contains(&t) {
        return None;
    }
    let after = KEYS.iter().position(|key| key.0 >= t).unwrap_or(0).max(1);
    KEYS.get(after).map(|key| (after, key.3))
}

/// The viewport at `t` seconds from the start of the motion: its zoom and the point of the drawing at its top-left
/// corner, in units, for a drawing of `drawing` units in a canvas of `canvas` points, `coarse` the zoom of
/// `zoom.coarse`. Nothing once the motion is over. Pans along the drawing zoomed out, zooms in at its end, pans at
/// 100 %, then just above `zoom.coarse`, and zooms out; the zoom goes geometrically from a key to the next. In height,
/// it follows the nodes: `centres`, those of the nodes in units, ordered by x, give the middle of the viewport, from the
/// node under its middle, so that a drawing taller than the canvas never shows only the wires between them.
pub fn viewport_at(t: f64, drawing: Vec2, centres: &[Vec2], canvas: Vec2, coarse: f32) -> Option<(f32, Vec2)> {
    if !(0.0..=motion_seconds()).contains(&t) {
        return None;
    }
    let fit = (canvas.x / drawing.x.max(1.0)).min(canvas.y / drawing.y.max(1.0)).clamp(labcraft_editor::MIN_ZOOM, 1.0);
    let zoom_of = |zoom: Zoom| match zoom {
        Zoom::Fit => fit,
        Zoom::AboveCoarse => (coarse * 1.05).max(fit),
        Zoom::At(zoom) => zoom.max(fit),
    };
    let after = KEYS.iter().position(|key| key.0 >= t).unwrap_or(0);
    let (from, to) = match (KEYS.get(after.saturating_sub(1)), KEYS.get(after)) {
        (Some(from), Some(to)) => (from, to),
        _ => return None,
    };
    let span = to.0 - from.0;
    let s = if span > 0.0 { ((t - from.0) / span) as f32 } else { 1.0 };
    let (z0, z1) = (zoom_of(from.1), zoom_of(to.1));
    let zoom = z0 * (z1 / z0).powf(s);
    let left = from.2 + (to.2 - from.2) * s;
    let seen = canvas / zoom;
    // Centred where the drawing is smaller than the canvas.
    let room = drawing - seen;
    let x = if room.x > 0.0 { left * room.x } else { room.x / 2.0 };
    let y = if room.y > 0.0 {
        let middle = height_at(centres, x + seen.x / 2.0).unwrap_or(drawing.y / 2.0);
        (middle - seen.y / 2.0).clamp(0.0, room.y)
    } else {
        room.y / 2.0
    };
    Some((zoom, Vec2::new(x, y)))
}

/// The height of the nodes at `x`, between the two nearest on each side, from `centres` ordered by x.
fn height_at(centres: &[Vec2], x: f32) -> Option<f32> {
    let after = centres.partition_point(|centre| centre.x < x);
    match (after.checked_sub(1).and_then(|before| centres.get(before)), centres.get(after)) {
        (Some(a), Some(b)) if b.x > a.x => Some(a.y + (b.y - a.y) * (x - a.x) / (b.x - a.x)),
        (Some(a), _) => Some(a.y),
        (None, b) => b.map(|b| b.y),
    }
}

/// The centres of the nodes of `drawing`, in units, ordered by x, for `viewport_at`.
pub fn centres(drawing: &labcraft_diagram::Drawing) -> Vec<Vec2> {
    columns(
        drawing.placed.iter().map(|placed| Vec2::new((placed.x + placed.width / 2) as f32, (placed.y + placed.height / 2) as f32) / 100.0).collect(),
    )
}

/// `points` ordered by x, those of one x as one, at their mean height: the motion does not jump from the top of a column
/// to its bottom as its middle crosses it.
fn columns(mut points: Vec<Vec2>) -> Vec<Vec2> {
    points.sort_by(|a, b| a.x.total_cmp(&b.x));
    let mut columns: Vec<(Vec2, f32)> = Vec::new();
    for point in points {
        match columns.last_mut() {
            Some((sum, count)) if sum.x / *count == point.x => {
                *sum += point;
                *count += 1.0;
            }
            _ => columns.push((point, 1.0)),
        }
    }
    columns.into_iter().map(|(sum, count)| sum / count).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_steady_display_at_60_hz_holds_60() {
        let mut frames = Frames::new();
        for k in 0..=600 {
            frames.tick(f64::from(k) * BUDGET, Some(0.004));
        }
        let stats = frames.phase();
        assert_eq!(stats.frames, 600);
        assert!((stats.per_second() - 60.0).abs() < 1e-6, "{stats:?}");
        // 16.67 ms is in the bucket of 16.6 to 16.7 ms; 4 ms in that of 4.0 to 4.1 ms.
        assert_eq!(stats.interval.p50, 16.7);
        assert_eq!(stats.interval.p95, 16.7);
        assert!((stats.interval.max - 1000.0 / 60.0).abs() < 1e-6);
        assert_eq!(stats.cpu.map(|cpu| cpu.p50), Some(4.1));
        assert_eq!(stats.late, 0);
        assert!(stats.holds_60(), "{}", stats.line());
        // The corner: the last 2 seconds, 120 frames, and the one at their edge, as the sum rounds.
        assert!((120..=121).contains(&frames.recent().frames), "{}", frames.recent().frames);
    }

    #[test]
    fn frames_missed_count_late_and_below_60() {
        let mut frames = Frames::new();
        let mut now = 0.0;
        frames.tick(now, None);
        // 90 frames on time, then 10 at twice the budget: 100 frames in 1.83 s, 54.5 a second.
        for k in 0..100 {
            now += if k < 90 { BUDGET } else { 2.0 * BUDGET };
            frames.tick(now, None);
        }
        let stats = frames.next_phase();
        assert_eq!((stats.frames, stats.late, stats.cpu), (100, 10, None));
        assert!(stats.interval.p95 > 33.0 && stats.interval.p50 < 17.0, "{stats:?}");
        assert!(!stats.holds_60(), "{}", stats.line());
        assert!(stats.line().contains("below 60") && stats.line().contains("cpu not given"), "{}", stats.line());
        // The next phase starts empty.
        assert_eq!(frames.phase().frames, 0);
    }

    #[test]
    fn a_frame_of_a_second_or_more_counts_in_the_last_bucket() {
        let mut frames = Frames::new();
        frames.tick(0.0, None);
        frames.tick(5.0, Some(4.0));
        let stats = frames.phase();
        assert_eq!(stats.interval.p50, 1000.0);
        assert_eq!(stats.interval.max, 5000.0);
        assert_eq!(stats.cpu.map(|cpu| cpu.max), Some(4000.0));
    }

    /// The motion over the drawing of `cargo xtask frames`, 59,603 by 3,224 units, in a canvas of 1,100 by 640 points, its
    /// nodes on a slope from the bottom left to the top right, as they are placed there.
    #[test]
    fn the_motion_pans_and_zooms_over_the_drawing_then_ends() {
        let (drawing, canvas) = (Vec2::new(59_603.0, 3_224.0), Vec2::new(1_100.0, 640.0));
        let centres: Vec<Vec2> = (0..1000).map(|k| Vec2::new(k as f32 * 59.6, 3_000.0 - k as f32 * 2.8)).collect();
        let at = |t: f64| viewport_at(t, drawing, &centres, canvas, 0.4);
        // Zoomed out as far as the canvas goes, from the left, to the right at 3 s.
        let (zoom, origin) = at(0.0).unwrap();
        assert_eq!(zoom, labcraft_editor::MIN_ZOOM);
        assert_eq!(origin.x, 0.0);
        let (_, right) = at(3.0).unwrap();
        assert!((right.x - (59_603.0 - 1_100.0 / 0.05)).abs() < 0.5, "{right:?}");
        // Taller than the drawing: centred in height.
        assert_eq!(origin.y, (3_224.0 - 640.0 / 0.05) / 2.0);
        // At 100 % at 5 s, its right edge at the right of the canvas, and at the top, where the last nodes are.
        let (zoom, origin) = at(5.0).unwrap();
        assert_eq!(zoom, 1.0);
        assert!((origin.x - (59_603.0 - 1_100.0)).abs() < 0.5 && origin.y == 0.0, "{origin:?}");
        // Worked out by hand. At 7 s, 100 %, the left at 95 % of the room: the middle at x 0.95 × 58,503 + 550 = 56,128,
        // where the nodes are at y 3,000 - 56,128 / 59.6 × 2.8 = 363.1, the middle of the canvas, so the top at 43.1.
        let (_, origin) = at(7.0).unwrap();
        assert!((origin.x - 0.95 * (59_603.0 - 1_100.0)).abs() < 0.5 && (origin.y - 43.1).abs() < 0.5, "{origin:?}");
        // At 9 s, 42 %, 2,619 by 1,524 units seen, the left at 80 %: the middle at x 0.8 × 56,984 + 1,310 = 46,897, the
        // nodes at y 796.8, so the top at 796.8 - 762 = 34.9.
        let (zoom, origin) = at(9.0).unwrap();
        assert!((zoom - 0.42).abs() < 1e-6 && (origin.y - 34.9).abs() < 0.5, "{zoom} {origin:?}");
        // Halfway from 5 % to 100 %, geometrically.
        let (zoom, _) = at(4.0).unwrap();
        assert!((zoom - (0.05f32 * 1.0).sqrt()).abs() < 1e-4, "{zoom}");
        // Over after 12 s, and not before it starts.
        assert!(at(12.0).is_some() && at(12.01).is_none() && at(-0.01).is_none());
    }

    /// `on_time` frames of a budget, then `late` of two.
    fn stats_of(on_time: usize, late: usize) -> Stats {
        let mut frames = Frames::new();
        let mut now = 0.0;
        frames.tick(now, None);
        for k in 0..on_time + late {
            now += if k < on_time { BUDGET } else { 2.0 * BUDGET };
            frames.tick(now, None);
        }
        frames.phase()
    }

    /// Each rule alone: 59 frames a second at least, 1 % late at most.
    #[test]
    fn holds_60_needs_both_rules() {
        // 1 late of 100: 101 budgets for 100 frames, 59.4 a second, and 1 %: holds.
        assert!(stats_of(99, 1).holds_60());
        // 2 late of 200, 1 %, 202 budgets, 59.4 a second: holds; 3 of 200, 1.5 %, 203 budgets, 59.1 a second: not, by
        // the late alone.
        assert!(stats_of(198, 2).holds_60());
        let three = stats_of(197, 3);
        assert!(three.per_second() >= 59.0 && !three.holds_60(), "{}", three.line());
        // No frame late, but at 1.02 budgets each: 58.8 a second, not by the rate alone.
        let mut frames = Frames::new();
        for k in 0..=100 {
            frames.tick(f64::from(k) * 1.02 * BUDGET, None);
        }
        let slow = frames.phase();
        assert!(slow.late == 0 && !slow.holds_60(), "{}", slow.line());
    }

    /// egui runs a frame twice when a widget asks for it, with the same time: one frame.
    #[test]
    fn a_second_pass_of_a_frame_is_not_a_frame() {
        let mut frames = Frames::new();
        for k in 0..=60 {
            frames.tick(f64::from(k) * BUDGET, None);
            frames.tick(f64::from(k) * BUDGET, None);
        }
        assert_eq!((frames.phase().frames, frames.recent().frames), (60, 60));
        assert!(frames.phase().holds_60());
    }

    /// The corner: exact percentiles of the last frames, by rank.
    #[test]
    fn the_recent_frames_have_exact_percentiles() {
        let mut frames = Frames::new();
        let mut now = 0.0;
        frames.tick(now, Some(0.001));
        // 20 frames of 10, 12, …, 48 ms, none at 25 ms, the edge of late: the 10th is 28 ms, the 19th 46 ms.
        for k in 0..20 {
            now += f64::from(10 + 2 * k) / 1000.0;
            frames.tick(now, Some(0.002));
        }
        let recent = frames.recent();
        assert_eq!(recent.frames, 20);
        assert!((recent.interval.p50 - 28.0).abs() < 1e-9 && (recent.interval.p95 - 46.0).abs() < 1e-9, "{recent:?}");
        assert!((recent.interval.max - 48.0).abs() < 1e-9);
        // Over 1.5 budgets, 25 ms: 26 to 48 ms, 12 frames.
        assert_eq!(recent.late, 12);
        assert!(recent.cpu.is_some_and(|cpu| (cpu.p50 - 2.0).abs() < 1e-4), "{recent:?}");
    }

    #[test]
    fn the_nodes_of_one_column_count_at_their_mean_height() {
        let points = vec![Vec2::new(10.0, 300.0), Vec2::new(0.0, 5.0), Vec2::new(10.0, 100.0), Vec2::new(20.0, 50.0)];
        assert_eq!(columns(points), vec![Vec2::new(0.0, 5.0), Vec2::new(10.0, 200.0), Vec2::new(20.0, 50.0)]);
    }

    #[test]
    fn the_moves_follow_the_keys() {
        assert_eq!(move_at(0.0), Some((1, "pan at the fit")));
        assert_eq!(move_at(3.0), Some((1, "pan at the fit")));
        assert_eq!(move_at(3.01), Some((2, "zoom in to 100 %")));
        assert_eq!(move_at(12.0), Some((7, "pan back at the fit")));
        assert_eq!(move_at(12.01), None);
        assert_eq!(move_at(-0.01), None);
    }

    #[test]
    fn the_height_of_the_nodes_is_between_the_nearest_two() {
        let centres = [Vec2::new(0.0, 0.0), Vec2::new(10.0, 100.0), Vec2::new(10.0, 300.0), Vec2::new(20.0, 200.0)];
        assert_eq!(height_at(&centres, 5.0), Some(50.0));
        assert_eq!(height_at(&centres, 15.0), Some(250.0));
        // Before the first and after the last, theirs; nothing without nodes.
        assert_eq!(height_at(&centres, -5.0), Some(0.0));
        assert_eq!(height_at(&centres, 25.0), Some(200.0));
        assert_eq!(height_at(&[], 5.0), None);
    }

    /// A drawing that fits at 100 % stays at 100 % and in the middle: the motion never zooms below the fit.
    #[test]
    fn a_small_drawing_stays_centred_at_its_fit() {
        let (drawing, canvas) = (Vec2::new(400.0, 200.0), Vec2::new(1_100.0, 640.0));
        for k in 0..=120 {
            let (zoom, origin) = viewport_at(f64::from(k) / 10.0, drawing, &[Vec2::new(200.0, 100.0)], canvas, 0.4).unwrap();
            assert_eq!(zoom, 1.0);
            assert_eq!(origin, Vec2::new((400.0 - 1_100.0) / 2.0, (200.0 - 640.0) / 2.0));
        }
    }
}
