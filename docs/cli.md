# Command line: `fmt`, `check`, `run` and `diagram`

*Last updated: 2026-11-08. Status: `fmt` and `check` implemented for milestone M0a; `run` a draft for
M0b (section 6), and `mcp` in [mcp.md](mcp.md); `diagram` a draft for M0v (section 7).*

This document is the contract of `labcraft-cli fmt`, `labcraft-cli check`, `labcraft-cli run` and
`labcraft-cli diagram`: their arguments, exit codes, reports and diagnostic codes. An agent reads the reports below to fix a program without help,
so they are part of the interface and are tested (acceptance scenarios
[2, 4 and 5](acceptance-m0.md)). Values marked *proposed* are starting points.

It follows the Craft CLIs, `gridcraft-cli check` above all: exit codes 0, 1 and 2, `--json` that
prints one object, a named schema version, a report that counts everything even when it lists
only part of it, and an error when a scan finds nothing to check.

The Python proof of concept (`tools/lcf-poc`) does not implement this document.

## 1. Common rules

```text
labcraft-cli fmt   [--check] [--json] [options] PATH...
labcraft-cli check [--json] [--deny-warnings] [--lib-path DIR]... [options] FOLDER...
labcraft-cli run   [--json] [run options] [options] FOLDER FUNCTION
labcraft-cli diagram --svg --out DIR [--json] [diagram options] [options] FOLDER [FUNCTION]...
labcraft-cli version
labcraft-cli --help
```

| Exit code | Meaning |
|---|---|
| 0 | No error. Warnings do not change the code, unless `--deny-warnings` is given. For `run`, an error outside what the run reaches does not count (section 6.1) |
| 1 | An error was found: a diagnostic of severity `error`, or a file that is not canonical under `fmt --check` |
| 2 | Wrong usage: unknown option, no path, a file given to `check`. A plain message on stderr, nothing on stdout, nothing read or written |

`run` adds 3 (`aborted`) and 4 (`fault`), and uses 1 for the statuses `error` (spec 8.2) and `refused`
([commands.md](commands.md), section 7). `diagram` uses 1 when a function is not drawn, and not for the errors that
it marks on a drawing (section 7.1). A missing
path is exit 1 with a diagnostic, not exit 2: a path is data, and a bad one does not hide the
diagnostics of the others.

| Option | Meaning |
|---|---|
| `--json` | One JSON document on stdout (section 4), nothing else |
| `--max-report N` | Diagnostics listed, default 1,000 *(proposed)*. Beyond it, errors are kept before warnings and `truncated` is true; the counts stay complete |
| `--max-file-size BYTES` | Default 4 MiB *(proposed, scenario 4)*, for a file and for what `fmt` would write, at most 1 GiB *(proposed)*: reading takes many times the size of a file, and running out of memory ends the process. A larger value is wrong usage. At the default, `fmt` and `check` of any file stay within the bounds of scenario 4; a function of 20,000 nodes, the most that a diagram draws, takes about 1.5 MiB with its `layout` |
| `--max-nesting N` | Depth of nested structures, parentheses of units, types and array literals, with one counter (spec 14.1), default 256, at most 1,024 *(proposed, scenario 4)*: a stack is sized from it. A larger value is wrong usage. The WASM hosts stop at 256, so a file nested deeper reads only here ([architecture](architecture.md), section 4) |
| `--` | What follows is a path, even if it starts with `-` |

The exit code describes the files, so a stdout that cannot be written does not change it.

Reaching a limit on the input is a diagnostic with a code, never a crash; `--max-report` only
shortens the report. Output is in English (D-21), an error of the operating system included, in UTF-8,
without color unless stdout is a terminal. Every failure gives a diagnostic and processing goes on
with the next file; the tool does not stop at the first bad file. A file that gets `LC0001`, `LC0003`,
`LC0004`, `LC0006`, `LC0007`, `LC0101` or `LC0102` is not read further, and gets no other diagnostic.

**Paths in reports** use `/` on every platform, relative to the current folder when the file is
under it. Files of the libraries built into the tool are shown as `<labcraft>/visa/close.lcf`.

**Walking a folder.** Recursive, in code point order of the paths: `.lcf` and `.lct` files, skipping
folders whose name starts with `.`. A symbolic link is not followed and gives the warning `LC0009`.
At most 32 levels of folders and 100,000 files *(proposed, the limits of `gridcraft-cli check`)*, and
1 GiB in a library *(proposed)*; what is left out gives the error `LC0010`. A path where nothing is found gives the error `LC0002`,
unless reading it gave an error of its own:
a typo in CI must not look like a success. A file named explicitly on the command line must have
one of the two extensions, written in lower case; so must what it points to, if it is a link.

These bounds apply to each folder given, which is the first level and counts among its 100,000 folders. Folders
are walked in the order of their paths, depth first, so the same ones stay on every machine; among the files
found, the first ones in the order of the paths stay. A file over the size limit counts only for what would be
read of it. A file is known by its path without links, so one given twice, or also reached through a folder,
counts once; the order of the paths on the command line changes nothing. A symbolic link named on the command line is followed, and `fmt`
writes through it, so it stays a link. A device or a pipe with one of the two extensions is never read, and
gives `LC0001`. On Windows, a stream (`notes.txt:a.lcf`) gives `LC0008`.

## 2. `fmt`

Writes each file in canonical form (spec 5.2). It works **one file at a time and reads no other
file**, so its result depends only on the bytes of the file.

- Without `--check`: a file whose canonical form differs is rewritten atomically (a temporary file,
  then a rename). A file that is already canonical is not touched, so its modification time does
  not change and a watcher sees nothing.
- With `--check`: nothing is written. Each file that would change gets the error `LC6001`, located
  at the first line that differs. Exit 1 if there is one.

A file that cannot be read or parsed gets its diagnostics and stays unchanged, whatever the option.

**A canonical form over `--max-file-size`** gets the error `LC6003`, and the file stays unchanged, whatever the
option. Indentation grows with nesting, so 4 MiB of short lines nested 256 levels deep would take over 1 GiB, and a
text over the limit could not be read again. **A file that cannot be replaced**, such as a read-only one, gets
the same error.

**What `fmt` changes:** line endings and the end of the file, a byte order mark (section 2 of the
spec); names and units in their one spelling (R-20); `use` lines; the order of lines; the layout of
the text; the items of `panel` and `layout`; layout entries that name nothing (`LC5004`): a `node` entry
on no node, a `wire` entry that starts at no name that the function binds or label, or at a terminal
(`raw.session`), or ends at no node.
An entry that names an input or an output of the function, or a line such as `k = step.low`, which a single
file cannot tell from a constant, stays for `check` (spec 11).

**Layout entries outside a function.** An `extern` file has no node, so `fmt` removes all of its
entries, even those that name a parameter (`LC5004`). In a type file it removes none: the section
stays for `check` to refuse (`LC0104`).

**What it never changes:** the text of strings and comments (apart from the space after `//` and
the spaces at the end of a line), the spelling of number literals, the meaning of the program
(scenario 5). **It never deletes a comment:** a duplicate `use`, a stale layout entry or an empty
section that has a comment attached stays (spec 5.2).

**A body it cannot order** (a cycle, a value or a label defined twice) keeps its written order: its
layout is still normalized, the warning `LC6002` says so, and no layout entry of the function is
removed. `fmt` does not know whether a name resolves in the library: a name that the file does not
define creates no dependency between lines, and that is all `fmt` needs to know. Finding it is the
job of `check`.

**What `fmt` reports**, and nothing else, so that it does not depend on the library: `LC0001` to
`LC0010`, `LC0099`, `LC0101`, `LC0102`, `LC1001` to `LC1005`, `LC5004` when it removes an entry, and
`LC6001` to `LC6003`. A number too large for any type, or a `panel` in a type file, is not its
business: the file parses, and `check` reports it.

## 3. `check`

Checks the libraries given as folders, without running anything: names, signatures, types and units,
dataflow rules, panel, layout, icon. It writes nothing and does not care whether a file is canonical.

**A library is a folder.** Its files are the `.lcf` and `.lct` files directly in it; each sub-folder
that holds such files is a sub-library, and is checked as well. Until M2, a manifest is read only for
the libraries shipped with LabCraft (spec, section 9): any other `labcraft.toml` is ignored. In M0a
a file bears the name of what it declares, and the names of files and folders are names that every system can hold (spec, R-22:
`LC2023`, `LC2024`, `LC2025`). A folder given inside another one given
is one of its sub-libraries, even one that a walk of the outer folder would not reach, such as `.hidden`: each
file is checked once, under the outermost folder given.

**Resolving `use lib`**, first match wins:

1. the primitives of the language (`core`, `string`, `array`, `errors`, `time`, `ui`), which cannot
   be replaced. A primitive library added later ranks with rule 4 instead, after the folders of the
   project (spec, 14.1, new words);
