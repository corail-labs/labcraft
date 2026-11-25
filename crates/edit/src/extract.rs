//! `function.extract` (commands.md 9.8): nodes of one frame moved into a new function of the same folder, and a call to
//! it in their place, one step over both files. What the nodes read from outside becomes its inputs, what they give
//! outside its outputs, named as the values, so that nothing outside changes; an error that enters and one that leaves
//! become its `err` (9.6). The call stands where the nodes were, and the nodes keep their places relative to one another.

use std::collections::{BTreeSet, HashMap, HashSet};

use labcraft_format::{is_name, read};
use labcraft_graph::{Arg, Decl, LayoutKind, OrderRef, Rhs, Statement, StatementKind, Target};
use labcraft_types::{Limits, Name};

use crate::fragment::{self, Clipped};
use crate::layout::{designate, shown};
use crate::model::Edit;
use crate::names::fresh;
use crate::places::{args, operand, slots};
use crate::scope::{self, BodyPath, Bound, Loc, Value};
use crate::surround::{push_entry, type_of};
use crate::wire::{depends_on_itself, names_in};
use crate::{EditError, Edited, Files, Judge, Kind, Lookup, Point, Renamed, TypeKind, cut};

/// The name of the input and of the output of the error chain (spec 7).
const ERR: &str = "err";

/// A value that crosses the edge of the selection: its name, as the caller has it, and its type as `check` writes it.
struct Crossing {
    name: Name,
    ty: String,
    error: bool,
}

