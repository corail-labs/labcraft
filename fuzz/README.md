# Fuzzing

`cargo fuzz` targets (scenario 4, point 4, of [the acceptance scenarios](../docs/acceptance-m0.md)). This
folder needs a nightly, pinned in `rust-toolchain.toml`, and works poorly on Windows, so it stays out of the
workspace, with its own `Cargo.lock`: nothing here reaches the product, which builds on stable.

| Target | What it says |
|---|---|
| `read` | On any input up to 64 KiB, `format::read` ends, with a model or with an error and never both, with at most one error, a code that stands alone stays alone, a location is well formed, and the model can be cloned, compared and dropped |
| `round_trip` | On any input up to 64 KiB that parses, what `fmt` writes holds every line of the declaration and reads again, to the same model and the same text (scenario 5), every comment is still there, nothing is left to remove, a body that cannot be ordered still gets its warning, and the text ends with one line feed, with no CR and no space at the end of a line |
| `check` | On a library of up to 32 files, separated by two NUL bytes, each named by its length: for two lengths in four after what it declares (R-22), for one `f<N>`, for one after what it declares with the other extension (`LC0103`), and `f<N>` when it does not read or a file before it took the name, with the shipped libraries, `check` ends without `LC0099`, places each diagnostic in a file of the library or of a shipped one, with a well-formed span, gives the same report whatever the order of the files, and the same codes after `fmt` of each file, but for a duplicate `use` (`LC2020`), when `fmt` removes an entry (`LC5004`), and when a budget or a limit of `check` made it say less (`Checked::cut_short`), since what it says then depends on the order of lines and entries |
| `diagram` | On a library as for `check`, each file named after what it declares, `diagram` draws every function that `check` lets it draw, in both views, without an internal error, gives the same SVG when it draws it again, refuses under a limit of 3 nodes what has more (`LC7002`), and keeps the rules of routing of notation 2.2 as `tests/harness/src/routing.rs` reads them, but in a body whose file places a node and for a wire that the file gives waypoints |
| `simulation` | On any simulation file up to 64 KiB, `runtime::simulation::read` and the judgement of `visa` end, with the codes they declare, errors at well-formed spans, and the same judgement twice; on a file they accept, each instrument opens, is sent every command its section names (each query of `commands`, which answers as the file says, each of `measure`, which answers a finite number, each `set` with a number in range, out of range and not a number, each prefix of `accept`, each command of `faults`, a query of `timeout` timing out), the error queue and the reserved commands (`*IDN?` answers its `idn`), and closes, after which its session is refused; on the first instrument a second session leaves an answer unread (`-410`), reads, has a call cancelled and is released, which are run and not compared with what they should give; at most 4,000 calls in a run, past which the run makes no other call; every call ends in a few steps, the clock does not overflow, the log keeps its bounds and its order, and two runs of the file give the same answers and the same log (`libs/visa/simulation.md`) |
| `run` | On a library as for `diagram`, each file named after what it declares, a run of its first function through `engine::run_to_end`, against the instruments of `bench.toml`, with its trace inline, under small limits (200 iterations, 20,000 nodes, a call depth of 64, 1 MiB of memory, 4 MiB made, 60 s simulated), ends with a report of a status of spec 8.2 or `refused`, never with `LC8004`, and gives the same report run again, the real time aside |
| `mcp` | On any input, one request per line, the MCP server on a project in memory, a function that counts and one that doubles, with the shipped libraries, reads it to its end and stops, and every line it writes is JSON-RPC 2.0: an answer with an `id`, or a notification. Requests are bounded to 64 KiB, the input of libFuzzer |
| `trace` | On any input, read as the trace of a run of a fixed function (`trace/walk.lcf`: a `for` over an array with a range and three carries, a `match` that binds a value), the reader of the viewer ends within its limits (cli.md 7.4) and gives the same fed whole or in three parts; its diagnostics are `LC7101` to `LC7104`, each at a line of the file; a trace that gives no values shows none; and the values over the drawing (notation 6.1) and their SVG are the same worked out twice. The seed is the trace of a run of that function |

Larger inputs belong to the Size family of the corpus (`LC0006`), and their time and memory to `cargo xtask bounds`
(`tests/harness/tests/bounds.rs`).

## Running it

On Linux, or in a Linux container:

```sh
(cd fuzz && rustup toolchain install)
cargo install cargo-fuzz --version 0.13.2 --locked
bash fuzz/run.sh read 60
bash fuzz/run.sh round_trip 60
bash fuzz/run.sh check 60
bash fuzz/run.sh diagram 60
bash fuzz/run.sh simulation 60
bash fuzz/run.sh run 60
bash fuzz/run.sh mcp 60
bash fuzz/run.sh trace 60
```

