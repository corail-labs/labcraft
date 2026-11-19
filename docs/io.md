# Real I/O (M4)

*Specification of the theme M4 for release 0.5.0, decided with the owner on 2026-11-19. State: specified, not built.
A value marked (proposed) is a starting point, settled when the Rust is written. The goal and the place of the theme are
in [outline.md](outline.md) (M4) and [roadmap.md](roadmap.md) ("Releases", 0.5.0).*

A program touches the world through resources: a session on an instrument, a TCP connection, a serial port, a file.
Until M4 the only resources were the simulated instruments of `visa`. This document says how a program reaches real
ones, how the same program runs on the simulator without changing a line, and how the human keeps an agent off the
hardware until they say yes. The libraries that do it have their own pages: [libs/net](../libs/net/README.md),
[libs/serial](../libs/serial/README.md), [libs/file](../libs/file/README.md), [libs/console](../libs/console/README.md),
[libs/visa](../libs/visa/README.md);
the new primitives (`bytes`, the bitwise operations) are in the spec, appendix A.

## 1. Principles

- **The same program runs on the simulator and on the hardware.** What changes is how it is run (`--sim FILE` or
  not, section 3), never a line of the program: the program of the bench opens `TCPIP0::10.0.0.5::5025::SOCKET` and
  `serial:COM3` in both cases.
- **Real hardware is a resource, not a target.** A target is where the program runs or what it becomes (M6 and after);
  a resource is what it touches, whatever the target (decision of the owner, 2026-11-18).
- **An agent touches real hardware only if the human has allowed it** (section 4). No tool grants it, and without the
  right a call gets an error, never a crash.
- **The transports are in pure Rust**, behind `NativeLibrary` (architecture.md 10.6): TCP and UDP by the standard
  library, serial by `serial2` (section 11, "Risks"). A host can give its own library of the same name, as the
  browser will.
- **A protocol is written in LabCraft** on top of a transport, as `scpi` is on top of `visa`: the bitwise operations,
  the integer division and `bytes` (spec, appendix A) are enough for Modbus TCP, which the example of section 10 proves.

## 2. Endpoints

An **endpoint** is what a real resource reaches, named the same way whichever library reaches it, so that one
consent covers it (decision of the owner, 2026-11-19):

| Endpoint | Reached by |
|---|---|
| `tcp:HOST:PORT` | `net.connect(HOST, PORT)`; `visa.open("TCPIP0::HOST::PORT::SOCKET")` |
| `udp:HOST:PORT` | `net.open_udp(HOST, PORT)` |
| `serial:PORT` | `serial.open(PORT)`; `visa.open("ASRL3::INSTR")` (`serial:COM3`), `visa.open("ASRL/dev/ttyUSB0::INSTR")` |

- **`HOST`** is the text the program gives, lower case for its ASCII letters: a name (`dmm.local`) or an address
  (`10.0.0.5`, `fe80::1`); an IPv6 address is written in brackets in the endpoint, whether the program gave them or
  not (`tcp:[fe80::1]:502`). A name is never resolved to judge consent: `tcp:localhost:502` and `tcp:127.0.0.1:502`
  are two endpoints. **`PORT`** is a number from 1 to 65,535, written in decimal without zeros in front, whatever the
  text it came from (`TCPIP0::h::0502::SOCKET` is `tcp:h:502`).
- **`PORT` of a serial endpoint** is a name whose form decides how it compares, never the system that runs it, so that
  one program and one simulation file mean the same on Windows, Linux, macOS and in the browser: `COM<n>` (`n` from
  1) is compared without case and written upper case (`com3` is `serial:COM3`); any other name, a path such as
  `/dev/ttyUSB0` or `/dev/cu.usbserial-1410`, is compared exactly. Which device a name opens is the system's: a
  `COM` name on Linux names no port, and the open gives `serial.port_not_found`.
