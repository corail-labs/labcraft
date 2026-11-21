//! The commands of editing (`docs/commands.md`, 6.7), in its order: those built, and those registered as `planned` with
//! the step of M1 that builds them (architecture 12.11).

use serde_json::{Value, json};

use super::{Command, Effect, boolean, integer, object, one_of, string};

const FILE: &str = "The `.lcf` of the function, from the root of the project, written with `/`.";
const REVISION: &str = "For each file the command may write, the revision it expects (`sha256:…`, from the `files` of an earlier result): a file that changed since is `file.stale`, and nothing is written.";

/// What every editing command can return (commands.md 6.7).
pub(super) const EDIT_ERRORS: &[&str] = &[
    "params.unknown",
    "params.missing",
    "params.invalid",
    "path.outside_project",
    "path.not_found",
    "path.invalid",
    "file.unreadable",
    "file.unwritable",
    "file.too_large",
    "file.unparsed",
    "file.stale",
    "edit.unknown",
    "edit.ambiguous",
    "internal",
];

const NAMING_ERRORS: &[&str] = &[
    "params.unknown",
    "params.missing",
    "params.invalid",
    "path.outside_project",
    "path.not_found",
    "path.invalid",
    "file.unreadable",
    "file.unwritable",
    "file.too_large",
    "file.unparsed",
    "file.stale",
    "edit.unknown",
    "edit.ambiguous",
    "name.taken",
    "name.invalid",
    "internal",
];

const WIRE_ERRORS: &[&str] = &[
    "params.unknown",
    "params.missing",
    "params.invalid",
    "path.outside_project",
    "path.not_found",
    "path.invalid",
    "file.unreadable",
    "file.unwritable",
    "file.too_large",
    "file.unparsed",
    "file.stale",
    "edit.unknown",
    "edit.ambiguous",
    "wire.occupied",
    "wire.type_mismatch",
    "wire.cycle",
    "wire.out_of_scope",
    "internal",
];

/// `structure.add_carry`, `add_clause`, `add_collect`, `add_last`: a new name, a value of the wrong type for a `for`, a value that the
/// structure makes (a cycle).
const PART_ERRORS: &[&str] = &[
    "params.unknown",
    "params.missing",
    "params.invalid",
    "path.outside_project",
    "path.not_found",
    "path.invalid",
    "file.unreadable",
    "file.unwritable",
    "file.too_large",
    "file.unparsed",
    "file.stale",
    "edit.unknown",
    "edit.ambiguous",
    "name.taken",
    "name.invalid",
    "wire.type_mismatch",
    "wire.cycle",
    "internal",
];

const CONSTANT_ERRORS: &[&str] = &[
    "params.unknown",
    "params.missing",
    "params.invalid",
    "path.outside_project",
    "path.not_found",
    "path.invalid",
    "file.unreadable",
    "file.unwritable",
    "file.too_large",
    "file.unparsed",
    "file.stale",
    "edit.unknown",
    "edit.ambiguous",
    "wire.occupied",
    "wire.type_mismatch",
    "internal",
];

/// A command that makes or renames a file, so that a name and a path can be refused.
const MAKING_ERRORS: &[&str] = &[
    "params.unknown",
    "params.missing",
    "params.invalid",
    "path.outside_project",
    "path.not_found",
    "path.invalid",
    "path.exists",
    "file.unreadable",
    "file.unwritable",
    "file.too_large",
    "file.unparsed",
    "file.stale",
    "edit.unknown",
    "edit.ambiguous",
    "name.taken",
    "name.invalid",
    "internal",
];

pub(super) const SETTINGS_ERRORS: &[&str] =
    &["command.not_enabled", "params.unknown", "params.missing", "params.invalid", "file.unparsed", "file.unwritable", "internal"];

const WINDOW_ERRORS: &[&str] =
    &["command.not_enabled", "agent.not_allowed", "params.unknown", "params.missing", "params.invalid", "edit.unknown", "internal"];

/// `view.select` names what it selects as an editing command does (commands.md 9.2).
const SELECT_ERRORS: &[&str] = &[
    "command.not_enabled",
    "agent.not_allowed",
    "params.unknown",
    "params.missing",
    "params.invalid",
    "edit.unknown",
    "edit.ambiguous",
    "path.not_found",
    "file.unparsed",
    "internal",
];

const PROJECT_ERRORS: &[&str] = &[
    "command.not_enabled",
    "agent.not_allowed",
    "params.unknown",
    "params.missing",
    "params.invalid",
    "path.invalid",
    "path.not_found",
    "path.exists",
    "file.unwritable",
    "internal",
];

const HISTORY_ERRORS: &[&str] =
    &["command.not_enabled", "params.unknown", "history.empty", "history.not_yours", "file.stale", "file.unwritable", "internal"];

const RESULT: &str = "the result of an edit: `step`, `files` and their revisions, `names`, `renamed`, `diagnostics`, `summary`";

pub(super) fn revision() -> Value {
    json!({ "type": "object", "description": REVISION, "additionalProperties": { "type": "string" } })
}

pub(super) fn position(description: &str) -> Value {
    json!({ "type": "array", "description": description, "items": { "type": "integer" }, "minItems": 2, "maxItems": 2 })
}

/// The size of a side of a block (`block.set_size`), in the range that `check` reads (spec 11.2).
fn places(side: labcraft_graph::BlockSide) -> Value {
    let (least, most) = labcraft_graph::places_range(side).into_inner();
    let description = format!("The places on the {} side, {least} to {most}; `null` removes the line.", side.word());
    json!({ "type": ["integer", "null"], "description": description, "minimum": least, "maximum": most })
}

pub(super) fn names(description: &str) -> Value {
    json!({ "type": "array", "description": description, "items": { "type": "string" }, "minItems": 1 })
}

/// An editing command that makes a step.
fn editing(
    id: &'static str,
    label: &'static str,
    summary: &'static str,
    description: &'static str,
    params: super::Params,
    errors: &'static [&'static str],
    direct: bool,
) -> Command {
    Command {
        id,
        label,
        summary,
        description,
        params,
        result: RESULT,
        errors,
        effects: &[Effect::Files],
        long: false,
        planned: None,
        undoable: true,
        at_once: false,
        direct,
        window: false,
    }
}

