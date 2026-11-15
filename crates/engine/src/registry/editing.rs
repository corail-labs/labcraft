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

const SETTINGS_ERRORS: &[&str] = &["command.not_enabled", "params.unknown", "params.missing", "params.invalid", "file.unwritable", "internal"];

const WINDOW_ERRORS: &[&str] =
    &["command.not_enabled", "agent.not_allowed", "params.unknown", "params.missing", "params.invalid", "edit.unknown", "internal"];

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
            "Adds a node to the body of the function. `call` is a function as a call names it (`visa.open`, `settle`), a struct type, or a variant that holds values. The command adds the `use` the file then needs. The node is named from its output (`timestamp`, `timestamp2`); an output that says nothing (`result` of `core`) names it from its function (`sum`, `product`). A node with no output gets a label (`#beep`). The required inputs stay unwired (`LC2101`); a value of a variant or a field of a struct gets a hole `?`. `at` fixes the node there (a `node … at` entry of `layout`); without it, the node is placed near its neighbours. `names.node` gives its name. `inside` and `arm` come with M1 step 5, `into` with M1 step 4.",
            object(
                &[
                    ("file", string(FILE)),
                    ("call", string("What the node calls: `visa.open`, `settle`, `Point`, `Reading.Value`.")),
                    ("inside", string("A structure, for a node inside it (M1 step 5).")),
                    ("arm", string("The arm of that structure, by its patterns (M1 step 5).")),
                    ("at", position("Where to fix the node, `[x, y]` in canvas units; without it, it is placed automatically.")),
                    ("name", string("Its name, instead of the one it would get.")),
                    ("into", string("A wire to insert the node into (M1 step 4).")),
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
            "Adds a named constant to the body of the function (spec 5.1). `value` is a constant written as in a file: a literal, an array of them, or a variant without values. A bare name is a value, not a constant. It is named `constant`, numbered when taken, or `name`. `at` fixes it there. `inside` and `arm` come with M1 step 5.",
            object(
                &[
                    ("file", string(FILE)),
                    ("value", string("The constant, as a file writes it: `240 V`, `\"MEAS?\"`, `Verdict.Pass`.")),
                    ("inside", string("A structure, for a constant inside it (M1 step 5).")),
                    ("arm", string("The arm of that structure, by its patterns (M1 step 5).")),
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
            "Wires `from` (a value, `node.output`, or a label for an order wire) to `to` (`node`, `node.input`, `r.0`, `msg.parts.2`, `node.after`, or `output x`). An input that is wired or holds a constant is `wire.occupied` unless `replace`; a value of a type the input does not take is `wire.type_mismatch` unless `accept_mismatch`, which writes it and gives `LC3001`. A wire that closes a cycle is `wire.cycle`; a value the input cannot see is `wire.out_of_scope`. A value wired to an output of the function takes the name of the output, and `renamed` says so. `arm`, and a wire out of an arm, come with M1 step 5.",
            object(
                &[
                    ("file", string(FILE)),
                    ("from", string("Where the wire starts: `reading`, `dmm.err`, `#beep`.")),
                    ("to", string("Where it ends: `verdict`, `sq.b`, `r.0`, `msg.parts.2`, `t.after`, `output voltage`.")),
                    ("arm", string("The arm of a `yield`, by its patterns (M1 step 5).")),
                    ("replace", boolean("Replace what the input holds.", false)),
                    ("accept_mismatch", boolean("Write a wire of the wrong type anyway.", false)),
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
                &[("file", string(FILE)), ("wire", string("The wire: `reading -> verdict`, `x -> sq.b`.")), ("expected_revision", revision())],
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
            description: "Runs a list of `{ \"id\", \"params\" }` in order, on the texts in memory, written once at the end, as one step of undo. If one fails, nothing is written, and the error gives its `index`. Otherwise it returns, for each command, its `names` and `renamed` (`commands`), and, for all, one `step`, one `files`, and the `diagnostics` and `summary` of one check of the root. It takes the commands that edit, at most 1,000; `expected_revision` is the batch's, judged once before its first command, and its commands take none.",
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
        planned(
            STEP_4,
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
        ),
        planned(
            STEP_2,
            editing(
                "node.move",
                "Move Nodes",
                "Move nodes of one frame by an offset, and fix them",
                "Moves nodes of one frame by `[dx, dy]` and fixes each one. Nodes of several frames are `params.invalid`.",
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
        ),
        planned(
            STEP_2,
            editing(
                "node.place",
                "Place Node",
                "Place one node at a position of its frame, and fix it",
                "Places one node at `at` in its frame, and fixes it.",
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
        ),
        planned(
            STEP_2,
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
        ),
        planned(
            STEP_4,
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
        ),
        planned(
            STEP_4,
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
        ),
        planned(
            STEP_4,
            editing(
                "input.set_constant",
                "Set Constant",
                "Write a constant on an input",
                "Writes a constant on the input (spec 5.1). For an input that is already wired or that holds a constant, and for a constant of a type that the input does not take, `replace` and `accept_mismatch` act as in `wire.connect`.",
                object(
                    &[
                        ("file", string(FILE)),
                        ("to", string("The input: `sq.b`, `msg.parts.2`.")),
                        ("arm", string("The arm of a structure, by its patterns (M1 step 5).")),
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
        ),
        planned(
            STEP_4,
            editing(
                "input.clear_constant",
                "Clear Constant",
                "Remove the constant of an input",
                "Removes the constant of an input: the input goes back to its default, is unwired, or gets a hole `?`. A constant is never the text `?`: clearing is what writes a hole (`params.invalid`).",
                object(
                    &[
                        ("file", string(FILE)),
                        ("to", string("The input: `sq.b`.")),
                        ("arm", string("The arm of a structure, by its patterns (M1 step 5).")),
                        ("expected_revision", revision()),
                    ],
                    &["file", "to"],
                ),
                EDIT_ERRORS,
                false,
            ),
        ),
        planned(
            STEP_4,
            editing(
                "input.remove_value",
                "Remove Value",
                "Remove a value of a variadic parameter or an element of an array",
                "Removes a value of a variadic parameter or an element of an array (`msg.parts.2`, `k.1`), with its wire or its constant. The values after it move up one place: this command alone moves them, on purpose.",
                object(
                    &[("file", string(FILE)), ("to", string("The value: `msg.parts.2`, `k.1`.")), ("expected_revision", revision())],
                    &["file", "to"],
                ),
                EDIT_ERRORS,
                false,
            ),
        ),
        planned(
            STEP_3,
            editing(
                "wire.route",
                "Route Wire",
                "Write the points of a wire (`wire … via`)",
                "Writes `wire … via` with the points of `via`, in the frame of the wire (spec 11). A branch starts at its first point when that point lies on a wire of the same value.",
                object(
                    &[
                        ("file", string(FILE)),
                        ("wire", string("The wire: `reading -> verdict`.")),
                        (
                            "via",
                            json!({ "type": "array", "description": "The points, `[x, y]` each, in canvas units.", "items": { "type": "array", "items": { "type": "integer" }, "minItems": 2, "maxItems": 2 } }),
                        ),
                        ("expected_revision", revision()),
                    ],
                    &["file", "wire", "via"],
                ),
                WIRE_ERRORS,
                false,
            ),
        ),
        planned(
            STEP_2,
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
        ),
        planned(
            STEP_6,
            editing(
                "frame.resize",
                "Resize Frame",
                "Write `frame size (w, h)`",
                "Writes `frame size (w, h)` from `[w, h]` (spec 11). A size too small for the content is `params.invalid`, and the message gives the smallest size.",
                object(
                    &[("file", string(FILE)), ("size", position("The size `[w, h]`, in canvas units.")), ("expected_revision", revision())],
                    &["file", "size"],
                ),
                EDIT_ERRORS,
                false,
            ),
        ),
        planned(
            STEP_6,
            editing(
                "terminal.move",
                "Move Terminal",
                "Write `input x at y` or `output x at y`",
                "Writes `input x at y` or `output x at y` (spec 11), with `y` as given: the grid of 8 and the 24 units between two terminals are rules of the gesture, and a file that places them closer draws them as written.",
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
        ),
        planned(
            STEP_6,
            editing(
                "block.set_terminal",
                "Set Block Terminal",
                "Write the place of a terminal on the block of a function",
                "Writes the line `side place name` of the `terminals` section (spec 11.2). `terminal` is `input x` or `output x`; `side` is `top`, `left` or `right`; `place` is a number, `last` or `middle`. A terminal has one line: a new place replaces the old one. `null` removes the line, and the section when nothing is left in it. A place outside its side, or one that another line holds, is `params.invalid`.",
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
        ),
        planned(
            STEP_6,
            editing(
                "block.set_size",
                "Set Block Size",
                "Write the number of places of the block, `top N` and `left N`",
                "Writes `top N` and `left N`; `null` removes one. A size whose places cannot hold the terminals that the section names is `params.invalid`, and the message gives the smallest.",
                object(
                    &[
                        ("file", string(FILE)),
                        (
                            "top",
                            json!({ "type": ["integer", "null"], "description": "The places on the top side; `null` removes the line.", "minimum": 1 }),
                        ),
                        (
                            "left",
                            json!({ "type": ["integer", "null"], "description": "The places on the left side; `null` removes the line.", "minimum": 1 }),
                        ),
                        ("expected_revision", revision()),
                    ],
                    &["file"],
                ),
                EDIT_ERRORS,
                false,
            ),
        ),
        planned(
            STEP_6,
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
        ),
        planned(
            STEP_6,
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
        ),
        planned(
            STEP_6,
            editing(
                "block.reset",
                "Reset Block",
                "Remove the `terminals` and `outline` sections of the block",
                "Removes the `terminals` and `outline` sections: the block goes back to the defaults, with the outline of the mask of its library when it has one.",
                object(&[("file", string(FILE)), ("expected_revision", revision())], &["file"]),
                EDIT_ERRORS,
                false,
            ),
        ),
        planned(
            STEP_6,
            editing(
                "library.set_mask",
                "Set Library Mask",
                "Write fields of the `[mask]` table of a library manifest",
                "Writes these fields of the table `[mask]` of the manifest of the library in `folder` (spec 11.3), and makes the manifest, holding the table alone, when the library has none; `null` removes one, and the table when nothing is left in it. The rest of the manifest is kept as written. A value that the mask refuses (a colour that is not `#RRGGBB`, an `outline` that `block.set_outline` would refuse) is `params.invalid`.",
                object(
                    &[
                        ("folder", string("The folder of the library, from the root of the project.")),
                        (
                            "band",
                            json!({ "type": ["string", "null"], "description": "The band of the library; `null` removes it.", "enum": ["hidden", "inside", "above", null] }),
                        ),
                        ("band_place", json!({ "type": ["integer", "string", "null"], "description": "The place of the band; `null` removes it." })),
                        ("color", json!({ "type": ["string", "null"], "description": "The colour, `#RRGGBB`; `null` removes it." })),
                        (
                            "outline",
                            json!({ "type": ["string", "null"], "description": "The outline, written as a multi-line TOML string; `null` removes it." }),
                        ),
                        ("expected_revision", revision()),
                    ],
                    &["folder"],
                ),
                EDIT_ERRORS,
                false,
            ),
        ),
        planned(
            STEP_5,
            editing(
                "structure.surround",
                "Surround With Structure",
                "Put nodes in a new `match`, `handle`, `for` or `loop`",
                "Puts the nodes in a new `match`, `handle`, `for` or `loop`, as section 9.6 of `docs/commands.md` says.",
                object(
                    &[
                        ("file", string(FILE)),
                        ("nodes", names("The nodes to surround.")),
                        ("kind", json!({ "type": "string", "description": "The structure to make.", "enum": ["match", "handle", "for", "loop"] })),
                        ("selector", string("The value a `match` or a `handle` looks at.")),
                        ("over", string("The array or the range a `for` goes over.")),
                        ("arm", string("The arm that takes the nodes; by default, the first.")),
                        ("expected_revision", revision()),
                    ],
                    &["file", "nodes", "kind"],
                ),
                EDIT_ERRORS,
                false,
            ),
        ),
        planned(
            STEP_5,
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
        ),
        planned(
            STEP_5,
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
        ),
        planned(
            STEP_5,
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
                NAMING_ERRORS,
                false,
            ),
        ),
        planned(
            STEP_5,
            editing(
                "structure.add_clause",
                "Add Clause",
                "Add the clause `name in over` to a `for`",
                "Adds the clause `name in over` to a `for`. `over` is an array value or a range (`0..n`).",
                object(
                    &[
                        ("file", string(FILE)),
                        ("structure", string("The `for`.")),
                        ("name", string("The name of the element.")),
                        ("over", string("The array value or the range: `0..n`.")),
                        ("expected_revision", revision()),
                    ],
                    &["file", "structure", "name", "over"],
                ),
                NAMING_ERRORS,
                false,
            ),
        ),
        planned(
            STEP_5,
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
                NAMING_ERRORS,
                false,
            ),
        ),
        planned(
            STEP_5,
            editing(
                "structure.remove_part",
                "Remove Part",
                "Remove an arm, a carry, a clause or a collect of a structure",
                "Removes one part of a structure, with what it holds. `part` is an arm (by its patterns), `carry x`, `in x` or `collect y`. The inputs that it fed become unwired, or get a hole. The last arm of a `match` or `handle` and the last clause of a `for` are `params.invalid`: delete the structure.",
                object(
                    &[
                        ("file", string(FILE)),
                        ("structure", string("The structure.")),
                        ("part", string("The part: an arm by its patterns, `carry x`, `in x` or `collect y`.")),
                        ("expected_revision", revision()),
                    ],
                    &["file", "structure", "part"],
                ),
                EDIT_ERRORS,
                false,
            ),
        ),
        planned(
            STEP_5,
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
        ),
        planned(
            STEP_6,
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
        ),
        planned(
            STEP_6,
            editing(
                "function.rename",
                "Rename Function",
                "Rename a function and its file, and every call to it",
                "Renames the function and its file. It updates every call and every reference to it in the project, in the same step.",
                object(&[("file", string(FILE)), ("name", string("Its new name.")), ("expected_revision", revision())], &["file", "name"]),
                MAKING_ERRORS,
                false,
            ),
        ),
        planned(
            STEP_6,
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
        ),
        planned(
            STEP_6,
            editing(
                "signature.add",
                "Add Terminal",
                "Add an input or an output to a function",
                "Adds an input (`input x`) or an output (`output x`). Callers are not changed: a new required input gives each of them `LC2101`.",
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
        ),
        planned(
            STEP_6,
            editing(
                "signature.remove",
                "Remove Terminal",
                "Remove an input or an output of a function, and its wires",
                "Removes an input or an output, and its wires in the body: what read the input becomes unwired or gets a hole. Callers are not changed: `check` shows each of them.",
                object(
                    &[("file", string(FILE)), ("terminal", string("The terminal: `input x` or `output x`.")), ("expected_revision", revision())],
                    &["file", "terminal"],
                ),
                EDIT_ERRORS,
                false,
            ),
        ),
        planned(
            STEP_6,
            editing(
                "signature.rename",
                "Rename Terminal",
                "Rename an input or an output, and the calls that name it",
                "Renames an input or an output in the body and in `layout`. It also updates the calls that name it in the project (`high: 250 V`), and their `wire` entries (`wire x -> c.high`), in the same step.",
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
        ),
        planned(
            STEP_6,
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
        ),
        planned(
            STEP_6,
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
        ),
        planned(
            STEP_6,
            editing(
                "type.create",
                "Create Type",
                "Write a new `.lct`: `type Name = definition`",
                "Writes a new `.lct`: `type Name = definition`, named after its file (R-22). `definition` is a type in text (spec 3.2). An existing file is `path.exists`.",
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
        ),
        planned(
            STEP_6,
            editing(
                "type.rename",
                "Rename Type",
                "Rename a type and its file, and every use of it",
                "Renames the type and its file, and every use of it in the project, in the same step.",
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
        ),
        planned(
            STEP_4,
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
        ),
        planned(
            STEP_4,
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
                        ("inside", string("A structure, for a paste inside it (M1 step 5).")),
                        ("arm", string("The arm of that structure, by its patterns (M1 step 5).")),
                        ("expected_revision", revision()),
                    ],
                    &["file", "text", "at"],
                ),
                EDIT_ERRORS,
                false,
            ),
        ),
        planned(
            STEP_6,
            plain(
                "settings.get",
                "Get Settings",
                "Read the settings, with where each comes from",
                "Returns `{ \"settings\": [{ \"key\", \"value\", \"from\" }] }`; `from` is `project`, `user` or `default`. Without `key`, it returns all of them.",
                object(&[("key", string("The key of one setting; without it, all."))], &[]),
                SETTINGS_ERRORS,
                &[],
            ),
        ),
        planned(
            STEP_6,
            Command {
                result: "`{ \"before\": …, \"scope\": \"user\" }`",
                ..plain(
                    "settings.set",
                    "Set Setting",
                    "Write a setting in the store of the user or of the project",
                    "Writes the setting in the store of the user or of the project, and returns the value it had. A setting is not a program, so this is not a step of history. An unknown key, or a value of the wrong type, is `params.invalid`.",
                    object(
                        &[
                            ("key", string("The key of the setting.")),
                            ("value", json!({ "description": "Its value." })),
                            ("scope", one_of("Where to write it.", &["user", "project"], "user")),
                        ],
                        &["key", "value"],
                    ),
                    SETTINGS_ERRORS,
                    &[Effect::Files],
                )
            },
        ),
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
            "Sets the selection of the window that shows the function, which `view.get_state` returns; all empty clears it. The window calls it on each click and rectangle of selection.",
            object(
                &[
                    ("function", string("The function shown.")),
                    ("nodes", json!({ "type": "array", "description": "The nodes selected.", "items": { "type": "string" } })),
                    ("wires", json!({ "type": "array", "description": "The wires selected.", "items": { "type": "string" } })),
                    ("terminals", json!({ "type": "array", "description": "The terminals selected.", "items": { "type": "string" } })),
                ],
                &["function"],
            ),
            WINDOW_ERRORS,
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
        planned(
            STEP_1,
            window(
                "view.detach",
                "Detach Tab",
                "Move the tab of a function into a window of its own",
                "Moves the tab of the function into a window of its own; on the web, a floating panel.",
                object(&[("function", string("The function."))], &["function"]),
                WINDOW_ERRORS,
            ),
        ),
        planned(
            STEP_1,
            window(
                "view.dock",
                "Dock Tab",
                "Move a detached tab back among the tabs of the project",
                "Moves the tab of the function back among the tabs of the window of the project.",
                object(&[("function", string("The function."))], &["function"]),
                WINDOW_ERRORS,
            ),
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
    ]
}

const STEP_1: &str = "M1 step 1 (#160)";
const STEP_2: &str = "M1 step 2 (#161)";
const STEP_3: &str = "M1 step 3 (#162)";
const STEP_4: &str = "M1 step 4 (#163)";
const STEP_5: &str = "M1 step 5 (#164)";
const STEP_6: &str = "M1 step 6 (#165)";

/// A command that a later step of M1 builds.
fn planned(step: &'static str, command: Command) -> Command {
    Command { planned: Some(step), ..command }
}

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
