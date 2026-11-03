"""Layered layout, left to right, main error chain straight (notation N-7).

For each body (function, loop, arm):
    1. node sizes (frames are laid out first, recursively);
    2. layers = columns (length of the longest path); the main error chain is the longest path of
          nodes with a pass-through err pair; their `err` rows are all at y = 0 (the axis), the other nodes stack
          above, in the same column;
    3. y of the boundary terminals (parameters, carries, outputs), in signature order when it matters;
    4. channels between columns: vertical lanes (one per wire, shared by wires from the same source), then x;
    5. orthogonal wires (one H-V-H bend, or H-V-H-V-H for a wire that skips columns).
Coordinates of a body are local to its frame; `emit.py` translates them.
"""
import math

from .model import ENUM, ERROR, ORDER, SESSION, Port

GRID = 8
LANE_CLEAR = 40  # gap between the error axis and the bottom of off-chain nodes
STACK_GAP = 24
FRAME_PAD = 16
HEADER = 30  # header band of a frame
ARM_GAP = 12
SPACING = 28  # minimum gap between boundary terminals


def up8(x):
    return int(math.ceil(x / GRID - 1e-9)) * GRID


class BodyLayout:
    def __init__(self, body):
        self.body = body
        self.w = self.h = 0.0
        self.pos = {}  # uid -> local (x, y), all ports visible in this body
        self.node_xy = {}      # GNode -> (x, y)
        self.routes = []       # (edge, points)
        self.labels = []       # (texte, x, y_haut, famille)
        self.label_boxes = []  # (texte, x, y_haut, famille, largeur, hauteur)
        self.edge_pills = []   # (texte, genre, x_droit, y_centre)
        self.junctions = []    # (x, y, famille)
        self.arcs = []
        self.children = {}     # GNode -> [(BodyLayout, ox, oy)]
        self.axis = 0.0
        self.header = HEADER
        self.left_zone = self.right_zone = 0.0
        self.notches = []  # (y) notches on the left edge
        self.entries = []  # boundary source ports
        self.sinks = []