2. a sub-folder `lib` of the `FOLDER` given to `check` (`use a.b`: the folder `a/b`);
3. `DIR/lib`, for each `--lib-path DIR`, in the order given;
4. the libraries shipped with LabCraft (`visa`, `scpi`), whose declarations are built into the tool,
   so that `check` works in any folder and in the WASM build.

A library shipped with LabCraft follows only rules 1 and 4: a folder `visa` in a project replaces
`visa` for that project, but not for `scpi`. A library found by rule 3 follows rules 1, 3 and 4, so it says the
same whichever folder reads it, and its errors are reported once. A `--lib-path` gives a name to what it
holds, even to a folder given: `check p --lib-path .` reads `p` again as a dependency when a `use p` asks for it,
and if `p` itself says `use p`, that is a cycle (`LC2021`). A dependency whose files a folder given has all
reached says nothing more, apart from a cycle. Names are compared exactly, even on a file system that
ignores case: `use VISA` does not find `visa`.

Names of files and folders are compared with those of the program after normalization (R-20), so
that `use é` finds a folder whose name the file system decomposes. A folder with no file of its own is no
library, though its sub-folders can be: `use a` does not find it, and `use a.b` still finds `a/b`. A symbolic
link is not followed. A dependency found by rules 3
and 4 is read for its signatures, types and manifest only: its bodies are checked when it is itself
the library being checked, and so are its sub-folders, which nothing reads until a `use` names them. A
dependency that cannot be read is reported, since it is the cause; so are its other errors, and not its
warnings. The limits of the command line do not apply to the libraries built into the tool. A `--lib-path` that
cannot be listed gets `LC0001` once and is skipped, and a file given there is wrong usage.

**One error, one diagnostic** (scenario 2). A value whose type is unknown because of an error
upstream takes an error type that matches everything; a call to a function whose file does not parse
gives no further diagnostic. A file with a syntax error gives at least one diagnostic and is not
checked further; the other files of the library are. A library with such a file may declare what a name
stands for, so a type that it does not seem to have gives no `LC2003`.

`--deny-warnings` makes warnings count for the status and the exit code. This is what scenario 1
asks of an agent, and what a project can ask of its CI.

## 4. Reports

### 4.1 Human-readable form

```text
bench/power.lcf:9:27: error[LC3002]: cannot add `f64[V]` and `f64[A]`: different dimensions
  in function `power`, node `p`: p = core.add(voltage, current)
  wire `voltage`: fn.voltage -> p.a
  wire `current`: current.result -> p.b
  expected `f64[V]`, found `f64[A]`
check: 1 library, 14 functions, 3 types: 1 error, 0 warnings
```

The first line of a diagnostic always starts with `file:line:column: severity[code]: message`
(`file: severity[code]: message` when the problem has no position). Details follow, indented. The
last line is the summary. Everything goes to stdout. So that a file name cannot forge a line or drive a
terminal, control characters, line separators and bidirectional controls and marks show as escapes (`\u{a}`).

`fmt` first names each file it rewrote, and its summary counts files:

```text
formatted bench/power.lcf
bench/old.lcf:4:1: error[LC1001]: expected a statement, found `)`
fmt: 12 files, 1 changed: 1 error, 0 warnings
```

When the list is cut, the summary ends with the number listed: `(1000 listed)`.

### 4.2 JSON form

`--json` gives the same content. The same diagnostics come from the MCP server (M0b).

```json
{
  "schema": "labcraft-check/1",
  "status": "error",
  "summary": {
    "libraries": 1, "functions": 14, "types": 3,
    "errors": 1, "warnings": 0, "truncated": false
  },
  "diagnostics": [
    {
      "code": "LC3002",
      "severity": "error",
      "message": "cannot add `f64[V]` and `f64[A]`: different dimensions",
      "file": "bench/power.lcf",
      "span": { "start": { "line": 9, "column": 27 }, "end": { "line": 9, "column": 34 } },
      "function": "power",
      "node": { "name": "p", "text": "p = core.add(voltage, current)" },
      "wires": [
        { "value": "voltage", "from": "fn.voltage", "to": "p.a" },
        { "value": "current", "from": "current.result", "to": "p.b" }
      ],
      "expected": "f64[V]",
      "found": "f64[A]"
    }
  ]
}
```

A fix and a related place have this form:

```json
{
  "fixes": [
    {
      "title": "Convert with `core.round_to_i32`",
      "edits": [
        {
          "span": { "start": { "line": 4, "column": 5 }, "end": { "line": 4, "column": 5 } },
          "text": "n2 = core.round_to_i32(x)\n    "
        }
      ]
    }
  ],
  "related": [
    {
      "message": "`voltage` is already defined here",
      "file": "bench/power.lcf",
      "span": { "start": { "line": 5, "column": 5 }, "end": { "line": 5, "column": 12 } }
    }
  ]
}
```

`fmt --json` has the same form, with the schema `labcraft-fmt/1`, a summary
(`files`, `changed`, `errors`, `warnings`, `truncated`) and a list
`files: [{ "file": "...", "result": "unchanged" | "formatted" | "not_canonical" | "error" }]`, in the order of
the paths. `changed` counts the files rewritten, so it is 0 under `--check`. A folder or a path that does not
exist is not a file: its diagnostics stand alone.

| Field | Always | Content |
|---|---|---|
| `schema` | yes | Name and version of this schema (section 4.4) |
| `status` | yes | `ok` or `error`; `error` exactly when the exit code is 1 |
| `summary`, `diagnostics` | yes | Counts, complete even when the list is cut; the list, `[]` when there is none |
| `code`, `severity`, `message` | yes | Stable code, severity set by the code (`error` or `warning`), English text |
| `file` | yes | Path (section 1) |
| `span` | no | Start and exclusive end. Lines and columns count from 1; **columns count Unicode code points** of the file as read, a byte order mark excluded, like the line width of spec 5.2. Absent when the problem concerns the file as a whole |
| `function` | no | The function at fault |
| `node` | no | The name of its line (spec 5.2), absent if empty, and its text in canonical form |
| `wires` | no | The wires or terminals at fault: `value`, the name of the wire; `from` and `to`, terminals (section 4.3). Each is present when it exists: a bad unit argument has only `to` (`n = core.unitless(voltage, [A])`: `to` is `n.unit`) |
| `expected`, `found` | no | Types as written in the language, units included. A scale that no unit name gives shows as a number (`f64[1000*m^2]`), which the language cannot read back; a type that an error upstream left unknown shows as `unknown` |
| `fixes` | no | A fix when one exists: a `title`, and `edits` to apply to the file when the fix is mechanical |
| `related` | no | Other places: where a name is already defined, the first of two conflicting arms |

A field that does not apply is absent, not `null`. So that a hostile file cannot make the report huge,
every text that can repeat a name of the file (`message`, `function`, `node`, `wires`, a type, the title
of a fix, the message of a related place) is cut at 200 code points and ends with `…`, and a diagnostic
lists at most 32 wires, fixes, edits or related places. An edit is never cut, since it would change the
program when applied: a fix whose edits are too long or too many keeps its title alone. Paths are not
cut: walking a folder bounds them.

A fix that calls a function of another library also adds its `use`, in its sorted place, when the file lacks it. Fixes are suggestions and nothing applies them in M0a. `core.with_unit`, for example, asserts something
the tool cannot know, and the choice stays with the author.

Diagnostics are sorted by `file` in code point order, then line, column (those with no `span` first),
code and message, so that the same input always gives the same report. The expected results of the
corpora ([tests/acceptance](../tests/acceptance/README.md)) compare everything except the text of
`message` and `fixes`. A problem across files
(`LC2013`, `LC2016`, `LC2017`, `LC2024`) is reported once, on the last file in path order for `LC2013` and `LC2024` and on the first
for `LC2016` and `LC2017`, with the others in `related`.

A value that does not convert gets the code of where it goes: `LC3001` or `LC3004` on an input, `LC3003` on an
output, `LC3006` in an arm, `LC3007` on a carry, `LC3001` as an element of an array, `LC3015` on a field or a value of a
construction, element of an array included, `LC3017` in a default value, whatever it builds. So a narrowing to an output is
`LC3003`, and its fix still names the conversion. A default has no fix that adds a node, since it reads nothing of the
diagram; its terminal is that of the parameter, `fn.gain`, or of the panel element, `panel.gain`. A generic primitive that does not match what it goes to has
no type to compare: `LC3008`, except in a construction or a default, which keep their code. A type argument or an array makes a variable fixed, and an input against
it is an input like another: `LC3001` where two wires that meet would give `LC3002`. An array of wires, or a constant
whose rows differ, gets the code of where it goes when an `[f64; 2]` is expected, and the fix names `array.from_rows` or says
to give the rows one length. The wire on a `string.format` field gets `LC3001` for a type or a format that it cannot show.

