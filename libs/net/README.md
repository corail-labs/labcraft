# `net`: TCP and UDP

*Status: built for release 0.5.0 (M4, [docs/io.md](../../docs/io.md)): its declarations are in this folder, and its native
bodies in the crate `labcraft-net` (`src/`). Values marked (proposed) are starting points.*

`net` talks to a device over the network in raw bytes, for the devices that speak a protocol of their own (Modbus TCP,
a board on Ethernet, a line protocol) rather than through `visa`. Its functions are native (`extern rust`, spec 4.1),
over the standard library of Rust. Endpoints, consent, simulation and what a run reaches are in
[docs/io.md](../../docs/io.md); this page gives the functions and what they guarantee.

## Functions

```text
fn net.connect(host: string, port: u16, timeout: duration = 2 s,
               err: error = none) -> (connection: net.connection, err: error)
fn net.write(connection: net.connection, data: bytes,
             err: error = none) -> (connection: net.connection, err: error)
fn net.write_line(connection: net.connection, text: string, terminator: string = "\n",
                  err: error = none) -> (connection: net.connection, err: error)
fn net.read(connection: net.connection, count: i32,
            err: error = none) -> (data: bytes, connection: net.connection, err: error)
fn net.read_until(connection: net.connection, terminator: bytes, max: i32 = 65536,
                  err: error = none) -> (data: bytes, connection: net.connection, err: error)
fn net.read_line(connection: net.connection, terminator: string = "\n", max: i32 = 65536,
                 err: error = none) -> (text: string, connection: net.connection, err: error)
fn net.read_available(connection: net.connection, max: i32 = 65536,
                      err: error = none) -> (data: bytes, connection: net.connection, err: error)
fn net.close(connection: net.connection, err: error = none) -> (err: error)   // always (spec 7.2)

fn net.open_udp(host: string, port: u16, local_port: u16 = 0, timeout: duration = 2 s,
                err: error = none) -> (socket: net.udp_socket, err: error)
fn net.send(socket: net.udp_socket, data: bytes,
            err: error = none) -> (socket: net.udp_socket, err: error)
fn net.receive(socket: net.udp_socket, max: i32 = 65507,
               err: error = none) -> (data: bytes, socket: net.udp_socket, err: error)
fn net.close_udp(socket: net.udp_socket, err: error = none) -> (err: error)   // always
```

`net.connection` and `net.udp_socket` are opaque types, and resources (the manifest says
`resources = ["connection", "udp_socket"]`, spec 8.1); `close` and `close_udp` are `always`. Two types rather than
one, so that a wire from a UDP socket into `read_line` is refused when it is drawn, not when it runs.

- **`connect`** opens a TCP connection to the endpoint `tcp:HOST:PORT` (io.md 2), within `timeout`, which the connection
  then keeps for each of its reads and writes. It fails with `net.host_not_found` (a name that does not resolve),
  `net.connection_refused`, `io.timeout`, or the errors of io.md 3 and 4 (`io.not_allowed`, `io.needs_real_time`,
  `io.unsupported`).
- **`open_udp`** opens a UDP socket bound to `local_port` (0: one the system chooses) and connected to the endpoint
  `udp:HOST:PORT`: `send` sends one datagram to it, and `receive` takes one datagram from it, those of other senders
  being dropped, as a connected socket does. Broadcast and a socket that listens to anyone come later (io.md 11).
- **A host** (decision of the owner, 2026-11-19) is an IP address, IPv4, or IPv6 with or without brackets (io.md 2), or
  a DNS name of at most 253 bytes made of ASCII letters, digits, `-`, `.` and `_`. Any other text (empty, with a space or
  a control character, a stray `:` or bracket, too long) is `net.host_not_found` at once, code 0, before the simulation
  file is consulted and before any question of consent: the name is judged first (io.md 3). A port of 0 is
  `io.invalid_argument`.
- **A real error that this page does not name** (decision of the owner, 2026-11-19): at `connect` and `open_udp`, a host
  or a network that cannot be reached is `net.host_not_found`, and any other error of the open is
  `net.connection_refused`; a connection reset or aborted during a call is `io.closed`.
- **A closed connection or socket** (decision of the owner, 2026-11-19): a call on one that is closed, or on one never
  opened (the default value of `net.connection` or `net.udp_socket`), fails with `io.closed`, whose message says which.
  `close` and `close_udp` on one already closed, or never opened, succeed with no error, as `always` functions.

## Writing

- **`write`** sends all its bytes, or fails: `io.timeout` when the system cannot take them within the timeout,
  `io.closed` when the other end has closed.
- **`write_line`** writes the UTF-8 bytes of `text`, then those of `terminator`.
- **`send`** sends one datagram; one larger than 65,507 bytes is `net.datagram_too_long`, and nothing is sent.

## Reading

The bytes that arrive wait in the connection, in order, until a read takes them.

- **`read`** takes exactly `count` bytes. If they are not all there it waits, up to the timeout of the connection
  counted from the start of the call; then it fails with `io.timeout` and **takes nothing**: the bytes stay for the
  next read. `count` 0 gives empty bytes at once.