/// `function.extract`: the caller `path` with a call in place of `nodes`, and the new function `name`, in `new_file`.
pub fn extract(
    files: &Files,
    path: &str,
    nodes: &[String],
    name: &str,
    new_file: &str,
    judge: &dyn Judge,
    limits: &Limits,
) -> Result<Edited, EditError> {
    let Some(text) = files.get(path) else {
        return Err(EditError::new("path.not_found", format!("`{path}` is not a file of the project")).of("file"));
    };
    if !is_name(name) {
        return Err(EditError::new(
            "name.invalid",
            format!("`{}` is not a name for a function: a letter, then letters, digits and `_`, and no reserved word (R-22)", cut(name)),
        )
        .of("name"));
    }
    let mut edit = Edit::read(path, text, limits)?;
    // A text not in canonical form names no node by its line, and the check could not give the types of what crosses:
    // the command reads its canonical form, which it writes anyway (commands.md 9.1).
    if edit.lines.is_empty() && !edit.body_function()?.body.statements.is_empty() {
        let canonical = edit.render()?;
        edit = Edit::read(path, &canonical, limits)?;
    }
    refuse_taken(&edit, judge, path, name)?;
    if nodes.is_empty() {
        return Err(EditError::invalid("nodes", "`nodes` names at least one node"));
    }
    let Clipped { mut clip, outer, held, entering, waited, mut items, origin, drawn, .. } =
        fragment::clip(&edit, judge, nodes, "moved into a function")?;
    let function = edit.body_function()?.clone();
    let frame: BodyPath = outer.first().map(|loc| loc.body.clone()).unwrap_or_default();
    refuse_panel(&edit, &function, &held)?;

    // What enters: each value read from outside, as the frame sees it, with its type.
    let mut values: HashMap<Name, Vec<Value>> = HashMap::new();
    for value in scope::values(&function) {
        values.entry(value.name).or_default().push(value);
    }
    let made_inside = |value: &Value| matches!(&value.bound, Bound::Line(loc, _) | Bound::Head(loc, _) if held.contains(loc));
    let mut inputs: Vec<Crossing> = Vec::new();
    for read_name in &entering {
        let mut candidates: Vec<&Value> =
            values.get(read_name).into_iter().flatten().filter(|value| !made_inside(value) && scope::sees(&function, value, &frame)).collect();
        // The output of an input of its name is not what a reader reads (R-2).
        if candidates.iter().any(|value| matches!(value.bound, Bound::Input)) {
            candidates.retain(|value| matches!(value.bound, Bound::Input));
        }
        let Some(value) = candidates.first() else { continue };
        inputs.push(crossing(&edit, judge, value)?);
    }

    // What leaves: each value that a node selected binds and that is read outside, or that is an output of the function.
    let reads_outside = reads_outside(&function, &held);
    let mut outputs: Vec<Crossing> = Vec::new();
    for loc in &outer {
        let Some(StatementKind::Binding { targets, .. }) = scope::statement(&function, loc).map(|statement| &statement.kind) else { continue };
        for (index, target) in targets.iter().enumerate() {
            let Target::Name(_, bound) = target else { continue };
            let value = Value { name: *bound, bound: Bound::Line(loc.clone(), index), seen_in: frame.clone() };
            let of_function = frame.is_empty() && function.signature.outputs.iter().any(|output| output.name == *bound);
            let read = reads_outside.get(bound).is_some_and(|places| places.iter().any(|seen_in| scope::sees(&function, &value, seen_in)));
            if of_function || read {
                outputs.push(crossing(&edit, judge, &value)?);
            }
        }
    }

    // The error chain: exactly one error in and one out, when no other value of the new function bears `err`.
    // It goes last on each side (spec 7).
    let chain = match chain(&clip, &inputs, &outputs)? {
        Some((entering, leaving)) => {
            let input = inputs.remove(entering);
            inputs.push(input);
            let output = outputs.remove(leaving);
            outputs.push(output);
            true
        }
        None => false,
    };
    let mut edited = Edited::default();
    let err = clip.intern(ERR);
    if chain {
        let (from_in, from_out) = (inputs.last().map(|found| found.name), outputs.last().map(|found| found.name));
        if let Some(from) = from_in.filter(|from| *from != err) {
            crate::names::rename_everywhere(&mut clip, from, err)?;
            edited.renamed.push(Renamed { file: new_file.to_owned(), from: clip.text(from).to_owned(), to: ERR.to_owned() });
        }
        if let Some(from) = from_out.filter(|from| *from != err)
            && give_err(&mut clip, from, err)?
        {
            rename_items(&mut items, from, err);
        }
    }

    // The new function: its name, its signature, the `use` lines it needs, and its places.
    let signature_text = signature(&edit, name, &inputs, &outputs, chain);
    let signature = read_signature(&mut clip, &signature_text)?;
    let new_name = clip.intern(name);
    {
        let function = clip.body_function_mut()?;
        function.name = new_name;
        function.signature = signature;
    }
    clip.file.uses = fragment::needed(&clip, &edit.file.uses)?;
    // A type of the signature can name a library that the caller does not import (`visa.session` given by `dmm.open`).
    for found in inputs.iter().chain(&outputs) {
        if let Some((library, _)) = found.ty.split_once('.')
            && labcraft_format::is_name(library)
        {
            crate::add::add_use(&mut clip, library);
        }
    }
    // The labels outside that the nodes waited for go to the call.
    fragment::take_out_all(&mut clip, judge, Vec::new(), waited.clone())?;
    let bare = clip.render()?;
    let wanted = corner_of(&clip, judge, new_file, &bare);
    match wanted {
        Some(corner) => fragment::shift_items(&mut clip, &mut items, corner)?,
        // Not drawn: what was counted from the corner of the selection is placed automatically.
        None => items.retain(|(_, in_frame)| !in_frame),
    }
    fragment::set_layout(&mut clip, items);
    let new_text = clip.render()?;
    if new_text.len() as u64 > limits.max_file_size {
        return Err(EditError::new("file.too_large", format!("`{new_file}` would be over the limit of {} bytes", limits.max_file_size)).of("nodes"));
    }

    // The caller: the call in place of the nodes, their entries of `layout` gone, the call where they were.
    let label = call_label(&edit, &function, &held, &outputs, name)?;
    let call = call_statement(&mut edit, name, &inputs, &outputs, chain, label.as_deref(), &waited)?;
    forget_held(&mut edit, &function, &held);
    let at = outer.first().map(|loc| loc.index).unwrap_or(0);
    {
        let body = scope::body_mut(edit.body_function_mut()?, &frame).ok_or_else(|| EditError::internal("a frame that went"))?;
        for loc in outer.iter().rev() {
            if loc.index < body.statements.len() {
                body.statements.remove(loc.index);
            }
        }
        let at = at.min(body.statements.len());
        body.statements.insert(at, call);
    }
    let call_loc = Loc { body: frame.clone(), index: at };
    if depends_on_itself(edit.body_function()?, &call_loc) {
        return Err(EditError::new(
            "wire.cycle",
            "a node between the nodes selected reads what they make and is read by them: the call would read its own output",
        )
        .of("nodes"));
    }
    let mut labelled = Vec::new();
    let named = designate(&mut edit, &call_loc, &mut labelled)?;
    if drawn {
        push_entry(&mut edit, named, origin, None)?;
    }
    edited.names.insert("function", name.to_owned());
    edited.names.insert("file", new_file.to_owned());
    edited.names.insert("node", shown(&edit, named));
    let caller = crate::route::follow(path, text, edit.finish()?, judge, limits)?;
    edited.texts.insert(path.to_owned(), caller);
    edited.texts.insert(new_file.to_owned(), new_text);
    Ok(edited)
}