### 4.3 Terminals

A terminal is written like the `wire` entries of the `layout` section (spec 11), with the names of
the signatures:

| Form | Terminal |
|---|---|
| `dmm.session`, `#close.err` | Output or input of a node, designated by the name of its line |
| `@12.err` | Output or input of a node with no name, on line 12: a call statement without a label, or a line whose outputs are all `_` or `next x` |
| `fn.power` | Input or output of the function: `fn` is reserved, so no node has that name |
| `next.n`, `until`, `collect.0`, `yield.1` | Carry `n`; the condition of the loop; a `collect` and a value of a `yield`, by position from 0 |
| `p.x`, `r.0` | A field of a node that builds a struct or a `waveform`; a value of a node that builds a variant, by position from 0 |
| `limit.value` | The output of a named constant (spec 5.1) |
| `panel.gain` | A panel element, for its `default:`: a `panel` is a section, and no node has that name |

### 4.4 What is stable

From the M0a tag:

- **Codes.** A code keeps its meaning forever. A code that is no longer used stays reserved, and is
  never given another meaning. Its severity can change only with a release note. A code of the form
  `library:code` (`visa:S001`) belongs to that library, which writes its meaning with its format
  ([libs/visa/simulation.md](../libs/visa/simulation.md), section 7) and keeps it by the same rule; how
  a library gives its diagnostics to the tool is written with the architecture of M0b.
- **Schema.** `labcraft-check/1`, as in the other Craft apps. Adding a field does not change it, and
  a reader ignores fields it does not know. Removing or redefining a field raises the version.
- **Human-readable form.** Its first line, as described above. The rest can change.
- **Messages** can improve at any time: a tool decides on `code` and on fields, never on the text of
  `message`.

## 5. Diagnostic codes

`LC` and four digits; the first two digits give the family. This first list comes from the rules of
the spec and from the corpora of scenarios 2 and 4. New codes can be added at any time, and the
message of each one is written with its implementation. `S2` marks a case of the corpus of
scenario 2.

Reserved for later specifications: `LC7xxx` for `diagram` (section 7.6), `LC8xxx` for `run` (faults and
arguments, section 6.6).
`LC9xxx` stays free: MCP reports a failed call with a command error ([commands.md](commands.md),
section 3), not a diagnostic.

### Reading a file: `LC00`, `LC01`

| Code | Severity | Problem | Spec |
|---|---|---|---|
| `LC0001` | error | Path missing or unreadable, or not a regular file | scenario 4 |
| `LC0002` | error | Nothing to check: no `.lcf` or `.lct` file under this path | 1 |
| `LC0003` | error | Not valid UTF-8 | 2 |
| `LC0004` | error | Forbidden character: a control character (Unicode category Cc) other than tab and line feed, a CR not followed by a line feed | 2 |
| `LC0005` | warning | Bidirectional control character (U+202A to U+202E, U+2066 to U+2069) in a string or a comment, once for each of them: the text does not read as it runs. In a name it is a syntax error | 2 |
| `LC0006` | error | File over the size limit | scenario 4 |
| `LC0007` | error | Nesting over the limit: the 257th level by default, all kinds counted together | 14.1, scenario 4 |
| `LC0008` | error | File named explicitly without the `.lcf` or `.lct` extension | 2 |
| `LC0009` | warning | Symbolic link met in a folder: not followed | scenario 4 |
| `LC0010` | error | Files left out: too many files, folders, levels of folders or bytes in a library | scenario 4 |
| `LC0099` | error | Internal error of the tool, reported instead of a crash: a bug, or a system that would not let the work start | scenario 4 |
| `LC0101` | error | First line is not a `labcraft N` header: a blank line or a comment before it, other text on its line, a version that is not an integer | 2, 14.2 |
| `LC0102` | error | Format version not supported: any integer other than the exact text `1`, such as a newer one (`labcraft 99999`), `0` or `01` | 2, 14.2 |
| `LC0103` | error | Declaration in the wrong kind of file: a `fn` in a `.lct`, a `type` in a `.lcf` | 2 |
| `LC0104` | error | `panel`, `layout` or `icon` section in a type file | R-19 |
| `LC0105` | error | Manifest of a shipped library that cannot be read or names another library, or an `always` on something other than a native function | 1, 7.2 |

### Syntax: `LC10`

| Code | Severity | Problem | Spec |
|---|---|---|---|
| `LC1001` | error | Syntax error; the message lists what was expected | 14 |
| `LC1002` | error | Unexpected end of file (truncated file) | 14 |
| `LC1003` | error | Unterminated string or unknown escape | 14.1 |
| `LC1004` | error | Call where a value is expected, nested call above all; the fix names the intermediate value, except in a default value, which has no line before it | R-9, 5.1, R-21 |
| `LC1005` | error | Reserved word where a name is expected | 14.1 |
| `LC1006` | error | Number or type out of bounds: `1e999`, an integer beyond `i64` and `u64`, a unit exponent or scale out of bounds (±127, 1e300), an array of 0 or more than 32 dimensions, a type nested beyond `--max-nesting`, through its lines or its files *(bounds proposed)* | 3, scenario 4 |

### Names: `LC20`

| Code | Severity | Problem | Spec |
|---|---|---|---|
| `LC2001` | error | Unknown value, also a path `foo.bar` whose `foo` is known neither as a value nor as a library being checked or a primitive, and a type where a value is expected | 5.2 |
| `LC2002` | error | Unknown function; for a function of `core` written without the prefix, the fix qualifies it | 4 |
| `LC2003` | error | Unknown type | 3 |
| `LC2004` | error | Unknown library in `use` | 4 |
| `LC2005` | error | Library used without `use`, once per library in a file; the fix adds it. In a call any unknown prefix counts as a library, and elsewhere only a primitive or a library being checked | 4 |
| `LC2006` | error | Unknown field | 3.2 |
| `LC2007` | error | Unknown variant, also a pattern written without its type or with an unknown one | 3.2 |
| `LC2008` | error | Unknown unit, in a `string.format` template too, where a unit with an invisible character counts as unknown | 3.3 |
| `LC2009` | error | Unknown named argument, a name given to a variadic parameter, or to a value of a variant | 4, 5.4 |
| `LC2010` | error | Unknown panel element in `ui.get` or `ui.set` | 10, R-1 |
| `LC2011` | error | Unknown label in `after:` | R-15 |
| `LC2012` | error | Value defined twice in a body | 5.1 |
| `LC2013` | error | Function or type defined twice in a library, or a function and a type with the same name. A copy in a file of another name, when the name has its own file, has `LC2023` instead | 4, R-22 |
| `LC2014` | error | Value with the name of a library that its file imports, of a type of its library or of a live panel element; function or type with the name of a library that any file of its library imports, on the declaration, with the `use` in `related` | R-1 |
| `LC2015` | error | Label used twice in a function | R-17 |
| `LC2016` | warning | Function that calls itself, directly or across files: a run stops at the limit of call depth (8.2). Only a call counts, not a function reference | 8.2 |
| `LC2017` | error | Type that contains itself. A cycle of types that passes through a `use` gives only `LC2021`, since its libraries are in a cycle too | 3.2 |
| `LC2018` | error | `next` for a name that is not a carry of the loop | R-6 |
| `LC2019` | error | Two parameters, outputs, fields or variants with the same name | 4, 3.2 |
| `LC2020` | warning | Library imported twice: `fmt` removes the line, unless it has a comment | 5.2 |
| `LC2021` | error | Libraries that use each other in a cycle | 4 |
| `LC2022` | error | Type with a name that the language gives to a type: a base type, `waveform`, `queue`, `ref`, `event`, `notifier`, `map`, `set`, `opaque` | 3.2 |
| `LC2023` | error | File whose name is not that of what it declares, case included (`gain.lcf` holding `fn scale`). The fixes rename the function or the type as the file, when the name of the file is a name, or the file as the declaration, each only when that name is free in the library, case aside: neither leads to `LC2013`, `LC2014`, `LC2022`, `LC2024` or `LC2025`. With neither, the fix says to give them a name of their own, or to delete a copy. A copy of a declaration in a file of another name has this code alone, not `LC2013`, and a call binds to the one in its own file | R-22 |
| `LC2024` | error | Two functions, or two types, of one library, or two folders side by side, whose names differ only by their case or their spelling (`Scale` and `scale`). Once, on the last one in the order of the paths, with the others in `related` | R-22 |
| `LC2025` | error | Function, type or folder with a name that Windows keeps for a device: `con`, `prn`, `aux`, `nul`, `com1` to `com9`, `lpt1` to `lpt9`, whatever their case and their extension: Git for Windows refuses such a file. Such a declaration has no `LC2023` | R-22 |

### Calls: `LC21`

