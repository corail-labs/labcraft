//! The block of the function drawn, as its callers see it, for the view `Block` (ide.md 3): drawn alone, with what its
//! sections write and what the mask of its library gives (spec 11.2 to 11.4), so that the view says where each value
//! comes from and sends the commands of the block (commands.md 9.4). One more field of the drawing, no command of its
//! own (decision of the owner, 2026-11-19).

use labcraft_check::{Checked, CheckedFile};
use labcraft_format::block::{Colours, least_sizes};
use labcraft_graph::{BandPlace, BlockSide, Decl, Outline, OutlineKind, Rgb, TerminalsKind};
use labcraft_types::{Interner, Name};

use crate::block::Block;
use crate::graph::{Builder, Kind};
use crate::shape::Drawing;
use crate::{DiagramLimits, PLACE_PITCH, ViewState, extent, shift};

/// The block of a function, alone.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct OwnBlock {
    /// One node, the block, a margin of one place around what it draws: [`Drawing::placed`] holds it alone.
    pub drawing: Drawing,
    /// Its library as a band names it; empty for the root library, whose functions have no band.
    pub library: String,
    /// Why the commands of the block cannot change it, as the view says it; empty when they can.
    pub fixed: String,
    /// The manifest of its library as reports show it, where `check` says what its mask breaks (spec 11.3).
    pub manifest: String,
    /// The places of its top side and of its left side, as it is drawn (spec 11.2).
    pub top: u32,
    pub left: u32,
    /// The smallest `top` and `left` that keep every line of its section applying: where a drag of an edge stops.
    pub least: (u32, u32),
    /// What the sections of its file write.
    pub written: BlockFields,
    /// What the `[mask]` of its library gives (spec 11.3); never a size.
    pub mask: BlockFields,
}

/// The fields of a block that a section or a mask writes, each `None` when it does not.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BlockFields {
    pub top: Option<u32>,
    pub left: Option<u32>,
    /// `hidden`, `inside` or `above`.
    pub band: Option<String>,
    /// `#RRGGBB`.
    pub color: Option<String>,
    pub fill: Option<String>,
    pub stroke: Option<String>,
    pub outline: Option<OutlineChoice>,
}

/// What an outline is, for the list of the view (spec 11.4).
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum OutlineChoice {
    /// `template d` or `template block`, by its name as written.
    Template(String),
    /// A body and its pieces, written by hand: the view `Text` changes it.
    Path,
}

/// `#RRGGBB`, in capitals, as notation 8 writes a colour.
fn hex(rgb: Rgb) -> String {
    crate::style::written(rgb.value())
}

/// A colour `#RRGGBB` as a file writes it (spec 11.2), read as `check` reads it: its red, green and blue.
pub fn written_rgb(text: &str) -> Option<[u8; 3]> {
    Rgb::parse(text).map(|rgb| rgb.0)
}

fn outline_choice(outline: &Outline, names: &Interner) -> OutlineChoice {
    match outline.items.as_slice() {
        [item] => match item.kind {
            OutlineKind::Template(name) => OutlineChoice::Template(labcraft_types::cut_text(names.resolve(name)).into_owned()),
            _ => OutlineChoice::Path,
        },
        _ => OutlineChoice::Path,
    }
}

