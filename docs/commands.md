# Commands: the catalog v0

*Last updated: 2026-11-10. Status: draft for milestone M0b, implemented in `engine` but for the runs of section 6.4
(step 6b), `guide.read` (step 9) and the commands of section 6.5, which are registered as `planned`. The desktop viewer
of M0v answers the `view.*` commands through a session of its own until they move into `engine`
(`docs/architecture.md`, 11.5): all of them but `view.open_trace`, which comes with the values of a run.*

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
| `effects` | What it can change, among `files` (writes in the project), `instruments` (talks to an instrument) and `run` (starts, steers or stops a run). A command with none only reads |
| `long` | `true` when it can outlast a call: it returns a task to follow (section 4) |
| `planned` | Present when the command is registered but not built yet: the milestone or the issue that brings it. Such a command is never enabled, and says why |

`enabled`, as in the Craft apps, is a state, not a field of the description: `commands.list` gives it
with each command, and a call to a command that is not enabled gets `command.not_enabled` and the
reason (`run.set_element` in deterministic mode, `view.zoom` before M0v). A planned command lets a milestone
register its commands before it builds them, as SolidCraft does with `planned!`: the catalog, the menus
and the agent see them, greyed, with the reason.

`cargo xtask commands` checks the registry against the tables of section 6, as `cargo xtask codes`
checks diagnostic codes against cli.md *(proposed)*. It also refuses two ids that would give one MCP tool
name, `a.b_c` and `a_b.c` ([mcp.md](mcp.md), section 2).

## 2. Calling a command

- **Parameters are a JSON object.** A missing parameter takes its default, or is an error if it has
  none: a command never asks a question (no dialog on a programmatic call). An unknown parameter is an
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
  libraries are out of reach, and so is a user library's manifest when M2 brings it.
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
                 "max_iterations", "max_nodes", "max_call_depth", "max_memory", "max_real_time",
                 "deny_warnings", "trace", "wait"]
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
| `internal` | A bug of LabCraft, caught: `message` says where. The session goes on |

A request too large to read is not a command error: no command was reached ([mcp.md](mcp.md), section 5).

## 4. Long commands: runs

A run can last for days on a bench, or a few milliseconds of real time in deterministic mode. It follows
the task model of SolidCraft (`render.start`, `render.status`, `render.cancel`), with one change for
agents: **`run.start` waits**. It returns the report of the run if the run ends within `wait` (30 s by
default, at most 300 s *proposed*), which is what nearly every deterministic run does, so that one call
runs a program and reads its result. Otherwise it returns

```json
{
  "schema": "labcraft-run/1", "status": "running", "run": "r3", "function": "two_benches",
  "mode": "real_time", "outputs": [], "clock": { "real_ms": 30000 },
  "resources": { "opened": 2, "open": [], "leaked": [] },
  "loops": [ { "function": "two_benches", "node": "mean_a", "iteration": 4 } ],
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
encoding stays short:

| Type | JSON | Example |
|---|---|---|
| `bool` | a boolean | `true` |
| integers | a number, exact; an `i64` or `u64` beyond ±(2^53−1) as a string of digits | `12`, `"9007199254740993"` |
| `f32`, `f64` without a unit | a number: the fewest digits that read back the same, in decimal notation from 10^-5 up to below 10^16 and with an exponent outside, written as the language writes it (`1.5e-7`, `1e16`); a float with no fraction keeps `.0`, so that it is not taken for an integer; `"nan"`, `"inf"`, `"-inf"` | `0.5`, `12.0`, `0.0015` |
| a number with a unit | a string, as the language writes a constant, in the unit of the type: the number as above, a space, the unit | `"230.03 V"`, `"-0.0015 A"`, `"-3 mV"` |
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
              "callers": [ { "function": "check_voltage", "node": "raw", "file": "check_voltage.lcf", "line": 15 } ] },
  "others": []
}
```

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
  lines, resources in the order they were opened, loops outermost first. Resources are in code point order
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

