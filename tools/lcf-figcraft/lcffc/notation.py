"""Reads `design/notation.figcraft`: tokens, components, style samples, text measurement.

Everything goes through `figcraft-cli run` (commands that return JSON: variable.list, page.list, node.get).
Nothing about the appearance is written here: colors, shapes, sizes and fonts come from the file.
"""
import re

from . import fc


class NotationError(Exception):
    pass


FAMILY_SAMPLE = "Wire/"


def ok(res, i=None):
    for r in res:
        if not r.get("ok", False):
            raise NotationError(f"{r.get('command')}: {r.get('error')}")
    return res if i is None else res[i]["result"]


class Style:
    """Font of a text sample."""

    def __init__(self, d):
        self.family = d.get("fontFamily", "Segoe UI")
        self.style = d.get("fontStyle", "Regular")
        self.size = float(d.get("fontSize", 12))
        self.color = d.get("_token")

    def key(self):
        return (self.family, self.style, self.size)

    @property
    def mono(self):
        return self.family.lower() in ("consolas", "courier new", "cascadia mono", "liberation mono")


class WireStyle:
    def __init__(self, family, frame, children):
        pd = frame.get("pluginData") or {}
        self.family = family
        trait = next(c for c in children if c["name"] == "stroke")
        self.token = trait["_token_stroke"]
        self.weight = float(trait.get("strokeWeight", 1.5))
        self.dashes = [float(x) for x in (trait.get("dashes") or [])]
        self.decor = pd.get("decor") or ""
        self.pitch = float(pd.get("pitch") or 0)
        self.chevron_l = float(pd.get("chevron_l") or 4)
        self.chevron_h = float(pd.get("chevron_h") or 8)
        self.tick_h = float(pd.get("tick_h") or 8)
        core = next((c for c in children if c["name"] == "core"), None)
        self.core_weight = float(core.get("strokeWeight", 2)) if core else 0.0
        chev = next((c for c in children if c["name"] == "chevron"), None)
        self.decor_weight = float(chev.get("strokeWeight", self.weight)) if chev else self.weight


