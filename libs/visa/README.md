# `visa`: sessions on instruments

*Status: the simulator of `SIM::` resources is implemented for milestone M0b (tag `v0.2.0-m0b`) by the crate
`labcraft-visa`, in `src/`, in deterministic mode and in real time. From release 0.5.0 (M4,
[docs/io.md](../../docs/io.md)), specified, not built yet: `visa` is written in LabCraft over `net`, `serial` and its
sub-library `visa.sim`, its session is the enum `visa.Session`, and it reaches real instruments over TCP and serial
(decisions of the owner, 2026-11-24 and 2026-11-26). Values marked (proposed) are starting points.*

`visa` is the library shipped with LabCraft that opens sessions on instruments, writes commands and reads answers.
Instrument drivers and `scpi` are libraries over it. Not every instrument goes through VISA: another transport is
another library, with its own rules.

**`visa` is written in LabCraft** (from 0.5.0), as `scpi` is, so that a user or an agent opens it in the editor and
reads how a name becomes a connection, and a contributor adds a form of VISA name over a new transport the same way.
It holds no transport of its own: `net` reaches TCP, `serial` the serial ports, and its sub-library `visa.sim` the
simulated instruments of a simulation file; those three are native. Only `visa.resolve`, which reads a resource name,
is native in `visa` itself, so that the name a human allows (`--allow-hardware`, io.md 4) and the name `visa` opens
are read by one piece of code (`runtime::io::endpoint`, [architecture.md](../../docs/architecture.md) 10.6).

## Functions

| Function | Does |
|---|---|
| `open(resource, timeout = 2 s)` | Resolves the name (below), then opens its transport, which keeps `timeout` for each of its reads and writes. `visa.resource_not_found` if nothing answers to the name within `timeout`; on any failure, the session is `Closed` |
| `write(session, command)` | Sends the command followed by `\n`, unless it ends with one |
| `read(session)` | Reads an answer up to its `\n`; `io.timeout` if none comes within the timeout of the session |
| `query(session, command)` | `write`, then `read` |
| `write_bytes(session, data)` | Sends bytes as they are, with no termination |
| `read_bytes(session, count)` | Reads exactly `count` bytes; `io.timeout` if they do not all come within the timeout, and nothing is taken |
| `set_serial(session, baud = 9600, …)` | Gives a serial session other settings, by `serial.configure`; accepted with no effect by a simulated instrument with an `ASRL` name; `io.unsupported` on any other session |
| `close(session)` | Closes the transport of the session; on a session already closed, or `Closed`, succeeds and does nothing |
| `resolve(resource)` | (native) Where a name goes in this run: a `visa.Route` |

```text
type visa.Session = enum {
    Closed,                          // the default value: a session never opened
    Tcp(net.connection),
    Serial(serial.port),
    Simulated(visa.sim.instrument),
}
type visa.Route = enum { Simulated(string), Tcp(visa.Address), Serial(string) }
type visa.Address = struct { host: string, port: u16 }
```

- **A session is an enum, not an opaque type** (decision of the owner, 2026-11-26), so that the functions of `visa`
  choose the transport of a session in LabCraft, with a `match`. Its name is `Session`, since a library cannot have a
  value named after one of its types other than an opaque one (spec, R-1), and every function of `visa` names its
  session `session`. It was the opaque `visa.session` until 0.5.0; the drivers that alias it are renamed with it
  (`psu.Session`, `relays.Bank`).
- **A session is a resource**, since it holds one (spec 8.1): two nodes that receive one with no order between them get
  the warning of spec 8.1. A program may `match` on a session, and may make one from a connection it opened itself
  (`visa.Session.Tcp(connection)`), to read and write it as an instrument. In JSON (commands.md 5) it shows as an
  enumeration does, the handle inside as an opaque value: `{ "visa.Session.Simulated": [{ "opaque":
  "visa.sim.instrument", "resource": "SIM::DMM0", "open": true }] }`, and `"visa.Session.Closed"`.
- **`close` is written in LabCraft**, so its body runs even after an error (spec 7.2), and the close of each transport
  is `always`: a session is closed after a failed measurement, as before. Like an `always` function, `close` gives the
  error it receives on its `err`, not its own, when it receives one (spec 7.2). The manifest of `visa` therefore marks no
  function `always` and declares no resource; those of `net`, `serial` and `visa.sim` do. The manifest of `visa` lists
  `net`, `serial` and `visa.sim` in its `dependencies`, and that of `visa.sim` lists `serial`, for `serial.Parity`.

## Where a name goes

`visa.resolve` reads a resource name and gives its route in this run, by the table of io.md 3: a simulation file is the
whole bench, and without one a real name reaches the hardware.

