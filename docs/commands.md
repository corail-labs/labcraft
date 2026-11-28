# Commands: the catalog v0

*Last updated: 2026-11-19. Status: implemented for milestone M0b (tag `v0.2.0-m0b`), in `engine`. The commands
of section 6.5, implemented for M0v (tag `v0.3.0-m0`), are in `engine::view` and `engine::drawings` (`docs/architecture.md`, 10.8 and 11.5).*

Everything LabCraft does is a command (D-18, [Craft conventions](craft-ecosystem.md)): the MCP server,
the IDE, scripts and plugins call the same ones, with the same parameters, results and errors. This
document is their contract: how a command is named and described, how it is called, what it returns,
and the commands of M0b. The MCP server that exposes them is in [mcp.md](mcp.md); the command line,
which calls the same code, in [cli.md](cli.md). Values marked *proposed* are starting points.

The reader that matters most here is an agent. It reads the description of a command once, calls it
with what it understood, and has to fix its call from the error alone. So every rule below is judged
on that: few commands, results that say what to do next, errors that name the parameter at fault.

## 1. What a command is

A command lives in the registry of `engine` as data, the same data that `commands.describe` returns:

| Field | Content |
|---|---|
| `id` | `domain.verb`, lower case, words joined by `_`: `file.write`, `run.start`, `run.set_element`. The domain is what it acts on, the verb what it does; a domain can have parts (`view.zoom.fit`), as in SolidCraft (`sketch.rectangle.two_point`). An id keeps its meaning forever, like a diagnostic code |
| `label` | The name a person reads in a menu or a palette: "Start Run". Translated in the interface (D-21); the id never is |
| `summary` | One line, under 100 characters, imperative: "Write a file of the project, then check its library" |
| `description` | What it does, its defaults, and what to call next. English, for an agent as much as for a person |
| `params` | A JSON Schema (2020-12) of an object, in the part that every MCP client reads: `type`, `properties`, `required`, `items`, `additionalProperties`, `default`, `enum`, `minimum`, `maximum`, and `description` on each property. `args`, whose keys are the inputs of a function, is an object with `additionalProperties`; `set` is an array with `items` |
| `result` | The name of what it returns: a schema of this document (`labcraft-run/1`) or of [cli.md](cli.md) (`labcraft-check/1`), or a small object described with the command |
| `errors` | The error codes it can return (section 3) |
| `effects` | What it can change, among `files` (writes in the project), `hardware` (can reach real hardware, or files outside the project, with the consent of [io.md](io.md) 4; named `instruments` until 0.5.0) and `run` (starts, steers or stops a run). A command with none only reads |
| `long` | `true` when it can outlast a call: it returns a task to follow (section 4) |
| `planned` | Present when the command is registered but not built yet: the milestone or the issue that brings it. Such a command is never enabled, and says why |

From M1 the registry also holds, beside this data, what a caller does with a command: `undoable` (it makes a step of
history, 9.1), `direct` (it is a tool of its own, [mcp.md](mcp.md) 2), `at_once` (the MCP server answers it outside its
queue) and `window` (a command of the window, 6.7) ([architecture.md](architecture.md), 12.5).

`enabled`, as in the Craft apps, is a state, not a field of the description: `commands.list` gives it
with each command, and a call to a command that is not enabled gets `command.not_enabled` and the
reason (`run.set_element` in deterministic mode, `view.zoom` before M0v). A planned command lets a milestone
register its commands before it builds them, as SolidCraft does with `planned!`: the catalog, the menus
and the agent see them, greyed, with the reason.

`cargo xtask commands` checks the registry against the tables of section 6, as `cargo xtask codes`
checks diagnostic codes against cli.md *(proposed)*: the ids, the parameters of each in order, whether each is
required, and their defaults, both ways: every default of the registry is written in the row as a value in code or
a number (`` (`false`) ``, `` (`30 s`) ``, `(20,000)`), and every default written so is the registry's. A default
in prose ("the root") is one the command works out, and the registry has none. A cell that starts in prose gives its
parameters after its first colon, outside code and parentheses, that a parameter follows, and is an error without
them. It also refuses two ids that would give one MCP tool
name, `a.b_c` and `a_b.c` ([mcp.md](mcp.md), section 2).

## 2. Calling a command

- **Parameters are a JSON object.** A missing parameter takes its default, or is an error if it has
  none: a command never asks a question (no dialog on a programmatic call; the one exception, the consent of the
  user to an agent attached to the IDE, is in [ide.md](ide.md), 8). An unknown parameter is an
  error, not ignored, since a misspelt `max_iteration` would otherwise run with the default and the
  agent would never know.
- **Paths** are relative to the root of the project ([mcp.md](mcp.md), section 1), written with `/`
  on every platform, as in the reports of cli.md. `..` may not leave the root, an absolute path must be
  inside it, and a symbolic link is not followed out of it: each is `path.outside_project`. Paths are
  resolved before they are compared, and a link counts by its target. The libraries shipped with
  LabCraft are read as `<labcraft>/visa`, and are never written.
- **Hidden files are out of reach**: a file or folder whose name starts with `.` (`.git`, `.env`) is
  `path.outside_project`, except `.labcraft/`, which LabCraft writes (section 8). The components `.` and
  `..` of a path are not names, and fall under the rule above.
- **`.toml` files** are reached only as simulation files: an existing one is read, written, renamed or
  deleted only if it reads as a simulation file, and a new one must read as one. It reads as one when
  it is TOML and every key at its top level names a library that simulates (in M0, `visa`:
  [libs/visa/simulation.md](../libs/visa/simulation.md)), so an empty file,
  as `no-instrument.toml` may be, is one. The name `labcraft.toml` is refused whatever it holds, since it
  names the manifest of a library. So `Cargo.toml`, `pyproject.toml` and the manifests of the shipped
  libraries are out of reach, and so is a user library's manifest when M2 brings it. One command writes a part of
  it, the `[mask]` table: `library.set_mask` (6.7, M1).
- **On every system**, these are refused before any call to the file system, as SolidCraft does on Windows, then
  checked again once the path is resolved, so that a link or a junction cannot lead to them, and a project behaves the
  same wherever it is opened: UNC and device paths
  in any spelling (`\\server\share`, `//server/share`, `\\?\UNC\…`, `\\.\…`, `\??\…`), a drive-relative
  path (`C:file`), a `:` in a name (a stream), a name that Windows reserves with any extension (`NUL`,
  `com1.lcf`, as `LC2025` does for a program), a short 8.3 name (`PROGRA~1`). Each is `path.invalid`.
- **A hard link** cannot be told apart from an ordinary file, so one that leads out of the project is
  reachable through it. This is a limit, as in SolidCraft.
- **A result is a JSON object.** A report carries its `schema`, and a reader ignores the fields it does
  not know (cli.md, section 4.4).
- **A result is bounded**, whatever the project: at most 4 MiB of JSON *(proposed)*. Each list that can
  grow with the project or the run (diagnostics, files, functions, resources and the commands each one
  received, events of an inline trace) has its own bound, given with the command, and says `truncated`
  when it is reached; values are cut as section 5 says. If the result would still pass 4 MiB, what helps
  the reader least gives way first, until it fits: the SVGs of `diagram.svg` (6.5), then the events of an
  inline trace, then the commands seen by instruments, then the diagnostics (warnings before errors), then the values of the outputs, each shown
  as `{ "$cut": true }`. Each list shortened says `truncated`, and the result carries `"truncated": true`
  at its top level. In M0b, `engine` cuts what still passes the bound once each command has bounded its own lists, as
  section 5 cuts a value, what comes first in the result staying. The message of an error, which can repeat a path or
  an id the caller sent, is cut past 200 code points, as a message of a diagnostic is (cli.md 4.2). A path of a command
  holds at most 64 names and 4,096 bytes *(proposed)*, `path.invalid` otherwise. The status, the run id, the error, the fault and the counts are never cut. MCP sends a
  result twice ([mcp.md](mcp.md), section 3), so a reply on the wire stays under about three times the
  bound, quoting included.
- **A program with errors is not a failed command.** `library.check` on a library that does not check
  succeeds, and returns a report whose `status` is `error`. A command fails only when it could not do
  what it was asked: a bad parameter, a path that does not exist, a run that is not there.

## 3. Errors of a command

A failed command returns one object:

```json
{
  "error": {
    "code": "params.unknown",
    "message": "unknown parameter `max_iteration`: did you mean `max_iterations`?",
    "parameter": "max_iteration",
    "expected": ["function", "folder", "sim", "args", "set", "mode", "schedule_seed", "timeout",
                 "max_iterations", "max_nodes", "max_call_depth", "max_memory", "max_made",
                 "max_real_time", "deny_warnings", "trace", "wait"]
  }
}
```

`code` and `message` are always there; `parameter` names the parameter at fault, `expected` lists what
would have been accepted, when that list is short. As for diagnostics, a tool decides on `code`, never on
the text of `message`. These codes are strings, not `LC` codes: they describe a call, not a program.

| Code | When |
|---|---|
| `command.unknown` | No command has this id. `expected` gives the nearest ids |
| `command.not_enabled` | The command exists but cannot run now; `message` says why |
| `params.unknown` | A parameter the command does not have |
| `params.missing` | A required parameter is absent |
| `params.invalid` | A parameter of the wrong JSON type, out of its range, or not one of its values |
| `path.outside_project` | A path that leaves the root of the project, or a shipped library to write |
| `path.not_found` | A file or folder that does not exist |
| `path.invalid` | A path the system cannot hold, or a file name that `check` would refuse (`LC2024`, `LC2025`), or an extension the command does not take |
| `file.unreadable`, `file.unwritable` | The system refused; `message` holds its error, in English |
| `file.too_large` | A file over `--max-file-size` (cli.md, section 1), read or about to be written |
| `file.not_simulation` | A `.toml` file that does not read as a simulation file, to write, rename or delete (section 2) |
| `run.unknown` | A run id that this session does not know, or no longer keeps, or no run at all |
| `run.busy` | `run.start` while another run of this session is going (section 4); `run` gives its id |
| `file.stale` | *(M1)* An editing command whose `expected_revision` is not the revision of the file (section 9); `message` gives the current one. Also `history.undo` or `history.redo` when a file of the step changed since: the step is not replayed over that change |
| `file.unparsed` | *(M1)* An editing command on a file whose text does not parse (9.2) |
| `path.exists` | *(M1)* A new function or type in a file that already exists; a new project in a folder that is not empty |
| `edit.unknown` | *(M1)* A node, value, wire, terminal, structure or arm that the function does not have (9.2); `expected` gives the nearest names |
| `edit.ambiguous` | *(M1)* A name that names several things, such as a name defined in two arms; `expected` lists them |
| `name.taken`, `name.invalid` | *(M1)* A new name that the function already binds, or that only a wire gives (notation 2.4), or that is the name of a library that the command imports; an input or an output that bears the name of a library that the command imports (9.2); a name that is not an identifier, or is a keyword |
| `wire.occupied` | *(M1)* An input that is already wired or that holds a constant, without `replace` (6.7); `message` says what is there |
| `wire.type_mismatch` | *(M1)* A value or a constant of a type that the input does not take, without `accept_mismatch`; `message` gives both types |
| `wire.cycle`, `wire.out_of_scope` | *(M1)* A wire that would close a cycle; a wire from a value that the input cannot see (6.7) |
| `history.empty`, `history.not_yours` | *(M1)* Nothing to undo or redo; a step that an agent may not undo, since it is not its own (9.5) |
| `gesture.busy` | *(M1)* An editing command of an agent waited 30 s *(proposed)* for the end of a gesture of the user, and changed nothing (9.1) |
| `agent.not_allowed` | *(M1)* A command of an agent attached to the IDE that its policy does not allow, that the user refused or did not answer in time, or that only the window calls ([ide.md](ide.md), 8) |
| `internal` | A bug of LabCraft, caught: `message` says where. The session goes on |

A request too large to read is not a command error: no command was reached ([mcp.md](mcp.md), section 5).

## 4. Long commands: runs

A run can last for days on a bench, or a few milliseconds of real time in deterministic mode. It follows
the task model of SolidCraft (`render.start`, `render.status`, `render.cancel`), with one change for
agents: **`run.start` waits**. It returns the report of the run if the run ends within `wait` (30 s by
default, at most 300 s *proposed*), which is what nearly every deterministic run does, so that one call
runs a program and reads its result. Otherwise it returns, here for a run of `two_benches` whose `count` and
`period` keep it going past the wait,

```json
{
  "schema": "labcraft-run/1", "status": "running", "run": "r3", "function": "two_benches",
  "mode": "real_time", "outputs": [], "clock": { "real_ms": 30000 },
  "resources": { "opened": 2, "open": [], "leaked": [] },
  "loops": [ { "function": "two_benches", "node": "n_a", "iteration": 4 },
             { "function": "two_benches", "node": "n_b", "iteration": 4 } ],
  "check": {
    "summary": { "libraries": 1, "functions": 2, "types": 0, "errors": 0, "warnings": 0, "truncated": false },
    "diagnostics": []
  }
}
```

and the run goes on. `run.wait` waits again and returns the same thing; `run.status` returns at once;
`run.set_element` sets a panel element of the running function (Q-20, [cli.md](cli.md), section 6.4);
`run.abort` aborts it (spec 8.2) and returns its final report.

- **The default wait stays under the timeouts of clients.** Many MCP clients give up on a request after
  60 s, and a cancelled request gets no answer, so a wait as long as that would lose the id of the run.
  A client that allows longer calls may ask for a longer `wait`.
- **The run id is optional.** `run.wait`, `run.status`, `run.set_element` and `run.abort` act on the run
  of the session when `run` is not given, so an agent that lost the id can still follow or stop its run.
  With no run going, `run.wait`, `run.status` and `run.abort` return the final report of the last run,
  with the status it ended with, and give `run.unknown` if there was none; `run.set_element` gives
  `run.unknown`.
- **One run at a time per session** *(proposed)*: a second `run.start` gets `run.busy`, which gives the id
  of the run going. A simulation belongs to its run, so two runs never share an instrument, but one at a
  time keeps reports, traces and the panel unambiguous for whoever reads them.
- **A run id** (`r1`, `r2`…) is given by the session. The session keeps the reports of its last 16 runs
  *(proposed)*; an older id gets `run.unknown`.
- **A run never outlives its session.** When the MCP client goes away, its run is aborted.

## 5. Values in JSON

Arguments, outputs, panel elements and the trace write values the same way. A value is read and written
**against its type**, which the report gives next to it and the trace with each frame (section 8), so the
encoding stays short. The keys of every object of a result, a report or a trace come in the order the tool writes them:
the fields of a struct in their declared order, a report in the order of section 7, a result in the order of its
example, the same through `--json` and MCP (decision of the owner, 2026-11-11):

