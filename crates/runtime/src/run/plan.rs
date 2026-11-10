//! What the interpreter needs of a region, found once per region of the program: where each value sits among the slots of an
//! instance, how many reads each has, and which nodes wait for which. **Readiness is counted, not searched** (10.4): a node
//! knows how many of the nodes before it it still waits for, and each node that ends counts down those after it.

use labcraft_ir::{Order, Region, Source, Use};

use crate::fault::Fault;

#[derive(Debug)]
pub(crate) struct Plan {
    /// The arguments come first, then the outputs of each node from its offset.
    offsets: Vec<usize>,
    args: usize,
    pub(crate) slots: usize,
    /// By slot: how many times the inputs of the nodes and the results of the region read it.
    pub(crate) reads: Vec<u32>,
    /// By node: the nodes it waits for, each once.
    pub(crate) missing: Vec<u32>,
    /// By node: the nodes that wait for it.
    pub(crate) dependents: Vec<Vec<u32>>,
    /// The nodes that wait for none, in order.
    pub(crate) starts: Vec<u32>,
}

impl Plan {
    pub(crate) fn new(region: &Region) -> Result<Plan, Fault> {
        let args = region.args.len();
        let mut offsets = Vec::with_capacity(region.nodes.len());
        let mut slots = args;
        for node in &region.nodes {
            offsets.push(slots);
            slots = slots.saturating_add(node.outputs.len());
        }
        let mut plan = Plan {
            offsets,
            args,
            slots,
            reads: vec![0; slots],
            missing: vec![0; region.nodes.len()],
            dependents: vec![Vec::new(); region.nodes.len()],
            starts: Vec::new(),
        };
        let mut before = Vec::new();
        for (index, node) in region.nodes.iter().enumerate() {
            before.clear();
            for used in &node.inputs {
                plan.read(used, &mut before)?;
            }
            for order in &node.after {
                match order {
                    Order::Value(used) => plan.waits(used, &mut before),
                    Order::Node(other) => before.push(*other),
                }
            }
            before.sort_unstable();
            before.dedup();
            let index = u32::try_from(index).map_err(|_| Fault::bug("a region of more than 2^32 nodes"))?;
            for other in &before {
                if *other == index {
                    return Err(Fault::bug("a node that waits for itself"));
                }
                plan.dependents.get_mut(*other as usize).ok_or_else(|| Fault::bug("a node waits for a node that its region has not"))?.push(index);
            }
            if let Some(count) = plan.missing.get_mut(index as usize) {
                *count = u32::try_from(before.len()).unwrap_or(u32::MAX);
            }
            if before.is_empty() {
                plan.starts.push(index);
            }
        }
        let mut ignored = Vec::new();
        for used in &region.results {
            plan.read(used, &mut ignored)?;
        }
        Ok(plan)
    }

    /// The slot of a value of the region: an argument, or an output of a node.
    pub(crate) fn slot(&self, source: &Source) -> Option<usize> {
        match source {
            Source::Arg(index) => ((*index as usize) < self.args).then_some(*index as usize),
            Source::Out { node, output } => {
                let start = *self.offsets.get(*node as usize)?;
                let end = self.offsets.get(*node as usize + 1).copied().unwrap_or(self.slots);
                let slot = start.checked_add(*output as usize)?;
                (slot < end).then_some(slot)
            }
            Source::Const(_) | Source::Array(_) => None,
        }
    }

    /// The first slot of the outputs of a node.
    pub(crate) fn outputs(&self, node: u32) -> Option<usize> {
        self.offsets.get(node as usize).copied()
    }

    /// Counts a read of `used`, through an array written of wires, with the explicit stack of what it holds.
    fn read(&mut self, used: &Use, before: &mut Vec<u32>) -> Result<(), Fault> {
        let mut pending = vec![used];
        while let Some(used) = pending.pop() {
            match &used.source {
                Source::Array(items) => pending.extend(items.iter()),
                Source::Const(_) => {}
                source => {
                    let slot = self.slot(source).ok_or_else(|| Fault::bug("a use of a value that its region has not"))?;
                    if let Some(reads) = self.reads.get_mut(slot) {
                        *reads = reads.saturating_add(1);
                    }
                    if let Source::Out { node, .. } = source {
                        before.push(*node);
                    }
                }
            }
        }
        Ok(())
    }

    /// The nodes a use waits for without reading it.
    fn waits(&self, used: &Use, before: &mut Vec<u32>) {
        let mut pending = vec![used];
        while let Some(used) = pending.pop() {
            match &used.source {
                Source::Array(items) => pending.extend(items.iter()),
                Source::Out { node, .. } => before.push(*node),
                Source::Arg(_) | Source::Const(_) => {}
            }
        }
    }
}