| Code | Severity | Problem | Spec |
|---|---|---|---|
| `LC2101` | error | Required input not wired: once per call, on the first one, and not when the arguments of the call have a mistake of their own, which may be the cause | 4 |
| `LC2102` | error | Too many arguments, or an input wired twice | 4 |
| `LC2103` | error | Number of targets different from the number of outputs. A loop that gives nothing takes one `_` | 5.1, 6.2 |
| `LC2104` | error | Positional argument after a named one, `after:` included | 4 |
| `LC2105` | error | Invalid `after:` (a literal, or two in a call), or a parameter named `after` | R-15 |
| `LC2106` | error | `string.format`: template not a literal, a field or a format not written as 5.4 says, a value without a name, or fields and arguments that differ (one diagnostic per node, with up to 5 names, and none when a field is malformed) | R-12 |
| `LC2107` | error | Variadic parameter misplaced, or more than one | R-11 |

### Types and units: `LC30`

| Code | Severity | Problem | Spec | |
|---|---|---|---|---|
| `LC3001` | error | Wrong type on an input, or what is not a unit, a category or an element on a parameter of that kind | 3, R-5 | S2 |
| `LC3002` | error | Incompatible dimensions in an operation | 3.3 | S2 |
| `LC3003` | error | Wrong type or dimension on a function output | 3.3 | S2 |
| `LC3004` | error | Implicit narrowing on an input, or two number types with no type that both widen to; the fix names the conversion when there is one | 3.4 | S2 |
| `LC3005` | error | Unit argument of another dimension, or the unit of a `string.format` field that does not convert each number of its value | 3.3 | S2 |
| `LC3006` | error | Arms that disagree in number or type of values | R-7 | S2 |
| `LC3007` | error | Carry whose type changes at `next`, or whose initial value does not fit the type on its line | 6.2 | S2 |
| `LC3008` | error | Function reference of the wrong signature, or a primitive that has no function type: one that takes a unit, a category or an element, `core.call`, `string.format`. When the input receives a function of its own library by its name, not an `extern` one, the fix says to give that function the signature expected and to bind each output | 4, R-2 | S2 |
| `LC3009` | error | Pattern that cannot apply: of the wrong type, such as a variant of another enumeration, that binds another number of values than its variant holds, or a range that holds no value | 6.1 | S2 |
| `LC3010` | error | `match` without a branch for every value: a variant, `true` or `false` missing, or no `_` on an integer or a string. It names at most five variants. Silent on a boolean or an enumeration when one of the patterns has a diagnostic of its own, a repeated pattern included, since what it was meant to cover is unknown | R-8 | |
| `LC3011` | error | Overlapping literal or range patterns, in one branch or in two: on the pattern that starts inside another, with that one in `related` | R-18 | |
| `LC3012` | error | `handle` without `_` | R-10 | |
| `LC3013` | error | Selector of the wrong type in a `match` or `handle`. With a selector of unknown type, its arms get no `LC3010`, `LC3011`, `LC4106`, nor `LC3022` on a variant; a repeated `_` and a `handle` without `_` are still reported | 6.1, 7.3 | |
| `LC3014` | error | `until` that is not a `bool` | 6.3 | |
| `LC3015` | error | Struct, `waveform` or variant construction: a field or a value missing, given twice, in excess or of the wrong type, a narrowing and a function of the wrong signature included; `{ }` on what is neither a struct nor a `waveform`; `samples` that is not an array. What is missing is one diagnostic per node, naming at most five fields, and none when a field is unknown or given twice, which may be the cause | 3.2, 5.1 | |
| `LC3016` | error | Unit not computable: `sqrt` of an odd exponent or of a scale that is no exact square, `pow` without a constant integer exponent, an exponent or a scale that goes out of bounds through a chain of `mul` | 3.3 | |
| `LC3017` | error | Default value of the wrong type, for a parameter or a panel element, or the default of a parameter that is not a constant: a value of the diagram, a call of a function, `after:`, a type argument. Every mistake in what a default builds has this code, in place of `LC3015` or `LC3019`. A parameter of unknown type says nothing of its default. A signature that is faulty as a whole (a name defined twice, `LC2019`, `LC2105`, `LC2107`), or a generic function, says nothing of any default | 4, 10, R-21 | |
| `LC3018` | error | Literal that does not keep its value in its type: `300` for an `i8`, `16777217` for an `f32`, a bound of a range even when it is excluded (`0..256` on a `u8`), a literal too large for its type by its value in the unit it goes to (`1e39` for an `f32` or a `c64`, `1e36 V` for an `f32[mV]`), in a default value too | 3.4, 6.1 | |
| `LC3019` | error | Type variable of a generic primitive that cannot be determined, a generic primitive as a value where no function type is expected, a `[]` whose type nothing says (on the line of a named constant, nothing does), the wrong number of type arguments, or a `waveform` that does not hold numbers (`waveform<string>`, `samples` of `[[f64]]`) | 3.2, 3.5, 4 | |
| `LC3020` | error | `for` clause over something other than an array, or a range with bounds that are not integers | 6.2 | |
| `LC3021` | error | Unit on a type that is not a number: `string[V]`, `duration[s]`, a declared type, even an alias of a number | 3.3 | |
| `LC3022` | error | The same pattern twice, in one branch or in two: `_`, a variant, or in a `handle` `ok`, a code or a category. On the second one, with the first in `related` | 6.1, 7.3 | |

### Dataflow and structures: `LC40`, `LC41`

| Code | Severity | Problem | Spec |
|---|---|---|---|
| `LC4001` | error | Cycle: a value can be fed back only through a carry. An order wire counts. Once per cycle, on its first written line, naming at most five values; what only waits for a cycle has nothing more to say | 5.2 |
| `LC4002` | error | Function output not bound (on its declaration), or bound twice (on the second written line, once per output) | R-2 |
| `LC4003` | error | Carry with no `next` (on its declaration), or with two (on the second one). Not on a loop where a `next` names no carry, which has its `LC2018`, nor on a `for parallel`, which has its `LC4004` | R-6 |
| `LC4004` | error | `for parallel` with a carry, once per loop | 6.2 |
| `LC4005` | error | External function without an `err` output, or that differs from its native body | R-13, 4.1 |
| `LC4006` | error | Feature planned for a later milestone: generic function or type written by the user, `ref<T>`, `queue<T>`, `event`, `notifier`, `map`, `set`… | 9 |
| `LC4101` | warning | `err` output of a call wired nowhere: `_`, a name that nothing reads, or a call alone. An order wire is a wire | 7.5 |
| `LC4102` | warning | Concurrent access to a resource: two function calls that receive it, with no dependency between them. Once per resource, on the second node, with the first in `related` | 8.1 |
| `LC4103` | warning | Panel element written by two unordered nodes, once per element | 8.1 |
| `LC4104` | warning | `for parallel` body that uses a resource from outside, or writes a panel element, once per loop | 8.1 |
| `LC4105` | warning | Two reads of the same `button`, not in different arms: only one sees a press. Once per element | 10 |
| `LC4106` | warning | `_` that can catch nothing, on a boolean or an enumeration whose values all have their pattern; the fix removes it from a list, and only says to remove its arm when it stands alone, since that arm can decide the type of the others (R-7). Silent when one of the patterns has a diagnostic of its own, a repeated pattern included | R-8 |

### Panel, layout, icon: `LC50`

| Code | Severity | Problem | Spec |
|---|---|---|---|
| `LC5001` | error | Panel element declared twice | 10 |
| `LC5002` | error | `control` with the name of an output only, or `indicator` with the name of an input only | 10 |
| `LC5003` | warning | Panel kind that M0 does not know (section 10); the element works without a widget | 10 |
| `LC5004` | warning | Layout entry that names nothing: a `node` entry on no node (an input, an output, a line such as `k = step.low`), a `wire` entry that starts at no value or label, ends at no node or output, names an input that the node does not have, or starts with a terminal (`raw.session`). Every entry of an `extern`. `fmt` removes those that a single file can tell (spec 11) | 11 |
| `LC5005` | error | Coordinate that is not an integer literal in the range (`5.0`, `1e3`, `2000000`) | 11 |
| `LC5006` | warning | Layout entry that is ambiguous (a `node` entry on a name defined in several bodies, a `wire` entry that matches several wires: naming the input settles it), or a `wire` entry whose value and node exist with no wire between them | 11 |
| `LC5007` | error | Two layout entries for the same node or wire, `node x at (…)` and `node x view standard` included | 11 |
| `LC5008` | error | Invalid icon: a grid not written `32`, `path` syntax, a radius or a size not above zero, `inf` or `nan`, an `accent` token that is not in the visual notation, section 8 | R-14 |
| `LC5009` | warning | Icon shape that leaves the grid: what it draws goes below 0 or above 32, a text by its box in the font of the notation (11.1) | R-14 |
| `LC5010` | error | Panel element that does not say its type (not an input or an output, a kind that holds several types, no `default:`) and that a `ui.get` reads, or that no `ui.set` writes, when the function has no other mistake, which may be a `ui.set` that nothing could bind. Not for a kind that M0 does not know, which has only `LC5003`, nor on an `extern`, which has no diagram | 10 |
| `LC5011` | error | Panel kind that does not show the type of the input or the output of its name: a wire that it refuses is `LC3001`, a `default:` `LC3017` | 10 |
| `LC5012` | error | `default:` on an input or an output, whether or not its signature has a default, or given twice | 10 |
| `LC5013` | error | Panel element that receives wires or constants of different types, `f64[V]` and `f64[mV]` included. On the element, naming at most five values. Not when its name, its kind or its `default:` gives the type, where each one goes to that type as on a terminal | 10 |