| Type | JSON | Example |
|---|---|---|
| `bool` | a boolean | `true` |
| integers | a number, exact; an `i64` or `u64` beyond ±(2^53−1) as a string of digits | `12`, `"9007199254740993"` |
| `f32`, `f64` without a unit | a number: the fewest digits that read back the same, in decimal notation from 10^-5 up to below 10^16 and with an exponent outside, written as the language writes it (`1.5e-7`, `1e16`); between two texts as short and as near, the one whose last digit is even, as Python writes them (`1658206780088562.2`); a float with no fraction keeps `.0`, so that it is not taken for an integer; `"nan"`, `"inf"`, `"-inf"` | `0.5`, `12.0`, `0.0015` |
| a number with a unit | a string, as the language writes a constant, in the unit of the type: the number as above, a space, the unit | `"230.03 V"`, `"-0.0015 A"`, `"-3.0 mV"` (an `f64[mV]`; the same value in an `f64[V]` is `"-0.003 V"`) |
| `duration` | a string, written in seconds, exactly from its nanoseconds; read in any unit of time | `"0.1 s"`, `"2 s"`; `"100 ms"` as an argument |
| `c64`, `c128` | `{ "re": …, "im": … }`, each as above | `{ "re": "1.0 V", "im": "0.0 V" }` |
| `string` | a string | `"MEAS:VOLT:DC?"` |
| `bytes` | `{ "base64": "…" }` | |
| `timestamp` | a string, RFC 3339, UTC, to the nanosecond. In deterministic mode, a time of the simulated clock, which starts at 1970-01-01T00:00:00Z (Q-21, spec 7.2) | `"2026-11-08T12:00:00.000000250Z"` in real time, `"1970-01-01T00:00:00.100000000Z"` 100 ms into a deterministic run |
| array | an array; an `[f64; 2]` is an array of rows | `[1, 2]`, `[[1, 2], [3, 4]]` |
| struct, `waveform` | an object, fields in declared order | `{ "t0": …, "dt": "0.001 s", "samples": […] }` |
| enumeration | a variant without values as a string, `Type.Variant`; with values, an object of one key | `"Verdict.Pass"`, `{ "Reading.Value": ["230.0 V"] }` |
| `error` | `null` for `none`; otherwise an object (spec 7.1) | below |
| opaque, function reference | an object that names it, read-only; the default of an opaque type designates nothing, and that of a function type no function (Q-21) | `{ "opaque": "visa.session", "resource": "SIM::DMM0", "open": false }`, `{ "opaque": "visa.session", "open": false }`, `{ "function": "measure" }`, `{ "function": "core.add" }`, `{ "function": null }` |

```json
{
  "category": "io.timeout", "code": -1073807339,
  "message": "no answer to `MEAS:VOLT:DC?` within 2 s",
  "origin": { "function": "scpi.query_f64", "node": "text", "file": "<labcraft>/scpi/query_f64.lcf", "line": 12,
              "callers": [ { "function": "check_voltage", "node": "dmm2", "file": "check_voltage.lcf", "line": 15 } ] },
  "others": []
}
```

`origin` is the node that made the error, always in a function written in LabCraft: the call of the primitive or of
the library function that failed (`text`, the `visa.query` inside `scpi.query_f64`, not `visa.query` itself). It gives
its `function`, its `node`, named as in section 8, its `file`, a path as reports write it (`<labcraft>/…` for a shipped
library), and the `line` where its statement starts. `callers` are the nodes of the calls that led there, innermost
first, up to the top-level function, each the same way, and `[]` for an error made in the top-level function; they are
cut as any list (section 2).

`others` is flat: the errors that `errors.merge` gathered, each without its own `others`, however they
were merged, at most 32 *(proposed)*, then `others_more` gives how many are left out.

- **An argument** takes the same forms, and a little more, since it is converted as a literal of the
  language is (spec 3.4, literals; cli.md, section 6.3): `"230 V"`, `"230V"` or `"0.23 kV"` for an
  `f64[V]`, `2` for an `f64`, `0.5` for an `f32`, `"Verdict.Pass"` or `"Pass"` for a `Verdict`. A number
  with no unit is not accepted for a type with a unit or a `duration`: the unit is what the run cannot
  guess. An argument that does not convert is a diagnostic of the run, with the name of the input
  (section 7).
- **Durations given to a command** (`wait`, `timeout`, the times of `set`) take the form of a `duration`:
  `"30 s"`, `"500 ms"`.
- **Order.** Objects and lists keep the order of the program: inputs and outputs in the order of the
  signature, the values of a node in the order of its outputs, the types of a frame in the order of the
  lines, resources in the order they were opened, loops in the global order of spec 8, whatever
  `schedule_seed`, so that each loop comes before the loops inside it. Resources are in code point order
  of their names (section 7). So two runs of one program write the same text, apart from the clock of real
  time.
- **A value too large for a report** is cut in the report, never in the program *(proposed figures)*:
  - an array of more than 64 elements shows `{ "$len": 100000, "$head": [ …16 elements… ] }`, at every
    level of nesting;
  - a string over 1,000 code points shows `{ "$len": 4000, "$head": "…" }`, so that a string that ends
    with `…` is never taken for a cut one; `bytes` over 1,000 bytes show
    `{ "$len": 4000, "$head": { "base64": "…" } }`, the first 1,000 bytes;
  - beyond that, **each value has a budget** of 64 KiB of JSON, and a depth of 64 levels: once either is
    spent, what is left of it shows as `{ "$cut": true }`: in an array, in the place of the elements left; in an
    object, as the value of the field that does not fit, or, when its name does not fit either, as the member
    `"$cut": true` that ends the object. A level is an array or an object, the
    two of `$len` and `$head` and the mark of a cut included. A struct, deep arrays or many strings cannot pass it,
    and a client's JSON reader never meets a document deeper than it reads.

  A name of the language never starts with `$`, so an object whose keys start with `$` is a cut, never a
  struct. The trace cuts the same way.

## 6. The catalog of M0b

What an agent needs to write, check, run and fix a program, and nothing it does not. In M0b each command was an
MCP tool; from M1 a command marked `direct` is a tool of its own, and the MCP server reaches the others through
`commands_execute` ([mcp.md](mcp.md), section 2). Parameters in *italics* are optional.

### 6.1 Learning the language

| Command | Parameters | Returns |
|---|---|---|
| `guide.read` | *`skill`*, *`file`* (`SKILL.md`) | The skills of `skills/`, from the copies built into LabCraft: without `skill`, each skill with its name and the line that says when to use it, followed by the `SKILL.md` of the skill that writes a program, so that one call starts the loop; with one, that file of it, in Markdown. The same files that a client which supports skills installs, so there is one text to keep right. Never reads a path from disk; an unknown skill or file is `params.invalid`, with the choices in `expected`. No skill holds the programs of the acceptance scenarios |
| `functions.list` | *`library`*, *`filter`*, *`docs`* (`false`) | Every function and type the project can call: primitives (`core`, `string`, `array`, `errors`, `time`, `ui`, `bytes`), shipped libraries (`visa`, `scpi`), and the project's own. Each as a signature written in the language, as appendix A writes it (`fn visa.open(resource: string, timeout: duration = 2 s, err: error = none) -> (session: visa.session, err: error)`), with `always`, its error categories, and the first line of its documentation, or all of it with `docs`. `filter` keeps the names that contain it |

`guide.read` returns, without `skill`, `{ "skills": [{ "name", "description", "files" }], "skill", "file", "text" }`:
each skill with the `description` of its `SKILL.md` and the paths of its files, then the `SKILL.md` of
`labcraft-writing`; with `skill`, `{ "skill", "file", "text" }`. The schema of `skill` lists the skills (`enum`), so
an agent sees them before its first call. The text is the file as `skills/` holds it, front matter included.

`functions.list` answers "what can I call" in one call: an agent writing its first program calls it
once, with no filter. The project's own entries come from the last `check`, so they agree with it. It
lists at most 1,000 entries *(proposed)*, primitives and shipped libraries first, then says `truncated`:
`library` or `filter` narrow it. Each entry is `{ "name", "library", "kind", "signature" }`, `kind` being
`function` or `type`, with `always`, `categories` and `doc` when they have something to say, each line of `doc` cut
past 200 code points, as a message is; `library` is `""` for the root, whose functions are named without one; a type of a
library is named by it in the signatures (`session: visa.session`), as a program that uses it writes it. The
categories of a function of a library are those its manifest declares for the library, which say no more.

Beside its signature, which stays the text to read, each entry gives its types as data, so that a client never reads
the signature again: a function its `inputs` and `outputs`; a type its `fields` (a struct), its `variants` (an enum,
each `{ "name", "values" }`) or `is` (an alias, what it names); an opaque type none of them. Each input, output, field
and value is `{ "name", "type", "fit" }`, `name` absent for a value of a variant, `type` the type as the signature
writes it, and `fit` what the rule of fit (spec 3.3, 3.4) judges it by, which holds outside a check:

- `{ "number": "f64", "unit": "mV" }`: a number, its kind and its unit, `unit` absent for a number without one;
- `{ "base": "duration" }`: `bool`, `string`, `bytes`, `timestamp`, `duration` or `error`;
- `{ "variable": "N", "bound": "Number" }`: a type variable of the signature, `bound` absent when it has none;
- `{ "numbers": ["A", "B"] }`: a number that the call works out from the type variables or the parameters it names
  (`A·B`, `√N`, `f64[unit]`);
- `{ "array": { "variable": "T" }, "dimensions": 2 }`: an array of a type variable, `dimensions` absent when the type
  does not write them; any other array is a `shape`, and so is one in more than 16 arrays *(proposed)*;
- `{ "shape": "visa.session" }`: any other type, by its text; two such types match when their texts are equal, a `…`
  matching any part (`fn(…) -> (…)`, the input of `core.call`, takes any function). An alias
  is followed to what it names, across libraries: an output of type `psu.session`, an alias of `visa.session`, has the
  fit `{ "shape": "visa.session" }`, and one of an alias of `f64[mV]` the fit of that number.

An input also says, only when it is true, `required` (no default, and not variadic), `variadic`, `named_only` (a
default after a variadic parameter, wired by name only, R-11) and `literal` (it takes only text written in the call,
never a wire: a `kind` parameter, the template of `string.format`); a `literal` input has no `fit`. What a signature
writes `…` (the inputs that the template of `string.format` adds, the outputs of `core.call`) is not listed. The
typed parts of one entry hold about 64 KiB at most *(proposed)*, each type cut as a signature is: past that, the
rest is left out and the entry says `truncated`. So `visa.open` is:

```json
{
  "name": "visa.open",
  "library": "visa",
  "kind": "function",
  "signature": "fn visa.open(resource: string, timeout: duration = 2 s, err: error = none) -> (session: visa.session, err: error)",
  "inputs": [
    { "name": "resource", "type": "string", "fit": { "base": "string" }, "required": true },
    { "name": "timeout", "type": "duration", "fit": { "base": "duration" } },
    { "name": "err", "type": "error", "fit": { "base": "error" } }
  ],
  "outputs": [
    { "name": "session", "type": "visa.session", "fit": { "shape": "visa.session" } },
    { "name": "err", "type": "error", "fit": { "base": "error" } }
  ],
  "categories": ["io.timeout", "visa.invalid_session", "visa.resource_not_found"],
  "doc": "Opens a session on a resource (`SIM::…`: a simulated instrument)."
}
```

### 6.2 Files and libraries

| Command | Parameters | Returns |
|---|---|---|
| `project.describe` | | The root, its libraries (folders, sub-libraries), their `.lcf` and `.lct` files with what each declares, the simulation files with the resources they define (those of the project, and those shipped with `visa`, as `<labcraft>/visa/sim/…`), and the summary of a `check` of the root (cli.md, section 4.2), without its diagnostics: `{ "root", "libraries": [{ "library", "folder", "files": [{ "file", "declares" }] }], "simulations": [{ "file", "resources", "valid" }], "summary", "truncated" }`, the root being the library `""` in the folder `.`. At most 1,000 files listed *(proposed)*, and at most 8 MiB of `.toml` files read to find the simulation files *(proposed)*, those over `--max-file-size` not counted, then `truncated` |
| `file.read` | `path`, *`from`* (1), *`lines`* (all) | The text of a `.lcf`, `.lct` or simulation file of the project or of a shipped library, from line `from`: `{ "file": …, "text": …, "from": 1, "lines": 120, "lines_total": 120, "truncated": false }`. At most 1 MiB of text per call *(proposed)*, counted as JSON writes it, cut at the end of a line, then `truncated`: the next call starts at `from + lines`. A line longer than that is cut at the end of a character, and counts as read |
| `file.write` | `path`, `text`, *`check`* (`true`) | Writes the file, atomically, creating its folder if needed. With `check`, then checks the root and returns its report (`labcraft-check/1`), as `library.check` would: one call writes and checks. `text` holds at most 4 MiB *(proposed)*, whatever `--max-file-size`, so that a request stays within the bound of [mcp.md](mcp.md), section 5; a larger file is `file.too_large`. A step of history (9.1), signed by its caller, which undo reverses; none when the bytes are those of the file, or when the file it replaces is over `--max-file-size`, since the history cannot keep it |
| `file.rename` | `from`, `to` | Renames a file in the project, a change of case alone included (R-22: the fix of `LC2023` can be a new name), then checks as `file.write` does. A final `/` or `.` of `to` names no file: `b.lcf/` is `b.lcf`. A step of history (9.1): the file is removed under its old name and made under the new one |
| `file.delete` | `path` | Deletes a `.lcf`, `.lct` or simulation file of the project, then checks. A step of history (9.1), which undo reverses; none for a file over `--max-file-size`, which the history cannot keep |
| `library.check` | *`folder`* (the root), *`deny_warnings`* (`false`) | `labcraft-check/1`, exactly what `labcraft-cli check --json` gives (cli.md, section 4.2) |
| `file.format` | `path` (a file or a folder), *`check_only`* (`false`) | `labcraft-fmt/1`, exactly what `labcraft-cli fmt --json` gives. The files it rewrites are one step of history (9.1) |

`file.write` writes the text as given, but for two changes: CR LF becomes LF, as the repository keeps
its programs (cli.md, section 1 reads both), and the entries of `layout` of a node that the text renamed take its new
name (spec 11, 9.1), which an agent never writes. It does not reformat, so that the next edit of an agent finds
the file it wrote; `file.format` puts it in canonical form when asked. It checks the root, and not the
library of the file alone, since `use` is resolved from the folder that `check` is given (cli.md,
section 3): the report is the one `library.check` gives. Writing only programs and simulation files keeps
an agent inside what LabCraft knows; other files are written by the client's own tools, if it has any.

**The session keeps its last `check`** *(proposed)*, so that a command that checks answers quickly
whatever the size of the project, well under the timeouts of clients ([mcp.md](mcp.md), section 1):

- When the session starts, it checks the root in the background and keeps the result.
- Before a command that reports a check, the session reads the files again and compares them with those the check
  read (their content, and the files each folder holds), so that a file written by the client's own tools is seen,
  even one rewritten with the same size within one tick of the clock of the file system. When nothing changed, the
  kept report is given at once; when something did, the folder is checked again, whole
  (decision of the owner, 2026-11-10: `examples/` takes 45 ms, 21,000 files 2.7 s). Checking again only the
  library of a change and those that use it is for later, when a measure asks for it.
- The report is always the one a full `check` of the root would give: keeping work is a speed, never a
  different answer. A command that needs the first check before it has ended waits for it, except
  `project.describe`, whose summary then says `"pending": true`, with no file saying what it declares.

How the session keeps its checks is written in the architecture of M0b (section 10.8).

### 6.3 Instruments

| Command | Parameters | Returns |
|---|---|---|
| `simulation.describe` | `path` | What a simulation file of the project or of `<labcraft>/visa/sim/` defines, part by part, `{ "file": …, "parts": [{ "library": "visa", "description": … }] }`: for each library that simulates, the description that library gives of its part (for `visa`, each instrument with its resource, its `*IDN?` answer, its latency, the queries it answers and what each one reads, the writes it accepts, the faults it injects: [libs/visa/simulation.md](../libs/visa/simulation.md), section 9); or `{ "file": …, "diagnostics": […] }`, the diagnostics of each library. A file that is not a simulation file at all (not TOML, nested too deep, a key that no library claims: `LC8109`, `LC0007`, `LC8110`) is `file.not_simulation` by the rule of section 2, whose message gives that diagnostic; one over `--max-file-size` is `file.too_large` |

### 6.4 Runs

