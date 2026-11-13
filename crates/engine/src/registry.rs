//! The registry: each command as data (`docs/commands.md`, section 1), the same data that `commands.describe` returns
//! and that the MCP server makes its tools from. `cargo xtask commands` checks it against the tables of section 6.

use std::sync::OnceLock;

use serde_json::{Map, Value, json};

/// What a command can change (`docs/commands.md`, section 1). A command with none only reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Files,
    Instruments,
    Run,
}

impl Effect {
    pub fn as_str(self) -> &'static str {
        match self {
            Effect::Files => "files",
            Effect::Instruments => "instruments",
            Effect::Run => "run",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Command {
    pub id: &'static str,
    pub label: &'static str,
    pub summary: &'static str,
    pub description: &'static str,
    /// Its parameters, in order: a JSON Schema of an object, in the part that every MCP client reads.
    pub params: Params,
    pub result: &'static str,
    pub errors: &'static [&'static str],
    pub effects: &'static [Effect],
    pub long: bool,
    /// The milestone or step that brings it, when it is registered but not built yet.
    pub planned: Option<&'static str>,
}

impl Command {
    /// Its full description, as `commands.describe` gives it.
    pub fn describe(&self) -> Value {
        let mut described = json!({
            "id": self.id,
            "label": self.label,
            "summary": self.summary,
            "description": self.description,
            "params": self.params.schema(),
            "result": self.result,
            "errors": self.errors,
            "effects": self.effects.iter().map(|effect| effect.as_str()).collect::<Vec<_>>(),
            "long": self.long,
        });
        if let (Some(planned), Some(object)) = (self.planned, described.as_object_mut()) {
            object.insert("planned".to_owned(), json!(planned));
        }
        described
    }

    /// The names of its parameters, in order.
    pub fn parameters(&self) -> Vec<&'static str> {
        self.params.properties.iter().map(|(name, _)| *name).collect()
    }

    /// Those it cannot do without.
    pub fn required(&self) -> &'static [&'static str] {
        self.params.required
    }
}

/// The parameters of a command, in the order that the documents and the errors give them.
#[derive(Clone, Debug)]
pub struct Params {
    pub properties: Vec<(&'static str, Value)>,
    pub required: &'static [&'static str],
}

impl Params {
    /// The schema of each parameter, by name.
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.properties.iter().find(|(property, _)| *property == name).map(|(_, schema)| schema)
    }

    /// As a JSON Schema of an object, which refuses a parameter it does not name.
    pub fn schema(&self) -> Value {
        let mut map = Map::new();
        for (name, schema) in &self.properties {
            map.insert((*name).to_owned(), schema.clone());
        }
        json!({ "type": "object", "properties": map, "required": self.required, "additionalProperties": false })
    }
}

/// Every command, in the order of `docs/commands.md`, section 6.
pub fn commands() -> &'static [Command] {
    static COMMANDS: OnceLock<Vec<Command>> = OnceLock::new();
    COMMANDS.get_or_init(catalog)
}

/// The command of this id.
pub fn command(id: &str) -> Option<&'static Command> {
    commands().iter().find(|command| command.id == id)
}

