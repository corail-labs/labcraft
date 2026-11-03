#!/usr/bin/env python3
"""Generates `design/notation.figcraft`, the notation library (throwaway: run it once).

The resulting file is then edited by hand in FigCraft; `lcf_figcraft.py` only reads it.
It contains, on the "Notation" page:
  - the "Notation" variable collection (Light and Dark modes): the tokens of visual-notation.md section 8;
  - the `Terminal` component (family x direction variants), `Primitive`, `Pill/*`, `Badge`, `Tab`;
  - the style samples: `Node`, `Structure/*`, and one wire per family (`Wire/number`, `Wire/error`...).
Parameters FigCraft cannot carry (chevron pitch...) are in the samples' `pluginData`.

    python tools/lcf-figcraft/build_notation.py [design/notation.figcraft]
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from lcffc import fc  # noqa: E402

COLLECTION = "Notation"
MODES = ("Light", "Dark")

# Tokens: (light, dark). The first 15 are those of visual-notation.md section 8 (provisional values from the
# board, dark theme invented here); the others are support tokens that section 8 does not name yet.
TOKENS = {
    "canvas.fill": ("#F7F7F5", "#16181C"),
    "canvas.grid": ("#E6E6E2", "#23262B"),
    "node.fill": ("#FFFFFF", "#20242A"),
    "node.stroke": ("#8A8F98", "#7B8390"),
    "node.title": ("#1F2328", "#ECEFF3"),
    "node.active": ("#15803D", "#4ADE80"),
    "frame.fill.1": ("#EEF0F2", "#1D2026"),
    "frame.fill.2": ("#E3E6EA", "#252930"),
    "frame.fill.3": ("#DADEE3", "#2D323A"),
    "frame.fill.4": ("#D0D5DB", "#353B44"),
    "frame.stroke": ("#8A8F98", "#7B8390"),
    "wire.data": ("#3D4148", "#C5CBD3"),
    "wire.resource": ("#1D6FB8", "#5BA8F0"),
    "wire.error": ("#C2410C", "#FF8F5E"),
    "wire.order": ("#6A7078", "#A3ABB6"),
    "mark.coercion": ("#7C3AED", "#B197FC"),
    "mark.invalid": ("#DC2626", "#FF6B6B"),
    "probe.fill": ("#FFF7D6", "#4A4320"),
    # support (not in section 8)
    "ink.muted": ("#6B7078", "#A3ABB6"),
    "ink.faint": ("#8A8F98", "#8B93A0"),
    "node.divider": ("#E1E3E6", "#343A43"),
    "frame.divider": ("#C9CDD2", "#434A55"),
    "pill.fill": ("#F0F1F3", "#2B3038"),
    "pill.stroke": ("#D0D3D8", "#424955"),
    "pill.text": ("#3D4148", "#C5CBD3"),
    "coercion.fill": ("#F1EAFE", "#2E2646"),
    "badge.fill": ("#1F2328", "#ECEFF3"),
    "badge.text": ("#FFFFFF", "#16181C"),
    "tab.fill": ("#E1E4E8", "#2B3038"),
}

FAMILIES = ["number", "boolean", "text", "enum", "struct", "array", "session", "error", "order",
            "function"]
DIRECTIONS = ["input", "output"]


def lum(hexcol):
    def ch(v):
        v /= 255
        return v / 12.92 if v <= 0.03928 else ((v + 0.055) / 1.055) ** 2.4
    r, g, b = (int(hexcol[i:i + 2], 16) for i in (1, 3, 5))
    return 0.2126 * ch(r) + 0.7152 * ch(g) + 0.0722 * ch(b)


def contrast(a, b):
    la, lb = sorted((lum(a), lum(b)), reverse=True)
    return (la + 0.05) / (lb + 0.05)


class B:
    """Small script builder: shapes bound to variables by name."""

    def __init__(self):
        self.s = fc.Script()

    def make(self, typ, name, parent=None, x=0, y=0, w=None, h=None, fill=None, stroke=None, weight=None,
             path=None, text=None, **props):
        p = {"type": typ, "name": name, "x": x, "y": y}
        if w is not None:
            p["width"] = w
        if h is not None:
            p["height"] = h
        if parent is not None:
            p["parent"] = parent
        if path:
            p["path"] = path
        if text is not None:
            p["text"] = text
        pr = dict(props)
        pr["fill"] = TOKENS[fill][0] if fill else "none"
        if stroke:
            pr["stroke"] = TOKENS[stroke][0]
            if weight:
                pr["strokeWeight"] = weight
        p["props"] = pr
        ref = self.s.add("node.create", p)
        if fill:
            self.bind(ref, "fill", fill)
        if stroke:
            self.bind(ref, "stroke", stroke)
        return ref

    def bind(self, ref, field, token):
        self.s.add("variable.bind", {"ids": [ref], "field": field, "variable": token}, ref=False)

    def text(self, name, parent, x, y, txt, color, family="Segoe UI", size=13, style="Regular", **props):
        return self.make("text", name, parent, x, y, fill=color, text=txt, fontFamily=family, fontSize=size,
                         fontStyle=style, **props)

    def comp(self, ref):
        return self.s.add("component.create", {"ids": [ref]})


def build(out_path):
    b = B()
    s = b.s
    s.add("file.new", {}, ref=False)
    s.add("variable.createCollection", {"name": COLLECTION, "modes": list(MODES)}, ref=False)
    for name, (c, d) in TOKENS.items():
        s.add("variable.create", {"collection": COLLECTION, "name": name, "type": "color",
                                  "values": {"Light": c, "Dark": d}}, ref=False)
    s.add("page.rename", {"id": 1, "name": "Notation"}, ref=False)

    # --- Title ---------------------------------------------------------------------------------------
    page = b.make("frame", "Notation library", None, 0, 0, 1500, 1500, fill="canvas.fill", clip=True)
    b.text("title", page, 40, 30, "LabCraft notation library", "node.title", size=28, style="Semibold")
    b.text("help", page, 40, 72, "Read by tools/lcf-figcraft: edit here, then rerun the render. The tokens are in the "
           "\"Notation\" collection (Light and Dark modes). The center of a Terminal is its anchor point.",
           "ink.muted", size=13)

    def heading(y, label):
        b.text("section", page, 40, y, label, "node.title", size=17, style="Semibold")

    # --- Terminals -----------------------------------------------------------------------------------
    heading(120, "Terminals: family x direction (the center is the wire anchor point)")
    shapes = {}
    comps = []
    for fi, fam in enumerate(FAMILIES + ["ignored"]):
        for di, d in enumerate(DIRECTIONS):
            f = b.make("frame", f"family={fam}, direction={d}", page, 56 + fi * 40, 166 + di * 40, 14, 14, clip=False)
            c = {"number": "wire.data", "boolean": "wire.data", "text": "wire.data", "enum": "wire.data",
                 "struct": "wire.data", "array": "wire.data", "session": "wire.resource",
                 "error": "wire.error", "order": "wire.order", "function": "wire.data", "ignored": "wire.data"}[fam]
            if fam == "number":
                b.make("ellipse", "shape", f, 2, 2, 10, 10, fill=c)
            elif fam == "boolean":
                b.make("vector", "shape", f, 0, 0, path="M7 1 L13 7 L7 13 L1 7 Z", fill=c)
            elif fam == "text":
                b.make("rectangle", "shape", f, 1, 3, 12, 8, fill=c)
            elif fam == "enum":
                b.make("vector", "shape", f, 0, 0, path="M2 1 L13 7 L2 13 Z", fill=c)
            elif fam == "struct":
                b.make("ellipse", "outline", f, 1, 1, 12, 12, stroke=c, weight=1.5)
                b.make("ellipse", "core", f, 4.5, 4.5, 5, 5, fill=c)
            elif fam == "array":
                b.make("vector", "brackets", f, 0, 0, path="M4 1 L1 1 L1 13 L4 13 M10 1 L13 1 L13 13 L10 13",
                       stroke=c, weight=1.5)
                b.make("ellipse", "core", f, 5, 5, 4, 4, fill=c)
            elif fam == "session":
                b.make("rectangle", "shape", f, 1, 1, 12, 12, fill=c)
            elif fam == "error":
                b.make("vector", "shape", f, 0, 0, path="M1 0 L13 7 L1 14 L5 7 Z", fill=c)
            elif fam == "order":
                b.make("vector", "shape", f, 0, 0, path="M3 1 L12 7 L3 13", stroke=c, weight=1.5)
            elif fam == "function":
                b.text("glyph", f, 3, -1, "ƒ", c, size=15, style="Bold Italic")
            else:  # ignored: hollow terminal, no wire
                b.make("ellipse", "shape", f, 2, 2, 10, 10, stroke=c, weight=1.5)
            comps.append(b.comp(f))
    tset = s.add("component.combineVariants", {"ids": comps})
    s.add("node.set", {"ids": [tset], "name": "Terminal"}, ref=False)

    # --- Primitive -------------------------------------------------------------------------------------
    heading(380, "Primitive: a 28 px circle with a glyph (\"glyph\" property)")
    pf = b.make("frame", "Primitive", page, 40, 410, 28, 28, clip=False)
    b.make("ellipse", "disc", pf, 0, 0, 28, 28, fill="node.fill", stroke="wire.data", weight=1.5)
    gl = b.text("glyph", pf, 0, 4, "+", "node.title", size=14, style="Semibold", textAlign="center",
                textAutoResize="none", width=28, height=20)
    s.add("node.set", {"ids": [gl], "width": 28, "height": 20, "textAlign": "center"}, ref=False)
    prim = b.comp(pf)
    s.add("component.addProperty", {"id": prim, "name": "glyph", "type": "text", "layer": gl}, ref=False)

    # --- Pills and badges ----------------------------------------------------------------------------
    heading(470, "Pills (attached constant, field, conversion), badge, tab: \"text\" property")

    def pill(name, x, y, fill, stroke, color, txt, font="Consolas", size=11, style="Regular", radius=9, pad=7, h=18):
        f = b.make("frame", name, page, x, y, 60, h, fill=fill, stroke=stroke, weight=1, cornerRadius=radius,
                   layout="horizontal", padding=[0, pad, 0, pad], gap=0, crossAlign="center", mainAlign="center",
                   sizingH="hug", sizingV="fixed", clip=False)
        t = b.text("text", f, 0, 0, txt, color, family=font, size=size, style=style)
        c = b.comp(f)
        s.add("component.addProperty", {"id": c, "name": "text", "type": "text", "layer": t}, ref=False)
        return c

    pill("Pill/constant", 40, 500, "pill.fill", "pill.stroke", "pill.text", "◂ 2 s")
    pill("Pill/field", 160, 500, "node.fill", "wire.data", "node.title", ".low")
    pill("Pill/coercion", 260, 500, "coercion.fill", "mark.coercion", "mark.coercion", "mV→V")
    pill("Badge", 380, 500, "badge.fill", None, "badge.text", "always", font="Segoe UI", style="Semibold", pad=8)
    pill("Tab", 480, 500, "tab.fill", None, "ink.muted", "State.Init", pad=8, radius=3, h=22)

    # --- Node (sample: its geometry sets the layout landmarks) ---------------------------------
    heading(560, "Node (sample read by the tool: body, title, divider, rows, label, wire name)")
    nf = b.make("frame", "Node", page, 40, 600, 220, 120, clip=False)
    b.make("rectangle", "body", nf, 0, 0, 220, 96, fill="node.fill", stroke="node.stroke", weight=1,
           cornerRadius=6)
    b.text("title", nf, 10, 4, "visa.open", "node.title", size=13, style="Semibold")
    b.make("line", "divider", nf, 0, 24, 220, 0, stroke="node.divider", weight=1)
    b.text("library", nf, 90, 4, "visa.", "ink.muted", size=13)
    b.text("input row", nf, 12, 36, "resource", "node.title", family="Consolas", size=12)
    b.text("output row", nf, 160, 36, "session", "node.title", family="Consolas", size=12)
    b.text("input row 2", nf, 12, 60, "timeout", "node.title", family="Consolas", size=12)
    b.text("label", nf, 0, 100, "#close:", "ink.faint", family="Consolas", size=12)
    b.text("wire name", nf, 100, 100, "e1", "wire.data", family="Consolas", size=11)
    b.text("unit", nf, 140, 100, "[V]", "ink.muted", family="Consolas", size=11)
    s.add("node.set", {"ids": [nf], "pluginData": {"role": "node sample"}}, ref=False)

    # --- Structures -------------------------------------------------------------------------------------
    heading(740, "Structures: frame, error frame (handle), title, notch, return arrow")
    for nm, strokecol, x in (("Structure/frame", "frame.stroke", 40), ("Structure/error frame", "wire.error", 300)):
        sf = b.make("frame", nm, page, x, 770, 220, 90, clip=False)
        b.make("rectangle", "frame", sf, 0, 0, 220, 90, fill="frame.fill.1", stroke=strokecol, weight=1,
               cornerRadius=6)
        b.text("title", sf, 10, 6, "match state", "node.title", family="Consolas", size=13, style="Bold")
        b.text("legend", sf, 10, 60, "next v becomes v on the next iteration", "ink.muted", size=11.5)
        s.add("node.set", {"ids": [sf], "pluginData": {"role": "structure sample"}}, ref=False)
    ef = b.make("frame", "Structure/notch", page, 560, 770, 4, 14, fill="frame.stroke", clip=False)
    s.add("node.set", {"ids": [ef], "pluginData": {"role": "notch where a wire crosses the edge"}}, ref=False)
    s.add("node.set", {"ids": [ef], "cornerRadius": 1}, ref=False)

    # --- Wires: one sample per family ------------------------------------------------------------------
    heading(890, "Wires: one sample per family (weight, dashes, color, decoration pitch read by the tool)")
    spec = {
        # family: (token, weight, dashes, decoration, pitch)
        "number": ("wire.data", 1.5, [], None, 0),
        "boolean": ("wire.data", 1.5, [], None, 0),
        "text": ("wire.data", 1.5, [], None, 0),
        "enum": ("wire.data", 1.5, [], None, 0),
        "struct": ("wire.data", 4, [], "core", 0),
        "array": ("wire.data", 1.5, [], "ticks", 8),
        "session": ("wire.resource", 2.5, [], None, 0),
        "error": ("wire.error", 2, [], "chevrons", 24),
        "order": ("wire.order", 1, [2, 3], None, 0),
        "function": ("wire.data", 1.5, [8, 3, 2, 3], None, 0),
    }
    for i, fam in enumerate(FAMILIES):
        x, y = 40 + (i % 2) * 330, 930 + (i // 2) * 50
        wf = b.make("frame", f"Wire/{fam}", page, x, y, 160, 28, clip=False)
        tok, weight, dashes, deco, pitch = spec[fam]
        props = {"dashes": dashes} if dashes else {}
        b.make("line", "stroke", wf, 4, 14, 152, 0, stroke=tok, weight=weight, **props)
        pd = {"family": fam, "decor": deco or "", "pitch": str(pitch)}
        if deco == "chevrons":
            for k in range(1, 6):
                b.make("vector", "chevron", wf, 24 * k - 18, 10, path="M0 0 L4 4 L0 8", stroke=tok, weight=weight,
                       strokeJoin="round")
            pd.update({"chevron_l": "4", "chevron_h": "8"})
        if deco == "ticks":
            for k in range(1, 18):
                b.make("line", "tick", wf, 4 + 8 * k, 10, 0, 8, stroke=tok, weight=1)
            pd.update({"tick_h": "8"})
        if deco == "core":
            b.make("line", "core", wf, 4, 14, 152, 0, stroke="canvas.fill", weight=2)
            pd.update({"core_weight": "2"})
        s.add("node.set", {"ids": [wf], "pluginData": pd}, ref=False)
        b.text("name", page, x + 170, y + 6, fam, "ink.muted", family="Consolas", size=11)

    # --- Carry (carry terminal, straddling the frame edge) -----------------------------------
    heading(1240, "Carry ↻: box straddling the edge of a loop (outline = family of the carry)")
    regs = []
    for fi, fam in enumerate(FAMILIES):
        tok, weight, _, _, _ = spec[fam]
        f = b.make("frame", f"family={fam}", page, 56 + fi * 40, 1280, 22, 20, clip=False)
        b.make("rectangle", "box", f, 0, 0, 22, 20, fill="node.fill", stroke=tok, weight=min(max(weight, 1.5), 2.5),
               cornerRadius=4, strokeAlign="inside")
        b.text("glyph", f, 0, 1, "↻", tok, size=13, style="Bold", textAlign="center", textAutoResize="none",
               width=22, height=17)
        regs.append(b.comp(f))
    rset = s.add("component.combineVariants", {"ids": regs})
    s.add("node.set", {"ids": [rset], "name": "Carry"}, ref=False)
    s.add("node.set", {"ids": [page], "height": 1340}, ref=False)

    out = Path(out_path)
    out.parent.mkdir(parents=True, exist_ok=True)
    res = fc.run(s, out=out)
    bad = [r for r in res if not r.get("ok", True)]
    if bad:
        raise SystemExit(f"failed: {bad[0]}")
    return res


def report_contrast():
    print("Wire contrast on the backgrounds (light / dark, target 4.5):")
    for mode, idx in (("light", 0), ("dark", 1)):
        for w in ("wire.data", "wire.resource", "wire.error", "wire.order", "mark.invalid"):
            row = []
            for bg in ("canvas.fill", "frame.fill.1", "frame.fill.2", "frame.fill.3", "frame.fill.4"):
                row.append(f"{contrast(TOKENS[w][idx], TOKENS[bg][idx]):.1f}")
            print(f"  {mode:6} {w:14} " + " ".join(row))


if __name__ == "__main__":
    target = sys.argv[1] if len(sys.argv) > 1 else str(Path(__file__).resolve().parents[2] / "design" / "notation.figcraft")
    build(target)
    print("written", target)
    report_contrast()