| Command | Parameters | Returns |
|---|---|---|
| `run.start` | `function`, *`folder`* (the root), *`sim`*, *`args`* (`{}`), *`set`* (`[]`), *`mode`* (`deterministic`), *`schedule_seed`*, *`timeout`*, *`max_iterations`*, *`max_nodes`*, *`max_call_depth`* (1,024), *`max_memory`* (1,073,741,824), *`max_made`*, *`max_real_time`*, *`deny_warnings`* (`false`), *`trace`* (`file`), *`wait`* (`30 s`) | Checks the library, then runs the function and returns `labcraft-run/1` (section 7), or `running` (section 4). `function` is named from the root (`check_voltage`, `dmm_driver.measure`); `args` by input name (section 5), at most `--max-file-size` bytes written as JSON, as a file of `--args` (cli.md
6.3), and `set` too; `sim`, a simulation file; `set`, panel elements and the time on the clock of the run at which each is set, `[{ "element": "stop_switch", "value": true, "at": "5 s" }]` (Q-20, cli.md, section 6.4); `schedule_seed`, the seed of the order among ready nodes (scenario 3); `trace`: `file` (the default, so that whoever watches the viewer sees the values of every run on the wires), `inline` or `none` (section 8) |
| `run.wait` | *`run`*, *`wait`* (`30 s`) | The final report, or `running` again |
| `run.status` | *`run`* | The report so far, at once: `running` with the clock and where each loop is, or the final report |
| `run.set_element` | *`run`*, `element`, `value` | Sets a panel element of the running top-level function (spec 10, Q-20), in real time only: in deterministic mode the time a call arrives would date the change, and a run would no longer replay, so a change is given to `run.start` by `set`, at its time on the simulated clock, and this command is not enabled. The element and its value follow the rules of `--set` (cli.md, section 6.4); the change is seen by the reads that start after it. The run is going, so a setting that `--set` would refuse is a command error, `params.invalid`, whose `message` gives the code of cli.md, section 6.6 (`LC8108: …`, `LC8104: …`), and the run goes on; so is a value that does not fit in the memory of the run (`LC8005: …`), which counts each value its `panel` keeps until it ends (decision of the owner, 2026-11-11). On a deterministic run, `command.not_enabled`. It returns once the run has taken the setting, between two nodes: its entry of `panel` (section 7), with `run`, as `{ "run": "r3", "element": "stop_switch", "value": true, "at_ms": 5012.25, "by": "run.set_element", "applied": true }`; a setting given as the run ends is never taken, and its entry says `"at_ms": null, "applied": false` (decision of the owner, 2026-11-11) |
| `run.abort` | *`run`* | Aborts the run (spec 8.2) and returns its final report, status `aborted`. A run that has already ended is left as it is, and its final report is returned with its own status (section 4) |
| `run.trace` | *`run`* or *`function`*, *`folder`* (the root), *`frame`*, *`node`*, *`event`*, *`from`* (1), *`limit`* (500) | Reads the trace written by a run (section 8), in pages: the events from number `from` that match the filters, at most `limit` (at most 2,000 *proposed*), with the first line of the trace, the `call` events of the frames they belong to, so that every value decodes, and `next`, the number to give as `from` for the next page, or `null`. `run` reads the trace of that run, or the one going; `function`, the last trace of that function run from `folder`; neither, the run of the session. `node` keeps the events of the nodes of that name, `frame` those of one frame, `event` those of one kind. A run with `trace: none`, or whose trace is no longer the last of its function, gives `run.unknown` with the reason |

`run.trace` is how an agent debugs a run without reading the whole trace, which can reach 256 MiB:
"what did `x` bind in each iteration" is one call with `node: "x"`. It reads the same file as the
viewer, and needs no file tool of the client.

The parameters of `run.start` are the options of `labcraft-cli run`, named in `snake_case`
(`--max-call-depth` is `max_call_depth`), with these differences: `--args FILE` and `--arg` give `args`, an object, and
`--set` gives `set`, an array; `--real-time` is `mode`; `--trace FILE` and `--no-trace` are `trace`,
which writes only under `.labcraft/runs/` and can also be `inline`; `wait` exists only for a call;
`--json`, `--lib-path` and the options of section 1 of cli.md are those of the server ([mcp.md](mcp.md),
section 1). `schedule_seed` is a number, or a string of digits beyond 2^53−1. Ranges and defaults are
written once, in [cli.md](cli.md), section 6.2.

**What `run.start` judges as the command line does.** An unknown function, a `folder` or a `sim` file
that does not exist or does not read, a `sim` that is not a simulation file (not TOML, `LC8109`; a key of no library,
`LC8110`; decision of the owner, 2026-11-10), an argument or a setting that does not convert, give the report of a
`refused` run with the diagnostic of cli.md, section 6.6, as `labcraft-cli run` does, not a command error:
an agent reads one answer, whichever way it calls. A call that breaks the other rules of section 2 (a path out of
the project, a hidden one, a form that Windows reads otherwise) is a command error, and so is what the command line
calls wrong usage (cli.md, section 6.2): a time of `set` below 0, an input name that is not a name, a
`schedule_seed` or a `max_real_time` with `mode: "real_time"` are `params.invalid`. `null` removes the
default of `max_iterations` and `max_nodes`, as `none` does on the command line.

### 6.5 Diagrams and the viewer

Enabled with M0v. None writes a
file; the `view.*` commands change only the state of the viewer, which the session holds. The files they read are those
of the session's root, and the libraries those of its `--lib-path`, as for `library.check`: the desktop viewer makes a
session for each folder it opens, whose root that folder is (decision of the owner, 2026-11-11). A command that writes
a file (section 6.2) leaves the viewer to read its folder again at its next command, so that what a command of the
viewer returns is never a drawing its files no longer give; a change made outside the session is drawn when the app that
watches the files says so.

| Command | Parameters | Returns |
|---|---|---|
| `diagram.svg` | The options of `labcraft-cli diagram` ([cli.md](cli.md), section 7), less the file it writes to and the folders of `--lib-path`, which are the session's: `folder`, *`functions`* (all by default), *`theme`* (`both`), *`max_diagram_nodes`* (20,000), *`trace`* (a trace of the project, with exactly one function in `functions`: its values on the wires, [cli.md](cli.md), 7.3), *`max_trace_size`* (536,870,912) | The SVG of each function asked for, the same as `labcraft-cli diagram --svg` writes, in the field `image` of its entry, and its report `labcraft-diagram/1` without `svg`, since no file is written, with its `trace` part when `trace` is given, so that an agent sees the drawing of what it wrote (D-18). When the result would pass the bound of a result (section 2), the SVGs give way from the last function to the first, each `image` as `{ "$cut": true }`, until it fits, and the result says `truncated` (decision of the owner, 2026-11-11): an agent can ask for one function at a time, or use `labcraft-cli diagram`, which writes files and has no such bound |
| `view.open` | `folder` (a library folder of the project, as the `folder` of `library.check`: `.` for its root) | The functions of the folder, each with its file and whether it is drawable, and the report of `check` (`labcraft-check/1`). The viewer shows it in place of the folder open before, and watches its files |
| `view.show_function` | `function` (a function of the open folder, named as the `FUNCTION` of `diagram`, cli.md 7), *`preview`* (`false`) (its tab is a preview, which the next preview replaces, ide.md 3; shown again without it, the tab stays) | Draws it in the viewer, as its file gives it (cli.md 7.1); returns its entry of `diagrams` in `labcraft-diagram/1` (function, file, drawn, nodes, codes). A function in a window of its own (ide.md 9) stays there: no tab opens, `preview` does nothing, and its entry is returned as its window last drew it. An unknown function is an error of the command |
| `view.set_standard_view` | `on` (boolean) | Writes `view.standard_everywhere` in the settings of the user (9.7), which shows every node in standard view (notation 3); returns the value it had, as `{ "before": false }`. Changes no file of the project; a store of the user that cannot be written is `file.unwritable` |
| `view.set_state` | `function`, `state`: the view state of notation 2.1 that the file does not hold, in part, as `{ "arms": { KEY: N }, "unfolded": { KEY: true }, "standard": { KEY: false }, "iterations": { KEY: N } }`, each part optional, `KEY` the key of a node of the drawing that follows it across a change of the file (per structure: the branch shown in tabs or unfolded; per construction: folded or not; per node: standard view or not; per loop: the iteration chosen, from 0, kept across runs and bounded by the last of each, the last following the last of the runs to come, the loops inside going to their last unless the same call chooses theirs, notation 6.1). Choosing an iteration also gives the tabs back to the arms that the run took | The drawing's entry of `diagrams` in `labcraft-diagram/1`. A click on a tab, a fold or an iteration selector goes through it |
| `view.open_trace` | *`file`* (a trace of the project, `labcraft-trace/1`), *`function`* (the one the window of the project shows) | Shows the values of that run on the function shown in a window (notation 6.1), read with the limits of cli.md 7.4, until that window shows another function; without `file`, those of its last run again (cli.md 8; decisions of the owner, 2026-11-13). The window is the one that shows `function` (ide.md 9: the IDE names the function of the window that has focus), else `params.invalid`; without it, the window of the project. Returns the `trace` part of `labcraft-diagram/1` (file, events, status, cut, stale, ignored), or `null` when the function has no trace. A file that does not exist or does not read is `LC7101` in that part, not an error of the command |

### 6.6 The registry itself

| Command | Parameters | Returns |
|---|---|---|
| `commands.list` | *`filter`* | Each command: `id`, `summary`, `effects`, `long`, `enabled` |
| `commands.describe` | `id` | Its full description (section 1) |

### 6.7 Editing (M1)

Every command of this section also takes *`expected_revision`*, except `history.*`, `settings.*`, the commands of
the window (`project.*`, `view.*`), `fragment.copy`, which writes nothing, and `commands.execute`, whose `params` carry
the `expected_revision` of the command it runs. It is an object that maps a path to a revision, for the files that the command may write
(9.1). A file it does not name is not judged. Each row lists it. An editing command can return `params.*`, `path.*`, `file.*` (section 3), `edit.unknown`,
`edit.ambiguous` and `internal`; a command that takes a new name, `name.taken` and `name.invalid`; a command that adds a
`use` line, `name.taken` (9.2); a command that
makes or renames a file, `path.exists`. Each row names only its other errors. A command that also changes other
files of the project (a rename that updates the callers) refuses, and writes nothing, when one of them does not
parse: `file.unparsed`, which names it. The commands that a later step of M1 builds are registered as `planned` (section 1), with that step.

**Nodes and constants**

| Command | Parameters | What it does |
|---|---|---|
| `node.add` | `file`, `call`, *`inside`* (the body of the function), *`arm`* (the first arm), *`at`* (placed automatically), *`name`* (from its output, N-14), *`into`*, *`expected_revision`* | Adds a node. `call` is what quick insertion lists (notation 2.6): a function as a call names it (`visa.open`, `settle`), a struct type or a variant that holds values (a construction, spec 5.1). The command adds the `use` that the file then needs, and renames a value named after its library (9.2). `inside` names a structure, and `arm` an arm of it, for a node inside it. `at` fixes the node (a `node … at` entry). Without it, the node is free and is placed near its neighbours (spec 11). `into` names a wire to insert the node into: the value enters the first input that takes its type, and the first output that the reader of the wire takes goes where the wire went, each judged as `wire.connect` judges a wire, among the first 64 inputs and the first 64 outputs *(proposed)*; a parameter that takes only a literal (the template of `string.format`) never takes it. When no input of the node takes the value, or none of its outputs goes where the wire went, it is `params.invalid`, and nothing is written (decision of the owner, 2026-11-17). The required inputs stay unwired (`LC2101`, 9.1). `string.format` is written with the empty template, `string.format("")`. `names.node` gives its name: that of its first output, numbered when taken (`timestamp2`); an output named `result`, which says nothing, gives the name of its function, `sum`, `difference`, `product` and `ratio` for `add`, `sub`, `mul` and `div` of `core` (N-14); the other outputs are `_` until wired (9.2); a node with no output gets a label (`#beep`) |
| `node.set_template` | `file`, `node`, `template`, *`expected_revision`* | Writes the template of a `string.format` (spec 5.4), a string literal as the text writes it. Each new field gets its named input, unwired; the named argument of each field that is gone goes, with its wire; the others keep their wires, by name. A node that is not a `string.format`, or a template that is not a literal or whose fields do not read (R-12), is `params.invalid`, with the message of `check`. A field is an input of its own, by its name (`text.u`): `wire.connect` and `input.set_constant` give it by name |
| `constant.add` | `file`, `value`, *`inside`* (the body of the function), *`arm`* (the first arm), *`at`* (placed automatically), *`name`* (`constant`, numbered when taken), *`expected_revision`* | Adds a named constant (spec 5.1): `limit = 240 V` |
| `node.delete` | `file`, `nodes`, *`expected_revision`* | Deletes the nodes, their wires and their entries in `layout`. A structure goes with all it holds. No other node is deleted (decision of the owner, 2026-11-15): an input that a deleted node fed becomes unwired (`LC2101` when it is required), or gets a hole where the text cannot leave it out (9.1), and an output of the function that it gave loses its value |
| `node.move` | `file`, `nodes`, `by`, *`expected_revision`* | Moves nodes of one frame by about `[dx, dy]` and fixes each one: the first one named to the nearest point of the grid of 8 of its frame, counted from its origin (the larger coordinate on a tie), so that a node off it comes back to it, and the others by as much, so that they keep their distances (notation 2.4). A move that comes to nothing once on the grid changes nothing: no entry is written and no step of undo is made. Nodes of several frames are `params.invalid`. A node that the `layout` section cannot name (no name, or a name of several bodies, 9.2) gets a label first, from the name of its line or of what it calls, numbered when another label has it (N-14; decision of the owner, 2026-11-17): `names.nodes` gives the nodes as named after it, and `renamed` each `@N` that became a label |
| `node.place` | `file`, `node`, `at`, *`expected_revision`* | Places one node at `at` in its frame, and fixes it. A node that the `layout` section cannot name gets a label first, as for `node.move`; `names.node` gives its name |
| `node.reframe` | `file`, `nodes`, *`inside`* (the body of the function), *`arm`* (the first arm), *`at`* (placed automatically), *`through`*, *`expected_revision`* | Moves nodes of one frame into another one, as one step of undo (visual notation 2.4, "Moving nodes into another frame"; decision of the owner, 2026-11-25). `inside` and `arm` name the frame as for `node.add`; the frame the nodes are in, nodes of several frames, and a structure among the nodes (moving a structure into another frame comes later) are `params.invalid`. `at` places the first node in the new frame, on its grid of 8 as `node.move` places it, and the others as far, each fixed; without it, they are free and placed automatically. A wire between two of the nodes keeps its route; the route of a wire that now crosses an edge goes, and the automatic routing draws it, as for `structure.surround` (9.6). **What crosses the edges** is judged as for `wire.connect`: a value that the new frame sees is passed in (spec 6) and nothing is written for it; a value that leaves an arm, one the nodes define and that is read outside their new frame or one they read from the frame they leave, adds an output to each structure that it leaves, that arm yielding it and the other arms a hole (9.1), as "Out of an arm" says; a value that leaves a loop goes out through each loop as `through` says for it. `through` maps the name of each such value to `last`, `collect`, `carry` or a list of them, one per loop, with the meaning and the names of `wire.connect`; a value that leaves a loop and that `through` does not name is `wire.out_of_scope`, and a name of `through` that leaves no loop is `params.invalid`. The IDE fills it from the type of what reads each value (visual notation 2.5). The nodes keep their labels and their values their names, but a value that leaves a structure: it keeps its name outside and takes a suffix inside (N-14), its readers there following; a value that was bound to an output of the function takes inside the name that inserting its node gives it, as for `structure.surround`. **Refused**, and nothing is written: a move that would make a structure read its own output, or close a cycle otherwise, `wire.cycle`; a value that a name of the new frame hides (R-3), and an order wire (`after:`) that would cross an edge (R-17), `wire.out_of_scope`; a value of the same name in the new frame, and a value named as an output of the function moved into its body, where it would give that output (R-2), `name.taken`. The entry of a node moved keeps its `view`. `names.nodes` gives the nodes as named after the move, `names.outputs` the outputs it adds, and `renamed` what it renamed |
| `node.set_view` | `file`, `nodes`, `view`, *`expected_revision`* | Writes `view standard` or `view compact` on each node; with `default`, removes it (spec 11) |
| `value.rename` | `file`, `value`, `name`, *`expected_revision`* | Renames a value or a label (`#close` to `#shut`) everywhere in the function: its readers, its `node` line and its `wire` lines. A name that the function already binds is `name.taken`, and so is the name of an output of the function, which only a wire gives (notation 2.4). A name that is not an identifier, or is a keyword, is `name.invalid`. An input or an output of the function is renamed by `signature.rename` (`params.invalid`). A name bound in several arms is renamed in each. `names.value` gives the new name; for a label, `names.node` |
| `node.set_label` | `file`, `line`, `label`, *`expected_revision`* | Gives a label (`#close`) to the node that starts on line `line` of the file (from 1, as diagnostics count), or replaces its label. This is the one command that names a node by its line, so that a node with no name, or with an ambiguous one, can be named (9.2). A line where no node starts is `edit.unknown`. `label` is written with or without its `#`; `names.node` gives it, `#close` |
| `input.set_constant` | `file`, `to`, *`arm`*, `value`, *`replace`* (`false`), *`accept_mismatch`* (`false`), *`expected_revision`* | Writes a constant on the input (spec 5.1). For an input that is already wired or that holds a constant, and for a constant of a type that the input does not take, `replace` and `accept_mismatch` act as in `wire.connect`. An output of the function, which a wire feeds, `after`, and the template of `string.format` (`node.set_template`) take no constant: `params.invalid` |
| `input.clear_constant` | `file`, `to`, *`arm`*, *`expected_revision`* | Removes the constant of an input: the input goes back to its default, is unwired, or gets a hole (9.1). A constant is never the text `?`: clearing is what writes a hole (`params.invalid`). An input that holds no constant (a wire, a hole, nothing) is `params.invalid` |
| `input.remove_value` | `file`, `to`, *`expected_revision`* | Removes a value of a variadic parameter or an element of an array (`msg.parts.2`, `k.1`), with its wire or its constant. The values after it move up one place: this command alone moves them, on purpose (9.1), and the `wire` entries that name them follow. Another input is `params.invalid`; a place that holds no value, `edit.unknown` |

