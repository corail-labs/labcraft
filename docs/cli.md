# Command line: `fmt` and `check`

*Last updated: 2026-11-04. Status: first draft, for milestone M0a. `run`, `diagram` and `mcp` come
with the specifications of M0b and M0v ([roadmap](roadmap.md)).*

This document is the contract of `labcraft-cli fmt` and `labcraft-cli check`: their arguments, exit
codes, reports and diagnostic codes. An agent reads the reports below to fix a program without help,
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
labcraft-cli version
labcraft-cli --help
```

| Exit code | Meaning |
|---|---|
| 0 | No error. Warnings do not change the code, unless `--deny-warnings` is given |
| 1 | An error was found: a diagnostic of severity `error`, or a file that is not canonical under `fmt --check` |
| 2 | Wrong usage: unknown option, no path, a file given to `check`. A plain message on stderr, nothing on stdout, nothing read or written |

`run` adds 3 (`aborted`) and 4 (`fault`), and uses 1 for the status `error` (spec 8.2). A missing
path is exit 1 with a diagnostic, not exit 2: a path is data, and a bad one does not hide the
diagnostics of the others.

| Option | Meaning |
|---|---|
| `--json` | One JSON document on stdout (section 4), nothing else |
| `--max-report N` | Diagnostics listed, default 1,000 *(proposed)*. Beyond it, errors are kept before warnings and `truncated` is true; the counts stay complete |
| `--max-file-size BYTES` | Default 16 MiB *(proposed, scenario 4)*, for a file and for what `fmt` would write, at most 1 GiB *(proposed)*: reading takes many times the size of a file, and running out of memory ends the process. A larger value is wrong usage |
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
option. Indentation grows with nesting, so 16 MiB of short lines nested 256 levels deep would take over 4 GiB, and a
text over the limit could not be read again. **A file that cannot be replaced**, such as a read-only one, gets
the same error.

**What `fmt` changes:** line endings and the end of the file, a byte order mark (section 2 of the
spec); names and units in their one spelling (R-20); `use` lines; the order of lines; the layout of
the text; the items of `panel` and `layout`; layout entries that name no node (`LC5004`), apart from
those that name an input or an output of the function, which stay for `check` (spec 11).

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
the name of a file is not compared with the name of its function (open question Q-26 of the spec). A folder given inside another one given
is one of its sub-libraries, even one that a walk of the outer folder would not reach, such as `.hidden`: each
file is checked once, under the outermost folder given.

**Resolving `use lib`**, first match wins:

1. the primitives of the language (`core`, `string`, `array`, `errors`, `time`, `ui`), which cannot
   be replaced;
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
(`LC2013`, `LC2016`, `LC2017`) is reported once, on the last file in path order for `LC2013` and on the first
for `LC2016` and `LC2017`, with the others in `related`.

A value that does not convert gets the code of where it goes: `LC3001` or `LC3004` on an input, `LC3003` on an
output, `LC3006` in an arm, `LC3007` on a carry, `LC3001` as an element of an array. So a narrowing to an output is
`LC3003`, and its fix still names the conversion. A type argument or an array makes a variable fixed, and an input against
it is an input like another: `LC3001` where two wires that meet would give `LC3002`.

### 4.3 Terminals

A terminal is written like the `wire` entries of the `layout` section (spec 11), with the names of
the signatures:

| Form | Terminal |
|---|---|
| `dmm.session`, `#close.err` | Output or input of a node, designated by the name of its line |
| `@12.err` | Output or input of a node with no name, on line 12: a call statement without a label, or a line whose outputs are all `_` or `next x` |
| `fn.power` | Input or output of the function: `fn` is reserved, so no node has that name |
| `next.n`, `until`, `collect.0`, `yield.1` | Carry `n`; the condition of the loop; a `collect` and a value of a `yield`, by position from 0 |

### 4.4 What is stable

From the M0a tag:

- **Codes.** A code keeps its meaning forever. A code that is no longer used stays reserved, and is
  never given another meaning. Its severity can change only with a release note.
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