What an agent needs to write, check, run and fix a program, and nothing it does not. Each command is an
MCP tool ([mcp.md](mcp.md), section 2). Parameters in *italics* are optional.

### 6.1 Learning the language

| Command | Parameters | Returns |
|---|---|---|
| `guide.read` | *`skill`*, *`file`* (`SKILL.md`) | The skills of `skills/`, from the copies built into LabCraft: without `skill`, each skill with its name and the line that says when to use it, followed by the `SKILL.md` of the skill that writes a program, so that one call starts the loop; with one, that file of it, in Markdown. The same files that a client which supports skills installs, so there is one text to keep right. Never reads a path from disk; an unknown skill or file is `params.invalid`, with the choices in `expected`. No skill holds the programs of the acceptance scenarios |
| `functions.list` | *`library`*, *`filter`*, *`docs`* (`false`) | Every function and type the project can call: primitives (`core`, `string`, `array`, `errors`, `time`, `ui`), shipped libraries (`visa`, `scpi`), and the project's own. Each as a signature written in the language, as appendix A writes it (`fn visa.open(resource: string, timeout: duration = 2 s, err: error = none) -> (session: visa.session, err: error)`), with `always`, its error categories, and the first line of its documentation, or all of it with `docs`. `filter` keeps the names that contain it |

`functions.list` answers "what can I call" in one call: an agent writing its first program calls it
once, with no filter. The project's own entries come from the last `check`, so they agree with it. It
lists at most 1,000 entries *(proposed)*, primitives and shipped libraries first, then says `truncated`:
`library` or `filter` narrow it. Each entry is `{ "name", "library", "kind", "signature" }`, `kind` being
`function` or `type`, with `always`, `categories` and `doc` when they have something to say, each line of `doc` cut
past 200 code points, as a message is; `library` is `""` for the root, whose functions are named without one; a type of a
library is named by it in the signatures (`session: visa.session`), as a program that uses it writes it. The
categories of a function of a library are those its manifest declares for the library, which say no more.

### 6.2 Files and libraries

| Command | Parameters | Returns |
|---|---|---|
| `project.describe` | | The root, its libraries (folders, sub-libraries), their `.lcf` and `.lct` files with what each declares, the simulation files with the resources they define (those of the project, and those shipped with `visa`, as `<labcraft>/visa/sim/…`), and the summary of a `check` of the root (cli.md, section 4.2), without its diagnostics: `{ "root", "libraries": [{ "library", "folder", "files": [{ "file", "declares" }] }], "simulations": [{ "file", "resources", "valid" }], "summary", "truncated" }`, the root being the library `""` in the folder `.`. At most 1,000 files listed *(proposed)*, and at most 8 MiB of `.toml` files read to find the simulation files *(proposed)*, those over `--max-file-size` not counted, then `truncated` |
| `file.read` | `path`, *`from`* (1), *`lines`* (all) | The text of a `.lcf`, `.lct` or simulation file of the project or of a shipped library, from line `from`: `{ "file": …, "text": …, "from": 1, "lines": 120, "lines_total": 120, "truncated": false }`. At most 1 MiB of text per call *(proposed)*, counted as JSON writes it, cut at the end of a line, then `truncated`: the next call starts at `from + lines`. A line longer than that is cut at the end of a character, and counts as read |
| `file.write` | `path`, `text`, *`check`* (`true`) | Writes the file, atomically, creating its folder if needed. With `check`, then checks the root and returns its report (`labcraft-check/1`), as `library.check` would: one call writes and checks. `text` holds at most 4 MiB *(proposed)*, whatever `--max-file-size`, so that a request stays within the bound of [mcp.md](mcp.md), section 5; a larger file is `file.too_large` |
| `file.rename` | `from`, `to` | Renames a file in the project, a change of case alone included (R-22: the fix of `LC2023` can be a new name), then checks as `file.write` does. A final `/` or `.` of `to` names no file: `b.lcf/` is `b.lcf` |
| `file.delete` | `path` | Deletes a `.lcf`, `.lct` or simulation file of the project, then checks |
| `library.check` | *`folder`* (the root), *`deny_warnings`* (`false`) | `labcraft-check/1`, exactly what `labcraft-cli check --json` gives (cli.md, section 4.2) |
| `file.format` | `path` (a file or a folder), *`check_only`* (`false`) | `labcraft-fmt/1`, exactly what `labcraft-cli fmt --json` gives |