/// The block of the function of `file`, alone; none for a type, a function that `check` does not declare, or one with more
/// terminals than [`crate::draw`] draws (`LC7002`).
pub(crate) fn of(checked: &Checked, file: &CheckedFile, names: &Interner, limits: &DiagramLimits) -> Option<OwnBlock> {
    let (name, id, external) = match &file.file.decl {
        Decl::Function(function) => (function.name, function.id, false),
        Decl::Extern(external) => (external.name, external.id, true),
        Decl::Type(_) => return None,
    };
    let library = checked.libraries.get(file.library as usize)?;
    let declared = library.functions.get(&name)?;
    let terminals = (declared.params.len() + declared.outputs.len()) as u64;
    if terminals > limits.max_nodes.saturating_mul(5) {
        return None;
    }
    let graph = Builder::new(checked, file, names).own_block(name, id);
    let mut drawing = crate::drawing(&graph, names, &ViewState::default(), Some(&checked.types));
    // The frame around it is not the block's.
    drawing.loose.clear();
    drawing.bodies.clear();
    drawing.warnings.clear();
    // Nor values: a call that no run reaches.
    drawing.trace = crate::values::TracePlan::default();
    let (mut left, mut top, mut right, mut bottom) = (i64::MAX, i64::MAX, i64::MIN, i64::MIN);
    for shape in drawing.groups.iter().flat_map(|group| group.shapes.iter()) {
        let (x0, y0, x1, y1) = extent(shape);
        (left, top, right, bottom) = (left.min(x0), top.min(y0), right.max(x1), bottom.max(y1));
    }
    if left > right {
        return None;
    }
    shift(&mut drawing, PLACE_PITCH - left, PLACE_PITCH - top);
    drawing.width = right - left + 2 * PLACE_PITCH;
    drawing.height = bottom - top + 2 * PLACE_PITCH;
    let placed = drawing.placed.first()?;
    let places = |length: i64| u32::try_from(length / PLACE_PITCH - 1).unwrap_or(0);
    let (top_places, left_places) = (places(placed.width), places(placed.height));
    let inputs: Vec<Name> = declared.params.iter().map(|param| param.name).collect();
    let outputs: Vec<Name> = declared.outputs.iter().map(|output| output.name).collect();
    let section = file.file.terminals.as_ref();
    let read = section.map(|section| labcraft_format::block::read(section, &inputs, &outputs, names).0).unwrap_or_default();
    let sized = |side: BlockSide| {
        section
            .is_some_and(|section| section.items.iter().any(|item| matches!(item.kind, TerminalsKind::Size { side: written, .. } if written == side)))
    };
    let written = BlockFields {
        top: sized(BlockSide::Top).then_some(read.top_places),
        left: if sized(BlockSide::Left) { read.left_places } else { None },
        ..fields(read.band, read.colours, file.file.outline.as_ref(), names)
    };
    let mask = &library.mask;
    let mask = fields(mask.band_place, mask.colours, mask.outline.as_ref().map(|outline| &outline.outline), names);
    let (top_least, left_least) = section.map_or((1, 0), |section| least_sizes(section, &inputs, &outputs, names));
    let least = (top_least, left_least.max(needed_left(&graph, top_places)));
    let fixed = if !library.checked {
        "A function shipped with LabCraft or found with --lib-path keeps its own block".to_owned()
    } else if external {
        "The commands of the block do not change an `extern` function yet: its sections are written in its file".to_owned()
    } else {
        String::new()
    };
    let band = graph.nodes.first().and_then(|node| match &node.kind {
        Kind::Call { library, .. } => library.clone(),
        _ => None,
    });
    let folder = file.shown.rsplit_once('/').map_or("", |(folder, _)| folder);
    let manifest = labcraft_check::manifest::shown(folder);
    Some(OwnBlock { drawing, library: band.unwrap_or_default(), fixed, manifest, top: top_places, left: left_places, least, written, mask })
}

/// The places that the left and right sides need for the terminals that no line places: a block grows down to hold
/// them (spec 11.2), so a smaller `left` would draw the same block.
fn needed_left(graph: &crate::graph::Graph<'_>, top: u32) -> u32 {
    let Some(node) = graph.nodes.first() else { return 0 };
    let block = match &node.kind {
        Kind::Call { block: Some(block), .. } => Block { left: Some(0), ..(**block).clone() },
        _ => {
            let optional = node.inputs.iter().filter(|input| input.default.is_some()).map(|input| input.name.clone()).collect();
            Block {
                left: Some(0),
                ..Block::of(&labcraft_format::block::Section { top_places: top, ..Default::default() }, &Interner::new(), optional)
            }
        }
    };
    crate::block::places(&block, node).left
}

/// The band, the colours and the outline that a section and an outline section, or a mask, write.
fn fields(band: Option<BandPlace>, colours: Colours, outline: Option<&Outline>, names: &Interner) -> BlockFields {
    BlockFields {
        band: band.map(|place| place.word().to_owned()),
        color: colours.color.map(hex),
        fill: colours.fill.map(hex),
        stroke: colours.stroke.map(hex),
        outline: outline.map(|outline| outline_choice(outline, names)),
        ..BlockFields::default()
    }
}
