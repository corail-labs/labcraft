# `console`: the text output of a program

*Status: specified for release 0.5.0 (M4, [docs/io.md](../../docs/io.md), section 5), not built yet: its declaration
and its crate `labcraft-console` come with its Rust.*

`console` lets a program write lines of text, to follow what it does while it runs: the value read from a board, the
step of a test, a frame sent. Its one function is native (`extern rust`, spec 4.1), and each host gives its own body,
so that the lines go where its user looks: the tab `Console` of the panel `Run` in the IDE, stderr for
`labcraft-cli run`, the stdout of a standalone executable, the console of the developer tools in the browser, and
always the report of the run (io.md 5, "The console").

```text
fn console.write_line(text: string) -> ()
```

- It has no `err` and never fails: a line written after an error is still written. It needs no consent.
- The report keeps the lines in its field `console` ([commands.md](../../docs/commands.md) 7), each with its time on
  the clock of the run, within the bounds of the report.
- Reading a line typed by the human is not in 0.5.0.