### `fmt`: `LC60`

| Code | Severity | Problem | Spec |
|---|---|---|---|
| `LC6001` | error | `fmt --check`: file not canonical | 5.2 |
| `LC6002` | warning | Body left in written order: a cycle, or a value or a label defined twice | 5.2 |
| `LC6003` | error | Canonical form could not be written: over `--max-file-size`, or the file system refused it, as for a read-only file. The file is unchanged | section 2 above |

## 6. `run`

*Status: draft for milestone M0b, nothing implemented.*

Runs one function of a library, against simulated instruments, and reports what it did. It is the
command line of `run.start` ([commands.md](commands.md), section 6.4): the same parameters, the same
report (`labcraft-run/1`, commands.md, section 7) and the same trace (`labcraft-trace/1`, section 8).

```text
labcraft-cli run [--json] [--deny-warnings] [--lib-path DIR]... [--sim FILE]
                 [--args FILE] [--arg NAME=VALUE]... [--set ELEMENT=VALUE@TIME]...
                 [--real-time] [--schedule-seed N] [--timeout DURATION]
                 [--max-iterations N] [--max-nodes N] [--max-call-depth N] [--max-memory BYTES]
                 [--max-real-time DURATION] [--trace FILE | --no-trace] [options] FOLDER FUNCTION
```

`FOLDER` is the root of the project, as for `check`, and `FUNCTION` is named from it: `check_voltage`,
`dmm_driver.measure`. `[options]` are those of section 1. Paths given to options are relative to the
current folder, as for `check`.

### 6.1 What it does

1. **Checks** `FOLDER`, as `check` would with the same options, and also the bodies of every function
   the run can reach, in every library: what the function calls, what those call, and the functions
   named as references (spec 4), including those of a dependency found by `--lib-path`, whose bodies
   `check` alone does not read (section 3). A run executes them, so they are judged first. An error in
   what the run can reach, its types included, or under `--deny-warnings` a warning in a file of `FOLDER`
   that the run can reach, refuses the run: status `refused`, exit 1, and nothing reaches an instrument
   (scenario 2, *with M0b*). An error
   elsewhere in `FOLDER` does not refuse the run: it is counted in the summary of the report, as
   everything `check` found, and the exit code follows the run, not that error. A warning in a shipped
   library or a dependency of `--lib-path` never refuses a run, as `check` reports none (section 3):
   the user cannot fix it there.
2. **Reads the arguments and the panel settings** against the signature and the panel of the
   function (6.3, 6.4), and the simulation file. This is done even when step 1 refuses, as long as the
   signature could be read, so that one report gives every problem. A problem is a diagnostic with a
   code (6.6), and the status is `refused`.
3. **Runs** the function, in deterministic mode unless `--real-time` is given (spec 8), and ends in one
   of the statuses of spec 8.2: `ok`, `error`, `aborted`, `fault`.
4. **Reports**: the human-readable form (6.7), or `labcraft-run/1` with `--json`. The report is written
   once, at the end; nothing else goes to stdout. The run id of a command-line run is `r1`.

Exit codes (section 1): 0 `ok`; 1 `error` or `refused`; 2 wrong usage; 3 `aborted`; 4 `fault`.

**The diagnostics of a run** (`LC81`, the simulation file) join those of `check` in `check.diagnostics`
and count in `check.summary`. Each has a file: the file of the function for an input, an argument or an
element, located at its name in the signature or the panel (at the signature for an unknown input); the
simulation file for its own problems; `FOLDER` itself, with no position, for an unknown `FUNCTION`.

### 6.2 Options

| Option | Meaning |
|---|---|
| `--sim FILE` | The simulation file of the bench: TOML, one table per library that simulates, by its name (in M0 only `visa`, whose format is [libs/visa/simulation.md](../libs/visa/simulation.md)). `run` reads the file, gives each library its table, and each library judges its part and reports its problems with its own codes (`visa:S001`…). A path is data: a file that does not exist or cannot be read gives `LC0001`, one over `--max-file-size` `LC0006`, one nested deeper than `--max-nesting` `LC0007`, one that is not TOML `LC8109`, a key at its top level that no library claims `LC8110`, each with `refused`, not exit 2. Without it, no resource exists, and `visa.open` returns an error |
| `--args FILE` | The values of inputs, as one JSON object, the `args` of MCP (6.3). A path is data, as for `--sim`; the file is bounded by `--max-file-size` |
| `--arg NAME=VALUE` | The value of an input of the function (6.3), which replaces the one of `--args` for that input. An input with a default may be left out |
| `--set ELEMENT=VALUE@TIME` | Sets a panel element of the function at a time of the run (6.4) |
| `--real-time` | Real time instead of deterministic mode. In real time, iterations and nodes are not limited unless a `--max-` option is given (spec 8.2) |
| `--schedule-seed N` | Deterministic mode only. The order among the nodes ready at the same time is drawn from `N` (an integer from 0 to 2^64−1) instead of the canonical order of spec 8 (scenario 3, point 1). The noise of simulated instruments does not depend on it: it belongs to each simulated quantity (spec 8) |
| `--timeout DURATION` | Aborts the run after `DURATION` on its clock: simulated in deterministic mode, real otherwise (spec 8.2, `abort.by` = `timeout`). A finite duration above 0, as for `--max-real-time` |
| `--max-iterations N` | Iterations of each `loop`, counted again each time the loop starts. Default 10,000 in deterministic mode, none in real time *(proposed, scenario 4)*. A `for` ends with its array and does not count here, only in `--max-nodes`. Reached: fault `LC8001` |
| `--max-nodes N` | Nodes run in all, a skipped node (spec 7.2) and the node of a call included. Default 1,000,000 in deterministic mode, none in real time *(proposed)*. Reached: fault `LC8002` |
| `--max-call-depth N` | Depth of calls, always limited. Default 1,024, at most 4,096 *(proposed)*: a stack is sized from it. Reached: fault `LC8003` |
| `--max-memory BYTES` | The logical size of the values the run holds at once, default 1 GiB, at least 1 MiB *(proposed)*. Reached: fault `LC8005`, at the node that would pass it |
| `--max-real-time DURATION` | Deterministic mode only: the real time the run may take, default 10 min *(proposed)*, since `--timeout` there counts simulated time, which a long computation does not advance. Reached: fault `LC8006`, at the node that was running. The only limit whose outcome depends on the speed of the machine, and the fault says so. With `--real-time`, `--timeout` counts real time already |
| `--trace FILE` | Writes the trace to `FILE` instead of `.labcraft/runs/` (commands.md, section 8). A `FILE` that ends in `.lcf`, `.lct`, `.toml` or `.json`, in any case (`main.LCF`), or that exists and does not start with the first line of a trace, is wrong usage, so that a typo cannot replace a program or a file of arguments |
| `--no-trace` | Writes no trace |

The `N` of the `--max-` options is an integer from 1 to 2^63−1, and `none` removes the default of
`--max-iterations` and `--max-nodes`; `--schedule-seed` has its own range, above.

**The logical size of a value** is counted the same on every platform, whatever the memory of the
process: 16 bytes for each value, and inside it for each element of an array, each field of a struct and
each value of a variant, so that an empty string or an empty array still costs something; plus the bytes
of each number by its type (1 for a `bool`, 4 for an `f32`, 8 for an `f64`, an `i64` or a `duration`,
twice that for a complex number, 16 for a `timestamp`), and the bytes of a `string` in UTF-8 and of
`bytes`. An `error` counts its fields as a struct does. So `array.repeat("", 2147483647)` counts 32 GiB,
and the limit stops it. A value counts once while some wire holds it, however many share it (D-11). Each
node that runs or waits, and each frame of a call, counts 256 bytes *(proposed)*, so that a `for parallel` over
millions of elements reaches the limit as a fault rather than the memory of the process.

**The clock of a run** counts whole nanoseconds from 0, and ends at 2^62 ns, about 146 years *(proposed)*: a wait or
a timeout that would take it further ends the run with the fault `LC8007`, in deterministic mode and in real
time, rather than wrapping around.