/// The entries of the new function follow a value renamed: a `node` entry that names its line, a `wire` from it.
fn rename_items(items: &mut [(labcraft_graph::LayoutItem, bool)], from: Name, to: Name) {
    for (item, _) in items.iter_mut() {
        match &mut item.kind {
            LayoutKind::Node { node, .. } | LayoutKind::Frame { node: Some(node), .. } if !node.is_label && node.name == from => node.name = to,
            LayoutKind::Wire { from: start, .. } if !start.node.is_label && start.node.name == from => start.node.name = to,
            _ => {}
        }
    }
}

/// A name that a function or a type of the library bears, a library that the file imports, or a value or a generic
/// variable of the caller, which would read the call as that (as for `function.rename`), is `name.taken`.
fn refuse_taken(edit: &Edit, judge: &dyn Judge, path: &str, name: &str) -> Result<(), EditError> {
    let taken_by_callable = matches!(judge.callable(path, name), Lookup::Found(found) if matches!(found.kind, Kind::Function | Kind::Struct));
    let library = edit.file.uses.iter().any(|used| edit.text(used.path.rest.last().copied().unwrap_or(used.path.first)) == name);
    let function = edit.body_function()?;
    let value = scope::values(function).iter().any(|value| edit.text(value.name) == name)
        || function.signature.generics.iter().any(|generic| edit.text(generic.name) == name);
    if taken_by_callable || judge.type_file(path, name).is_some() || library || value {
        let what = if library {
            "a library that the file imports"
        } else if value {
            "a value of the function"
        } else {
            "a function or a type of the library"
        };
        return Err(EditError::new("name.taken", format!("`{name}` is the name of {what}: choose another name")).of("name"));
    }
    Ok(())
}

/// A node that reads or writes a panel element stays with its call (spec 8.2, Q-24).
fn refuse_panel(edit: &Edit, function: &labcraft_graph::Function, held: &HashSet<Loc>) -> Result<(), EditError> {
    for loc in held {
        let Some(statement) = scope::statement(function, loc) else { continue };
        let call = match &statement.kind {
            StatementKind::Call(call) => call,
            StatementKind::Binding { value: Rhs::Call(call), .. } => call,
            _ => continue,
        };
        if edit.text(call.callee.first) == "ui" && !call.callee.rest.is_empty() {
            return Err(EditError::invalid(
                "nodes",
                "a node that reads or writes a panel element stays with the function of its panel: the elements belong to their call (spec 8.2)",
            ));
        }
    }
    Ok(())
}