- **A VISA name reduces to its endpoint**, its VISA words and its host without case, as VISA reads its names, a path
  inside it exactly: `TCPIP[n]::HOST::PORT::SOCKET` to `tcp:HOST:PORT`; `ASRL<n>::INSTR` (`n` from 1) to
  `serial:COM<n>` on every system, whose real open reaches `COM<n>` on Windows and `/dev/ttyS<n−1>` elsewhere, the
  usual numbering of VISA; `ASRL<path>::INSTR`, a path that starts with `/`, to `serial:<path>`. `ASRL0` and any
  other text that is no VISA name give `visa.resource_not_found`. The other VISA forms (`::INSTR` over VXI-11,
  `::hislip0::INSTR`, `USB…`, `GPIB…`) come later in the theme (section 11) and give `io.unsupported` today.
- A file is not an endpoint: section 5 says what a run may write.

## 3. What a run reaches

| How the run is started | `SIM::…` | A real name (`tcp:`, `serial:`, a VISA name) |
|---|---|---|
| With `--sim FILE` | The device of the file, or `resource_not_found` | The device of the file, or `resource_not_found`; never the hardware |
| Without `--sim`, real time | `resource_not_found` | The hardware, with the consent of section 4 |
| Without `--sim`, deterministic | `resource_not_found` | `io.needs_real_time` |

- **With `--sim`, the file is the whole bench** (decision of the owner, 2026-11-19): a simulation file may describe a
  device by its real name (`resource = "TCPIP0::10.0.0.5::5025::SOCKET"`, `endpoint = "serial:COM3"`), and a real name
  that it does not describe is not found, as a `SIM::` name is: a typo in the file never sends a command to a real
  supply. A bench where some devices are simulated and others real is not in 0.5.0. `resource_not_found` is the category
  of the library (`visa.resource_not_found`, `net.host_not_found`, `serial.port_not_found`), after the timeout of the
  open, as for `visa` today.
- **Real hardware needs real time** (decision of the owner, 2026-11-19): in deterministic mode the clock of the run is
  simulated, and a real device does not wait for it. The open of a real resource in deterministic mode gives
  `io.needs_real_time`, at once, whose message says to run with `--real-time` (`mode: "real_time"` in `run.start`, the
  entry "Real hardware" in the IDE). The default of `run` does not change.
- **The simulation of a raw device** (`net`, `serial`) is a list of exchanges (section 6), on the clock of the run, in
  both modes, as the instruments of `visa` are.
- **A host without the transport** (the browser, a host that does not give it) gives `io.unsupported` at the open,
  with or without `--sim`, unless the host gives its own library.
- **The order of the checks at an open**, the same in every library (decision of the owner, 2026-11-19): the name, a text that is no
  name giving `resource_not_found`; then `io.unsupported`; then the rows of the table above, so that with `--sim` the
  file decides, and without it `io.needs_real_time` comes in deterministic mode before any right is judged; then the
  right of section 4, `io.not_allowed` or the question; then the open of the hardware, which may give
  `resource_not_found` after its timeout. No question is asked whose answer could not be used: an agent that opens a
  real name in deterministic mode gets `io.needs_real_time`, and no window.

## 4. Consent

**Who has the right** to reach a real endpoint, in a run:

| The run is started by | Real endpoints | Files outside the project (section 5) |
|---|---|---|
| The human: `labcraft-cli run`, `labcraft-cli exec`, Run in the IDE | All | All |
| An agent on `labcraft-cli mcp`, headless | Those given by `--allow-hardware ENDPOINT` when the server starts | Those under an `--allow-path DIR` given when it starts |
| An agent attached to the IDE | Those the human allows in the window, endpoint by endpoint | Those the human allows, folder by folder |

- **`--allow-hardware ENDPOINT`**, repeated, takes an endpoint (`tcp:10.0.0.5:502`) or a VISA name, which it reduces
  (section 2). Names are exact: no wildcard in 0.5.0 (decision of the owner, 2026-11-19). A name that is neither is
  wrong usage (exit 2). **`--allow-path DIR`**, repeated, allows the folder and what is under it, after `..` and links
  are resolved; a folder that does not exist is wrong usage. Both are given by the human who starts the server, in the
  configuration of the MCP client: no tool, no command and no setting changes them (mcp.md 6).