**At one time on the clock**, a run does in this order: the `--set` settings of that time, then the
timeout if it is reached, which aborts before the nodes ready at that time run, then those nodes, which
can each end the run with a fault. So a timeout and a setting at the same time are exact (scenario 3,
point 5).

**Durations** on the command line are written as the language writes them, and the space before the unit
may be left out, since a shell splits on it: `--timeout 30s`, `--timeout "30 s"`, `@500ms`. A number with
no unit (`--timeout 30`) is wrong usage, and the message gives the form to write: seconds are not assumed.
A duration that is not finite, or below 0 (`@-1s`, `inf s`), is wrong usage.

**What is wrong usage** (exit 2, nothing run, nothing written) is what can be seen without reading the
program: an unknown option, a missing `FOLDER` or `FUNCTION` or one more positional argument, a
`FUNCTION` that is not names joined by `.`, an input or an element that is not a name (spec 14.1: a
reserved word such as `next` is none), an `--arg` with no `=`, the
same input given twice by `--arg`, an option that takes one value given twice, `--max-real-time` with `--real-time`, a `--set` with no `@TIME` or a time as
above, a `--schedule-seed` with `--real-time`, a number out of the range of its option, `--trace` with
`--no-trace`. What needs the program to be judged (an unknown function or input, a value that does not
convert) is a diagnostic, and the status `refused`: a value is data, as a path is (section 1).

**Ctrl-C before the run starts**, during the check, ends with the status `aborted`, at 0 ms, nothing run.

`run` is not part of the WASM build of M0 (D-22; [acceptance-m0.md](acceptance-m0.md), common conditions),
so its limits are those of the native process.

### 6.3 Arguments

A `VALUE` takes the form that a value has in JSON (commands.md, section 5), read against the type of its
input, with two more freedoms: what JSON writes as a string may be written without its quotes, and the
space before a unit may be left out.

**`--args FILE`** holds a JSON object whose keys are input names and whose values take the JSON form
exactly, as the `args` of `run.start` do: `{"resource": "SIM::DMM0", "limits": {"low": "220 V", "high":
"240 V"}}`. No shell quoting is involved, so this is the form for structs and arrays in a script or on
Windows. An `--arg` replaces the value of the file for its input. A diagnostic about a value of the file is
located in the file, at its key; a file that is not a JSON object, or gives a key twice, gets `LC8104`.

| Input | Example |
|---|---|
| `string` | `--arg resource=SIM::DMM0`: the text after `=`, as it is, always; never read as JSON |
| a number with a unit | `--arg low="220 V"`, `--arg low=220V`, `--arg low=0.22kV` |
| a `duration` | `--arg settle=100ms`, `--arg settle="0.1 s"`; a number alone is refused, as for a unit |
| a number without a unit, `bool` | `--arg n=10`, `--arg gain=0.5`, `--arg verbose=true` |
| an enumeration | `--arg mode=Fast` or `--arg mode=Mode.Fast`; a variant with values in JSON: `--arg r='{"Reading.Value": ["230 V"]}'` |
| an array, a struct | JSON: `--arg limits='{"low": "220 V", "high": "240 V"}'`, `--arg points='["1 V", "2 V"]'`, `--arg xs='[1, 2]'` |
| `error` | `none` (or `null`) only: a run cannot be started with an error on its input |

A value is converted **as a literal of the language is** (spec 3.4, literals): it takes the number type of
its input, so `0.5` reaches an `f32` and `10` a `u8`, and `[1, 2]` an `[f64]`; a unit converts as a wire
does, so `0.22kV` reaches an `f64[V]` as 220 V. A number with no unit is refused for a type with a unit or
a `duration`, since the unit is what the run cannot guess. An integer type takes an integer written
without `.` or exponent (`10`, not `10.0` or `1e1`), and an `i64` or a `u64` beyond ±(2^53−1) as a string
of digits, as section 5 of commands.md writes it; `10.0` for an `i32` gets `LC8104`. Only a whole value may
leave out its quotes: inside JSON (`'[a, b]'`), a string is quoted. A JSON value nests at most `--max-nesting` levels, like a
file.

An input whose type holds an opaque type (`visa.session`, a struct with a session in it) or a function
reference cannot be given: a function that takes one is called by another function, which the run
starts. It gets `LC8107` whether it has a default or not.

**Quoting.** The examples are written for a POSIX shell and PowerShell 7. Windows PowerShell 5.1 drops the
double quotes inside an argument, so `'{"low": "220 V"}'` arrives as several broken arguments. `--args FILE`
works in every shell; writing the inner quotes as `\"` works in PowerShell 5.1 only, and breaks in
PowerShell 7, so a script that must run in both uses `--args`.

### 6.4 Panel elements: `--set` (Q-20)

`--set ELEMENT=VALUE@TIME` changes a panel element of the function at `TIME` on the clock of the run,
counted from its start: simulated in deterministic mode, so that a run replays exactly (scenario 3,
point 4), real otherwise. The time is after the last `@`, so a value may hold one (`--set
note=a@b@0s`); `@0s` sets it before the first node runs.

- **Which elements.** Any element of the panel of the function that is run (spec 10) and that is not an
  input or an output of it: panel elements belong to their call (spec 8.2), and a subfunction's are out of
  reach (Q-24). An element with the name of an input is set with `--arg`, since it is read once, at the
  start; one with the name of an output is written by the function. An element whose type `check` could
  not settle cannot be set. Each of these gets `LC8108`.
- **What a setting does**, as a user's action on the panel would: a `switch` keeps the value until
  something changes it again; a `button` set to `true` is a press, which stays true until its first
  read (spec 10). A `button` set to `false` gets `LC8108` in M0: what it does depends on the mode of the
  button, and the modes of a button are still open (spec, Q-30). Any other element holds the value
  until the diagram or another setting changes it. The value converts as an argument does (6.3), to the
  type of the element.
- **When it is seen.** Before the nodes ready at that time run, so the reads made at `TIME` see it
  (spec 8, and 6.2 above). Two settings at the same time apply in the order given. A setting after the
  end of the run never applies, and the report says so.
- **In the report**, `panel` lists each setting with its time, its value, how it came (`set` or
  `run.set_element`) and whether it applied (commands.md, section 7).

MCP takes the same settings in `run.start`, as `set: [{ "element": "stop_switch", "value": true,
"at": "5 s" }]`, and in real time also sets an element while the run goes, with `run.set_element`
(commands.md, section 6.4). The command line has no way to set an element live in M0b: a human at a
terminal stops a run with Ctrl-C.

### 6.5 Stopping

- **Ctrl-C** aborts the run (spec 8.2, `abort.by` = `interrupt`): no node starts any more, waits end,
  native calls end or reach their timeout within the grace period (5 s, *proposed*), then the
  resources are released and the report is written, with exit 3. A second Ctrl-C ends the grace period
  at once: the calls still going are abandoned and their resources reported as leaked. The report is
  still written.
- **`--timeout`** aborts the same way, `abort.by` = `timeout`.
- **A limit** (`--max-…`) ends the run with a fault (exit 4), at the node and the iteration where it was
  reached.

### 6.6 Codes of `run`

These codes join the tables of section 5 when they are implemented, as `cargo xtask codes` requires. The
family `LC80` is for faults, `LC81` for arguments, panel settings and the simulation file as a whole. What a
library finds in its own part of the simulation file has a code of that library, not of the tool: a library
ships its own format, and the tool does not read it.

| Code | Severity | When |
|---|---|---|
| `LC8001` | error | Fault: the iteration limit of a `loop` is reached. Gives the loop and the iteration |
| `LC8002` | error | Fault: the limit of nodes run is reached |
| `LC8003` | error | Fault: the call depth limit is reached. Gives the chain of calls, shortened |
| `LC8004` | error | Fault: a bug of the runtime, caught. Gives where |
| `LC8005` | error | Fault: the memory limit of the run is reached, at the node that would pass it |
| `LC8006` | error | Fault: the real-time limit of a deterministic run is reached; the message says that this limit depends on the machine |
| `LC8007` | error | Fault: the clock of the run would pass 2^62 ns, about 146 years: a wait or a timeout that long, or one that is not finite |
| `LC8101` | error | `FUNCTION` is not a function of the library; fix: the nearest names |
| `LC8102` | error | An `--arg`, a key of `--args` or of `args` names no input of the function; fix: the nearest names |
| `LC8103` | error | An input with no default has no `--arg` |
| `LC8104` | error | A value does not read as its type, or as JSON where JSON is needed, or nests too deep; an `--args` file that is not a JSON object, or that gives a key twice |
| `LC8105` | error | A value has no unit, or a unit of another dimension, for a type with a unit or a `duration` |
| `LC8106` | error | A value is out of the range of its type (`300` for a `u8`, `1e999`), or an integer is not exact |
| `LC8107` | error | An input whose type holds an opaque type or a function reference, which an argument cannot give; it replaces `LC8103` for such an input |
| `LC8108` | error | A `--set` names no panel element of the function, an element that is an input or an output, or one whose type is not settled, or sets a `button` to `false` |
| `LC8109` | error | The simulation file does not read as TOML (syntax, encoding, a key given twice, an integer that does not fit in 64 bits) |
| `LC8110` | error | A key at the top level of the simulation file names no library that simulates; fix: the nearest name |