**Wires and placement**

| Command | Parameters | What it does |
|---|---|---|
| `wire.connect` | `file`, `from`, `to`, *`arm`*, *`replace`* (`false`), *`accept_mismatch`* (`false`), *`through`*, *`expected_revision`* | Wires `from` to `to` (9.2); `arm` names the arm of a `yield`. **Occupied input:** an input that is already wired, or that holds a constant, is `wire.occupied`, and the message says what is there. With `replace`, the old wire and its `wire` entry go. The IDE passes `replace` for a drop on such an input, unless its setting "replace only with Alt" is on and Alt is up (notation 2.4); an agent passes it only when it means it (9.5). **Wrong type:** a value of a type that the input does not take is `wire.type_mismatch`, and the message gives both types. With `accept_mismatch`, the wire is written and `check` gives `LC3001`; the IDE passes it when its setting "accept it" is on. **Refused wires:** a wire that would close a cycle is `wire.cycle`. A wire to the template of `string.format`, which is text written in the call and has no terminal, is `params.invalid` (`node.set_template` writes it, spec 5.4). A wire from a value that the input cannot see is `wire.out_of_scope`: one of a value that a name of the input's own frame hides (R-3), or one from inside a loop to outside it without `through`. **Out of a loop:** with `through`, each loop that the wire leaves, up to the frame that the input sees, takes the value out (decision of the owner, 2026-11-18): `last`, its value at the last iteration, `last v`, its output named as `structure.add_last` names it; `collect`, an array of its values, `collect v`, its output named as `structure.add_collect` names it; or `carry`, a carry that holds its last value, `carry v2: T = ?` and `next v2 = v`, the carry and its output named after the value (N-14), of its type (without one when `check` does not know it, which `check` then reports), its initial value a hole (9.1), or `carry e2 = none` for a value of type `error`. `through` is `last`, `collect` or `carry` for every loop, or a list of them, one per loop from the inside out; a list of another length, and `carry` for a `for parallel` (spec 6.2), are `params.invalid`, and so is `through` on a wire that leaves no loop; an order wire (`after`) leaves no structure (R-17): `wire.out_of_scope`. With `through`, a bare name names the value inside the loop, even when an outer value of that name is seen (R-3); `node.output` names one value wherever its name is defined. An arm on the way gives the value as below. A loop whose first output changes keeps its entries of `layout`, which follow the name of its line (spec 11). **Out of an arm:** a wire from inside an arm of a `match` or `handle` to outside, into the frame around or into a structure beside it, adds an output to each structure that it leaves, up to the frame that both see: that arm yields it, the other arms yield a hole for it (9.1), and `names.output` gives the output that the input reads. Into another arm of the same structure, the structure would read its own output: `wire.cycle`. **Into a structure that gives nothing:** `to` naming a `match` or a `handle` that gives nothing (`_ = match …` without a `yield`, spec 6.1), with `arm`, names the `yield` of that arm: the wire adds an output to the structure, named after the value (N-14) as "Out of an arm" names it and given by `names.output`, which that arm yields and the other arms yield a hole for (9.1). A constant there has no output to go to: `params.invalid`. **Names:** wiring a value to an output of the function names the value as notation 2.4 says, and `renamed` lists what that renamed |
| `wire.disconnect` | `file`, `wire`, *`arm`*, *`expected_revision`* | Removes the wire and its entry. Its input goes back to its default, is unwired, or gets a hole (9.1). A value unwired from an output of the function takes back the name of its node (notation 2.4). `wire` ends where `wire.connect` names an input: a `yield` by the output of its structure, with `arm`, which can be left out when one arm alone yields that value and the selector does not read it; a `collect` or a `last`, an `until` or `n.next.x`; the line `next x = …` of a carry goes |
| `wire.route` | `file`, `wire`, `via`, *`expected_revision`* | Writes `wire … via` with the points of `via` (one at least, 256 at most), in the frame of the wire (spec 11), in place of the entry that named that wire. `wire` names a wire as `wire.disconnect` does, the input left out when one wire joins them, and also a wire into a structure (`k -> total.k`) or, from inside, to its `next`, `until`, `collect`, `last` or `yield` (`stop -> n`); the entry names the input only when the value reaches that node by several wires. A wire that the function does not draw is `edit.unknown`, and a function that is not drawn `params.invalid`. A branch starts at its first point when that point lies on a wire of the same value, its junction (notation 2.4). `names.wire`: the wire |
| `layout.arrange` | `file`, *`nodes`*, *`wires`*, *`terminals`*, *`inside`*, *`arm`*, *`expected_revision`* | Arrange (9.1). It removes the entries of what it arranges, and the automatic placement places that again around the rest, which stays fixed. It arranges the `nodes` and the routes of the wires between them, the routes of the `wires`, and the `terminals` (`input x`, `output x`): their `input x at y` and `output x at y` entries (spec 11). With `inside` (and `arm`), it arranges that branch. The size of a structure among the nodes it arranges goes (`frame … size`), and of each structure inside the branch it arranges; with `inside`, the structure itself keeps its size. With none of these, it arranges the whole function, and every `frame` entry goes too. "Remove the fixed position" of a node or a terminal calls it with that node or terminal. An agent calls it only when asked to (9.1) |
| `frame.resize` | `file`, `size`, *`inside`*, *`expected_revision`* | Writes `frame size (w, h)` from `[w, h]` (spec 11), or with `inside` `frame ys size (w, h)` for that structure, measured from the origin of its frame; its entry replaces the one there. A size too small for the content is `params.invalid`, and the message gives the smallest size; `inside` on a node that is no structure is `params.invalid`. `size` `null` removes the entry, and the size is automatic again ("Remove the fixed size", visual notation 2.4). A structure that the `layout` section cannot name gets a label first, as for `node.move`; `names.nodes` gives its name |
| `terminal.move` | `file`, `terminal`, `y`, *`expected_revision`* | Writes `input x at y` or `output x at y` (spec 11), with `y` as given: the grid of 8 and the 24 units between two terminals are rules of the gesture (notation 2.4), and a file that places them closer draws them as written |

**The block of a function and the mask of a library**

These write the file of the function whose block they change, or the manifest of its library, never the file of a
caller (spec 11.2, 11.3, 11.4). A function or a library that the project does not hold (shipped with LabCraft, or found with
`--lib-path`) is `path.outside_project`.

| Command | Parameters | What it does |
|---|---|---|
| `block.set_terminal` | `file`, `terminal`, `side`, `place`, *`expected_revision`* | Writes the line `side place name` of the `terminals` section (spec 11.2), with `input` or `output` before the name when the name alone would mean the other terminal. `terminal` is `input x` or `output x`, as `layout.arrange` names it; `side` is `top`, `left` or `right`; `place` is a number, `last` or `middle`. A terminal has one line: a new place replaces the old one. `null` removes the line, and the section when nothing is left in it. A place outside its side, a place that another line holds, the place of `after:`, or a change that would leave another line of the section unable to apply (a pair named alone that would follow onto a place held) is `params.invalid`; a place that a terminal holds by default is free, and that terminal takes the first free place from the top, as the defaults do. The IDE calls it on the file of the function called when a terminal of a block is dropped on a place (notation 2.4), and in the view `Block` (ide.md 3) |
| `block.set_size` | `file`, *`top`*, *`left`*, *`expected_revision`* | Writes `top N` and `left N`, `top` 1 to 64, `left` 0 to 64 (spec 11.2); `null` removes one. A size whose places cannot hold the terminals that the section names is `params.invalid`, and the message gives the smallest |
| `block.set_band` | `file`, `band`, *`expected_revision`* | Writes `band hidden`, `band inside` or `band above`; `null` removes it, and the function follows the mask of its library |
| `block.set_color` | `file`, `color`, *`expected_revision`* | Writes the line `color "#RRGGBB"` of the `terminals` section (spec 11.2), the colour of the band and, without `fill`, the tint of the body; `null` removes it, and the function follows the mask of its library. A colour that is not `#RRGGBB` is `params.invalid`; `fmt` writes it as given, in either case |
| `block.set_fill` | `file`, `fill`, *`expected_revision`* | Writes the line `fill "#RRGGBB"`, the fill of the body, as written, in both themes; `null` removes it. A colour that is not `#RRGGBB` is `params.invalid` |
| `block.set_stroke` | `file`, `stroke`, *`expected_revision`* | Writes the line `stroke "#RRGGBB"`, the colour of the outline, as written, in both themes; `null` removes it. A colour that is not `#RRGGBB` is `params.invalid` |
| `block.set_outline` | `file`, `outline`, *`expected_revision`* | Writes the `outline` section (spec 11.4) from `outline`, its items one per line (`template d`, `template primitive`, or a `path` and its pieces), in canonical form; `null` removes the section, and the block takes the outline of its mask or the ordinary one. Items that do not read or that close the section, two `path` items, two pieces for one side, a piece without a `path`, a `template` with another item, an unknown template, a `round` outside 1 to 16, an integer outside 0 to 1,024, or more than 64 corners in the body or 16 points in a piece are `params.invalid`; an outline that reads but breaks a guard of 11.4 is written, and `check` gives its warning, as the drawing judges it at every size |
| `block.reset` | `file`, *`expected_revision`* | Removes the `terminals` and `outline` sections: the block goes back to the defaults of notation 3.2, with the outline of the mask of its library when it has one |
| `library.set_mask` | `folder`, *`band`*, *`band_place`*, *`color`*, *`fill`*, *`stroke`*, *`outline`*, *`expected_revision`* | Writes these fields of the table `[mask]` of the manifest of the library in `folder` (spec 11.3), and makes the manifest, holding the table alone, when the library has none; `null` removes one, the table when nothing is left in it, and the manifest when nothing else is left in it either. The rest of the manifest, its comments and its order are kept as written. A value that the mask refuses (a colour that is not `#RRGGBB`, a `band` that 11.3 refuses, an `outline` that `block.set_outline` would refuse) is `params.invalid`; `outline` is written as a multi-line TOML string. The manifest is otherwise out of reach of the commands (section 1) |

**Structures**