`file.write` writes the text as given, but for one change: CR LF becomes LF, as the repository keeps
its programs (cli.md, section 1 reads both). It does not reformat, so that the next edit of an agent finds
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
| `run.start` | `function`, *`folder`* (the root), *`sim`*, *`args`* (`{}`), *`set`* (`[]`), *`mode`* (`deterministic`), *`schedule_seed`*, *`timeout`*, *`max_iterations`*, *`max_nodes`*, *`max_call_depth`*, *`max_memory`*, *`max_real_time`*, *`deny_warnings`* (`false`), *`trace`* (`file`), *`wait`* (30 s) | Checks the library, then runs the function and returns `labcraft-run/1` (section 7), or `running` (section 4). `function` is named from the root (`check_voltage`, `dmm_driver.measure`); `args` by input name (section 5); `sim`, a simulation file; `set`, panel elements and the time on the clock of the run at which each is set, `[{ "element": "stop_switch", "value": true, "at": "5 s" }]` (Q-20, cli.md, section 6.4); `schedule_seed`, the seed of the order among ready nodes (scenario 3); `trace`: `file` (the default, so that whoever watches the viewer sees the values of every run on the wires), `inline` or `none` (section 8) |
| `run.wait` | *`run`*, *`wait`* (30 s) | The final report, or `running` again |
| `run.status` | *`run`* | The report so far, at once: `running` with the clock and where each loop is, or the final report |
| `run.set_element` | *`run`*, `element`, `value` | Sets a panel element of the running top-level function (spec 10, Q-20), in real time only: in deterministic mode the time a call arrives would date the change, and a run would no longer replay, so a change is given to `run.start` by `set`, at its time on the simulated clock, and this command is not enabled. The element and its value follow the rules of `--set` (cli.md, section 6.4); the change is seen by the reads that start after it. The run is going, so a setting that `--set` would refuse is a command error, `params.invalid`, whose `message` gives the code of cli.md, section 6.6 (`LC8108`, `LC8104`…), and the run goes on |
| `run.abort` | *`run`* | Aborts the run (spec 8.2) and returns its final report, status `aborted`. A run that has already ended is left as it is, and its final report is returned with its own status (section 4) |
| `run.trace` | *`run`* or *`function`*, *`folder`* (the root), *`frame`*, *`node`*, *`event`*, *`from`* (1), *`limit`* (500) | Reads the trace written by a run (section 8), in pages: the events from number `from` that match the filters, at most `limit` (at most 2,000 *proposed*), with the first line of the trace, the `call` events of the frames they belong to, so that every value decodes, and `next`, the number to give as `from` for the next page, or `null`. `run` reads the trace of that run, or the one going; `function`, the last trace of that function run from `folder`; neither, the run of the session. `node` keeps the events of the nodes of that name, `frame` those of one frame, `event` those of one kind. A run with `trace: none`, or whose trace is no longer the last of its function, gives `run.unknown` with the reason |

`run.trace` is how an agent debugs a run without reading the whole trace, which can reach 256 MiB:
"what did `raw` bind in each iteration" is one call with `node: "raw"`. It reads the same file as the
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

Registered as `planned` until M0v brings them (section 1), then enabled. None writes a file; the `view.*` commands
change only the state of the viewer, which the session holds.

