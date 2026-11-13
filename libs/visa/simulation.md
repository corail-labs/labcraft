# Simulated instruments of `visa`

*Last updated: 2026-11-13. Status: implemented for milestone M0b (tag `v0.2.0-m0b`). The proof of concept (`tools/lcf-poc`) and the crate
`labcraft-visa` (`src/`) implement it, the refusals of section 7 included. Values marked *proposed* are starting points.*

`visa` answers for the resources `SIM::…` as the instruments of a simulation file describe them, so a
program, and the drivers it uses (`scpi`, a driver written over `visa`), runs and is tested without
hardware. This document is the contract of that description: what a file holds, how a simulated instrument
answers, what is refused. It belongs to `visa`, not to the language: another transport (Modbus, a data
acquisition card) simulates at its own level, in its own section of the same file.

## 1. The simulation file and the `visa` section

A simulation file describes a bench: all its simulated instruments, whatever library talks to them. It is
TOML, and each key at its top level belongs to the library of that name ([cli.md](../../docs/cli.md),
section 6.2): `labcraft-cli run` reads the file, gives each library its table, and reports a key that no
library claims. In M0, `visa` is the only library that simulates, so a file holds at most a `visa` table.
A file with none, such as `no-instrument.toml`, defines no instrument: every `visa.open` fails.

```toml
# A multimeter that reads 230 V, with a little noise.
[[visa.instrument]]
resource = "SIM::DMM0"
idn = "SIM,DMM,0,1.0"

[visa.instrument.measure]
"MEAS:VOLT:DC?" = "voltage"

[visa.instrument.quantity.voltage]
value = 230.0
noise = 0.05
seed = 1
```

The `visa` table holds one key, `instrument`, an array of instruments. Every other key, at every level,
is refused (section 7): a misspelt `noize` must not run with no noise and say nothing.

## 2. An instrument

| Key | Value | Default |
|---|---|---|
| `resource` | The VISA resource name, starting with `SIM::`; unique in the file | required |
| `idn` | The answer to `*IDN?` | `SIM,<resource>,0,1.0` |
| `latency_ms` | The time each command takes, on the clock of the run | 0 |
| `commands` | A table: a query and its fixed answer (`"*OPC?" = "1"`) | none |
| `measure` | A table: a query and the quantity it reads (`"MEAS:VOLT:DC?" = "voltage"`) | none |
| `set` | A table: the header of a write and the quantity it sets (`"VOLT" = "voltage"`, for `VOLT 5`) | none |
| `accept` | An array of prefixes of writes that are accepted and do nothing (`"ROUT:CLOS"`) | none |
| `quantity` | A table of quantities, by name (section 3) | none |
| `faults` | Faults to inject (section 5) | none |

A query, a header, a prefix and an answer are texts of at most 1,000 code points with no control
character (Unicode category Cc, as for `LC0004` in a program). A query ends with `?`, and the keys of
`commands` and `measure` and the entries of `faults.timeout` are queries. These keys and entries, and those
of `set` and `faults.error_on`, have no space (U+0020) at either end, are not empty, and none is a command the simulator answers itself
(`*IDN?`, `SYST:ERR?`, `*CLS`, `*RST`); a query is in `commands` or in `measure`, not both. A quantity name
is an ASCII name (a letter or `_`, then letters, digits or `_`), so that `SIM::PSU0.voltage` reads one way.

A command matches a key exactly, case included: `meas:volt:dc?` and the short form `MEAS:VOLT?` are other
commands in M0. Accepting the case and the short forms of SCPI can come later without breaking a file.

## 3. A quantity

A quantity is what an instrument measures or sets: a voltage, a current, the presence of a product. It has
a **true value**, which changes with time, and a **noise**, added each time it is read.

