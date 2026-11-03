"""IR (lcf-poc) -> graph: nodes, terminals in signature order, wires typed by family.

The graph knows nothing about layout or FigCraft. Typing approximations (documented in the README):
types declared when the IR has them (parameters, outputs, annotated carries, `.lct` fields), outputs known for
primitives by table, otherwise "number".
"""
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Optional

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "lcf-poc"))
from lcfpoc import stdlib  # noqa: E402
from lcfpoc.ir import (Call, For, FnIR, Handle, Loop, Match, Operand, StructLit)  # noqa: E402
from lcfpoc.library import load_library  # noqa: E402

# Familles (visual-notation.md 4.1)
NUMBER, BOOL, TEXT, ENUM, STRUCT, ARRAY = "number", "boolean", "text", "enum", "struct", "array"
SESSION, ERROR, ORDER, FUNCTION = "session", "error", "order", "function"

INT_TYPES = {"i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64"}
NUM_TYPES = INT_TYPES | {"f32", "f64", "c64", "c128", "duration", "timestamp"}
PRIMITIVES = {"add": "+", "sub": "−", "mul": "×", "div": "÷", "abs": "|x|", "ge": "≥", "le": "≤", "gt": ">",
              "lt": "<", "eq": "=", "ne": "≠", "and": "∧", "or": "∨", "not": "¬", "select": "?",
              "with_unit": "[V]", "unitless": "[·]"}
BOOL_OUT = {"ge", "le", "gt", "lt", "eq", "ne", "and", "or", "not", "errors.is_none"}
KEEP_UNIT_OF_FIRST = {"add", "sub", "abs", "neg", "min", "max"}
MAX_PILL = 26


@dataclass
class Ty:
    family: str = NUMBER
    unit: Optional[str] = None
    base: str = ""  # f64, i16... (for conversions)
    elem: Any = None  # Ty of the element of an array
    name: str = ""  # name of the user type

    @property
    def label(self):
        t = self.name or self.base
        if self.family == ARRAY:
            t = f"[{self.elem.label if self.elem else '?'}]"
        return (t or self.family) + (f"[{self.unit}]" if self.unit else "")


class Port:
    """Un terminal. `role` : in | out | after | entry | exit | carry | next | until | collect | yield | iter | select."""
    _n = 0

    def __init__(self, owner, side, name, ty, role="in", label=None, hollow=False):
        Port._n += 1
        self.uid = Port._n
        self.owner, self.side, self.name, self.ty, self.role = owner, side, name, ty, role
        self.label = name if label is None else label
        self.hollow = hollow
        self.pill = None  # text of an attached constant
        self.pill_kind = "constant"
        self.x = self.y = 0.0  # absolute position (filled in by the layout)
        self.mirror = None  # same position, other side of the edge (frame)
        self.inner = None  # frame output: inner sink whose position it takes
        self.chain = False
        self.pair = None

    @property
    def family(self):
        return ORDER if self.role == "after" else self.ty.family

    def __repr__(self):
        return f"<Port {self.owner.title if hasattr(self.owner, 'title') else self.owner}.{self.name}:{self.role}>"


class Edge:
    def __init__(self, src, dst, path=(), coercion=None, family=None):
        self.src, self.dst, self.path, self.coercion = src, dst, tuple(path), coercion
        self.forced = family if family == ORDER else None
        self.points = []

    @property
    def family(self):
        if self.forced:
            return self.forced
        ty = self.src.ty
        return ty.family

    @property
    def label_name(self):
        return self.src.name

    def __repr__(self):
        return f"<Edge {self.src!r} -> {self.dst!r}>"


class GNode:
    _n = 0

    def __init__(self, kind, title, lib=""):
        GNode._n += 1
        self.uid = GNode._n
        self.kind, self.title, self.lib = kind, title, lib
        self.glyph = ""
        self.always = False
        self.label = None  # label #close:
        self.badges = []
        self.ins, self.outs = [], []
        self.after = None
        self.hidden = 0  # unwired optional inputs (+n)
        self.frame_kind = None   # loop | for | match | handle
        self.bodies = []  # [(title, Body)]; a single one for loop/for
        self.header_ports = []  # selector etc.
        self.entries = {}  # frame: key -> input Port (left edge)
        self.exits = []  # frame: output ports (right edge)
        self.line = 0
        self.pos = (0, 0)
        self.size = (0, 0)

    @property
    def is_frame(self):
        return self.kind == "frame"

    def __repr__(self):
        return f"<GNode {self.kind} {self.title}>"