| Command | Parameters | What it does |
|---|---|---|
| `structure.surround` | `file`, `kind`, *`nodes`* (none: an empty structure), *`selector`*, *`over`*, *`arm`* (the first arm), *`inside`* (the body of the function), *`inside_arm`* (the first arm), *`at`* (placed automatically), *`expected_revision`* | Puts the nodes in a new `match`, `handle`, `for` or `loop`, or makes an empty one; see 9.6. A `match` on an enumeration of a library adds its `use`, and renames a value named after that library (9.2) |
| `structure.add_arm` | `file`, `structure`, `patterns`, *`expected_revision`* | Adds an arm to a `match` or a `handle`, with its patterns as the text writes them (`1, 3..=5`, `io.timeout(err)`). Its `yield` gives a hole for each output of the structure (9.1); in a structure that gives nothing, it has none. The arm goes before a last arm `_`, which would take its values, and at the end otherwise. Patterns that do not read as the patterns of one arm of that kind of structure are `params.invalid`, with the message of the parser. `names.arm` gives them as the text writes them |
| `structure.set_patterns` | `file`, `structure`, `arm`, `patterns`, *`expected_revision`* | Changes the patterns of an arm. `arm` names the arm by its patterns as the text writes them (`_`, not `other`). The nodes of the arm stay; a name that the old patterns bound and that they read is theirs no more, and `check` says so. `names.arm` gives the new patterns |
| `structure.add_carry` | `file`, `structure`, `name`, *`initial`* (a hole), *`type`* (that of `initial`), *`expected_revision`* | Adds `carry name = initial` to a loop (spec 6.2), after its other clauses, and its output, also named `name` (R-3), after the outputs of the other carries. `initial` is a value that the frame of the loop sees, or a constant; one that the loop makes, or that depends on it, is `wire.cycle`. Without `initial`, the carry starts from a hole, and then needs `type`. A `type` of a library adds its `use`, and renames a value named after that library (9.2); a `name` that is the name of that library is `name.taken`. Its `next` has no line until it is wired: `check` says so. A `for parallel` takes no carry (spec 6.2): `params.invalid`. A first output that changes the name of the line of the loop takes its entries of `layout` with it (spec 11). `names.output` gives its output |
| `structure.add_clause` | `file`, `structure`, `over`, *`name`* (from the value, N-14, as 9.6 names a clause), *`expected_revision`* | Adds the clause `name in over` to a `for`, after its other `in` clauses. `over` is an array value or a range (`0..n`), as for `structure.surround`; a value that the loop makes, or that depends on it, is `wire.cycle`. `names.clause` gives its name |
| `structure.add_collect` | `file`, `structure`, `value`, *`name`* (from the value, N-14), *`expected_revision`* | Adds `collect value` to a loop, at the end of its body, and its output, after the others. `value` is a value that the body of the loop sees, or a constant; an outer one that the loop makes, or that depends on it, is `wire.cycle`. `names.output` gives its output |
| `structure.add_last` | `file`, `structure`, `value`, *`name`* (from the value, N-14), *`expected_revision`* | Adds `last value` to a loop (spec 6.2), at the end of its body, and its output, after the others. `value` and `name` are as for `structure.add_collect`. `names.output` gives its output |
| `structure.remove_part` | `file`, `structure`, `part`, *`expected_revision`* | Removes one part of a structure, with what it holds: an arm with its nodes, a carry with its `next`, a `collect` or a `last` with its output. `part` is an arm (by its patterns), `carry x`, `in x`, `collect y` or `last y` (`y` is its output). The inputs that it fed become unwired, or get a hole (9.1); the entries of `layout` of the structure follow the name of its line, or a label when it has none now (spec 11). The last arm of a `match` or `handle` and the last clause of a `for` are `params.invalid`: delete the structure |
| `structure.set_passage` | `file`, `structure`, `part`, `to`, *`expected_revision`* | Changes how a loop takes one value out (decision of the owner, 2026-11-18): `part`, a `carry x`, `collect y` or `last y` as `structure.remove_part` names it, becomes a `to`: `carry`, `collect` or `last`. Its output keeps its name and what reads it, and the outputs their order (spec 6.2): the carries, then the `collect` and `last` lines in their written order. **Between `collect` and `last`**, the word of the line changes, and the type of the output with it: `check` reports a reader that does not take it. **A carry into a `collect` or a `last`:** the carry and its initial value go, and its line `next x = v` becomes `collect v` or `last v`, in its place; a node that binds `next x` binds a value named after the carry (N-14), which a new line reads after it; without a `next`, `collect ?`; the label of a line `next x = v` goes, and the `after:` that name it lose it (a `collect` takes no label). A body that reads the carry itself (`next acc = core.add(acc, x)`, an `until`) is `params.invalid`: the message names the carry. So is a carry with several `next` lines, which `check` refuses (R-6). **A `collect` or a `last` into a carry:** `carry y = none` for a value of type `error`, `carry y: T = ?` otherwise (`T`, the type of the value; none when `check` does not know it), as `wire.connect` writes it with `through`, after the other carries; the line becomes `next y = v`, in its place. A value of the body or an `in` clause named `y` would be defined twice: `params.invalid`, rename one first. **An `error`:** a carry of one stops a `loop` on the first error (spec 6.3), a `collect` or a `last` does not: the change adds or removes that stop, and the IDE says so in the tooltip of the entry (decision of the owner, 2026-11-18). A `for parallel` takes no carry (spec 6.2): `params.invalid`. A `to` that is already the kind of the part, an arm and an `in` clause are `params.invalid`. A first output that changes the name of the line of the loop takes its entries of `layout` with it (spec 11); an entry of a wire to the input that goes, from a value of the loop, is forgotten, unless it may be the route of another wire of that value to an input without a name (a `last`, an `until`). `names.output` gives its output |
| `structure.set_option` | `file`, `structure`, `option`, `on`, *`expected_revision`* | Turns `continue_on_error` of a `loop` (spec 6.3) or `parallel` of a `for` (spec 6.2) on or off. An option of the other kind of loop is `params.invalid`, and so is `parallel` on a `for` that has a carry (spec 6.2) |

**Functions, signatures and types**

| Command | Parameters | What it does |
|---|---|---|
| `function.create` | `path`, *`doc`*, *`expected_revision`* | Writes a new function, `fn name() -> () {}`, named after its file (R-22). An existing file is `path.exists` |
| `function.extract` | `file`, `nodes`, `name`, *`expected_revision`* | Moves the nodes, of one frame, into a new function `name`, in the folder of `file`, and puts a call to it in their place, one step over both files; see 9.8. What the nodes read from outside becomes its inputs, what they give outside its outputs, named as the values. `names.function`, `names.file` and `names.node` give what it made |
| `function.rename` | `file`, `name`, *`expected_revision`* | Renames the function and its file, in its folder (R-22). It updates every call and every reference to it in the project, in the same step (decision of the owner, 2026-11-15): a call (`f(…)`, `lib.f(…)`, `a.b.f(…)` from a sub-library) and a function passed as a value (`measure: f`), but not a value of the function that holds it whose name it bears (spec 4). A new name that a file it changes already gives a value, a generic variable or a library it uses, which would read the renamed uses as that, is `name.taken`, and nothing is written (decision of the owner, 2026-11-19). `names.file` gives its new file; a name whose file exists is `path.exists`. The tabs and the windows that show the function follow it |
| `function.set_doc` | `file`, `doc`, *`expected_revision`* | Writes the documentation comment (`///`); an empty text removes it |
| `signature.add` | `file`, `terminal`, `type`, *`default`* (required), *`position`* (the last, before `err`), *`expected_revision`* | Adds an input (`input x`) or an output (`output x`). Callers are not changed: a new required input gives each of them `LC2101`. An output named like a value that a line of the body of the function binds is given by that line, as the text writes it (spec 5.2). A `type` of a library adds its `use`, and renames a value named after that library (9.2) |
| `signature.remove` | `file`, `terminal`, *`expected_revision`* | Removes an input or an output, and its wires in the body: what read the input becomes unwired or gets a hole (9.1), and its entry of `layout` and its line of `terminals` (spec 11, 11.2). Callers are not changed: `check` shows each of them |
| `signature.rename` | `file`, `terminal`, `name`, *`expected_revision`* | Renames an input or an output in the body, in `layout` and in the line of `terminals` that names it (spec 11.2; decision of the owner, 2026-11-19). It also updates the calls that name it in the project (`high: 250 V`), and their `wire` entries (`wire x -> c.high`), in the same step (decision of the owner, 2026-11-15) |
| `signature.set_type` | `file`, `terminal`, `type`, *`expected_revision`* | Changes the type of an input or an output; `check` judges the wires that it reaches. A `type` of a library adds its `use`, and renames a value named after that library (9.2) |
| `signature.set_default` | `file`, `terminal`, `default`, *`expected_revision`* | Sets the default value of an input (a constant, spec 4); `null` makes the input required |
| `type.create` | `path`, `definition`, *`doc`*, *`expected_revision`* | Writes a new `.lct`: `type Name = definition`, named after its file (R-22). `definition` is a type in text (spec 3.2): one that does not read, that is more than one type, or that a section follows (`layout`), is `params.invalid`, with why. An existing file is `path.exists` |
| `type.rename` | `file`, `name`, *`expected_revision`* | Renames the type and its file, in its folder (R-22), and every use of it in the project, in the same step: a type written in a signature, a carry or another type (`Step`, `lib.Step`), a construction (`Step { … }`), a variant (`State.Init`, `lib.State.Init`, `a.b.State.Init`), but not a generic variable of its name. A new name taken in a file it changes is `name.taken`, as for `function.rename`. `names.file` gives its new file; a name whose file exists is `path.exists` |

**Copy and paste**

| Command | Parameters | What it does |
|---|---|---|
| `fragment.copy` | `file`, `nodes` | Returns `{ "text": … }`, the nodes as the text of a function `clipboard`, in canonical form, with the `use` they need, the wires between them and their `layout` entries counted from the top-left corner of the selection (notation 2.6). An input wired from outside is left out, or gets a hole (9.1). A node without a name has no entry. Writes nothing, and is not a step of history. Nodes of several frames are `params.invalid`. A text above 4 MiB *(proposed)* is `file.too_large`, never cut |
| `fragment.paste` | `file`, `text`, `at`, *`inside`* (the body of the function), *`arm`* (the first arm), *`expected_revision`* | Puts the body of the function that `text` holds into the frame, its top-left corner at `at`, each node that has a `node … at` entry fixed. Of its `layout` section, only the `node` and `wire` entries are taken (notation 2.6). It adds the `use` that the file then needs, and renames a value of the function named after its library (9.2). A name already taken in the function, labels and the outputs of the function included, gets a suffix (N-14), and its readers in the fragment follow; `renamed` lists them. A name that the fragment reads without binding it is left out, or gets a hole (9.1). A `text` that does not parse, or holds no function, is `params.invalid`, with the message of the parser; above 4 MiB *(proposed)*, `file.too_large`, as for `file.write`. `names.nodes` gives the names of the nodes pasted, as a command names them; a node without a name, which a command names only by its line, is not in it |

**History, batches and settings**

| Command | Parameters | What it does |
|---|---|---|
| `history.undo` | *`step`* | Undoes the last step of the project (9.1), and returns what an editing command returns, for the files it restored. When there is nothing to undo, it is `history.empty`. With `step`, the number of the step meant (`history.list`), another step last is `file.stale` and nothing changes, so that a step that came since is never undone in its place (the notice of an agent, ide.md 8; decision of the owner, 2026-11-21). For an agent, see 9.5 |
| `history.redo` | | Redoes the last step undone, under the same rules |
| `history.list` | *`limit`* (`50`) | The steps, newest first, and where undo and redo stand: `{ "steps": [{ "step", "author", "label", "files", "time" }], "undo": 42, "redo": null, "truncated": false }` |
| `commands.execute` | `id`, *`params`* (`{}`) | Runs the command `id` with `params`, as its own tool would (mcp.md 2): the MCP server reaches through it every command that is not a tool of its own. It returns what that command returns, and its errors. Its effects are those of the command it runs: the policy of an agent (ide.md 8), the author, the line of the journal and the label of the step are that command's. `commands.execute` itself and `commands.batch` are `params.invalid`, and so is `commands.execute` inside a batch |
| `commands.batch` | `commands`, *`label`* (the labels of its commands), *`expected_revision`* | Runs a list of `{ "id", "params" }` in order, as one step of undo (9.1). If one fails, nothing is written, and the command returns its error with its `index`. Otherwise it returns, for each command, its `names` and `renamed`, and, for all, one `step`, one `files`, and the `diagnostics` and `summary` of one check of the root, made at the end. It takes the commands of 6.7 that edit, and at most 1,000 *(proposed)*: `history.*`, `settings.*`, `fragment.copy`, the commands of the window, `function.create`, `function.extract`, `function.rename`, `type.create` and `type.rename`, which make or rename a file, `library.set_mask`, which writes a manifest, and `commands.batch` itself are `params.invalid`. The `expected_revision` of the batch is judged once, before its first command; its commands do not take one (`params.unknown`). Before its commands, each node that they name by its line (`@N`, 9.2) and that the `layout` section cannot name gets its label, as `node.move` gives it, in one pass over the text before the batch, and each `@N` is read as that label: a label that a command gives can move lines (spec 5.2), and a later `@N` would name another node (decision of the owner, 2026-11-17). The `renamed` of the first command that names it says so. In a parameter that names a node, a wire or an input (9.2), `$i` names the node that the earlier command `i` of the batch named, its `names.node`: `[{ "id": "node.add", "params": { "file": "f.lcf", "call": "core.abs" } }, { "id": "wire.connect", "params": { "file": "f.lcf", "from": "reading", "to": "$0.x" } }]` is quick insertion from a wire (notation 2.6), one step; a node renamed later in the batch (folded into an output, given a label) keeps its `$i` under its new name. A `$i` is a whole name, at the start of the parameter or of the end of a wire after its arrow, followed by `.`, a space or nothing, its number without a leading zero (`$0`, `$0.x`, `reading -> $0.x`; not `$0x`, `$01`), and the `$i` of one command bring at most 1 MiB *(proposed)*; otherwise, or for a `$i` of a command that is not earlier, or that named no node, it is `params.invalid` (decision of the owner, 2026-11-17) |
| `settings.get` | *`key`* (all) | `{ "settings": [{ "key", "value", "from", "project" }] }`, in the order of the table of 9.7; `from` is `project`, `user` or `default`, and `project` says whether a project may hold the key. An unknown key is `params.invalid` |
| `settings.set` | `key`, `value`, *`scope`* (`user`) | Writes the setting in the store of the user or of the project (9.7), and returns `{ "before": …, "scope": "user" }`, `before` being what that store held, `null` for nothing. `value: null` takes the key out of the store: the other store or the default gives it again (`Reset`). A setting is not a program, so this is not a step of history. An unknown key, a value of the wrong type, or `scope: "project"` for a key that a project may not hold, is `params.invalid` |

**The window**

The commands of the window of the IDE ([ide.md](ide.md)). They change what the window shows, and `project.new` writes the first file of a new project; none is a step of
the history; the layout they change is a setting of the user. An agent attached to the IDE may call
`view.get_state` only: the others are `agent.not_allowed` (ide.md 8). `project.*` takes a folder anywhere on the
computer, chosen by the user in a dialog of the system, so it is not a path of a project (section 2); the root of a
project keeps the rules of mcp.md 1 (the home folder and the root of a disk are refused). In `labcraft-cli` and in the MCP server alone, which
have no window, they are `command.not_enabled`. The `view.*` commands of the viewer (6.5) stay, for the window; to an attached agent, those that change
what the window shows are `agent.not_allowed` too (ide.md 8).

| Command | Parameters | What it does |
|---|---|---|
| `project.new` | `folder` | Makes the folder with `main.lcf` (`fn main() -> () {}`), and opens it with `main` shown (ide.md 10). A folder that exists and is not empty is `path.exists` |
| `project.open` | `folder` | Opens the folder as a project: in this window when it shows the start page, in a new window otherwise |
| `project.download` | | Saves the project as a zip, `NAME.zip`, `NAME` being the name of its folder: on the web page, which holds the project in memory, the files that its session holds (ide.md 12). The desktop window, whose files are on the disk, says so in a notice and saves nothing; in `labcraft-cli` and the MCP server alone it is `command.not_enabled`, and to an attached agent `agent.not_allowed`, as the other commands of the window. A project past the limits of a zip without zip64 (65,534 entries, 4 GiB) is `file.too_large`, a path that cannot name an entry `path.invalid` |
| `view.get_state` | | `{ "workspace": "edit", "windows": [{ "tabs": [{ "function", "file", "view", "split" }], "shown", "selection", "zoom" }], "gesture", "truncated": false }`: the gesture of the user in progress, one of `rectangle`, `move`, `wire`, `route`, `resize` and `field` (a text typed and not written yet), or `null` (9.1); for each window of the project, the window of the project first and then the detached ones in the order they were detached (a detached window has the one tab of its function), its tabs (the function named as in 6.5, its file, its view among `diagram`, `text` and `block`, the second view of a split or `null`), the index of the tab shown or `null` on the start page, the selection of the tab shown as the editing commands name it (9.2), at most 1,000 items *(proposed)*, and the zoom. Not `view.set_state` (6.5), which sets the view state of a drawing |
| `view.select` | `function`, *`nodes`*, *`wires`*, *`terminals`* (each `[]`) | Sets the selection of the window that shows the function, which `view.get_state` returns; all empty clears it. The function is the one shown, else `params.invalid`; each name names what the function has, as an editing command names it (9.2): a node, a wire that exists, `input x` or `output x`, else `edit.unknown` (or `edit.ambiguous`) on its parameter; at most 1,000 names *(proposed)*. The window calls it on each click and rectangle of selection ([architecture.md](architecture.md), 12.5). A node named by its line (`@N`, 9.2) leaves the selection, with the wires into it, when the file changes so that its line names another node or none |
| `view.close_function` | `function` (a function whose tab is open) | Closes its tab; when it was shown, the next tab is shown, else the one before it. The tab of a function in a window of its own closes with its window (ide.md 9). Returns `{ "shown" }`, the function that the window of the project shows after, or `null` |
| `view.set_view` | `function`, `view`, *`split`* (`null`) | Shows the function in its tab with that view, and with `split` the second view beside it: `text` with `diagram` or with `block`. `panel` is `command.not_enabled` until M3; another pair is `params.invalid` |
| `view.detach` | `function` (a function whose tab is open) | Moves the tab of the function into a window of its own (ide.md 9), with what it shows: its drawing, its values, its selection, its zoom; on the web, a floating panel. When it was shown, the window of the project shows the next tab, else the one before it. A function already in a window of its own stays there. Returns `{ "shown" }`, the function that the window of the project shows, or `null` |
| `view.dock` | `function` (a function in a window of its own) | Moves it back among the tabs of the window of the project, as its last tab, shown there with what its window showed; its window closes. A function whose tab is there already stays. Returns `{ "shown" }` |
| `view.toggle_panel` | `panel`, *`on`* (the other state) | Shows or hides `project`, `palette`, `inspector`, `problems` or `run` |
| `view.reset_layout` | | Puts the panels back where ide.md 2 says, and docks every detached tab |
| `view.set_workspace` | `workspace` | Changes the workspace of the window (ide.md 6); a planned one is `command.not_enabled`, and the message gives its milestone |
| `agents.disconnect` | *`agent`* (the one agent attached) | Disconnects the agent attached that signs its steps with that name (`claude`, `claude 2`, 9.5), as Disconnect in the menu of the pill (ide.md 8): its call in progress ends, and its MCP client sees the server go. With no agent of that name, or none named while several are attached, the window says so and nothing changes |
| `agents.set_policy` | *`agent`* (the one agent attached), `policy` (`ask`, `edits` or `all`), *`keep`* (`false`) | Sets the policy of the agent for this connection, as the menu of the pill does (ide.md 8); with `keep`, also keeps it for its client and this project in `ai.policies` (9.7), which answers the next connection of that client to that project without asking |