Reserved for later specifications: `LC7xxx` for `diagram`, `LC8xxx` for `run` (faults and arguments),
`LC9xxx` for MCP.

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
| `LC2008` | error | Unknown unit | 3.3 |
| `LC2009` | error | Unknown named argument, a name given to a variadic parameter, or to a value of a variant | 4, 5.4 |
| `LC2010` | error | Unknown panel element in `ui.get` or `ui.set` | 10, R-1 |
| `LC2011` | error | Unknown label in `after:` | R-15 |
| `LC2012` | error | Value defined twice in a body | 5.1 |
| `LC2013` | error | Function or type defined twice in a library, or a function and a type with the same name | 4 |
| `LC2014` | error | Value with the name of a library that its file imports, of a type of its library or of a live panel element; function or type with the name of a library that any file of its library imports, on the declaration, with the `use` in `related` | R-1 |
| `LC2015` | error | Label used twice in a function | R-17 |
| `LC2016` | warning | Function that calls itself, directly or across files: a run stops at the limit of call depth (8.2). Only a call counts, not a function reference | 8.2 |
| `LC2017` | error | Type that contains itself. A cycle of types that passes through a `use` gives only `LC2021`, since its libraries are in a cycle too | 3.2 |
| `LC2018` | error | `next` for a name that is not a carry of the loop | R-6 |
| `LC2019` | error | Two parameters, outputs, fields or variants with the same name | 4, 3.2 |
| `LC2020` | warning | Library imported twice: `fmt` removes the line, unless it has a comment | 5.2 |
| `LC2021` | error | Libraries that use each other in a cycle | 4 |
| `LC2022` | error | Type with a name that the language gives to a type: a base type, `waveform`, `queue`, `ref`, `opaque` | 3.2 |

### Calls: `LC21`

| Code | Severity | Problem | Spec |
|---|---|---|---|
| `LC2101` | error | Required input not wired: once per call, on the first one, and not when the arguments of the call have a mistake of their own, which may be the cause | 4 |
| `LC2102` | error | Too many arguments, or an input wired twice | 4 |
| `LC2103` | error | Number of targets different from the number of outputs. A loop that gives nothing takes one `_` | 5.1, 6.2 |
| `LC2104` | error | Positional argument after a named one, `after:` included | 4 |
| `LC2105` | error | Invalid `after:` (a literal, or two in a call), or a parameter named `after` | R-15 |
| `LC2106` | error | `string.format`: template not a literal, or fields and arguments that differ | R-12 |
| `LC2107` | error | Variadic parameter misplaced, or more than one | R-11 |

### Types and units: `LC30`

| Code | Severity | Problem | Spec | |
|---|---|---|---|---|
| `LC3001` | error | Wrong type on an input, or what is not a unit, a category or an element on a parameter of that kind | 3, R-5 | S2 |
| `LC3002` | error | Incompatible dimensions in an operation | 3.3 | S2 |
| `LC3003` | error | Wrong type or dimension on a function output | 3.3 | S2 |
| `LC3004` | error | Implicit narrowing on an input, or two number types with no type that both widen to; the fix names the conversion when there is one | 3.4 | S2 |
| `LC3005` | error | Unit argument of another dimension | 3.3 | S2 |
| `LC3006` | error | Arms that disagree in number or type of values | R-7 | S2 |
| `LC3007` | error | Carry whose type changes at `next`, or whose initial value does not fit the type on its line | 6.2 | S2 |
| `LC3008` | error | Function reference of the wrong signature | 4 | S2 |
| `LC3009` | error | Pattern that cannot apply: of the wrong type, such as a variant of another enumeration, that binds another number of values than its variant holds, or a range that holds no value | 6.1 | S2 |
| `LC3010` | error | `match` without a branch for every value: a variant, `true` or `false` missing, or no `_` on an integer or a string. It names at most five variants. Silent on a boolean or an enumeration when one of the patterns has a diagnostic of its own, a repeated pattern included, since what it was meant to cover is unknown | R-8 | |
| `LC3011` | error | Overlapping literal or range patterns, in one branch or in two: on the pattern that starts inside another, with that one in `related` | R-18 | |
| `LC3012` | error | `handle` without `_` | R-10 | |
| `LC3013` | error | Selector of the wrong type in a `match` or `handle`. With a selector of unknown type, its arms get no `LC3010`, `LC3011`, `LC4106`, nor `LC3022` on a variant; a repeated `_` and a `handle` without `_` are still reported | 6.1, 7.3 | |
| `LC3014` | error | `until` that is not a `bool` | 6.3 | |
| `LC3015` | error | Struct or variant construction: field or value missing, or of the wrong type | 5.1 | |
| `LC3016` | error | Unit not computable: `sqrt` of an odd exponent or of a scale that is no exact square, `pow` without a constant integer exponent, an exponent or a scale that goes out of bounds through a chain of `mul` | 3.3 | |
| `LC3017` | error | Default value of the wrong type, for a parameter or a panel element, or the default of a parameter that is not a constant: a value of the diagram, a call of a function, `after:`, a type argument | 4, 10, R-21 | |
| `LC3018` | error | Integer literal that does not keep its value in its type: `300` for an `i8`, `16777217` for an `f32`, a bound of a range even when it is excluded (`0..256` on a `u8`) | 3.4, 6.1 | |
| `LC3019` | error | Type variable of a generic primitive that cannot be determined, a `[]` whose type nothing says, or the wrong number of type arguments | 3.5 | |
| `LC3020` | error | `for` clause over something other than an array, or a range with bounds that are not integers | 6.2 | |
| `LC3021` | error | Unit on a type that is not a number: `string[V]`, `duration[s]`, a declared type, even an alias of a number | 3.3 | |
| `LC3022` | error | The same pattern twice, in one branch or in two: `_`, a variant, or in a `handle` `ok`, a code or a category. On the second one, with the first in `related` | 6.1, 7.3 | |

