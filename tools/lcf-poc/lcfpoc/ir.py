"""Intermediate representation: the Lark tree is lowered to simple objects, then checked and run."""
import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Optional

from lark import Token, Tree
from lark.exceptions import LarkError

from .values import UnitError, Q, parse_unit

SPECIAL = {"true": True, "false": False, "none": None, "nan": float("nan"), "inf": float("inf")}


@dataclass
class Operand:
    kind: str  # "ref" | "const" | "array" | "unit" (`[V]`, value = "V") | "label" (`#close`, in `after:`)
    parts: tuple = ()
    value: Any = None
    items: list = field(default_factory=list)
    line: int = 0
    text: str = ""


@dataclass
class Arg:
    name: Optional[str]
    op: Operand


@dataclass
class Target:
    kind: str  # "name" | "skip" | "next"
    name: str = ""


@dataclass
class Call:
    path: tuple
    args: list
    line: int = 0
    bound: dict = field(default_factory=dict)  # filled in by the check
    after: Optional[list] = None  # order inputs (`after:`): values and labels; None if absent
    callee: Any = None


@dataclass
class StructLit:
    path: tuple
    fields: list  # [(name, Operand)]
    line: int = 0


@dataclass
class Carry:
    name: str
    type: Any
    init: Operand


@dataclass
class ForIn:
    name: str
    lo: Operand
    hi: Optional[Operand]


@dataclass
class Pattern:
    kind: str  # wild | lit | range | variant | ok | cat | code
    value: Any = None
    hi: Any = None
    inclusive: bool = False
    binds: tuple = ()  # names bound by the pattern: `_(e)`, `io.timeout(e)`, `R.Two(a, b)`
    parts: tuple = ()

    @property
    def bind(self):
        return self.binds[0] if self.binds else None


@dataclass
class Arm:
    pat: Pattern
    body: "Body"
    more: tuple = ()  # the rest of a list in a `match`: `1, 3..=5, _`; none of them binds a name

    @property
    def pats(self):
        return (self.pat, *self.more)


@dataclass
class Match:
    sel: Operand
    arms: list
    line: int = 0


@dataclass
class Handle:
    sel: Operand
    arms: list
    line: int = 0


@dataclass
class Loop:
    continue_on_error: bool
    carries: list
    body: "Body"
    line: int = 0


@dataclass
class For:
    parallel: bool
    clauses: list
    carries: list
    body: "Body"
    line: int = 0


@dataclass
class Body:
    nodes: list = field(default_factory=list)
    untils: list = field(default_factory=list)
    collects: list = field(default_factory=list)
    yields: list = field(default_factory=list)  # one list of operands per `yield`
    line: int = 0
    own: frozenset = frozenset()  # names defined by the nodes of this block (filled in when lowering)


@dataclass
class Node:
    label: Optional[str]
    targets: list
    rhs: Any
    line: int = 0
    free: frozenset = frozenset()

    @property
    def name(self):
        for t in self.targets:
            if t.kind != "skip":
                return ("next " if t.kind == "next" else "") + t.name
        return self.label or f"_{self.line}"

    def describe(self):
        head = self.label and f"#{self.label}" or ", ".join(
            ("next " if t.kind == "next" else "") + (t.name if t.kind != "skip" else "_") for t in self.targets)
        if isinstance(self.rhs, Call):
            return f"{head} = {'.'.join(self.rhs.path)}(…)"
        return head


@dataclass
class Param:
    name: str
    type: Any
    type_text: str
    default: Optional[Operand]


@dataclass
class FnIR:
    name: str
    params: list
    outputs: list  # [(name, type tree, text)]
    body: Body
    uses: list
    file: str
    line: int
    doc: str = ""
    extern: Any = None  # (interface, {argument: value}) of an `extern` declaration (4.1); no body
    lib: Any = None
    native: Any = None  # what runs an `extern` declaration: its signature, and the native body


@dataclass
class TypeDecl:
    name: str
    tree: Any
    text: str
    file: str
    line: int
    doc: str = ""
    lib: Any = None
    uses: Any = ()

    def enum_variants(self):
        t = self.tree
        if isinstance(t, Tree) and t.data == "enum_type":
            return [str(v.children[0]) for v in t.children]
        return None


class LowerError(Exception):
    def __init__(self, msg, line=0):
        super().__init__(msg)
        self.msg, self.line = msg, line


