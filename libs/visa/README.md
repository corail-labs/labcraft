# `visa`: sessions on instruments

*Status: implemented for milestone M0b (tag `v0.2.0-m0b`) by the crate `labcraft-visa`, in `src/`, in deterministic
mode and in real time. The Python proof of concept followed it too, until its retirement after M0 (2026-11-15). Real
instruments over TCP and serial, `read_bytes`, `write_bytes` and `set_serial`: specified for release 0.5.0 (M4,
[docs/io.md](../../docs/io.md)), not built yet.*

`visa` is the transport library shipped with LabCraft: it opens sessions on instruments, writes commands
and reads answers. Its functions are declared in this folder (`extern rust`, spec 4.1), and their native
bodies are in its Rust crate, `labcraft-visa`, since M0b. Instrument drivers and `scpi` are libraries
over it. Not every instrument goes through VISA: another transport is another library, with its own
rules. From 0.5.0 its crate reaches TCP and serial through the same Rust code as `net` and `serial`, so each transport is
written once; whether `visa` itself is written again in LabCraft over them is weighed after M2 (io.md 11).

| Function | Does |
|---|---|
| `open(resource, timeout = 2 s)` | Opens a session on a resource (`SIM::DMM0`, a VISA resource name). `visa.resource_not_found` if there is none |
| `write(session, command)` | Sends a command |
| `read(session)` | Reads an answer; `io.timeout` if none comes within the timeout of the session |
| `query(session, command)` | Writes, then reads the answer |
| `write_bytes(session, data)` | Sends bytes as they are, with no termination (0.5.0) |
| `read_bytes(session, count)` | Reads exactly `count` bytes; `io.timeout` if they do not all come within the timeout, and nothing is taken (0.5.0) |
| `set_serial(session, baud = 9600, …)` | Sets the serial settings of an `ASRL` session, as `serial.open` takes them; `io.unsupported` on another session (0.5.0) |
| `close(session)` | Closes the session; `always` (spec 7.2): it runs even after an error |

## Resources and transports

*Specified for 0.5.0 ([docs/io.md](../../docs/io.md)).* A resource name says how a session reaches its instrument:

| Name | Reaches |
|---|---|
| `SIM::DMM0` | The instrument of that name in the simulation file (simulation.md) |
| `TCPIP0::10.0.0.5::5025::SOCKET` | A raw TCP socket, the endpoint `tcp:10.0.0.5:5025` (io.md 2), as instruments open one for SCPI |
| `ASRL3::INSTR`, `ASRL/dev/ttyUSB0::INSTR` | A serial port, the endpoint `serial:COM3` on Windows or `serial:/dev/ttyUSB0`, at 9600 baud, 8 data bits, no parity, one stop bit, no flow control until `set_serial` |

A real name is read without case, as VISA reads it, apart from a path inside it (`ASRL/dev/ttyUSB0::INSTR`), and
stands for its endpoint (io.md 2); a `SIM::` name is compared exactly,
as in M0. With `--sim`, every name, real ones included, is an instrument of the
file or is not found; without it, a `SIM::` name is not found and a real one reaches the hardware, in real time, with
the consent of io.md 4. The other forms of VISA (VXI-11 `::INSTR`, HiSLIP, USB, GPIB) give `io.unsupported` until
their libraries come (io.md 11). On a real session:

- **Text and its termination.** `write` sends the command followed by `\n`, unless it ends with one; `read` reads up to
  the next `\n` and gives the text without it and without a `\r` before it, as the simulator gives its answers, so that
  a driver reads the same text from both. A response that is not UTF-8 is `io.invalid_text`: a binary one is read with
  `read_bytes`, as `scpi.query_block` does.
- **`read_bytes` and `write_bytes`** move bytes as they are, with no termination: the data of a SCPI binary block, a
  frame of a protocol of the instrument.
- **The codes** of its errors are those of VISA (simulation.md 6), whatever the system: `VI_ERROR_TMO` for
  `io.timeout`, `VI_ERROR_RSRC_NFOUND` for `visa.resource_not_found` (a host that does not answer, a port that does not
  exist), `VI_ERROR_CONN_LOST` (`-1073807194`) for `io.closed`.

## Sessions

- **A session is a resource** (the manifest says `resources = ["session"]`, spec 8.1): two nodes that
  receive one with no order between them get the warning of spec 8.1, and a struct, a variant or an
  array that holds one is a resource too.
- **A session keeps the timeout** given to `open`: a read that gets no answer waits that long, then gives
  `io.timeout`.
- **A session passes through** `write`, `read` and `query` (spec 5.3), so that `close` still gets it after
  an error.

## What `visa` guarantees

The language leaves to each library what a call guarantees on its resources (spec 8.1). `visa` guarantees:

- **Each call is atomic on its session.** `query` writes then reads without another call on that session
  stepping in, so a race never corrupts a message. A call that finds its session in use waits until it
  is free; in deterministic mode, the calls that wait for one session take it in the global order of
  spec 8. The order between two calls that the program does not order stays undetermined, as the warning
  says.
- **One command at a time per instrument**, whichever session sends it: a command waits until the one
  before it on that instrument has ended. For a simulated instrument, [simulation.md](simulation.md),
  section 4; for a real one, what the instrument and its VISA driver do.
- **After an abort or a fault**, the runtime has `visa` release what the run opened: the sessions still
  open are closed. A call still waiting for its session has not started, and ends at once; a call in
  progress ends, or reaches the timeout of its session, within the grace period of the runtime, or is
  abandoned and its session reported as leaked (spec 8.2). Closing a session does not switch a supply off.

## Simulation

A `SIM::` resource answers as the instruments of a simulation file: [simulation.md](simulation.md)
gives the format, how an instrument answers, and the files shipped in [sim/](sim/). From 0.5.0 a file may describe an
instrument by its real name too, so that the program of a bench runs on the simulator without changing a line
(io.md 3).
