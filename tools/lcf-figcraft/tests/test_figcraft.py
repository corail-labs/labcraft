"""Render tests: determinism, completeness (nodes and wires), execution by figcraft-cli."""
import json
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
import lcf_figcraft as app  # noqa: E402
from lcffc import emit, fc, metrics, model  # noqa: E402

EXAMPLES = ROOT.parents[1] / "examples"
NOTATION = app.DEFAULT_NOTATION


_CACHE = {}


def build(fn):
    if fn not in _CACHE:
        _CACHE[fn] = app.build_script(str(EXAMPLES), fn)
    return _CACHE[fn]


def all_nodes_edges(body):
    nodes, edges = list(body.nodes), list(body.edges)
    for n in body.nodes:
        for _, b in n.bodies:
            sn, se = all_nodes_edges(b)
            nodes += sn
            edges += se
    return nodes, edges


def all_routes(L):
    out = list(L.routes)
    for n in L.node_xy:
        for item in getattr(n, "layouts", []):
            out += all_routes(item[0])
    return out


class FigcraftTests(unittest.TestCase):
    def test_deterministic(self):
        for fn in ("check_voltage", "settle"):
            a, _ = app.build_script(str(EXAMPLES), fn)
            b, _ = app.build_script(str(EXAMPLES), fn)
            self.assertEqual(a.dumps().encode("utf-8"), b.dumps().encode("utf-8"), fn)

    def test_every_node_and_edge_is_drawn(self):
        for fn in ("check_voltage", "settle"):
            script, (lib, f, root, body, L) = build(fn)
            names = [s["params"].get("name", "") for s in script.steps if s["command"] in ("node.create", "node.set")]
            text = "\n".join(names)
            nodes, edges = all_nodes_edges(body)
            # every IR node (call, primitive, structure) has its block, circle or frame
            for n in nodes:
                if n.kind == "prim":
                    self.assertIn(f"primitive {n.title}", text)
                elif n.kind == "call":
                    self.assertIn(f"node {n.lib}{n.title}", text)
                elif n.frame_kind in ("loop", "for"):
                    self.assertIn(f"{n.frame_kind} ", text)
                else:
                    self.assertIn(f"{n.frame_kind} {n.title}", text)
            # every wire of the graph has a drawn path
            drawn = sum(1 for nm in names if nm.startswith("wire ") and "(core)" not in nm)
            routed = all_routes(L)
            self.assertEqual(len(routed), len(edges), fn)
            # every wire has a drawn path (buses of outer values and arcs come on top)
            self.assertGreaterEqual(drawn, len(edges), fn)
            # IR: as many top-level nodes as non-alias bindings
            from lcfpoc.ir import Call, Handle, Loop, Match, For, StructLit
            expected = sum(1 for nd in f.body.nodes if isinstance(nd.rhs, (Call, Handle, Loop, Match, For, StructLit)))
            self.assertEqual(len(body.nodes), expected, fn)

    def test_chain_is_straight(self):
        script, (lib, f, root, body, L) = build("check_voltage")
        err_edges = [(e, pts) for e, pts in L.routes if e.family == "error"]
        straight = [pts for e, pts in err_edges if len(pts) == 2 and abs(pts[0][1] - pts[1][1]) < 0.01]
        # err -> open -> query -> close -> output: 4 wires on the axis, no bend
        self.assertGreaterEqual(len(straight), 4)

    # Wire crossings (wires from different sources, all bodies together) measured on commit 0de0473
    # (before barycentric sweeps, loop entry ordering, lane choice by cost): they must not go up again.
    BASELINE = {"check_voltage": 15, "settle": 35}

    def test_crossings_do_not_regress(self):
        for fn, base in self.BASELINE.items():
            _, (lib, f, root, body, L) = build(fn)
            c = metrics.crossings(L)
            self.assertLessEqual(c, base, fn)
            if fn == "settle":
                self.assertLessEqual(c, 20, "settle must stay well below its baseline (35)")

    def test_lanes_follow_terminal_order(self):
        # low is above high on the left edge, and feeds the upper comparator: their wires must not cross
        _, (lib, f, root, body, L) = build("check_voltage")
        routes = {e.src.name: pts for e, pts in L.routes if e.src.owner is root}
        self.assertLess(routes["low"][0][1], routes["high"][0][1])
        self.assertEqual(emit.Emitter.crossing_points(routes["low"], [routes["high"]]), [])

    def test_long_wire_takes_the_shorter_side(self):
        # voltage -> right edge passes under the comparators and frames, not over the whole diagram
        _, (lib, f, root, body, L) = build("check_voltage")
        pts = next(pts for e, pts in L.routes if e.src.name == "voltage" and e.dst.owner is root)
        top = min(y for n, (x, y) in L.node_xy.items())
        self.assertGreater(min(y for x, y in pts), top)

    def test_output_terminals_show_their_unit(self):
        script, _ = build("check_voltage")
        texts = {s["params"].get("name"): s["params"].get("text") for s in script.steps
                 if s["command"] == "node.create" and s["params"].get("type") == "text"}
        self.assertEqual(texts.get("output voltage"), "voltage [V]")
        self.assertEqual(texts.get("output verdict"), "verdict")  # named type: shape + tooltip, no text

    def test_wire_labels_never_collide(self):
        for fn in ("check_voltage", "settle"):
            _, (lib, f, root, body, L) = build(fn)
            self.assertGreater(len(metrics.collect(L)["labels"]), 5, fn)
            self.assertEqual(metrics.label_collisions(L), 0, fn)

    def test_registers_replace_return_arcs(self):
        script, _ = build("settle")
        names = [s["params"].get("name", "") for s in script.steps if s["command"] in ("node.create", "node.set")]
        self.assertFalse([n for n in names if "return arc" in n or "arc legend" in n])
        # two carries (v, e): one box on the left and one on the right each
        regs = [n for n in names if n.startswith("carry")]
        self.assertEqual(len(regs), 4, regs)

    def test_runs_through_figcraft(self):
        with tempfile.TemporaryDirectory() as tmp:
            png, _ = app.render(str(EXAMPLES), "settle", tmp, scale=1)
            self.assertTrue(png.exists() and png.stat().st_size > 1000)
            self.assertTrue((Path(tmp) / "settle.svg").exists())
            self.assertTrue((Path(tmp) / "settle.figcraft").exists())


if __name__ == "__main__":
    unittest.main()