- **In the IDE**, a run started by an agent that opens a real endpoint, or a file outside the project, waits, and the
  window asks: the client, the project, the endpoint (`tcp:10.0.0.5:5025`, with the name the program gave,
  `TCPIP0::10.0.0.5::5025::SOCKET`) or the folder, and three answers (decision of the owner, 2026-11-19): **For this
  run**, **Until the window closes**, **Refuse**. With no answer in 60 s *(proposed)*, it is refused. An answer is never
  kept in the settings of the user or of the project. Disconnecting the client, or changing its policy, forgets its
  answers. The policies Ask, Edits and All (ide.md 8) do not reach the hardware: under All, the question is still asked.
  **While a question waits, the run stops**: in deterministic mode no node runs and its clock does not move, so that
  the result does not depend on when the human answers (a file outside the project can be asked about in that mode);
  in real time, the other branches go on, as for any call that waits.
- **Without the right**, the open fails with `io.not_allowed`, code 0, whose message names the endpoint and how it is
  allowed (`--allow-hardware tcp:10.0.0.5:5025`, or the question of the window). It is an error on a wire, which the
  program handles or reports (spec 7.5), not a crash and not a refused run.
- **The right is judged at the open**, once per endpoint and per run: the calls on an open connection, port or session
  do not ask again. A run that is aborted while a question waits ends as any abort (spec 8.2), and the question goes away.
- **The effect `hardware`** (commands.md 1), formerly `instruments`, marks the commands that can reach real hardware or
  files outside the project: `run.start`. It says what a command can do; the question above is what guards it. Since
  what a run writes with `file` is no step of the history, `run.start` is also `destructiveHint` for MCP (mcp.md 2).
- **What consent is not.** It bounds what an agent does through LabCraft. An agent that has a shell of its own can run
  `labcraft-cli run` as the human does: that is the permission its client gave it, outside LabCraft, and the
  configuration of the client is where the human refuses it.

## 5. Files and the console

### Files

`file` (libs/file) reads and writes real files in every mode, under `--sim` too: a simulation file describes devices,
not a file system (decision of the owner, 2026-11-19).

- **A relative path** is resolved against the project: the folder given to `run`, the root of the MCP server, the
  project of the window. `/` separates folders on every system, and `\` too on Windows.
- **Inside the project, a run of an agent needs no consent**, except under a hidden name (decision of the owner,
  2026-11-19); outside, and under a hidden name, it needs the consent of section 4 (`--allow-path`, the question of the
  window). A path is inside when, `..` and links resolved for the part of it that exists, it is under the project: a
  link that leads out is out. A hidden name is a folder or a file of the path, below the project, whose name starts
  with `.` (`.git/hooks/pre-commit`, `.labcraft/config.toml`), as the commands refuse such a path to an agent
  (commands.md 2): a run of an agent must not plant a hook that runs later, nor change the settings of the project.
  Reading is judged as writing: both reach the same files.
- **A file is a resource without a handle** (spec 8.1): each call of `file` is atomic on its file within a run (two
  appends from two branches do not interleave), and a write replaces the file at once, so that a reader never sees
  half of it. Two calls that the program does not order, on one file, are its concern; `check` cannot see that two
  paths name one file, and gives no warning for them.

### The console

`console` (libs/console) gives a program its text output, the stdout of a program as a stream that every host can
show (decision of the owner, 2026-11-19):

```text
fn console.write_line(text: string) -> ()
```

- **One contract, one place per host**: what a program writes is the same everywhere, and each host shows its lines
  where its user looks, through its own `NativeLibrary` (section 1):

  | Host | Where the lines go |
  |---|---|
  | The IDE | The tab `Console` of the panel `Run`, as they come (ide.md 4) |
  | `labcraft-cli run` | stderr, as they come, since stdout carries only the report (cli.md 6.1), with `--json` too |
  | `labcraft-cli mcp` | Only the report, since stdout carries the protocol; `run.status` gives the lines so far |
  | A standalone executable (M6) | The stdout of its process |
  | The browser | The console of the developer tools of the page, and the tab `Console` once the IDE runs there |

  Wherever they go, the report of the run keeps them too (`console`, commands.md 7), so that an agent reads them and a
  run is compared with another.
- **It never fails and never asks.** It reaches nothing outside LabCraft, so it needs no consent, and works under
  `--sim`, in both modes and in the browser. It has no `err`, so a line written after an error is still written, which
  is what one wants to see while finding a fault. A host that cannot take a line (a closed stdout) drops it.
- **A line** is the text given, and a `\n` in it starts another line where it is shown. Two calls that the program does
  not order come in the global order of spec 8 in deterministic mode, in any order in real time: the console is a
  resource without a handle, as a file is, and gets no warning.
- **Reading a line typed by the human** (stdin, a field of the tab) is not in 0.5.0: the front panel of M3 is how the
  operator gives values.

## 6. Simulated raw devices

`net` and `serial` simulate a device by its exchanges, in their section of the simulation file (decision of the owner,
2026-11-19), as `visa` simulates an instrument by its commands:

```toml
# An Arduino board on COM3 that answers two commands.
[[serial.device]]
endpoint = "serial:COM3"
latency_ms = 2

