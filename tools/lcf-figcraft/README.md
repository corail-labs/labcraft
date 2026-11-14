# lcf-figcraft: render a .lcf function as a FigCraft design (throwaway)

Renders a function of a `.lcf` library following `docs/visual-notation.md`, without an LLM: the same `.lcf` always
gives the same JSONL. Currently supported: `read_voltage`, `check_voltage` and `settle` (`station` is out of
scope).

The diagrams of the main README came from this tool until M0v; they are now drawn by
`labcraft-cli diagram --svg --theme light` (and `dark`).

    python tools/lcf-figcraft/lcf_figcraft.py examples --fn check_voltage --out build/

Writes `build/<fn>.figcraft`, `.svg`, `.png` (and `.jsonl`, the script that was run). `--theme Dark` switches the
token mode; `--scale` sets the PNG scale. Requires `figcraft-cli` from FigCraft
(<https://github.com/storytold/figcraft>): variable `FIGCRAFT_CLI`, otherwise the default Windows path
`C:\Program Files\ArtCraft\FigCraft\figcraft-cli.exe`. Also `pip install -r tools/lcf-poc/requirements.txt` for the PoC.

## Architecture

    .lcf -> IR (lcf-poc) -> graph (model.py) -> layout (layout.py) -> JSONL (emit.py)
         -> figcraft-cli run --in design/notation.figcraft --out build/<fn>.figcraft -> export svg / png

- `design/notation.figcraft` (at the repository root): notation library, **editable by hand in FigCraft**. Created once by
  `build_notation.py`; after that the tool only reads it (`notation.py`, through `variable.list`, `page.list`,
  `node.get`). Contents: variable collection `Notation` (modes Light, Dark); components `Terminal` (family x
  direction variants, the center is the anchor point), `Primitive` (property `glyph`), `Pill/constant|field|coercion`,
  `Badge`, `Tab` (property `text`), `Carry` (family variants); samples `Node`, `Structure/frame`,
  `Structure/error frame`, `Structure/notch`, and one `Wire/<family>` per family.
- Components are instantiated (`instance.create` + `instance.setProperties`); colors are variables bound by name
  (`variable.bind`), so dark mode costs nothing.
- Read from the file: all tokens; terminal shapes; disc and glyph of primitives; pills, badge, tab; body, divider,
  header, row pitch, fonts and margins of a node; stroke, border and radius of structures; weight, dashes and color
  of each wire family; pitch and size of chevrons / ticks (in the `pluginData` of the `Wire/*` samples).
- Hard-coded (layout rules): grid of 8, wire corner radius 6, shape of a chevron (`›`) and of a tick, gaps, lanes,
  terminal order, the "pass-through pair" rule, return arcs, level token names (`frame.fill.n`), the "◂" prefix of
  constants.

## Layout (layout.py)

Layers from left to right; the main error chain (longest path of nodes with an `err` pair) is laid on a straight
axis, the other nodes stack above it in their column (N-7). Boundary terminals in signature order (the `err`
terminal on the axis). Orthogonal wires with rounded corners, vertical lanes shared by source, long segments routed
around nodes, junction points. Lanes of a channel are chosen left to right by crossing count; a new lane can open
anywhere in the channel, so nested wires keep the order of their terminals (`low` above `high` stays above, no
crossing). A long segment runs at the height of its ends, in a gap between nodes, above the nodes, or under the lowest
node halfway to the error axis (columns with no chain node): the cheapest in crossings, then in length. Junction
points wherever two wires of the same source and family part, at a corner or as a T on a segment. Chevrons and ticks
are left out at crossings with other wires. Right-edge terminals are labelled like the left ones: name and unit
(`voltage [V]`), never a named type (`verdict`, not `Verdict`: the shape tells the family, notation 4.1). Attached
constants as pills stuck to the terminal, field accesses and coercions as pills on the wire. `loop`: current / next
carry terminals at the same height, return arc above (data) or below (error). `match` / `handle`: unfolded view,
branches stacked with a tab.

## Approximations (to know)

- Types: declared when the IR has them (parameters, outputs, annotated carries, `.lct` fields); outputs of
  primitives by table in `model.py` (`err` error, `session`/`bank`/`source` session, `text` text, comparisons
  boolean, `with_unit` carries the requested unit, arithmetic keeps the unit of the first operand); otherwise number.
  No unit propagated for `mul`/`div`. Coercions: `mV→V` only if both units are known.
- Unfolded view: the output of a `match` / `handle` leaves from the terminal of the first branch; the other
  branches have their terminal of the same name on the right edge.
- An outer value used in several branches: a notch on the frame, then a small vertical bus.
- `select`: condition on the left side (not on top). `for`: minimal support, untested (outside phase 1).
- No tooltips (`///` documentation, exact type): an image has none; the unit is written `[V]`.
- No `+n` when all useful inputs are wired (the case for the examples).
- A wire that leaves the error axis upward (the `e2` of `check_voltage` towards `handle e2`) crosses every wire that
  leaves the same node above it, and every wire going from the left of that node to the region under its run: it is
  forced by the topology (going under the axis would cross the chain instead).
- Text measurements depend on the installed fonts (Segoe UI, Consolas): the render is identical from one run to the
  next on one machine, not necessarily from one machine to another.

## Changing the design

1. Open `design/notation.figcraft` in FigCraft: change a token (collection `Notation`), a `Terminal` shape, a pill,
   a `Wire/*` sample (weight, dashes, color; `pitch` in the pluginData).
2. Rerun the command above. No code to touch as long as the names (variants, layers `stroke`, `body`...) are kept.
   To regenerate the library from scratch: `python tools/lcf-figcraft/build_notation.py`.

Tests: `python -m unittest discover tools/lcf-figcraft/tests` (JSONL identical from one run to the next, every node
and every wire present, execution by figcraft-cli).
