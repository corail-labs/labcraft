//! `block.set_terminal`, `block.set_size`, `block.set_band`, `block.set_color`, `block.set_fill`, `block.set_stroke`, `block.set_outline` and `block.reset` (commands.md 9.4): the
//! `terminals` and `outline` sections of the function whose block they change (spec 11.2, 11.4), never the file of a
//! caller. `fmt` puts the lines in their order.

use labcraft_format::block::{Problem, Which, named, read};
use labcraft_graph::{Attached, BandPlace, BlockPlace, BlockSide, ColourRole, Place, Terminals, TerminalsItem, TerminalsKind, places_range};
use std::collections::HashSet;

use labcraft_graph::Id;
use labcraft_types::Name;

use crate::model::Edit;
use crate::signature::Terminal;
use crate::{EditError, Edited};

/// Where `block.set_terminal` puts a terminal: a side and a place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockAt {
    pub side: BlockSide,
    pub place: Place,
}

impl BlockAt {
    /// `side` is `top`, `left` or `right`; `place` a number from 1, `last` or `middle`.
    pub fn parse(side: &str, place: &str) -> Result<BlockAt, EditError> {
        let side = BlockSide::from_word(side)
            .ok_or_else(|| EditError::invalid("side", format!("`{}` is no side: `top`, `left` or `right`", crate::cut(side))))?;
        let place = match place {
            "last" => Place::Last,
            "middle" => Place::Middle,
            number => match number.parse::<u32>() {
                Ok(number) if number >= 1 => Place::Number(number),
                _ => return Err(EditError::invalid("place", format!("`{}` is no place: a number from 1, `last` or `middle`", crate::cut(number)))),
            },
        };
        Ok(BlockAt { side, place })
    }
}

/// The inputs and the outputs of the function, by name.
fn terminals_of(edit: &Edit) -> Result<(Vec<Name>, Vec<Name>), EditError> {
    let signature = &edit.body_function()?.signature;
    Ok((signature.params.iter().map(|param| param.name).collect(), signature.outputs.iter().map(|output| output.name).collect()))
}

fn section(edit: &mut Edit) -> &mut Terminals {
    let id = edit.id();
    edit.file.terminals.get_or_insert_with(|| Terminals { id, comments: Attached::default(), items: Vec::new(), end: Vec::new() })
}

/// The section goes when nothing is left in it.
fn drop_if_empty(edit: &mut Edit) {
    if edit.file.terminals.as_ref().is_some_and(|section| section.items.is_empty() && section.end.is_empty()) {
        edit.file.terminals = None;
    }
}

/// What `check` would say of the section as it stands: the reading of `format`, shared with `check` and the drawing, so that
/// a command never writes a line that they would refuse (11.2).
fn problems(edit: &Edit) -> Result<Vec<Problem>, EditError> {
    let (inputs, outputs) = terminals_of(edit)?;
    Ok(edit.file.terminals.as_ref().map(|section| read(section, &inputs, &outputs, &edit.names).1).unwrap_or_default())
}

/// The message of a problem, for a refusal: without the reference to the spec, which `check` gives.
fn reason(message: &str) -> String {
    message.trim_end_matches(" (11.2)").to_owned()
}

