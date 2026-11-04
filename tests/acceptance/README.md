# Acceptance corpora

Files for the [M0 acceptance scenarios](../../docs/acceptance-m0.md), and what the tool must report on
them, in the terms of [docs/cli.md](../../docs/cli.md). Nothing runs them against Rust yet: the proof of
concept has no static type checking and no stable codes. `tools/lcf-poc/tests/test_corpus.py` checks
that the corpora themselves are sound, and that the proof of concept does not crash on them.

## `type-errors/`: scenario 2

One folder per case, one error in each. From inside the folder, `labcraft-cli check --json .` must
give `expected.json`:

- `schema`, `status`, `summary.errors` and `summary.warnings` are compared; the other counts are not;
- each diagnostic is compared field by field, except the text of `message` and `fixes`;
- `fix_mentions` is not a field of the report: some fix of the diagnostic must mention that text;
- `related` is compared by `file` and `span`.

Every program is in canonical form and has exactly one error, so a second diagnostic is a failure
("one error, one diagnostic"). Each code marked `S2` in `docs/cli.md` has one case.

## `hostile/`: scenario 4

One folder per case; these files are stored byte for byte (`.gitattributes`). `expected.json` gives,
for `labcraft-cli check .` and for `labcraft-cli fmt .` on a copy of the folder, the exit code and the
exact set of codes reported. `changes` says whether `fmt` rewrites a file. Whatever the file, the
process ends normally: never a panic, an abort, a stack overflow or memory exhaustion.

These families are too large or cannot be stored, so the test harness generates them:

| Family | Cases | Expected |
|---|---|---|
| Truncation | Every prefix, byte by byte, of every file of `examples/` and `libs/` | Exit 0 or 1; `fmt` leaves a file it cannot parse unchanged |
| Size | A file of 100 MB, one just over the limit, a line of 10 MB | `LC0006` when over the limit |
| Size | A function of 300,000 nodes under the limit, a library of 100,000 files | Ends within the bounds of scenario 4 |
| Size | A library of 100,001 files | `LC0010` |
| Nesting | 100,000 nested `match`, `loop` or `handle`, parentheses in a unit, nested array types | `LC0007` |
| Library | A path that does not exist, an unreadable file | `LC0001` |
| Library | A folder with nothing to check | `LC0002` |
| Library | A symbolic link loop, next to a clean file | `LC0009`, exit 0 |

The families Instruments, Arguments and MCP of scenario 4 come with M0b.
