//! How far the whole drawing moved from one drawing of a function to the next, which the canvas follows and the figure
//! of stability leaves out (notation 2.1, #266).

use std::collections::HashMap;

use crate::Drawing;

/// How far the whole drawing moved from `old` to `new`, the same function drawn again, in hundredths. Across, by the frame
/// of the function: a node that the file places left of it moves the whole drawing (spec 11). Down, by the shift that
/// the most nodes share, so that the most nodes stay still on the screen: a placement that grows upward moves everything
/// below its new top, and a node that the file places above the frame does too. Only the nodes found again by their key
/// count, among them those that kept their place across and whose `layout` entry did not change: a node that a move,
/// an undo or an agent put elsewhere moved, whatever the others did. On a tie, the shift of the frame, then the smallest.
pub fn shift_between(old: &Drawing, new: &Drawing) -> (i64, i64) {
    let frame = match (old.bodies.first(), new.bodies.first()) {
        (Some(old), Some(new)) => (new.origin.0 - old.origin.0, new.origin.1 - old.origin.1),
        _ => (0, 0),
    };
    let was: HashMap<&str, &crate::Placed> =
        old.placed.iter().filter(|placed| !placed.key.is_empty()).map(|placed| (placed.key.as_str(), placed)).collect();
    let mut counts: HashMap<i64, usize> = HashMap::new();
    for placed in &new.placed {
        if let Some(was) = was.get(placed.key.as_str())
            && was.at == placed.at
            && placed.x - was.x == frame.0
        {
            *counts.entry(placed.y - was.y).or_default() += 1;
        }
    }
    let down =
        counts.into_iter().max_by_key(|&(dy, count)| (count, dy == frame.1, std::cmp::Reverse(dy.abs()), dy > 0)).map_or(frame.1, |(dy, _)| dy);
    (frame.0, down)
}
