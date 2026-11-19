# `serial`: serial ports

*Status: specified for release 0.5.0 (M4, [docs/io.md](../../docs/io.md)), not built yet: its declarations and its
crate `labcraft-serial` come with its Rust. Values marked (proposed) are starting points.*

`serial` talks to a device on a serial port in raw bytes: an Arduino or an ESP32 board, a USB-serial adapter, an
instrument with a protocol of its own. Its functions are native (`extern rust`, spec 4.1), over the crate `serial2`
(`BSD-2-Clause OR Apache-2.0`, taken under Apache-2.0; tried on 2026-11-19, io.md 11). Endpoints, consent,
simulation and what a run reaches are in [docs/io.md](../../docs/io.md).

## Functions

```text
fn serial.list(err: error = none) -> (ports: [string], err: error)
fn serial.open(port: string, baud: u32 = 9600, data_bits: u8 = 8,
               parity: serial.Parity = serial.Parity.None,
               stop_bits: serial.StopBits = serial.StopBits.One,
               flow: serial.Flow = serial.Flow.None,
               timeout: duration = 2 s, settle: duration = 0 s,
               err: error = none) -> (port: serial.port, err: error)
fn serial.write(port: serial.port, data: bytes,
                err: error = none) -> (port: serial.port, err: error)
fn serial.write_line(port: serial.port, text: string, terminator: string = "\n",
                     err: error = none) -> (port: serial.port, err: error)
fn serial.read(port: serial.port, count: i32,
               err: error = none) -> (data: bytes, port: serial.port, err: error)
fn serial.read_until(port: serial.port, terminator: bytes, max: i32 = 65536,
                     err: error = none) -> (data: bytes, port: serial.port, err: error)
fn serial.read_line(port: serial.port, terminator: string = "\n", max: i32 = 65536,
                    err: error = none) -> (text: string, port: serial.port, err: error)
fn serial.read_available(port: serial.port, max: i32 = 65536,
                         err: error = none) -> (data: bytes, port: serial.port, err: error)
fn serial.close(port: serial.port, err: error = none) -> (err: error)   // always (spec 7.2)
```

```text
type serial.Parity = enum { None, Odd, Even }
type serial.StopBits = enum { One, Two }
type serial.Flow = enum { None, Software, Hardware }     // XON/XOFF; RTS/CTS
```

`serial.port` is an opaque type and a resource (`resources = ["port"]`, spec 8.1); `close` is `always`. The writes and
the reads are those of `net`, with the same rules ([libs/net](../net/README.md), "Writing" and "Reading"), on a port
instead of a connection.

- **`list`** gives the ports the system has, sorted in code point order, each once: `COM3` on Windows, a path elsewhere.
  On macOS, only the `/dev/cu.*` ports, the ones to open, not their `/dev/tty.*` twins. The list may hold ports with
  nothing behind them, virtual ones among them: the system does not say which. It gives no USB information (VID, PID),
  which `serial2` does not read (io.md 11). Listing reaches no device, and needs no consent. Under `--sim`, the serial
  endpoints of the file (io.md 6).
- **`open`** opens the port, endpoint `serial:PORT` (io.md 2), with its settings; `data_bits` is 5 to 8, `baud` above 0,
  any rate the system takes. The port keeps `timeout` for each of its reads and writes. It fails with
  `serial.port_not_found`, `serial.port_busy` (open elsewhere, or already open in this run), `io.invalid_argument` (a
  setting out of range, or one the system refuses, such as 5 data bits with two stop bits), `io.timeout`, or the
  errors of io.md 3 and 4.
- **`settle`**: many boards reset when their port is opened (an Arduino, an ESP32), and what they print as they start
  is not an answer. `open` then waits `settle`, and empties the input, before it gives the port: about 2 s for an
  Arduino Uno *(to be measured on the boards of the final trial)*. On the clock of the run, so a simulated board waits
  too.

## What `serial` guarantees

- **Each call is atomic on its port** (io.md 7), as `net` is on its connection.
- **A port is open once in a run**: a second `open` of the same port, while the first is open, is `serial.port_busy`.
  Between two programs, it is up to the system: Windows refuses a second open; Linux and macOS may let two programs
  open one port, and `serial2` asks them for no exclusive open (the trial of 2026-11-19); asking for one (`TIOCEXCL`)
  would take `unsafe` or another crate, and is not in 0.5.0.
- **After an abort or a fault**, the ports the run still holds are closed (spec 8.2).
- **A board unplugged** while open gives `io.closed` to the next call on its port.

## Simulation

The section `serial` of a simulation file holds an array `device`, each one an endpoint `serial:…` and its exchanges,
with the settings it expects if any, in the format of [io.md 6](../../docs/io.md). Its codes:

| Code | Severity | When |
|---|---|---|
| `serial:S001` | error | A key that the format does not have; fix: the nearest key |
| `serial:S002` | error | A value of the wrong TOML type |
| `serial:S003` | error | A device without `endpoint`, or one that is not `serial:PORT` (io.md 2) |
| `serial:S004` | error | An endpoint defined again, at the second one (`serial:com3` is `serial:COM3`) |
| `serial:S005` | error | A time below 0 or above one day; a setting out of the range of `open` (`parity = "mark"`) |
| `serial:S006` | error | A request or a response given in both forms or, for a request, in neither; an empty request; hexadecimal that is not pairs of digits; more than 65,536 bytes *(proposed)* |
| `serial:S007` | error | A request that equals another request of the same device, or is the start of one (io.md 6) |
| `serial:S008` | error | More than 256 devices, or 10,000 exchanges in one device *(proposed)* |

`parity` is `"none"`, `"odd"` or `"even"`, `stop_bits` 1 or 2, `flow` `"none"`, `"software"` or `"hardware"`.