/// `block.set_terminal`: the line of `terminal` on `at`, in place of the one it had; `None` removes its line.
pub(crate) fn set_terminal(edit: &mut Edit, terminal: &Terminal, at: Option<BlockAt>, edited: &mut Edited) -> Result<(), EditError> {
    crate::signature::find(edit, terminal)?;
    edited.names.insert("terminal", terminal.shown());
    let (inputs, outputs) = terminals_of(edit)?;
    let output = matches!(terminal, Terminal::Output(_));
    let name = edit.intern(terminal.name());
    let mine = |kind: &TerminalsKind| named(kind, &inputs, &outputs) == Some(Which { output, name });
    // The lines that apply now, which the change must leave applying (a line that no longer would, `check` would refuse).
    let before: HashSet<Id> = problems(edit)?.into_iter().map(|problem| problem.2).collect();
    if let Some(section) = &mut edit.file.terminals {
        section.items.retain(|item| !mine(&item.kind));
    }
    let Some(at) = at else {
        // A line removed can still push out another: the other terminal of a pair, now free to follow it, onto a place held.
        if let Some((_, message, _)) = problems(edit)?.iter().find(|problem| !before.contains(&problem.2)) {
            return Err(EditError::invalid("place", reason(message)));
        }
        drop_if_empty(edit);
        return Ok(());
    };
    // `input` or `output` only where the name alone would mean the other terminal (11.2).
    let both = inputs.contains(&name) && outputs.contains(&name);
    let alone_means_output = if both { at.side == BlockSide::Right } else { outputs.contains(&name) };
    let which = (alone_means_output != output).then_some(output);
    let place = match at.place {
        Place::Number(number) => BlockPlace::Number(edit.intern(&number.to_string())),
        Place::Last => BlockPlace::Last,
        Place::Middle => BlockPlace::Middle,
    };
    let id = edit.id();
    section(edit).items.push(TerminalsItem {
        id,
        comments: Attached::default(),
        kind: TerminalsKind::Place { side: at.side, place, output: which, name },
    });
    // What `check` would leave out after it: the new line (a place outside its side, the place of `after:`, one that another
    // line holds, on its side or as the facing place of a pair), or a line it pushes out, which applied before.
    let after = problems(edit)?;
    let refused = after.iter().find(|problem| problem.2 == id).or_else(|| after.iter().find(|problem| !before.contains(&problem.2)));
    if let Some((_, message, _)) = refused {
        return Err(EditError::invalid("place", reason(message)));
    }
    Ok(())
}

/// `block.set_size`: `top N` and `left N`, each when given; `Some(None)` removes one.
pub(crate) fn set_size(edit: &mut Edit, top: Option<Option<u32>>, left: Option<Option<u32>>) -> Result<(), EditError> {
    for (key, value, side) in [("top", top, BlockSide::Top), ("left", left, BlockSide::Left)] {
        let range = places_range(side);
        if let Some(Some(places)) = value
            && !range.contains(&places)
        {
            let (least, most) = range.into_inner();
            return Err(EditError::invalid(key, format!("this side has {least} to {most} places")));
        }
    }
    // A line that applied before and no longer does: the size cannot hold it.
    let before: HashSet<Id> = problems(edit)?.into_iter().map(|problem| problem.2).collect();
    let worse = |edit: &Edit| -> Result<bool, EditError> { Ok(problems(edit)?.iter().any(|problem| !before.contains(&problem.2))) };
    // Each size in turn, so that a refusal names the one that cannot hold the lines.
    for (key, value, which) in [("top", top, BlockSide::Top), ("left", left, BlockSide::Left)] {
        let Some(value) = value else { continue };
        write_size(edit, which, value);
        if worse(edit)? {
            // The smallest size that holds the lines, as `check` counts them.
            let mut least = None;
            let range = places_range(which);
            for places in value.unwrap_or(*range.start())..=*range.end() {
                write_size(edit, which, Some(places));
                if !worse(edit)? {
                    least = Some(places);
                    break;
                }
            }
            let message = match least {
                Some(least) => format!("the lines of the section need a larger side: the smallest size is {least}"),
                None => "the lines of the section need a larger side than any size".to_owned(),
            };
            return Err(EditError::invalid(key, message));
        }
    }
    drop_if_empty(edit);
    Ok(())
}

/// Writes `top N` or `left N`, or removes it.
fn write_size(edit: &mut Edit, which: BlockSide, value: Option<u32>) {
    if let Some(section) = &mut edit.file.terminals {
        section.items.retain(|item| !matches!(item.kind, TerminalsKind::Size { side, .. } if side == which));
    }
    if let Some(places) = value {
        let (id, digits) = (edit.id(), edit.intern(&places.to_string()));
        section(edit).items.push(TerminalsItem { id, comments: Attached::default(), kind: TerminalsKind::Size { side: which, places: digits } });
    }
}

