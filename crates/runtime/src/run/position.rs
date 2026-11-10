//! The position of a node in the global order of spec 8: the canonical positions of what contains it, from the top-level
//! function down through calls, loops and arms, each loop adding its iteration, compared element by element. A position is a
//! link to the position of what contains it plus one index (`docs/architecture.md`, 10.4), so that making one costs the same at
//! any depth; comparing two walks up to where they meet, which is near for nodes ready together.

use std::cmp::Ordering;
use std::rc::Rc;

#[derive(Debug)]
pub(crate) struct Step {
    parent: Option<Pos>,
    index: u64,
    depth: u32,
}

pub(crate) type Pos = Rc<Step>;

/// A chain as deep as calls and loops nest is freed link by link, not by a recursion of drops, which would overflow the stack
/// of a run 4,096 calls deep in loops.
impl Drop for Step {
    fn drop(&mut self) {
        let mut next = self.parent.take();
        while let Some(link) = next {
            match Rc::try_unwrap(link) {
                Ok(mut step) => next = step.parent.take(),
                Err(_) => break,
            }
        }
    }
}

/// The position of the top-level function: the empty list.
pub(crate) fn root() -> Pos {
    Rc::new(Step { parent: None, index: 0, depth: 0 })
}

/// The position one level down, at this index.
pub(crate) fn child(parent: &Pos, index: u64) -> Pos {
    Rc::new(Step { parent: Some(Rc::clone(parent)), index, depth: parent.depth.saturating_add(1) })
}

/// The list of indices, outermost first, for tests and reports.
#[cfg(test)]
pub(crate) fn indices(pos: &Pos) -> Vec<u64> {
    let mut out = Vec::new();
    let mut at = Some(pos);
    while let Some(step) = at {
        if step.depth > 0 {
            out.push(step.index);
        }
        at = step.parent.as_ref();
    }
    out.reverse();
    out
}

fn up(pos: &Pos) -> Option<&Pos> {
    pos.parent.as_ref()
}

/// Element by element; a list that is the start of the other comes first. Positions that are made from one another share
/// their steps, so two steps with the same parent and the same index are the same list.
pub(crate) fn compare(a: &Pos, b: &Pos) -> Ordering {
    if Rc::ptr_eq(a, b) {
        return Ordering::Equal;
    }
    let (mut x, mut y) = (a, b);
    let longer = a.depth.cmp(&b.depth);
    while x.depth > y.depth {
        let Some(parent) = up(x) else { break };
        x = parent;
    }
    while y.depth > x.depth {
        let Some(parent) = up(y) else { break };
        y = parent;
    }
    if Rc::ptr_eq(x, y) {
        return longer;
    }
    // Walking up, the last difference met is the outermost, which decides.
    let mut decided = Ordering::Equal;
    loop {
        let here = x.index.cmp(&y.index);
        if here != Ordering::Equal {
            decided = here;
        }
        match (up(x), up(y)) {
            (Some(p), Some(q)) if Rc::ptr_eq(p, q) => return decided.then(longer),
            (Some(p), Some(q)) => {
                x = p;
                y = q;
            }
            _ => return decided.then(longer),
        }
    }
}
