# `file`: files and folders

*Status: specified for release 0.5.0 (M4, [docs/io.md](../../docs/io.md)), not built yet: its declarations and its
crate `labcraft-file` come with its Rust.*

`file` reads and writes real files, as text or as bytes, and lists, creates, renames and deletes them: to write the
results of a test, to read a list of steps. Its functions are native (`extern rust`, spec 4.1). Where a relative path
leads, and what a run of an agent may touch, is in [io.md 5](../../docs/io.md); reading a file into a declared type
(`csv`, `json`) waits for the generics of M2.

## Functions

```text
fn file.read_text(path: string, err: error = none) -> (text: string, err: error)
fn file.read_bytes(path: string, err: error = none) -> (data: bytes, err: error)
fn file.write_text(path: string, text: string, err: error = none) -> (err: error)
fn file.write_bytes(path: string, data: bytes, err: error = none) -> (err: error)
fn file.append_text(path: string, text: string, err: error = none) -> (err: error)
fn file.append_bytes(path: string, data: bytes, err: error = none) -> (err: error)
fn file.exists(path: string, err: error = none) -> (exists: bool, folder: bool, err: error)
fn file.list(folder: string, err: error = none) -> (files: [string], folders: [string], err: error)
fn file.create_folder(path: string, err: error = none) -> (err: error)
fn file.rename(from: string, to: string, err: error = none) -> (err: error)
fn file.delete(path: string, err: error = none) -> (err: error)
```

- **`read_text`** reads a UTF-8 file, a byte order mark at its start removed; one that is not UTF-8 is `io.invalid_text`.
  Line endings stay as they are.
- **`write_text`** and **`write_bytes`** create the file, or replace it whole, at once (io.md 5): a reader sees the old
  file or the new one. The folder must exist (`file.not_found` otherwise).
- **`append_text`** and **`append_bytes`** add at the end, creating the file if it does not exist: a log of results
  line by line. Each call adds all its text or nothing.
- **`exists`** says whether the path is there, and whether it is a folder. It does not fail on a path that is not
  there; it fails on one it may not read.
- **`list`** gives the names (not the paths) of the files and of the folders directly in a folder, each in code point
  order. A link is listed by its name, as what it leads to.
- **`create_folder`** creates the folder and those above it that are missing; one that exists already is no error.
- **`rename`** moves a file or a folder; one at `to` already is `file.exists`, and nothing changes.
- **`delete`** removes a file or an empty folder; a folder that holds something is `file.not_empty`. Nothing deletes
  a tree in one call.

Errors: `file.not_found`, `file.exists`, `file.not_empty`, `file.denied` (the system refuses), `io.invalid_text`,
`io.not_allowed` (io.md 4 and 5), `io.unsupported` (a host without files, the browser), each with the code of the
system (io.md 7). A file read whole is held in the memory of the run, so a file larger than what the run may hold is a
fault (`LC8005`, spec 8.2).

## What `file` guarantees

- **Each call is atomic on its file** within a run (io.md 5), and the paths it touches are resolved once, at the call.
- **Files are real under `--sim` too** (io.md 5), and in both modes: a deterministic run that reads a file which
  changes between two runs gives another result, as one given other arguments would.
- `file` holds nothing open between two calls, so an abort leaves no file open; a write in progress ends, or does not
  happen.
