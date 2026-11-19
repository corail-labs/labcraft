//! The terminals of a compact block that the `terminals` section of its function places (spec 11.2, visual notation
//! 3.2): worked out by `labcraft_check::places`, which `check` judges the lines with, so that the two never disagree on
//! where a terminal goes. A block without a section is drawn as notation 3.2 says, by `size`.

pub use labcraft_check::places::{Block, PITCH, Places};

use crate::graph::Node;

/// The places of the terminals of `node`, whose function has the section `block`.
pub fn places(block: &Block, node: &Node<'_>) -> Places {
    labcraft_check::places::places(block, &node.inputs, &node.outputs)
}