/// What crosses, with its type as `check` writes it. A value whose type is not known yet is `params.invalid`.
fn crossing(edit: &Edit, judge: &dyn Judge, value: &Value) -> Result<Crossing, EditError> {
    let found = type_of(edit, judge, value);
    if found.kind == TypeKind::Unknown || found.shown.is_empty() {
        return Err(EditError::invalid(
            "nodes",
            format!("`{}` has no known type yet: wire it or give it a type first, then make the function", edit.text(value.name)),
        ));
    }
    // A generic variable of the caller means nothing in the new function (commands.md 9.8).
    let generics: HashSet<&str> = edit.body_function()?.signature.generics.iter().map(|generic| edit.text(generic.name)).collect();
    if found.shown.split(|c: char| !(c.is_alphanumeric() || c == '_')).any(|word| generics.contains(word)) {
        return Err(EditError::invalid(
            "nodes",
            format!(
                "`{}` is a `{}`, which names a generic variable of the function: the new function would not know it",
                edit.text(value.name),
                found.shown
            ),
        ));
    }
    Ok(Crossing { name: value.name, ty: found.shown, error: found.kind == TypeKind::Error })
}

/// Each name read outside the statements `held`, with the bodies where it is read: the operands of a node, and its
/// `after:`.
fn reads_outside(function: &labcraft_graph::Function, held: &HashSet<Loc>) -> HashMap<Name, Vec<BodyPath>> {
    let mut found: HashMap<Name, Vec<BodyPath>> = HashMap::new();
    for loc in scope::all(function).into_iter().filter(|loc| !held.contains(loc)) {
        let Some(statement) = scope::statement(function, &loc) else { continue };
        let mut names = Vec::new();
        for (slot, seen_in) in slots(statement, &loc) {
            if let Some(read) = operand(statement, slot) {
                names_in(read, &mut names);
            }
            for name in names.drain(..) {
                found.entry(name).or_default().push(seen_in.clone());
            }
        }
        for arg in args(statement).unwrap_or_default() {
            let Arg::After { refs, .. } = arg else { continue };
            for reference in refs {
                if let OrderRef::Value(read) = reference {
                    names_in(read, &mut names);
                }
            }
        }
        for name in names {
            found.entry(name).or_default().push(loc.body.clone());
        }
    }
    found
}

/// The places of the error chain among the inputs and the outputs, when exactly one error enters and one leaves, and no
/// other value of the new function bears `err`.
fn chain(clip: &Edit, inputs: &[Crossing], outputs: &[Crossing]) -> Result<Option<(usize, usize)>, EditError> {
    let entering: Vec<usize> = inputs.iter().enumerate().filter(|(_, found)| found.error).map(|(index, _)| index).collect();
    let leaving: Vec<usize> = outputs.iter().enumerate().filter(|(_, found)| found.error).map(|(index, _)| index).collect();
    let ([entering], [leaving]) = (entering.as_slice(), leaving.as_slice()) else { return Ok(None) };
    let is_err = |name: Name| clip.text(name) == ERR;
    let other = |list: &[Crossing], chosen: usize| list.iter().enumerate().any(|(index, found)| index != chosen && is_err(found.name));
    let leaving_name = outputs.get(*leaving).map(|found| found.name);
    let other_err = other(inputs, *entering)
        || other(outputs, *leaving)
        || scope::values(clip.body_function()?).iter().any(|value| is_err(value.name) && Some(value.name) != leaving_name);
    Ok((!other_err).then_some((*entering, *leaving)))
}