#[allow(clippy::too_many_lines)]
pub(super) fn catalog() -> Vec<Command> {
    vec![
        editing(
            "node.add",
            "Add Node",
            "Add a node: a call of a function, a struct or a variant with values, named at once",
            "Adds a node to the body of the function. `call` is a function as a call names it (`visa.open`, `settle`), a struct type, or a variant that holds values. The command adds the `use` the file then needs. The node is named from its output (`timestamp`, `timestamp2`); an output that says nothing (`result` of `core`) names it from its function (`sum`, `product`). A node with no output gets a label (`#beep`). The required inputs stay unwired (`LC2101`); a value of a variant or a field of a struct gets a hole `?`. `at` fixes the node there (a `node … at` entry of `layout`); without it, the node is placed near its neighbours. `names.node` gives its name. `into` names a wire to insert the node into: the value enters the first input that takes its type, and the first output that the reader takes goes where the wire went; a parameter that takes only a literal (the template of `string.format`) never takes it, and a node that no input or no output fits is `params.invalid`. `inside` names a structure and `arm` an arm of it, by its patterns, for a node inside it.",
            object(
                &[
                    ("file", string(FILE)),
                    ("call", string("What the node calls: `visa.open`, `settle`, `Point`, `Reading.Value`.")),
                    ("inside", string("A structure, for a node inside it.")),
                    ("arm", string("The arm of that structure, by its patterns; by default, the first.")),
                    ("at", position("Where to fix the node, `[x, y]` in canvas units; without it, it is placed automatically.")),
                    ("name", string("Its name, instead of the one it would get.")),
                    ("into", string("A wire to insert the node into, named as `wire.disconnect` names it: `reading -> verdict.a`.")),
                    ("expected_revision", revision()),
                ],
                &["file", "call"],
            ),
            NAMING_ERRORS,
            true,
        ),
        editing(
            "constant.add",
            "Add Constant",
            "Add a named constant: `limit = 240 V`",
            "Adds a named constant to the body of the function (spec 5.1). `value` is a constant written as in a file: a literal, an array of them, or a variant without values. A bare name is a value, not a constant. It is named `constant`, numbered when taken, or `name`. `at` fixes it there. `inside` names a structure and `arm` an arm of it, by its patterns, for a node inside it.",
            object(
                &[
                    ("file", string(FILE)),
                    ("value", string("The constant, as a file writes it: `240 V`, `\"MEAS?\"`, `Verdict.Pass`.")),
                    ("inside", string("A structure, for a constant inside it.")),
                    ("arm", string("The arm of that structure, by its patterns; by default, the first.")),
                    ("at", position("Where to fix the constant, `[x, y]` in canvas units.")),
                    ("name", string("Its name, instead of `constant`.")),
                    ("expected_revision", revision()),
                ],
                &["file", "value"],
            ),
            NAMING_ERRORS,
            true,
        ),
        editing(
            "node.delete",
            "Delete Nodes",
            "Delete nodes, their wires and their entries of layout",
            "Deletes the nodes, their wires and their entries in `layout`; a structure goes with all it holds. No other node is deleted: an input that a deleted node fed is left out, or gets a hole `?` where the text cannot leave it out (a value of a variant, of an array or of a variadic parameter, a positional input before another), and an output of the function that it gave loses its value. A node is named by its label (`#close`) or by the name of its line.",
            object(
                &[("file", string(FILE)), ("nodes", names("The nodes: `reading`, `#close`.")), ("expected_revision", revision())],
                &["file", "nodes"],
            ),
            EDIT_ERRORS,
            true,
        ),
        editing(
            "wire.connect",
            "Connect Wire",
            "Wire a value to an input, or to an output of the function",
            "Wires `from` (a value, `node.output`, or a label for an order wire) to `to` (`node`, `node.input`, `r.0`, `msg.parts.2`, `node.after`, or `output x`). An input that is wired or holds a constant is `wire.occupied` unless `replace`; a value of a type the input does not take is `wire.type_mismatch` unless `accept_mismatch`, which writes it and gives `LC3001`. A wire that closes a cycle is `wire.cycle`; a value the input cannot see is `wire.out_of_scope`. A value wired to an output of the function takes the name of the output, and `renamed` says so. `to` names a `yield` by the output of its structure, with `arm`, the patterns of its arm; a `collect` by the output of its loop, an `until` by the loop, and `next x` by `n.next.x`. A wire from inside an arm to outside adds an output to the structure: that arm yields it, the other arms a hole, and `names.output` gives it. A wire from inside a loop to outside needs `through`: each loop it leaves takes the value out as a `last`, a `collect` or a carry of its last value.",
            object(
                &[
                    ("file", string(FILE)),
                    ("from", string("Where the wire starts: `reading`, `dmm.err`, `#beep`.")),
                    ("to", string("Where it ends: `verdict`, `sq.b`, `r.0`, `msg.parts.2`, `t.after`, `output voltage`.")),
                    ("arm", string("The arm of a `yield`, by its patterns: `true`, `_`, `1, 3..=5`.")),
                    ("replace", boolean("Replace what the input holds.", false)),
                    ("accept_mismatch", boolean("Write a wire of the wrong type anyway.", false)),
                    (
                        "through",
                        json!({
                            "description": "How each loop that the wire leaves takes the value out: `last` (its value at the last iteration), `collect` (an array of its values) or `carry` (a carry that holds its last value, from a hole, or from `none` for an `error`), for every loop, or a list, one per loop from the inside out.",
                            "oneOf": [
                                { "type": "string", "enum": ["last", "collect", "carry"] },
                                { "type": "array", "items": { "type": "string", "enum": ["last", "collect", "carry"] }, "minItems": 1 }
                            ]
                        }),
                    ),
                    ("expected_revision", revision()),
                ],
                &["file", "from", "to"],
            ),
            WIRE_ERRORS,
            true,
        ),
        editing(
            "wire.disconnect",
            "Disconnect Wire",
            "Remove a wire, and its entry of layout",
            "Removes the wire `value -> node` or `value -> node.input` and its entry in `layout`. Its input goes back to its default, is unwired, or gets a hole `?` where the text cannot leave it out. A value unwired from an output of the function takes back the name of its node, and `renamed` says so.",
            object(
                &[
                    ("file", string(FILE)),
                    ("wire", string("The wire: `reading -> verdict`, `x -> sq.b`.")),
                    ("arm", string("The arm of a `yield` that the wire reaches, by its patterns.")),
                    ("expected_revision", revision()),
                ],
                &["file", "wire"],
            ),
            EDIT_ERRORS,
            true,
        ),
        Command {
            id: "history.undo",
            label: "Undo",
            summary: "Undo the last step of the project",
            description: "Undoes the last step of the project, whoever made it, and returns what an editing command returns for the files it restored, with the `author` and the `label` of the step. Called by an agent, it undoes only a step of its own: otherwise `history.not_yours`, which names the author. A file that changed since the step is `file.stale`. Nothing to undo is `history.empty`.",
            params: object(&[], &[]),
            result: RESULT,
            errors: HISTORY_ERRORS,
            effects: &[Effect::Files],
            long: false,
            planned: None,
            undoable: false,
            at_once: false,
            direct: true,
            window: false,
        },
        Command {
            id: "history.redo",
            label: "Redo",
            summary: "Redo the last step undone",
            description: "Redoes the last step undone, under the rules of `history.undo`. Called by an agent, it redoes only a step that this agent undid.",
            params: object(&[], &[]),
            result: RESULT,
            errors: HISTORY_ERRORS,
            effects: &[Effect::Files],
            long: false,
            planned: None,
            undoable: false,
            at_once: false,
            direct: false,
            window: false,
        },
        Command {
            id: "history.list",
            label: "List History",
            summary: "List the steps of the history, newest first",
            description: "The steps, newest first, and where undo and redo stand: `{ \"steps\": [{ \"step\", \"author\", \"label\", \"files\", \"time\" }], \"undo\": 42, \"redo\": null, \"truncated\": false }`.",
            params: object(&[("limit", integer("At most this many steps.", 1, 1_000, Some(50)))], &[]),
            result: "a small object: `steps`, `undo`, `redo`",
            errors: &["command.not_enabled", "params.unknown", "params.invalid", "internal"],
            effects: &[],
            long: false,
            planned: None,
            undoable: false,
            at_once: false,
            direct: false,
            window: false,
        },
        Command {
            id: "commands.execute",
            label: "Execute Command",
            summary: "Run any command by its id, as its own tool would",
            description: "Runs the command `id` with `params`, as its own tool would: the MCP server reaches through it every command that is not a tool of its own (`commands.list` lists them). It returns what that command returns, and its errors; the author, the journal and the label of the step are that command's. `commands.execute` and `commands.batch` are `params.invalid` here.",
            params: object(
                &[
                    ("id", string("The id of the command: `node.move`.")),
                    ("params", json!({ "type": "object", "description": "Its parameters.", "default": {} })),
                ],
                &["id"],
            ),
            result: "what the command returns",
            errors: &["params.unknown", "params.missing", "params.invalid", "command.unknown", "command.not_enabled", "internal"],
            effects: &[Effect::Files],
            long: false,
            planned: None,
            undoable: false,
            at_once: false,
            direct: true,
            window: false,
        },
        Command {
            id: "commands.batch",
            label: "Run Batch",
            summary: "Run editing commands in order, as one step of undo",
            description: "Runs a list of `{ \"id\", \"params\" }` in order, on the texts in memory, written once at the end, as one step of undo. If one fails, nothing is written, and the error gives its `index`. Otherwise it returns, for each command, its `names` and `renamed` (`commands`), and, for all, one `step`, one `files`, and the `diagnostics` and `summary` of one check of the root. It takes the commands that edit but `function.create`, which makes a file, at most 1,000; `expected_revision` is the batch's, judged once before its first command, and its commands take none. In a parameter that names a node or a wire, `$i` names the node that the earlier command `i` named (`names.node`): `{ \"to\": \"$0.a\" }` after a `node.add`.",
            params: object(
                &[
                    (
                        "commands",
                        json!({ "type": "array", "description": "The commands, in order: `{ \"id\": \"node.add\", \"params\": { … } }`.", "items": { "type": "object" } }),
                    ),
                    ("label", string("The label of the step; by default, the labels of its commands.")),
                    ("expected_revision", revision()),
                ],
                &["commands"],
            ),
            result: "the result of an edit, with `commands`: the `names` and `renamed` of each",
            errors: &[
                "params.unknown",
                "params.missing",
                "params.invalid",
                "command.unknown",
                "command.not_enabled",
                "path.outside_project",
                "path.not_found",
                "path.invalid",
                "file.unreadable",
                "file.unwritable",
                "file.too_large",
                "file.unparsed",
                "file.stale",
                "edit.unknown",
                "edit.ambiguous",
                "name.taken",
                "name.invalid",
                "wire.occupied",
                "wire.type_mismatch",
                "wire.cycle",
                "wire.out_of_scope",
                "internal",
            ],
            effects: &[Effect::Files],
            long: false,
            planned: None,
            undoable: true,
            at_once: false,
            direct: true,
            window: false,
        },
        editing(
            "node.set_template",
            "Set Template",
            "Write the template of a `string.format`",
            "Writes the template of a `string.format` (spec 5.4), a string literal as the text writes it. Each new field gets its named input, unwired; the named argument of each field that is gone goes, with its wire; the others keep their wires, by name. A node that is not a `string.format`, or a template that is not a literal or whose fields do not read, is `params.invalid`.",
            object(
                &[
                    ("file", string(FILE)),
                    ("node", string("The `string.format` node.")),
                    ("template", string("The template, a string literal as the text writes it: `\"{a} and {b}\"`.")),
                    ("expected_revision", revision()),
                ],
                &["file", "node", "template"],
            ),
            EDIT_ERRORS,
            false,
        ),
        editing(
            "node.move",
            "Move Nodes",
            "Move nodes of one frame by an offset, and fix them",
            "Moves nodes of one frame by `[dx, dy]` and fixes each one. Nodes of several frames are `params.invalid`. A node that the `layout` section cannot name (no name, or a name of several bodies) gets a label first, from the name of its line or of what it calls; `names.nodes` gives the nodes as named after it, and `renamed` each `@N` that became a label.",
            object(
                &[
                    ("file", string(FILE)),
                    ("nodes", names("The nodes to move.")),
                    ("by", position("The offset `[dx, dy]`, in canvas units.")),
                    ("expected_revision", revision()),
                ],
                &["file", "nodes", "by"],
            ),
            EDIT_ERRORS,
            false,
        ),
        editing(
            "node.place",
            "Place Node",
            "Place one node at a position of its frame, and fix it",
            "Places one node at `at` in its frame, and fixes it. A node that the `layout` section cannot name gets a label first, as for `node.move`; `names.node` gives its name.",
            object(
                &[
                    ("file", string(FILE)),
                    ("node", string("The node to place.")),
                    ("at", position("Where to fix the node, `[x, y]` in canvas units.")),
                    ("expected_revision", revision()),
                ],
                &["file", "node", "at"],
            ),
            EDIT_ERRORS,
            false,
        ),
        editing(
            "node.set_view",
            "Set Node View",
            "Write `view standard` or `view compact` on nodes",
            "Writes `view standard` or `view compact` on each node. With `default`, it removes the line.",
            object(
                &[
                    ("file", string(FILE)),
                    ("nodes", names("The nodes to change.")),
                    (
                        "view",
                        json!({ "type": "string", "description": "The view to write, or `default` to remove it.", "enum": ["standard", "compact", "default"] }),
                    ),
                    ("expected_revision", revision()),
                ],
                &["file", "nodes", "view"],
            ),
            EDIT_ERRORS,
            false,
        ),
        editing(
            "value.rename",
            "Rename Value",
            "Rename a value or a label everywhere in the function",
            "Renames a value or a label (`#close` to `#shut`) everywhere in the function: its readers, its `node` line and its `wire` lines. A name that the function already binds is `name.taken`, and so is the name of an output of the function. A name that is not an identifier, or is a keyword, is `name.invalid`. An input or an output of the function is renamed by `signature.rename`.",
            object(
                &[
                    ("file", string(FILE)),
                    ("value", string("The value or the label to rename.")),
                    ("name", string("Its new name.")),
                    ("expected_revision", revision()),
                ],
                &["file", "value", "name"],
            ),
            NAMING_ERRORS,
            true,
        ),
        editing(
            "node.set_label",
            "Set Label",
            "Give a label to the node that starts on a line, or replace it",
            "Gives a label (`#close`) to the node that starts on line `line` of the file (from 1, as diagnostics count), or replaces its label. It is the one command that names a node by its line. A line where no node starts is `edit.unknown`.",
            object(
                &[
                    ("file", string(FILE)),
                    ("line", integer("The line where the node starts, from 1.", 1, 4_294_967_295, None)),
                    ("label", string("The label: `#close`.")),
                    ("expected_revision", revision()),
                ],
                &["file", "line", "label"],
            ),
            NAMING_ERRORS,
            false,
        ),
        editing(
            "input.set_constant",
            "Set Constant",
            "Write a constant on an input",
            "Writes a constant on the input (spec 5.1). For an input that is already wired or that holds a constant, and for a constant of a type that the input does not take, `replace` and `accept_mismatch` act as in `wire.connect`.",
            object(
                &[
                    ("file", string(FILE)),
                    ("to", string("The input: `sq.b`, `msg.parts.2`.")),
                    ("arm", string("The arm of a `yield`, by its patterns: `true`, `_`, `1, 3..=5`.")),
                    ("value", string("The constant, as a file writes it: `240 V`.")),
                    ("replace", boolean("Replace what the input holds.", false)),
                    ("accept_mismatch", boolean("Write a constant of the wrong type anyway.", false)),
                    ("expected_revision", revision()),
                ],
                &["file", "to", "value"],
            ),
            CONSTANT_ERRORS,
            true,
        ),
        editing(
            "input.clear_constant",
            "Clear Constant",
            "Remove the constant of an input",
            "Removes the constant of an input: the input goes back to its default, is unwired, or gets a hole `?`. A constant is never the text `?`: clearing is what writes a hole (`params.invalid`).",
            object(
                &[
                    ("file", string(FILE)),
                    ("to", string("The input: `sq.b`.")),
                    ("arm", string("The arm of a `yield`, by its patterns: `true`, `_`, `1, 3..=5`.")),
                    ("expected_revision", revision()),
                ],
                &["file", "to"],
            ),
            EDIT_ERRORS,
            false,
        ),
        editing(
            "input.remove_value",
            "Remove Value",
            "Remove a value of a variadic parameter or an element of an array",
            "Removes a value of a variadic parameter or an element of an array (`msg.parts.2`, `k.1`), with its wire or its constant. The values after it move up one place: this command alone moves them, on purpose.",
            object(&[("file", string(FILE)), ("to", string("The value: `msg.parts.2`, `k.1`.")), ("expected_revision", revision())], &["file", "to"]),
            EDIT_ERRORS,
            false,
        ),
        editing(
            "wire.route",
            "Route Wire",
            "Write the points of a wire (`wire … via`)",
            "Writes `wire … via` with the points of `via`, in the frame of the wire (spec 11), in place of the entry it had. A wire is named as `wire.disconnect` names it, `reading -> verdict.a`, the input left out when one wire joins them; also a wire into a structure (`k -> total.k`) or, from inside, to its `next`, `until`, `collect` or `yield` (`stop -> n`). A branch starts at its first point when that point lies on a wire of the same value: its junction, which then follows its trunk after every editing command (notation 2.5). `layout.arrange` with `wires` gives a wire its automatic route back. `names.wire` gives the wire.",
            object(
                &[
                    ("file", string(FILE)),
                    ("wire", string("The wire: `reading -> verdict.a`.")),
                    (
                        "via",
                        json!({ "type": "array", "description": "The points, `[x, y]` each, in canvas units, in the frame of the wire.", "items": { "type": "array", "items": { "type": "integer" }, "minItems": 2, "maxItems": 2 }, "minItems": 1, "maxItems": 256 }),
                    ),
                    ("expected_revision", revision()),
                ],
                &["file", "wire", "via"],
            ),
            EDIT_ERRORS,
            false,
        ),
        editing(
            "layout.arrange",
            "Arrange",
            "Remove fixed positions so that the automatic placement arranges them",
            "Removes the entries of what it arranges, and the automatic placement places that again around the rest, which stays fixed. It arranges the `nodes` and the routes of the wires between them, the routes of the `wires`, and the `terminals`. With `inside` (and `arm`), it arranges that branch. With none of these, it arranges the whole function, and `frame size` goes too.",
            object(
                &[
                    ("file", string(FILE)),
                    ("nodes", names("The nodes to arrange.")),
                    (
                        "wires",
                        json!({ "type": "array", "description": "The wires whose routes to arrange.", "items": { "type": "string" }, "minItems": 1 }),
                    ),
                    (
                        "terminals",
                        json!({ "type": "array", "description": "The terminals to arrange: `input x`, `output x`.", "items": { "type": "string" }, "minItems": 1 }),
                    ),
                    ("inside", string("A structure whose branch to arrange.")),
                    ("arm", string("The arm of that structure, by its patterns.")),
                    ("expected_revision", revision()),
                ],
                &["file"],
            ),
            EDIT_ERRORS,
            false,
        ),
        editing(
            "frame.resize",
            "Resize Frame",
            "Write `frame size (w, h)`, of the function or of a structure",
            "Writes `frame size (w, h)` from `[w, h]`, or with `inside` `frame ys size (w, h)` for that structure (spec 11), measured from the origin of the frame. A size too small for the content is `params.invalid`, and the message gives the smallest size. `size` `null` removes the entry: the size is automatic again (\"Remove the fixed size\"). A structure that the `layout` section cannot name gets a label first, as for `node.move`; `names.nodes` gives its name.",
            object(
                &[
                    ("file", string(FILE)),
                    (
                        "size",
                        json!({ "type": ["array", "null"], "items": { "type": "integer" }, "minItems": 2, "maxItems": 2, "description": "The size `[w, h]`, in canvas units; `null` removes the entry." }),
                    ),
                    ("inside", string("The structure whose frame it sizes; the frame of the function without it.")),
                    ("expected_revision", revision()),
                ],
                &["file", "size"],
            ),
            EDIT_ERRORS,
            false,
        ),
        editing(
            "terminal.move",
            "Move Terminal",
            "Write `input x at y` or `output x at y`",
            "Writes `input x at y` or `output x at y` (spec 11), with `y` as given, in place of the entry that the terminal has: the grid of 8 and the 24 units between two terminals are rules of the gesture, and a file that places them closer draws them as written. `layout.arrange` with the terminal removes the entry (\"Remove the fixed position\"); `names.terminal` gives the terminal.",
            object(
                &[
                    ("file", string(FILE)),
                    ("terminal", string("The terminal: `input x` or `output x`.")),
                    ("y", json!({ "type": "integer", "description": "Its position, in canvas units." })),
                    ("expected_revision", revision()),
                ],
                &["file", "terminal", "y"],
            ),
            EDIT_ERRORS,
            false,
        ),
        editing(
            "block.set_terminal",
            "Set Block Terminal",
            "Write the place of a terminal on the block of a function",
            "Writes the line `side place name` of the `terminals` section (spec 11.2). `terminal` is `input x` or `output x`; `side` is `top`, `left` or `right`; `place` is a number, `last` or `middle`. A terminal has one line: a new place replaces the old one. `null` removes the line, and the section when nothing is left in it. A place outside its side, one that another line holds, the place of `after:` (`top 1`), or a change that would leave another line unable to apply, is `params.invalid`; a place that a terminal holds by default is free, and that terminal takes the first free place.",
            object(
                &[
                    ("file", string(FILE)),
                    ("terminal", string("The terminal: `input x` or `output x`.")),
                    ("side", json!({ "type": "string", "description": "The side of the block.", "enum": ["top", "left", "right"] })),
                    (
                        "place",
                        json!({ "type": ["integer", "string", "null"], "description": "A number, `last` or `middle`; `null` removes the line." }),
                    ),
                    ("expected_revision", revision()),
                ],
                &["file", "terminal", "side", "place"],
            ),
            EDIT_ERRORS,
            false,
        ),
        editing(
            "block.set_size",
            "Set Block Size",
            "Write the number of places of the block, `top N` and `left N`",
            "Writes `top N` and `left N`; `null` removes one. A size whose places cannot hold the terminals that the section names is `params.invalid`, and the message gives the smallest.",
            object(
                &[
                    ("file", string(FILE)),
                    ("top", places(labcraft_graph::BlockSide::Top)),
                    ("left", places(labcraft_graph::BlockSide::Left)),
                    ("expected_revision", revision()),
                ],
                &["file"],
            ),
            EDIT_ERRORS,
            false,
        ),
        editing(
            "block.set_band",
            "Set Block Band",
            "Write the band of the block: hidden, inside or above",
            "Writes `band hidden`, `band inside` or `band above`; `null` removes it, and the function follows the mask of its library.",
            object(
                &[
                    ("file", string(FILE)),
                    (
                        "band",
                        json!({ "type": ["string", "null"], "description": "The band; `null` removes it.", "enum": ["hidden", "inside", "above", null] }),
                    ),
                    ("expected_revision", revision()),
                ],
                &["file", "band"],
            ),
            EDIT_ERRORS,
            false,
        ),
        editing(
            "block.set_color",
            "Set Block Color",
            "Write the colour of the band of the block, which tints its body",
            "Writes the line `color \"#RRGGBB\"` of the `terminals` section (spec 11.2): the colour of the band and, without `fill`, the tint of the body; `null` removes it, and the function follows the mask of its library. A colour that is not `#RRGGBB` is `params.invalid`.",
            object(
                &[
                    ("file", string(FILE)),
                    ("color", json!({ "type": ["string", "null"], "description": "The colour, `#RRGGBB`; `null` removes it." })),
                    ("expected_revision", revision()),
                ],
                &["file", "color"],
            ),
            EDIT_ERRORS,
            false,
        ),
        editing(
            "block.set_fill",
            "Set Block Fill",
            "Write the fill of the body of the block",
            "Writes the line `fill \"#RRGGBB\"` of the `terminals` section (spec 11.2): the fill of the body, as written, in both themes; `null` removes it. A colour that is not `#RRGGBB` is `params.invalid`.",
            object(
                &[
                    ("file", string(FILE)),
                    ("fill", json!({ "type": ["string", "null"], "description": "The fill, `#RRGGBB`; `null` removes it." })),
                    ("expected_revision", revision()),
                ],
                &["file", "fill"],
            ),
            EDIT_ERRORS,
            false,
        ),
        editing(
            "block.set_stroke",
            "Set Block Stroke",
            "Write the colour of the outline of the block",
            "Writes the line `stroke \"#RRGGBB\"` of the `terminals` section (spec 11.2): the colour of the outline, as written, in both themes; `null` removes it. A colour that is not `#RRGGBB` is `params.invalid`.",
            object(
                &[
                    ("file", string(FILE)),
                    ("stroke", json!({ "type": ["string", "null"], "description": "The colour of the outline, `#RRGGBB`; `null` removes it." })),
                    ("expected_revision", revision()),
                ],
                &["file", "stroke"],
            ),
            EDIT_ERRORS,
            false,
        ),
        editing(
            "block.set_outline",
            "Set Block Outline",
            "Write the `outline` section of the block",
            "Writes the `outline` section (spec 11.4) from `outline`, its items one per line (`template d`, or a `path` and its pieces), in canonical form; `null` removes the section, and the block takes the outline of its mask or the ordinary one. Items that do not read, or that break the limits of 11.4, are `params.invalid`; an outline that reads but breaks a guard is written, and `check` gives its warning.",
            object(
                &[
                    ("file", string(FILE)),
                    ("outline", json!({ "type": ["string", "null"], "description": "The items, one per line; `null` removes the section." })),
                    ("expected_revision", revision()),
                ],
                &["file", "outline"],
            ),
            EDIT_ERRORS,
            false,
        ),
        editing(
            "block.reset",
            "Reset Block",
            "Remove the `terminals` and `outline` sections of the block",
            "Removes the `terminals` and `outline` sections: the block goes back to the defaults, with the outline of the mask of its library when it has one.",
            object(&[("file", string(FILE)), ("expected_revision", revision())], &["file"]),
            EDIT_ERRORS,
            false,
        ),
        editing(
            "library.set_mask",
            "Set Library Mask",
            "Write fields of the `[mask]` table of a library manifest",
            "Writes these fields of the table `[mask]` of the manifest of the library in `folder` (spec 11.3), and makes the manifest, holding the table alone, when the library has none; `null` removes one, the table when nothing is left in it, and the manifest when nothing else is left in it either. The rest of the manifest, its comments and its order are kept as written. A value that the mask refuses (a colour that is not `#RRGGBB`, a `band` that 11.3 refuses, an `outline` that `block.set_outline` would refuse) is `params.invalid`; `outline` is written as a multi-line literal TOML string.",
            object(
                &[
                    ("folder", string("The folder of the library, from the root of the project.")),
                    (
                        "band",
                        json!({ "type": ["string", "null"], "description": "The text of the band, one line of 64 characters at most; `null` removes it." }),
                    ),
                    (
                        "band_place",
                        json!({ "type": ["string", "null"], "description": "The place of the band; `null` removes it.", "enum": ["hidden", "inside", "above", null] }),
                    ),
                    ("color", json!({ "type": ["string", "null"], "description": "The colour of the band, `#RRGGBB`; `null` removes it." })),
                    ("fill", json!({ "type": ["string", "null"], "description": "The fill of the body, `#RRGGBB`; `null` removes it." })),
                    ("stroke", json!({ "type": ["string", "null"], "description": "The colour of the outline, `#RRGGBB`; `null` removes it." })),
                    ("outline", json!({ "type": ["string", "null"], "description": "The items of the outline, one per line; `null` removes it." })),
                    ("expected_revision", revision()),
                ],
                &["folder"],
            ),
            EDIT_ERRORS,
            false,
        ),
        editing(
            "structure.surround",
            "Surround With Structure",
            "Put nodes in a new `match`, `handle`, `for` or `loop`, or make an empty one",
            "Puts the nodes of one frame in a new `match`, `handle`, `for` or `loop`, as section 9.6 of `docs/commands.md` says, one step: nothing moves on the screen. A `match` and a `handle` need `selector`, a value, and get the arms that make them complete; a `for` needs `over`, an array value or a range (`0..n`). The nodes go into `arm`, by its patterns, or the first. A value made inside and read outside becomes an output that keeps its name, and the value inside takes a suffix (`renamed`); in a loop, a `collect`. With no `nodes`, the structure is empty: it goes `inside` a structure and its `inside_arm`, or in the body of the function, at `at` or placed automatically. `names.structure` names it and `names.outputs` gives its outputs.",
            object(
                &[
                    ("file", string(FILE)),
                    ("kind", json!({ "type": "string", "description": "The structure to make.", "enum": ["match", "handle", "for", "loop"] })),
                    ("nodes", names("The nodes to surround, of one frame; none for an empty structure.")),
                    ("selector", string("The value a `match` or a `handle` looks at.")),
                    ("over", string("The array or the range a `for` goes over: `readings`, `0..n`.")),
                    ("arm", string("The arm that takes the nodes, by its patterns; by default, the first.")),
                    ("inside", string("For an empty structure: the structure it goes in.")),
                    ("inside_arm", string("For an empty structure: the arm of `inside` it goes in, by its patterns.")),
                    ("at", position("For an empty structure: where to fix it, `[x, y]` in canvas units.")),
                    ("expected_revision", revision()),
                ],
                &["file", "kind"],
            ),
            WIRE_ERRORS,
            false,
        ),
        editing(
            "structure.add_arm",
            "Add Arm",
            "Add an arm to a `match` or a `handle`",
            "Adds an arm to a `match` or a `handle`, with its patterns as the text writes them (`1, 3..=5`, `io.timeout(err)`). Its `yield` gives a hole for each output of the structure; in a structure that gives nothing, it has none.",
            object(
                &[
                    ("file", string(FILE)),
                    ("structure", string("The structure.")),
                    ("patterns", string("The patterns of the arm, as the text writes them.")),
                    ("expected_revision", revision()),
                ],
                &["file", "structure", "patterns"],
            ),
            EDIT_ERRORS,
            false,
        ),
        editing(
            "structure.set_patterns",
            "Set Patterns",
            "Change the patterns of an arm",
            "Changes the patterns of an arm. `arm` names the arm by its patterns as the text writes them (`_`, not `other`).",
            object(
                &[
                    ("file", string(FILE)),
                    ("structure", string("The structure.")),
                    ("arm", string("The arm, by its patterns.")),
                    ("patterns", string("The new patterns.")),
                    ("expected_revision", revision()),
                ],
                &["file", "structure", "arm", "patterns"],
            ),
            EDIT_ERRORS,
            false,
        ),
        editing(
            "structure.add_carry",
            "Add Carry",
            "Add `carry name = initial` to a loop",
            "Adds `carry name = initial` to a loop (spec 6.2), and its output. Without `initial`, the carry starts from a hole, and then needs `type`. Its `next` has no line until it is wired: `check` says so.",
            object(
                &[
                    ("file", string(FILE)),
                    ("structure", string("The loop.")),
                    ("name", string("The name of the carry.")),
                    ("initial", string("Its initial value; by default, a hole.")),
                    ("type", string("Its type; by default, that of `initial`.")),
                    ("expected_revision", revision()),
                ],
                &["file", "structure", "name"],
            ),
            PART_ERRORS,
            false,
        ),
        editing(
            "structure.add_clause",
            "Add Clause",
            "Add the clause `name in over` to a `for`",
            "Adds the clause `name in over` to a `for`, after its other `in` clauses. `over` is an array value or a range (`0..n`); `name`, by default, comes from the value (`readings` gives `reading`). `names.clause` gives it.",
            object(
                &[
                    ("file", string(FILE)),
                    ("structure", string("The `for`.")),
                    ("over", string("The array value or the range: `0..n`.")),
                    ("name", string("The name of the element; by default, from the value.")),
                    ("expected_revision", revision()),
                ],
                &["file", "structure", "over"],
            ),
            PART_ERRORS,
            false,
        ),
        editing(
            "structure.add_collect",
            "Add Collect",
            "Add `collect value` to a loop",
            "Adds `collect value` to a loop, and its output.",
            object(
                &[
                    ("file", string(FILE)),
                    ("structure", string("The loop.")),
                    ("value", string("The value to collect.")),
                    ("name", string("The name of the output; by default, from the value.")),
                    ("expected_revision", revision()),
                ],
                &["file", "structure", "value"],
            ),
            PART_ERRORS,
            false,
        ),
        editing(
            "structure.add_last",
            "Add Last",
            "Add `last value` to a loop",
            "Adds `last value` to a loop, and its output: the value at the last iteration, which needs no initial value.",
            object(
                &[
                    ("file", string(FILE)),
                    ("structure", string("The loop.")),
                    ("value", string("The value whose last one the loop gives.")),
                    ("name", string("The name of the output; by default, from the value.")),
                    ("expected_revision", revision()),
                ],
                &["file", "structure", "value"],
            ),
            PART_ERRORS,
            false,
        ),
        editing(
            "structure.remove_part",
            "Remove Part",
            "Remove an arm, a carry, a clause, a collect or a last of a structure",
            "Removes one part of a structure, with what it holds. `part` is an arm (by its patterns), `carry x`, `in x`, `collect y` or `last y`. The inputs that it fed become unwired, or get a hole. The last arm of a `match` or `handle` and the last clause of a `for` are `params.invalid`: delete the structure.",
            object(
                &[
                    ("file", string(FILE)),
                    ("structure", string("The structure.")),
                    ("part", string("The part: an arm by its patterns, `carry x`, `in x`, `collect y` or `last y`.")),
                    ("expected_revision", revision()),
                ],
                &["file", "structure", "part"],
            ),
            EDIT_ERRORS,
            false,
        ),
        editing(
            "structure.set_passage",
            "Set Passage",
            "Make a carry, a collect or a last of a loop another of the three",
            "Changes how a loop takes one value out: `part` (`carry x`, `collect y` or `last y`) becomes a `carry`, a `collect` or a `last`, its output and what reads it kept. A carry that the body reads becomes neither; a `for parallel` takes no carry.",
            object(
                &[
                    ("file", string(FILE)),
                    ("structure", string("The loop.")),
                    ("part", string("The part: `carry x`, `collect y` or `last y`.")),
                    ("to", json!({ "type": "string", "description": "What it becomes.", "enum": ["carry", "collect", "last"] })),
                    ("expected_revision", revision()),
                ],
                &["file", "structure", "part", "to"],
            ),
            EDIT_ERRORS,
            false,
        ),
        editing(
            "structure.set_option",
            "Set Option",
            "Turn `continue_on_error` or `parallel` on or off",
            "Turns `continue_on_error` of a `loop` (spec 6.3) or `parallel` of a `for` (spec 6.2) on or off.",
            object(
                &[
                    ("file", string(FILE)),
                    ("structure", string("The structure.")),
                    ("option", json!({ "type": "string", "description": "The option.", "enum": ["continue_on_error", "parallel"] })),
                    ("on", json!({ "type": "boolean", "description": "Whether the option is on." })),
                    ("expected_revision", revision()),
                ],
                &["file", "structure", "option", "on"],
            ),
            EDIT_ERRORS,
            false,
        ),
        editing(
            "function.create",
            "Create Function",
            "Write a new function, `fn name() -> () {}`",
            "Writes a new function, `fn name() -> () {}`, named after its file (R-22). An existing file is `path.exists`.",
            object(
                &[
                    ("path", string("The new `.lcf`, from the root of the project, written with `/`.")),
                    ("doc", string("Its documentation comment.")),
                    ("expected_revision", revision()),
                ],
                &["path"],
            ),
            MAKING_ERRORS,
            false,
        ),
        editing(
            "function.rename",
            "Rename Function",
            "Rename a function and its file, and every call to it",
            "Renames the function and its file, in its folder. It updates every call and every reference to it in the project, in the same step; `names.file` gives the new file. It does not go in a `commands.batch`.",
            object(&[("file", string(FILE)), ("name", string("Its new name.")), ("expected_revision", revision())], &["file", "name"]),
            MAKING_ERRORS,
            false,
        ),
        editing(
            "function.set_doc",
            "Set Documentation",
            "Write the documentation comment of a function",
            "Writes the documentation comment (`///`); an empty text removes it.",
            object(
                &[("file", string(FILE)), ("doc", string("The documentation; empty removes it.")), ("expected_revision", revision())],
                &["file", "doc"],
            ),
            EDIT_ERRORS,
            false,
        ),
        editing(
            "signature.add",
            "Add Terminal",
            "Add an input or an output to a function",
            "Adds an input (`input x`) or an output (`output x`). Callers are not changed: a new required input gives each of them `LC2101`. An output named like a value that a line of the body binds is given by that line.",
            object(
                &[
                    ("file", string(FILE)),
                    ("terminal", string("The terminal: `input x` or `output x`.")),
                    ("type", string("Its type.")),
                    ("default", string("Its default value, for an input; without it, the input is required.")),
                    ("position", integer("Its position among the terminals; by default, the last, before `err`.", 0, 1_000, None)),
                    ("expected_revision", revision()),
                ],
                &["file", "terminal", "type"],
            ),
            NAMING_ERRORS,
            false,
        ),
        editing(
            "signature.remove",
            "Remove Terminal",
            "Remove an input or an output of a function, and its wires",
            "Removes an input or an output, and its wires in the body: what read the input becomes unwired or gets a hole, and its entry of `layout` and its line of `terminals`. Callers are not changed: `check` shows each of them.",
            object(
                &[("file", string(FILE)), ("terminal", string("The terminal: `input x` or `output x`.")), ("expected_revision", revision())],
                &["file", "terminal"],
            ),
            EDIT_ERRORS,
            false,
        ),
        editing(
            "signature.rename",
            "Rename Terminal",
            "Rename an input or an output, and the calls that name it",
            "Renames an input or an output in the body, in `layout` and in the line of `terminals` that names it. It also updates the calls that name it in the project (`high: 250 V`), and their `wire` entries (`wire x -> c.high`), in the same step.",
            object(
                &[
                    ("file", string(FILE)),
                    ("terminal", string("The terminal: `input x` or `output x`.")),
                    ("name", string("Its new name.")),
                    ("expected_revision", revision()),
                ],
                &["file", "terminal", "name"],
            ),
            NAMING_ERRORS,
            false,
        ),
        editing(
            "signature.set_type",
            "Set Terminal Type",
            "Change the type of an input or an output",
            "Changes the type of an input or an output; `check` judges the wires that it reaches.",
            object(
                &[
                    ("file", string(FILE)),
                    ("terminal", string("The terminal: `input x` or `output x`.")),
                    ("type", string("Its new type.")),
                    ("expected_revision", revision()),
                ],
                &["file", "terminal", "type"],
            ),
            EDIT_ERRORS,
            false,
        ),
        editing(
            "signature.set_default",
            "Set Terminal Default",
            "Set the default value of an input",
            "Sets the default value of an input (a constant, spec 4); `null` makes the input required.",
            object(
                &[
                    ("file", string(FILE)),
                    ("terminal", string("The input: `input x`.")),
                    (
                        "default",
                        json!({ "type": ["string", "null"], "description": "The constant, as a file writes it; `null` makes the input required." }),
                    ),
                    ("expected_revision", revision()),
                ],
                &["file", "terminal", "default"],
            ),
            EDIT_ERRORS,
            false,
        ),
        editing(
            "type.create",
            "Create Type",
            "Write a new `.lct`: `type Name = definition`",
            "Writes a new `.lct`: `type Name = definition`, named after its file (R-22). `definition` is a type in text (spec 3.2); one that does not read is `params.invalid`, with why. An existing file is `path.exists`. It does not go in a `commands.batch`.",
            object(
                &[
                    ("path", string("The new `.lct`, from the root of the project, written with `/`.")),
                    ("definition", string("The type, in text.")),
                    ("doc", string("Its documentation comment.")),
                    ("expected_revision", revision()),
                ],
                &["path", "definition"],
            ),
            MAKING_ERRORS,
            false,
        ),
        editing(
            "type.rename",
            "Rename Type",
            "Rename a type and its file, and every use of it",
            "Renames the type and its file, in its folder, and every use of it in the project (signatures, other types, constructions, variants), in the same step; `names.file` gives the new file. It does not go in a `commands.batch`.",
            object(
                &[
                    ("file", string("The `.lct` of the type, from the root of the project, written with `/`.")),
                    ("name", string("Its new name.")),
                    ("expected_revision", revision()),
                ],
                &["file", "name"],
            ),
            MAKING_ERRORS,
            false,
        ),
        Command {
            result: "`{ \"text\": … }`, the nodes as the text of a function `clipboard`",
            ..plain(
                "fragment.copy",
                "Copy Fragment",
                "Return nodes as text, in canonical form; writes nothing",
                "Returns `{ \"text\": … }`, the nodes as the text of a function `clipboard`, in canonical form, with the `use` they need, the wires between them and their `layout` entries counted from the top-left corner of the selection. An input wired from outside is left out, or gets a hole. Writes nothing, and is not a step of history. Nodes of several frames are `params.invalid`. A text above 4 MiB is `file.too_large`, never cut.",
                object(&[("file", string(FILE)), ("nodes", names("The nodes to copy."))], &["file", "nodes"]),
                EDIT_ERRORS,
                &[],
            )
        },
        editing(
            "fragment.paste",
            "Paste Fragment",
            "Put the body of a function text into the frame",
            "Puts the body of the function that `text` holds into the frame, its top-left corner at `at`, each node that has a `node … at` entry fixed. Of its `layout` section, only the `node` and `wire` entries are taken. It adds the `use` that the file then needs. A name already taken in the function gets a suffix, and `renamed` lists them. A `text` that does not parse, or holds no function, is `params.invalid`; above 4 MiB, `file.too_large`. `names.nodes` gives the names of the nodes pasted.",
            object(
                &[
                    ("file", string(FILE)),
                    ("text", string("The text of a function, as `fragment.copy` returns it.")),
                    ("at", position("Where the top-left corner goes, `[x, y]` in canvas units.")),
                    ("inside", string("A structure, for a paste inside it.")),
                    ("arm", string("The arm of that structure, by its patterns; by default, the first.")),
                    ("expected_revision", revision()),
                ],
                &["file", "text", "at"],
            ),
            EDIT_ERRORS,
            false,
        ),
        plain(
            "settings.get",
            "Get Settings",
            "Read the settings, with where each comes from",
            "Returns `{ \"settings\": [{ \"key\", \"value\", \"from\", \"project\" }] }`; `from` is `project`, `user` or `default`, and `project` says whether a project may hold the key. Without `key`, it returns all of them.",
            object(&[("key", string("The key of one setting; without it, all."))], &[]),
            SETTINGS_ERRORS,
            &[],
        ),
        Command {
            result: "`{ \"before\": …, \"scope\": \"user\" }`",
            ..plain(
                "settings.set",
                "Set Setting",
                "Write a setting in the store of the user or of the project",
                "Writes the setting in the store of the user or of the project, and returns what that store held, `null` for nothing. `value: null` takes the key out of the store: the other store or the default gives it again. A setting is not a program, so this is not a step of history. An unknown key, a value of the wrong type, or `scope: \"project\"` for a key that a project may not hold, is `params.invalid`.",
                object(
                    &[
                        ("key", string("The key of the setting, `group.name` (`editor.drag_pans`).")),
                        ("value", json!({ "description": "Its value; `null` takes the key out of the store." })),
                        ("scope", one_of("Where to write it.", &["user", "project"], "user")),
                    ],
                    &["key", "value"],
                ),
                SETTINGS_ERRORS,
                &[Effect::Files],
            )
        },
        Command {
            effects: &[Effect::Files],
            ..window(
                "project.new",
                "New Project",
                "Make a folder with `main.lcf`, and open it",
                "Makes the folder with `main.lcf` (`fn main() -> () {}`), and opens it with `main` shown. A folder that exists and is not empty is `path.exists`.",
                object(&[("folder", string("The folder to make, anywhere on the computer."))], &["folder"]),
                PROJECT_ERRORS,
            )
        },
        window(
            "project.open",
            "Open Project",
            "Open a folder as a project",
            "Opens the folder as a project: in this window when it shows the start page, in a new window otherwise.",
            object(&[("folder", string("The folder to open, anywhere on the computer."))], &["folder"]),
            PROJECT_ERRORS,
        ),
        Command {
            direct: true,
            result: "`{ \"workspace\", \"windows\", \"truncated\" }`: the tabs, the selection and the zoom of each window",
            ..window(
                "view.get_state",
                "Get View State",
                "Return the tabs, the selection and the zoom of the windows",
                "Returns `{ \"workspace\": \"edit\", \"windows\": [{ \"tabs\", \"shown\", \"selection\", \"zoom\" }], \"truncated\": false }`: for each window of the project, its tabs (the function, its file, its view and the second view of a split), the index of the tab shown, the selection of the tab shown as the editing commands name it, and the zoom. It is the one command of the window that an attached agent may call.",
                object(&[], &[]),
                WINDOW_ERRORS,
            )
        },
        window(
            "view.select",
            "Select",
            "Set the selection of the window that shows a function",
            "Sets the selection of the window that shows the function, which `view.get_state` returns; all empty clears it. The function is the one shown, else `params.invalid`; each name names what it has, as an editing command names it (`#close`, `x -> s.b`, `input x`), else `edit.unknown`. The window calls it on each click and rectangle of selection.",
            object(
                &[
                    ("function", string("The function shown.")),
                    ("nodes", json!({ "type": "array", "description": "The nodes selected.", "items": { "type": "string" } })),
                    ("wires", json!({ "type": "array", "description": "The wires selected.", "items": { "type": "string" } })),
                    ("terminals", json!({ "type": "array", "description": "The terminals selected.", "items": { "type": "string" } })),
                ],
                &["function"],
            ),
            SELECT_ERRORS,
        ),
        window(
            "view.set_view",
            "Set View",
            "Show a function with a view, and a second one beside it",
            "Shows the function in its tab with that view, and with `split` the second view beside it: `text` with `diagram` or with `block`. `panel` is `command.not_enabled` until M3; another pair is `params.invalid`.",
            object(
                &[
                    ("function", string("The function.")),
                    ("view", json!({ "type": "string", "description": "The view.", "enum": ["diagram", "text", "block", "panel"] })),
                    (
                        "split",
                        json!({ "type": ["string", "null"], "description": "The second view, beside the first; `null` for none.", "enum": ["diagram", "text", "block", null], "default": null }),
                    ),
                ],
                &["function", "view"],
            ),
            WINDOW_ERRORS,
        ),
        window(
            "view.close_function",
            "Close Tab",
            "Close the tab of a function",
            "Closes the tab of the function; when it was shown, the next tab is shown, else the one before it. Returns `{ \"shown\" }`, the function shown after.",
            object(&[("function", string("The function whose tab is open."))], &["function"]),
            WINDOW_ERRORS,
        ),
        window(
            "view.detach",
            "Detach Tab",
            "Move the tab of a function into a window of its own",
            "Moves the tab of the function into a window of its own, with what it shows; on the web, a floating panel. The window of the project shows its next tab. A function already in a window of its own stays there. Returns `{ \"shown\" }`, the function that the window of the project shows.",
            object(&[("function", string("The function whose tab is open."))], &["function"]),
            WINDOW_ERRORS,
        ),
        window(
            "view.dock",
            "Dock Tab",
            "Move a detached tab back among the tabs of the project",
            "Moves the tab of the function back among the tabs of the window of the project, as its last tab, and shows it there. A function whose tab is there already stays. Returns `{ \"shown\" }`.",
            object(&[("function", string("The function in a window of its own."))], &["function"]),
            WINDOW_ERRORS,
        ),
        window(
            "view.toggle_panel",
            "Toggle Panel",
            "Show or hide a panel of the window",
            "Shows or hides `project`, `palette`, `inspector`, `problems` or `run`. Without `on`, it puts the panel in the other state.",
            object(
                &[
                    (
                        "panel",
                        json!({ "type": "string", "description": "The panel.", "enum": ["project", "palette", "inspector", "problems", "run"] }),
                    ),
                    ("on", json!({ "type": "boolean", "description": "Whether to show it; by default, the other state." })),
                ],
                &["panel"],
            ),
            WINDOW_ERRORS,
        ),
        window(
            "view.reset_layout",
            "Reset Layout",
            "Put the panels back and dock every detached tab",
            "Puts the panels back where the IDE puts them by default, and docks every detached tab.",
            object(&[], &[]),
            WINDOW_ERRORS,
        ),
        window(
            "view.set_workspace",
            "Set Workspace",
            "Change the workspace of the window",
            "Changes the workspace of the window. A planned workspace is `command.not_enabled`, and the message gives its milestone.",
            object(&[("workspace", string("The workspace: `edit`."))], &["workspace"]),
            WINDOW_ERRORS,
        ),
        // The `Text` panel (commands.md 9.1; decision of the owner, 2026-11-21).
        Command {
            effects: &[Effect::Files],
            undoable: true,
            result: "`{ \"written\": true, \"revision\" }`, or `{ \"written\": false, \"kept\", \"error\" }` or `{ \"written\": false, \"kept\", \"stale\": true, \"revision\" }`",
            ..window(
                "text.save",
                "Edit Text",
                "Write the text of the Text panel when it parses, else keep it in the recovery folder",
                "Reads the text as `check` reads a file. When it parses (its grammar is right, whatever its errors of types or of `check`), writes it as `file.write` does, one step of history, and removes its copy in `.labcraft/recovery/`. When it does not, writes nothing in the project, keeps it in `.labcraft/recovery/` under the path of its file, and returns the error that stops the reading. With `expected_revision`, a file that changed since that revision is not written either: the text is kept, and `stale` says so.",
                object(
                    &[
                        ("path", string("A `.lcf` or `.lct` file of the project that exists, written with `/` from the root.")),
                        ("text", string("The whole text of the file.")),
                        (
                            "expected_revision",
                            string("The revision of the file that the text was made from; none writes over whatever the file holds."),
                        ),
                        ("keep", boolean("Only keep the text in the recovery folder, and write nothing in the project.", false)),
                    ],
                    &["path", "text"],
                ),
                TEXT_ERRORS,
            )
        },
        Command {
            result: "`{ \"texts\": [{ \"path\", \"text\", \"modified\" }], \"truncated\" }`",
            ..window(
                "text.recovered",
                "Recovered Texts",
                "List the texts of the recovery folder that are newer than their file",
                "Lists the texts kept in `.labcraft/recovery/` whose file of the project exists and changed before them, with their time in nanoseconds since 1970. At most 4 MiB per text and 16 MiB in all; past that, `truncated`.",
                object(&[], &[]),
                TEXT_ERRORS,
            )
        },
        Command {
            effects: &[Effect::Files],
            result: "`{ \"discarded\": true }`, false when there was no copy",
            ..window(
                "text.discard",
                "Discard Recovered Text",
                "Remove the copy of a file in the recovery folder",
                "Removes the text that `.labcraft/recovery/` keeps for the file, if it keeps one.",
                object(&[("path", string("A `.lcf` or `.lct` file of the project, written with `/` from the root."))], &["path"]),
                TEXT_ERRORS,
            )
        },
    ]
}

/// The commands of the `Text` panel judge a path and write files.
const TEXT_ERRORS: &[&str] = &[
    "command.not_enabled",
    "agent.not_allowed",
    "params.unknown",
    "params.missing",
    "params.invalid",
    "path.outside_project",
    "path.not_found",
    "path.invalid",
    "file.unreadable",
    "file.unwritable",
    "file.too_large",
    "internal",
];

/// A command that is not a step of history: it reads, or it writes something that is not the program.
fn plain(
    id: &'static str,
    label: &'static str,
    summary: &'static str,
    description: &'static str,
    params: super::Params,
    errors: &'static [&'static str],
    effects: &'static [Effect],
) -> Command {
    Command {
        id,
        label,
        summary,
        description,
        params,
        result: RESULT,
        errors,
        effects,
        long: false,
        planned: None,
        undoable: false,
        at_once: false,
        direct: false,
        window: false,
    }
}

/// A command of the window of the IDE.
fn window(
    id: &'static str,
    label: &'static str,
    summary: &'static str,
    description: &'static str,
    params: super::Params,
    errors: &'static [&'static str],
) -> Command {
    Command { result: "a small object: what changed in the window", window: true, ..plain(id, label, summary, description, params, errors, &[]) }
}