### Dataflow and structures: `LC40`, `LC41`

| Code | Severity | Problem | Spec |
|---|---|---|---|
| `LC4001` | error | Cycle: a value can be fed back only through a carry | 5.2 |
| `LC4002` | error | Function output not bound, or bound twice | R-2 |
| `LC4003` | error | Carry with no `next`, or with two | R-6 |
| `LC4004` | error | `for parallel` with a carry | 6.2 |
| `LC4005` | error | External function without an `err` output, or that differs from its native body | R-13, 4.1 |
| `LC4006` | error | Feature planned for a later milestone: generic function or type written by the user, `ref<T>`, `queue<T>`… | 9 |
| `LC4101` | warning | `err` output wired nowhere | 7.5 |
| `LC4102` | warning | Concurrent access to a resource: two nodes with no dependency between them. Once per resource | 8.1 |
| `LC4103` | warning | Panel element written by two unordered nodes | 8.1 |
| `LC4104` | warning | `for parallel` body that uses a resource | 8.1 |
| `LC4105` | warning | Two reads of the same `button`: only one sees a press | 10 |
| `LC4106` | warning | `_` that can catch nothing, on a boolean or an enumeration whose values all have their pattern; the fix removes it from a list, and only says to remove its arm when it stands alone, since that arm can decide the type of the others (R-7). Silent when one of the patterns has a diagnostic of its own, a repeated pattern included | R-8 |

### Panel, layout, icon: `LC50`

| Code | Severity | Problem | Spec |
|---|---|---|---|
| `LC5001` | error | Panel element declared twice | 10 |
| `LC5002` | error | `control` with the name of an output only, or `indicator` with the name of an input only | 10 |
| `LC5003` | warning | Panel kind that M0 does not know (section 10); the element works without a widget | 10 |
| `LC5004` | warning | Layout entry that names no node; `fmt` removes it, unless it names an input or an output of the function | 11 |
| `LC5005` | error | Coordinate that is not an integer literal in the range (`5.0`, `1e3`, `2000000`) | 11 |
| `LC5006` | warning | Layout entry that is ambiguous (a name defined in several bodies, or a `wire` that does not say which terminals), or a `wire` between terminals that are not connected | 11 |
| `LC5007` | error | Two layout entries for the same node or wire | 11 |
| `LC5008` | error | Invalid icon: grid, `path` syntax, unknown `accent` token | R-14 |
| `LC5009` | warning | Icon shape that leaves the grid | R-14 |

### `fmt`: `LC60`

| Code | Severity | Problem | Spec |
|---|---|---|---|
| `LC6001` | error | `fmt --check`: file not canonical | 5.2 |
| `LC6002` | warning | Body left in written order: a cycle, or a value or a label defined twice | 5.2 |
| `LC6003` | error | Canonical form could not be written: over `--max-file-size`, or the file system refused it, as for a read-only file. The file is unchanged | section 2 above |

## 6. Left for later

No scenario of M0a needs these, and each can be added without breaking anything: applying fixes
(`check --fix`), reading from stdin, a `--diff` for `fmt`, per-project configuration of severities,
giving `check` a file or a selection of functions, `--json-out FILE` (as in `solidcraft-cli`, since a
shell's `>` can re-encode the output), a `--target web` for the error on external functions
(spec 4.1), and an `--exclude` for folders such as `target/` and `node_modules`, which are walked like any
other: a folder over the bounds of section 1 gives `LC0010` even if it holds no program.