class Layouter:
    def __init__(self, notation):
        self.N = notation
        self.hints = {}  # uid of a frame boundary source -> y of the outer source (for entry ordering)

    def hint_of(self, p):
        return self.hints.get(p.uid)

    def unsize(self, n):
        n.sized = False
        for _, b in n.bodies:
            for m in b.nodes:
                self.unsize(m)

    def update_hints(self, body, L):
        """y of the outer sources of this body's frames; True if a hint changed."""
        changed = False
        nodes = set(body.nodes)
        for e in body.edges:
            f = e.dst.owner
            if f in nodes and f.kind == "frame" and e.src.uid in L.pos:
                inner = e.dst.mirror or e.dst
                y = round(L.pos[e.src.uid][1], 1)
                if self.hints.get(inner.uid) != y:
                    self.hints[inner.uid] = y
                    changed = True
        return changed

    # ------------------------------------------------------------------------------------------------
    # Mesures

    def tw(self, style, text):
        return self.N.measure(style, text)[0]

    def pill_w(self, text):
        return self.tw(self.N.pill_text, text) + self.N.pill_pad

    def label_w(self, p):
        w = self.tw(self.N.st["input row"], p.label)
        if p.ty.unit and p.role in ("in", "out", "entry", "exit"):
            w += 6 + self.tw(self.N.st["unit"], f"[{p.ty.unit}]")
        return w

    def wire_label(self, p):
        return p.name + (f" [{p.ty.unit}]" if p.ty.unit else "")

    def sink_label(self, p):
        """Label of a right-edge terminal: like the inputs, the unit is written (`voltage [V]`), a named type is not
        (its family is in the shape of the terminal, the exact type in the tooltip, notation 4.1-4.2)."""
        return p.label + (f" [{p.ty.unit}]" if p.label and p.ty.unit else "")

    # ------------------------------------------------------------------------------------------------
    # Nodes

    def size_node(self, n):
        if getattr(n, "w", None) is not None and n.__dict__.get("sized"):
            return
        n.sized = True
        n.off = {}      # uid -> (dx, dy)
        n.ext_top = n.ext_bottom = 0.0
        N = self.N
        if n.kind == "prim":
            d = N.prim_d
            n.w = n.h = d
            k = len(n.ins)
            offs = {1: [0], 2: [-8, 8], 3: [-10, 0, 10]}.get(k) or [(-10 + 20 * i / max(k - 1, 1)) for i in range(k)]
            for p, dy in zip(n.ins, offs):
                dx = d / 2 - math.sqrt(max((d / 2) ** 2 - dy * dy, 0))
                n.off[p.uid] = (dx, d / 2 + dy)
            for p in n.outs:
                n.off[p.uid] = (d, d / 2)
            n.err_row = None
            return
        if n.kind == "frame":
            self.size_frame(n)
            return
        self.size_block(n)

    def size_block(self, n):
        N = self.N
        ins, outs = n.ins, n.outs
        pairs = {}
        for fam in (ERROR, SESSION):
            ii = [i for i, p in enumerate(ins) if p.family == fam]
            oo = [o for o, p in enumerate(outs) if p.family == fam]
            for i, o in zip(ii, oo):
                pairs[i] = o
        # crossed pairings: keep the longest increasing run (simple: cut at the first crossing)
        last, clean = -1, {}
        for i in sorted(pairs):
            if pairs[i] > last:
                clean[i] = pairs[i]
                last = pairs[i]
        rows = self.assign_rows(len(ins), len(outs), clean)
        n.rows = rows
        title = n.lib + n.title
        tw_title = self.tw(N.st["title"], title)
        head = N.pad_x + tw_title + 14
        if n.always:
            head += 8 + self.tw(N.badge_text, "always") + N.badge_pad
        if n.hidden:
            head += 8 + self.tw(N.st["unit"], f"+{n.hidden}")
        if n.after is not None:
            head += 16
        wrow = 0
        for ri, ro in rows:
            lw = rw = 0
            if ri is not None:
                p = ins[ri]
                lw = self.label_w(p) + (8 + self.pill_w("◂ " + p.pill) if p.pill else 0)
            if ro is not None:
                rw = self.label_w(outs[ro])
            wrow = max(wrow, N.pad_x * 2 + lw + rw + (24 if lw and rw else 0))
        n.w = up8(max(head + N.pad_x, wrow, 112))
        nrows = max(len(rows), 1)
        n.h = N.row0_center + (nrows - 1) * N.row_pitch + N.node_bottom
        n.err_row = None
        for k, (ri, ro) in enumerate(rows):
            y = N.row0_center + k * N.row_pitch
            if ri is not None:
                n.off[ins[ri].uid] = (0, y)
            if ro is not None:
                n.off[outs[ro].uid] = (n.w, y)
            if ri is not None and ro is not None and ins[ri].family == ERROR and outs[ro].family == ERROR:
                n.err_row = y
        if n.after is not None:
            n.off[n.after.uid] = (0, N.header_h / 2)
        if n.label:
            n.ext_top = 18

    @staticmethod
    def assign_rows(ni, no, pairs):
        inv = {o: i for i, o in pairs.items()}
        rows, i, o = [], 0, 0
        guard = 0
        while (i < ni or o < no) and guard < 1000:
            guard += 1
            hi = i < ni
            ho = o < no
            if hi and i in pairs:
                t = pairs[i]
                if ho and o == t:
                    rows.append((i, o)); i += 1; o += 1
                    continue
                if ho and o < t:
                    rows.append((None, o)); o += 1
                    continue
            if ho and o in inv:
                if hi and i < inv[o]:
                    rows.append((i, None)); i += 1
                    continue
            if hi and ho and i not in pairs and o not in inv:
                rows.append((i, o)); i += 1; o += 1
            elif hi and i not in pairs:
                rows.append((i, None)); i += 1
            elif ho and o not in inv:
                rows.append((None, o)); o += 1
            elif hi:
                rows.append((i, None)); i += 1
            else:
                rows.append((None, o)); o += 1
        return rows

    # ------------------------------------------------------------------------------------------------
    # Cadres (structures)

    def size_frame(self, n):
        N = self.N
        n.err_row = None
        if n.frame_kind in ("loop", "for"):
            body = n.bodies[0][1]
            L = self.layout_body(body, HEADER, owner=n, title=n.title)
            n.child = [(L, 0, 0)]
            n.layouts = [(L, 0, 0)]
            n.w, n.h = L.w, L.h
            for p in L.entries:
                m = p.mirror or p
                n.off[p.uid] = L.pos[p.uid]
                if p.mirror is not None:
                    n.off[p.mirror.uid] = L.pos[p.uid]
            for p, outer in n.entries.items():
                if outer.uid in L.pos:
                    n.off[outer.uid] = L.pos[outer.uid]
            for out in n.outs:
                if out.inner is not None and out.inner.uid in L.pos:
                    n.off[out.uid] = L.pos[out.inner.uid]
            # return arcs: above (data), below (error)
            ncar = [c for c in body.entries if c.role == "carry"]
            err = [c for c in ncar if c.family == ERROR]
            oth = [c for c in ncar if c.family != ERROR]
            n.arc_data = oth
            n.arc_err = err
            n.ext_top = n.ext_bottom = 0.0  # no return arc in exports (the carry carries the information)
            # error row of the chain: the error carry
            for c in err:
                n.err_row = L.pos[c.uid][1]
            if n.err_row is not None:
                n.err_in = n.entries.get(("carry", err[0].name))
                n.err_out = next((o for o, f in zip(n.outs, [x.pair for x in ncar] if False else []) if False), None)
                idx = [c.name for c in ncar].index(err[0].name)
                n.err_out = n.outs[idx] if idx < len(n.outs) else None
            return
        # match / handle: stacked arms, unfolded view
        arms = n.bodies
        tab_h = N.tab_h
        arm_layouts = []
        maxw = 0
        for title, ab in arms:
            L = self.layout_body(ab, tab_h + 14, owner=n, title=title)
            arm_layouts.append((title, L))
            maxw = max(maxw, L.w)
        n.hdr = HEADER
        n.pad = FRAME_PAD
        y = n.hdr + 6
        placed = []
        for title, L in arm_layouts:
            placed.append((L, n.pad, y))
            y += L.h + ARM_GAP
        n.w = up8(max(maxw + 2 * n.pad, self.tw(N.frame_style["Structure/frame"]["title"], n.title) + 120))
        # the arms take the full width of the frame
        arm_layouts = [(t, self.layout_body(ab, tab_h + 14, owner=n, title=t, min_w=n.w - 2 * n.pad))
                       for t, ab in arms]
        placed = [(L, n.pad, oy) for (L, _, oy), (t, L) in zip(placed, arm_layouts)]
        n.layouts = placed
        n.arm_titles = [t for t, _ in arm_layouts]
        n.h = y - ARM_GAP + n.pad // 2 + 6
        # frame terminals
        n.off[n.sel_port.uid] = (0, n.hdr / 2 + 2)
        n.bus = []
        ents = {}
        for k, (L, ox, oy) in enumerate(placed):
            for p in L.entries:
                if p.role == "entry" and p.owner is n and p.uid in L.pos:
                    ents.setdefault(p.uid, []).append((p, ox + L.pos[p.uid][0], oy + L.pos[p.uid][1]))
        for uid, lst in ents.items():
            p, x0, y0 = lst[0]
            n.off[uid] = (0, y0)
            xs = n.pad / 2
            for _, xk, yk in lst:
                n.bus.append((p, [(0, y0), (xs, y0), (xs, yk), (xk, yk)]))
        for out in n.outs:
            if out.inner is not None:
                for (L, ox, oy) in placed[:1]:
                    if out.inner.uid in L.pos:
                        n.off[out.uid] = (ox + L.pos[out.inner.uid][0], oy + L.pos[out.inner.uid][1])
        n.arm_layouts = placed

    # ------------------------------------------------------------------------------------------------
    # Corps

    def boundary_sources(self, body, owner):
        ents = list(body.entries)
        seen = {p.uid for p in ents}
        for e in body.edges:
            p = e.src
            if p.owner is owner and p.uid not in seen:
                ents.append(p)
                seen.add(p.uid)
        return ents

    def err_ports(self, n):
        """(input, output) of the pass-through error pair of a node, or None."""
        if n.kind == "frame":
            ei, eo = getattr(n, "err_in", None), getattr(n, "err_out", None)
            return (ei, eo) if ei is not None and eo is not None else None
        if n.err_row is None:
            return None
        i = next((p for p in n.ins if p.family == ERROR and n.off[p.uid][1] == n.err_row), None)
        o = next((p for p in n.outs if p.family == ERROR and n.off.get(p.uid, (0, -1))[1] == n.err_row), None)
        return (i, o) if i is not None and o is not None else None

    def layout_body(self, body, header, owner=None, title="", root=False, min_w=0):
        """Several variants of initial order / sweep; keep the one with the fewest crossings."""
        from .metrics import crossings

        def run(variant):
            L = self._layout_body(body, header, owner, title, root, min_w, variant)
            for _ in range(2):
                if not self.update_hints(body, L):
                    break
                for n in body.nodes:
                    if n.kind == "frame":
                        self.unsize(n)
                L = self._layout_body(body, header, owner, title, root, min_w, variant)
            return L

        best_v, best_c, last = 0, None, 0
        for variant in range(4):
            L = run(variant)
            last = variant
            c = crossings(L)
            if best_c is None or c < best_c:
                best_v, best_c = variant, c
            if best_c == 0:
                break
        if best_v != last:
            L = run(best_v)
        return L

    def _layout_body(self, body, header, owner=None, title="", root=False, min_w=0, variant=0):
        N = self.N
        L = BodyLayout(body)
        L.header = header
        nodes = list(body.nodes)
        nset = set(nodes)
        for n in nodes:
            self.size_node(n)
        ents = self.boundary_sources(body, owner if owner is not None else body.owner)
        sinks = list(body.sinks)
        L.entries, L.sinks = ents, sinks
        edges = list(body.edges)

        def node_of(p):
            return p.owner if p.owner in nset else None

        # --- couches -----------------------------------------------------------------------------------
        preds = {n: [] for n in nodes}
        for e in edges:
            a, b = node_of(e.src), node_of(e.dst)
            if a is not None and b is not None and a is not b:
                preds[b].append(a)
        layer = {}

        def lay(n, stack=()):
            if n in layer:
                return layer[n]
            if n in stack:
                return 0
            layer[n] = 0
            layer[n] = max([lay(p, stack + (n,)) + 1 for p in preds[n]] or [0])
            return layer[n]

        for n in nodes:
            lay(n)
        ncol = (max(layer.values()) + 1) if nodes else 0

        # main error chain ------------------------------------------------------------------------------------
        pair = {n: self.err_ports(n) for n in nodes}
        nxt = {}
        boundary_start = {}
        for e in edges:
            a, b = node_of(e.src), node_of(e.dst)
            if e.family != ERROR:
                continue
            if b is not None and pair.get(b) and e.dst is pair[b][0]:
                if a is None:
                    boundary_start.setdefault(b, e.src)
                elif pair.get(a) and e.src is pair[a][1]:
                    nxt.setdefault(a, []).append(b)
        best = []

        def longest(n, seen=()):
            opts = [longest(m, seen + (n,)) for m in nxt.get(n, []) if m not in seen]
            tail = max(opts, key=lambda c: len(c), default=[])
            return [n] + tail

        for n in sorted(boundary_start, key=lambda m: m.line):
            c = longest(n)
            if len(c) > len(best):
                best = c
        chain = best
        chain_set = set(chain)
        # the chain node must be in a column strictly after its chain predecessor: already true (layers)

        # boundary terminals of the chain ---------------------------------------------------------------------
        chain_src = boundary_start.get(chain[0]) if chain else None
        chain_sink = None
        if chain:
            last_out = pair[chain[-1]][1]
            for e in edges:
                if e.src is last_out and node_of(e.dst) is None and e.dst in sinks:
                    chain_sink = e.dst
        ordered_sig = (body.owner is not None and body.owner.frame_kind == "fn")
        # rank of boundary sources: signature, else order of outer sources (hints), else declared order
        ranked = list(ents)
        if not ordered_sig and any(self.hint_of(p) is not None for p in ents):
            ranked.sort(key=lambda p: (self.hint_of(p) if self.hint_of(p) is not None else 1e9, ents.index(p)))
        rank_e = {p.uid: i for i, p in enumerate(ranked)}
        rank_s = {p.uid: i for i, p in enumerate(sinks)}
        ref_e = rank_e[chain_src.uid] if chain_src is not None else len(ranked) - 1
        ref_s = rank_s[chain_sink.uid] if chain_sink is not None else len(sinks) - 1
        pseudo = {}
        for p in ents:
            pseudo[p.uid] = (ref_e - rank_e[p.uid]) * 28.0
        for p in sinks:
            pseudo[p.uid] = (ref_s - rank_s[p.uid]) * 28.0
        # columns and stack (barycentric sweeps, chain pinned on the axis) --------------------------------------------
        cols = [[] for _ in range(ncol)]
        for n in nodes:
            cols[layer[n]].append(n)
        succs = {n: [] for n in nodes}
        for n in nodes:
            for q in preds[n]:
                succs[q].append(n)
        in_ent = {n: [] for n in nodes}
        out_snk = {n: [] for n in nodes}
        for e in edges:
            a_, b_ = node_of(e.src), node_of(e.dst)
            if a_ is None and b_ is not None:
                in_ent[b_].append(e.src)
            if b_ is None and a_ is not None:
                out_snk[a_].append(e.dst)
        d_center = {}  # n -> d of the middle (positive upward)
        d_bottom, d_top = {}, {}

        def stack(orders):
            for c, col in enumerate(cols):
                chain_n = next((n for n in col if n in chain_set), None)
                if chain_n is not None:
                    er = chain_n.err_row
                    d_top[chain_n] = er
                    d_bottom[chain_n] = er - chain_n.h
                    d_center[chain_n] = er - chain_n.h / 2
                    cur = d_top[chain_n] + getattr(chain_n, "ext_top", 0) + STACK_GAP
                else:
                    cur = LANE_CLEAR
                for n in orders[c]:
                    cur += getattr(n, "ext_bottom", 0)
                    d_bottom[n] = cur
                    d_top[n] = cur + n.h
                    d_center[n] = cur + n.h / 2
                    cur = d_top[n] + n.ext_top + STACK_GAP

        def score():
            tot = 0.0
            for e in edges:
                a_, b_ = node_of(e.src), node_of(e.dst)
                ya = d_center[a_] if a_ is not None else pseudo[e.src.uid]
                yb = d_center[b_] if b_ is not None else pseudo[e.dst.uid]
                tot += abs(ya - yb)
            return tot

        sg = -1 if variant % 2 == 0 else 1
        orders = [sorted([n for n in col if n not in chain_set], key=lambda n: (sg * n.line, sg * n.uid)) for col in cols]
        stack(orders)
        best_s, best_o = score(), [list(o) for o in orders]
        for _ in range(4):
            for direction in ((1, -1) if variant < 2 else (-1, 1)):
                rng = range(ncol) if direction == 1 else range(ncol - 1, -1, -1)
                for c in rng:
                    def key(n):
                        if direction == 1:
                            vals = [d_center[q] for q in preds[n]] + [pseudo[p.uid] for p in in_ent[n]]
                        else:
                            vals = [d_center[q] for q in succs[n]] + [pseudo[p.uid] for p in out_snk[n]]
                        return sum(vals) / len(vals) if vals else d_center[n]
                    keys = {n: key(n) for n in orders[c]}
                    orders[c].sort(key=lambda n: (keys[n], sg * n.line, sg * n.uid))
                    stack(orders)
            sc = score()
            if sc < best_s - 1e-6:
                best_s, best_o = sc, [list(o) for o in orders]
        orders = best_o
        stack(orders)
        top_d = max([d_top[n] + n.ext_top for n in nodes] or [0])
        low_d = min([d_bottom[n] - n.ext_bottom for n in nodes] or [0])
        L.axis_d = 0

        def y_of_port(p):
            n = node_of(p)
            dx, dy = n.off[p.uid]
            return -d_top[n] + dy  # local y to the axis (axis = 0, upward negative)

        # boundary terminals: y --------------------------------------------------------------------------------
        out_pref = {}
        for e in edges:
            a, b = node_of(e.src), node_of(e.dst)
            if a is None and b is not None:
                out_pref.setdefault(e.src.uid, []).append(y_of_port(e.dst))
            if b is None and a is not None:
                out_pref.setdefault(e.dst.uid, []).append(y_of_port(e.src))
        pin = {}
        if chain_src is not None:
            pin[chain_src.uid] = 0.0
        if chain_sink is not None:
            pin[chain_sink.uid] = 0.0
        # an error carry: both its terminals are on the axis, even with no chain node
        for p in ents:
            if p.role == "carry" and p.family == ERROR and p.pair is not None and chain:
                pin.setdefault(p.uid, 0.0)
                pin.setdefault(p.pair.uid, 0.0)
        if not chain:
            for p in ents:
                if p.role == "carry" and p.family == ERROR:
                    pin.setdefault(p.uid, 0.0)
                    pin.setdefault(p.pair.uid, 0.0)

        def pref(p):
            v = out_pref.get(p.uid)
            return sum(v) / len(v) if v else None

        # pairs (carries): same height
        pair_y = {}
        for p in ents:
            if p.role == "carry" and p.pair is not None:
                if p.uid in pin:
                    pair_y[p.uid] = pin[p.uid]
                else:
                    vals = [v for v in (pref(p), pref(p.pair)) if v is not None]
                    pair_y[p.uid] = sum(vals) / len(vals) if vals else 0.0
        ys = {}
        # inputs: only the axis terminals are fixed
        items = []
        for k, p in enumerate(ents):
            pr = pair_y.get(p.uid, pref(p))
            items.append([p, pr if pr is not None else 0.0, p.uid in pin, k])
        if not ordered_sig:
            hinted = [it for it in items if self.hint_of(it[0]) is not None and not it[2]]
            slots_ = sorted(it[1] for it in hinted)
            for it, v in zip(sorted(hinted, key=lambda it: (self.hint_of(it[0]), it[3])), slots_):
                it[1] = v
        self.resolve_ys(items, ordered_sig and not any(p.role != "entry" for p in ents), ys, pin, {})
        for p in ents:
            if p.pair is not None:
                pair_y[p.uid] = ys[p.uid]
        # outputs: a carry keeps the height of its current terminal
        items = []
        for k, p in enumerate(sinks):
            if p.pair is not None and p.pair.uid in pair_y:
                pr, fixed = pair_y[p.pair.uid], True
                pin[p.uid] = pr
            else:
                pr, fixed = pref(p), p.uid in pin
                if p.uid in pin:
                    pr = pin[p.uid]
            items.append([p, pr if pr is not None else 0.0, fixed, k])
        self.resolve_ys(items, ordered_sig, ys, pin, {})
        for p in ents:
            if p.mirror is not None and p.mirror.uid not in ys:
                ys[p.mirror.uid] = ys[p.uid]
        # sizes of the left / right zones (labels of boundary terminals) ----------------------------------------
        zone_l = 0.0
        for p in ents:
            w = self.tw(N.st["input row"], self.wire_label(p)) + 16 + (N.carry_w / 2 if p.role == "carry" else 0)
            if p.pill:
                w = w + self.pill_w("◂ " + p.pill) + 30
            zone_l = max(zone_l, w)
            if p.mirror is not None and p.mirror.pill:
                zone_l = max(zone_l, self.pill_w(p.mirror.pill) + 14)
        zone_r = 0.0
        for p in sinks:
            w = self.tw(N.st["output row"], self.sink_label(p)) + 18 + (N.carry_w / 2 if p.role == "next" else 0)
            if p.pill:
                w = max(w, self.pill_w(p.pill) + 18)
            zone_r = max(zone_r, w)
        # --- canaux, pistes, x ---------------------------------------------------------------------------------
        col_w = [max([n.w for n in col] or [0]) for col in cols]

        def col_of_src(p):
            n = node_of(p)
            return layer[n] if n is not None else -1

        def col_of_dst(p):
            n = node_of(p)
            return layer[n] if n is not None else ncol

        # y of the ends
        def py(p):
            n = node_of(p)
            if n is not None:
                return y_of_port(p)
            return ys.get(p.uid, 0.0)

        # obstacles (boxes local in y; the y of a node is -d_top .. -d_bottom)
        boxes = {c: [(-d_top[n] - n.ext_top, -d_bottom[n] + n.ext_bottom, n) for n in col] for c, col in enumerate(cols)}

        # wire plan: y of long segments (by crossing cost), then lanes ----------------------------------------------
        info = {}
        for e in edges:
            info[e] = (col_of_src(e.src), col_of_dst(e.dst), py(e.src), py(e.dst))
        # approximate geometry: channels of 48, to estimate crossings
        cwp = {}
        xx = 0.0
        colx_p = {}
        for c in range(-1, ncol):
            cwp[c] = xx + 24  # center of channel c
            xx += 48
            if c + 1 < ncol:
                colx_p[c + 1] = xx
                xx += col_w[c + 1]
        colx_p[ncol] = xx

        def poly(e, yr):
            a, b, y1, y2 = info[e]
            xs = (colx_p[a] + col_w[a]) if a >= 0 else 0.0
            xd = colx_p[b] if b < ncol else xx
            if b <= a + 1:
                return [(xs, y1), (cwp[a], y1), (cwp[a], y2), (xd, y2)]
            return [(xs, y1), (cwp[a], y1), (cwp[a], yr), (cwp[b - 1], yr), (cwp[b - 1], y2), (xd, y2)]

        def cross_count(P, Q):
            n = 0
            for A, B in ((P, Q), (Q, P)):
                for (a0, a1) in zip(A, A[1:]):
                    if abs(a0[1] - a1[1]) > 0.01:
                        continue
                    for (b0, b1) in zip(B, B[1:]):
                        if abs(b0[0] - b1[0]) > 0.01:
                            continue
                        if min(a0[0], a1[0]) + 0.5 < b0[0] < max(a0[0], a1[0]) - 0.5 and                                 min(b0[1], b1[1]) + 0.5 < a0[1] < max(b0[1], b1[1]) - 0.5:
                            n += 1
            return n

        def verts_for(yr_of):
            verticals = {}   # canal -> [(edge, kind, ya, yb, y_entrant, y_sortant)]
            for e in edges:
                a, b, y1, y2 = info[e]
                yr = yr_of[e]
                if b == a + 1 or b <= a:
                    if abs(y1 - y2) > 0.01:
                        verticals.setdefault(a, []).append((e, 0, min(y1, y2), max(y1, y2), y1, y2))
                else:
                    if abs(yr - y1) > 0.01:
                        verticals.setdefault(a, []).append((e, 0, min(y1, yr), max(y1, yr), y1, yr))
                    if abs(yr - y2) > 0.01:
                        verticals.setdefault(b - 1, []).append((e, 1, min(yr, y2), max(yr, y2), yr, y2))
            return verticals

        def assign(verticals):
            """Lanes of each channel, left to right. A vertical joins a free lane or opens a new one at any
            position (not only on the right), so two nested wires can keep their terminal order."""
            slots, nslots = {}, {}
            for ch, vs in verticals.items():
                vs = sorted(vs, key=lambda v: (-(v[3] - v[2]), v[2], v[0].src.uid, v[0].dst.uid))
                occ = []  # per lane: list of verticals
                for v in vs:
                    e, kind, ya, yb, yin, yout = v
                    best, bmove = None, None
                    # (join, k): into lane k; (insert, k): new lane before lane k
                    moves = [("join", k) for k in range(len(occ))] + [("insert", k) for k in range(len(occ), -1, -1)]
                    for how, sl in moves:
                        if how == "join" and not all((yb + 4 < o[2] or ya - 4 > o[3]) or o[0].src.uid == e.src.uid
                                                     for o in occ[sl]):
                            continue
                        cost = 0
                        for s2, lst in enumerate(occ):
                            if how == "join" and s2 == sl:
                                continue
                            for o in lst:
                                if o[0].src.uid == e.src.uid:
                                    continue
                                left, right = (o, v) if s2 < sl else (v, o)
                                if left[2] < right[4] < left[3]:
                                    cost += 1
                                if right[2] < left[5] < right[3]:
                                    cost += 1
                        cost += 0.01 * (sl if how == "join" else len(occ)) + (0.001 * (len(occ) - sl) if how == "insert" else 0)
                        if best is None or cost < best - 1e-9:
                            best, bmove = cost, (how, sl)
                    how, sl = bmove
                    if how == "insert":
                        occ.insert(sl, [])
                    occ[sl].append(v)
                for k, lst in enumerate(occ):
                    for v in lst:
                        slots[(v[0], v[1])] = k
                nslots[ch] = len(occ)
            return slots, nslots

        def poly2(e, yr, slots):
            a, b, y1, y2 = info[e]
            xs = (colx_p[a] + col_w[a]) if a >= 0 else 0.0
            xd = colx_p[b] if b < ncol else xx
            pts = [(xs, y1)]
            if b <= a + 1:
                if abs(y1 - y2) > 0.01:
                    cx = cwp[a] - 16 + 8 * slots[(e, 0)]
                    pts += [(cx, y1), (cx, y2)]
            else:
                if abs(yr - y1) > 0.01:
                    cx = cwp[a] - 16 + 8 * slots[(e, 0)]
                    pts += [(cx, y1), (cx, yr)]
                if abs(yr - y2) > 0.01:
                    cx = cwp[b - 1] - 16 + 8 * slots[(e, 1)]
                    pts += [(cx, yr), (cx, y2)]
            pts.append((xd, y2))
            return pts

        def total_cost(yr_of):
            slots, nslots = assign(verts_for(yr_of))
            polys = {e: poly2(e, yr_of[e], slots) for e in edges}
            tot = 0
            for i, e in enumerate(edges):
                for o in edges[i + 1:]:
                    if o.src.uid != e.src.uid:
                        tot += cross_count(polys[e], polys[o])
            return tot * 10 + 0.5 * sum(nslots.values())

        yr_of = {e: info[e][2] for e in edges}
        longs = sorted([e for e in edges if info[e][1] > info[e][0] + 1], key=lambda e: (-(info[e][1] - info[e][0]),
                                                                                          e.src.uid, e.dst.uid))
        for _round in range(2):
            for e in longs:
                a, b, y1, y2 = info[e]
                chain_edge = abs(y1) < 0.5 and abs(y2) < 0.5
                best, besty = None, yr_of[e]
                for y in self.run_candidates(boxes, a, b, y1, y2):
                    cost = 0.0
                    if not chain_edge and abs(y) < 14:
                        cost += 1e6
                    for c in range(a + 1, b):
                        for top, bot, nn in boxes.get(c, []):
                            if top - 6 <= y <= bot + 6:
                                cost += 1e4
                    for o in longs:
                        if o is e or o.src.uid == e.src.uid:
                            continue
                        oa, ob = info[o][0], info[o][1]
                        if oa < b and a < ob and abs(yr_of[o] - y) < 18:
                            cost += 40
                    cost += 0.3 * (abs(y - y1) + abs(y - y2))
                    if best is not None and cost >= best:
                        continue
                    yr_of[e] = y
                    cost += total_cost(yr_of)
                    if best is None or cost < best - 1e-9:
                        best, besty = cost, y
                yr_of[e] = besty
            # lane swaps between two long wires (a single move does not undo an inversion)
            cur = total_cost(yr_of)
            for i, e1 in enumerate(longs):
                for e2 in longs[i + 1:]:
                    if e1.src.uid == e2.src.uid or yr_of[e1] == yr_of[e2]:
                        continue
                    y1_, y2_ = yr_of[e1], yr_of[e2]
                    ok_ = True
                    for ee, yy in ((e1, y2_), (e2, y1_)):
                        a_, b_ = info[ee][0], info[ee][1]
                        if any(top - 6 <= yy <= bot + 6 for c in range(a_ + 1, b_) for top, bot, _ in boxes.get(c, [])):
                            ok_ = False
                        if abs(yy) < 14 and not (abs(info[ee][2]) < 0.5 and abs(info[ee][3]) < 0.5):
                            ok_ = False
                    if not ok_:
                        continue
                    yr_of[e1], yr_of[e2] = y2_, y1_
                    c2 = total_cost(yr_of)
                    if c2 < cur - 1e-9:
                        cur = c2
                    else:
                        yr_of[e1], yr_of[e2] = y1_, y2_
        plan = {}
        for e in edges:
            a, b, y1, y2 = info[e]
            plan[e] = (a, b, y1, y2, yr_of[e])
        slots, nslots = assign(verts_for(yr_of))
        # channel zones: source labels (start) and wire pills (end)
        lab_zone, tail_zone = {}, {}
        for e, (a, b, y1, y2, yr) in plan.items():
            lw = self.tw(N.st["wire name"], self.wire_label(e.src)) + 14
            lab_zone[a] = max(lab_zone.get(a, 0), lw)
            tail = 0.0
            for txt in self.edge_pill_texts(e):
                tail += self.pill_w(txt) + 4
            tail = max(tail + 10, 0)
            if e.dst.pill:
                tail = max(tail, self.pill_w(e.dst.pill) + 12)
            ch_end = max(b - 1, a)
            tail_zone[ch_end] = max(tail_zone.get(ch_end, 0), tail)
        for n in nodes:
            c = layer[n]
            for p in n.ins:
                if p.pill and n.kind != "call":
                    tail_zone[c - 1] = max(tail_zone.get(c - 1, 0), self.pill_w(p.pill) + 12)
        lab_zone[-1] = max(lab_zone.get(-1, 0), zone_l)
        tail_zone[ncol - 1] = max(tail_zone.get(ncol - 1, 0), zone_r)
        if ncol == 0:
            tail_zone[-1] = max(tail_zone.get(-1, 0), zone_r)
        chw = {}
        for ch in range(-1, ncol):
            ns = nslots.get(ch, 0)
            chw[ch] = max(40 if ch >= 0 else 24, lab_zone.get(ch, 0) + tail_zone.get(ch, 0) + 8 * ns + 16)
            if ch in (-1, ncol - 1) and ns == 0:
                chw[ch] = max(chw[ch], lab_zone.get(ch, 0) + tail_zone.get(ch, 0) + 16)
            chw[ch] = up8(chw[ch])
        x = 0.0
        ch_left = {}
        col_x = {}
        for ch in range(-1, ncol):
            ch_left[ch] = x
            x += chw[ch]
            if ch + 1 < ncol:
                col_x[ch + 1] = x
                x += col_w[ch + 1]
        W = max(x, min_w)
        # --- positions finales -------------------------------------------------------------------------------------
        top_y = -(top_d)
        low_y = -low_d
        # edge: header above the first terminal or node
        all_y = [y for y in ys.values()] + [-d_top[n] - n.ext_top for n in nodes] + [
            yr - 20 for (a_, b_, y1_, y2_, yr) in plan.values() if b_ > a_ + 1]
        miny = min(all_y + [0.0])
        maxy = max([-d_bottom[n] + n.ext_bottom for n in nodes] + list(ys.values()) + [LANE_CLEAR * 0.0])
        shift = header + 12 - miny
        H = maxy + shift + (28 if chain or any(p.role == "carry" for p in ents) else 20)
        L.axis = shift
        L.w, L.h = W, H

        def X(c, s, kind):
            return ch_left[c] + lab_zone.get(c, 0) + 8 + 8 * s

        for n in nodes:
            nx = col_x[layer[n]]
            ny = -d_top[n] + shift
            L.node_xy[n] = (nx, ny)
            for uid, (dx, dy) in n.off.items():
                L.pos[uid] = (nx + dx, ny + dy)
            if n.kind == "frame":
                for out in n.outs:
                    if out.uid in n.off:
                        L.pos[out.uid] = (nx + n.off[out.uid][0], ny + n.off[out.uid][1])
        for p in ents:
            L.pos[p.uid] = (0.0, ys.get(p.uid, 0.0) + shift)
            if p.mirror is not None:
                L.pos[p.mirror.uid] = L.pos[p.uid]
        for p in sinks:
            L.pos[p.uid] = (W, ys.get(p.uid, 0.0) + shift)
        # fils
        for e, (a, b, y1, y2, yr) in plan.items():
            xs, ys_ = L.pos[e.src.uid], L.pos[e.dst.uid]
            # start / end x: the terminal; lanes are in the channels
            pts = [xs]
            if b == a + 1 or b <= a:
                if abs(y1 - y2) > 0.01:
                    cx = X(a, slots[(e, 0)], 0)
                    pts += [(cx, xs[1]), (cx, ys_[1])]
            else:
                if abs(yr - y1) > 0.01:
                    cx1 = X(a, slots[(e, 0)], 0)
                    pts += [(cx1, xs[1]), (cx1, yr + shift)]
                if abs(yr - y2) > 0.01:
                    cx2 = X(b - 1, slots[(e, 1)], 1)
                    pts += [(cx2, yr + shift), (cx2, ys_[1])]
            pts.append(ys_)
            e.points = self.clean(pts)
            L.routes.append((e, e.points))
        self.decorate(L, edges, plan, ncol, ch_left, lab_zone, tail_zone, chw)
        L.chain = chain
        L.shift = shift
        return L

    # ------------------------------------------------------------------------------------------------

    @staticmethod
    def clean(pts):
        out = []
        for p in pts:
            if out and abs(out[-1][0] - p[0]) < 0.01 and abs(out[-1][1] - p[1]) < 0.01:
                continue
            out.append(p)
        i = 1
        while i < len(out) - 1:
            a, b, c = out[i - 1], out[i], out[i + 1]
            if (abs(a[0] - b[0]) < 0.01 and abs(b[0] - c[0]) < 0.01) or (abs(a[1] - b[1]) < 0.01 and abs(b[1] - c[1]) < 0.01):
                out.pop(i)
            else:
                i += 1
        return out

    def edge_pill_texts(self, e):
        out = []
        for f in e.path:
            out.append("." + f)
        if e.coercion:
            out.append(e.coercion)
        return out

    def pick_run(self, boxes, a, b, y1, y2, e, chain_set, used=()):
        """y of a horizontal segment that crosses columns a+1..b-1 without entering a node."""
        span = range(a + 1, b)

        chain_edge = abs(y1) < 0.5 and abs(y2) < 0.5

        def free(y):
            if not chain_edge and abs(y) < 14:
                return False
            for ua, ub, uy, us in used:
                if us != e.src.uid and ua < b and a < ub and abs(uy - y) < 18:
                    return False
            for c in span:
                for top, bot, n in boxes.get(c, []):
                    if top - 6 <= y <= bot + 6:
                        return False
            return True

        cands = [y1, y2]
        gaps = set()
        for c in span:
            bs = sorted(boxes.get(c, []), key=lambda t: t[0])
            if bs:
                gaps.add(bs[0][0] - 14)
            for (t1, b1, _), (t2, b2, _) in zip(bs, bs[1:]):
                gaps.add((b1 + t2) / 2)
        for g in sorted(gaps, key=lambda g: (abs(g - y1) + abs(g - y2), g)):
            cands.append(g)
        tops = [t for c in span for t, _, _ in boxes.get(c, [])]
        if tops:
            cands += [min(tops) - 14 - 10 * k for k in range(4)]
        cands += [c + k * 18 for c in (y1, y2) for k in (-1, 1, -2, 2, -3, 3)]
        for y in cands:
            if free(y):
                return y
        return y1

    def run_candidates(self, boxes, a, b, y1, y2):
        span = range(a + 1, b)
        cands = [y1, y2]
        gaps = set()
        for c in span:
            bs = sorted(boxes.get(c, []), key=lambda t: t[0])
            if bs:
                gaps.add(bs[0][0] - 14)
            for (t1, b1, _), (t2, b2, _) in zip(bs, bs[1:]):
                gaps.add((b1 + t2) / 2)
            # under the lowest node: halfway to the error axis when the column has no chain node
            if bs and max(b for _, b, _ in bs) < 0:
                gaps.add(max(b for _, b, _ in bs) / 2)
        cands += sorted(gaps, key=lambda g: (abs(g - y1) + abs(g - y2), g))
        tops = [t for c in span for t, _, _ in boxes.get(c, [])]
        if tops:
            cands += [min(tops) - 14 - 12 * k for k in range(4)]
        cands += [c + k * 14 for c in (y1, y2) for k in (-2, -1, 1, 2)]
        out = []
        for y in cands:
            if y not in out:
                out.append(y)
        return out

    def resolve_ys(self, items, ordered, ys, pin, pair_y):
        """items: [port, preference, fixed, rank]. Fills ys (uid -> y). Minimum gap SPACING; fixed ones do not move."""
        if not items:
            return
        its = sorted(items, key=lambda it: (it[3] if ordered else (it[1], it[3])))
        n = len(its)
        y = [it[1] for it in its]
        fixed = [it[2] or it[0].uid in pin for it in its]
        for i in range(n):
            if it_fixed(its, i, pin):
                y[i] = pin.get(its[i][0].uid, pair_y.get(its[i][0].uid, y[i]))
        for _ in range(3):
            for i in range(1, n):
                if y[i] < y[i - 1] + SPACING and not fixed[i]:
                    y[i] = y[i - 1] + SPACING
            for i in range(n - 2, -1, -1):
                if y[i] > y[i + 1] - SPACING and not fixed[i]:
                    y[i] = y[i + 1] - SPACING
        for it, v in zip(its, y):
            ys[it[0].uid] = v

    # ------------------------------------------------------------------------------------------------

    def decorate(self, L, edges, plan, ncol, ch_left, lab_zone, tail_zone, chw):
        """Source labels, field / conversion pills, junction points."""
        N = self.N
        # wire pills: on the last horizontal segment before the terminal
        for e, pts in L.routes:
            texts = self.edge_pill_texts(e)
            if not texts:
                continue
            x2, y2 = pts[-1]
            xr = x2 - 8
            # pills read left to right: field then conversion, the last one touching the terminal
            for txt, kind in reversed([(t, "field" if t.startswith(".") else "coercion") for t in texts]):
                w = self.pill_w(txt)
                L.edge_pills.append((txt, kind, xr, y2))
                xr -= w + 4
        # labels: one per source, never under a wire (nor on a node, nor on another label)
        by_src = {}
        for e, pts in L.routes:
            by_src.setdefault(e.src.uid, []).append((e, pts))
        segs = [(a, b) for e, pts in L.routes for a, b in zip(pts, pts[1:])]
        boxes = [(x, y, x + n.w, y + n.h) for n, (x, y) in L.node_xy.items()]
        placed = []

        def hits(r):
            x0, y0, x1, y1 = r
            n = 0
            for a, b in segs:
                if max(a[0], b[0]) >= x0 and min(a[0], b[0]) <= x1 and max(a[1], b[1]) >= y0 and min(a[1], b[1]) <= y1:
                    n += 1
            for bx0, by0, bx1, by1 in boxes:
                if bx1 > x0 and bx0 < x1 and by1 > y0 and by0 < y1:
                    n += 1
            for q in placed:
                if q[2] > x0 and q[0] < x1 and q[3] > y0 and q[1] < y1:
                    n += 1
            return n

        def candidates(lst, w, h, first_only=False):
            """Possible rectangles: above then below each horizontal segment, pitch 8."""
            out = []
            for e, pts in lst:
                for k, ((xa, ya), (xb, yb)) in enumerate(zip(pts, pts[1:])):
                    if abs(ya - yb) > 0.01 or xb - xa < w + 12:
                        continue
                    start = xa + (12 + (N.carry_w / 2 if (k == 0 and e.src.role == "carry") else 0) if k == 0 else 6)
                    x = start
                    while x + w <= xb - 6:
                        out.append((x, ya - 3 - h, x + w, ya - 3))
                        x += 8
                    x = start
                    while x + w <= xb - 6:
                        out.append((x, ya + 3, x + w, ya + 3 + h))
                        x += 8
            return out

        for uid in sorted(by_src):
            lst = by_src[uid]
            e0 = lst[0][0]
            text = self.wire_label(e0.src)
            if not text or e0.src.hollow:
                continue
            w, h = N.measure(N.st["wire name"], text)
            cands = candidates(lst, w, h)
            best, bestn = None, None
            for r in cands:
                n = hits(r)
                if bestn is None or n < bestn:
                    best, bestn = r, n
                if n == 0:
                    break
            if best is not None:
                placed.append(best)
                L.labels.append((text, best[0], best[1], e0.src.family))
                L.label_boxes.append((text, best[0], best[1], e0.src.family, w, h))
        # carry with no consumer: its name is still written (v in the body is the current value)
        for p in L.entries:
            if p.role == "carry" and p.uid not in by_src and p.uid in L.pos:
                text = self.wire_label(p)
                w, h = N.measure(N.st["wire name"], text)
                x0, y0 = L.pos[p.uid]
                r = (x0 + 12 + N.carry_w / 2, y0 - 3 - h, x0 + 12 + N.carry_w / 2 + w, y0 - 3)
                placed.append(r)
                L.labels.append((text, r[0], r[1], p.family))
                L.label_boxes.append((text, r[0], r[1], p.family, w, h))
        # long wire: repeat the name (N-3, 400 units), only at a free spot
        for uid in sorted(by_src):
            lst = by_src[uid]
            e0 = lst[0][0]
            text = self.wire_label(e0.src)
            if not text or e0.src.hollow:
                continue
            w, h = N.measure(N.st["wire name"], text)
            e, pts = lst[0]
            for (xa, ya), (xb, yb) in zip(pts, pts[1:]):
                if abs(ya - yb) < 0.01 and xb - xa > 400:
                    for fr in (0.5, 0.35, 0.65, 0.8, 0.2):
                        x = xa + (xb - xa) * fr - w / 2
                        for r in ((x, ya - 3 - h, x + w, ya - 3), (x, ya + 3, x + w, ya + 3 + h)):
                            if hits(r) == 0:
                                placed.append(r)
                                L.labels.append((text, r[0], r[1], e0.src.family))
                                L.label_boxes.append((text, r[0], r[1], e0.src.family, w, h))
                                break
                        else:
                            continue
                        break
        # junction points: where two wires of the same source part (at a corner or as a T on a segment)
        for uid, lst in by_src.items():
            if len(lst) < 2:
                continue
            dots = []
            for i, (e, pts) in enumerate(lst):
                for f, qts in lst[i + 1:]:
                    if f.family != e.family:  # an order wire (after:) leaves without a dot
                        continue
                    p = self.split_point(pts, qts)
                    if p is None or any(p == d[:2] for d in dots):
                        continue
                    if any(abs(p[0] - r[0]) < 0.05 and abs(p[1] - r[1]) < 0.05 for r in (pts[0], pts[-1], qts[-1])):
                        continue
                    dots.append((p[0], p[1], e.family))
            L.junctions += dots

    @staticmethod
    def split_point(A, B):
        """Last point shared by two orthogonal polylines that leave from the same point (rounded to 0.1)."""
        A, B = [tuple(p) for p in A], [tuple(p) for p in B]
        if not A or not B or A[0] != B[0]:
            return None
        p, i, j = A[0], 0, 0
        while i + 1 < len(A) and j + 1 < len(B):
            a, b = A[i + 1], B[j + 1]
            la, lb = math.hypot(a[0] - p[0], a[1] - p[1]), math.hypot(b[0] - p[0], b[1] - p[1])
            if la < 0.01:
                i += 1
                continue
            if lb < 0.01:
                j += 1
                continue
            da = ((a[0] - p[0]) / la, (a[1] - p[1]) / la)
            db = ((b[0] - p[0]) / lb, (b[1] - p[1]) / lb)
            if abs(da[0] - db[0]) > 0.01 or abs(da[1] - db[1]) > 0.01:
                break
            if abs(la - lb) < 0.05:
                p, i, j = a, i + 1, j + 1
            elif la < lb:
                p, i = a, i + 1
            else:
                p, j = b, j + 1
        return (round(p[0], 1), round(p[1], 1))

    # ------------------------------------------------------------------------------------------------

    def layout_fn(self, root, body):
        L = self.layout_body(body, HEADER, owner=root, title=root.title, root=True)
        return L


def it_fixed(its, i, pin):
    return its[i][2] or its[i][0].uid in pin