[[serial.device.reply]]
request = "LED ON\n"
response = "OK\r\n"

[[serial.device.reply]]
request = "TEMP?\n"
response = "23.5\r\n"

# A Modbus TCP server that answers one request: read two holding registers from 0.
[[net.device]]
endpoint = "tcp:10.0.0.7:502"

[[net.device.reply]]
request_hex = "0001 0000 0006 01 03 0000 0002"
response_hex = "0001 0000 0007 01 03 04 00e6 0019"
```

| Key of a device | Value | Default |
|---|---|---|
| `endpoint` | `tcp:…` or `udp:…` for `net`, `serial:…` for `serial` (section 2); unique in its section | required |
| `latency_ms` | The time between the end of a request and its response, on the clock of the run | 0 |
| `reply` | An array of exchanges: a `request` and its `response` | none |
| `unknown` | The response to a request that matches no exchange, as text, or `unknown_hex` | none: no response |
| `faults` | `hang_ms`: from that time on, the device takes no request and gives no response. `refused = true` (`net`, TCP): every connect gives `net.connection_refused`. `close_ms` (`net`, TCP): the device closes the connection at that time | none |
| `baud`, `data_bits`, `parity`, `stop_bits`, `flow` | (`serial`) The settings the device expects; an open with other settings gets no response from it, and the report of the run says so | any |

- **A request and a response** are text (`request`, `response`: the UTF-8 bytes of a TOML string, its escapes read,
  `\n` included) or bytes (`request_hex`, `response_hex`: pairs of hexadecimal digits, spaces between pairs allowed),
  one form each, of at most 65,536 bytes *(proposed)*. A request is not empty; a response may be left out: the request
  is taken and nothing answers.
- **How a device answers.** It keeps the bytes it has received and not yet matched, in order. After each write, as
  long as they start with the request of an exchange, it takes that request from the front, and its response is ready
  to be read `latency_ms` after the write; so `write("LED ON\nTEMP?\n")` gets both answers, in order. When what is left
  is the start of some request, it waits for more. Otherwise what is left matches nothing: the device forgets it and
  answers `unknown`, if it has one. A request that equals another one, or is the start of another one, is refused,
  since the other could never match. Over UDP, each datagram is a whole request, and a datagram that equals no request
  matches nothing.
- **Reading** takes from what the device has answered, in order. A read that needs more than is there waits, up to the
  timeout of its connection or port, and then fails with `io.timeout`, taking nothing (libs/net, "Reading").
- **The report of a run** lists, for each simulated or real endpoint that a run reached, what was written and read, in
  `instruments` (commands.md 7): one entry per write (`sent`) and one per read (`answer`), in order, since a stream has
  no pairs of a command and its answer; text when it is UTF-8 without control characters other than `\r`, `\n` and
  `\t`, hexadecimal with `"hex": true` otherwise *(proposed)*, within the bounds of `instruments`.
- **What is refused** is said by the library, with its codes (libs/net, libs/serial, "Simulation"), and the run is
  `refused`, as for `visa` (simulation.md 7).
- `serial.list` under `--sim` gives the serial endpoints of the file: those of `serial` and the `ASRL` instruments of
  `visa`, by their port name.

## 7. Running real calls

- **A real call never stops the other branches.** A blocking read, write or open runs apart from the run, as the
  `Blocking` work of a native call (architecture.md 10.6), while the call waits; the other branches go on.
- **Each call is atomic on its connection, port or session**, as `visa` makes its calls atomic on its session
  (libs/visa, "What `visa` guarantees"): a read that waits for its timeout holds its connection, so a write from
  another branch waits for it. Reading and writing one connection from two branches at once comes with the events of M2
  *(proposed)*.
- **After an abort or a fault**, the runtime has each library release what the run opened (spec 8.2): connections
  and ports are closed, sessions too. A call in progress ends, or reaches its timeout, within the grace period of the
  runtime, or is abandoned and its resource reported as leaked. Closing a port does not switch off what is behind it.
- **The code of an error** is 0 for `io.not_allowed`, `io.needs_real_time` and `io.unsupported`, which no transport
  gives, in every library. Otherwise, it is that of VISA for `visa`, simulated or real (simulation.md 6), so that a
  program written against the simulator reads the same codes. For `net`, `serial` and `file`, it is that of the system for a real error
  (`errno`, the Windows error), which differs from one system to another, and 0 for a simulated one: a program tells
  these errors apart by their category (spec 7.3), which does not depend on the system.

- **Tests.** The CI runs the transports against simulated devices (section 6), and `net` against a TCP and a UDP
  server on `127.0.0.1` that its tests start, written in Rust. A real serial port is not in the CI: `serial` is tried
  by hand on a virtual pair of ports (com0com on Windows, `socat` on Linux) and on the boards of the final trial, as the
  trial of `serial2` was.
- **How the runtime gives this to a library** (architecture.md 10.6, `runtime::io`): a library learns, besides
  its section of the simulation file, whether a file was given at all (with `--sim visa-only.toml`, `net` must not go to
  the hardware), the mode of the run, the project, who started it, and the rights of section 4; and it can ask the host
  the question of section 4, and wait for its answer.

## 8. Categories

Every library of M4 declares its categories in its manifest (spec 7.1). Those shared by the transports are under `io`:

| Category | When |
|---|---|
| `io.timeout` | Nothing came, or could be sent, within the timeout (exists since M0) |
| `io.not_allowed` | The run has no right to this endpoint or this path (section 4) |
| `io.needs_real_time` | A real resource opened in deterministic mode (section 3) |
| `io.unsupported` | The host has no such transport, or the name is a form that comes later (section 2) |
| `io.closed` | The other end closed the connection, or the port went away (a board unplugged) |
| `io.invalid_text` | Bytes read as text that are not UTF-8 |
| `io.too_long` | `read_until` or `read_line` reached its `max` without its terminator |
| `io.invalid_argument` | A count or a `max` below 0, a baud rate of 0, a port number of 0 |

Each library adds its own: `net.host_not_found`, `net.connection_refused`, `net.datagram_too_long`;
`serial.port_not_found`, `serial.port_busy`; `file.not_found`, `file.exists`, `file.denied`, `file.not_empty`; and
`visa.resource_not_found`, `visa.invalid_session` (since M0); `scpi.unexpected_response` (spec, appendix A.3), beside
`scpi.instrument_error`. `bytes.out_of_range` and `bytes.invalid_text` are those of the primitive `bytes` (spec,
appendix A.5).

## 9. In the IDE

- **The `Run` panel** chooses how the function shown runs (ide.md 4): a simulation file of the project or of
  `<labcraft>/`, **No simulation**, or **Real hardware**, which runs without `--sim`, in real time. The entry
  `Run > Real hardware` of the menu (ide.md 6) chooses it; it is no longer planned.
- **A run started by the user** reaches what it opens, with no question. A run started by an attached agent asks
  (section 4), in the window, over whatever the user is doing, as the first connection of a client asks (ide.md 8).

## 10. The example that validates M4: Modbus TCP

`examples/modbus_tcp` reads holding registers from a Modbus TCP server, in LabCraft only, on top of `net` and `bytes`:
it builds the MBAP header (transaction, protocol 0, length, unit) and the PDU (function code 3, first register, count),
in big endian, writes them with `net.write`, reads the 7 bytes of the header of the response with `net.read`, then the
rest by the length it gives, and turns a response whose function code has its high bit set (`core.and(code, 0x80)`)
into an error with the exception code. It proves that the primitives are enough, serves as the model of a protocol for
contributors, and avoids `tokio-modbus`, which relies on tokio. It is tested by `check` and against a simulated server
(section 6) in the CI, and by hand against a Modbus test server written in Rust for the tests, and against the hardware
of the final trial.

## 11. What 0.5.0 leaves for later

In the theme, as separate libraries, one issue each, according to demand and the hardware available: VXI-11, HiSLIP,
USBTMC, the binding to an installed VISA (its C API, loaded at run time: `unsafe`, which would be an exception that the
repository names; how `unsafe` is allowed is not decided, craft-ecosystem.md), GPIB through an adapter (D-17);
Modbus RTU (CRC-16 by the bitwise operations; frames cut by the length that the function code announces, not by the
silence of 3.5 characters, which is hard to hold from a diagram), MQTT, CAN, DAQmx, `i2c`/`spi`, an Arduino board as a
slave; a raw `usb` library, on which USBTMC can be written in LabCraft, as Modbus TCP on `net`. Also later: the lines
of a serial port (DTR, RTS, break), a listening TCP server, UDP broadcast, the discovery of devices (USB VID and PID,
which `serial2` does not give), a bench where some devices are simulated and others real, wildcards in
`--allow-hardware`, patterns in the requests of a simulated device, SRQ as events (M2), a dedicated interface for the
simulation files (the `Text` view is enough), and reading a file into a declared type (`csv`, `json`, `config` of
Q-15), which needs the generics of M2.

**`visa` in LabCraft, to weigh after M2** (owner, 2026-11-19). In 0.5.0 `visa` stays native, and its crate reaches TCP
and serial through the same Rust code as `net` and `serial`. Written in LabCraft, it would be a dispatcher over those
transports (and `usb` later), the simulator of `SIM::` staying a native library apart. What stops it today: the atomicity
of `visa.query` (spec 8.1), which a function in LabCraft cannot hold before the access structure of `ref<T>` (M2,
Q-19); the report, whose `instruments` lists the commands of each instrument, where such a `visa` would show only the
bytes of its endpoints; and the codes of VISA, which each call would have to make again.

**Risks.**

- **The licence of the serial transport**: `serialport` 4.10 is MPL-2.0, which `cargo xtask licences` accepts (it
  refuses only GPL, LGPL and AGPL), and needs `libudev` on Linux; `serial2` 0.2.39 is `BSD-2-Clause OR Apache-2.0`, a
  permissive licence as the rest of the repository, taken under Apache-2.0, maintained, with `libc`, `cfg-if` and
  `windows-sys` as its only dependencies. Owner's decision (2026-11-18): `serial2`. Its trial (2026-11-19, Windows, a
  virtual port; `cargo check` for Linux and macOS) confirmed it: a read with a timeout gives `TimedOut` after it, a port
  that does not exist `NotFound`, a second open `PermissionDenied`. It does not build for `wasm32`, so `labcraft-serial`
  uses it only outside the browser. What it found is written where it applies: the list of ports (libs/serial, `list`),
  the reset of a board at the open (`settle`), the exclusive open. To try again on Linux and on the boards of the final
  trial.
- **The runtime**: blocking real-time I/O touches the scheduler, where a pull request of M1 on `engine` may cross those
  of M4.
- **USBTMC on Windows** (an instrument installed with the driver of its maker cannot be seen through WinUSB): not in
  release 0.5.0, it stays the risk of its library.