/// The parameters of a command: its properties in order, and those required.
fn object(properties: &[(&'static str, Value)], required: &'static [&'static str]) -> Params {
    Params { properties: properties.to_vec(), required }
}

fn string(description: &str) -> Value {
    json!({ "type": "string", "description": description })
}

fn boolean(description: &str, default: bool) -> Value {
    json!({ "type": "boolean", "description": description, "default": default })
}

fn integer(description: &str, minimum: u64, maximum: u64, default: Option<u64>) -> Value {
    let mut schema = json!({ "type": "integer", "description": description, "minimum": minimum, "maximum": maximum });
    if let (Some(default), Some(object)) = (default, schema.as_object_mut()) {
        object.insert("default".to_owned(), json!(default));
    }
    schema
}

fn one_of(description: &str, values: &[&str], default: &str) -> Value {
    json!({ "type": "string", "description": description, "enum": values, "default": default })
}

/// A limit of a run that `null` can remove (`docs/commands.md`, 6.4).
fn limit(description: &str) -> Value {
    json!({ "type": ["integer", "null"], "description": description, "minimum": 1, "maximum": 9_223_372_036_854_775_807_u64 })
}

const PATH_RULES: &str = "Paths are relative to the root of the project, written with `/`; a shipped library reads as `<labcraft>/visa/open.lcf`.";

const CHECK_ERRORS: &[&str] = &["params.unknown", "params.missing", "params.invalid", "internal"];
const FILE_ERRORS: &[&str] = &[
    "params.unknown",
    "params.missing",
    "params.invalid",
    "path.outside_project",
    "path.not_found",
    "path.invalid",
    "file.unreadable",
    "file.unwritable",
    "file.too_large",
    "file.not_simulation",
    "internal",
];
/// `view.open_trace`: a trace that does not exist or does not read is a warning of its `trace` part (`LC7101`).
const TRACE_ERRORS: &[&str] = &["params.unknown", "params.invalid", "path.outside_project", "path.invalid", "internal"];
const RUN_ERRORS: &[&str] =
    &["params.unknown", "params.missing", "params.invalid", "path.outside_project", "path.invalid", "run.unknown", "internal"];
/// `run.set_element` on a deterministic run is not enabled (commands.md 6.4).
const SET_ELEMENT_ERRORS: &[&str] = &["command.not_enabled", "params.unknown", "params.missing", "params.invalid", "run.unknown", "internal"];

#[allow(clippy::too_many_lines)]
fn catalog() -> Vec<Command> {
    let run = "An optional run id (`r1`); without it, the run of the session: the one going, or the last one.";
    vec![
        // 6.1 Learning the language
        Command {
            id: "guide.read",
            label: "Read the Guide",
            summary: "Read the skills that teach the language, starting with the one that writes a program",
            description: "Without `skill`, lists each skill with the line that says when to use it, followed by the SKILL.md of the skill that writes a program, so that one call starts the loop; with one, returns that file of it, in Markdown. Read once at the start of a session.",
            params: object(
                &[
                    (
                        "skill",
                        json!({
                            "type": "string",
                            "description": "The name of a skill, as the list gives it.",
                            "enum": crate::guide::skills().iter().map(|skill| skill.name).collect::<Vec<_>>(),
                        }),
                    ),
                    ("file", json!({ "type": "string", "description": "A file of the skill.", "default": "SKILL.md" })),
                ],
                &[],
            ),
            result: "a small object: the skills, or one file of a skill",
            errors: &["params.unknown", "params.invalid", "internal"],
            effects: &[],
            long: false,
            planned: None,
        },
        Command {
            id: "functions.list",
            label: "List Functions",
            summary: "List every function and type the project can call, as signatures of the language",
            description: "Every function and type the project can call: the primitives (`core`, `string`, `array`, `errors`, `time`, `ui`), the shipped libraries (`visa`, `scpi`) and the project's own, each as a signature written in the language, with `always`, its error categories and the first line of its documentation (all of it with `docs`). The project's own entries come from the last check of the root. At most 1,000 entries, primitives and shipped libraries first; `library` or `filter` narrow it. Call it once before writing a first program.",
            params: object(
                &[
                    ("library", string("Only this library: `core`, `visa`, `dmm_driver`.")),
                    ("filter", string("Only the names that contain this text.")),
                    ("docs", boolean("All of the documentation of each entry, not its first line.", false)),
                ],
                &[],
            ),
            result: "a small object: `entries`, `truncated`",
            errors: &["params.unknown", "params.invalid", "internal"],
            effects: &[],
            long: false,
            planned: None,
        },
        // 6.2 Files and libraries
        Command {
            id: "project.describe",
            label: "Describe Project",
            summary: "Describe the project: its libraries, their files and what each declares, its simulation files",
            description: "The root, its libraries and sub-libraries, their `.lcf` and `.lct` files with what each declares, the simulation files with the resources they define (those of the project, and those shipped with `visa`, as `<labcraft>/visa/sim/...`), and the summary of a check of the root, without its diagnostics (`library.check` gives them). At most 1,000 files listed, then `truncated`. While the first check of the session is still going, the summary says `pending`.",
            params: object(&[], &[]),
            result: "a small object: `root`, `libraries`, `simulations`, `summary`, `truncated`",
            errors: &["params.unknown", "internal"],
            effects: &[],
            long: false,
            planned: None,
        },
        Command {
            id: "file.read",
            label: "Read File",
            summary: "Read a program, a type or a simulation file of the project or of a shipped library",
            description: "The text of a `.lcf`, `.lct` or simulation file from line `from`: `{ file, text, from, lines, lines_total, truncated }`. At most 1 MiB of text per call, as JSON writes it, cut at the end of a line: the next call starts at `from + lines`.",
            params: object(
                &[
                    ("path", string(PATH_RULES)),
                    ("from", integer("The first line to read, from 1.", 1, u64::from(u32::MAX), Some(1))),
                    ("lines", integer("How many lines to read; all of them by default.", 1, u64::from(u32::MAX), None)),
                ],
                &["path"],
            ),
            result: "a small object: `file`, `text`, `from`, `lines`, `lines_total`, `truncated`",
            errors: FILE_ERRORS,
            effects: &[],
            long: false,
            planned: None,
        },
        Command {
            id: "file.write",
            label: "Write File",
            summary: "Write a file of the project, then check its library",
            description: "Writes a `.lcf`, `.lct` or simulation file atomically, creating its folder if needed; CR LF becomes LF, and nothing else changes (`file.format` puts it in canonical form). With `check`, then checks the root and returns its report, as `library.check` does: one call writes and checks. `text` holds at most 4 MiB. A simulation file must read as one: TOML whose top-level keys each name a library that simulates.",
            params: object(
                &[
                    ("path", string(PATH_RULES)),
                    ("text", string("The whole text of the file.")),
                    ("check", boolean("Check the root once written, and return its report.", true)),
                ],
                &["path", "text"],
            ),
            result: "labcraft-check/1, or `{ written }` without `check`",
            errors: FILE_ERRORS,
            effects: &[Effect::Files],
            long: false,
            planned: None,
        },
        Command {
            id: "file.rename",
            label: "Rename File",
            summary: "Rename a file of the project, then check",
            description: "Renames a `.lcf`, `.lct` or simulation file of the project, a change of case alone included (the fix of `LC2023` can be a new name), then checks the root and returns its report.",
            params: object(&[("from", string(PATH_RULES)), ("to", string("The new path, under the rules of `from`."))], &["from", "to"]),
            result: "labcraft-check/1",
            errors: FILE_ERRORS,
            effects: &[Effect::Files],
            long: false,
            planned: None,
        },
        Command {
            id: "file.delete",
            label: "Delete File",
            summary: "Delete a file of the project, then check",
            description: "Deletes a `.lcf`, `.lct` or simulation file of the project, then checks the root and returns its report.",
            params: object(&[("path", string(PATH_RULES))], &["path"]),
            result: "labcraft-check/1",
            errors: FILE_ERRORS,
            effects: &[Effect::Files],
            long: false,
            planned: None,
        },
        Command {
            id: "library.check",
            label: "Check Library",
            summary: "Check a library and every library it uses, and return the diagnostics",
            description: "`labcraft-check/1`, exactly what `labcraft-cli check --json` gives for the folder: read each library, find what it uses, check what it declares, each body, its types, its dataflow and its panel. A library that does not check is not a failed command: its report has `status: error`. The session keeps its last check, so this answers at once when nothing changed.",
            params: object(
                &[
                    ("folder", string("A library folder of the project; the root by default.")),
                    ("deny_warnings", boolean("Warnings count for the status.", false)),
                ],
                &[],
            ),
            result: "labcraft-check/1",
            errors: &["params.unknown", "params.invalid", "path.outside_project", "path.not_found", "path.invalid", "internal"],
            effects: &[],
            long: false,
            planned: None,
        },
        Command {
            id: "file.format",
            label: "Format",
            summary: "Write a file or each file of a folder in canonical form",
            description: "`labcraft-fmt/1`, exactly what `labcraft-cli fmt --json` gives: each `.lcf` and `.lct` file in canonical form (spec 5.2), written atomically. With `check_only`, writes nothing and reports the files that are not canonical, with the first line that differs. The next command that reports a check sees what changed.",
            params: object(
                &[("path", string(PATH_RULES)), ("check_only", boolean("Write nothing; a file that would change is an error.", false))],
                &["path"],
            ),
            result: "labcraft-fmt/1",
            errors: &["params.unknown", "params.missing", "params.invalid", "path.outside_project", "path.not_found", "path.invalid", "internal"],
            effects: &[Effect::Files],
            long: false,
            planned: None,
        },
        // 6.3 Instruments
        Command {
            id: "simulation.describe",
            label: "Describe Simulation",
            summary: "Describe what a simulation file defines: instruments, resources, queries, faults",
            description: "What a simulation file of the project or of `<labcraft>/visa/sim/` defines, part by part: for each library that simulates, the description it gives of its part (for `visa`, each instrument with its resource, its `*IDN?` answer, its latency, the queries it answers and what each reads, the writes it accepts, its faults); or the diagnostics of the file. It gives the resource names to open, without reading the file.",
            params: object(&[("path", string(PATH_RULES))], &["path"]),
            result: "a small object: `file`, `parts` or `diagnostics`",
            errors: FILE_ERRORS,
            effects: &[],
            long: false,
            planned: None,
        },
        // 6.4 Runs
        Command {
            id: "run.start",
            label: "Start Run",
            summary: "Check a library, then run one of its functions and return its report",
            description: "Checks the library, then runs the function and returns `labcraft-run/1`, or `running` if it lasts longer than `wait`. `function` is named from the root (`check_voltage`, `dmm_driver.measure`); `args` gives inputs by name; `sim` a simulation file; `set` the panel elements to set and when, on the clock of the run. An unknown function, an argument that does not convert or a simulation file that does not read gives a `refused` report with its diagnostic, not a command error.",
            params: object(
                &[
                    ("function", string("The function to run, named from the root.")),
                    ("folder", string("The library folder; the root by default.")),
                    ("sim", string("A simulation file of the bench.")),
                    (
                        "args",
                        json!({ "type": "object", "description": "The value of each input, by name, in the JSON form of its type.", "additionalProperties": true, "default": {} }),
                    ),
                    (
                        "set",
                        json!({
                            "type": "array",
                            "description": "Panel elements to set, each at a time on the clock of the run.",
                            "items": object(&[("element", string("A panel element of the function.")), ("value", json!({ "description": "Its value, in the JSON form of its type." })), ("at", string("A duration from the start of the run: `5 s`."))], &["element", "value", "at"]).schema(),
                            "default": []
                        }),
                    ),
                    ("mode", one_of("Deterministic, with a simulated clock, or real time.", &["deterministic", "real_time"], "deterministic")),
                    (
                        "schedule_seed",
                        json!({ "type": ["integer", "string"], "description": "Deterministic mode: the seed of the order among ready nodes, 0 to 2^64-1; a string of digits beyond 2^53-1." }),
                    ),
                    ("timeout", string("Aborts the run after this duration on its clock: `30 s`.")),
                    ("max_iterations", limit("Iterations of each loop; default 10,000 in deterministic mode, none in real time; `null` removes it.")),
                    ("max_nodes", limit("Nodes run in all; default 1,000,000 in deterministic mode, none in real time; `null` removes it.")),
                    ("max_call_depth", integer("Depth of calls.", 1, 4096, Some(1024))),
                    (
                        "max_memory",
                        integer("The logical size of the values held at once, in bytes.", 1_048_576, 9_223_372_036_854_775_807, Some(1_073_741_824)),
                    ),
                    (
                        "max_made",
                        limit(
                            "The bytes of the values the nodes make, in all; default 1 GiB in deterministic mode, none in real time; `null` removes it.",
                        ),
                    ),
                    ("max_real_time", string("Deterministic mode: the real time the run may take; default `10 min`.")),
                    ("deny_warnings", boolean("Warnings of the folder refuse the run.", false)),
                    (
                        "trace",
                        one_of(
                            "Where the trace goes: a file under `.labcraft/runs/`, inline in the report, or nowhere.",
                            &["file", "inline", "none"],
                            "file",
                        ),
                    ),
                    (
                        "wait",
                        json!({ "type": "string", "description": "How long to wait for the end of the run before returning `running`, at most `300 s`.", "default": "30 s" }),
                    ),
                ],
                &["function"],
            ),
            result: "labcraft-run/1",
            errors: &[
                "params.unknown",
                "params.missing",
                "params.invalid",
                "path.outside_project",
                "path.invalid",
                "file.not_simulation",
                "run.busy",
                "internal",
            ],
            effects: &[Effect::Instruments, Effect::Run],
            long: true,
            planned: None,
        },
        Command {
            id: "run.wait",
            label: "Wait for Run",
            summary: "Wait for a run to end, and return its report",
            description: "Waits up to `wait` for the run, then returns its final report, or `running` again.",
            params: object(
                &[
                    ("run", string(run)),
                    ("wait", json!({ "type": "string", "description": "How long to wait, at most `300 s`.", "default": "30 s" })),
                ],
                &[],
            ),
            result: "labcraft-run/1",
            errors: RUN_ERRORS,
            effects: &[],
            long: false,
            planned: None,
        },
        Command {
            id: "run.status",
            label: "Run Status",
            summary: "Return the report of a run so far, at once",
            description: "The report so far: `running` with the clock and where each loop is, or the final report.",
            params: object(&[("run", string(run))], &[]),
            result: "labcraft-run/1",
            errors: RUN_ERRORS,
            effects: &[],
            long: false,
            planned: None,
        },
        Command {
            id: "run.set_element",
            label: "Set Panel Element",
            summary: "Set a panel element of the running function, in real time",
            description: "Sets a panel element of the running top-level function, in real time only; in deterministic mode, give the change to `run.start` in `set`, at its time on the simulated clock. The change is seen by the reads that start after it.",
            params: object(
                &[
                    ("run", string(run)),
                    ("element", string("A panel element of the function.")),
                    ("value", json!({ "description": "Its value, in the JSON form of its type." })),
                ],
                &["element", "value"],
            ),
            result: "its entry of `panel` in labcraft-run/1, with `run`: `element`, `value`, `at_ms` (`null` when the run ended first), `by`, `applied`",
            errors: SET_ELEMENT_ERRORS,
            effects: &[Effect::Run],
            long: false,
            planned: None,
        },
        Command {
            id: "run.abort",
            label: "Abort Run",
            summary: "Abort a run and return its final report",
            description: "Aborts the run (spec 8.2): no node starts any more, the resources are released, and its final report comes back with status `aborted`. A run that has already ended is left as it is.",
            params: object(&[("run", string(run))], &[]),
            result: "labcraft-run/1",
            errors: RUN_ERRORS,
            effects: &[Effect::Run],
            long: false,
            planned: None,
        },
        Command {
            id: "run.trace",
            label: "Read Trace",
            summary: "Read the trace of a run, in pages, filtered by frame, node or kind of event",
            description: "The events of the trace written by a run from number `from` that match the filters, at most `limit`, with the first line of the trace and the `call` events of their frames, so that every value decodes, and `next`, the `from` of the next page, or `null`. `node: \"raw\"` gives what `raw` bound in each iteration.",
            params: object(
                &[
                    ("run", string(run)),
                    ("function", string("The last trace of this function, in place of `run`.")),
                    ("folder", string("The folder `function` was run from; the root by default.")),
                    ("frame", integer("Only the events of this frame.", 0, u64::from(u32::MAX), None)),
                    ("node", string("Only the events of the nodes of this name.")),
                    (
                        "event",
                        json!({ "type": "string", "description": "Only the events of this kind.", "enum": ["call", "ran", "skipped", "arm", "set", "end"] }),
                    ),
                    ("from", integer("The number of the first event.", 1, u64::MAX >> 11, Some(1))),
                    ("limit", integer("How many events, at most 2,000.", 1, 2000, Some(500))),
                ],
                &[],
            ),
            result: "a small object: `file`, `header`, `calls`, `events`, `next`, and `truncated` when the page was cut",
            errors: RUN_ERRORS,
            effects: &[],
            long: false,
            planned: None,
        },
        // 6.5 Diagrams and the viewer
        Command {
            id: "diagram.svg",
            label: "Draw Diagram",
            summary: "Draw functions of a folder as SVG, with the report of each drawing",
            description: "The SVG of each function asked for, the same as `labcraft-cli diagram --svg` gives, in the field `image` of its entry, and the report `labcraft-diagram/1`, so that an agent sees the drawing of what it wrote. Past the bound of a result, the SVGs give way from the last function, each as `{ \"$cut\": true }`, and the result says `truncated`: ask for one function at a time, or use `labcraft-cli diagram`, which writes files.",
            params: object(
                &[
                    ("folder", string("A library folder of the project.")),
                    (
                        "functions",
                        json!({ "type": "array", "items": { "type": "string" }, "description": "The functions to draw; all of them by default." }),
                    ),
                    ("theme", one_of("One theme only, instead of both.", &["light", "dark", "both"], "both")),
                    ("max_diagram_nodes", integer("A function with more nodes is not drawn.", 1, 100_000, Some(20_000))),
                ],
                &["folder"],
            ),
            result: "labcraft-diagram/1",
            errors: &["params.unknown", "params.missing", "params.invalid", "path.outside_project", "path.not_found", "path.invalid", "internal"],
            effects: &[],
            long: false,
            planned: None,
        },
        Command {
            id: "view.open",
            label: "Open Folder",
            summary: "Open a library folder in the viewer",
            description: "The functions of the folder, each with its file and whether it is drawable, and the report of `check`. The viewer shows it in place of the folder open before, and watches its files. Libraries are looked for as `library.check` looks for them.",
            params: object(&[("folder", string("A library folder of the project: `.` for its root."))], &["folder"]),
            result: "a small object: `functions`, and labcraft-check/1",
            errors: &["params.unknown", "params.missing", "params.invalid", "path.outside_project", "path.not_found", "path.invalid", "internal"],
            effects: &[],
            long: false,
            planned: None,
        },
        Command {
            id: "view.show_function",
            label: "Show Function",
            summary: "Draw a function of the open folder in the viewer",
            description: "Draws it in the viewer, as its file gives it, and returns its entry of `diagrams` in `labcraft-diagram/1`.",
            params: object(&[("function", string("A function of the open folder."))], &["function"]),
            result: "an entry of labcraft-diagram/1",
            errors: CHECK_ERRORS,
            effects: &[],
            long: false,
            planned: None,
        },
        Command {
            id: "view.set_standard_view",
            label: "Standard View Everywhere",
            summary: "Show every node in standard view, or not",
            description: "The preference of the IDE that shows every node in standard view, kept with the preferences; returns the value it had, as `{ \"before\": false }`. Changes no file.",
            params: object(&[("on", json!({ "type": "boolean", "description": "Every node in standard view." }))], &["on"]),
            result: "a small object: `before`, the value it had",
            errors: CHECK_ERRORS,
            effects: &[],
            long: false,
            planned: None,
        },
        Command {
            id: "view.set_state",
            label: "Set View State",
            summary: "Change the view state of a function that its file does not hold",
            description: "The tab shown of a structure, a construction folded or not, a node in standard view or not, the iteration chosen of a loop, by the key of each node; returns the drawing's entry of `diagrams`.",
            params: object(
                &[
                    ("function", string("A function of the open folder.")),
                    ("state", json!({ "type": "object", "description": "`arms`, `unfolded`, `standard`, each optional." })),
                ],
                &["function", "state"],
            ),
            result: "an entry of labcraft-diagram/1",
            errors: CHECK_ERRORS,
            effects: &[],
            long: false,
            planned: None,
        },
        Command {
            id: "view.open_trace",
            label: "Open Trace",
            summary: "Show the values of a run on the function shown",
            description: "Reads a trace (`labcraft-trace/1`) and shows its values on the wires of the function shown, until another function is shown; without `file`, those of its last run again. Returns the `trace` part of `labcraft-diagram/1`, or null when the function has no trace.",
            params: object(&[("file", string("A trace file of the project; without it, the last run of the function."))], &[]),
            result: "the trace part of labcraft-diagram/1, or null",
            errors: TRACE_ERRORS,
            effects: &[],
            long: false,
            planned: None,
        },
        // 6.6 The registry itself
        Command {
            id: "commands.list",
            label: "List Commands",
            summary: "List the commands, whether each is enabled, and why not",
            description: "Each command with its id, summary, effects, whether it is long, whether it is enabled now, and why not. `commands.describe` gives one in full.",
            params: object(&[("filter", string("Only the ids that contain this text."))], &[]),
            result: "a small object: `commands`",
            errors: &["params.unknown", "params.invalid", "internal"],
            effects: &[],
            long: false,
            planned: None,
        },
        Command {
            id: "commands.describe",
            label: "Describe Command",
            summary: "Describe a command in full: its parameters, result, errors and effects",
            description: "Its full description: id, label, summary, description, the schema of its parameters, its result, its errors, its effects, whether it is long, and when a command is planned, what brings it.",
            params: object(&[("id", string("The id of a command: `run.start`."))], &["id"]),
            result: "a small object: the command, and `enabled`",
            errors: &["params.unknown", "params.missing", "params.invalid", "command.unknown", "internal"],
            effects: &[],
            long: false,
            planned: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn ids_are_unique_and_give_distinct_tool_names() {
        let mut ids = HashSet::new();
        let mut tools = HashSet::new();
        for command in commands() {
            assert!(ids.insert(command.id), "{} twice", command.id);
            assert!(tools.insert(command.id.replace('.', "_")), "{} gives the tool name of another", command.id);
        }
    }

    #[test]
    fn each_summary_is_one_short_line_and_each_parameter_is_described() {
        for command in commands() {
            assert!(command.summary.chars().count() < 100 && !command.summary.contains('\n'), "{}", command.id);
            for (name, schema) in &command.params.properties {
                assert!(schema["description"].is_string(), "{}: {name} has no description", command.id);
            }
            for required in command.required() {
                assert!(command.parameters().contains(required), "{}: {required}", command.id);
            }
        }
    }
}