class Notation:
    def __init__(self, path):
        self.path = str(path)
        self.var_name = {}      # VariableID -> name
        self.tokens = {}        # name -> {mode: hex}
        self.modes = []
        self.ids = {}           # object name -> id
        self.nodes = {}         # name -> node.get result
        self.kids = {}          # name -> children (node.get results)
        self.wires = {}
        self.text_cache = {}
        self.pending = set()
        self._load()

    # --- reading ----------------------------------------------------------------------------------------
    def run(self, steps):
        s = fc.Script()
        for c, p in steps:
            s.add(c, p, ref=False)
        return ok(fc.run(s, src=self.path))

    def _load(self):
        r = self.run([("variable.list", {}), ("page.list", {})])
        vl, pages = r[0]["result"], r[1]["result"]
        self.modes = [m["name"] for m in vl["collections"][0]["modes"]] if vl["collections"] else []
        modeid = {m["id"]: m["name"] for c in vl["collections"] for m in c["modes"]}
        for v in vl["variables"]:
            self.var_name[v["id"]] = v["name"]
            vals = {}
            for mid, val in v["values"].items():
                c = val["value"]
                vals[modeid.get(mid, mid)] = "#%02X%02X%02X" % tuple(round(c[k] * 255) for k in "rgb")
            self.tokens[v["name"]] = vals
        page = next((p for p in pages if p["name"] == "Notation"), pages[0])
        top = self.run([("node.get", {"id": page["id"]})])[0]["result"]["children"]
        frame = next(c for c in top if c["type"] == "FRAME")
        kids = self.run([("node.get", {"id": frame["id"]})])[0]["result"]["children"]
        by_name = {}
        for k in kids:
            by_name.setdefault(k["name"], k)
        want = [n for n in by_name if n in ("Terminal", "Carry") or n == "Primitive" or n == "Badge" or n == "Tab"
                or n == "Node" or n.startswith(("Pill/", "Structure/", FAMILY_SAMPLE))]
        got = self.run([("node.get", {"id": by_name[n]["id"]}) for n in want])
        for n, g in zip(want, got):
            self.ids[n] = by_name[n]["id"]
            self.nodes[n] = g["result"]
            self.kids[n] = [self._decorate(c) for c in g["result"].get("children", [])]
        for n in want:
            if n.startswith(FAMILY_SAMPLE):
                fam = n[len(FAMILY_SAMPLE):]
                self.wires[fam] = WireStyle(fam, self.nodes[n], self.kids[n])
        # components: Terminal variants
        self.terminal_variants = set()
        self.variant_id = {}
        tv = self.kids.get("Terminal", [])
        for v in tv:
            m = dict(re.findall(r"(\w+)=([^,]+)", v["name"]))
            key = (m.get("family", "").strip(), m.get("direction", "").strip())
            self.terminal_variants.add(key)
            self.variant_id[key] = v["id"]
        self.terminal_w = float(tv[0]["width"]) if tv else 14.0
        self.terminal_h = float(tv[0]["height"]) if tv else 14.0
        self.carry_variant = {}
        for v in self.kids.get("Carry", []):
            m = dict(re.findall(r"(\w+)=([^,]+)", v["name"]))
            self.carry_variant[m.get("family", "").strip()] = v["id"]
        rv = self.kids.get("Carry", [])
        self.carry_w = float(rv[0]["width"]) if rv else 22.0
        self.carry_h = float(rv[0]["height"]) if rv else 20.0
        self.prim_d = float(self.nodes["Primitive"]["width"])
        self.pill_h = float(self.nodes["Pill/constant"]["height"])
        pad = (self.nodes["Pill/constant"].get("layout") or {}).get("padding") or [0, 7, 0, 7]
        self.pill_pad = float(pad[1]) + float(pad[3])
        self.pill_text = Style(self.kids["Pill/constant"][0])
        self.badge_h = float(self.nodes["Badge"]["height"])
        bpad = (self.nodes["Badge"].get("layout") or {}).get("padding") or [0, 8, 0, 8]
        self.badge_pad = float(bpad[1]) + float(bpad[3])
        self.badge_text = Style(self.kids["Badge"][0])
        self.tab_h = float(self.nodes["Tab"]["height"])
        tpad = (self.nodes["Tab"].get("layout") or {}).get("padding") or [0, 8, 0, 8]
        self.tab_pad = float(tpad[1]) + float(tpad[3])
        self.tab_text = Style(self.kids["Tab"][0])
        self._node_metrics()

    def _decorate(self, c):
        bv = c.get("boundVariables") or {}
        c = dict(c)
        c["_token"] = self.var_name.get(bv.get("fill"))
        c["_token_stroke"] = self.var_name.get(bv.get("stroke"))
        return c

    def kid(self, sample, name):
        return next(c for c in self.kids[sample] if c["name"] == name)

    def _node_metrics(self):
        n = self.nodes["Node"]
        body = self.kid("Node", "body")
        self.node_radius = float((body.get("cornerRadius") or 6))
        self.node_fill = body["_token"]
        self.node_stroke = body["_token_stroke"]
        self.node_stroke_w = float(body.get("strokeWeight", 1))
        sep = self.kid("Node", "divider")
        self.header_h = float(sep["y"])
        self.sep_token, self.sep_w = sep["_token_stroke"], float(sep.get("strokeWeight", 1))
        l1, l2 = self.kid("Node", "input row"), self.kid("Node", "input row 2")
        self.row_pitch = float(l2["y"]) - float(l1["y"])
        self.row0_center = float(l1["y"]) + float(l1["height"]) / 2
        self.node_bottom = float(body["height"]) - (float(l2["y"]) + float(l2["height"]) / 2)
        self.pad_x = float(l1["x"])
        self.st = {k: Style(self.kid("Node", k)) for k in
                   ("title", "library", "input row", "output row", "label", "wire name", "unit")}
        self.title_y = float(self.kid("Node", "title")["y"]) + float(self.kid("Node", "title")["height"]) / 2
        self.frame_style = {}
        for k in ("Structure/frame", "Structure/error frame"):
            c = self.kid(k, "frame")
            t = self.kid(k, "title")
            self.frame_style[k] = {"stroke": c["_token_stroke"], "weight": float(c.get("strokeWeight", 1)),
                                   "radius": float((c.get("cornerRadius") or 6)), "title": Style(t),
                                   "title_h": float(t["height"]), "title_x": float(t["x"]),
                                   "legend": Style(self.kid(k, "legend"))}
        enc = self.nodes["Structure/notch"]
        self.notch_w, self.notch_h = float(enc["width"]), float(enc["height"])
        self.notch_token = (self._decorate(enc))["_token"]

    # --- text measurement ----------------------------------------------------------------------------------
    def measure(self, style, text):
        """(width, height); an estimate until `resolve()` has measured the string."""
        key = (style.family, style.style, style.size, text)
        hit = self.text_cache.get(key)
        if hit is not None:
            return hit
        self.pending.add(key)
        w = (0.55 if style.mono else 0.53) * style.size * len(text)
        h = round(style.size * (1.17 if style.mono else 1.33))
        return (w, h)

    def resolve(self):
        """Measures every requested string in one pass; True if there was anything new."""
        todo = sorted(self.pending - set(self.text_cache))
        if not todo:
            return False
        s = fc.Script()
        for fam, sty, size, text in todo:
            ref = s.add("node.create", {"type": "text", "text": text or " ", "x": 0, "y": 0,
                                        "props": {"fontFamily": fam, "fontStyle": sty, "fontSize": size}})
            s.add("node.get", {"id": ref}, ref=False)
        res = ok(fc.run(s, src=self.path))
        gets = [r for r in res if r["command"] == "node.get"]
        for key, g in zip(todo, gets):
            r = g["result"]
            self.text_cache[key] = (float(r["width"]) if key[3] else 0.0, float(r["height"]))
        self.pending.clear()
        return True