- **`read_until`** takes the bytes up to `terminator`, which it includes. Without the terminator in `max` bytes it fails
  with `io.too_long` and takes those `max` bytes, so that the next read starts after them; within the timeout, as
  `read`. An empty terminator is `io.invalid_argument`.
- **`read_line`** is `read_until` on the UTF-8 bytes of `terminator`, then removes the terminator and, when it is
  `"\n"`, one `"\r"` before it, so that `"OK\r\n"` reads `OK`. Bytes that are not UTF-8 fail with `io.invalid_text`,
  and are taken.
- **`read_available`** takes what has arrived, up to `max` bytes, at once, empty bytes included: to empty the input
  before a command, or to poll.
- **`receive`** takes one datagram, waiting for one up to the timeout of the socket. One longer than `max` fails with
  `net.datagram_too_long`, and is dropped.
- When the other end has closed, what has arrived is still read; a read that would need more fails with `io.closed`.
  So does `read_available` when nothing waits (decision of the owner, 2026-11-19); while bytes wait, it gives them.
- A `count` or a `max` below 0 is `io.invalid_argument`; more than 16 MiB *(proposed)* too, since a read holds what it
  takes in the memory of the run.

## What `net` guarantees

- **Each call is atomic on its connection or socket** (io.md 7): a call that finds it in use waits until it is free, in
  the global order of spec 8 in deterministic mode. A read that waits for its timeout holds the connection.
- **After an abort or a fault**, the connections and sockets the run still holds are closed (spec 8.2). A call in
  progress is not cut short: it ends, or reaches its timeout, within the grace period of the runtime, or is abandoned and
  its connection reported as leaked (io.md 7).
- **A real connection and a simulated one read the same**: the same bytes and the same categories of error. The code
  of an error is that of the system for a real one, and 0 for a simulated one (io.md 7), and for `io.not_allowed`,
  `io.needs_real_time` and `io.unsupported`.
- **The report of a run** (decision of the owner, 2026-11-19) lists in `instruments` each endpoint reached, simulated or
  real, with one entry per write (its `sent`, and an `answer` of `null`) and one per read (its `answer`, and a `sent` of
  `null`), in order (commands.md 7, io.md 6). A read that took nothing gives no entry: empty bytes, a `count` of 0, an
  empty `read_available`, or one that failed without taking. A read that failed and took bytes gives its entry
  (`io.too_long`, `io.invalid_text`, and a datagram dropped by `net.datagram_too_long`). A write that went gives its
  entry, even when a hung device ignores its bytes.

## Simulation

The section `net` of a simulation file holds an array `device`, each one an endpoint `tcp:…` or `udp:…` and its
exchanges, in the format of [io.md 6](../../docs/io.md). `net` judges its section before the run, and the run is
`refused` if anything is wrong:

| Code | Severity | When |
|---|---|---|
| `net:S001` | error | A key that the format does not have; fix: the nearest key |
| `net:S002` | error | A value of the wrong TOML type |
| `net:S003` | error | A device without `endpoint`, or one that is not `tcp:HOST:PORT` or `udp:HOST:PORT` (io.md 2), its host a host as "Functions" says |
| `net:S004` | error | An endpoint defined again, at the second one |
| `net:S005` | error | A time below 0 or above one day, as in simulation.md 3 |
| `net:S006` | error | A request or a response given in both forms or, for a request, in neither; an empty request; hexadecimal that is not pairs of digits; more than 65,536 bytes *(proposed)* |
| `net:S007` | error | A request that equals another request of the same device, or is the start of one (io.md 6) |
| `net:S008` | error | `refused` or `close_ms` on a UDP device |
| `net:S009` | error | More than 256 devices, or 10,000 exchanges in one device *(proposed)* |

An endpoint of the file is written as a program reaches it (io.md 2), so `tcp:Board.local:0023` is `tcp:board.local:23`.
An endpoint that a `visa` instrument of the file describes by a real name is reached by `visa.open` only (0.5.0, io.md 6):
`connect` to it gives `net.host_not_found` after its timeout, as an endpoint the file does not describe (io.md 3),
whose message says to open it with `visa.open`.
The faults of a device are times on the clock of the run, as in simulation.md 3 (decision of the owner, 2026-11-19):

- **`close_ms`**: from that time the connections to the device are closed. What was ready before is still read; a read
  that needs more, and a write, fail with `io.closed`; a read that waits across that time fails then. A `connect` after it
  succeeds, with a connection already closed.
- **`hang_ms`**: from that time the device takes no request and gives no response. A write still succeeds, and its bytes
  are ignored; a read waits for its timeout. A `connect` after it succeeds.
- A response that would be ready at `hang_ms` or `close_ms`, or after, is never given.
- **What waits unread** (decision of the owner, 2026-11-19): a device keeps at most 16 MiB of answers not read yet per
  connection, and per UDP socket the bytes of its datagrams waiting. A response that would pass that is dropped, as by a
  device whose buffer overflows; the write that asked for it still succeeds, and the report keeps its entry.
- **`refused = true`**: every `connect` fails at once with `net.connection_refused`, code 0.