| Command | Parameters | Returns |
|---|---|---|
| `diagram.svg` | The options of `labcraft-cli diagram` ([cli.md](cli.md), section 7), less the file it writes to | The SVG of each function asked for, the same as `labcraft-cli diagram --svg` gives when it fits, and its report `labcraft-diagram/1`, so that an agent sees the drawing of what it wrote (D-18). An SVG that does not fit in the bound of a result (section 2) is `{ "$cut": true }`, and the result says `truncated`: an agent can ask for one function at a time, or use `labcraft-cli diagram`, which writes files and has no such bound |
| `view.open` | `folder` (a library folder, as the `FOLDER` of `check`), *`lib_path`* | The functions of the folder, each with its file and whether it is drawable, and the report of `check` (`labcraft-check/1`). The viewer shows it in place of the folder open before, and watches its files |
| `view.show_function` | `function` (a function of the open folder, named as the `FUNCTION` of `diagram`, cli.md 7) | Draws it in the viewer, as its file gives it (cli.md 7.1); returns its entry of `diagrams` in `labcraft-diagram/1` (function, file, drawn, nodes, codes). An unknown function is an error of the command |
| `view.set_standard_view` | `on` (boolean) | The preference of the IDE that shows every node in standard view (notation 3), kept with the preferences; returns the value it had. Changes no file |
| `view.set_state` | `function`, `state`: the view state of notation 2.1 that the file does not hold, in part, as `{ "arms": { KEY: N }, "unfolded": { KEY: true }, "standard": { KEY: false } }`, each part optional, `KEY` the key of a node of the drawing that follows it across a change of the file (per structure: the branch shown in tabs or unfolded; per construction: folded or not; per node: standard view or not, optional inputs shown; per loop: the iteration chosen, notation 6.1) | The drawing's entry of `diagrams` in `labcraft-diagram/1`. A click on a tab, a fold or an iteration selector goes through it |
| `view.open_trace` | `file` (a trace, `labcraft-trace/1`) | Shows the values of that run on the function shown (notation 6.1), read with the limits of cli.md 7.4; returns the `trace` part of `labcraft-diagram/1` (file, events, status, cut, stale, ignored) |

### 6.6 The registry itself