**The `Text` panel**

The commands of the `Text` panel (9.1; decisions of the owner, 2026-11-21). They are commands of the window: in
`labcraft-cli` and in the MCP server alone they are `command.not_enabled`, and to an attached agent
`agent.not_allowed`, since an agent writes a text with `file.write`. Each takes a `.lcf` or `.lct` file of the project
(`path.invalid` otherwise); the recovery folder is `.labcraft/recovery/`, where the copy of a file has the path of the
file ([architecture.md](architecture.md), 12.6), and a folder of it that a link leads elsewhere is neither written nor
read.

| Command | Parameters | What it does |
|---|---|---|
| `text.save` | `path` (a file that exists), `text`, *`expected_revision`* (none), *`keep`* (`false`) | Reads the text as `check` reads a file (CR LF made LF, at most 4 MiB, as `file.write`). When it parses (spec 14: its grammar is right, whatever its errors of types or of `check`), writes it as `file.write` does, one step of history labelled `Edit Text` (none when the bytes are those of the file), removes its copy in the recovery folder, and returns `{ "written": true, "revision" }`, the revision of the file written: not that of `text` when a rename moved entries of `layout` (9.1), and the panel then shows the file. When it does not parse, writes nothing in the project, keeps the text in the recovery folder, and returns `{ "written": false, "kept", "error" }`, `error` being the diagnostic that stops the reading, as `labcraft-check/1` writes one, and `kept` whether the copy could be written. `expected_revision` is the revision of the file that the text was made from, a text: when the file has another one, nothing is written either, the text is kept, and it returns `{ "written": false, "kept", "stale": true, "revision" }`, the revision the file has (decision of the owner, 2026-11-21). With `keep`, it only keeps the text in the recovery folder, and returns `{ "written": false, "kept" }`: the window keeps so a text that the file changed under |
| `text.recovered` | | The texts of the recovery folder whose file exists and changed before them (a time that the system does not give counts as before): `{ "texts": [{ "path", "text", "modified" }], "truncated": false }`, `modified` in nanoseconds since 1970, by path. A copy over 4 MiB, or that is not UTF-8, is not given; at most 16 MiB of text in all and 1,000 entries of the folder looked at *(proposed)*, then `truncated`. The window calls it when the project opens |
| `text.discard` | `path` | Removes the copy of the file in the recovery folder: `{ "discarded": true }`, `false` when there was none |

## 7. The report of a run: `labcraft-run/1`

What `run.start`, `run.wait`, `run.status` and `run.abort` return, and what `labcraft-cli run --json`
prints. One example, then the fields.

```json
{
  "schema": "labcraft-run/1",
  "status": "ok",
  "run": "r2",
  "function": "check_voltage",
  "mode": "deterministic",
  "outputs": [
    { "name": "verdict", "type": "Verdict", "value": "Verdict.Pass" },
    { "name": "voltage", "type": "f64[V]", "value": "230.071217 V" },
    { "name": "err", "type": "error", "value": null }
  ],
  "clock": { "simulated_ms": 0.0, "real_ms": 2 },
  "resources": { "opened": 1, "open": [], "leaked": [] },
  "instruments": [
    { "resource": "SIM::DMM0", "commands": [ { "t_ms": 0.0, "sent": "MEAS:VOLT:DC?", "answer": "230.071217" } ] }
  ],
  "trace": { "file": ".labcraft/runs/check_voltage.trace.jsonl", "events": 41, "truncated": false },
  "check": {
    "summary": { "libraries": 1, "functions": 1, "types": 1, "errors": 0, "warnings": 0, "truncated": false },
    "diagnostics": []
  }
}
```

| Field | Always | Content |
|---|---|---|
| `schema`, `run`, `function`, `mode` | yes | `mode`: `deterministic` or `real_time` |
| `status` | yes | `ok`, `error`, `aborted`, `fault` (spec 8.2); `running` while it goes on (section 4); `refused` when `check` found an error in what the run can reach (or, under `deny_warnings`, a warning in a file of the folder that the run can reach), or the function is unknown, or an argument, a setting or the simulation file was refused, and nothing ran (cli.md, section 6.1). `labcraft-cli run` exits with 1 on `refused`, as `check` does on an error |
| `outputs` | yes | The outputs of the function, in the order of its signature, each with its type and its value (section 5); `[]` when it did not finish (`aborted`, `fault`, `refused`). Those that the bound of a result leaves no room for, even with their value cut, are counted in `outputs_more` (section 2) |
| `error` | no | When `status` is `error`: the error on the `err` output, as in section 5, repeated here so that it is found without looking for the output |
| `fault` | no | When `status` is `fault`: `code` (an `LC` code, family `LC80`, cli.md, section 6.6), `message`, `function`, `node` (`null` for a fault at no node), `file`, `span` (that of the statement of the node, from its start to its end, a loop or an arm with its body; `null` with `node`), `loops`, the loops around it as below, at most 1,000 *(proposed)*, then `loops_more`, and for `LC8001` `iteration`, that of the loop itself which would pass the limit, from 0 |
| `abort` | no | When `status` is `aborted`: `by` (`request`, `timeout`, `interrupt`), `at_ms` on the clock of the run, `loops` (as below, when the abort came), and `warning`: the instruments are in an unknown state. When a native call was in progress, the run ends after `at_ms`: when that call ends, or when the grace period ends if it is abandoned (spec 8.2); `clock` says when |
| `loops` | no | When `status` is `running`: each loop going, in the global order of spec 8, each loop before the loops inside it, with its `function`, its `node` (section 8) and its `iteration`, from 0 |
| `clock` | yes | `simulated_ms` in deterministic mode; `real_ms` always. In real time, the times of the report and of the trace (`at_ms`, `t_ms`) are on the real clock, from the start of the run |
| `resources` | yes | The resources the run opened (spec 8.1: a session of `visa`, or any opaque type a library declares as one). `opened`: how many; `open`: those still open at its end, each with its `type` (`visa.session`), what it designates (`resource`: `SIM::DMM0`, the name as the program gave it, also for an instrument of `visa.sim` opened by a real name) and where it was opened (`function`, `node`, `file`, `span`), which scenario 1 wants empty; `leaked`: those an abort could not release (spec 8.2), the same way. At most 1,000 of each *(proposed)*, then `"truncated": true` |
| `instruments` | no | With a simulation: per resource that received a command, in code point order of the resource names so that two seeds give the same list, the commands it received in order, each text cut as a string is (section 5). Each command gives `t_ms`, when the instrument handled it, after its latency; `sent`, the command as the instrument read it (simulation.md, section 4); and `answer`, its answer, or `null` for a command that has none: a write, or a query never answered (a timeout, an unknown query). A resource that received no command is not listed; when none did, or the run has no simulation, the field is absent. At most 64 resources and 1,000 commands per resource in the report *(proposed)*: past them, a resource says `"truncated": true` and how many commands it leaves out in `more`, and `instruments_more` after the list how many resources; the bound of a result (section 2) can shorten it further. From 0.5.0 the field is given for real instruments too, not only with a simulation, and it lists the endpoints that `net` and `serial` reached, simulated or real, by their endpoint (`tcp:10.0.0.7:502`), with one entry per write, its `sent` and an `answer` of `null`, and one per read, its `answer` and a `sent` of `null`, in order, since a stream has no pairs; in text, or in hexadecimal with `"hex": true` ([io.md](io.md) 6). An open of a simulated `serial` device with settings it does not expect is said on its endpoint, listed for it even with no command (decision of the owner, 2026-11-20): `settings`, one item per such open, or (0.5.0) per `serial.configure` to such settings, each with its `t_ms` and, for each setting that differs only (`baud`, `data_bits`, `parity` as `"none"`, `"odd"` or `"even"`, `stop_bits` 1 or 2, `flow` as `"none"`, `"software"` or `"hardware"`), `{"expected": X, "opened": Y}`; at most 1,000 per resource *(proposed)*, then `"truncated": true` and how many it leaves out in `settings_more` |
| `logs` | no | What libraries kept of the values a run gave them, such as `results_log.append_csv` (spec A.4), which writes no file in M0: `{ "entries": [ … ], "truncated": false }`, one entry per call, in the order of the calls, `{ "library": "results_log", "function": "append_csv", "t_ms": 0.0, "type": "[results_log.StepResult]", "value": … }`, the value as in section 5, with the type of the input of the signature it was given on. At most 1,000 entries for each library *(proposed)*, and what the libraries of a run keep for its report, the commands of `instruments` included, within 16 MiB *(proposed)*, or `--max-memory` when lower, a value counting its logical size (cli.md 6.2): the entries after them are counted, not kept, and `truncated` is then `true`, with their number in `more`, so that the log of each library, as the commands of each instrument, is the start of what it was given, and what they keep stays within that bound. Entries of two libraries come in the order of `t_ms`, then of the libraries. The bound of a result (section 2) can shorten it further |
| `console` | no | *(0.5.0, [io.md](io.md) 5)* The lines the program wrote with `console.write_line`, in order: `{ "lines": [{ "t_ms": 12.5, "text": "T = 23.5" }], "truncated": false }`, each text cut as a string is (section 5). At most 10,000 lines *(proposed)*, within the budget of `logs`: past them, lines are counted in `more`, not kept. Absent when the run wrote none |
| `panel` | no | Each setting of `set` or `run.set_element`, in the order applied: `element`, `value`, `at_ms` on the clock of the run when it applied (in real time, a setting of `set` can apply a little after the time it was given for; decision of the owner, 2026-11-11), `by` (`set`, `run.set_element`), and `applied` (`false` for a setting after the end of the run); then those that never applied, a setting of `run.set_element` given as the run ended with `at_ms: null` |
| `check` | yes | The `check` made before the run: its `summary`, which counts everything found in the folder, and its `diagnostics`, the fields of `labcraft-check/1` (cli.md, section 4.2): those of what the run can reach, and the errors found elsewhere in the folder, which do not refuse the run but are listed, so that the counts and the list agree |
| `trace` | no | With `trace: file`: `{ "file": ".labcraft/runs/check_voltage.trace.jsonl", "events": 41, "truncated": false }`; with `trace: inline`: `{ "header": …, "events": […], "truncated": false }`, the first line and the events themselves (section 8), at most 2,000 *(proposed)*, and fewer if the bound of a result is reached (section 2). A trace that cannot be written (a read-only project, a full disk) does not stop the run: `trace` then gives `error`, a command error of section 3 (`file.unwritable`) |

An argument that does not convert, an input with no value and no default, or an unknown input is a
diagnostic in `check.diagnostics`, with the name of the input, and the status `refused`: like a type
error, it is found before anything reaches an instrument (scenario 2, *with M0b*). Its codes are those
of the family `LC81` (cli.md, section 6.6).

## 8. The trace of a run: `labcraft-trace/1`

What a run did, node by node, for the agent that debugs and for the viewer that puts values on the
wires (scenario 6, point 7). JSON Lines: one object per line, written as the run goes, so that a long
run can be followed and a cut file still reads.

```text
{"schema":"labcraft-trace/1","run":"r2","function":"check_voltage","mode":"deterministic","files":[{"file":"check_voltage.lcf","sha256":"…"},{"file":"Verdict.lct","sha256":"…"},{"file":"<labcraft>/scpi/query_f64.lcf","sha256":"…"},{"file":"<labcraft>/visa/open.lcf","sha256":"…"},…]}
{"seq":1,"t_ms":0.0,"event":"call","frame":0,"function":"check_voltage","in":[{"name":"resource","type":"string","value":"SIM::DMM0"},{"name":"low","type":"f64[V]","value":"220.0 V"},{"name":"high","type":"f64[V]","value":"240.0 V"},{"name":"err","type":"error","value":null}],"types":{"dmm":"visa.session","e1":"error","dmm2":"visa.session","raw":"f64","e2":"error","measured":"Verdict"}}
{"seq":2,"t_ms":0.0,"event":"call","frame":1,"parent":0,"node":"dmm","function":"visa.open","in":[…],"types":{…}}
{"seq":5,"t_ms":0.0,"frame":0,"node":"dmm","event":"ran","out":{"dmm":{"opaque":"visa.session","resource":"SIM::DMM0","open":true},"e1":null}}
{"seq":6,"t_ms":0.0,"event":"call","frame":2,"parent":0,"node":"dmm2","function":"scpi.query_f64","in":[{"name":"session","type":"visa.session","value":{"opaque":"visa.session","resource":"SIM::DMM0","open":true}},{"name":"command","type":"string","value":"MEAS:VOLT:DC?"},{"name":"err","type":"error","value":null}],"types":{"text":"string","s":"visa.session","e1":"error","value":"f64","e2":"error"}}
{"seq":9,"t_ms":0.0,"frame":2,"node":"text","event":"ran","out":{"text":"230.071217","s":{"opaque":"visa.session","resource":"SIM::DMM0","open":true},"e1":null}}
{"seq":12,"t_ms":0.0,"frame":0,"node":"dmm2","event":"ran","out":{"dmm2":{"opaque":"visa.session","resource":"SIM::DMM0","open":true},"raw":230.071217,"e2":null}}
{"seq":24,"t_ms":0.0,"frame":0,"node":"measured","event":"arm","arm":"true"}
{"seq":41,"t_ms":0.0,"event":"end","status":"ok"}
```

The example leaves lines out, and files from the first line.

- **The first line** names the schema, the run, the function and its mode, and each file the run read
  (the program, its types, the files of the libraries it calls) with its SHA-256, so that a viewer knows
  whether the trace matches the files it shows.
- **A frame** is one call of a function: the `call` event gives it a number, the number of the frame that
  called it (`parent`), the node of the caller and its `iter` there, the function, its inputs (`in`: name,
  type and value of each, in the order of the signature, as the outputs of a report), and the type of each
  value its body binds, by name (`next.n` for a carry), so that the values that follow decode alone
  (section 5). A name that two bodies bind with different types (two arms, spec 11) is left out of
  `types`, and each `ran` that binds it gives its type in a `types` of its own. Frame 0 is the top-level
  function. Every other event names its frame by number, so a line stays short however deep the calls are.
- **`node`** names the line as a terminal does (cli.md, section 4.3): its first output, its label
  (`#close`), or `@12` for a line with neither. A name that several bodies bind is told apart by `loops`, for
  bodies of two loops, and by the `arm` event that came before it in its frame and iteration, for two arms.
