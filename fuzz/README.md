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
```

`run.sh` is what the workflow `fuzz` runs (`.github/workflows/fuzz.yml`), for 60 s. It also fails, with exit 3, when
libFuzzer stopped before 90% of the time, which it calls a success when its clock jumps. The time is read from
`/proc/uptime`, which setting the clock does not move; it still counts a suspend, and without it (macOS) the check
cannot see a jump. `bash fuzz/run.sh <target> --replay <file>` runs one input again, alone, with the same limits. `--release` with debug assertions and AddressSanitizer are the defaults of `cargo fuzz`.

**The seeds** are every file of `examples/`, `libs/`, `docs/rfc/` and `tests/acceptance/`, some of which are not
programs, plus what `deep_seeds` generates: the Nesting family, from `crates/format/tests/deep`, and the files of each
folder of those four joined, as many as fit in 64 KiB, for `check`. libFuzzer writes
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

## When it finds something

The job keeps what crashed as the artifact `fuzz-artifacts`, and the log gives a command to replay it.

1. Replay it (`bash run.sh <target> --replay artifacts/<target>/<file>`), and make it smaller:
   `cargo fuzz tmin <target> artifacts/<target>/<file>`, from this folder.
2. Fix the bug.
3. The file becomes a case of [`tests/acceptance/hostile/`](../tests/acceptance/README.md), with its
   `expected.json`.

## Keeping it up to date

- **`Cargo.lock`:** when a dependency of a crate of the workspace changes, or the version of the workspace, run
  `cargo metadata --format-version 1 > /dev/null` from this folder. `cargo xtask fuzz-licences`, from the root,
  fails on a stale lock. It reads the licences of this folder, which the command `licences` does not see.
- **The nightly:** change `channel` in `rust-toolchain.toml`, in a pull request of its own.