/// The error `from` that leaves becomes the output `err`: renamed, when the new function does not read it, else given
/// by a line `err = from`, since a reader of `err` reads the input (R-2). Whether it was renamed.
fn give_err(clip: &mut Edit, from: Name, err: Name) -> Result<bool, EditError> {
    let (read_inside, value) = {
        let function = clip.body_function()?;
        let read_inside = reads_outside(function, &HashSet::new()).contains_key(&from);
        let value = scope::values(function)
            .into_iter()
            .find(|value| value.name == from && matches!(&value.bound, Bound::Line(loc, _) if loc.body.is_empty()));
        (read_inside, value)
    };
    if read_inside {
        let line = format!("{ERR} = {}\n", clip.text(from));
        let statement = read_statement(clip, &line)?;
        clip.body_function_mut()?.body.statements.push(statement);
        return Ok(false);
    }
    let value = value.ok_or_else(|| EditError::internal("the error that leaves went"))?;
    crate::names::rename(clip, &value, err)?;
    Ok(true)
}

/// Each crossing as `name: type`; with the error chain, the last one is `err`, written `last`.
fn typed(edit: &Edit, list: &[Crossing], chain: bool, last: &str) -> Vec<String> {
    let mut written: Vec<String> = list.iter().map(|found| format!("{}: {}", edit.text(found.name), found.ty)).collect();
    if chain && let Some(end) = written.last_mut() {
        *end = last.to_owned();
    }
    written
}

/// `fn name(a: T, …, err: error = none) -> (o: U, …, err: error) {}`, the error chain last on each side.
fn signature(edit: &Edit, name: &str, inputs: &[Crossing], outputs: &[Crossing], chain: bool) -> String {
    let params = typed(edit, inputs, chain, &format!("{ERR}: error = none"));
    let results = typed(edit, outputs, chain, &format!("{ERR}: error"));
    format!("labcraft 1\n\nfn {name}({}) -> ({}) {{\n}}\n", params.join(", "), results.join(", "))
}

/// The signature that `text`, a whole file, declares, read with the names of `edit`.
fn read_signature(edit: &mut Edit, text: &str) -> Result<labcraft_graph::Signature, EditError> {
    let limits = edit.limits().clone();
    let reading = read(text.as_bytes(), &mut edit.names, &limits);
    match reading.file.map(|file| file.decl) {
        Some(Decl::Function(function)) => Ok(function.signature),
        _ => {
            let why = reading.report.diagnostics().first().map(|diagnostic| diagnostic.message.clone()).unwrap_or_default();
            Err(EditError::invalid("nodes", format!("the signature of the new function does not read: {why}")))
        }
    }
}

/// One line of a body, read from its text with the names of `edit`.
fn read_statement(edit: &mut Edit, line: &str) -> Result<Statement, EditError> {
    let not_read = || EditError::internal(format!("`{}` does not read: a bug of `edit`", cut(line.trim())));
    let mut statement = crate::parts::read_one(edit, line).ok().flatten().ok_or_else(not_read)?;
    statement.id = edit.id();
    Ok(statement)
}

/// The label of the call: that of a node selected that an order wire from outside waits for, or `#name` when the call
/// gives nothing (N-14). Two such labels are `params.invalid`.
fn call_label(
    edit: &Edit,
    function: &labcraft_graph::Function,
    held: &HashSet<Loc>,
    outputs: &[Crossing],
    name: &str,
) -> Result<Option<String>, EditError> {
    let waited_outside: BTreeSet<String> = scope::labels(function)
        .into_iter()
        .filter(|(label, loc)| {
            held.contains(loc)
                && crate::delete::label_readers(function, *label).iter().any(|reader| match reader {
                    crate::delete::Reader::Slot(at, _) | crate::delete::Reader::After(at) => !held.contains(at),
                })
        })
        .map(|(label, _)| edit.text(label).to_owned())
        .collect();
    let mut waited = waited_outside.into_iter();
    match (waited.next(), waited.next()) {
        (Some(_), Some(_)) => Err(EditError::invalid(
            "nodes",
            "nodes outside wait for two labels of the selection: the call can bear one; make the function of each apart",
        )),
        (Some(one), None) => Ok(Some(one)),
        (None, _) if outputs.is_empty() => {
            let taken: BTreeSet<String> = scope::labels(function).into_iter().map(|(label, _)| edit.text(label).to_owned()).collect();
            Ok(Some(fresh(name, &taken)))
        }
        (None, _) => Ok(None),
    }
}

