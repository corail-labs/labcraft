# Glossary

The terms LabCraft uses, with their meaning. Keywords of the language are in `code`.

| Term | Meaning |
|---|---|
| **dataflow** | Execution model: a node runs when its inputs are available; the order of lines in the text has no effect. |
| **node** | One call of a function or primitive in a diagram. |
| **native function** | A function implemented outside LabCraft: a standard-library primitive or an `extern` function. Only native functions are skipped on an error or marked `always`; a function written in LabCraft always runs its body (spec 7.2). |
| **skipped node** | A node of a native function that did not run because its `err` input carried an error; its outputs take their default value, and an output named like an input passes that input through. |
| **default value** | The value of a type when nothing else gives one: `false`, `0` in the type's unit, `""`, an empty array, `none` for an error… It is what a skipped node outputs and what `ui.get` returns without a user interface (spec 7.2). |
| **`always` node** | A node of a native function marked `always`, which runs even when its `err` input carries an error (for example `visa.close`), but not after an abort or a fault (spec 8.2). |
| **primitive** | A `core` operation (`add`, `gt`, `select`…) whose compact view is a circle with a glyph. |
| **wire** | A connection from one output to one or more inputs. |
| **error wire**, **error chain** | The wire that carries an `error` value from node to node; the chain that orders "set, then measure". |
| **order wire** | A wire created by `after:`, which carries no data, only an ordering. `after:` takes a value, a label or a list of them (`after: e2`, `after: #beep`, `after: (#beep, e2)`), on any node. There is no sequence structure: order comes from data, the error wire or order wires (spec 5.3). |
| **terminal** | The point where a wire enters (input) or leaves (output) a node or a structure. |
| **structure**, **frame** | `loop`, `for`, `match`, `handle`: drawn as a frame that contains a sub-diagram. |
| **carry** | A loop register (`carry`): its value goes from one iteration to the next through `next`. |
| **shift register** | LabVIEW's name for a loop register; LabCraft calls it a carry. |
| **struct** | A record type with named fields (`struct { voltage: f64[V], ok: bool }`); LabVIEW calls it a cluster (spec 3.2). |
| **enum** | A type whose value is one of a list of named variants (`enum { Pass, Fail, Error }`), chosen between with `match` (spec 3.2). |
| **`handle`** | The structure that branches on an error's category (`ok`, `io.timeout`, `_`…). |
| **session**, **opaque type** | A value that designates an open connection (`visa.session`) without showing its content. |
| **variadic parameter** | A parameter that takes any number of arguments (`parts: string...`). |
| **pass-through pair** | An input and an output of the same session or error type on one line, so the wire crosses the node straight. |
| **notch** | Where a wire from outside a structure enters its frame. |
| **the carry trap** | Inside the body, a carry holds its current value, not the one just assigned with `next`. |
| **panel** | The front panel attached to a function (`panel` section): controls and indicators. |
| **compact view**, **standard view** | The two ways a node is drawn: a small shape with an icon or short label (default), or a block with one named terminal per line. |
| **inline constant**, **pill** | A constant written as an argument, drawn on its terminal as a small pill rather than as a node. |
| **layout** | Positions and wire routes on screen (`layout` section), separate from the program's meaning. |
| **icon** | The drawing of a function's compact view (`icon` section). |
| **canonical form** | The single text a program is formatted to (`labcraft-cli fmt`), so that diffs only show real changes. Its order of lines and the layout of its text are defined in spec 5.2. |
| **call statement** | A call written alone, with no output bound and an optional label (`#beep: sound.beep()`), for a function without outputs (spec 5.1). |
| **contextual word** | A word reserved only where the syntax expects it (`ok` and `code` in a `handle` pattern, `in` in a `for` clause, `after` in an argument list); elsewhere an ordinary name (spec 14.1). |
| **unit**, **dimension** | `f64[V]`: a number with a physical unit; its dimension is the vector of SI exponents. |
| **coercion** | An implicit conversion the editor shows (`mV` to `V`, `i16` to `i32`): it never changes the dimension or loses range. |
| **implicit widening**, **explicit narrowing** | Conversions that cannot lose information are implicit; those that can are written. |
| **error**, **fault** | An `error` is a value that flows in the program. A fault (proof of concept: `lcf.*`) is a failure of the run itself, such as an iteration limit: no wire carries it, and it stops the run (spec 8.2). The proof of concept reports some checks, such as unit mismatches, only at run time; the language checks units at compile time. |
| **warning** | An editor message that does not prevent running. |
| **probe** | A value shown on a wire while the program runs. |
| **label** | A name given to a node (`#close:`), unique in its function; `after:` and the layout designate the node by it, and traces show it. |
| **library**, **package** | A set of functions under one name (`visa`); a package is a library distributed with a version. |
| **simulated instrument** | An instrument described in TOML that answers like the real one, so a program can be tested without hardware: the simulator of the `visa` library answers for its `SIM::` resources. The definitions are in `instruments/` today, and move to `libs/visa/sim/` when the Rust code starts. |
| **test bench**, **test executive** | The hardware that tests a product; the software that sequences tests and records results (TestStand and others). LabCraft is called by an executive, it is not one. |
| **milestone** | A step of the roadmap (M0, M1…). |
| **Accepted**, **Proposed**, **Open** | Status of a decision in [decisions.md](decisions.md). |
| **project lead** | The person who accepts decisions. |
| **proof of concept (PoC)**, **throwaway tool** | Python tools in `tools/` used during the specification; they will not become the product. |
| **clean room** | The rule that LabCraft is designed from public documentation and user feedback only, never from NI software or files (D-23). |
| **call for feedback** | The current phase: the specification is public and open to comments before the language is frozen. |
