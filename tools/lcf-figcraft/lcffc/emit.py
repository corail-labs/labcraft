"""Layout -> FigCraft script (JSONL). All appearance comes from the notation library (tokens bound by name)."""
import math

from . import fc
from .layout import HEADER
from .model import ERROR, ORDER

CORNER = 6
MARGIN = 16


def fnum(x):
    return fc.num(x)


class Emitter:
    def __init__(self, N, lay, theme=None):
        self.N, self.lay, self.theme = N, lay, theme
        self.s = fc.Script()
        self.layers = {k: [] for k in range(6)}   # 0 frames, 1 wires, 2 pills/labels, 3 nodes, 4 terminals
        self.root = None

    # ---- primitives ---------------------------------------------------------------------------------------
    def op(self, layer, fn):
        self.layers[layer].append(fn)

    def bind(self, ref, field, token):
        if token:
            self.s.add("variable.bind", {"ids": [ref], "field": field, "variable": token}, ref=False)

    def hexof(self, token):
        t = self.N.tokens.get(token)
        return list(t.values())[0] if t else "#808080"

    def rect(self, x, y, w, h, fill=None, stroke=None, weight=1, radius=0, name="rect", parent=None, dashes=None):
        props = {"fill": self.hexof(fill) if fill else "none"}
        if stroke:
            props.update({"stroke": self.hexof(stroke), "strokeWeight": weight})
        if radius:
            props["cornerRadius"] = radius
        if dashes:
            props["dashes"] = dashes
        ref = self.s.add("node.create", {"type": "rectangle", "name": name, "x": fnum(x), "y": fnum(y),
                                         "width": fnum(w), "height": fnum(h), "parent": parent or self.root,
                                         "props": props})
        self.bind(ref, "fill", fill)
        self.bind(ref, "stroke", stroke)
        return ref

    def frame(self, x, y, w, h, name, fill=None, stroke=None, weight=1, radius=0, parent=None, top=False):
        props = {"fill": self.hexof(fill) if fill else "none", "clip": False}
        if stroke:
            props.update({"stroke": self.hexof(stroke), "strokeWeight": weight})
        if radius:
            props["cornerRadius"] = radius
        prm = {"type": "frame", "name": name, "x": fnum(x), "y": fnum(y), "width": fnum(w), "height": fnum(h),
               "props": props}
        if not top:
            prm["parent"] = parent or self.root
        ref = self.s.add("node.create", prm)
        self.bind(ref, "fill", fill)
        self.bind(ref, "stroke", stroke)
        return ref

    def text(self, x, y, txt, style, token, name=None, parent=None, align=None):
        props = {"fontFamily": style.family, "fontStyle": style.style, "fontSize": style.size,
                 "fill": self.hexof(token)}
        if align:
            props["textAlign"] = align
        ref = self.s.add("node.create", {"type": "text", "name": name or txt[:24], "text": txt, "x": fnum(x),
                                         "y": fnum(y), "parent": parent or self.root, "props": props})
        self.bind(ref, "fill", token)
        return ref

    def line(self, x1, y1, x2, y2, token, weight, name="stroke", parent=None, dashes=None):
        return self.path(f"M{fnum(x1)} {fnum(y1)} L{fnum(x2)} {fnum(y2)}", token, weight, name, parent, dashes)

    def path(self, d, token, weight, name="wire", parent=None, dashes=None, fill=None, join="round", cap="none"):
        props = {"fill": self.hexof(fill) if fill else "none"}
        if token and not fill:
            props.update({"stroke": self.hexof(token), "strokeWeight": weight, "strokeJoin": join, "strokeCap": cap})
        if dashes:
            props["dashes"] = dashes
        ref = self.s.add("node.create", {"type": "vector", "name": name, "path": d, "x": 0, "y": 0,
                                         "parent": parent or self.root, "props": props})
        if fill:
            self.bind(ref, "fill", fill)
        elif token:
            self.bind(ref, "stroke", token)
        return ref

    def instance(self, comp_id, x, y, name, props=None, parent=None):
        ref = self.s.add("instance.create", {"component": comp_id, "x": fnum(x), "y": fnum(y),
                                             "parent": parent or self.root})
        self.s.add("node.set", {"ids": [ref], "name": name}, ref=False)
        if props:
            self.s.add("instance.setProperties", {"ids": [ref], "properties": props}, ref=False)
        return ref

    # ---- components ---------------------------------------------------------------------------------------------
    def terminal(self, x, y, family, direction, name, hollow=False):
        N = self.N
        fam = "ignored" if hollow else family
        if (fam, direction) not in N.terminal_variants:
            fam = "number"
        vid = N.variant_id[(fam, direction)]
        return self.instance(vid, x - N.terminal_w / 2, y - N.terminal_h / 2, name)

    def pill(self, kind, text, x_left, y_center, name=None):
        N = self.N
        comp = {"constant": "Pill/constant", "field": "Pill/field", "coercion": "Pill/coercion"}[kind]
        return self.instance(N.ids[comp], x_left, y_center - N.pill_h / 2, name or f"pill {text}",
                             {"text": text})

    def pill_width(self, text):
        return self.N.measure(self.N.pill_text, text)[0] + self.N.pill_pad

    # ---- paths --------------------------------------------------------------------------------------------------
    @staticmethod
    def rounded(points, r=CORNER):
        """Orthogonal polyline -> SVG path with rounded corners (radius r, reduced on short segments)."""
        from .layout import Layouter
        pts = Layouter.clean(list(points))
        if len(pts) < 2:
            pts = [pts[0], pts[0]] if pts else [(0, 0), (0, 0)]
        if len(pts) == 2:
            return f"M{fnum(pts[0][0])} {fnum(pts[0][1])} L{fnum(pts[1][0])} {fnum(pts[1][1])}"
        d = [f"M{fnum(pts[0][0])} {fnum(pts[0][1])}"]
        for i in range(1, len(pts) - 1):
            p0, v, p1 = pts[i - 1], pts[i], pts[i + 1]
            l0 = math.hypot(v[0] - p0[0], v[1] - p0[1])
            l1 = math.hypot(p1[0] - v[0], p1[1] - v[1])
            rr = min(r, l0 / 2, l1 / 2)
            a = (v[0] + (p0[0] - v[0]) * rr / l0, v[1] + (p0[1] - v[1]) * rr / l0)
            b = (v[0] + (p1[0] - v[0]) * rr / l1, v[1] + (p1[1] - v[1]) * rr / l1)
            d.append(f"L{fnum(a[0])} {fnum(a[1])} Q{fnum(v[0])} {fnum(v[1])} {fnum(b[0])} {fnum(b[1])}")
        d.append(f"L{fnum(pts[-1][0])} {fnum(pts[-1][1])}")
        return " ".join(d)

    @staticmethod
    def crossing_points(pts, others):
        """Points where the polyline crosses (perpendicularly) one of the polylines `others`."""
        out = []
        for a, b in zip(pts, pts[1:]):
            for q in others:
                for c, d in zip(q, q[1:]):
                    for (h0, h1), (v0, v1) in (((a, b), (c, d)), ((c, d), (a, b))):
                        if abs(h0[1] - h1[1]) > 0.01 or abs(v0[0] - v1[0]) > 0.01:
                            continue
                        x, y = v0[0], h0[1]
                        if min(h0[0], h1[0]) < x < max(h0[0], h1[0]) and min(v0[1], v1[1]) < y < max(v0[1], v1[1]):
                            out.append((x, y))
        return out

    @staticmethod
    def along(points, spacing, margin_start, margin_end, avoid, keep_out=()):
        """Evenly spaced positions (x, y, dx, dy) along the polyline, away from the corners and from the points
        `keep_out` (crossings with other wires: a chevron is never drawn over another wire)."""
        segs = []
        total = 0.0
        for a, b in zip(points, points[1:]):
            l = math.hypot(b[0] - a[0], b[1] - a[1])
            segs.append((a, b, l, total))
            total += l
        out = []
        corners = [sum(s[2] for s in segs[:i]) for i in range(1, len(segs))]
        pos = margin_start
        while pos <= total - margin_end:
            if all(abs(pos - c) > avoid for c in corners):
                for a, b, l, t0 in segs:
                    if t0 <= pos <= t0 + l and l > 0:
                        f = (pos - t0) / l
                        x, y = a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f
                        if all(math.hypot(x - kx, y - ky) > avoid for kx, ky in keep_out):
                            out.append((x, y, (b[0] - a[0]) / l, (b[1] - a[1]) / l))
                        break
            pos += spacing
        return out

    def wire(self, e, pts, ox, oy, bg_token, name=None, crossings=()):
        """A wire: stroke + decoration of its family (chevrons, ticks, core), all read from the sample.
        `crossings`: local points where other wires cross it, kept free of decoration."""
        W = self.N.wires[e.family]
        P = [(x + ox, y + oy) for x, y in pts]
        K = [(x + ox, y + oy) for x, y in crossings]
        d = self.rounded(P)
        nm = name or f"wire {e.src.name} → {e.dst.owner.title if hasattr(e.dst.owner, 'title') else ''}.{e.dst.name}"
        if W.decor == "chevrons" and W.pitch:
            for x, y, dx, dy in self.along(P, W.pitch, 18, 14, CORNER + 4, K):
                nx, ny = -dy, dx
                l, h = W.chevron_l, W.chevron_h
                p1 = (x - dx * l / 2 + nx * h / 2, y - dy * l / 2 + ny * h / 2)
                p2 = (x + dx * l / 2, y + dy * l / 2)
                p3 = (x - dx * l / 2 - nx * h / 2, y - dy * l / 2 - ny * h / 2)
                d += f" M{fnum(p1[0])} {fnum(p1[1])} L{fnum(p2[0])} {fnum(p2[1])} L{fnum(p3[0])} {fnum(p3[1])}"
        elif W.decor == "ticks" and W.pitch:
            for x, y, dx, dy in self.along(P, W.pitch, 12, 12, CORNER + 3, K):
                nx, ny = -dy, dx
                h = W.tick_h / 2
                d += f" M{fnum(x + nx * h)} {fnum(y + ny * h)} L{fnum(x - nx * h)} {fnum(y - ny * h)}"
        ref = self.path(d, W.token, W.weight, nm, dashes=W.dashes or None)
        if W.decor == "core" and W.core_weight:
            self.path(self.rounded(P), bg_token, W.core_weight, nm + " (core)")
        return ref

    # ---- scene --------------------------------------------------------------------------------------------------------
    def build(self, root_node, L, fn_title):
        N, s = self.N, self.s
        s.add("page.add", {"name": "Render"}, ref=False)
        total_w, total_h = L.w + 2 * MARGIN, L.h + 2 * MARGIN
        self.root = self.frame(0, 0, total_w, total_h, fn_title, fill="canvas.fill", top=True)
        # function frame
        fs = N.frame_style["Structure/frame"]
        self.draw_frame_shell(MARGIN, MARGIN, L.w, L.h, f"fn {fn_title}", fs, "canvas.fill", name=f"function {fn_title}",
                              level=0, error=False)
        self.draw_body(L, MARGIN, MARGIN, level=0, owner=root_node, fn_root=True)
        for k in range(6):
            for fn in self.layers[k]:
                fn()
        if self.theme and self.theme != (N.modes[0] if N.modes else None):
            s.add("variable.setMode", {"ids": [self.root], "collection": "Notation", "mode": self.theme}, ref=False)
        return s

    def fill_token(self, level):
        return "canvas.fill" if level == 0 else f"frame.fill.{min(level, 4)}"

    def draw_frame_shell(self, x, y, w, h, title, fs, fill, name, level, error, badges=()):
        N = self.N

        def go():
            self.rect(x, y, w, h, fill=fill, stroke=fs["stroke"], weight=fs["weight"], radius=fs["radius"], name=name)
            tx = x + fs["title_x"]
            self.text(tx, y + 7, title, fs["title"], "node.title", name="title " + name)
            bx = tx + N.measure(fs["title"], title)[0] + 10
            for b in badges:
                w_b = N.measure(N.badge_text, b)[0] + N.badge_pad
                self.instance(N.ids["Badge"], bx, y + 5, f"badge {b}", {"text": b})
                bx += w_b + 6
        self.op(0, go)

    def draw_body(self, L, ox, oy, level, owner, fn_root=False):
        N = self.N
        bg = self.fill_token(level)
        # wires
        for e, pts in L.routes:
            others = [q for f, q in L.routes if f.src.uid != e.src.uid]
            cross = self.crossing_points(pts, others)
            self.op(1, lambda e=e, pts=pts, cross=cross: self.wire(e, pts, ox, oy, bg, crossings=cross))
        for x, y, fam in L.junctions:
            tok = N.wires[fam].token
            self.op(1, lambda x=x, y=y, tok=tok: self.dot(x + ox, y + oy, tok))
        for txt, x, y, fam in L.labels:
            tok = N.wires[fam].token
            self.op(2, lambda txt=txt, x=x, y=y, tok=tok: self.text(
                x + ox, y + oy, txt, N.st["wire name"], tok, name=f"name {txt}"))
        for txt, kind, xr, y in L.edge_pills:
            self.op(2, lambda txt=txt, kind=kind, xr=xr, y=y: self.pill(
                kind, txt, xr + ox - self.pill_width(txt), y + oy))
        # nodes
        for n, (nx, ny) in L.node_xy.items():
            self.draw_node(n, nx + ox, ny + oy, level)
        # boundary terminals of this body
        self.draw_boundary(L, ox, oy, level, owner, fn_root)

    def dot(self, x, y, tok):
        r = self.s.add("node.create", {"type": "ellipse", "name": "junction", "x": fnum(x - 3), "y": fnum(y - 3),
                                       "width": 6, "height": 6, "parent": self.root,
                                       "props": {"fill": self.hexof(tok)}})
        self.bind(r, "fill", tok)

    def draw_boundary(self, L, ox, oy, level, owner, fn_root):
        N = self.N
        for p in L.entries:
            x, y = L.pos[p.uid]
            x, y = x + ox, y + oy
            if p.role in ("entry",) and not fn_root and p.owner is owner and p.uid not in {q.uid for q in L.body.entries}:
                self.op(4, lambda x=x, y=y: self.notch(x, y))
                continue
            if p.role == "carry":
                self.op(4, lambda p=p, x=x, y=y: self.carry(x, y, p.ty.family, f"carry {p.name}"))
            else:
                self.op(4, lambda p=p, x=x, y=y: self.terminal(x, y, p.ty.family, "output", f"terminal {p.name}"))
            if p.mirror is not None and p.mirror.pill:
                txt = p.mirror.pill
                gap = N.carry_w / 2 + 4 if p.role == "carry" else 10
                self.op(2, lambda txt=txt, x=x, y=y, gap=gap: self.pill("constant", txt,
                                                                        x - gap - self.pill_width(txt), y,
                                                                        name=f"constant {txt}"))
            if fn_root and p.pill:
                txt = "◂ " + p.pill
                lw = N.measure(N.st["wire name"], self.lay.wire_label(p))[0]
                self.op(2, lambda txt=txt, x=x, y=y, lw=lw: self.pill("constant", txt, x + 12 + lw + 8, y,
                                                                      name=f"default {txt}"))
        for p in L.sinks:
            x, y = L.pos[p.uid]
            x, y = x + ox, y + oy
            if p.role == "next":
                self.op(4, lambda p=p, x=x, y=y: self.carry(x, y, p.ty.family, f"carry {p.label}"))
            else:
                self.op(4, lambda p=p, x=x, y=y: self.terminal(x, y, p.ty.family, "input", f"terminal {p.label}",
                                                               hollow=False))
            if p.label:
                st = N.st["output row"]
                lift = (N.pill_h / 2 + 2) if p.pill else 4
                gx = 12 + (N.carry_w / 2 - 6 if p.role == "next" else 0)
                txt = self.lay.sink_label(p)
                self.op(2, lambda p=p, x=x, y=y, st=st, lift=lift, gx=gx, txt=txt: self.text(
                    x - gx - N.measure(st, txt)[0], y - lift - N.measure(st, txt)[1], txt, st,
                    "node.title", name=f"output {p.label}"))
            if p.pill:
                txt = p.pill
                self.op(2, lambda txt=txt, x=x, y=y: self.pill("constant", txt, x - 12 - self.pill_width(txt), y,
                                                              name=f"constant {txt}"))

    def carry(self, x, y, family, name):
        N = self.N
        fam = family if family in N.carry_variant else "number"
        return self.instance(N.carry_variant[fam], x - N.carry_w / 2, y - N.carry_h / 2, name)

    def notch(self, x, y):
        N = self.N
        self.rect(x - N.notch_w / 2, y - N.notch_h / 2, N.notch_w, N.notch_h, fill=N.notch_token, name="notch",
                  radius=1)

    # ---- nodes ------------------------------------------------------------------------------------------------------------
    def draw_node(self, n, x, y, level):
        N = self.N
        if n.kind == "frame":
            return self.draw_struct(n, x, y, level)
        if n.kind == "prim":
            return self.draw_prim(n, x, y)
        # call block
        w, h = n.w, n.h

        def go():
            g = self.frame(x, y, w, h, f"node {n.lib}{n.title}", fill=None)
            self.rect(0, 0, w, h, fill=N.node_fill, stroke=N.node_stroke, weight=N.node_stroke_w,
                      radius=N.node_radius, name="body", parent=g)
            ts = N.st["title"]
            lib_w = N.measure(N.st["library"], n.lib)[0] if n.lib else 0
            th = N.measure(ts, "X")[1]
            ty = N.title_y - th / 2
            if n.lib:
                self.text(N.pad_x, ty, n.lib, N.st["library"], "ink.muted", name="library", parent=g)
            self.text(N.pad_x + lib_w, ty, n.title, ts, "node.title", name="name", parent=g)
            self.line(0, N.header_h, w, N.header_h, N.sep_token, N.sep_w, name="divider", parent=g)
            bx = w - N.pad_x
            if n.always:
                bw = N.measure(N.badge_text, "always")[0] + N.badge_pad
                bx -= bw
                self.instance(N.ids["Badge"], bx, (N.header_h - N.badge_h) / 2, "badge always", {"text": "always"},
                              parent=g)
                bx -= 8
            if n.hidden:
                t = f"+{n.hidden}"
                tw = N.measure(N.st["unit"], t)[0]
                self.text(bx - tw, N.title_y - N.measure(N.st["unit"], t)[1] / 2, t, N.st["unit"], "ink.muted",
                          name="hidden inputs", parent=g)
            for ri, ro in n.rows:
                row_y = N.row0_center + n.rows.index((ri, ro)) * N.row_pitch
                if ri is not None:
                    p = n.ins[ri]
                    st = N.st["input row"]
                    lw, lh = N.measure(st, p.label)
                    self.text(N.pad_x, row_y - lh / 2, p.label, st, "node.title", name=f"input {p.label}", parent=g)
                    cx = N.pad_x + lw
                    if p.ty.unit:
                        uw, uh = N.measure(N.st["unit"], f"[{p.ty.unit}]")
                        self.text(cx + 6, row_y - uh / 2, f"[{p.ty.unit}]", N.st["unit"], "ink.muted", name="unit",
                                  parent=g)
                        cx += 6 + uw
                    if p.pill:
                        self.pill("constant", "◂ " + p.pill, cx + 8, row_y, name=f"constant {p.pill}") \
                            if False else self.instance(N.ids["Pill/constant"], cx + 8, row_y - N.pill_h / 2,
                                                         f"constant {p.pill}", {"text": "◂ " + p.pill}, parent=g)
                if ro is not None:
                    p = n.outs[ro]
                    st = N.st["output row"]
                    lw, lh = N.measure(st, p.label)
                    rx = w - N.pad_x - lw
                    self.text(rx, row_y - lh / 2, p.label, st, "node.title", name=f"output {p.label}", parent=g)
                    if p.ty.unit:
                        uw, uh = N.measure(N.st["unit"], f"[{p.ty.unit}]")
                        self.text(rx - 6 - uw, row_y - uh / 2, f"[{p.ty.unit}]", N.st["unit"], "ink.muted",
                                  name="unit", parent=g)
            for p in n.ins:
                self.terminal_in(g, n, p, 0, "input")
            for p in n.outs:
                self.terminal_in(g, n, p, w, "output")
            if n.after is not None:
                self.terminal_in(g, n, n.after, 0, "input")
            if n.label:
                self.text(x, y - 16, f"#{n.label}:", N.st["label"], "ink.faint", name="label")
        self.op(3, go)

    def terminal_in(self, g, n, p, dx, direction):
        if p.uid not in n.off:
            return
        ox_, oy_ = n.off[p.uid]
        N = self.N
        self.terminal_child(g, ox_, oy_, p.family, direction, f"terminal {p.label or p.name}", p.hollow)

    def terminal_child(self, parent, x, y, family, direction, name, hollow=False):
        N = self.N
        fam = "ignored" if hollow else family
        if (fam, direction) not in N.terminal_variants:
            fam = "number"
        vid = N.variant_id[(fam, direction)]
        return self.instance(vid, x - N.terminal_w / 2, y - N.terminal_h / 2, name, parent=parent)

    def draw_prim(self, n, x, y):
        N = self.N

        def go():
            g = self.frame(x, y, n.w, n.h, f"primitive {n.title}", fill=None)
            self.instance(N.ids["Primitive"], 0, 0, f"disc {n.title}", {"glyph": n.glyph}, parent=g)
            for p in n.ins:
                self.terminal_child(g, n.off[p.uid][0], n.off[p.uid][1], p.family, "input", f"terminal {p.name}")
                if p.pill:
                    txt = p.pill
                    w = self.pill_width(txt)
                    self.instance(N.ids["Pill/constant"], n.off[p.uid][0] - 10 - w - 0,
                                  n.off[p.uid][1] - N.pill_h / 2, f"constant {txt}", {"text": txt}, parent=g)
            for p in n.outs:
                self.terminal_child(g, n.off[p.uid][0], n.off[p.uid][1], p.family, "output", f"terminal {p.name}",
                                    p.hollow)
        self.op(3, go)

    # ---- structures ---------------------------------------------------------------------------------------------------------
    def draw_struct(self, n, x, y, level):
        N = self.N
        err = n.frame_kind == "handle"
        fs = N.frame_style["Structure/error frame" if err else "Structure/frame"]
        lvl = level + 1
        title = n.title
        badges = list(n.badges)
        self.draw_frame_shell(x, y, n.w, n.h, title, fs, self.fill_token(lvl), name=f"{n.frame_kind} {n.title}",
                              level=lvl, error=err, badges=badges)
        if n.frame_kind in ("loop", "for"):
            L, ox, oy = n.layouts[0]
            self.draw_body(L, x + ox, y + oy, lvl, owner=n)
            return
        # match / handle: frame outputs = sinks of the first arm; bus of outer values
        ent_ports = {}
        for p, pts in n.bus:
            P = [(px + x, py + y) for px, py in pts]
            tok = N.wires[p.family].token
            self.op(1, lambda P=P, p=p, tok=tok: self.wire_poly(P, p.family, self.fill_token(lvl)))
        for p, pts in n.bus:
            self.op(4, lambda p=p, pts=pts: self.notch(x + pts[0][0], y + pts[0][1]))
        self.op(4, lambda: self.terminal(x, y + n.off[n.sel_port.uid][1], n.sel_port.family, "input", "terminal selector"))
        if n.sel_port.pill:
            txt = n.sel_port.pill
            self.op(2, lambda txt=txt: self.pill("constant", txt, x - 10 - self.pill_width(txt),
                                                y + n.off[n.sel_port.uid][1], name=f"constant {txt}"))
        for k, ((L, ox, oy), title) in enumerate(zip(n.layouts, n.arm_titles)):
            ax, ay = x + ox, y + oy
            arm_fs = N.frame_style["Structure/error frame" if err else "Structure/frame"]

            def go(ax=ax, ay=ay, L=L, title=title):
                self.rect(ax, ay, L.w, L.h, fill=self.fill_token(lvl + 1), stroke=arm_fs["stroke"], weight=0.5 if False else 1,
                          radius=4, name=f"arm {title}", dashes=None)
                self.instance(N.ids["Tab"], ax + 8, ay + 6, f"tab {title}", {"text": title})
            self.op(0, go)
            self.draw_body(L, ax, ay, lvl + 1, owner=n)

    def wire_poly(self, P, family, bg):
        W = self.N.wires[family]
        self.path(self.rounded(P), W.token, W.weight, "wire (outer value)", dashes=W.dashes or None)