/// `#label: o1, o2 = name(a, b, err: e, after: (#x, #y))`.
fn call_statement(
    edit: &mut Edit,
    name: &str,
    inputs: &[Crossing],
    outputs: &[Crossing],
    chain: bool,
    label: Option<&str>,
    waited: &[Name],
) -> Result<Statement, EditError> {
    let mut args: Vec<String> = inputs.iter().map(|found| edit.text(found.name).to_owned()).collect();
    if chain && let Some(end) = args.last_mut() {
        *end = format!("{ERR}: {end}");
    }
    // In the order of the signature: the error chain last.
    let targets: Vec<String> = outputs.iter().map(|found| edit.text(found.name).to_owned()).collect();
    let order: Vec<String> = waited.iter().map(|label| format!("#{}", edit.text(*label))).collect();
    match order.as_slice() {
        [] => {}
        [one] => args.push(format!("after: {one}")),
        many => args.push(format!("after: ({})", many.join(", "))),
    }
    let mut line = String::new();
    if let Some(label) = label {
        line.push_str(&format!("#{label}: "));
    }
    if !targets.is_empty() {
        line.push_str(&format!("{} = ", targets.join(", ")));
    }
    line.push_str(&format!("{name}({})\n", args.join(", ")));
    read_statement(edit, &line)
}

/// The entries of `layout` of the nodes selected and of the wires that reach them or leave them go: the routes inside go
/// with them to the new function, those that crossed the edge are drawn again (commands.md 9.8).
fn forget_held(edit: &mut Edit, function: &labcraft_graph::Function, held: &HashSet<Loc>) {
    let nodes_of = fragment::index(function);
    let values_inside: HashSet<Name> = scope::values(function)
        .into_iter()
        .filter(|value| matches!(&value.bound, Bound::Line(loc, _) | Bound::Head(loc, _) if held.contains(loc)))
        .map(|value| value.name)
        .collect();
    let labels_inside: HashSet<Name> = scope::labels(function).into_iter().filter(|(_, loc)| held.contains(loc)).map(|(label, _)| label).collect();
    let is_held = |node: &labcraft_graph::NodeRef| fragment::named_by(&nodes_of, node).is_some_and(|loc| held.contains(loc));
    let Some(layout) = &mut edit.file.layout else { return };
    layout.items.retain(|item| match &item.kind {
        LayoutKind::Frame { node: Some(node), .. } | LayoutKind::Node { node, .. } => !is_held(node),
        LayoutKind::Wire { from, to, .. } => {
            let from_inside = if from.node.is_label { labels_inside.contains(&from.node.name) } else { values_inside.contains(&from.node.name) };
            !from_inside && !is_held(&to.node)
        }
        LayoutKind::Frame { node: None, .. } | LayoutKind::Terminal { .. } => true,
    });
}

/// Where the drawing of the new function puts the top-left corner of its nodes, when it draws it.
fn corner_of(clip: &Edit, judge: &dyn Judge, new_file: &str, text: &str) -> Option<Point> {
    let function = clip.body_function().ok()?;
    let nodes_of = fragment::index(function);
    let top: HashSet<String> = (0..function.body.statements.len())
        .filter_map(|index| fragment::designated(&nodes_of, function, &Loc { body: Vec::new(), index }))
        .map(|named| shown(clip, named))
        .collect();
    let places = judge.places(new_file, text);
    let placed: Vec<Point> = places.iter().filter(|place| top.contains(&place.name)).map(|place| place.at).collect();
    let x = placed.iter().map(|point| point.x).min()?;
    let y = placed.iter().map(|point| point.y).min()?;
    Some(Point { x, y })
}
