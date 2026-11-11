//! The memory a run holds at once, by the logical size of cli.md 6.2: what its wires hold, its loops gather and its panels
//! keep, and 256 bytes for each region that runs and each of its nodes. **A value counts once while some wire holds it,
//! however many share it** (D-11): what a value holds behind an `Arc` is counted by that `Arc`, once, and given back when the
//! last place that holds it lets it go. A value that a body takes, the last read of its wire, is no longer counted while the
//! body works: the body asks for the room of what it builds (`Host::room`), and its outputs are counted again.
//!
//! **What the nodes make is counted too, in all** (`--max-made`, cli.md 6.2): an output of a node that no place held, that
//! nothing else holds, and that is not one that a read of the node just took, which its body changed in place or passed
//! through. A copy, a new array or a new text counts its size; `array.set` on an array that nothing else holds counts nothing,
//! and neither does a part read out of a value (a field, `array.get`), which the value still holds. A new value counts its own
//! level and its parts that nothing else holds, the same way down; a part that another value holds counts only its place, 16
//! bytes, so that a struct built again to change one number does not count the array it shares (decision of the owner,
//! 2026-11-11). Values without an `Arc` are not counted there: their cost is that of the nodes that make them, which
//! `--max-nodes` bounds.

use std::collections::HashMap;
use std::sync::Arc;

use crate::value::{VALUE_COST, Value, logical_size};

/// What each node and each region that runs counts *(proposed, cli.md 6.2)*.
pub const NODE_COST: u64 = 256;

#[derive(Debug, Default)]
pub(crate) struct Memory {
    held: u64,
    limit: u64,
    /// By the address of an `Arc` held: how many places hold it, and what it was counted.
    shared: HashMap<usize, (u32, u64)>,
    /// The bytes of the values made, in all, and the limit of the run.
    made: u64,
    made_limit: Option<u64>,
    /// The addresses that the reads of the node that runs took and let go: held again, they were not made.
    taken: Vec<usize>,
}

fn address(value: &Value) -> Option<usize> {
    Some(match value {
        Value::String(text) => Arc::as_ptr(text) as *const u8 as usize,
        Value::Bytes(bytes) => Arc::as_ptr(bytes) as *const u8 as usize,
        Value::Error(Some(error)) => Arc::as_ptr(error) as usize,
        Value::Array(items) => Arc::as_ptr(items) as usize,
        Value::Grid(grid) => Arc::as_ptr(grid) as usize,
        Value::Struct(fields) => Arc::as_ptr(fields) as *const Value as usize,
        Value::Variant(variant) => Arc::as_ptr(variant) as usize,
        _ => return None,
    })
}

/// Whether nothing but this value holds what is behind its `Arc`: a value just made. A part read out of another value, which
/// that value still holds, is not.
fn alone(value: &Value) -> bool {
    match value {
        Value::String(text) => Arc::strong_count(text) == 1,
        Value::Bytes(bytes) => Arc::strong_count(bytes) == 1,
        Value::Error(Some(error)) => Arc::strong_count(error) == 1,
        Value::Array(items) => Arc::strong_count(items) == 1,
        Value::Grid(grid) => Arc::strong_count(grid) == 1,
        Value::Struct(fields) => Arc::strong_count(fields) == 1,
        Value::Variant(variant) => Arc::strong_count(variant) == 1,
        _ => false,
    }
}

/// What a value just made counts in the bytes made: its logical size, but for each part that another value holds too, which
/// counts only its place. Only the parts made are walked, so this costs no more than making them; an array of numbers, whose
/// items have no parts, is not walked.
fn made_size(value: &Value) -> u64 {
    let part = |part: &Value| if address(part).is_some() && !alone(part) { VALUE_COST } else { made_size(part) };
    let parts = |parts: &[Value]| parts.iter().fold(0_u64, |total, value| total.saturating_add(part(value)));
    match value {
        Value::Array(items) if items.items().first().is_some_and(|item| address(item).is_some()) => VALUE_COST.saturating_add(parts(items.items())),
        Value::Struct(fields) => VALUE_COST.saturating_add(parts(fields)),
        Value::Variant(variant) => VALUE_COST.saturating_add(4).saturating_add(parts(&variant.values)),
        _ => logical_size(value),
    }
}

impl Memory {
    pub(crate) fn new(limit: u64, made_limit: Option<u64>) -> Self {
        Memory { held: 0, limit, shared: HashMap::new(), made: 0, made_limit, taken: Vec::new() }
    }

    /// The bytes made so far, and the limit when they pass it.
    pub(crate) fn made_over(&self) -> Option<(u64, u64)> {
        self.made_limit.filter(|limit| self.made > *limit).map(|limit| (self.made, limit))
    }

    /// A node starts: what the reads of the node before took is no longer its.
    pub(crate) fn forget_taken(&mut self) {
        self.taken.clear();
    }

    pub(crate) fn held(&self) -> u64 {
        self.held
    }

    pub(crate) fn room(&self) -> u64 {
        self.limit.saturating_sub(self.held)
    }

    /// Counts `bytes`, unless that would pass the limit.
    pub(crate) fn charge(&mut self, bytes: u64) -> bool {
        match self.held.checked_add(bytes) {
            Some(after) if after <= self.limit => {
                self.held = after;
                true
            }
            _ => false,
        }
    }

    pub(crate) fn refund(&mut self, bytes: u64) {
        self.held = self.held.saturating_sub(bytes);
    }

    /// One more place holds this value; `false`, and nothing counted, when the run cannot hold it.
    pub(crate) fn hold(&mut self, value: &Value) -> bool {
        self.hold_counting(value, false)
    }

    /// [`Memory::hold`] for an output of a node, which counts what the node made.
    pub(crate) fn hold_output(&mut self, value: &Value) -> bool {
        self.hold_counting(value, true)
    }

    fn hold_counting(&mut self, value: &Value, output: bool) -> bool {
        let Some(address) = address(value) else { return self.charge(logical_size(value)) };
        if let Some((places, _)) = self.shared.get_mut(&address) {
            *places = places.saturating_add(1);
            return true;
        }
        let size = logical_size(value);
        if !self.charge(size) {
            return false;
        }
        self.shared.insert(address, (1, size));
        let taken = self.taken.iter().position(|taken| *taken == address);
        if let Some(at) = taken {
            self.taken.swap_remove(at);
        } else if output && alone(value) {
            self.made = self.made.saturating_add(made_size(value));
        }
        true
    }

    /// A read takes the value from the last place that held it: held again at the same address, it was changed in place or
    /// passed through, not made.
    pub(crate) fn release_taken(&mut self, value: &Value) {
        if let Some(address) = address(value)
            && self.shared.get(&address).is_some_and(|(places, _)| *places == 1)
        {
            // Only those of one node are wanted: a region of no node, which starts none, does not make them grow.
            if self.taken.len() >= 1024 {
                self.taken.clear();
            }
            self.taken.push(address);
        }
        self.release(value);
    }

    /// A place lets the value go.
    pub(crate) fn release(&mut self, value: &Value) {
        let Some(address) = address(value) else {
            self.refund(logical_size(value));
            return;
        };
        let Some((places, size)) = self.shared.get_mut(&address) else { return };
        *places = places.saturating_sub(1);
        if *places == 0 {
            let size = *size;
            self.shared.remove(&address);
            self.refund(size);
        }
    }
}
