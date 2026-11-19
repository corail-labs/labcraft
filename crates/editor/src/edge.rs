//! A terminal of the frame of the function moved by hand along its edge (notation 2.4, spec 11): dragged, or moved by
//! Ctrl and an arrow (Up, Down), 8 at a time or 64 with Shift; `terminal.move` at the drop, or at the end of a run of
//! presses. It stays on its edge, on the grid of 8, at least a place (24) from the other terminals of its edge, and its
//! order among them can change. Nothing is written while it moves: the canvas shows where it goes.

use egui::Pos2;
use labcraft_diagram::Drawing;

use crate::moving::GRID;
use crate::signature::{frame_height, frame_terminal};

/// The room between two terminals of one edge, in units (notation 2.1).
const PLACE: i64 = 24;
/// How far a terminal can be looked for a free place, in steps of the grid: past it, it stays where the pointer is.
const SEARCH: i64 = 4096;

/// How the move goes on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum State {
    Dragging,
    /// A run of presses of Ctrl and an arrow, the last at `last`, in seconds: one move (notation 2.6).
    Keys {
        last: f64,
    },
    /// Asked of the host: the terminal shows where it went until the new drawing comes, or the host says it failed.
    Asked,
}

/// A terminal of the frame of the function that moves.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Sliding {
    /// Its port, in [`Drawing::ports`].
    pub port: usize,
    /// As `terminal.move` names it: `input x` or `output x`.
    pub terminal: String,
    pub state: State,
    /// Where it is on the canvas across, and the height of the origin of the frame (spec 11), in units.
    x: f32,
    origin: f32,
    /// Where the press was, in units, for a drag.
    from: f32,
    /// Its height when the move started, and those of the other terminals of its edge, from the origin, in units.
    start: i64,
    others: Vec<i64>,
    /// The height it goes to, from the origin, in units.
    pub y: i64,
}

impl Sliding {
    /// The terminal of the frame of the function at `port`, pressed at the height `from`, in units.
    pub fn new(drawing: &Drawing, port: usize, state: State, from: f32) -> Option<Sliding> {
        let found = drawing.ports.get(port)?;
        let terminal = frame_terminal(found)?;
        let origin = drawing.bodies.first()?.origin.1;
        let others = drawing
            .ports
            .iter()
            .enumerate()
            .filter(|&(at, other)| at != port && other.input == found.input && frame_terminal(other).is_some())
            .filter_map(|(_, other)| frame_height(drawing, other))
            .collect();
        let start = frame_height(drawing, found)?;
        Some(Sliding { port, terminal, state, x: found.x as f32 / 100.0, origin: origin as f32 / 100.0, from, start, others, y: start })
    }

    /// The same move over a new drawing, its terminal found again by its name: where it started and where it goes stay,
    /// the other terminals are those of the new drawing. None when the terminal is gone.
    pub fn again(self, drawing: &Drawing) -> Option<Sliding> {
        let port = drawing.ports.iter().position(|port| frame_terminal(port).as_deref() == Some(self.terminal.as_str()))?;
        let fresh = Sliding::new(drawing, port, self.state, self.from)?;
        Some(Sliding { start: self.start, y: self.y, ..fresh })
    }

    /// The pointer at the height `at`, in units: the terminal goes to the place nearest it on the grid, free of the
    /// others.
    pub fn set_at(&mut self, at: f32) {
        let wanted = self.start + (at - self.from).round() as i64;
        let snapped = (wanted as f32 / GRID).round() as i64 * GRID as i64;
        let toward = if wanted >= self.start { 1 } else { -1 };
        self.y = nearest_free(snapped, &self.others, toward);
    }

    /// One press: by `by` units, to the next free place in that direction.
    pub fn step(&mut self, by: i64) {
        let snapped = ((self.y + by) as f32 / GRID).round() as i64 * GRID as i64;
        self.y = next_free(snapped, &self.others, by.signum());
    }

    /// Whether it goes elsewhere than where it was.
    pub fn changed(&self) -> bool {
        self.y != self.start
    }

    /// Where it goes on the canvas, in units.
    pub fn at(&self) -> Pos2 {
        Pos2::new(self.x, self.origin + self.y as f32)
    }
}

fn is_free(y: i64, others: &[i64]) -> bool {
    others.iter().all(|other| (y - other).abs() >= PLACE)
}

/// The free place on the grid nearest `y`, the one `toward` first when two are as near.
fn nearest_free(y: i64, others: &[i64], toward: i64) -> i64 {
    let step = GRID as i64;
    for k in 0..=SEARCH {
        for side in [toward, -toward] {
            let candidate = y + side * k * step;
            if is_free(candidate, others) {
                return candidate;
            }
        }
    }
    y
}

/// The first free place on the grid from `y` on, in the direction `toward`.
fn next_free(y: i64, others: &[i64], toward: i64) -> i64 {
    let step = GRID as i64 * if toward < 0 { -1 } else { 1 };
    (0..=SEARCH).map(|k| y + k * step).find(|&candidate| is_free(candidate, others)).unwrap_or(y)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sliding(start: i64, others: Vec<i64>) -> Sliding {
        Sliding { port: 0, terminal: "input a".to_owned(), state: State::Dragging, x: 0.0, origin: 0.0, from: 100.0, start, others, y: start }
    }

    /// Notation 2.4, by hand: on the grid of 8, at least 24 from another terminal of its edge, its order free to change.
    #[test]
    fn a_dragged_terminal_goes_on_the_grid_and_keeps_its_room() {
        let mut moving = sliding(40, vec![88, 136]);
        // 13 down from 40 is 53, on the grid 56.
        moving.set_at(113.0);
        assert_eq!(moving.y, 56);
        // 100 is 104 on the grid, within 24 of 88: the nearest free place, going down first, is 112, 24 from both.
        moving.set_at(160.0);
        assert_eq!(moving.y, 112);
        // Past the last one.
        moving.set_at(300.0);
        assert_eq!(moving.y, 240);
        moving.set_at(100.0);
        assert!(!moving.changed());
    }

    #[test]
    fn a_press_goes_to_the_next_free_place_in_its_direction() {
        let mut moving = sliding(40, vec![64]);
        // 48 and 56 are within 24 of 64: the next free one down is 88.
        moving.step(8);
        assert_eq!(moving.y, 88);
        moving.step(-8);
        assert_eq!(moving.y, 40);
        moving.step(-64);
        assert_eq!(moving.y, -24);
    }
}
