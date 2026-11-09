//! What a wire carries besides its line (notation 3.3, 4, 4.2): the pill just before its reader, and its name, written
//! where it covers nothing.
//!
//! A name is placed only where it is free of every node, pill, wire and other name of its body. The test goes through
//! the edges of what is there, kept merged by line, so that each costs a few searches whatever the size of the body.

use std::collections::BTreeMap;

/// The edges of what a body draws, merged by line.
#[derive(Default)]
pub struct Obstacles {
    /// By height: the intervals across, keyed by their start.
    horizontal: BTreeMap<i64, BTreeMap<i64, i64>>,
    /// By abscissa: the intervals down.
    vertical: BTreeMap<i64, BTreeMap<i64, i64>>,
}

impl Obstacles {
    /// A segment of a wire, across or down.
    pub fn segment(&mut self, a: (i64, i64), b: (i64, i64)) {
        if a.1 == b.1 {
            add(self.horizontal.entry(a.1).or_default(), a.0.min(b.0), a.0.max(b.0));
        } else if a.0 == b.0 {
            add(self.vertical.entry(a.0).or_default(), a.1.min(b.1), a.1.max(b.1));
        }
    }

    /// A box: its four edges.
    pub fn rect(&mut self, x0: i64, y0: i64, x1: i64, y1: i64) {
        self.segment((x0, y0), (x1, y0));
        self.segment((x0, y1), (x1, y1));
        self.segment((x0, y0), (x0, y1));
        self.segment((x1, y0), (x1, y1));
    }

    /// Whether nothing crosses or touches the box: no edge in it. A box is never wholly inside a node, since it sits
    /// above a wire that no node covers.
    pub fn free(&self, x0: i64, y0: i64, x1: i64, y1: i64) -> bool {
        !self.horizontal.range(y0..=y1).any(|(_, list)| overlaps(list, x0, x1))
            && !self.vertical.range(x0..=x1).any(|(_, list)| overlaps(list, y0, y1))
    }
}

/// Adds `[start, end]` to disjoint intervals, merging those it meets.
pub fn add(list: &mut BTreeMap<i64, i64>, start: i64, end: i64) {
    let (mut start, mut end) = (start, end);
    // The intervals it meets are the last ones that start at or before its end.
    while let Some((&other_start, &other_end)) = list.range(..=end).next_back() {
        if other_end < start {
            break;
        }
        list.remove(&other_start);
        start = start.min(other_start);
        end = end.max(other_end);
    }
    list.insert(start, end);
}

/// Whether an interval of the list meets `[start, end]`.
fn overlaps(list: &BTreeMap<i64, i64>, start: i64, end: i64) -> bool {
    list.range(..=end).next_back().is_some_and(|(_, &other_end)| other_end >= start)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_box_is_free_of_what_does_not_touch_it() {
        let mut obstacles = Obstacles::default();
        obstacles.segment((0, 100), (50, 100));
        obstacles.segment((60, 100), (90, 100));
        obstacles.segment((55, 0), (55, 40));
        assert!(obstacles.free(0, 50, 50, 90));
        assert!(!obstacles.free(0, 50, 50, 100), "a wire on its lower edge");
        assert!(!obstacles.free(52, 30, 58, 50), "a wire through it, down");
        assert!(obstacles.free(51, 60, 59, 90), "between two wires across");
        obstacles.segment((40, 100), (70, 100));
        assert!(!obstacles.free(51, 60, 59, 100), "merged");
        obstacles.rect(200, 200, 300, 300);
        assert!(!obstacles.free(190, 250, 210, 260));
    }
}