- **`iter`**, when the node is in loops of its function: the iteration of each, outermost first, from 0,
  after **`loops`**, the loop of each level, named as `node` names it (`"loops":["n_a"],"iter":[3]`), so that two
  loops that bind the same names (the two branches of `two_benches`) are told apart (decision of the owner,
  2026-11-10). A `for` is a loop here. With the `iter` of each `call` up the chain of frames, a value inside a subfunction is placed in the
  iteration of every loop around it.
- **`event`**: `ran` (the node ran; `out` gives each value it bound, by the name of the value, which is
  the name of its wire, as the node gives it: before a conversion that the line puts on it to bind it, spec 3.3, and
  of the type that `types` gives); `skipped` (an error on its `err` input, spec 7.2; `out` gives the defaults and the
  error passed on); `call` (a call to a function written in LabCraft begins: its nodes follow, then the
  `ran` of the calling node); `arm` (a `match` or `handle` took `arm`, written as its pattern); `set` (a
  panel element changed by `ui.set`, `set` or `run.set_element`: `element`, `value`, `by`); `truncated`;
  `end` (the last line, with the status; absent if the process died).
- **`t_ms`** is the clock of the run when the event happened: simulated in deterministic mode. Events come in
  the order the run made them: among the nodes ready at one time, the global order of spec 8, or the order drawn from
  `schedule_seed`.
- **Bounded** *(proposed figures)*. Values are cut as in section 5. A whole line stays within 16 MiB, its
  `types` and the `files` of the first line included: past it, the values of the line that are left show as
  `{ "$cut": true }`, then the entries of `types` and `files` that are left are dropped and the line says
  `"truncated": true`; a line that would still pass 16 MiB (a signature of hundreds of thousands of inputs) keeps
  only its `seq`, `t_ms`, `event`, `frame`, `node` and `"truncated": true`. So a reader that takes 16 MiB per line
  reads every line whole. After 1,000,000 events
  or 256 MiB, a line `{"seq":…,"t_ms":…,"event":"truncated"}` is written, then only `end`. A run in real time that lasts a day traces if
  asked, within that bound.
- **Where.** With `trace: file`, the trace of the last run of each function is
  `.labcraft/runs/<function>.trace.jsonl` under the root of the project, the function named from its folder
  (`dmm_driver.measure`), and `.labcraft/runs/<folder>/<function>.trace.jsonl` for a function run from a `folder` that is
  not the root (decision of the owner, 2026-11-10), so two functions never share a file. The folder starts with `.`, so no walk reads
  it as a library, and LabCraft writes a `.gitignore` holding `*` in `.labcraft/` when it creates that folder, so that runs
  leave nothing for git to see. From M1, that `.gitignore` holds `*`, `!.gitignore` and `!config.toml`, so that it and the settings of
  the project are versioned (section 9). A run or the journal that finds `.labcraft/` leaves its `.gitignore` as it is;
  a write of the settings of the project adds the lines it lacks, those there kept: a project made before has
  `.labcraft/` already. The run writes `<function>.trace.jsonl.<id>.partial` as it goes, where
  `<id>` is unique to the run (the process, the session and the run), which can be followed, and renames it over the
  last trace when it ends, so the trace of the last finished run is always whole, and two runs of one
  function at once never write the same file: the one that ends last stays. A run first removes the
  `.partial` files of its function left by a process that no longer runs, so a killed run leaves nothing
  for long. A process that writes a `.partial` holds the lock of `.<process>.lock` in its folder until it ends, and
  the system lets it go however the process ends: the `.partial` of a process whose lock is free, or missing, was left
  by one that no longer runs, and its lock file goes with it once no `.partial` of it is left there (decision of the
  owner, 2026-11-11). A `refused` run writes no
  trace, and the last one stays. A reader opens a trace so that it can be replaced while open (on Windows, with `FILE_SHARE_DELETE`);
  if the rename still fails, the trace stays as `.partial` and the report says so in `trace.error`. The
  viewer of M0v shows the values of the last run of the function it draws; an agent reads it with
  `run.trace` (section 6.4); `labcraft-cli run --trace FILE` writes elsewhere, its `.partial` beside `FILE`, and
  leaves the last trace of the function as it was.

## 9. Editing (M1)

M1 adds the commands that edit a function as a graph: add a node, wire, move, rename, arrange. Their catalog moved to
6.7 with step 0 of M1, where `cargo xtask commands` checks it against the registry (decision of the owner, 2026-11-15).
It registers the commands first as `planned` (section 1).

### 9.1 The editing model

What all editing commands share, decided with the owner on 2026-11-14 (spec, section 11;
[visual-notation.md](visual-notation.md), section 2.4).

- **The file is the truth.** A command that succeeds writes the canonical form of each file it touches (spec 5.2)
  at once: there is no unsaved state of a function and no "save". The IDE, the agent and the command line edit
  the same files through the same commands.
- **One history per project.** Every step is signed by its author: the user, the agent, or an external write.
  Undo (Ctrl+Z) undoes the last step, whoever made it, and the status bar says what it undid and whose it was;
  in a text field, Ctrl+Z stays in the field. A step restores the text of each file it touched byte for byte,
  even a text that was not canonical (written by an agent), from a snapshot of each file before the step. Redo
  replays it. A gesture is one step: a drag, a wire drawn, a rename with what follows it, a run of presses of
  Ctrl and an arrow on the same nodes (spec 11, notation 2.4 and 2.6).
- **External writes.** A file of the project that changes on disk, and whose hash is not that of the bytes LabCraft
  last saw, is read again, and the change becomes a step signed "external": undo goes back over it as over any other.
  Every program and simulation file of the project is followed from the first command, and those that appear: a
  `git checkout` is a step that undo takes back, a file it made included. Writes in a burst, of one file or of several,
  are one step: those whose files were modified less than 1 s apart, by the time the system gives them. The session
  looks before each command, and when the app says that files changed (decisions of the owner, 2026-11-15): a write
  made while no window watches the files, under `labcraft-cli mcp`, is a step before the next command reads the file.
- **A rename in the text.** When an external write, a `file.write` or a `text.save` changes a function, the session
  recognizes the values and labels that the new text renamed (spec 11) and moves their entries of `layout` to the new
  names, in their place in the text, in the same step: undo gives back the file as it was before the write. For an
  external write, the session writes the file again; a command writes that text, whose revision `text.save` returns
  (decisions of the owner, 2026-11-25).
- **Revisions.** The revision of a file is the hash of its bytes. Every editing command can carry
  `expected_revision`, per file: when it is stale, the command is refused, changes nothing and returns the
  current revision (`file.stale`, section 3), so that an agent that read a file before the user changed it
  cannot overwrite that change.
- **`batch`.** Several commands in one call, applied in order, written once at the end, one step of undo. A
  command that fails rolls the whole batch back, and nothing is written.
- **The agent and a gesture.** While the user is in a gesture (a wire being drawn, a field being typed in), an
  editing command of the agent waits for the end of the gesture, then applies to the result; a stale
  `expected_revision` then refuses it as above. It waits at most 30 s *(proposed)*, then is `gesture.busy`; until
  the gesture ends, the next ones are `gesture.busy` at once. A command that changes no file does not wait.
- **The `Text` panel.** The text of a function, edited by hand in the IDE, is written when it parses (grammar
  correct, with or without errors of types or of `check`, which the diagram shows), after about 0.5 s without a
  key or when the panel loses the focus; each such pause is one step (`text.save`, 6.7). A text that does not parse
  stays in the IDE, with a banner, and in the recovery folder; the file keeps the last text that parsed, and its
  diagram cannot be edited until the text parses again. When the file changes while the panel holds a text not yet
  written, the text stays in the panel and in the recovery folder, and a banner offers the file or the text; with no
  such text, the panel follows the file, and the change is one step of the undo of the panel (decisions of the owner,
  2026-11-21).
- **Incomplete is normal.** A node added without its required inputs writes a file that has `LC2101`: the
  diagram shows it, and wiring it removes the error. **Where the text cannot leave a place out** (a selector, an
  `until`, a `yield`, `collect`, `last`, `next x`, the initial value of a carry, a bound of a range, a value of a variant,
  of an array or of a variadic parameter, a field of a construction), and before a positional argument, which
  would otherwise move, a command writes a hole `?` there (spec 5.1), which is `LC2101` too. An input of a call left
  out is otherwise left out, as before; a removed order wire takes its name out of `after:`, and an empty `after:`
  goes. So no command ever writes a file that does not
  parse, and a value removed never shifts the positions after it (decision of the owner, 2026-11-15). Wiring the
  place, or setting a constant on it, fills the hole; a hole is not an occupied input.
- **Arrange.** One command, on the selection, a branch or the whole function: it removes the `layout` entries
  of what it arranges (spec 11), and the automatic placement places it again, around the fixed nodes outside
  the selection; one step. An agent calls it only when asked to: arranging is the user's choice.
- **A junction follows its trunk.** After every editing command, whoever calls it, the first point of a branch that
  lay on a wire of its value keeps its distance from the start of that segment, or goes when the trunk no longer has
  it (visual notation 2.5), in the same step; a branch whose entry the command wrote is as the command wrote it
  (decision of the owner, 2026-11-17). When the drawing after the command has no wire of the trunk's name (the command
  removed it, as `wire.disconnect`, `node.delete` and `node.set_template` do, or renamed its reader, as `value.rename`
  does), the other wires of the value in the same frame decide: one through the point keeps it, some but none through
  it drop it, none leave the branch its whole route (decision of the owner, 2026-11-25).
- **Settings.** The settings of a project are in `.labcraft/config.toml`, versioned with the project, and those
  of the user in the folder of the user (`craft-settings`); a key of the project wins over the same key of the
  user. The commands `settings.get` and `settings.set` read and write them, for the IDE, the agent and the
  command line; `labcraft.toml` stays the manifest of the libraries (D-25). In `.labcraft/`, git sees `config.toml`
  and the `.gitignore` only; the runs and the traces stay out of it (section 8). Section 9.7 lists the keys.

### 9.2 Naming what a command edits

An editing command uses the names of the `layout` section (spec 11), written the same way. So an agent that reads a
file and calls a command uses one vocabulary.

- **A function is its file.** An editing command takes `file`, the path of a `.lcf` of the project (section 2), or
  of a `.lct` for `type.rename`; a file holds one function or one type. A command that makes a file takes `path`. The shipped libraries are never written: `path.outside_project`. A file whose text does
  not parse cannot be edited as a graph: `file.unparsed`, and `file.write` or the `Text` panel mends it. A file with
  errors of types or of `check` is edited like any other.
- **A node** is named by its label (`#close`) or by the name of its line, and a named constant by its name (spec 11).
  It is also named by `@N`, the line where it starts in the file, from 1 as diagnostics count, as the reports write a
  node with no name (`docs/cli.md`, 4.3): `@41`, and `x -> @41.b` for a wire into it. So a node with no name, or with
  a name defined in two arms, is named without a label (decision of the owner, 2026-11-17). The line is that of the
  text as the file holds it; when canonical form would move a line, `@N` names nothing. The `layout` section has no
  such name (spec 11).
  A structure is a node. **A wire** is named by `value -> node` or `value -> node.input`, and **a terminal of the
  frame** by `input x` or `output x`.
- **An input**, which a command wires or sets (`to`), is named as the end of a wire in spec 11: `verdict`,
  `sq.b`, `total.sum` (the initial value of the carry `sum`), `n.stop`. A node alone names its only input, or its
  only input without a name, such as the selector of a `match`; with `arm`, the structure names the `yield` of
  that arm. There are five additions:
  - `next x` of a loop `n`, by `n.next.x`, since `n.x` names the initial value of `x`, and an outer value can
    reach both;
  - an input without a name, by its position among those of its node, from 0: `r.0` for the value of
    `r = Reading.Value(v)`;
  - a value of a variadic parameter, by its position in it, from 0 (`msg.parts.2`, its third value); the position
    just after the last one adds a value;
  - a `yield`, a `collect` or a `last`, by the output of the structure that it gives: `verdict`, for `verdict = match …`.
    For a `yield`, the command also takes `arm`, the patterns of its arm as the text writes them. A `collect` or a `last` whose
    output is also an output of the function is named by it no more: that name names the output;
  - the `until` of a loop `n`, by `n` alone or by `n.until`, which stays one name when `n` is also an output of the
    function (`until` is a keyword, so no carry bears that name); the selector of a `match` or a `handle` likewise by
    `m` or by `m.0`.

  An order wire ends at `node.after` (spec 5.3).
- **An output**, where a wire starts (`from`), is named as one of two things:
  - a value: a name that the body binds (an input of the function, a line, a carry, an `in` clause, a pattern),
    or a label for an order wire;
  - `node.output`, an output of a node by its name in the signature. If that output has no name (`_`), it gets
    one (N-14), which the result gives.
- **Names that fail.** A name that names nothing is `edit.unknown`, and `expected` gives the nearest names. A name
  that names several things, such as a name defined in two arms, is `edit.ambiguous`, and `expected` lists them: a
  label, its line (`@N`) or the name of the input settles it. A node that has no name, such as a call statement
  without a label, or a name defined in two arms, gets a label from `node.set_label`, which names the node by its line. The editor never
  makes such a node, since it names every node it places (N-14), but a text written by hand can.
- **A library that a command imports.** A command that adds a `use` line (`node.add`, `fragment.paste`, a type of a
  library written in a signature or a carry, `structure.surround` with a `match` on an enumeration of a library) makes
  the name of that library one that no value of the function can bear (spec R-1). Each value that bears it, a line, a
  carry, an `in` clause or a pattern, is renamed with the suffix of N-14 (`array` gives `array2`, or the next free
  name: the names that the command gives come first), its readers and its entries of `layout` following, in the same
  step; `renamed` lists it. A name bound in several bodies takes one new name in all of them, so that what hid what
  still does (R-3). For `fragment.paste`, the values of the file are renamed before the fragment comes in, to a name
  that the fragment does not bring either; a path of the fragment (`visa.State.Open`) names the library. An input or an
  output of the function that bears it is `name.taken`, and nothing is written: renaming it would change its callers,
  so `signature.rename` comes first (decision of the owner, 2026-11-23); so is a `name` that the caller gives, which is
  never renamed behind their back. A `use` that the file already had changes nothing.
- **Positions** are `[x, y]` in canvas units, in the frame of what they place (spec 11). They are integers in the
  range of spec 11; any other value is `params.invalid`.
- **Text of the language.** Where a command takes a constant (`240 V`, `"MEAS?"`, `Verdict.Pass`), a type
  (`f64[V]`) or a pattern (`1..=5`, `io.timeout(err)`), it is written as in a file and read by the grammar
  (spec 14). Text that does not parse is `params.invalid`, with the message of the parser. Where a parameter takes a
  value or a constant, a bare name is a value.

### 9.3 What an editing command returns

```json
{
  "step": 42,
  "files": [{ "file": "check_voltage.lcf", "revision": "…" }],
  "names": { "node": "reading2" },
  "renamed": [{ "file": "check_voltage.lcf", "from": "voltage", "to": "voltage2" }],
  "diagnostics": [],
  "summary": { "errors": 1, "warnings": 0 },
  "truncated": false
}
```

- **`step`** is the number of the step of history that the command made (9.1); `history.list` shows it. A command
  that changes nothing, such as a node moved to where it is, writes nothing and gives `"step": null`.
- **`files`** gives each file written and its new revision, which the next command can send as
  `expected_revision`. The revision is opaque text: the SHA-256 of the bytes of the file, `sha256:` and 64 lowercase hexadecimal
  digits ([architecture.md](architecture.md), 12.5).
