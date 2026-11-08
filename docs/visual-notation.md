# Visual notation — v0

*Last updated: 2026-11-08. Status: draft, nothing is frozen; the compact view (3.2), automatic placement (2.1) and wire routing (2.2) are written for M0v.*

How a LabCraft diagram is drawn. The language spec (`language-spec-v0.md`) defines the text; this
document defines its graphical view. It serves the M0v viewer and the M1 editor and, before them,
the project images generated from real `.lcf` files. Open questions are numbered N-1, N-2… at the
end of the document.

[![Board of the visual notation v0: wire families and terminals, nodes and primitives, structures, run-time states, zoom levels and theme tokens](images/visual-notation-board.png)](images/visual-notation-board.png)

*The board, drawn by hand on 2026-10-31 (click to enlarge). It does not show the compact view of
nodes yet (3.2, N-10 to N-12).*

A function rendered from its file by `tools/lcf-figcraft`, not drawn by hand
([examples/check_voltage.lcf](../examples/check_voltage.lcf)):

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="images/rfc/check_voltage-dark.png">
  <img alt="Diagram of check_voltage rendered from its file: error chain in red along the bottom, VISA session in blue, match and handle unfolded on the right" src="images/rfc/check_voltage-light.png">
</picture>

**Files.** Both open with [FigCraft](https://github.com/storytold/figcraft), a design app of the
Craft family:

- [design/visual-notation-v0.figcraft](../design/visual-notation-v0.figcraft): the board above.
- [design/notation.figcraft](../design/notation.figcraft): the component library that
  `tools/lcf-figcraft` reads to draw diagrams (terminals, primitives, pills, node and structure
  samples, one sample per wire family, theme tokens in light and dark modes). Edit it, rerun the
  tool, and every rendered diagram follows.

## 1. Principles

1. **The drawing is a view of the text (D-08).** Every drawn element corresponds to a construct of
   the text, and the reverse. Only the position comes from elsewhere (`layout` section, spec 11).
   If a graphical element has no equivalent in the text, it does not exist.
2. **Readable without color.** The type is read from the shape of the terminals and the pattern of
   the wires; color only doubles the information. A diagram stays readable printed in black and
   white, or by a color-blind person.
3. **Order and resources first.** On a test bench, the question is "in what order do the actions
   happen, and on which equipment?". The error wire carries the order (spec 5.3), the session
   carries the resource: these are the only two wires drawn in color. Data is drawn in a single
   neutral ink.
4. **The unit is visible.** A physical quantity shows its unit on its terminal and on its wire; an
   implicit conversion (`mV` to `V`) is written on the wire.
5. **Its own identity (D-23).** A notation designed from the LabCraft spec and from the general
   conventions of schematics and node editors. No palette, icon or shape borrowed from NI. Every
   proposed shape is justified by a construct of the text, never by "as in such and such tool".
6. **Theme tokens.** No hard-coded color: tokens (`wire.error`, `node.fill`…) defined by the
   `craft-ui` theme, light and dark (Craft convention).

## 2. The canvas

- The flow goes **left to right**: inputs on the left of a node, outputs on the right.
- **The function is a frame**, like a structure: its inputs are terminals on the left edge, its
  outputs on the right edge, in signature order, except `err`, which is always at the bottom of each
  edge (2.1). An optional input shows its default value next to
  its terminal (`low ◂ 220 V`). The `///` documentation is the tooltip of the title.
- Orthogonal wires, rounded corners (radius 6). Grid of 8 units.
- Canvas units: 1 unit = 1 logical pixel at 100% zoom, origin at the top left;
  `node x at (40, 80)` places the top-left corner of the node (spec 11).

### 2.1 Automatic placement

A function without a `layout` section, which is what an agent writes, is placed automatically, and so
are the nodes that a `layout` section does not place (spec 11).

- **One drawing per file.** Placement and routing depend only on the file, on what it uses from other
  files (the signatures, type declarations, icons and bands of what it calls), on the fixed font of the
  notation, and on the view state: the view of each node, tabs or unfolded branches and the branch
  shown, folded or unfolded constructions, optional inputs shown (3, 5.2, N-2). With the same
  view state, the same file gives the same drawing on screen, when it is opened again, in an export and
  in the snapshots of CI (scenario 6 of `acceptance-m0.md`). Nothing comes from a previous drawing, from
  the size of the window or from the order in which files were read.
- **Stable by construction.** Every choice rests on what changes little: the dependencies, the order of
  the signature and the names, through the canonical order of lines (spec 5.2), which keeps the written
  order only for ties (call statements without a label, lines whose outputs are all `_`, `collect`
  lines, the arms of a `match`, a body that cannot be ordered). No choice is random. A small
  change moves what depends on it, as far as the rules below allow. For each example and each data wire
  between two nodes, CI inserts a neutral node on the wire (`core.select(true, v, v)`, which passes the
  value on unchanged), compares the drawing with the drawing before, and reports how many other nodes
  change place. M0v reports the figure; the threshold is set before the M0v tag, on measures of the
  viewer itself.
- **Columns.** The flow goes left to right: a node stands in the column after the last of the nodes
  that feed it, and a node fed only by the inputs of the function in the first column. Nodes with no
  input (a named constant, `time.now()`) are left out of that count; each one then stands in the column
  just before its leftmost reader, and when that reader is in the first column, a column opens before
  it. A node with no input and no reader stands in the last column.
- **The main error chain on one straight line** (N-7). The main chain is the chain of nodes joined by
  their error pass-through pairs (3.1) that starts at the `err` input of the function and goes through
  the most nodes; on a tie, the one that reaches the `err` output, then the first in the canonical order
  of lines. A function with no `err` input starts it at the first node, in that order, whose error input
  is not wired. The chain runs along the bottom of the nodes it crosses. Every other node goes above it,
  stacked in its column, in the order that keeps crossings low, ties broken by the canonical order of
  lines. Below the axis, apart from the lower edge of the nodes on the chain, there are only error wires
  that leave the main chain (`e2` in `check_voltage` leaves it at `query` and joins `handle e2`). A
  function with no error pair at all (`projectile`) has no axis: its nodes stack upward from the bottom
  of the frame, by the same rules.
- **Error pairs of structures.** A `loop` or a `for` with an error register has the two boxes of the
  register as its pair. A `match` or a `handle` whose arms take an outer error and yield an error output
  has a pair too: that error, entering by its notch or as the selector of a `handle`, on the bottom line
  of the left edge, and that output. With two error registers or two error outputs, the first in the
  text makes the pair. Inside a body, the chain starts at the inner side of the pair (the register box,
  the notch, the value bound by the arm) and ends at `next` of the register or at the error that the arm
  yields.
- **Through structures.** A structure on the chain is placed so that its error input and output sit on
  the axis around it, and the chain inside goes on along the same line: one straight line through every
  level of nesting. Structures grow upward, and their error lines line up. A structure off the main
  chain keeps its own chain straight along its own bottom.
- **A partial `layout` section.** The nodes it places never move. A node that it does not place goes into
  free space between what it connects: right of what feeds it, left of what it feeds, above the error
  chain if there is one. The other unplaced nodes move apart only as far as needed. With no position in
  the file, this gives the placement above. What the file writes always wins: around a position or a
  `via` that it gives, the rules that say "never" hold as far as they can. The axis of the chain goes
  through the first node of the chain, placed or not, and a node of the chain that the file does not
  place goes on the axis.
- **When the file changes**, in the live viewer, the nodes glide from their old place to the new one in
  about 150 ms *(proposed)*, so that one sees what moved. There is no glide when the system asks for
  reduced motion. The drawing at the end is always the one computed from the file. The update time of
  scenario 6 runs to the new drawing; the glide comes after it. A node is followed across the change by
  the body that holds it (the function, an arm, a loop body) and the name that designates it in the
  `layout` section (spec 11: its label or the name of its line); a node with no name, by its rank among
  the nodes with no name of its body, in the canonical order. A node whose name changed appears in its
  new place.
- When the developer moves a node (M1), its position is written in the `layout` section, and it no
  longer moves.

### 2.2 Wire routing

- A wire never crosses a node, a label, a pill, a band or a wire name, and it enters a structure only
  through its terminals and its notches (section 5).
- A wire whose two ends are at the same height is straight. Between neighbouring columns, it otherwise
  has a single vertical segment, in the channel between them. A wire that skips columns, other than an
  error wire, runs on a horizontal track above the nodes it passes over (`low` and `high` in
  `check_voltage`), with one vertical segment at each end; a secondary error wire does the same below
  the chain.
- Two wires never share a segment: parallel wires are 8 units apart.
- The order of preference is the fewest crossings, then the shortest length, then the fewest bends.
  Ties are broken by the order of the signature and by the names, as for placement. These are
  preferences that a fixed procedure follows as well as it can, not an exact optimum, which large
  diagrams could not afford (scenario 6, point 4).
- A value that feeds several inputs runs as one trunk, with a junction point at each split (section 4).
- Every wire that is not an error wire (data, session, order, function) stays above the main error
  chain and never crosses it. An error wire that leaves the main chain runs below it, and crosses it
  once, in the channel before the node it reaches.
- Order wires are routed like the others.
- The waypoints that a `via` gives (spec 11) are kept, and the routing joins them.
- A terminal can sit on any side of its node, and the routing leaves it in that direction, so that
  sides chosen later (N-15) need no other routing. The terminals of a circle are off the grid of 8 (3.2), and the wire reaches them with a short
  segment.

## 3. Nodes

A function call has **two views**:

- **Compact, the default**: a small shape with the function's icon or a short label, under a band
  naming its library when it comes from another one, unnamed terminals (3.2). It lets a whole
  function fit on screen.
- **Standard, on request**: a block with the name as a header and named terminals (3.1).

The view of a node is changed in the IDE and saved in the `layout` section (`view standard`,
spec 11); an IDE preference displays everything in standard view. The read-only viewer of M0v
draws the view that the file gives; a view changed there is a state of the viewer, never written to
the file, like the branch shown in a `match` (N-2). **Primitives** are not a special
case: they are the `core` functions whose compact view is a circle with a glyph (3.2); a project's own
`add` is an ordinary function, with no glyph. The choice
of view changes nothing in the program: the text, its canonical form and its diffs stay the same.

### 3.1 Standard view

Rectangular block with rounded corners. Header: the function name (`visa.open`, the library in
muted ink). Named terminals, **in signature order**: inputs on the left, outputs on the right. The
sides of the terminals, and the error chain along the bottom, are **defaults**: what the IDE and the
agents draw when nobody chooses. The author of a function may later choose other sides for its
terminals (N-15).

```text
                ┌─ visa.open ───────────────┐
  resource ─────▬ resource              dmm ■━━━━━━━━ dmm
                │ timeout ◂ 2 s             │
  err ──›──›────▸ err                    e1 ▸──›──›── e1
                └───────────────────────────┘
```

- **Lines of terminals**: a line holds at most one input, on the left, and one output, on the right.
- **Pass-through pair on the same line**: an input and an output of the same opaque type (session)
  or error type share a line, so that the wire crosses the node in a straight line. Pairs are made
  by name first (an input `to` with the output `to`), then in signature order among the terminals of
  that type that are left. A pair that would cross one made before it, in signature order, is not
  kept: its two terminals are placed like the others. The `err` pair is always on the bottom line,
  even when `err` is not last in the signature (it is last by convention): lines are made as if `err`
  were last in both lists, so the error chain runs along the bottom of the nodes. A function with an
  error output and no error input (`errors.merge`) has its error output alone on the bottom line.
- **The other terminals**: the pairs fix their lines, in signature order. Before the first pair,
  between two pairs and after the last, the inputs and the outputs that lie there in their own
  lists fill lines side by side, in signature order; when one side has more, the rest take lines of
  their own. `visa.open` above: `resource` and `dmm`, then `timeout` alone, then the pair `err`,
  `e1`. `scpi.query_f64` (inputs `session`, `command`, `err`; outputs `value`, `session`, `err`):
  `value` alone, the pair `session`, `command` alone, the pair `err`, so four lines.
- **Inline constants**: a constant written as an argument (spec 5.1) is shown on its terminal
  (`timeout ◂ 2 s`, `◂ "MEAS:VOLT:DC?"`), not as a separate node. Long text is truncated (`…`),
  in full in the tooltip.
- **Named constant** (`limit = 240 V`, spec 5.1): a small node without a body, the user places it near what it
  feeds. It shows its name, its value in a pill and one output terminal (`limit  240 V ●`), and its wire splits like any
  other. Dragging a wire from an inline constant to a second terminal makes one. A new name for a wire (`err = e3`) still
  draws nothing.
- **Constant construction** (`k = Limits { low: 220 V, high: 240 V }`, or a variant with values): when every field is a
  constant and the values fit in a tooltip, it is drawn folded by default, as a small node (`k  Limits {…} ●`). Each node can be
  unfolded into its block; the file does not change, and an exported image shows the block.
- **Nodes of `ui`** (`ui.set`, `ui.get`, spec 10): the element is written on the face of the node (`set live_voltage`), with no
  terminal for it, since it is no wire. An element named like an input or an output of the function is also that terminal of the
  function, and selecting an element selects its nodes and terminals. The default of an input has one field in the inspector,
  shared with the pill on its terminal, since it belongs to the signature.
- **`led` on a panel** (spec 10): until the developer chooses, grey when off and one accent colour, neither green nor red, when
  on, with the name of the variant on an enumeration. No default gives a colour a meaning: colours by position would make a
  `Fail` green.
- **Unwired optional inputs**: hidden by default, with a `+2` indicator in the header; shown with
  their default value on hover or on request.
- **Ignored output** (`_`): hollow terminal, no wire.
- **`always`**: `always` badge in the header. This is the node that runs even after an error; it
  must be spotted at a glance.
- **Label** (`#close:`): shown above the node.
- **`after:`**: small order terminal to the left of the header, connected by an order wire (4.1).
- **The name of the node** (key of the `layout` section) is that of its first named output, already
  written on its terminal, or its label (spec 11): nothing more to draw.

### 3.2 Compact view

```text
               ┌─ visa ─┐
  resource ────▬        ■━━━━━━━━ dmm
     2 s ◂ ────●  open  │
  err ──›──›───▸        ▸──›──›── e1
               └────────┘
```

A compact node is a **block** whose center says what the function does: its icon, or a short label.
When the function comes from another library, a **band** above the block says which.

- **Band**: the name of the library of the function (`visa`, `scpi`, `dmm_driver`, `results_log`), whole
  up to 12 characters, beyond that cut to 11 followed by `…`, in the token `band.text` on `band.fill`, neutral by default,
  with a contrast of at least 4.5:1 in both themes. The band is 12 units high and as wide as the block,
  which widens to hold it. A call to a function of `core`, or of the library that holds the diagram, has
  no band. A folder of the project is a library like any other (spec 4), so a call from the root to
  `relays.close` has the band `relays`. A sub-library `a.b` shows its last segment, `b`, with the full
  path in the tooltip; when two bands of one diagram would read the same (`x.util` and `y.util`), each
  shows the full path. The texts compared are those drawn, after the cut: when they still read the
  same, each is cut in the middle instead, keeping its start and its end (`measurement_a` and
  `measurement_b` give `measu…ment_a` and `measu…ment_b`). The label (`#close:`) is drawn above the band, or above the block when there is
  no band. Neither counts in the position: `node x at (40, 80)` places the top-left corner of the block
  (spec 11), so a band never moves the terminals. A library cannot yet change its band (N-11).
- **Block**: at least 40 × 40 units. Its width holds the label or the band, rounded up to the grid.
  Its height is the larger of 40 and 16 units per line of terminals (below), plus 8 for each pair of
  neighbouring lines that both carry an inline constant: 40 for one or two lines, then 48, 64, 80.
- **Center**: the function's icon (`icon` section, spec 11.1), alone, drawn 28 × 28 units; otherwise a
  short label: the last segment of the name (`visa.open` → `open`). A label of more than 8 characters
  that holds an underscore is written on **two lines**, cut at an underscore, which is not drawn. The
  cut is the one whose longer line is the shortest, the first one on a tie, and never one that leaves
  a line empty: `query_f64` → `query` / `f64`, `bench_selftest` → `bench` / `selftest`,
  `read_dc_volts` → `read_dc` / `volts`, `count_to_fixed` → `count` / `to_fixed`. When no cut is allowed (`abcdefgh_`), the label stays on
  one line. A line of more than 12 characters is cut to 11 followed by `…`. Here and in the band,
  characters are code points, and a cut falls between two graphemes, before the one that would pass
  the limit. The author does not declare a label: the icon is there for that. The qualified
  name (`visa.open`) is in the tooltip, and in the `<title>` of the node in an exported SVG.
- **Unnamed terminals**: inputs on the left, outputs on the right by default (3.1, N-15), on the
  **same lines as in standard view**: signature order, pass-through pairs on one line. Switching a
  node from one view to the other never reorders its terminals. By default (N-15), lines are counted
  from the bottom: the last one is 8 units above the lower edge, and each line above it is 16 units higher, or 24 when both lines carry an inline
  constant, since a pill is 18 high. A block of one line has it at 32 from the top, a block of two
  lines at 16 and 32. The name of a terminal is shown in the tooltip and while wiring, on the targeted
  terminal. The shape of the terminal still tells its family (4.1).
- **The `err` pair stays on the bottom line**, as in standard view: the error chain crosses the
  node in a straight line.
- **What the compact view never hides**: the `always` badge (reduced to `▴` at the bottom center,
  clear of the `err` terminal, drawn over the icon if they meet), the inline constants (abbreviated to
  8 characters), the label (`#close:`), the `after:` terminal (on the left of the band; with no band, on
  the top edge of the block, 8 units from its left corner), the conversion marks and the run-time state
  (6). Otherwise the diagram would lie about the execution.
- **Unwired optional inputs** are hidden, with no `+n` indicator: the tooltip lists them with their
  default values.
- **Construction** (`StepResult { … }`, 3.3): the label is the name of the type, without its library,
  which goes to the band (`results_log` above `StepResult`). **Variant constructor**: the type and the
  variant on two lines (`Reading` / `Value`).
- **Nodes of `ui`** (`ui.set`, `ui.get`, 3.1): no band; the label is the operation and the element on
  two lines (`set` / `live_voltage`), so the element stays on the face of the node.
- **Indistinguishable inputs**: when two or more wired inputs have the same type and nothing
  tells them apart (no inline constant, no wire name), the editor warns: "inputs `low` and `high`
  indistinguishable in compact view", with the action "show in standard view". This is a warning,
  not an error.

**Primitives.** The compact view of the `core` operations is a **circle** of 28 units with a
glyph, unnamed terminals (name in the tooltip). The list is closed (N-4):

| Glyph | Primitives |
|---|---|
| `+` `−` `×` `÷` | `add`, `sub`, `mul`, `div` |
| `−` with one input | `neg` |
| `√` | `sqrt` |
| `\|x\|` | `abs` |
| `≥` `≤` `>` `<` `=` `≠` | comparisons |
| `∧` `∨` `¬` | `and`, `or`, `not` |
| `?` | `select` (condition on the upper input, then value if true, value if false) |
| `[V]` | `with_unit` (shows the added unit); `unitless`: the requested unit, struck through |

A unit too long for the circle (`[kg/(m*s)]`) turns it into a capsule of the same height, as wide as
the unit needs. When two neighbouring inputs both carry an inline constant, the circle stretches into
a capsule of the same width, as tall as needed for those inputs to be 24 apart, as on a block, since
a pill is 18 high (`select(ok, 1 V, 0 V)`). The other `core` functions (`min`, `max`, `pow`, the conversions, `call`) have the
ordinary compact view (block, no band).

**Constant on a primitive input**: the circle has no terminal line, so the pill is attached to the
left of the terminal, outside the circle (`◂ 0.5` in front of `>`).

**Terminals of a circle**: on its edge. One input at the middle of the left side, two at 8 units
above and below the middle, three at 10 above, at the middle and 10 below; the output at the middle
of the right side. They are not on the grid of 8: the routing of the wires (2.2) joins them.

### 3.3 Field access, structs, variants

- **Field access** (`step.low`): no node. The wire starts at `step` and carries, just before the
  terminal, a `.low` pill. This is what the text says: a path as an argument, not a call.
- **Construction** (`StepResult { name: n, … }`): block with the header `StepResult { }`, one
  input per field in declaration order, one output.
- **Enum value** (`Verdict.Pass`): inline constant. **Variant constructor** (`Reading.Value(v)`):
  small `Reading.Value` block.

## 4. Wires

**One value = one wire** (spec 5.1). The wire starts at a named output terminal; it splits at
junction points when the value feeds several inputs. The name of the value is that of the output
terminal; a long wire repeats its name halfway.

### 4.1 Wire families

| Family | Types | Wire | Terminal | Color |
|---|---|---|---|---|
| Number | integers, floats, complex, `duration`, `timestamp` | solid, 1.5 | filled circle | `wire.data` |
| Boolean | `bool` | solid, 1.5 | diamond | `wire.data` |
| Text | `string`, `bytes` | solid, 1.5 | rectangle | `wire.data` |
| Enumeration | `enum` | solid, 1.5 | triangle | `wire.data` |
| Struct | `struct` | sheathed: outline of 4, empty core | double circle | `wire.data` |
| Array | `[T]`, `[T; n]` | that of `T`, with cross-ticks (small perpendicular strokes) every 8; one tick per dimension | that of `T` in brackets | `wire.data` |
| Session, resource | opaque types, `ref<T>`, `queue<T>` | solid, 2.5 | filled square | `wire.resource` |
| Error | `error` | solid, 2, chevrons `›` every 24 in the flow direction | chevron | `wire.error` |
| Order | `after:` | thin dotted | hollow chevron | `wire.order` (muted) |
| Function | function references (Q-14) | dash-dot | `ƒ` | `wire.data` |

- The chevrons of the error wire show **the direction of the order**: this is the wire you follow
  to know what happens before what.
- The exact type (`f64[V]`, `[StepResult]`) is always in the tooltip of the wire and the terminal.
- **Order wires** are the text's `after:` (spec 5.3). The command "order in sequence"
  (`order.chain`) chains the selected nodes with order wires, in the order they were selected, and
  adds the labels that are missing.

### 4.2 Units and conversions

- A terminal or wire that carries a unit shows a badge: `voltage [V]`, `period [ms]`.
- **Implicit conversion** (numeric widening spec 3.4, scale change spec 3.3): written on the
  wire, just before the terminal that receives it, in a `mV→V` or `i16→i32` pill (token
  `mark.coercion`). You read the conversion instead of guessing what a mark means.
- Unit error at edit time (`V + A`): the wire is drawn in `mark.invalid`, interrupted at the
  terminal, with the message in the tooltip.

## 5. Structures

A structure is a **frame** with, as header, the keyword of the text (`loop continue_on_error`,
`for x in readings`, `match state`, `handle e6`). Its outputs are terminals on the right edge, in
the order of the text, except its error output, which is always on the bottom line, so that the error
chain crosses the frame in a straight line (2.1).

- **Outer values** used inside (spec 6): the wire crosses the edge; a notch marks the crossing.
  No terminal to wire. A value that is also the selector of a `match` or a `handle` enters by the
  selector, which serves as its notch: one wire outside, not two side by side. The developer can still
  give it a notch of its own, which the `layout` section keeps (spec 11).
- **Nesting**: frames contain each other; the background darkens by one step per level (token
  `frame.fill.n`).

### 5.1 `loop` and `for`

```text
        ┌─ loop ───────────────────────────────┐
 0 V ──┤↻├ v ─────── …               … ── next v ┤↻├───── value
        │                             until ◆  │
 err ──┤↻├ e ───›─── …               … ─›─ next e ┤↻├───›── err_out
        └──────────────────────────────────────┘
```

- **`carry` register: a register terminal**, a small box straddling the edge, marked `↻` ("the
  value goes around"). It renders the four values of the text:
  - **on the left**, the initial value enters from outside (inline constant or wire) and the
    **current** value (`v`) leaves on the inside;
  - **on the right**, `next v` enters on the inside and the final value leaves on the outside.

  The two boxes of a register are at the same height and carry the same name: they are paired
  without an arrow, whatever the number of registers. The outline of the box takes the color of
  the register's family (error style for an `error` register). The labels `v` on the left and
  `next v` on the right make visible the trap noted in spec 13: `v` in the body is the current
  value, not the new one.
- **Return arc: in the editor only.** On hover or selection of a register, an arc connects its two
  boxes over the frame, with the tooltip "`next v` becomes `v` at the next iteration". Exported
  images do not have it: with several registers (five in `station`), stacked arcs would become
  unreadable. No pair of vertical arrows on the edge, a shape too close to an existing editor
  (D-23).
- **`collect`**: `[ ]` terminal on the right edge; the outgoing wire is an array.
- **`for x in array`**: `]▸` terminal on the left edge (the array enters, the element leaves);
  `for i in 0..n`: `0..n` terminal on the left edge.
- **`until`**: `until` terminal on the right, inside the frame, just above the error register, so
  that the error chain keeps the bottom line (2.1); it is a `bool`, so a diamond.
- **Error register**: box in error style, on the bottom line of the frame; it is a reminder that the loop stops when it contains
  an error (spec 6.3). **`continue_on_error`**: badge in the header, since it changes this default
  behavior.
- **`for parallel`**: header `for parallel`, double-line frame.

### 5.2 `match` and `handle`

- **Selector**: terminal on the left edge, at the level of the header. The selector of a `handle`,
  which is an error, enters on the bottom line instead, which is the axis when the `handle` is on the
  error chain (2.1).
- **One branch at a time**: a tab bar in the header lists the branches (`true`, `false`;
  `State.Init`, `State.WaitDut` …; `_` is written `other`). A branch with several patterns is one
  tab, which lists them (`1, other`). The displayed branch is highlighted.
  A bound value (`Reading.Value(v)`, `io.timeout(err)`) appears as a terminal on the inner left
  edge of that branch.
- **`yield`**: each branch wires the same output terminals on the right edge. A terminal that a
  branch does not wire is impossible (the text forbids it), so there is nothing to flag here.
- **Unfolded view**: all branches stacked vertically, each with its title. This is how exported
  images and printouts are rendered, and an editor mode. The last branch in the text sits at the
  bottom, on the axis of the error chain; the others are stacked above it, each with its own straight error chain, joined to
  the error output of the frame. Where the output wire starts when each
  branch produces the value, and how to draw an outer value used in several branches: N-9.
- **`handle`**: same frame, edge in error style; the tabs are the categories (`ok`,
  `dmm.overload`, `visa.*`, a code, `other`).

## 6. Run-time states

For the values of a run on the wires (M0v, once M0b runs programs), the debugger (M5) and images
rendered from a run trace
(`tools/lcf-poc/lcf.py run … --trace`):

| State | Rendering |
|---|---|
| Value of a wire (probe) | bubble attached to the wire: `230.06 V`, `Verdict.Pass`, `[3 elements]` |
| Skipped node (error on input) | hatched body, muted ink |
| Node that produced an error | `wire.error` outline, category in a badge (`io.timeout`) |
| Running node | `node.active` outline |
| Loop | iteration counter in the header (`× 6`) |

## 7. Zoom and density

M0v target: 1,000 nodes and 2,000 wires, panned and zoomed at 60 frames/s; M1 keeps it while
editing (`roadmap.md`). Rendering depends on the
zoom:

| Zoom | Shown |
|---|---|
| ≥ 75% | everything |
| 40 to 75% | no terminal names, no long inline constants |
| < 40% | shapes, wires, structure titles; nodes as plain rectangles |

## 8. Theme tokens

`canvas.fill`, `canvas.grid`, `node.fill`, `node.stroke`, `node.title`, `node.active`,
`frame.fill.1..4`, `frame.stroke`, `wire.data`, `wire.resource`, `wire.error`, `wire.order`,
`mark.coercion`, `mark.invalid`, `probe.fill`. Supporting tokens, which appeared while drawing
real diagrams (`tools/lcf-figcraft`): `ink.muted`, `ink.faint` (secondary text), `node.divider`,
`frame.divider`, `pill.fill`, `pill.stroke`, `pill.text` (inline constants, fields),
`coercion.fill`, `badge.fill`, `badge.text`, `tab.fill`, `band.fill`, `band.text` (library band of the
compact view, 3.2). Contrast of at least 4.5:1 between each
wire and `canvas.fill`, in both themes. The values are chosen with the `craft-ui` theme, with no
reference to another editor's palette (D-23).

- **"Muted" does not exempt from contrast.** `wire.order` is told apart by its pattern (thin
  dotted), not by a pale color: a light gray falls below 4.5:1.
- **Contrast is also checked on the structure backgrounds** (`frame.fill.1..4`), not only on
  `canvas.fill`: a wire at the limit on the canvas falls below it inside a nested frame.
- **Provisional values of the light theme**, those of the board
  (`design/visual-notation-v0.figcraft`), contrast on `canvas.fill` `#F7F7F5`: `wire.data`
  `#3D4148` (9.6:1), `wire.resource` `#1D6FB8` (4.8:1), `wire.error` `#C2410C` (4.8:1),
  `wire.order` `#6A7078` (4.7:1), `mark.invalid` `#DC2626` (4.5:1). A first attempt at `#D9480F`
  for the error (4.0:1) and `#9AA0A6` for the order (2.5:1) failed. On `frame.fill.1` `#EEF0F2`,
  `wire.error` drops to 4.5:1, and **from `frame.fill.2` on, the error and order wires fall
  below** (4.1, then 3.8 and 3.5 at levels 3 and 4, measured by
  `tools/lcf-figcraft/build_notation.py`); open question N-13. The values of `frame.fill.3`,
  `frame.fill.4` and of the dark theme in `design/notation.figcraft` are provisional.

## 9. Open questions

| # | Question | Lead |
|---|---|---|
| N-1 | A "color by type" mode for data wires? | Not in v0: shape and pattern are enough; to revisit on real diagrams |
| N-2 | Branch displayed by default for a `match` | The first; the editor remembers the last one viewed (editor state, not saved in the file) |
| N-3 | Repeated wire names: from what length? | 400 units; to tune on `station` |
| N-4 | Primitives: closed list of glyphs or extensible by a library? | **Settled (2026-11-08):** closed, the table of 3.2; `neg` and `sqrt` added |
| N-5 | Units and origin of the canvas in the `layout` section | **Settled (2026-11-02):** those of section 2, written in spec 11 |
| N-6 | Layout stability when a signature changes (terminals added) | The top-left corner is fixed, the node grows downward. In compact view the lines are counted from the bottom (3.2), so a block that grows moves its bottom line, and the error chain, down: from 32 to 40 when it goes from two lines to three. This holds for a node that the `layout` section places; a node placed automatically follows 2.1 |
| N-7 | Does automatic layout keep the error chain straight? | **Settled (2026-11-08): yes, at every level** (2.1). The main error chain occupies one horizontal line, at the bottom of the nodes it crosses, through every structure on it; every other node goes above it, and only error wires leave it downward. Columns follow the order of dependencies. Tried on `check_voltage` and `settle` (`tools/lcf-figcraft`, which does not yet keep the line straight through a `match`) |
| N-8 | Accessibility: keyboard traversal of the diagram and through AccessKit (D-03) | Traversal order = canonical form of the text (spec 5.2) |
| N-9 | Unfolded view of a `match` or `handle`: where does the output wire start when each branch produces the value? How to draw an outer value used in several branches? | Current `lcf-figcraft` rendering: the output starts from the first branch, the others have a terminal of the same name on the right edge; an outer value passes a notch, then a small vertical bus. To judge on `station` |
| N-10 | Short label of the compact view: always derived from the name, or declarable by the author? | **Settled (2026-11-08):** derived from the name, on two lines when it holds an underscore (3.2); the icon is there for anyone who wants better. A cut at 8 characters gave `bench_s…` and `query_f…` on the examples |
| N-11 | An icon shared by several functions of a library, and the band of a library | Copied into each file in v0, and the band is the plain name of the library (3.2). To judge on the viewer of M0v and settle before the editor (M1): a **library mask**, which a library declares once to change its band (a shorter text, `DMM` for `dmm_driver`; a proposed color, from which the theme derives the light and dark fills and which the developer can change) and to give the icons of its functions a common vector background, in the shapes of spec 11.1. Where it is declared, the manifest or a text file of the library, is open too. Lead: the band stays text and color, readable at 12 units high; drawing belongs to the icon |
| N-12 | The `tools/lcf-figcraft` rendering draws only the standard view | To complete in order to judge the compact view on `check_voltage`, `settle` and `station` |
| N-13 | Error and order wires fall below 4.5:1 from `frame.fill.2` on (section 8) | Lighten the nesting backgrounds rather than darken the wires |
| N-14 | How does the editor name the values of what the user draws? The text needs a name for each wire, and the `layout` section designates a node by the name of its line (spec 11) | The user never names a line, only a wire, if they want to. A new node gets a name at once, from the output of its signature (`timestamp`), or from the function when that output says nothing (`result` of `core` gives `sum`, `product`), with a suffix when the name is taken. Renaming the wire on the canvas renames the value in the text and in `layout`. A node with no useful output gets a label when it needs one. To judge on a mock-up in M1: `timestamp2`, `timestamp3` are what the user reads on the wires |
| N-15 | Sides of the terminals, and their place along a side, chosen by the author of the function: a configuration entering from the top, a vertical error chain, a wire that goes back to the left, a single line in the middle of a block | Left and right, with the error chain along the bottom and the lines counted from the bottom, are the defaults (3.1, 3.2); nothing in v0 chooses otherwise, so adding the choice later breaks no file. To settle before the editor (M1). Retained (2026-11-08): the choice belongs to the author of the function and is stored in its file, next to its `icon`, so that every call to it looks the same on every screen and in every export; never to each node, nor to a preference of the IDE. Lead: any of the four sides; a mark of direction on a terminal that leaves its default side, since the shape of a terminal tells its family, not its direction. The routing of the wires leaves by any side from M0v on (2.2) |