| Command | Parameters | Returns |
|---|---|---|
| `commands.list` | *`filter`* | Each command: `id`, `summary`, `effects`, `long`, `enabled` |
| `commands.describe` | `id` | Its full description (section 1) |

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
| `fault` | no | When `status` is `fault`: `code` (an `LC` code, family `LC80`, cli.md, section 6.6), `message`, `function`, `node` (`null` for a fault at no node), `file`, `span`, `loops`, the loops around it as below, at most 1,000 *(proposed)*, then `loops_more`, and for `LC8001` `iteration`, that of the loop itself which would pass the limit, from 0 |
| `abort` | no | When `status` is `aborted`: `by` (`request`, `timeout`, `interrupt`), `at_ms` on the clock of the run, `loops` (as below, when the abort came), and `warning`: the instruments are in an unknown state. When a native call was in progress, the run ends after `at_ms`: when that call ends, or when the grace period ends if it is abandoned (spec 8.2); `clock` says when |
| `loops` | no | When `status` is `running`: each loop going, outermost first, with its `function`, its `node` (section 8) and its `iteration`, from 0 |
| `clock` | yes | `simulated_ms` in deterministic mode; `real_ms` always |
| `resources` | yes | The resources the run opened (spec 8.1: a session of `visa`, or any opaque type a library declares as one). `opened`: how many; `open`: those still open at its end, each with its `type` (`visa.session`), what it designates (`resource`: `SIM::DMM0`) and where it was opened (`function`, `node`, `file`, `span`), which scenario 1 wants empty; `leaked`: those an abort could not release (spec 8.2), the same way. At most 1,000 of each *(proposed)*, then `"truncated": true` |
| `instruments` | no | With a simulation: per resource, in code point order of the resource names so that two seeds give the same list, the commands it received in order, dated, with their answers, each text cut as a string is (section 5). At most 64 resources and 1,000 commands per resource in the report *(proposed)*: past them, a resource says `"truncated": true` and how many commands it leaves out in `more`, and `instruments_more` after the list how many resources; the bound of a result (section 2) can shorten it further |
| `logs` | no | What libraries kept of the values a run gave them, such as `results_log.append_csv` (spec A.4), which writes no file in M0: `{ "entries": [ … ], "truncated": false }`, one entry per call, in the order of the calls, `{ "library": "results_log", "function": "append_csv", "t_ms": 0.0, "type": "[results_log.StepResult]", "value": … }`, the value as in section 5, with the type of the input of the signature it was given on. At most 1,000 entries for each library *(proposed)*, and what the libraries of a run keep for its report, the commands of `instruments` included, within 16 MiB *(proposed)*, or `--max-memory` when lower, a value counting its logical size (cli.md 6.2): the entries after them are counted, not kept, and `truncated` is then `true`, with their number in `more`, so that the log of each library, as the commands of each instrument, is the start of what it was given, and what they keep stays within that bound. Entries of two libraries come in the order of `t_ms`, then of the libraries. The bound of a result (section 2) can shorten it further |
| `panel` | no | Each setting of `set` or `run.set_element`, in the order applied: `element`, `value`, `at_ms` on the clock of the run, `by` (`set`, `run.set_element`), and `applied` (`false` for a setting after the end of the run) |
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
{"seq":1,"t_ms":0.0,"event":"call","frame":0,"function":"check_voltage","in":[{"name":"resource","type":"string","value":"SIM::DMM0"},{"name":"low","type":"f64[V]","value":"220.0 V"},{"name":"high","type":"f64[V]","value":"240.0 V"},{"name":"err","type":"error","value":null}],"types":{"dmm":"visa.session","e1":"error","raw":"f64","dmm2":"visa.session","e2":"error","measured":"Verdict"}}
{"seq":2,"t_ms":0.0,"event":"call","frame":1,"parent":0,"node":"dmm","function":"visa.open","in":[…],"types":{…}}
{"seq":5,"t_ms":0.0,"frame":0,"node":"dmm","event":"ran","out":{"dmm":{"opaque":"visa.session","resource":"SIM::DMM0","open":true},"e1":null}}
{"seq":6,"t_ms":0.0,"event":"call","frame":2,"parent":0,"node":"raw","function":"scpi.query_f64","in":[{"name":"session","type":"visa.session","value":{"opaque":"visa.session","resource":"SIM::DMM0","open":true}},{"name":"command","type":"string","value":"MEAS:VOLT:DC?"},{"name":"err","type":"error","value":null}],"types":{"text":"string","s":"visa.session","e1":"error","value":"f64","e2":"error"}}
{"seq":9,"t_ms":0.0,"frame":2,"node":"text","event":"ran","out":{"text":"230.071217","s":{"opaque":"visa.session","resource":"SIM::DMM0","open":true},"e1":null}}
{"seq":12,"t_ms":0.0,"frame":0,"node":"raw","event":"ran","out":{"raw":230.071217,"dmm2":{"opaque":"visa.session","resource":"SIM::DMM0","open":true},"e2":null}}
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
  the name of its wire); `skipped` (an error on its `err` input, spec 7.2; `out` gives the defaults and the
  error passed on); `call` (a call to a function written in LabCraft begins: its nodes follow, then the
  `ran` of the calling node); `arm` (a `match` or `handle` took `arm`, written as its pattern); `set` (a
  panel element changed by `ui.set`, `set` or `run.set_element`: `element`, `value`, `by`); `truncated`;
  `end` (the last line, with the status; absent if the process died).
- **`t_ms`** is the clock of the run when the event happened: simulated in deterministic mode.
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
  leave nothing for git to see. The run writes `<function>.trace.jsonl.<id>.partial` as it goes, where
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

## 9. Left for later

Each can be added without breaking anything above: commands that edit the graph rather than the text
(M1: add a node, wire, move), the other commands of the viewer (M0v: zoom, pan), probes and breakpoints (M5), batches of commands in one call with a rollback
(SolidCraft's `batch`), scripts of commands in a file, run by `labcraft-cli script` (D-18; `run` runs
a program), undo and the journal of commands (M1), crash reports that replay the commands
that led to a bug (a Craft rule), real instruments and the consent they need (M4, [mcp.md](mcp.md),
section 6).