Which other failures are faults rather than `error` values is Q-25; their codes come with its answer.

### 6.7 Human-readable form

Without `--json`, the outputs one per line, then what a reader checks first: the resources, the error, the
trace. Values are written as the language writes constants, cut as commands.md, section 5 cuts them, and
with the escapes of section 4.1, so that the answer of an instrument cannot drive a terminal.

```text
check_voltage: ok, 0 ms simulated, 2 ms real
  verdict = Verdict.Pass
  voltage = 230.071217 V
  err     = none
resources: 1 opened, 0 open
trace: .labcraft/runs/check_voltage.trace.jsonl, 41 events
```

A `refused` run gives the diagnostics in the form of section 4.1, then `check_voltage: refused`. An
`error` gives the error with its category, its message and where it came from (spec 7.1); a `fault`,
the first line of a diagnostic (`file:line:column: error[LC8001]: fault: …`); an `aborted` run, how and
when it was aborted, where each loop was, and the warning that the instruments are in an unknown state.
While a run goes on, a terminal on stderr shows the clock and where each loop is, once a second; never
with `--json` or when stderr is not a terminal.

## 7. `diagram`

*Status: draft for milestone M0v, nothing implemented.*

Draws the functions of a library as SVG, following the [visual notation](visual-notation.md): the
drawing of the viewer's canvas, which paints only the shapes that the `diagram` crate computes, for tests
and for agents (scenario 6 of [acceptance-m0.md](acceptance-m0.md)). Through MCP, the command `diagram.svg` of the
catalog returns the same SVG (D-18, [commands.md](commands.md), *with M0b*), with the options below as its
parameters.

```text
labcraft-cli diagram --svg --out DIR [--json] [--lib-path DIR]... [--theme light|dark]
                     [--trace FILE] [--max-diagram-nodes N] [--max-trace-size BYTES]
                     [options] FOLDER [FUNCTION]...
```

`FOLDER` is the root of the project, as for `check` and `run`. Without `FUNCTION`, every function of
`FOLDER` and of its sub-libraries is drawn, not those of a dependency or of a shipped library. A
`FUNCTION` is named from the root, as for `run` (`check_voltage`, `dmm_driver.measure`), and a function
given twice is drawn once. `[options]` are those of section 1.

```sh
labcraft-cli diagram --svg --out diagrams .
labcraft-cli diagram --svg --out diagrams --trace .labcraft/runs/check_voltage.trace.jsonl . check_voltage
```

### 7.1 What it does

1. **Checks** `FOLDER`, as `check` would with the same options: the drawing needs what `check` finds,
   the types of the wires, the badges and the marks of the notation.
2. **Draws** each function, as the viewer of M0v draws it:
   - each node in the view that its file gives it: compact, unless the `layout` section writes `view
     standard` for it (notation, section 3; spec 11). There is no option for all the nodes at once:
     the view belongs to each node;
   - as every exported image: `match` and `handle` unfolded (notation 5.2), a construction as its
     block (notation 3.1), and the rest of the view state at its default (notation 2.1);
   - at 100%, at which everything is shown (notation 7).

   A function is **drawable** when its file parses, its names resolve and it has no cycle (scenario 6,
   point 3); a type error or another error of `check` is drawn with the marks of the notation. A
   function that is not drawable, or that is too large to draw (`LC7002`), gets an SVG that shows its
   diagnostics in place of the drawing (7.2), so that the SVG of a function never shows a drawing that
   its file no longer gives. A file that does not parse is known by its path. An unknown `FUNCTION`
   (`LC7001`) gets no SVG, and the SVG of a function that no longer exists stays: `diagram` removes
   nothing.
3. **Puts the values of a run** on the wires with `--trace` (7.3).
4. **Writes** an SVG for each function at the path of its file under `DIR`, with `.svg` for `.lcf`
   (`DIR/check_voltage.svg`, `DIR/dmm_driver/measure.svg`), so that two functions never share one, then
   the report (7.5). `DIR` and the folders under it are created when they do not exist. On a file
   system that ignores case, a path that differs from one before it, in code point order, only in case
   gets `LC7004`. Each SVG is
   written to a temporary file of `DIR`, then renamed over the old one, so that a reader never sees
   half an image. Nothing else in `DIR` is written or removed. `DIR` may be inside `FOLDER`: no walk
   reads an `.svg`, but its folders count toward the bounds of a walk (section 1).

Exit codes (section 1): 0 when every function is drawn; 1 when one is not (not drawable, `LC7002`), when
a `FUNCTION` is unknown (`LC7001`), when there is nothing to draw (`LC0002`, `LC7003`), or when an SVG
could not be written (`LC7004`); 2 wrong usage. An error that `check` finds in a function that is still
drawn does not change the exit code, and neither does a problem of the trace (7.4): they are marked on
the drawing and listed in the report.

**The diagnostics of `diagram`** (`LC70`, `LC71`) join those of `check` in `check.diagnostics` and count in
`check.summary`, as those of a run do (6.1). One about a function is located in its file, with no
position; one about the trace in the trace file, at its line; one about an unknown `FUNCTION` at
`FOLDER`, with no position.

**What is wrong usage** (exit 2, nothing drawn, nothing written): no `--svg` or no `--out`, an unknown
option, a missing `FOLDER`, a `FUNCTION` that is not names joined by `.`, `--trace` without exactly one
`FUNCTION`, a `--theme` other than `light` or `dark`, an option that takes one value given twice, a
number out of the range of its option. A `DIR` that exists and is not a folder, or a `--trace` file that
does not exist or cannot be read, is data: `LC7004` for each function, `LC7101` for the trace.

### 7.2 The SVG

- **The drawing of the notation**, with its font named and not embedded (notation 2.3), and its colors as
  CSS variables named after the tokens, light and dark under `prefers-color-scheme` (notation 8).
  `--theme light` or `--theme dark` writes one theme only, without the media query. The `viewBox` holds
  the frame of the function and 16 units around it *(proposed)*, and one unit is one pixel at 100%.
- **What each shape is**, for an agent and for the tests, in attributes that name the program as the
  reports do (sections 4.2 and 4.3), and only in their forms: an attribute is absent where the reports
  have no form for it.
  - the root carries `data-schema="labcraft-diagram/1"` and `data-function` (`check_voltage`), and
    `data-drawn="false"` when the function is not drawn;
  - each node, a structure included, is a `<g data-node="raw">`, named as the terminals of section 4.3
    name a node (the name of its line, `#close`, `@12`), and its `<title>` holds the full name of what it
    calls (`visa.open`), as its tooltip does (notation 3.2);
  - each wire is a `<g>` with the fields of a wire of a diagnostic (4.2): `data-wire`, the name of its
    value; `data-from`, the terminal it leaves; `data-to`, the terminals it reaches, separated by
    spaces, those that have a form. In `check_voltage`, `<g data-wire="e2" data-from="raw.err"
    data-to="#close.err">`: `e2` also enters the `handle` as its selector, which has no form;
  - each branch of a `match` or `handle` is a `<g data-arm="State.Init">`, written as its tab (`other`
    for `_`);
  - each mark of a diagnostic carries `data-code` (`LC3002`), so that the place of a diagnostic in the
    image can be found;
  - each value of a run (7.3) is a `<g data-value="raw">`, named after its wire, and its `<title>` holds
    the value, as its tooltip does (notation 6.1).

  A name or a text in an attribute or a `<title>` is cut at 200 code points and ends with `…`, as in a
  report (4.2), so that a long name repeated on every node cannot make the SVG huge; two names that
  differ only after the cut then read the same, as in a report. A character that
  XML 1.0 cannot hold, even escaped (a control character other than tab and line feed, U+FFFE, U+FFFF),
  is written as its code, `⟨U+0000⟩`, as the notation shows a bidirectional control (notation 2.3);
  `<`, `>`, `&` and quotes are escaped as XML escapes them.
- **A function that is not drawn** shows its frame with its name, and its diagnostics, one per line, as
  the first line of section 4.1 writes them (`check_voltage.lcf:4:9: error[LC2001]: …`), each cut at
  200 code points, at most 32 *(proposed)*, then how many are left out.