def line_of(t):
    if isinstance(t, Token):
        return t.line or 0
    m = getattr(t, "meta", None)
    return getattr(m, "line", 0) if m and not m.empty else 0


class Lowerer:
    def __init__(self, src, file):
        self.src, self.file = src, file

    def text(self, t):
        if isinstance(t, Token):
            return str(t)
        return self.src[t.meta.start_pos:t.meta.end_pos]

    # Literals and operands ------------------------------------------------------------

    def number(self, t):
        raw = str(t.children[0]).replace("_", "")
        if raw in SPECIAL:
            val = SPECIAL[raw]
        elif raw == "-inf":
            val = -float("inf")
        elif re.search(r"[.eE]", raw):
            val = float(raw)
        else:
            val = int(raw)
        if len(t.children) > 1:
            unit = self.text(t.children[1])
            try:
                parse_unit(unit)
            except UnitError as e:
                raise LowerError(str(e), line_of(t)) from None
            return Q.of(float(val), unit)
        return val

    def string(self, t):
        s = str(t.children[0])[1:-1]
        return re.sub(r"\\(.)", lambda m: {"n": "\n", "t": "\t", "r": "\r"}.get(m.group(1), m.group(1)), s)

    def operand(self, t):
        line, txt = line_of(t), self.text(t)
        d = t.data
        if d == "path":
            parts = tuple(str(c) for c in t.children)
            if len(parts) == 1 and parts[0] in SPECIAL:
                return Operand("const", value=SPECIAL[parts[0]], line=line, text=txt)
            return Operand("ref", parts=parts, line=line, text=txt)
        if d == "number_lit":
            return Operand("const", value=self.number(t), line=line, text=txt)
        if d == "string":
            return Operand("const", value=self.string(t), line=line, text=txt)
        if d in ("true", "false", "none"):
            return Operand("const", value=SPECIAL[d], line=line, text=txt)
        if d == "array_lit":
            return Operand("array", items=[self.operand(c) for c in t.children if c is not None], line=line, text=txt)
        raise LowerError(f"unexpected operand ({d})", line)

    def arg_value(self, t):
        """Argument: an operand, a unit in brackets (`[V]`) or a label (`#close`, in `after:`)."""
        if t.data == "unit_arg":
            return Operand("unit", value=self.text(t.children[0]), line=line_of(t), text=self.text(t))
        if t.data == "label_ref":
            return Operand("label", parts=("#" + str(t.children[0]),), line=line_of(t), text=self.text(t))
        return self.operand(t)

    # Types -----------------------------------------------------------------------------

    def param(self, t):
        name, ty = str(t.children[0]), t.children[1]
        default = None
        if len(t.children) > 2 and t.children[2] is not None:
            d = t.children[2]
            if d.data in ("call", "struct_lit"):  # R-21, which this proof of concept reads and does not run
                default = Operand("constructed", line=line_of(d), text=self.text(d))
            else:
                default = self.operand(d)
        return Param(name, ty, self.text(ty), default)

    # Blocks -----------------------------------------------------------------------------

    def body(self, stmts, line=0):
        b = Body(line=line)
        for st in stmts:
            if st.data == "binding":
                b.nodes.append(self.binding(st))
            elif st.data == "call_stmt":  # a call alone: no output bound (B9)
                kids = list(st.children)
                label = str(kids.pop(0).children[0]) if isinstance(kids[0], Tree) and kids[0].data == "label" else None
                b.nodes.append(Node(label, [], self.rhs(kids[-1]), line_of(st)))
            elif st.data == "until_stmt":
                b.untils.append(self.operand(st.children[0]))
            elif st.data == "collect_stmt":
                b.collects.append(self.operand(st.children[0]))
            elif st.data == "yield_stmt":
                b.yields.append([self.operand(c) for c in st.children])
        names = {t.name for n in b.nodes for t in n.targets if t.kind == "name"}
        names |= {"#" + n.label for n in b.nodes if n.label}  # a label is an order source (`after: #close`)
        b.own = frozenset(names)
        return b

    def binding(self, t):
        kids = list(t.children)
        label = None
        if isinstance(kids[0], Tree) and kids[0].data == "label":
            label = str(kids.pop(0).children[0])
        rhs = self.rhs(kids.pop())
        targets = []
        for k in kids:
            name = str(k.children[0]) if k.children else "_"
            if k.data == "t_skip" or name == "_":
                targets.append(Target("skip"))
            elif k.data == "t_next":
                targets.append(Target("next", name))
            else:
                targets.append(Target("name", name))
        return Node(label, targets, rhs, line_of(t))

    def rhs(self, t):
        d, line = t.data, line_of(t)
        if d == "call":
            path = tuple(str(c) for c in t.children[0].children)
            args = []
            for a in t.children[1:]:
                if isinstance(a, Tree) and a.data == "arg":
                    if len(a.children) == 2:
                        args.append(Arg(str(a.children[0]), self.arg_value(a.children[1])))
                    else:
                        args.append(Arg(None, self.arg_value(a.children[0])))
                elif isinstance(a, Tree) and a.data == "after_arg":  # after: #label | after: (x, #label)
                    args += [Arg("after", self.arg_value(c)) for c in a.children]
            return Call(path, args, line)
        if d == "struct_lit":
            path = tuple(str(c) for c in t.children[0].children)
            return StructLit(path, [(str(f.children[0]), self.operand(f.children[1])) for f in t.children[1:]], line)
        if d == "match_expr":
            return Match(self.operand(t.children[0]), [self.arm(a, self.pattern) for a in t.children[1:]], line)
        if d == "handle_expr":
            return Handle(self.operand(t.children[0]), [self.arm(a, self.handle_pattern) for a in t.children[1:]], line)
        if d == "loop_expr":
            cont = any(isinstance(c, Token) and c.type == "CONTINUE_ON_ERROR" for c in t.children)
            carries = [self.carry(c) for c in t.children if isinstance(c, Tree) and c.data == "carry"]
            block = t.children[-1]
            return Loop(cont, carries, self.body(block.children, line_of(block)), line)
        if d == "for_expr":
            par = any(isinstance(c, Token) and c.type == "PARALLEL" for c in t.children)
            clauses, carries = [], []
            for c in t.children:
                if isinstance(c, Tree) and c.data == "for_in":
                    hi = self.operand(c.children[2]) if len(c.children) > 2 else None
                    clauses.append(ForIn(str(c.children[0]), self.operand(c.children[1]), hi))
                elif isinstance(c, Tree) and c.data == "carry":
                    carries.append(self.carry(c))
            block = t.children[-1]
            return For(par, clauses, carries, self.body(block.children, line_of(block)), line)
        return self.operand(t)

    def carry(self, c):
        ty = c.children[1] if len(c.children) == 3 else None
        return Carry(str(c.children[0]), ty, self.operand(c.children[-1]))

    def arm(self, a, pattern_fn):
        *pats, body = a.children
        stmts = [c for c in body.children if isinstance(c, Tree)]
        return Arm(pattern_fn(pats[0]), self.body(stmts, line_of(body)), tuple(pattern_fn(p) for p in pats[1:]))

    def pattern(self, p):
        d = p.data
        if d == "wildcard":
            return Pattern("wild")
        if d in ("true", "false"):
            return Pattern("lit", value=SPECIAL[d])
        if d == "number_lit":
            return Pattern("lit", value=self.number(p))
        if d == "string":
            return Pattern("lit", value=self.string(p))
        if d == "range_pat":
            return Pattern("range", value=int(str(p.children[0]).replace("_", "")),
                           hi=int(str(p.children[1]).replace("_", "")), inclusive="..=" in self.text(p))
        if d == "variant_pat":
            parts = tuple(str(c) for c in p.children[0].children)
            return Pattern("variant", parts=parts, binds=tuple(str(c) for c in p.children[1:]))
        raise LowerError(f"unexpected pattern ({d})", line_of(p))

    def handle_pattern(self, p):
        d = p.data
        if d == "h_ok":
            return Pattern("ok")
        if d == "h_code":
            binds = (str(p.children[1]),) if len(p.children) > 1 else ()
            return Pattern("code", value=int(str(p.children[0]).replace("_", "")), binds=binds)
        if d == "h_other":
            return Pattern("wild", binds=(str(p.children[0]),) if p.children else ())
        cat = p.children[0]
        binds = (str(p.children[1]),) if len(p.children) > 1 else ()
        parts = tuple(str(c) for c in cat.children if str(c) != ".*")
        if parts == ("ok",) and not binds:  # `ok` is contextual: here it is the keyword
            return Pattern("ok")
        return Pattern("cat", parts=parts, binds=binds)

    def function(self, fn, uses, file, doc):
        kids = [c for c in fn.children if c is not None]
        names = [str(c) for c in kids if isinstance(c, Token) and c.type == "NAME"]
        extern = None
        if fn.data == "extern_decl":  # `extern interface(args)`, then the signature; no body
            head = re.search(r"^extern\s+\w+\s*\((.*?)\)\s*$", self.src, re.M | re.S)
            extern = (names[0], dict(re.findall(r'(\w+)\s*:\s*"([^"]*)"', head.group(1) if head else "")))
            names = names[1:]
        name = names[0]
        params = [self.param(c) for c in kids if isinstance(c, Tree) and c.data == "param"]
        outputs = [(str(c.children[0]), c.children[1], self.text(c.children[1]))
                   for c in kids if isinstance(c, Tree) and c.data == "output"]
        block = kids[-1]
        body = self.body([] if extern else block.children, line_of(fn if extern else block))
        fir = FnIR(name, params, outputs, body, uses, file, line_of(fn), doc, extern)
        self.compute_free(body, set())
        return fir

    # Free variables --------------------------------------------------------------------

    def refs(self, op):
        if op.kind in ("ref", "label"):
            return {op.parts[0]}
        if op.kind == "array":
            return set().union(*[self.refs(i) for i in op.items]) if op.items else set()
        if op.kind == "unit" and re.fullmatch(r"[^\W\d_]\w*", op.value):
            return {op.value}  # `[x]` may be an array of the value x (R-5): wait for x if it exists
        return set()

    def compute_free(self, body, extra):
        """Fills in `node.free`: names read by the node and defined outside its own sub-blocks."""
        local = set(extra) | set(body.own)
        free = set()
        for n in body.nodes:
            n.free = frozenset(self.rhs_free(n.rhs))
            free |= n.free
        for op in body.untils + body.collects + [o for y in body.yields for o in y]:
            free |= self.refs(op)
        return free - local

    def rhs_free(self, r):
        if isinstance(r, Call):
            s = set()
            for a in r.args:
                s |= self.refs(a.op)
            return s
        if isinstance(r, StructLit):
            return set().union(*[self.refs(o) for _, o in r.fields]) if r.fields else set()
        if isinstance(r, (Match, Handle)):
            s = self.refs(r.sel)
            for arm in r.arms:
                s |= self.compute_free(arm.body, set(arm.pat.binds))
            return s
        if isinstance(r, Loop):
            s = set().union(*[self.refs(c.init) for c in r.carries]) if r.carries else set()
            return s | self.compute_free(r.body, {c.name for c in r.carries})
        if isinstance(r, For):
            s = set()
            for c in r.clauses:
                s |= self.refs(c.lo) | (self.refs(c.hi) if c.hi else set())
            for c in r.carries:
                s |= self.refs(c.init)
            names = {c.name for c in r.clauses} | {c.name for c in r.carries}
            return s | self.compute_free(r.body, names)
        return self.refs(r)