class Body:
    def __init__(self, title="", owner=None):
        self.title = title
        self.owner = owner  # frame GNode (None for the function)
        self.nodes, self.edges = [], []
        self.entries = []  # left-edge sources visible in this body
        self.sinks = []  # right-edge sinks
        self.pattern_error = False


@dataclass
class Ref:
    port: Optional[Port]
    path: tuple = ()
    const: Optional[str] = None


class Scope:
    def __init__(self, body, parent=None, frame=None):
        self.names = {}
        self.body, self.parent, self.frame = body, parent, frame

    def lookup(self, name):
        if name in self.names:
            return self.names[name]
        if self.parent is None:
            return None
        r = self.parent.lookup(name)
        if r is None or r.port is None or self.frame is None:
            return r
        # outer value: an entry (notch) on the frame, an outer wire to it
        ent = self.frame.entries.get(name)
        if ent is None:
            ent = Port(self.frame, "in", name, r.port.ty, role="entry")
            self.frame.entries[name] = ent
            self.parent.body.edges.append(Edge(r.port, ent, r.path, family=r.port.family))
            ent.outer_ref = r
        ref = Ref(ent, ())
        self.names[name] = ref
        return ref


# --------------------------------------------------------------------------------------------------
# Types

def split_unit(t):
    m = re.match(r"^(.*?)\s*\[([^\]]+)\]$", t.strip())
    return (m.group(1), m.group(2)) if m else (t.strip(), None)


class Typer:
    def __init__(self, lib):
        self.lib = lib

    def tree_text(self, td, tree):
        try:
            return td.src[tree.meta.start_pos:tree.meta.end_pos]
        except Exception:
            return ""

    def classify(self, text, depth=0):
        t = (text or "").strip()
        if not t:
            return Ty(NUMBER)
        if t.startswith("["):  # [T] or [T; n]
            inner = t[1:t.rfind("]")]
            inner = inner.split(";")[0].strip()
            return Ty(ARRAY, elem=self.classify(inner, depth + 1) if depth < 4 else Ty(NUMBER))
        if t.startswith("fn"):
            return Ty(FUNCTION)
        base, unit = split_unit(t)
        base = re.sub(r"<.*>$", "", base).strip()
        if base == "error":
            return Ty(ERROR, base="error")
        if base == "bool":
            return Ty(BOOL, base="bool")
        if base in ("string", "bytes"):
            return Ty(TEXT, base=base)
        if base in NUM_TYPES:
            return Ty(NUMBER, unit, base)
        if base in stdlib_opaque() or base.split(".")[0] in ("queue", "ref"):
            return Ty(SESSION, name=base)
        td = self.lib.types.get(base.split(".")[0])
        if td is not None and depth < 4:
            tree = td.tree
            kind = getattr(tree, "data", "")
            if kind == "struct_type":
                return Ty(STRUCT, name=td.name)
            if kind == "enum_type":
                return Ty(ENUM, name=td.name)
            return self.classify(td.text, depth + 1)
        return Ty(NUMBER, unit, base)

    def field_type(self, ty, field):
        td = self.lib.types.get(ty.name) if ty.family == STRUCT else None
        if td is None:
            return Ty(NUMBER)
        for f in td.tree.children:
            if str(f.children[0]) == field:
                return self.classify(self.tree_text(td, f.children[1]))
        return Ty(NUMBER)


def stdlib_opaque():
    from lcfpoc.library import OPAQUE
    return OPAQUE


# --------------------------------------------------------------------------------------------------

def fmt_const(op):
    t = (op.text or "").strip()
    return t if len(t) <= MAX_PILL else t[:MAX_PILL - 1] + "…"