| Name | Under `--sim` | Without `--sim` |
|---|---|---|
| `SIM::DMM0` | `Simulated`: the instrument of that name in the file | `Simulated`: no file, so `visa.sim.open` finds nothing |
| `TCPIP0::10.0.0.5::5025::SOCKET` | `Simulated`: the instrument with that name in the file | `Tcp`: a raw TCP socket, the endpoint `tcp:10.0.0.5:5025` |
| `ASRL3::INSTR`, `ASRL/dev/ttyUSB0::INSTR` | `Simulated`, as above | `Serial`: `COM3`, or the path `/dev/ttyUSB0` |
| VXI-11 `::INSTR`, `::hislip0::INSTR`, `USB…`, `GPIB…` | `io.unsupported` | `io.unsupported` |
| `ASRL0::INSTR`, any text that is no VISA name | `visa.resource_not_found`, at once | `visa.resource_not_found`, at once |

- `Simulated` holds the name as it was given, which `visa.sim.open` reads again: the route says where a name goes, and
  the simulator finds its instrument.
- A real name is read without case, as VISA reads it, apart from a path inside it, and reduces to its endpoint (io.md 2);
  a `SIM::` name is compared exactly, as in M0.
- **`ASRL<n>::INSTR` is `COM<n>` on every system** (decision of the owner, 2026-11-26), the name `serial.open` is
  given: on Windows it opens that port; elsewhere `serial` finds no `COM` port, and the open fails with
  `visa.resource_not_found`, with in its `others` the `serial.port_not_found` whose message, written by `serial`, which
  knows the system, says to name the port by its path (`ASRL/dev/ttyS2::INSTR`, libs/serial, "Functions", `open`). So a
  simulation file means the same on every system, and the endpoint a human allows is the port that opens.
- **Under `--sim`, every name goes to `visa.sim`**, which finds it among the instruments of the section `visa` or
  gives `visa.resource_not_found` after the timeout of the open, as `visa.open` does today: a typo in the file never
  sends a command to a real supply (io.md 3). A raw device of the section `net` or `serial` is not a VISA instrument.
- `resolve` reaches no device and asks no question: the consent of io.md 4 and `io.needs_real_time` come at the open of
  the transport, from `net` or `serial`, in the order of io.md 3. Its result depends on the run, and only on whether a
  simulation file was given.

## Text, bytes and termination

The same on every transport, and from the simulator too, which answers as a real instrument sends: its text followed
by `\n` ([simulation.md](simulation.md) 4).

- **`write`** sends the UTF-8 bytes of the command, followed by `\n` unless it ends with one.
- **`read`** takes the bytes up to the next `\n` (`read_until` of its transport, within at most 16 MiB *(proposed)*,
  past which it fails with `io.too_long`), and gives the text without the `\n` and one `\r` before it. Bytes that are not
  UTF-8 fail with `io.invalid_text`, and are taken: a binary answer is read with `read_bytes`, as `scpi.query_block`
  does.
- **`write_bytes` and `read_bytes`** move bytes as they are, with no termination: the data of a SCPI binary block, a
  frame of a protocol of the instrument. `read_bytes` is the `read` of its transport: all `count` bytes, or
  `io.timeout` and nothing taken.
- **A serial session** opens at 9600 baud, 8 data bits, no parity, one stop bit, no flow control, with no `settle`,
  until `set_serial` changes them.

## Errors and their codes

The errors that VISA has are given with its codes, on every transport and from the simulator, so that a program
written against the simulator reads the same errors and codes from a real instrument; the others pass on as their
library gives them. `visa.sim` gives them itself; for `net` and `serial`, `visa` makes them in LabCraft, by a `handle`
on the error of the transport and `errors.make` (spec 7.3, A.2):

| Error of the transport | Error of `visa` | Code |
|---|---|---|
| `io.timeout`, in `open` | `visa.resource_not_found`, as from the simulator (decision of the owner, 2026-11-26) | `-1073807343` (`VI_ERROR_RSRC_NFOUND`) |
| `io.timeout`, in any other call | `io.timeout` | `-1073807339` (`VI_ERROR_TMO`) |
| `net.host_not_found`, `net.connection_refused`, `serial.port_not_found` | `visa.resource_not_found` | `-1073807343` (`VI_ERROR_RSRC_NFOUND`) |
| `io.closed` | `io.closed` | `-1073807194` (`VI_ERROR_CONN_LOST`) |
| `bytes.invalid_text`, in `read` | `io.invalid_text` | 0 |
| Any other: `io.not_allowed`, `io.needs_real_time`, `io.unsupported`, `serial.port_busy`, `io.too_long`, `io.invalid_argument` | The same | Its own |

- **Only an error born in the call is made again**: one that the call receives on its `err` passes on unchanged, as a
  skipped node passes it (spec 7.2), so that `visa` never recodes the error of another library.
- **Nothing is lost**: the error of `visa` is the main one, with its own message, which names the resource at the open
  (`TCPIP0::10.0.0.5::5025::SOCKET: nothing answers at tcp:10.0.0.5:5025`), and the error of the transport is in its
  `others` (spec 7.4).