- **The same bytes every time.** The same files, the same options and the same trace give the same SVG,
  on every platform and in the WASM build (scenario 6, point 2): no date, no time (the banner of a
  trace that no longer matches names the files, without the time of the trace), a path relative to
  `FOLDER` whatever the current folder, numbers in the shortest form with at most two decimals and no exponent,
  elements in the order of drawing and attributes in a fixed order, UTF-8 without a byte order mark, LF
  line endings.
- **What is stable** (section 4.4): the `data-` attributes and the names of the CSS variables, under
  `labcraft-diagram/1`; a reader ignores those it does not know. The geometry is not: a change of the
  drawing is a reviewed change of the snapshots of CI.

### 7.3 Values of a run: `--trace`

`--trace FILE` reads the trace of a run (`labcraft-trace/1`, [commands.md](commands.md), section 8) and
draws its values on the wires of the function, as the viewer does (notation 6.1): the bubbles, the
banners of a run that did not end well and of a trace that no longer matches, the branches that the run
did not take muted. An image does not move, so each loop shows its last iteration, the default of its
iteration selector, and there is no selector and no curve; the `<title>` of a bubble gives the whole
value.

The trace is named on the command line, never taken from `.labcraft/runs/` on its own, so that a
snapshot does not depend on the last run. A trace of another function gives `LC7102`, and the function
is drawn without values. Whether the trace matches the files is judged as in the viewer, on the files
that `diagram` resolved to draw the function.

### 7.4 Reading a trace: limits (scenario 4)

The trace is an input like a file (notation 6.1), and the viewer reads it with the same limits and the
same code. Reading it never crashes and never stops the drawing: what was read before a problem stands,
as in a cut trace, and the banner says that the trace was cut. *Proposed figures*:

| Limit | Bound | Reached |
|---|---|---|
| Size of the file | `--max-trace-size`, default 512 MiB, twice the 256 MiB after which a run stops tracing (commands.md, section 8), at most 4 GiB; a larger value is wrong usage | The values read up to there; `LC7103` |
| A line | 16 MiB, every line, of every frame: the bound of a whole line that a run writes (commands.md, section 8, *with M0b*) | The values end before that line; `LC7103` |
| Lines | 1,000,002 after the first: the 1,000,000 events of a run, then `truncated` and `end` | The values read up to there; `LC7103` |
| Nesting of a line | 256 levels; a value of a trace nests at most 64 (commands.md, section 5) | A line that does not read |

- **A line that does not read** (not JSON, not an object, a field of the schema missing or of the wrong
  kind, a negative number where an iteration, a frame or a sequence number is expected) ends the values
  there, with `LC7103`, which gives its line. A first line that does not read, or that names another
  schema or a later version of it, gives `LC7101` and no values.
- **Only frame 0 gives values**, the call of the function drawn: the events of the other frames are
  not decoded beyond what tells their frame, within the limits above.
- **An event that fits nothing in the drawing** is ignored: an event of frame 0 that names a node or a
  wire that the function drawn does not have, a value of another type than its wire or longer than the 64 KiB
  that a run writes (commands.md, section 5), an iteration whose
  levels are not the loops around the node, or a value already given for that node and iteration, an
  iteration number from 2^63 on; an event that names a frame that no `call` opened. Iterations may come in any order (`for parallel`,
  `--schedule-seed`). A trace that matches its files never holds such an event. `LC7104` counts them and
  gives the line of the first, and every other value is shown. A loop counts its iterations up to the
  highest that the trace gives.
- **Bounded resources.** Reading a trace at these limits takes at most 5 s and 512 MB on a CI runner
  *(proposed)*, measured by `cargo xtask bounds`, whatever the numbers it holds: memory follows the
  values of frame 0 that it keeps, not the size of the file, and a hostile iteration number allocates
  nothing. What it keeps, the index of the values included, is bounded too: past 256 MiB *(proposed)*,
  reading stops there, with `LC7103` ([architecture.md](architecture.md), section 11.2). The viewer draws the function at once,
  without values, reads the trace beside the drawing, and shows the values once it is read; the canvas
  stays responsive meanwhile (scenario 6, point 5). A trace replaced while it is read (a new run) is read
  again from its start.
- `diagram` opens the trace so that a run can replace it while it is read (commands.md, section 8).

### 7.5 Reports

The human-readable form gives the diagnostics as section 4.1 does, then one line per function, in code
point order of their names, then the summary:

```text
broken.lcf:4:9: error[LC2001]: unknown value `x`
  in function `broken`, node `y`: y = core.add(x, 1)
drawn check_voltage: diagrams/check_voltage.svg, 12 nodes
not drawn broken: diagrams/broken.svg, its diagnostics
diagram: 2 functions, 1 drawn, 1 not drawn: 1 error, 0 warnings
```

With `--trace`, a line before the summary says what was read: `trace: …/check_voltage.trace.jsonl, 41
events, ok`: the status of its `end` event or `no end`, then `cut` when the run or a limit of
  reading cut it, and the files that have changed since the run.

`--json` prints `labcraft-diagram/1`:

```json
{
  "schema": "labcraft-diagram/1",
  "status": "error",
  "summary": { "functions": 2, "drawn": 1, "not_drawn": 1 },
  "diagrams": [
    { "function": "broken", "file": "broken.lcf", "svg": "diagrams/broken.svg", "drawn": false, "nodes": 3, "codes": ["LC2001"] },
    { "function": "check_voltage", "file": "check_voltage.lcf", "svg": "diagrams/check_voltage.svg", "drawn": true, "nodes": 12 }
  ],
  "check": {
    "summary": { "libraries": 1, "functions": 2, "types": 1, "errors": 1, "warnings": 0, "truncated": false },
    "diagnostics": [ … ]
  }
}
```

| Field | Always | Content |
|---|---|---|
| `schema` | yes | Name and version of this schema (section 4.4) |
| `status` | yes | `ok` or `error`; `error` exactly when the exit code is 1 |
| `summary` | yes | `functions` asked or found, how many are `drawn` and `not_drawn` |
| `diagrams` | yes | One entry per function, in code point order of their names: `function`, `file` (absent for an unknown `FUNCTION`), `svg` (absent when it could not be written), `drawn`; `nodes`, the nodes counted against `--max-diagram-nodes`, when the file parses; `codes`, the codes that keep it from being drawn |
| `trace` | no | With `--trace`: `file`; `events`, the events read; `status`, that of its `end` event, absent when it has none; `cut`, true when the run or a limit of reading cut it; `stale`, the files that have changed or are gone since the run, `[]` when it matches; `ignored`, the events counted by `LC7104` |
| `check` | yes | As in the report of a run (commands.md, section 7): the `summary` and the `diagnostics` of `check`, those of `diagram` included |

### 7.6 Codes of `diagram`

These codes join the tables of section 5 when they are implemented, as `cargo xtask codes` requires. The
family `LC70` is for drawing, `LC71` for reading a trace.

| Code | Severity | When |
|---|---|---|
| `LC7001` | error | `FUNCTION` is not a function of `FOLDER`; fix: the nearest names |
| `LC7002` | error | The function is too large to draw: more nodes than `--max-diagram-nodes`, every level counted, structures included, or more than five times as many terminals. Default 20,000, at most 100,000 *(proposed)*; above the default, the bounds of scenario 4 are not promised. Gives the counts |
| `LC7003` | error | `FOLDER` holds no function to draw, only types |
| `LC7004` | error | An SVG cannot be written, with the message of the system; the other functions are still written |
| `LC7101` | warning | The trace file does not exist or cannot be read, or its first line does not read, or names another schema or a later version: no values |
| `LC7102` | warning | The trace is that of another function: no values |
| `LC7103` | warning | Reading the trace stopped at a line that does not read or at a limit (7.4): the values up to there, as in a cut trace. Gives the line and the limit |
| `LC7104` | warning | Events of the trace fit nothing in the drawing and are ignored (7.4). Gives how many, and the line of the first |

## 8. Left for later

No scenario of M0a needs these, and each can be added without breaking anything: applying fixes
(`check --fix`), reading from stdin, a `--diff` for `fmt`, per-project configuration of severities,
giving `check` a file or a selection of functions, `--json-out FILE` (as in `solidcraft-cli`, since a
shell's `>` can re-encode the output), a `--target web` for the error on external functions
(spec 4.1), and an `--exclude` for folders such as `target/` and `node_modules`, which are walked like any
other: a folder over the bounds of section 1 gives `LC0010` even if it holds no program.

For `run`, besides what [commands.md](commands.md), section 9 lists: setting a panel element live from
the command line (the first Ctrl-C as a press of a Stop switch, the second as an abort), arguments from a
JSON file read from stdin (`--args -`).

For `diagram`: other formats (`--png`), an SVG on stdout, the view state from the command line (every
node in standard view, as the preference of the IDE, folded constructions, the branch of a `match` in
tabs), an iteration chosen with `--trace`, an SVG that embeds a subset of the font with its notice.
