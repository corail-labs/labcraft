//! A block at every size that it takes: the size of its section, with one value for each variadic parameter, and the size
//! of each call of the project, whose variadic parameters take a place per value. There, the lines of a `terminals`
//! section whose left and right sides grow (spec 11.2: `middle` and `last` are counted on the grown side, so whether two
//! lines end on one place is known only at the size the block takes, `LC5014`), and the guards of an `outline` (11.4,
//! `LC5017`). A warning goes on the item, in the file of the function, once for that item whatever the number of calls.
//! Done once every body is resolved, since a call can be in any file of the run.

use std::collections::{BTreeSet, HashMap, HashSet};

use labcraft_format::block::Section;
use labcraft_graph::{BlockSide, Decl, Id};
use labcraft_types::{Code, Name, cut_text};

use super::super::Pass;
use crate::declare::{DeclaredFunction, DeclaredLibrary};
use crate::load::at;
use crate::outline::{CHAMFER, Frame, Problem, Read, draw, read};
use crate::places::{Block, places, section_of, signed};
use crate::resolved::{Meaning, Resolved, Terminal};

/// What a block of its own is judged on: its outline when it draws one, its section, whether its lines are judged at each
/// size (a `terminals` section without `left`), and its variadic parameters, by index.
struct Judged {
    outline: Option<Read>,
    section: Section,
    lines_grow: bool,
    variadic: Vec<u32>,
}

impl<'a> Pass<'a> {
    /// Judges the block of each function of the libraries checked at every size that it takes.
    pub(super) fn judge_blocks(&mut self, declared: &[DeclaredLibrary], all: &[Vec<Resolved>]) {
        let loaded = self.loaded;
        let mut judged: HashMap<(u32, Name), Judged> = HashMap::new();
        for (library, source) in loaded.iter().enumerate().filter(|(_, source)| source.checked) {
            let Ok(index) = u32::try_from(library) else { continue };
            for file in &source.files {
                let Some(model) = &file.model else { continue };
                let Decl::Function(function) = &model.file.decl else { continue };
                if model.file.outline.is_none() && model.file.terminals.is_none() {
                    continue;
                }
                let Some(found) = declared.get(library).and_then(|found| found.functions.get(&function.name)) else { continue };
                let outline = model.file.outline.as_ref().map(|outline| read(outline, self.names).0).filter(|outline| *outline != Read::Block);
                let inputs: Vec<Name> = function.signature.params.iter().map(|param| param.name).collect();
                let outputs: Vec<Name> = function.signature.outputs.iter().map(|output| output.name).collect();
                let section = section_of(model.file.terminals.as_ref(), &inputs, &outputs, self.names);
                let lines_grow = model.file.terminals.is_some() && section.left_places.is_none();
                if outline.is_none() && !lines_grow {
                    continue;
                }
                let variadic: Vec<u32> =
                    found.params.iter().enumerate().filter(|(_, param)| param.variadic).filter_map(|(at, _)| u32::try_from(at).ok()).collect();
                judged.insert((index, function.name), Judged { outline, section, lines_grow, variadic });
            }
        }
        // The values that each call gives to their variadic parameters: one walk of the arguments of each call to one of them.
        let mut sizes: HashMap<(u32, Name), BTreeSet<Vec<u32>>> = HashMap::new();
        if judged.values().any(|judged| !judged.variadic.is_empty()) {
            for resolved in all.iter().flatten() {
                for call in resolved.calls() {
                    let Meaning::Function { library, name } = call.callee else { continue };
                    let Some(variadic) = judged.get(&(library, name)).map(|judged| &judged.variadic).filter(|variadic| !variadic.is_empty()) else {
                        continue;
                    };
                    let mut counts = vec![0_u32; variadic.len()];
                    for terminal in call.terminals.iter() {
                        if let Terminal::Param(index) = terminal
                            && let Ok(at) = variadic.binary_search(index)
                            && let Some(count) = counts.get_mut(at)
                        {
                            *count = count.saturating_add(1);
                        }
                    }
                    sizes.entry((library, name)).or_default().insert(counts);
                }
            }
        }
        for (library, source) in loaded.iter().enumerate().filter(|(_, source)| source.checked) {
            let Ok(index) = u32::try_from(library) else { continue };
            for file in &source.files {
                let Some(model) = &file.model else { continue };
                let Decl::Function(function) = &model.file.decl else { continue };
                let key = (index, function.name);
                let Some(block) = judged.get(&key) else { continue };
                let Some(found) = declared.get(library).and_then(|found| found.functions.get(&function.name)) else { continue };
                // The size of the section first, one value for each variadic parameter, then those of the calls.
                let mut counts: Vec<Vec<u32>> = vec![vec![1; block.variadic.len()]];
                if let Some(called) = sizes.get(&key) {
                    counts.extend(called.iter().filter(|values| values.iter().any(|count| *count != 1)).cloned());
                }
                let mut said: HashSet<Id> = HashSet::new();
                for values in counts {
                    for (code, message, id) in self.block_at(block, found, declared, &values) {
                        if said.insert(id) {
                            let mut diagnostic = at(code, message, &file.shown, &model.source_map, id);
                            diagnostic.function = Some(cut_text(self.names.resolve(function.name)).into_owned());
                            self.push(library, diagnostic);
                        }
                    }
                }
            }
        }
    }

    /// What a block says at the size that these values of its variadic parameters give, in order: the lines of a side
    /// that grows that end on a place another line took, then the guards of its outline.
    fn block_at(&self, judged: &Judged, function: &DeclaredFunction, libraries: &[DeclaredLibrary], values: &[u32]) -> Vec<Problem> {
        let section = &judged.section;
        let (inputs, outputs, optional) = signed(function, self.names, &self.table, libraries, Some(values));
        let block = Block::of(section, self.names, optional);
        let found = places(&block, &inputs, &outputs);
        if found.inputs.len() != inputs.len() {
            // Past the bound of the places, the drawing gives the defaults of the notation: no line applies, no outline.
            return Vec::new();
        }
        let mut problems = Vec::new();
        if judged.lines_grow {
            for (line, placed) in section.lines.iter().zip(&block.lines) {
                if line.side != BlockSide::Top && !found.holds(placed, &inputs, &outputs) {
                    let message = format!(
                        "another line took this place on a side that grows to {} places, so `{}` keeps its default place (11.2)",
                        found.left,
                        cut_text(&placed.name)
                    );
                    problems.push((Code::Lc5014, message, line.id));
                }
            }
        }
        if let Some(outline) = &judged.outline {
            problems.extend(draw(outline, &Frame::of(&found, &inputs, &outputs, CHAMFER)).err().unwrap_or_default());
        }
        problems
    }
}