class Builder:
    def __init__(self, lib, fn):
        self.lib, self.fn = lib, fn
        self.typer = Typer(lib)
        self.err_count = 0

    # ---- fonction ---------------------------------------------------------------------------------
    def build(self):
        fn = self.fn
        body = Body(fn.name, None)
        root = GNode("frame", fn.name)
        root.frame_kind = "fn"
        root.bodies = [(fn.name, body)]
        body.owner = root
        scope = Scope(body)
        outs = [o[0] for o in fn.outputs]
        params = {p.name for p in fn.params}
        both = params & set(outs)
        for p in fn.params:
            port = Port(root, "out", p.name, self.typer.classify(p.type_text), role="entry")
            if p.default is not None:
                port.pill = fmt_const(p.default)
            body.entries.append(port)
            scope.names[p.name] = Ref(port)
        for name, _, text in fn.outputs:
            port = Port(root, "in", name, self.typer.classify(text), role="exit")
            body.sinks.append(port)
        self.fill_body(body, fn.body, scope, outputs=dict(zip(outs, body.sinks)), both=both, ctx=None)
        return root, body

    # ---- corps -------------------------------------------------------------------------------------
    def fill_body(self, body, ir, scope, outputs=None, both=(), ctx=None):
        """Two passes: (1) create the nodes and register the names; (2) wire the operands."""
        pending = []
        for n in ir.nodes:
            made = self.make_node(n, body, scope, ctx)
            pending.append((n, made))
            names_only_out = outputs is not None and True
            for t, port in made["target_ports"]:
                if t.kind == "name":
                    if t.name in both:  # output only: the name designates the input of the same name
                        outputs_edge = outputs[t.name]
                        made.setdefault("to_output", []).append((port, outputs_edge))
                    else:
                        scope.names[t.name] = Ref(port)
                        if outputs is not None and t.name in outputs:
                            made.setdefault("to_output", []).append((port, outputs[t.name]))
                elif t.kind == "next":
                    made.setdefault("to_next", []).append((port, t.name))
            if n.rhs is not None and made.get("alias"):
                pass
        # aliases: `x = y` without a node
        for n, made in pending:
            if made.get("alias") is not None:
                op = n.rhs
                for t in n.targets:
                    if t.kind == "name":
                        ref = self.resolve(op, scope)
                        if t.name in both:
                            made["alias_out"] = (outputs[t.name], ref, op)
                        else:
                            scope.names[t.name] = ref if ref is not None else Ref(None, const=fmt_const(op))
                            if outputs is not None and t.name in outputs:
                                made["alias_out"] = (outputs[t.name], ref, op)
                    elif t.kind == "next":
                        made["alias_next"] = (t.name, op)
        for n, made in pending:
            self.wire_node(n, made, body, scope, ctx)
            for port, sink in made.get("to_output", []):
                body.edges.append(Edge(port, sink, family=port.family))
            for port, nxt in made.get("to_next", []):
                sink = ctx["next"].get(nxt) if ctx else None
                if sink is not None:
                    body.edges.append(Edge(port, sink, family=port.family))
            if made.get("alias_out"):
                sink, ref, op = made["alias_out"]
                self.link(body, ref, op, sink, scope)
            if made.get("alias_next") and ctx:
                nm, op = made["alias_next"]
                sink = ctx["next"].get(nm)
                if sink is not None:
                    self.link(body, self.resolve(op, scope), op, sink, scope)
        # block terminals: until, collect, yield
        if ctx:
            for op in ir.untils:
                sink = ctx["until"]
                self.link(body, self.resolve(op, scope), op, sink, scope)
            for op, sink in zip(ir.collects, ctx.get("collect_ports", [])):
                self.link(body, self.resolve(op, scope), op, sink, scope)
            if ir.yields and ctx.get("yield_ports") is not None:
                for op, sink in zip(ir.yields[0], ctx["yield_ports"]):
                    self.link(body, self.resolve(op, scope), op, sink, scope)

    # operands ------------------------------------------------------------------------------------------
    def resolve(self, op, scope):
        """Operand -> Ref (port + field path) or None for a constant / a type name."""
        if op.kind == "ref":
            r = scope.lookup(op.parts[0])
            if r is None:
                return None
            if r.port is None:
                return r
            return Ref(r.port, tuple(r.path) + tuple(op.parts[1:]))
        return None

    def ty_of(self, op, scope):
        if op.kind == "const":
            v = op.value
            if isinstance(v, bool):
                return Ty(BOOL, base="bool")
            if isinstance(v, str):
                return Ty(TEXT, base="string")
            if v is None:
                return Ty(ERROR, base="error")
            unit = None
            if hasattr(v, "label"):
                pass
            m = re.search(r"\s([^\s\d][^\s]*)$", op.text or "")
            if m and not isinstance(v, (int, float)):
                unit = m.group(1)
            return Ty(NUMBER, unit, "f64")
        if op.kind == "array":
            return Ty(ARRAY, elem=Ty(NUMBER))
        r = self.resolve(op, scope)
        if r is None:
            if op.kind == "ref" and op.parts[0] in self.lib.types:
                return Ty(ENUM, name=op.parts[0])
            return Ty(NUMBER)
        return self.ref_ty(r)

    def ref_ty(self, r):
        if r.port is None:
            return Ty(NUMBER)
        ty = r.port.ty
        for f in r.path:
            ty = self.typer.field_type(ty, f)
        return ty

    def link(self, body, ref, op, dst, scope):
        """Wires operand `op` to terminal `dst`: a wire, or an attached constant."""
        if ref is None or ref.port is None:
            if ref is not None and ref.const:
                dst.pill = ref.const
            else:
                dst.pill = fmt_const(op) if op.kind != "array" else fmt_const(op)
            dst.pill_kind = "constant"
            return None
        coercion = self.coercion(self.ref_ty(ref), dst.ty)
        e = Edge(ref.port, dst, ref.path, coercion, family=self.ref_ty(ref).family if ref.path else ref.port.family)
        body.edges.append(e)
        return e

    @staticmethod
    def coercion(src, dst):
        if src.family == NUMBER and dst.family == NUMBER:
            if src.unit and dst.unit and src.unit != dst.unit:
                return f"{src.unit}→{dst.unit}"
            if src.base in NUM_TYPES and dst.base in NUM_TYPES and src.base != dst.base and not src.unit \
                    and not dst.unit and src.base in INT_TYPES | {"f32"} and dst.base in INT_TYPES | {"f32", "f64"}:
                return f"{src.base}→{dst.base}"
        return None

    # node creation --------------------------------------------------------------------------------------
    def make_node(self, n, body, scope, ctx):
        r = n.rhs
        made = {"target_ports": []}
        if isinstance(r, Call):
            node = self.make_call(n, r, body, scope)
        elif isinstance(r, StructLit):
            node = self.make_struct(n, r, body)
        elif isinstance(r, (Match, Handle)):
            node = self.make_branch(n, r, body)
        elif isinstance(r, (Loop, For)):
            node = self.make_loop(n, r, body)
        else:
            made["alias"] = True
            return made
        node.line = n.line
        node.label = n.label
        body.nodes.append(node)
        made["node"] = node
        # targets -> output ports (the order of the targets is that of the outputs)
        if node.is_frame:
            node.outs = [Port(node, "out", "_", Ty(NUMBER), role="exit") for _ in n.targets]
        for t, port in zip(n.targets, node.outs):
            if t.kind == "skip":
                port.hollow = True
                port.label = "_"
                port.name = "_"
            else:
                port.name = t.name
                port.label = ("next " if t.kind == "next" else "") + t.name
            made["target_ports"].append((t, port))
        return made

    def callee(self, call, scope):
        res = self.lib.resolve(call.path, self.fn.uses)
        return res

    def call_param_ty(self, callee, name, tyhint=""):
        if isinstance(callee, FnIR):
            for p in callee.params:
                if p.name == name:
                    return self.typer.classify(p.type_text)
        return Ty(NUMBER)

    def make_call(self, n, call, body, scope):
        res = self.callee(call, scope)
        qname = ".".join(call.path)
        node = GNode("call", qname.split(".")[-1], qname.rsplit(".", 1)[0] + "." if "." in qname else "")
        node.qname = qname
        node.call = call
        if res and res[0] == "builtin":
            b = res[1]
            node.always = b.always
            node.callee_kind = "builtin"
            if qname in PRIMITIVES and "." not in qname:
                node.kind = "prim"
                node.glyph = PRIMITIVES[qname]
                if qname in ("with_unit", "unitless"):
                    u = call.bound.get("unit")
                    unit_txt = (u[0].value if u[0].kind == "unit" else u[0].text) if u else "V"
                    node.glyph = f"[{unit_txt}]" if qname == "with_unit" else f"[{unit_txt}]"
                    node.unit_arg = unit_txt
            params = [(p.name, p.default is stdlib.REQ, p.kind, Ty(NUMBER)) for p in b.params]
            outs = self.builtin_outs(qname, b, call, scope)
        elif res and res[0] == "fn":
            f = res[1]
            node.callee_kind = "fn"
            params = [(p.name, p.default is None, "value", self.typer.classify(p.type_text)) for p in f.params]
            outs = [(o, self.typer.classify(t)) for o, _, t in f.outputs]
        elif res and res[0] == "variant":
            node.title = f"{res[1]}.{res[2]}"
            node.lib = ""
            node.callee_kind = "variant"
            params = [(f"v{i}", True, "value", Ty(NUMBER)) for i, _ in enumerate(call.args)]
            outs = [("value", Ty(ENUM, name=res[1]))]
        else:
            node.callee_kind = "inconnu"
            params = [(a.name or f"a{i}", True, "value", Ty(NUMBER)) for i, a in enumerate(call.args)]
            outs = [("result", Ty(NUMBER))]
        # inputs: required + wired; the rest is hidden (+n)
        wired = set()
        pos = [a for a in call.args if a.name is None]
        for (pname, _, _, _), a in zip(params, pos):
            wired.add(pname)
        wired |= {a.name for a in call.args if a.name and a.name != "after"}
        node.params_all = params
        for pname, req, kind, ty in params:
            if req or pname in wired:
                p = Port(node, "in", pname, ty, role="in")
                node.ins.append(p)
            else:
                node.hidden += 1
        for oname, ty in outs:
            node.outs.append(Port(node, "out", oname, ty, role="out"))
        if call.after is not None:
            node.after = Port(node, "in", "after", Ty(ORDER), role="after", label="")
        return node

    def builtin_outs(self, qname, b, call, scope):
        outs = []
        first_unit = None
        if qname in KEEP_UNIT_OF_FIRST and call.args:
            a0 = call.bound.get(b.params[0].name)
            if a0:
                first_unit = self.ty_of(a0[0], scope).unit
        for oname in b.outs:
            if qname in BOOL_OUT or oname == "result" and qname in ("ge", "le", "gt", "lt", "eq", "ne"):
                ty = Ty(BOOL, base="bool")
            elif oname == "err":
                ty = Ty(ERROR, base="error")
            elif oname in ("session", "bank", "source"):
                ty = Ty(SESSION, name="visa.session")
            elif oname == "text":
                ty = Ty(TEXT, base="string")
            elif oname == "array":
                ty = Ty(ARRAY, elem=Ty(NUMBER))
            elif qname == "with_unit":
                u = call.bound.get("unit")
                ty = Ty(NUMBER, (u[0].value if u[0].kind == "unit" else u[0].text) if u else None, "f64")
            elif qname == "dmm_driver.read_dc_volts" and oname == "volts":
                ty = Ty(NUMBER, "V", "f64")
            elif qname in ("time.now", "time.elapsed"):
                ty = Ty(NUMBER, "s", "duration" if qname == "time.elapsed" else "timestamp")
            elif qname == "select":
                a1 = call.bound.get("if_true")
                ty = self.ty_of(a1[0], scope) if a1 else Ty(NUMBER)
            elif qname == "array.get" and oname == "item":
                a0 = call.bound.get("array")
                t0 = self.ty_of(a0[0], scope) if a0 else Ty(NUMBER)
                ty = t0.elem or Ty(NUMBER)
            else:
                ty = Ty(NUMBER, first_unit, "f64")
            outs.append((oname, ty))
        return outs

    def make_struct(self, n, lit, body):
        name = ".".join(lit.path)
        node = GNode("call", name + " { }")
        node.qname = name
        node.callee_kind = "struct"
        node.struct_lit = lit
        td = self.lib.types.get(name)
        decl = []
        if td is not None and getattr(td.tree, "data", "") == "struct_type":
            decl = [(str(f.children[0]), self.typer.classify(self.typer.tree_text(td, f.children[1])))
                    for f in td.tree.children]
        else:
            decl = [(f, Ty(NUMBER)) for f, _ in lit.fields]
        for fname, ty in decl:
            node.ins.append(Port(node, "in", fname, ty, role="in"))
        node.outs.append(Port(node, "out", name, Ty(STRUCT, name=name), role="out"))
        return node

    def make_branch(self, n, r, body):
        is_handle = isinstance(r, Handle)
        kw = "handle" if is_handle else "match"
        node = GNode("frame", f"{kw} {r.sel.text}")
        node.frame_kind = kw
        node.sel_op = r.sel
        n_yield = len(r.arms[0].body.yields[0]) if r.arms and r.arms[0].body.yields else len(n.targets)
        node.n_yield = max(n_yield, len(n.targets))
        node.arms_ir = r.arms
        node.sel_port = Port(node, "in", "sel", Ty(NUMBER), role="select", label=r.sel.text if r.sel.kind == "ref" else "")
        # outputs: the render port is the `yield` sink of the first arm (unfolded view)
        return node

    def make_loop(self, n, r, body):
        is_for = isinstance(r, For)
        title = "for" if is_for else "loop"
        node = GNode("frame", title)
        node.frame_kind = "for" if is_for else "loop"
        node.ir = r
        if is_for:
            if r.parallel:
                node.title = "for parallel"
                node.double = True
            node.title += " " + ", ".join(f"{c.name} in {c.lo.text}" + (f"..{c.hi.text}" if c.hi else "")
                                           for c in r.clauses)
        elif r.continue_on_error:
            node.badges.append("continue_on_error")
        return node

    # node wiring -----------------------------------------------------------------------------------------
    def wire_node(self, n, made, body, scope, ctx):
        node = made.get("node")
        if node is None:
            return
        r = n.rhs
        if node.kind in ("call", "prim") and isinstance(r, Call):
            self.wire_call(node, r, body, scope)
        elif isinstance(r, StructLit):
            for fname, op in r.fields:
                port = next((p for p in node.ins if p.name == fname), None)
                if port is not None:
                    self.link(body, self.resolve(op, scope), op, port, scope)
        elif node.frame_kind in ("match", "handle"):
            self.wire_branch(node, r, n, body, scope)
        elif node.frame_kind in ("loop", "for"):
            self.wire_loop(node, r, n, body, scope)

    def wire_call(self, node, call, body, scope):
        by_name = {p.name: p for p in node.ins}
        order = [p.name for p in node.ins]
        # positional: in signature order (all inputs, including hidden ones)
        allnames = [p[0] for p in node.params_all]
        pos = [a for a in call.args if a.name is None]
        for pname, a in zip(allnames, pos):
            self.wire_arg(node, by_name.get(pname), a.op, body, scope, pname)
        for a in call.args:
            if a.name is None:
                continue
            if a.name == "after":
                ref = self.resolve(a.op, scope)
                if ref is not None and ref.port is not None:
                    body.edges.append(Edge(ref.port, node.after, ref.path, family=ORDER))
                continue
            self.wire_arg(node, by_name.get(a.name), a.op, body, scope, a.name)

    def wire_arg(self, node, port, op, body, scope, pname):
        if port is None:
            return
        kind = next((p[2] for p in node.params_all if p[0] == pname), "value")
        if kind == "unit" and op.kind in ("ref", "unit"):
            # the requested unit is written in the glyph ([V]), not wired
            node.ins = [p for p in node.ins if p is not port]
            return
        if kind == "category":
            port.pill = op.text
            port.pill_kind = "constant"
            return
        ref = self.resolve(op, scope)
        if ref is not None and ref.port is not None:
            ty = self.ref_ty(ref)
            if port.ty.family == NUMBER and not port.ty.base and not port.ty.unit:
                port.ty = ty  # generic primitive: the terminal takes the type of the wire
            self.link(body, ref, op, port, scope)
        else:
            if op.kind == "ref" and op.parts[0] in self.lib.types:
                port.ty = Ty(ENUM, name=op.parts[0])
            elif op.kind == "const":
                port.ty = self.ty_of(op, scope)
            self.link(body, ref, op, port, scope)

    # ---- structures ----------------------------------------------------------------------------------------
    def wire_branch(self, node, r, n, body, scope):
        # selector
        ref = self.resolve(r.sel, scope)
        if ref is not None and ref.port is not None:
            node.sel_port.ty = self.ref_ty(ref)
            body.edges.append(Edge(ref.port, node.sel_port, ref.path, family=self.ref_ty(ref).family))
        else:
            node.sel_port.pill = fmt_const(r.sel)
        is_handle = node.frame_kind == "handle"
        for arm in r.arms:
            title = self.arm_title(arm.pat, is_handle)
            ab = Body(title, node)
            ab.pattern_error = is_handle
            node.bodies.append((title, ab))
            sc = Scope(ab, scope, node)
            yields = arm.body.yields[0] if arm.body.yields else []
            ports = []
            for i, op in enumerate(yields):
                nm = n.targets[i].name if i < len(n.targets) and n.targets[i].kind != "skip" else "_"
                ports.append(Port(node, "in", nm, Ty(NUMBER), role="yield"))
            ab.sinks = ports
            if arm.pat.bind:
                bty = Ty(ERROR) if is_handle else Ty(NUMBER)
                bp = Port(node, "out", arm.pat.bind, bty, role="entry")
                ab.entries.append(bp)
                sc.names[arm.pat.bind] = Ref(bp)
            self.fill_body(ab, arm.body, sc, ctx={"next": {}, "until": None, "yield_ports": ports})
            for p, op in zip(ports, yields):
                p.ty = self.ty_of(op, sc)
        # frame outputs = yield sinks of the first arm
        first = node.bodies[0][1].sinks if node.bodies else []
        node.exits = list(first)
        # the other arms: same names
        for _, ab in node.bodies[1:]:
            for p, q in zip(ab.sinks, node.exits):
                p.name = q.name
                p.label = q.label
        # output type: that of the first known value
        for i, p in enumerate(node.exits):
            for _, ab in node.bodies:
                if i < len(ab.sinks) and ab.sinks[i].ty.family != NUMBER:
                    p.ty = ab.sinks[i].ty
                    break
            for _, ab in node.bodies:
                if i < len(ab.sinks):
                    ab.sinks[i].ty = p.ty
        for out, sink in zip(node.outs, node.exits):
            out.inner, out.ty = sink, sink.ty

    @staticmethod
    def arm_title(pat, is_handle):
        k = pat.kind
        if k == "wild":
            return "others"
        if k == "ok":
            return "ok"
        if k == "lit":
            v = pat.value
            return "true" if v is True else "false" if v is False else repr(v) if not isinstance(v, str) else f'"{v}"'
        if k == "range":
            return f"{pat.value}..{'=' if pat.inclusive else ''}{pat.hi}"
        if k == "variant":
            return ".".join(pat.parts) + (f"({pat.bind})" if pat.bind else "")
        if k == "cat":
            return ".".join(pat.parts) + (f"({pat.bind})" if pat.bind else "")
        if k == "code":
            return f"code {pat.value}" + (f"({pat.bind})" if pat.bind else "")
        return k

    def wire_loop(self, node, r, n, body, scope):
        lb = Body(node.title, node)
        node.bodies = [(node.title, lb)]
        sc = Scope(lb, scope, node)
        nexts, carry_ports = {}, []
        # clauses `for`
        for c in getattr(r, "clauses", []) or []:
            lo = self.resolve(c.lo, scope)
            ety = Ty(NUMBER)
            if lo is not None and lo.port is not None:
                at = self.ref_ty(lo)
                ety = at.elem if at.family == ARRAY and at.elem else Ty(NUMBER, base="i64")
            ip = Port(node, "out", c.name, ety, role="iter")
            ip.clause = c
            lb.entries.append(ip)
            sc.names[c.name] = Ref(ip)
            outer = Port(node, "in", c.name, Ty(ARRAY) if c.hi is None else Ty(NUMBER), role="entry")
            # the outer wire arrives on the clause terminal (left edge)
            ip.mirror, outer.mirror = outer, ip
            if c.hi is None:
                self.link(body, lo, c.lo, outer, scope)
            else:
                outer.pill = f"{c.lo.text}..{c.hi.text}" if (lo is None and self.resolve(c.hi, scope) is None) else None
                self.link(body, lo, c.lo, outer, scope)
                self.link(body, self.resolve(c.hi, scope), c.hi, outer, scope)
            node.entries.setdefault(("iter", c.name), outer)
        for c in r.carries:
            init_ref = self.resolve(c.init, scope)
            ty = self.typer.classify(self.typer.tree_text_of(c) if False else "") if False else None
            if c.type is not None:
                ty = self.typer.classify(self._type_text(c.type))
            else:
                ty = self.ty_of(c.init, scope)
            cur = Port(node, "out", c.name, ty, role="carry")
            nxt = Port(node, "in", "next " + c.name, ty, role="next")
            cur.pair = nxt
            nxt.pair = cur
            lb.entries.append(cur)
            lb.sinks.append(nxt)
            sc.names[c.name] = Ref(cur)
            nexts[c.name] = nxt
            carry_ports.append((cur, nxt, c))
            # initial value: outer wire to the current terminal, or attached constant
            outer = Port(node, "in", c.name, ty, role="entry")
            cur.mirror, outer.mirror = outer, cur
            self.link(body, init_ref, c.init, outer, scope)
            node.entries[("carry", c.name)] = outer
        collect_ports = []
        for i, op in enumerate(lb_ir_collects(r.body)):
            collect_ports.append(Port(node, "in", "[ ]", Ty(ARRAY, elem=Ty(NUMBER)), role="collect"))
        until = None
        if r.body.untils:
            until = Port(node, "in", "until", Ty(BOOL, base="bool"), role="until")
            lb.sinks.append(until)
        lb.sinks.extend(collect_ports)
        ctx = {"next": nexts, "until": until, "collect_ports": collect_ports, "yield_ports": None}
        self.fill_body(lb, r.body, sc, ctx=ctx)
        for p, op in zip(collect_ports, r.body.collects):
            elem = self.ty_of(op, sc)
            p.ty = Ty(ARRAY, elem=elem)
        # node outputs: one per target; carries first (final value), then collects
        finals = [nxt for _, nxt, _ in carry_ports] + collect_ports
        node.exits = finals
        for out, sink in zip(node.outs, finals):
            out.inner, out.ty = sink, sink.ty

    def _type_text(self, t):
        # the text of a carry annotation: no source here, read the tree
        try:
            from lark import Tree, Token
            def tx(x):
                if isinstance(x, Token):
                    return str(x)
                if isinstance(x, Tree):
                    if x.data == "path":
                        return ".".join(str(c) for c in x.children)
                    if x.data == "array_type":
                        return "[" + tx(x.children[0]) + "]"
                    if x.data == "unit_annot":
                        return "[" + "".join(tx(c) for c in x.children) + "]"
                    return "".join(tx(c) for c in x.children)
                return ""
            return tx(t)
        except Exception:
            return ""


def lb_ir_collects(body):
    return body.collects


# --------------------------------------------------------------------------------------------------

def load(folder, fn_name):
    lib = load_library(folder)
    if lib.errors:
        raise SystemExit("invalid library: " + "; ".join(str(d) for d in lib.errors[:3]))
    if fn_name not in lib.functions:
        raise SystemExit(f"unknown function \"{fn_name}\" (available: {', '.join(sorted(lib.functions))})")
    fn = lib.functions[fn_name]
    root, body = Builder(lib, fn).build()
    return lib, fn, root, body