- **`names`** gives what the command named: the node it added, the output that got a name. The next command can
  name it without reading the file.
- **`renamed`** gives each value that the rules of the gestures renamed as well (notation 2.4: a value unwired from
  an output of the function takes back the name of its node). So an agent never loses track of a name.
- **`diagnostics`** gives those of the files written, and of each other file whose diagnostics the command
  changed: a caller that a new required input leaves with `LC2101`. They come from the check of the root that
  follows every editing command, as after `file.write` (6.2). There are at most 200 diagnostics *(proposed)*; after that, the
  result says `truncated`. **`summary`** is that of the whole root (cli.md, section 4.2).

### 9.4 The catalog

The catalog of the editing commands is in 6.7, where `cargo xtask commands` checks it against the registry (decision of
the owner, 2026-11-15).

### 9.5 The agent and the work of the user

The agent and the user share the history, but the agent never undoes or replaces the user's work without
saying so (decision of the owner, 2026-11-15).

- **The author of a call** is its client. A command from the window of the IDE is the user's. A command from an MCP
  client, attached to the IDE or not, is the agent's; two MCP clients are two authors, each its own. A client is
  named by the `clientInfo.name` it gives at `initialize`; one with no name, or named `user` or `external`, is
  `agent`, so that a step of an agent is never shown as the user's. Two clients of one name attached to a window at
  once are numbered: the second is `agent 2` (or `claude 2`), the third `agent 3`.
- **Undo.** Called by an agent, `history.undo` undoes the last step only when that step is its own. Otherwise it is
  `history.not_yours`, and the message names the author and the label of that step. Called by an agent,
  `history.redo` redoes only a step that this agent undid, so that it never brings back what the user chose to
  undo. The user's Ctrl+Z undoes the last step, whoever made it (9.1).
- **Replace.** A wire or a constant on an occupied input is written only with `replace`. The agent passes it when it
  means to replace what is there, never by default.
- **Arrange** is called by the agent only when it is asked to (9.1).
- **The command line** runs one command per process, with no history to undo. In `labcraft-cli`, `history.*` is
  `command.not_enabled`. Its writes reach an open IDE as external writes (9.1).

### 9.6 Surrounding with a structure

`structure.surround` puts nodes in a new structure, as one step of undo.

- **Where.** The `nodes` are in one frame: the body of the function, of a loop, or an arm. Nodes of several frames
  are `params.invalid`. The structure goes in that frame, around them. Their entries in `layout` are rewritten in its
  frame, and the structure gets a `node … at` entry where it stands, so that the nodes surrounded do not move on the
  screen: a node that was free gets an entry where it was drawn, and a wire between two of them keeps its route, in the
  new frame; the route of a wire that now crosses the edge of the frame goes, and the automatic routing draws it. The
  nodes around that are placed automatically are placed again, as after any change of the file (notation 2.1). Its size
  is automatic (spec 11).
- **An empty structure.** With no `nodes`, the structure is empty. It goes in the frame that `inside` and
  `inside_arm` name, as `inside` and `arm` do for `node.add` (the body of the function by default), at `at`, or placed
  automatically without it. It gives nothing (spec 6.1, 6.2): its target is `_`, each arm of a `match` or a `handle`
  has empty braces, a `for` has an empty body, and a `loop` has `until ?`. With `nodes`, `inside`, `inside_arm` and
  `at` are `params.invalid`: the frame and the place are those of the nodes (decision of the owner, 2026-11-18).
- **The head of each kind:**
  - `match` needs `selector`, a value. Its arms are those that make it complete: `true` and `false` for a boolean,
    each variant of an enumeration, and `_` alone for an integer or a string. A variant that holds values binds
    each of them under a name from its type, as N-14 names a value (spec 6.1): the name of a value of its type
    (`step` for `Step`), `value` for a type of 3.1, numbered when taken.
    A selector of a type that a `match` does not take (spec 6.1: a float, a struct) is `wire.type_mismatch`; a value
    with no type yet gets the single arm `_`.
  - `handle` needs `selector`, an error value. Its arms are `ok` and `_`.
  - `for` needs `over`, an array value or a range (`0..n`, each bound an integer or a value). Its clause takes a name
    from the value (N-14): `readings` gives `reading`, a value whose name does not end in `s` gives `item`, and a range
    gives `i`, each numbered when taken.
  - `loop` needs neither. Its `until` is a hole until it is wired (9.1).
  - `arm` names the arm that receives the nodes, by its patterns; by default, the first, in the order above. A
    `selector` or an `over` that a kind does not take is `params.invalid`, and so is a `selector` or an `over` that the
    frame cannot see.
- **What crosses the frame.** A value that the nodes read from outside is passed in (spec 6), and nothing is written
  for it. A value that the nodes define and that is read outside becomes an output:
  - in a `match` or a `handle`, the arm that holds the nodes yields it, and the other arms yield a hole for it
    (9.1). When the nodes give nothing outside, the structure gives nothing: its target is `_`, and its arms have no
    `yield` (spec 6.1);
  - in a loop, it becomes a `collect` output, an array of its values. A reader that does not take an array shows
    `LC3001`.

  The output keeps the name of the value, so that what reads it outside, an output of the function included, does not
  change; inside, the value takes a suffix (N-14: `b2`, `e2_2` for a name that ends in a digit, and `e2_3` for `e2_2`), and its readers there follow, which `renamed` lists. A value that was
  bound to an output of the function takes inside the name that inserting its node gives it (notation 2.4).
- **The error chain.** Sometimes exactly one value of type `error` enters the nodes and exactly one leaves them. In a
  `match` or a `handle`, that output is then given by the other arms from the error that entered (`yield e1`). In a
  loop, it becomes a carry of type `error` (`carry err = e1`, `next err = e4`), named `err`, numbered when taken: the
  nodes that read the error that entered read the carry, and what read the error that left outside reads the output of
  the carry. That carry stops a `loop` on an error (spec 6.3); a `for` runs to its end.
- `names.structure` gives the name of the structure as a command names it (a label when it gives nothing: `#match_node`,
  spec 11), and `names.outputs` its outputs, in order.

### 9.7 The settings

- **Two stores.** The project's is `.labcraft/config.toml` (9.1). The user's is `settings.toml`, in the folder that
  `craft-settings` gives LabCraft on that computer, beside the `config.toml` where the window keeps its layout and
  the projects opened last; on the web, the storage of the page (architecture 12.9). A key of the project wins over the same key of
  the user; a key in neither has its default. The session reads both when it opens; a store changed by another
  process meanwhile is read at the next opening. A write changes only its key, in the file as it is then: what another
  process wrote there stays.
- **A key is `group.name`**, and in a file, the key `name` of the table `[group]`: `[editor]` then `drag_pans = true`.
  What else a file holds (comments, keys LabCraft does not know) stays as written. A key of the table below whose
  value has the wrong type, or that a project may not hold, is ignored where it is written. A project's file that does
  not read as TOML gives no key, and `settings.set` does not write it (`file.unparsed`) until it is mended.
- **What a project may hold** is said by each key (decision of the owner, 2026-11-20): what one person sees (the
  theme, the colours, motion, what the canvas shows) is theirs, and a project never imposes it; how the editor reacts
  to a gesture can be a rule of the team, so a project may fix it.
- **Who follows them.** The window, and what it draws: a drawing that a command gives (`diagram`, 6.5) is that of the
  code, whatever the settings. An agent reads and writes them as the window does; `settings.set` is an edit for its
  policy (ide.md 8). `labcraft-cli exec` and `mcp` read and write the store of the user, but do not follow it: what
  they draw, and the places their edits write, start from the defaults and follow only the keys of the user that a
  command of that process sets, so that a command gives the same file on every computer (decision of the owner,
  2026-11-20).
- **Bounds** *(proposed)*: a store holds at most 1 MiB, and one that holds more gives no key and is not written;
  `appearance.library_colors` names at most 1,000 libraries, each name of 1 to 256 bytes. Taking out the last key of a
  table takes the table out, unless a comment stands above it.

| Key | Values | Default | Project | What it does |
|---|---|---|---|---|
| `appearance.theme` | `light`, `dark` | `light` | no | The theme of the window and of the canvas (ide.md 2) |
| `appearance.look` | `studio`, `lab` | `studio` | no | The look of the window (ide.md 2) |
| `appearance.reduced_motion` | boolean | `false` | no | No glide (visual notation 2.1) |
| `appearance.block_fill` | `code`, `neutral` | `code` | no | `neutral` draws every block in the neutral tokens, whatever its `color`, `fill` and `stroke` (spec 11.2) |
| `appearance.library_colors` | a table: library → `"#RRGGBB"` | `{}` | no | The `color` of a library's blocks, in place of that of its mask (spec 11.3), from which the theme derives the band and the tint in both themes, as for a `color` written; a function's own `color` stays |
| `appearance.color_by_type` | boolean | `true` | no | The data wires, their terminals and their names in the colour of their family (visual notation 4.3) |
| `view.standard_everywhere` | boolean | `false` | no | Every node in standard view (visual notation 3); `view.set_standard_view` writes it |
| `view.values_shown` | boolean | `true` | no | The values of the last run on the wires (visual notation 6.1) |
| `view.preview_tabs` | boolean | `true` | no | A single click on a file opens it as a preview (ide.md 3) |
| `editor.drag_pans` | boolean | `false` | yes | A drag on the empty canvas pans; off, it selects with a rectangle (visual notation 2.6) |
| `editor.fix_every_node` | boolean | `false` | yes | A move fixes every free node of the function (visual notation 2.4) |
| `editor.mark_fixed` | boolean | `true` | no | A pin on each fixed node (visual notation 2.4) |
| `editor.replace_with_alt` | boolean | `false` | yes | A wire dropped on an input that holds something replaces it only with Alt down (visual notation 2.4) |
| `editor.accept_mismatch` | boolean | `false` | yes | A wire of the wrong type is written, and `check` gives `LC3001` (visual notation 2.4) |
| `editor.drag_on_wire` | `route`, `branch` | `route` | yes | What a drag on a wire does: `route` moves its segment (`wire.route`), and Alt with it starts a branch; `branch` swaps the two (visual notation 2.6) |
| `editor.drag_between_frames` | `move`, `stay` | `move` | yes | What a drag of nodes released over another frame does: `move` moves them into it (`node.reframe`), and Shift with it keeps them in their frame; `stay` swaps the two (visual notation 2.4) |
| `editor.route_with_shift_alt` | boolean | `false` | yes | Shift+Alt in place of Alt, for the drag on a wire with the modifier and on a terminal of a block (visual notation 2.6) |
| `editor.autopatch` | boolean | `true` | yes | Autopatch from the keyboard (visual notation 2.6) |
| `editor.into_wire` | boolean | `true` | yes | Inserting into a wire (visual notation 2.6) |
| `run.record_values` | boolean | `false` | no | A run records its values, its trace, for the wires (ide.md 3) |
| `ai.enabled` | boolean | `true` | no | The switch of AI: off, no agent attaches to the window, and the window shows nothing of AI (ide.md 8) |
| `ai.policies` | a table: MCP client → canonical folder of a project → `ask`, `edits` or `all` | `{}` | no | The policy the user kept for a client on a project, which answers its first connection without asking (ide.md 8) |

The settings of AI and of the agents (`ai.*`, ide.md 8) are the user's: an agent neither reads nor writes them
(`agent.not_allowed`), and `settings.get` without a key leaves them out for it. Where the
panels are, the detached windows and the insertions of the user are the state of the window, kept with it, not keys.

### 9.8 Making a function from a selection

`function.extract` moves nodes into a new function, and puts a call to it in their place, as one step of undo over
both files (decisions of the owner, 2026-11-24).

- **The nodes** are in one frame, as for `structure.surround` (9.6): the body of the function, of a loop, or an arm;
  a structure goes with all it holds. Nodes of several frames, or none, are `params.invalid`, and so is a `next`, a
  `collect`, a `last` or an `until`, which stays in its loop (R-6). So is a node that reads or writes a panel element
  (`ui.get`, `ui.set`, spec 10): the elements belong to their call (spec 8.2), and in a new function the node would
  reach other ones (Q-24).
- **The new file** is `name.lcf`, in the folder of `file`: the same library, so the call needs no `use`. It holds the
  `use` lines that its body needs, the signature, and the lines of the nodes in the order of the text, what each
  holds (constants, the nodes inside a structure) and the order wires between them; no documentation comment. A
  `name` that is not an identifier or is a keyword is `name.invalid`; one that a function or a type of the library
  already bears, that names a library that the file imports, or that the caller gives a value or a generic variable,
  which would read the call as that (as for `function.rename`), is `name.taken`; an existing file is `path.exists`.
  Nothing is written then.
- **Inputs.** Each value that the nodes read and that is bound outside them (an input of the function, a line, a
  carry, an `in` clause, a pattern) is an input, in the order of the text where the nodes first read it, named as
  the value, with no default, of the type that `check` gives it; the new file imports the library that such a type
  names (`visa.session`). A value whose type is not known yet, or whose type names a generic variable of the caller,
  is `params.invalid`, and the message names it: wiring it or typing it comes first. An order wire from outside (`after: #beep`) goes to the call.
- **Outputs.** Each value that the nodes bind and that is read outside them, an output of the function included, is
  an output, in the order of the text, named as the value: the call binds the same names, so what reads them outside
  does not change. A labelled node of the selection named by an order wire from outside (`after: #close`) gives its
  label to the call; two such labels are `params.invalid`. When nothing leaves, the call has no output and gets a
  label, `#name`, numbered when taken (N-14). A node outside that reads what the nodes make and that they read is
  `wire.cycle`: the call would read its own output.
- **The error chain**, as in 9.6: when exactly one value of type `error` enters the nodes and exactly one leaves
  them, and no other value of the new function bears `err`, they become `err: error = none`, the last input, and
  `err`, the last output (spec 7). Inside, the readers of the error that entered read `err`, which `renamed` lists; the
  error that leaves is renamed `err`, or, when the nodes also read it, given by a line `err = e2`, since a reader of
  `err` reads the input (R-2).
- **The call** is one line, where the first line of the nodes was: its positional arguments in the order of the
  inputs, `err:` named, as the examples write it (`raw, dmm2, e2 = read_dmm(resource, err: err)`).
- **Places.** In the caller, the entries of `layout` of the nodes and of the wires between them leave, and the call
  gets a `node … at` entry at the top-left corner of the box of the nodes as drawn, so that it stands where they were;
  the route of a wire that crossed the edge of the selection goes, and the automatic routing draws it. In the new
  function, each node gets an entry, the nodes keeping their places relative to one another, the top-left corner of
  their box where the automatic placement of that function puts the top-left corner of its nodes, and the wires
  between them keep their `via` points, as for `fragment.paste`.
- **What stays.** The `use` lines of the caller stay, as after `node.delete`. The callers of the caller do not
  change: its signature does not.
- `names.function` gives the name of the new function, `names.file` its file, and `names.node` the call, as a command
  names it; `files` gives both files. Undo removes the new file and gives the caller back, byte for byte (9.1).
  Like `function.create`, it makes a file: `commands.batch` refuses it.

## 10. Left for later

Each can be added without breaking anything above: the other commands of the viewer (M0v: zoom, pan), probes and breakpoints (M5),
scripts of commands in a file, run by `labcraft-cli script` (D-18; `run` runs
a program), the journal of commands (M1, step 8 of its specifications), pasting with the wires from outside kept (notation 2.6), taking nodes out
of a structure, reordering a signature, moving a function to another library, editing the fields of a type (the text
does it in M1), crash reports that replay the commands
that led to a bug (a Craft rule). Real instruments and the consent they need are specified for 0.5.0 (M4,
[io.md](io.md)): they add no command, only the effect `hardware` (section 1).