def lower_file(src, file, tree):
    """Lowers a parsed file to (FnIR, TypeDecl, `use` lines, format version)."""
    lw = Lowerer(src, file)
    uses = [".".join(str(c) for c in u.children[0].children) for u in tree.children
            if isinstance(u, Tree) and u.data == "use_decl"]
    uses_lines = {".".join(str(c) for c in u.children[0].children): line_of(u) for u in tree.children
                  if isinstance(u, Tree) and u.data == "use_decl"}
    decl = next((c for c in tree.children if isinstance(c, Tree)
                 and c.data in ("fn_decl", "extern_decl", "type_decl")), None)
    if decl is None:
        raise LowerError("no declaration found", 1)
    doc = "\n".join(re.sub(r"^\s*/// ?", "", l) for l in src.splitlines()[:max(line_of(decl) - 1, 0)]
                    if l.strip().startswith("///"))
    header = next(c for c in tree.children if isinstance(c, Tree) and c.data == "header")
    version = int(str(header.children[0]))
    if decl.data in ("fn_decl", "extern_decl"):
        fir = lw.function(decl, uses, file, doc)
        fir.src = src
        return fir, None, uses_lines, version
    kids = [c for c in decl.children if c is not None]
    name = str(kids[0])
    ty = kids[-1]
    td = TypeDecl(name, ty, lw.text(ty), file, line_of(decl), doc)
    td.src = src
    return None, td, uses_lines, version