| Key | Value | Default |
|---|---|---|
| `value` | The value it holds, or reaches after its ramp | 0 |
| `initial` | The value it holds before `start_ms` | `value` |
| `start_ms` | When it starts to move from `initial` to `value` | 0 |
| `settle_ms` | How long a move takes: from `initial` to `value`, and after each write of `set` | 0 |
| `noise` | The standard deviation of the noise, in the unit of the value | 0 |
| `seed` | The seed of its noise, an integer from 0 to 2^63−1; required when `noise` is above 0, so that two quantities never share a noise by mistake | 0 |
| `source` | Another quantity whose true value it takes, as `"SIM::PSU0.voltage"`: a multimeter wired to a source. With `source`, `value`, `initial`, `start_ms` and `settle_ms` are refused, and no write of `set` may set it | none |

A value, an initial value and a noise are finite numbers of at most 10^100 in size, compared as 64-bit
floats with `1e100`; a time is from 0 to one day (86,400,000 ms). An integer of the file fits in 64 bits, as
TOML requires: a larger one makes the file not TOML (`LC8109`, a code of the tool).

**The clock** of the run counts whole nanoseconds, from 0 to 2^62 ([cli.md](../../docs/cli.md), section 6.2). Each time of the file, in milliseconds, is read as a
64-bit float, multiplied by 10^6 in 64-bit floats, rounded to nearest, then rounded to the nearest integer,
ties to even: that integer is its number of nanoseconds (`latency_ms = 0.1` is 100,000 ns). So a sum of
latencies is exact, and ten commands of 5 ms end at 50 ms, not a little before.

**The true value at time t** (in ns): `initial` before `start_ms`; then, while
`start_ms ≤ t < start_ms + settle_ms`, `initial + (value − initial) × ((t − start_ms) / settle_ms)`, where
the two differences of times are exact integers, each converted to a 64-bit float, and the operations are
done in that order, each rounded to nearest, ties to even; then `value`. A write of `set` at time t starts a
new move, from the true value at t to the value written, over `settle_ms`. A quantity with a `source` has
the true value of its source at the same time; a chain of sources ends, and a loop is refused.

**The noise** is drawn from a generator that each quantity owns, so that two quantities never share their
draws (spec 8), and that every implementation gives the same numbers:

1. The generator is SplitMix64, started at `seed`. Each draw adds `0x9E3779B97F4A7C15` to the state, then
   returns `z ^ (z >> 31)`, after `z = state`, `z = (z ^ (z >> 30)) * 0xBF58476D1CE4E5B9`,
   `z = (z ^ (z >> 27)) * 0x94D049BB133111EB`, all modulo 2^64.
2. Each read takes 12 draws, keeps the top 53 bits of each (`draw >> 11`), and adds them as integers
   into `S`. The noise factor is `g = (S − 6 × 2^53) × 2^−53`, where `S − 6 × 2^53` is converted to a
   64-bit float, rounding to nearest, ties to even, and the product is exact. `g` follows the sum of 12
   uniform values, which is close to a normal law of deviation 1, and stays within ±6.
3. The value read is `true + noise × g`: a product, then a sum, each a 64-bit float rounded to nearest,
   never fused into one operation.

Every read draws, `noise = 0` included, so that the draws of a quantity depend only on how many times it
was read. Only integer operations, conversions, and rounded additions, products and divisions take part: no
library function, whose last bit can differ between platforms. So a value can be worked out by hand or with
any other tool, which is how the tests get their expected values: seed 1 gives `g = 1.4243480905156507`
first, so `dmm-230V.toml` answers `230.071217`.

## 4. How an instrument answers

A program sends a command with `visa.write` or `visa.query`. The command is the text it gave, without a
termination it may have written (one `\n` or `\r\n` at the end), then without spaces (U+0020) at both ends. Any other
control character stays in it, so it matches no key. Each command first takes `latency_ms` on the clock of
the run, then, in this order:

1. A query in `faults.timeout` gets no answer: the read that waits for it ends with its timeout
   (below). The instrument does nothing else with it, so an entry of `error_on` for it adds no error; it stays free, and
   handles the next command as usual.
2. A command in `faults.error_on` adds its error to the error queue of the instrument, then goes on.
3. `SYST:ERR?` answers the oldest error of the queue and removes it, as `-113,"Undefined header"`, or
   `0,"No error"` when the queue is empty; a `"` in a message is doubled, as in a SCPI string. `*IDN?` answers `idn`. `*CLS` empties the queue. `*RST` does
   nothing in M0.
