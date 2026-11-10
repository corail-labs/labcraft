//! The memory a run holds at once, by the logical size of cli.md 6.2: what its wires hold, its loops gather and its panels
//! keep, and 256 bytes for each region that runs and each of its nodes. **A value counts once while some wire holds it,
//! however many share it** (D-11): what a value holds behind an `Arc` is counted by that `Arc`, once, and given back when the
//! last place that holds it lets it go. A value that a body takes, the last read of its wire, is no longer counted while the
//! body works: the body asks for the room of what it builds (`Host::room`), and its outputs are counted again.

use std::collections::HashMap;
use std::sync::Arc;

use crate::value::{Value, logical_size};

/// What each node and each region that runs counts *(proposed, cli.md 6.2)*.
pub const NODE_COST: u64 = 256;

#[derive(Debug, Default)]
pub(crate) struct Memory {
    held: u64,
    limit: u64,
    /// By the address of an `Arc` held: how many places hold it, and what it was counted.
    shared: HashMap<usize, (u32, u64)>,
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

impl Memory {
    pub(crate) fn new(limit: u64) -> Self {
        Memory { held: 0, limit, shared: HashMap::new() }
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
        true
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
