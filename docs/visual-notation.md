# Visual notation — v0

*Last updated: 2026-11-03. Status: draft, nothing is frozen.*

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
  outputs on the right edge, in signature order. An optional input shows its default value next to
  its terminal (`low ◂ 220 V`). The `///` documentation is the tooltip of the title.
- Orthogonal wires, rounded corners (radius 6). Grid of 8 units.
- Canvas units: 1 unit = 1 logical pixel at 100% zoom, origin at the top left;
  `node x at (40, 80)` places the top-left corner of the node (spec 11).

## 3. Nodes

A function call has **two views**:

- **Compact, the default**: a small shape with the function's icon or a short label, unnamed
  terminals (3.2). It lets a whole function fit on screen.
- **Standard, on request**: a block with the name as a header and one named terminal per line
  (3.1).

The view of a node is changed in the IDE and saved in the `layout` section (`view standard`,
spec 11); an IDE preference displays everything in standard view. **Primitives** are not a special
case: they are the `core` functions whose compact view is a circle with a glyph (3.2); a project's own
`add` is an ordinary function, with no glyph. The choice
of view changes nothing in the program: the text, its canonical form and its diffs stay the same.

### 3.1 Standard view

Rectangular block with rounded corners. Header: the function name (`visa.open`, the library in
muted ink). One line per terminal, named, **in signature order**.

```text
                ┌─ visa.open ───────────────┐
  resource ─────▬ resource                  │
                │ timeout ◂ 2 s             │
                │                       dmm ■━━━━━━━━ dmm
  err ──›──›────▸ err                    e1 ▸──›──›── e1
                └───────────────────────────┘
```

- **Pass-through pair on the same line**: an input and an output of the same opaque type (session)
  or error type share a line, so that the wire crosses the node in a straight line. Since `err` is
  by convention the last input and the last output, the error chain runs along the bottom of the
  nodes.
- **Inline constants**: a constant written as an argument (spec 5.1) is shown on its terminal
  (`timeout ◂ 2 s`, `◂ "MEAS:VOLT:DC?"`), not as a separate node. Long text is truncated (`…`),
  in full in the tooltip.
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
               ┌──────┐
  resource ────▬      ■━━━━━━━━ dmm
               │ open │
     2 s ◂ ────▸      │
  err ──›──›───▸ ▴    ▸──›──›── e1
               └──────┘
```

- **Shape**: square block of at least 40 units, which grows downward in steps of 12 units per
  terminal beyond three. At the center, the function's icon (`icon` section, spec 11.1); without
  an icon, a short label: the last segment of the name (`visa.open` → `open`), truncated to 8
  characters. The full name is in the tooltip.
- **Unnamed terminals**, in signature order: inputs on the left, outputs on the right. The name is
  shown in the tooltip and while wiring, on the targeted terminal. The shape of the terminal still
  tells its family (4.1).
- **The `err` pair stays on the bottom line**, as in standard view: the error chain crosses the
  node in a straight line.
- **What the compact view never hides**: the `always` badge (reduced to `▴` in the bottom-left
  corner), the inline constants (abbreviated to 8 characters), the label (`#close:`), the `after:`
  terminal, the conversion marks and the run-time state (6). Otherwise the diagram would lie about
  the execution.
- **Indistinguishable inputs**: when two or more wired inputs have the same type and nothing
  tells them apart (no inline constant, no wire name), the editor warns: "inputs `low` and `high`
  indistinguishable in compact view", with the action "show in standard view". This is a warning,
  not an error.

**Primitives.** The compact view of the `core` operations is a **circle** of 28 units with a
glyph, unnamed terminals (name in the tooltip):

| Glyph | Primitives |
|---|---|
| `+` `−` `×` `÷` | `add`, `sub`, `mul`, `div` |
| `\|x\|` | `abs` |
| `≥` `≤` `>` `<` `=` `≠` | comparisons |
| `∧` `∨` `¬` | `and`, `or`, `not` |
| `?` | `select` (condition on top, then value if true, value if false) |
| `[V]` | `with_unit` (shows the added unit); `unitless`: the requested unit, struck through |

The other `core` functions have the ordinary compact view (square block).

**Constant on a primitive input**: the circle has no terminal line, so the pill is attached to the
left of the terminal, outside the circle (`◂ 0.5` in front of `>`).

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
the order of the text.

- **Outer values** used inside (spec 6): the wire crosses the edge; a notch marks the crossing.
  No terminal to wire.
- **Nesting**: frames contain each other; the background darkens by one step per level (token
  `frame.fill.n`).

### 5.1 `loop` and `for`

```text
        ┌─ loop ───────────────────────────────┐
 0 V ──┤↻├ v ─────── …               … ── next v ┤↻├───── value
        │                                      │
 err ──┤↻├ e ───›─── …               … ─›─ next e ┤↻├───›── err_out
        │                             until ◆  │
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
- **`until`**: `until` terminal at the bottom right, inside the frame; it is a `bool`, so a
  diamond.
- **Error register**: box in error style; it is a reminder that the loop stops when it contains
  an error (spec 6.3). **`continue_on_error`**: badge in the header, since it changes this default
  behavior.
- **`for parallel`**: header `for parallel`, double-line frame.

### 5.2 `match` and `handle`

- **Selector**: terminal on the left edge, at the level of the header.
- **One branch at a time**: a tab bar in the header lists the branches (`true`, `false`;
  `State.Init`, `State.WaitDut` …; `_` is written `other`). The displayed branch is highlighted.
  A bound value (`Reading.Value(v)`, `io.timeout(err)`) appears as a terminal on the inner left
  edge of that branch.
- **`yield`**: each branch wires the same output terminals on the right edge. A terminal that a
  branch does not wire is impossible (the text forbids it), so there is nothing to flag here.
- **Unfolded view**: all branches stacked vertically, each with its title. This is how exported
  images and printouts are rendered, and an editor mode. Where the output wire starts when each
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
`coercion.fill`, `badge.fill`, `badge.text`, `tab.fill`. Contrast of at least 4.5:1 between each
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
| N-4 | Primitives: closed list of glyphs or extensible by a library? | Closed for `core` in v0 |
| N-5 | Units and origin of the canvas in the `layout` section | **Settled (2026-11-02):** those of section 2, written in spec 11 |
| N-6 | Layout stability when a signature changes (terminals added) | The top-left corner is fixed, the node grows downward |
| N-7 | Does automatic layout keep the error chain straight? | **Yes.** The main error chain occupies a horizontal line, at the bottom of the nodes it crosses; columns follow the order of dependencies. Tested on `check_voltage` and `settle` (`tools/lcf-figcraft`) |
| N-8 | Accessibility: keyboard traversal of the diagram and through AccessKit (D-03) | Traversal order = canonical form of the text (spec 5.2) |
| N-9 | Unfolded view of a `match` or `handle`: where does the output wire start when each branch produces the value? How to draw an outer value used in several branches? | Current `lcf-figcraft` rendering: the output starts from the first branch, the others have a terminal of the same name on the right edge; an outer value passes a notch, then a small vertical bus. To judge on `station` |
| N-10 | Short label of the compact view: always derived from the name, or declarable by the author? | Derived from the name in v0; the icon is enough for anyone who wants better |
| N-11 | An icon shared by several functions of a library | Copied into each file in v0; later, a library icon referenced by its name |
| N-12 | The `tools/lcf-figcraft` rendering draws only the standard view | To complete in order to judge the compact view on `check_voltage`, `settle` and `station` |
| N-13 | Error and order wires fall below 4.5:1 from `frame.fill.2` on (section 8) | Lighten the nesting backgrounds rather than darken the wires |
