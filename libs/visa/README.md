# `visa`: sessions on instruments

*Status: draft for milestone M0b. The proof of concept follows it (`tools/lcf-poc/lcfpoc/stdlib.py`), and so does the crate
`labcraft-visa`, in `src/`, in deterministic mode.*

`visa` is the transport library shipped with LabCraft: it opens sessions on instruments, writes commands
and reads answers. Its functions are declared in this folder (`extern rust`, spec 4.1), and their native
bodies come with its Rust crate, `labcraft-visa`, in M0b. Instrument drivers and `scpi` are libraries
over it. Not every instrument goes through VISA: another transport is another library, with its own
rules.

| Function | Does |
|---|---|
| `open(resource, timeout = 2 s)` | Opens a session on a resource (`SIM::DMM0`, a VISA resource name). `visa.resource_not_found` if there is none |
| `write(session, command)` | Sends a command |
| `read(session)` | Reads an answer; `io.timeout` if none comes within the timeout of the session |
| `query(session, command)` | Writes, then reads the answer |
| `close(session)` | Closes the session; `always` (spec 7.2): it runs even after an error |

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
gives the format, how an instrument answers, and the files shipped in [sim/](sim/).