- **A call on a session that is not open**, the same on every transport and from the simulator (decisions of the owner,
  2026-11-26): on `Closed`, the default value and the session of a failed `open`, `visa.invalid_session`, code
  `-1073807346` (`VI_ERROR_INV_OBJECT`); on a session that was opened and then closed, by this run or by the other end,
  `io.closed`, code `-1073807194` (`VI_ERROR_CONN_LOST`), since a value never changes (spec 5.1) and what its transport
  knows is only that it is closed. `close` is the exception: on either, it succeeds and does nothing, as the `close` of
  each transport does.

## Sessions

- **A session keeps the timeout** given to `open`, as its transport does: a read that gets no answer waits that long,
  then gives `io.timeout`.
- **A session passes through** `write`, `read`, `query` and the others (spec 5.3), so that `close` still gets it after
  an error.

## What `visa` guarantees

The language leaves to each library what a call guarantees on its resources (spec 8.1). `visa` guarantees:

- **Each call of a transport is atomic** on its connection, port or instrument (libs/net, libs/serial, `visa.sim`
  below): `write`, `read`, `write_bytes`, `read_bytes`, `set_serial` and `close` are one call of their transport each.
- **`query` is a `write` and then a `read`**, two calls (decision of the owner, 2026-11-24): it is atomic when the
  program orders the calls on its session, as following the session wire does. Two branches that use one session with
  no order between them get the warning of spec 8.1, and a call of one may then come between the write and the read of
  the other, until functions marked `atomic` (decision of the owner, 2026-11-26): the runtime will then hold the
  session for the whole body of `query`. Holding a session across several calls the program draws comes with the access
  structure of release 0.7.0 (theme M2, Q-19).
- **One command at a time per simulated instrument**, the machine the file describes, whichever session sends it
  ([simulation.md](simulation.md) 4); for a real one, what the instrument does.
- **After an abort or a fault**, the runtime has each transport release what the run opened (spec 8.2): connections,
  ports and simulated instruments are closed. A call in progress ends, or reaches its timeout, within the grace period
  of the runtime, or is abandoned and its resource reported as leaked. Closing a session does not switch a supply off.

## Simulation: `visa.sim`

`visa.sim` is the sub-library of `visa` in [sim/](sim/), beside the simulation files shipped there; its native bodies
are in the crate `labcraft-visa`. It reads the section `visa` of the simulation file, in the format of
[simulation.md](simulation.md), and gives a handle on each instrument, as `net` gives a connection. Unlike a stream,
each `visa.sim.write` is one command, as each `visa.write` was in M0 (decision of the owner, 2026-11-26): its text, a
final `\n` or `\r\n` removed, even when it holds another `\n`; an answer is read as a real instrument sends it, its text
followed by `\n`. The section `visa` of the file belongs to the library `visa`, which its sub-library reads, and its
diagnostics keep the prefix `visa:S…` (simulation.md 7). A program calls `visa`, which calls it.

```text
fn visa.sim.open(resource: string, timeout: duration = 2 s,
                 err: error = none) -> (instrument: visa.sim.instrument, err: error)
fn visa.sim.write(instrument: visa.sim.instrument, data: bytes,
                  err: error = none) -> (instrument: visa.sim.instrument, err: error)
fn visa.sim.read(instrument: visa.sim.instrument, count: i32,
                 err: error = none) -> (data: bytes, instrument: visa.sim.instrument, err: error)
fn visa.sim.read_until(instrument: visa.sim.instrument, terminator: bytes, max: i32 = 65536,
                       err: error = none) -> (data: bytes, instrument: visa.sim.instrument, err: error)
fn visa.sim.set_serial(instrument: visa.sim.instrument, baud: u32 = 9600, data_bits: u8 = 8,
                       parity: serial.Parity = serial.Parity.None,
                       stop_bits: serial.StopBits = serial.StopBits.One,
                       flow: serial.Flow = serial.Flow.None,
                       err: error = none) -> (instrument: visa.sim.instrument, err: error)
fn visa.sim.close(instrument: visa.sim.instrument, err: error = none) -> (err: error)   // always (spec 7.2)
```

- `visa.sim.instrument` is an opaque type, a handle on one instrument of the file, and a resource; `close` is `always`.
  Each call on a handle is atomic on it, a `write` ends when the instrument has handled the command (its latency,
  simulation.md 4), and a read waits as simulation.md 4 says: so a program reads the same values and the same times
  whether it names an instrument `SIM::DMM0` or by a real name. A call on a handle that was closed gives `io.closed`,
  on a default one `visa.invalid_session`, and `close` of either succeeds, as for any session (above).
- **The report** keeps, for each simulated instrument, its commands and their answers, as in M0 (commands.md 7), under
  the `resource` of the file, one entry per instrument whichever spelling of its name opened it; a real session shows the bytes of its endpoint, as `net` and `serial` keep them.
- **The endpoints of its instruments with a real name** count for `LC8112`: an endpoint described by `visa` and by
  `net` or `serial` refuses the run (io.md 6). They are reached by `visa.open` only: `net.connect` and `serial.open`
  to one find nothing, and `serial.list` does not give them (io.md 6; decision of the owner, 2026-11-26).
