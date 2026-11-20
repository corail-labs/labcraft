//! Alt and a drag on a terminal of a block in compact view (notation 2.4): the terminal goes onto another place of its
//! block, or onto another side, and the drop writes the line of the `terminals` section of the function called
//! (`block.set_terminal`, spec 11.2), so every call to it changes. The free places light up while it moves.

use egui::Pos2;
use labcraft_diagram::Drawing;

/// The places are 8 units apart on every block (spec 11.2), as the drawing places them.
const PITCH: f32 = labcraft_diagram::PLACE_PITCH as f32 / 100.0;

/// A side of a block that holds terminals.
pub use labcraft_diagram::BlockSide as Side;

/// A terminal of a block being moved.
#[derive(Clone, Debug, PartialEq)]
pub struct BlockDrag {
    pub port: usize,
    pub placed: usize,
    /// As `block.set_terminal` names it: `input x` or `output x`.
    pub terminal: String,
    /// The file of the function called, which the drop writes.
    pub file: String,
    /// Where it is now, and where the pointer would put it.
    pub from: (Side, u32),
    pub to: (Side, u32),
    /// The pointer, in units.
    pub at: Pos2,
    /// What the drag says, that every call to the function changes, made once.
    pub note: String,
}

/// The box of a block, in units: its left, top, width and height.
fn area(drawing: &Drawing, placed: usize) -> Option<(f32, f32, f32, f32)> {
    let found = drawing.placed.get(placed)?;
    Some((found.x as f32 / 100.0, found.y as f32 / 100.0, found.width as f32 / 100.0, found.height as f32 / 100.0))
}

/// The places of the top side and of the left and right sides of a block of that size.
fn counts(width: f32, height: f32) -> (u32, u32) {
    let count = |length: f32| ((length / PITCH).round() as i64 - 1).clamp(0, 1_024) as u32;
    (count(width), count(height))
}

/// The side and the place of a point of a block, the nearest that a terminal can take.
fn nearest(drawing: &Drawing, placed: usize, at: Pos2) -> Option<(Side, u32)> {
    let (x, y, width, height) = area(drawing, placed)?;
    let (top, left) = counts(width, height);
    // Only the sides that have a place a terminal can take: none on the left and right of `left 0`, none on the top of
    // `top 1`, whose one place is `after:`'s (spec 11.2).
    let distances = [(Side::Left, (at.x - x).abs()), (Side::Right, (at.x - x - width).abs()), (Side::Top, (at.y - y).abs())];
    let open = |side: Side| if side == Side::Top { top >= 2 } else { left >= 1 };
    let side = distances.iter().filter(|found| open(found.0)).min_by(|a, b| a.1.total_cmp(&b.1)).map(|found| found.0)?;
    let along = |from: f32, length: u32, least: u32| {
        let place = ((from / PITCH).round() as i64).clamp(i64::from(least), i64::from(length.max(least)));
        place as u32
    };
    Some(match side {
        // The first place of the top side is `after:`'s.
        Side::Top => (Side::Top, along(at.x - x, top, 2)),
        Side::Left | Side::Right => (side, along(at.y - y, left, 1)),
    })
}

/// Where a place of a block is, in units.
pub fn place_at(drawing: &Drawing, placed: usize, (side, place): (Side, u32)) -> Option<Pos2> {
    let (x, y, width, _) = area(drawing, placed)?;
    let along = place as f32 * PITCH;
    Some(match side {
        Side::Top => Pos2::new(x + along, y),
        Side::Left => Pos2::new(x, y + along),
        Side::Right => Pos2::new(x + width, y + along),
    })
}

impl BlockDrag {
    /// From the terminal `port`: what it moves, or why it cannot (a function that the project does not hold, a node that
    /// is no block).
    pub fn new(drawing: &Drawing, port: usize, at: Pos2) -> Result<BlockDrag, String> {
        let found = drawing.ports.get(port).ok_or_else(String::new)?;
        let placed = found.placed.filter(|_| !found.frame && !found.order && !found.caption.is_empty()).ok_or_else(String::new)?;
        let node = drawing.placed.get(placed).ok_or_else(String::new)?;
        if node.callee.is_empty() {
            return Err("only the block in compact view of a function of the project moves its terminals: a function shipped with LabCraft or found with --lib-path keeps its own".to_owned());
        }
        let terminal = format!("{} {}", if found.input { "input" } else { "output" }, found.caption);
        let from = nearest(drawing, placed, Pos2::new(found.x as f32 / 100.0, found.y as f32 / 100.0)).ok_or_else(String::new)?;
        let file = node.callee.clone();
        let name = file.rsplit(['/', '\\']).next().unwrap_or(&file);
        let note = format!("Every call to {} changes", name.strip_suffix(".lcf").unwrap_or(name));
        Ok(BlockDrag { port, placed, terminal, file, from, to: from, at, note })
    }

    pub fn set_at(&mut self, drawing: &Drawing, at: Pos2) {
        self.at = at;
        if let Some(to) = nearest(drawing, self.placed, at) {
            self.to = to;
        }
    }

    pub fn changed(&self) -> bool {
        self.to != self.from
    }

    /// The places of the block where the pointer can drop it: every place but its own, the first of the top side and those
    /// that the lines of the section hold. A place that another terminal holds by default is free (commands.md 9.4: that
    /// terminal takes the first free place).
    pub fn free(&self, drawing: &Drawing) -> Vec<(Side, u32)> {
        let Some((_, _, width, height)) = area(drawing, self.placed) else { return Vec::new() };
        let (top, left) = counts(width, height);
        let sides = (2..=top).map(|place| (Side::Top, place)).chain((1..=left).flat_map(|place| [(Side::Left, place), (Side::Right, place)]));
        let held = drawing.placed.get(self.placed).map_or(&[][..], |node| node.held.as_slice());
        sides.filter(|place| *place != self.from && !held.iter().any(|(side, at)| *at == place.1 && side == place.0.word())).collect()
    }
}