/// The line of a terminal renamed follows it, as its `layout` entry does (spec 11.2, commands.md 9.4): called once the
/// signature has the new name.
pub(crate) fn rename_line(edit: &mut Edit, output: bool, from: Name, to: Name) -> Result<(), EditError> {
    let (mut inputs, mut outputs) = terminals_of(edit)?;
    // The signature before the rename, by which the lines name their terminals.
    for name in if output { outputs.iter_mut() } else { inputs.iter_mut() } {
        if *name == to {
            *name = from;
        }
    }
    let Some(section) = &mut edit.file.terminals else { return Ok(()) };
    // Each line that names `from` or `to` says which terminal it names, so that a name now shared by an input and an output
    // keeps naming the same one; `fmt` leaves out what the name alone already says.
    for item in &mut section.items {
        let found = named(&item.kind, &inputs, &outputs);
        if let TerminalsKind::Place { name, output: which, .. } = &mut item.kind
            && let Some(found) = found.filter(|found| found.name == from || found.name == to)
        {
            *which = Some(found.output);
            if found == (Which { output, name: from }) {
                *name = to;
            }
        }
    }
    Ok(())
}

/// The line of a terminal removed goes with it: called before the signature loses it.
pub(crate) fn remove_line(edit: &mut Edit, output: bool, name: Name) -> Result<(), EditError> {
    let (inputs, outputs) = terminals_of(edit)?;
    if let Some(section) = &mut edit.file.terminals {
        section.items.retain(|item| named(&item.kind, &inputs, &outputs) != Some(Which { output, name }));
    }
    drop_if_empty(edit);
    Ok(())
}

/// `block.set_band`: `band hidden`, `band inside` or `band above`; `None` removes it.
pub(crate) fn set_band(edit: &mut Edit, band: Option<BandPlace>) -> Result<(), EditError> {
    if let Some(section) = &mut edit.file.terminals {
        section.items.retain(|item| !matches!(item.kind, TerminalsKind::Band(_)));
    }
    if let Some(band) = band {
        let id = edit.id();
        section(edit).items.push(TerminalsItem { id, comments: Attached::default(), kind: TerminalsKind::Band(band) });
    }
    drop_if_empty(edit);
    Ok(())
}

/// `block.set_color`, `block.set_fill` and `block.set_stroke`: the line `color "#RRGGBB"`, as given (spec 11.2); `None`
/// removes it. A colour that is not `#RRGGBB` is refused.
pub(crate) fn set_colour(edit: &mut Edit, role: ColourRole, colour: Option<&str>) -> Result<(), EditError> {
    if let Some(colour) = colour
        && labcraft_graph::Rgb::parse(colour).is_none()
    {
        return Err(EditError::invalid(role.word(), format!("`{}` is no colour `#RRGGBB`", crate::cut(colour))));
    }
    if let Some(section) = &mut edit.file.terminals {
        section.items.retain(|item| !matches!(item.kind, TerminalsKind::Colour { role: written, .. } if written == role));
    }
    if let Some(colour) = colour {
        let (id, text) = (edit.id(), edit.names.intern(colour));
        section(edit).items.push(TerminalsItem { id, comments: Attached::default(), kind: TerminalsKind::Colour { role, text } });
    }
    drop_if_empty(edit);
    Ok(())
}

/// `block.set_outline`: the `outline` section from its items, one per line; `None` removes it. Items that do not read, or
/// that `check` refuses as an error (`LC5016`), are refused; an outline that breaks a guard of 11.4 is written, and `check`
/// gives its warning, as the drawing judges it at every size.
pub(crate) fn set_outline(edit: &mut Edit, items: Option<&str>) -> Result<(), EditError> {
    edit.body_function()?;
    let Some(items) = items else {
        edit.file.outline = None;
        return Ok(());
    };
    let mut section = labcraft_format::read_outline(items, &mut edit.names)
        .map_err(|message| EditError::invalid("outline", format!("the items do not read as an `outline` section: {message}")))?;
    let (_, problems) = labcraft_check::outline::read(&section, &edit.names);
    if let Some((_, message, _)) = problems.iter().find(|(code, _, _)| *code == labcraft_types::Code::Lc5016) {
        return Err(EditError::invalid("outline", message.trim_end_matches(" (11.4)").to_owned()));
    }
    section.id = edit.id();
    for item in &mut section.items {
        item.id = edit.id();
    }
    edit.file.outline = (!section.items.is_empty()).then_some(section);
    Ok(())
}

/// `block.reset`: the block goes back to the defaults of notation 3.2, or to the outline of the mask of its library.
pub(crate) fn reset(edit: &mut Edit) -> Result<(), EditError> {
    edit.body_function()?;
    edit.file.terminals = None;
    edit.file.outline = None;
    Ok(())
}
