# Fuzzing

`cargo fuzz` targets (scenario 4, point 4, of [the acceptance scenarios](../docs/acceptance-m0.md)). This
folder needs a nightly, pinned in `rust-toolchain.toml`, and works poorly on Windows, so it stays out of the
workspace, with its own `Cargo.lock`: nothing here reaches the product, which builds on stable.

| Target | What it says |
|---|---|
| `read` | On any input up to 64 KiB, `format::read` ends, with a model or with an error and never both, with at most one error, a code that stands alone stays alone, a location is well formed, and the model can be cloned, compared and dropped |

Larger inputs belong to the Size family of the corpus (`LC0006`), which comes with step 6 of
[the architecture](../docs/architecture.md). The round trip of `fmt` comes with step 5, and `check` with step 9.

## Running it

On Linux, or in a Linux container:

```sh
(cd fuzz && rustup toolchain install)
cargo install cargo-fuzz --version 0.13.2 --locked
bash fuzz/run.sh read 60
```

`run.sh` is what the workflow `fuzz` runs (`.github/workflows/fuzz.yml`), for 60 s. It also fails when too little
was executed. `--release` with debug assertions and AddressSanitizer are the defaults of `cargo fuzz`.

**The seeds** are every file of `examples/`, `libs/`, `docs/rfc/` and `tests/acceptance/`, some of which are not
programs, plus the Nesting family, which `deep_seeds` generates from `crates/format/tests/deep`. libFuzzer writes
what it finds to `corpus/read`, ignored by git. Each run of the job starts again from the seeds.

**In Docker Desktop**, add `--security-opt seccomp=unconfined`: the sanitizer may need `personality`, which the
default profile refuses, and then fails at random, before reading anything. The clock of Docker Desktop can jump
by many seconds, which gives false `slow-unit` files, false timeouts and runs that stop early. Time such a file
alone before believing it.

**The long run before a tag** (8 hours, scenario 4) is `bash fuzz/run.sh read 28800` on a machine of your own, since a
job on a GitHub runner stops at 6 hours. `corpus/read` stays between runs, so it can be done in several parts.

## The stack

`read` runs at the default limit of nesting, 256, on the main thread of libFuzzer. With the sanitizer it needs
at most 1 MiB there (measured, every kind of nesting of `crates/format/tests/deep`), and a Linux main thread
has 8 MiB. The ceiling of nesting and `format::STACK_SIZE` are the business of `crates/format/tests/stack.rs`.

## When it finds something

The job keeps what crashed as the artifact `fuzz-artifacts`, and the log gives a command to replay it.

1. Replay it, and make it smaller: `cargo fuzz tmin read artifacts/read/<file>`, from this folder.
2. Fix the bug.
3. The file becomes a case of [`tests/acceptance/hostile/`](../tests/acceptance/README.md), with its
   `expected.json`.

## Keeping it up to date

- **`Cargo.lock`:** when a dependency of a crate of the workspace changes, or the version of the workspace, run
  `cargo metadata --format-version 1 > /dev/null` from this folder. `cargo xtask fuzz-licences`, from the root,
  fails on a stale lock. It reads the licences of this folder, which the command `licences` does not see.
- **The nightly:** change `channel` in `rust-toolchain.toml`, in a pull request of its own.