The workflow `fuzz` (`.github/workflows/fuzz.yml`) builds every target and `deep_seeds` once, then runs the targets
with `bash fuzz/side_by_side.sh 60 <target>...`, four at a time, the vCPUs of the runner, so that two targets never
share one. `side_by_side.sh` runs `run.sh` for each target at once, keeps each output in `target/<target>.out`, prints
them one after the other when all have ended, or when the job stops it, and fails when one of them failed, naming it.
Run on a machine with fewer CPUs than targets, the targets share them and each fuzzes less in its 60 s.

`run.sh` runs one target, for a number of seconds. It also fails, with exit 3, when
libFuzzer stopped before 90% of the time, which it calls a success when its clock jumps. The time is read from
`/proc/uptime`, which setting the clock does not move; it still counts a suspend, and without it (macOS) the check
cannot see a jump. `bash fuzz/run.sh <target> --replay <file>` runs one input again, alone, with the same limits. `--release` with debug assertions and AddressSanitizer are the defaults of `cargo fuzz`.

**The seeds** are every file of `examples/`, `libs/`, `docs/rfc/` and `tests/acceptance/`, some of which are not
programs, the simulation files of `libs/visa/sim/` among them, plus what `deep_seeds` generates: the Nesting family, from
`crates/format/tests/deep`, the files of each folder of those four joined, as many as fit in 64 KiB, for `check`, and
simulation files nested at the limit of the reader of TOML and one level above it, for `simulation`, and a session of
requests, for `mcp`. libFuzzer writes
what it finds to `corpus/<target>`, ignored by git. Each run of the job starts again from the seeds.

**In Docker Desktop**, add `--security-opt seccomp=unconfined`: the sanitizer may need `personality`, which the
default profile refuses, and then fails at random, before reading anything. The clock of Docker Desktop can jump
by many seconds, which gives false `slow-unit` files, false timeouts and runs that stop early. Run such a file
again alone, with `--replay`, before believing it.

**The long run before a tag** (8 hours per target, scenario 4) is `bash fuzz/long.sh <target> 8` for each target, on a
machine of your own that does not sleep, since a job on a GitHub runner stops at 6 hours. The targets can run side by
side. `long.sh` runs `run.sh` in parts of one hour, and counts an hour only when its part succeeded:

- a part that stopped early is run again;
- a timeout is run again alone: if it passes, the clock or the load made it, and the part is run again; if it times
  out again, the target stops, as on a crash.

Each part starts by running the whole of `corpus/<target>` again, within its hour: with ten thousand inputs, `check`
spends several minutes there. `target/long/<target>.log` has one line for each part, `target/long/<target>.<N>.out` the
output of part `N`, and `target/long/<target>.<N>.<artifact>.out` that of a timeout run again alone. `corpus/<target>`
stays between parts and between runs, so the run can also be done in several sittings: a later sitting goes on with the
next `N`. In Docker Desktop, from the root of the repository:

```sh
docker run --rm -it --security-opt seccomp=unconfined -v "$PWD:/src" -w /src rust:1 bash
```

then the commands of "Running it" above, and `bash fuzz/long.sh <target> 8`.

## The stack

The targets run at the default limit of nesting, 256: `read` and `round_trip` on the main thread of libFuzzer, `check` on
a thread of `format::STACK_SIZE`, as the program does. With the sanitizer `read` needs
at most 1 MiB there (measured, every kind of nesting of `crates/format/tests/deep`), and a Linux main thread
has 8 MiB. `round_trip` reads the same deep seeds on that thread; the parser takes the most stack of what it runs
(`crates/format/tests/stack.rs`). The ceiling of nesting and `format::STACK_SIZE` are the business of `crates/format/tests/stack.rs`.
`simulation` runs on the main thread: the reader of TOML recurses at most 80 levels deep, and the count of
`runtime::simulation` before it does not recurse (cli.md 6.2).

## When it finds something

The job keeps what crashed as the artifact `fuzz-artifacts`, and the log gives a command to replay it.

1. Replay it (`bash run.sh <target> --replay artifacts/<target>/<file>`), and make it smaller:
   `cargo fuzz tmin <target> artifacts/<target>/<file>`, from this folder.
2. Fix the bug.
3. The file becomes a case of [`tests/acceptance/hostile/`](../tests/acceptance/README.md), with its
   `expected.json`; for the target `diagram`, a case of [`tests/harness/routing-cases/`](../tests/harness/routing-cases),
   listed in the test `the_hard_cases_keep_the_rules` of `tests/harness/src/routing.rs`.

## Keeping it up to date

- **`Cargo.lock`:** when a dependency of a crate of the workspace changes, or the version of the workspace, run
  `cargo metadata --format-version 1 > /dev/null` from this folder. `cargo xtask fuzz-licences`, from the root,
  fails on a stale lock. It reads the licences of this folder, which the command `licences` does not see.
- **The nightly:** change `channel` in `rust-toolchain.toml`, in a pull request of its own.