4. A query of `commands` answers its text. A query of `measure` reads its quantity (section 3) and
   answers it with 6 decimals: the exact value rounded to 6 decimals, ties to even, with `.`, no exponent,
   and `-` when it is negative, `-0.000000` included (`230.071217`).
5. A write (not a query) whose header is in `set` (the text before the first space) sets its quantity to
   the number after the spaces that follow the header. The number is written as SCPI writes a decimal
   number, in ASCII digits: an optional sign, digits with an optional fraction, and an optional exponent
   (`5`, `-0.5`, `.5`, `1.2E3`); otherwise the write adds
   `-224,"Illegal parameter value"` to the queue. A number above 10^100 in size adds
   `-222,"Data out of range"`. Either way the quantity does not change.
6. A write that is not a query and starts with a prefix of `accept` does nothing.
7. Any other command adds `-113,"Undefined header"` to the queue; if it is a query, it is not answered.

**Answers and reads.** The answer of a query waits in the session until `visa.read` takes it; `visa.query`
is a write followed by a read. A read with no answer waiting waits for the timeout of the session, then
fails (section 6): so does the read of a query that is never answered. A read with an answer waiting takes
no time. A new command sent while an answer is still waiting discards it, and adds
`-410,"Query INTERRUPTED"` to the queue when the instrument handles that command, before step 1: the queue follows the order
in which the instrument handles its commands, whichever session sends them.

**The error queue** holds at most 32 errors *(proposed)*, as SCPI allows: one more replaces the last by
`-350,"Queue overflow"`. So a program that never reads its errors cannot fill the memory.

**One command at a time.** An instrument handles one command at a time, whichever session sends it: a
command waits until the one before it on that instrument has ended, on the clock of the run (spec 8.1 locks
a session during a call; two sessions on one instrument still meet here). A command ends when its latency
has passed: a read that waits for an answer, or for its timeout, waits in its session and does not hold the
instrument.

**In real time** (`--real-time`, spec 8; decision of the owner, 2026-11-11), an instrument is the same machine on
the clock of the run, which is then the real one: `latency_ms`, the move of a quantity (`start_ms`, `settle_ms`) and the
timeout of a session take that much real time, from the start of the run. The noise, the answers and the error queue do
not change: the noise of a quantity depends on how many times it is read, not on when, so a program whose order of reads
does not depend on time reads the same values in both modes. Only the times differ, by what the machine adds. A simulated
call never blocks a thread: it waits as a node does, so the other branches go on.

## 5. Faults

| Key | Value |
|---|---|
| `timeout` | An array of queries that are never answered: an instrument that hangs |
| `error_on` | A table: a command and the error it adds to the queue, as `[code, "message"]` (`"MEAS:CURR?" = [-221, "Settings conflict"]`) |
| `error_queue` | An array of errors in the queue when the run starts, as `[code, "message"]` |

An error code is an integer from −2^31 to 2^31−1, and its message a text as in section 2.

## 6. What the program sees

| Case | Error on `err` (spec 7.1) |
|---|---|
| `visa.open` of a resource the file does not define | `visa.resource_not_found`, code `-1073807343` (`VI_ERROR_RSRC_NFOUND`), after the timeout of the open |
| A read with no answer, in time | `io.timeout`, code `-1073807339` (`VI_ERROR_TMO`), after the timeout of the session |
| A session that is closed or was never opened | `visa.invalid_session`, code `-1073807346` (`VI_ERROR_INV_OBJECT`) |

The codes are those of VISA, so that a program written against the simulator reads the same codes from a
real instrument (M4). An error in the queue is not an error of the call: the program reads it with
`SYST:ERR?`, as `scpi.check_errors` does.

## 7. What is refused

`visa` judges its part of the file before the run starts, and the run is `refused` if anything is wrong
([cli.md](../../docs/cli.md), section 6.1). Each problem is a diagnostic located at its key or its value,
with a code of `visa`, which keeps its meaning forever as a code of the tool does (cli.md, section 4.4):

| Code | Severity | When |
|---|---|---|
| `visa:S001` | error | A key that the format does not have; fix: the nearest key |
| `visa:S002` | error | A value of the wrong TOML type (`latency_ms = "5"`, `accept = [5]`, `seed = 1.5`) |
| `visa:S003` | error | An instrument without `resource`, a resource that does not start with `SIM::`, or a quantity with a noise and no `seed` |
| `visa:S004` | error | A resource defined twice |
| `visa:S005` | error | A number out of its range (section 3): a time below 0 or above one day, a noise below 0, a value above 10^100 in size, `nan` or `inf` anywhere, a seed or an error code out of its range |
| `visa:S006` | error | `measure` or `set` names a quantity the instrument does not have, or `set` names a quantity that has a `source` |
| `visa:S007` | error | A `source` that names no quantity of the file, a chain of sources that loops, or a `source` with `value`, `initial`, `start_ms` or `settle_ms` |
| `visa:S008` | error | A text over 1,000 code points, with a control character, or a quantity name that is not an ASCII name |
| `visa:S009` | error | An entry of `error_on` or `error_queue` that is not `[code, "message"]` |
| `visa:S010` | error | More than 256 instruments, 1,000 quantities in one instrument, 10,000 entries in one table or array, or 32 errors in `error_queue`, the size of the queue *(proposed)* |
| `visa:S011` | error | A key of `commands`, `measure`, `set` or `error_on`, or an entry of `faults.timeout`, with a space at an end, empty, or a command the simulator answers itself; a key of `commands` or `measure`, or an entry of `faults.timeout`, that is not a query; a query in both `commands` and `measure` |

A problem of the file as TOML (it does not parse, it is not UTF-8, an integer does not fit in 64 bits, it
nests deeper than `--max-nesting`, or it is larger than `--max-file-size`) is judged by the tool before `visa` sees it, and has a code of the tool
([cli.md](../../docs/cli.md), section 6.6).

## 8. The files shipped with `visa`

`libs/visa/sim/`, shown as `<labcraft>/visa/sim/…` in reports and listed by `project.describe`
([commands.md](../../docs/commands.md), section 6.2):

| File | What it simulates |
|---|---|
| `dmm-230V.toml`, `dmm-250V.toml` | A multimeter that reads 230 V or 250 V (scenario 1) |
| `no-instrument.toml` | No instrument (scenario 1) |
| `dmm-timeout.toml` | A multimeter that never answers its measure |
| `dmm-scpi-error.toml` | A multimeter with errors in its queue |
| `dmm-settle-300ms.toml`, `dmm-settle-3s.toml` | A multimeter whose reading settles in 300 ms or 3 s |
| `dmm-5V.toml`, `dmm-5V-high.toml` | A multimeter that reads about 5 V |
| `psu-1W5.toml`, `psu-2W5.toml`, `psu-timeout.toml` | A power supply, and one that hangs |
| `bench.toml` | A multimeter wired to a source, and a relay matrix: the bench of `station` |
| `two-dmm.toml`, `two-dmm-timeout.toml` | Two multimeters without latency, and the same with the first one hung (scenario 3) |

## 9. Describing a file

`simulation.describe` ([commands.md](../../docs/commands.md), section 6.3) gives, for the `visa` part of a
file, each instrument in the order of the file: its resource, `idn`, `latency_ms`, the queries it answers
with what each reads (the quantity, its true value at 0 ms, its noise), the writes of `set` and `accept`,
and its faults; or the diagnostics of section 7. An agent learns from it the resource names to open,
without reading the file.

## 10. Hostile files

A simulation file is read as hostile as a program is (scenario 4, family Instruments): it never crashes the
tool, and every problem is a diagnostic. Its size and its nesting are bounded by `--max-file-size` and
`--max-nesting`, and its nesting by 80 levels at most, as the reader of TOML goes (cli.md 6.2), its tables by section 7, and a chain of sources is followed without recursion, once, when
the file is read. The fuzz target `simulation` reads and judges simulation files, and runs every instrument of a file it
accepts, as the other targets are run (`fuzz/README.md`, `fuzz/run.sh`, `fuzz/long.sh`), seeded with the files of section 8.
