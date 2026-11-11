#!/usr/bin/env python3
"""Throwaway viewer for LabCraft v0 files (.lcf, .lct).

Checks the syntax against the grammar of docs/language-spec-v0.md (section 14) and writes an
HTML page that draws each function (elkjs layout). A specification-phase tool,
meant to be replaced by the `format` crate in Rust.

    python tools/lcf-view/lcf_view.py examples -o lcf-view.html
    python tools/lcf-view/lcf_view.py examples --spec docs/language-spec-v0.md
"""
import argparse
import html
import json
import re
import sys
import unicodedata
from pathlib import Path

from lark import Lark, Token, Tree
from lark.exceptions import LarkError

# Keep in sync with the EBNF of spec section 14. Differences of form only: Lark has no reserved
# words, so NAME excludes the reserved words of 14.1 by a lookahead; line endings (14.1) are
# checked after parsing by check_lines(); `[x]` as an argument is read as a unit (unit_arg has
# priority over array_lit), the checker turns it back into an array for a value parameter (R-5);
# UNIT_NAME also covers the IDENT of a unit variable (unit_term).
RESERVED = ("labcraft use fn extern type struct enum match for parallel loop continue_on_error carry next "
            "collect until yield handle events panel layout icon true false none nan inf").split()

# Spec 14.1: Default_Ignorable_Code_Point, from DerivedCoreProperties.txt (Unicode 17 and 18). None of
# these is a letter or a mark, whatever its category. Same table as `crates/format/src/lexer.rs`.
_DEFAULT_IGNORABLE = frozenset(
    cp
    for first, last in (
        (0x00AD, 0x00AD), (0x034F, 0x034F), (0x061C, 0x061C), (0x115F, 0x1160), (0x17B4, 0x17B5),
        (0x180B, 0x180F), (0x200B, 0x200F), (0x202A, 0x202E), (0x2060, 0x206F), (0x3164, 0x3164),
        (0xFE00, 0xFE0F), (0xFEFF, 0xFEFF), (0xFFA0, 0xFFA0), (0xFFF0, 0xFFF8), (0x1BCA0, 0x1BCA3),
        (0x1D173, 0x1D17A), (0xE0000, 0xE0FFF),
    )
    for cp in range(first, last + 1)
)


def _unicode_class(prefix):
    """Regex class body for the code points of spec 14.1 whose Unicode category starts with `prefix`."""
    out, start, prev = [], None, None
    for cp in range(0x30000):
        if unicodedata.category(chr(cp)).startswith(prefix) and cp not in _DEFAULT_IGNORABLE:
            if start is None:
                start = cp
            prev = cp
        elif start is not None:
            out.append(f"\\U{start:08x}-\\U{prev:08x}")
            start = None
    return "".join(out)


_LETTER = _unicode_class("L")  # spec 14.1: letter
_CONT = _LETTER + _unicode_class("M") + "0-9_"  # spec 14.1: letter, mark, digit or "_"

GRAMMAR = r"""
start: header use_decl* DOC* (fn_decl | extern_decl | type_decl) panel_section? layout_section? icon_section?
header: "labcraft" INT
use_decl: "use" path
path: NAME ("." NAME)*

type_decl: "type" NAME generics? "=" type_expr
fn_decl: _fn_sig block
extern_decl: "extern" NAME "(" [arg ("," arg)*] ")" _fn_sig
_fn_sig: "fn" NAME generics? "(" [param ("," param)* ","?] ")" "->" "(" [output ("," output)* ","?] ")"
generics: "<" generic ("," generic)* ">"
generic: NAME (":" path)?
param: NAME ":" type_expr ("=" _constant)?
     | NAME ":" type_expr "..." -> variadic_param
output: NAME ":" type_expr

?type_expr: named_type | array_type | struct_type | enum_type | fn_type
named_type: path ("<" type_expr ("," type_expr)* ">")? unit_annot?
array_type: "[" type_expr (";" INT)? "]"
struct_type: "struct" "{" field ("," field)* ","? "}"
field: NAME ":" type_expr
enum_type: "enum" "{" variant ("," variant)* ","? "}"
variant: NAME ("(" type_expr ("," type_expr)* ")")?
fn_type: "fn" "(" [fn_arg ("," fn_arg)*] ")" "->" "(" [type_expr ("," type_expr)*] ")"
fn_arg: (NAME ":")? type_expr
unit_annot: "[" unit_expr "]"
unit_expr: unit_term (UNIT_OP unit_term)*
UNIT_OP: "*" | "·" | "/"
unit_term: (UNIT_NAME | "1") ("^" "-"? INT)? | "(" unit_expr ")"

block: "{" statement* "}"
loop_body: "{" _loop_stmt* until_stmt _loop_stmt* "}"
for_body: "{" _loop_stmt* "}"
?statement: binding | call_stmt
_loop_stmt: loop_binding | call_stmt | collect_stmt
binding: label? target ("," target)* "=" rhs
loop_binding: label? _loop_target ("," _loop_target)* "=" rhs -> binding
call_stmt: label? call
label: "#" NAME ":"
target: NAME -> t_name
      | "_" -> t_skip
_loop_target: target | next_target
next_target: "next" NAME -> t_next
?rhs: call | structure | struct_lit | operand
call: path type_args? "(" [arg ("," arg)* ","?] ")"
type_args: "<" type_expr ("," type_expr)* ">"
arg: (NAME ":")? (operand | unit_arg)
   | "after" ":" (label_ref | "(" _order_ref ("," _order_ref)* ")") -> after_arg
_order_ref: operand | label_ref
label_ref: "#" NAME
unit_arg.2: "[" unit_expr "]"
?operand: path | literal
_constant: call | struct_lit | operand
struct_lit: path "{" field_init ("," field_init)* ","? "}"
field_init: NAME ":" operand

?literal: number_lit | STRING -> string | "true" -> true | "false" -> false | "none" -> none | array_lit
number_lit: NUMBER unit_expr?
NUMBER: /-?([0-9][0-9_]*(\.[0-9][0-9_]*)?([eE]-?[0-9][0-9_]*)?|inf(?!\w))/ | /nan(?!\w)/
array_lit: "[" [operand ("," operand)*] "]"

?structure: match_expr | for_expr | loop_expr | handle_expr
match_expr: "match" operand "{" match_arm* "}"
match_arm: (bound_pattern | pattern ("," pattern)*) "=>" arm_body
bound_pattern: path "(" NAME ("," NAME)* ")" -> variant_pat
?pattern: literal | range_pat | variant_pat | "_" -> wildcard
range_pat: SIGNED_INT ".." "="? SIGNED_INT
SIGNED_INT: /-?[0-9][0-9_]*/
variant_pat: path
for_expr: "for" PARALLEL? for_in ("," for_in)* ("," carry)* for_body
PARALLEL: "parallel"
for_in: NAME "in" operand (".." operand)?
loop_expr: "loop" CONTINUE_ON_ERROR? (carry ("," carry)*)? loop_body
CONTINUE_ON_ERROR: "continue_on_error"
carry: "carry" NAME (":" type_expr)? "=" operand
collect_stmt: "collect" operand
until_stmt: "until" operand
handle_expr: "handle" operand "{" handle_arm* "}"
handle_arm: handle_pat "=>" arm_body
handle_pat: "ok" -> h_ok
          | category ("(" NAME ")")? -> h_category
          | "code" SIGNED_INT ("(" NAME ")")? -> h_code
          | "_" ("(" NAME ")")? -> h_other
category: NAME ("." NAME)* WILDCARD?
WILDCARD: ".*"
arm_body: yield_stmt | "{" statement* yield_stmt "}"
yield_stmt: "yield" (operand | "(" operand ("," operand)* ")")

panel_section: "panel" "{" panel_item* "}"
panel_item: (CONTROL | INDICATOR) NAME ":" NAME ("(" [arg ("," arg)*] ")")?
CONTROL: "control"
INDICATOR: "indicator"
layout_section: "layout" "{" layout_item* "}"
layout_item: "node" node_ref ("at" point view? | view) -> layout_node
           | "wire" terminal_ref "->" terminal_ref ("via" point+)? -> layout_wire
view: "view" VIEW_KIND
VIEW_KIND: "standard" | "compact"
terminal_ref: node_ref ("." NAME)?
node_ref: "#"? NAME
point: "(" NUMBER "," NUMBER ")"

icon_section: "icon" INT "{" icon_item* "}"
icon_item: "rect" point point ("radius" NUMBER)? icon_style -> icon_rect
         | "circle" point NUMBER icon_style -> icon_circle
         | "line" point point icon_style -> icon_line
         | "path" STRING icon_style -> icon_path
         | "text" STRING "at" point ("size" NUMBER)? icon_style -> icon_text
icon_style: FILL? ("accent" path)?
FILL: "fill"

NAME: /(?!(?:__RESERVED__)(?![__CONT__]))[__LETTER__][__CONT__]*/
UNIT_NAME: /(?!(?:__RESERVED__)(?![__CONT__]))°?[__LETTER__][__CONT__]*/
INT: /[0-9][0-9_]*/
STRING: /"(\\["\\ntr]|[^"\\\n])*"/
DOC: /\/\/\/[^\n]*/
COMMENT: /\/\/(?!\/)[^\n]*/
%import common.WS
%ignore WS
%ignore COMMENT
""".replace("__RESERVED__", "|".join(RESERVED)).replace("__LETTER__", _LETTER).replace("__CONT__", _CONT)

PARSER = Lark(GRAMMAR, start="start", parser="earley", propagate_positions=True)
ERR_NAME = re.compile(r"^(e\d*|err\w*)$")


class LineError(LarkError):
    """Line-ending rule of spec 14.1 (B1), checked after parsing."""

    def __init__(self, message, line, column=1):
        super().__init__(f"{message}, at line {line} col {column}")
        self.line, self.column = line, column


def _span(t):
    """(start line, end line, start pos, end pos) of a token or a tree; None if empty."""
    if isinstance(t, Token):
        return t.line, t.end_line, t.start_pos, t.end_pos
    m = t.meta
    if m.empty:
        return None
    return m.line, m.end_line, m.start_pos, m.end_pos


LEAVES = {"header", "use_decl", "call_stmt", "until_stmt", "collect_stmt", "yield_stmt", "panel_item",
          "layout_node", "layout_wire", "icon_rect", "icon_circle", "icon_line", "icon_path", "icon_text"}
STRUCTURES = {"match_expr", "handle_expr", "loop_expr", "for_expr"}
SEQUENCES = {"start", "block", "loop_body", "for_body", "arm_body", "match_expr", "handle_expr",
             "panel_section", "layout_section", "icon_section"}


def check_lines(src, tree):
    """Line endings (spec 14.1): they end a statement, an arm, a `use` and a `panel`/`layout`/`icon`
    item; they are ignored inside ( ) [ ], inside the braces of a struct literal or of a struct/enum
    type, and in a structure header up to its `{`. Two statements never share a line.
    Raises LineError at the first violation."""

    def flat(a, b, what):  # no line ending outside brackets in src[a:b]
        depth, i = 0, a
        while i < b:
            c = src[i]
            if c == '"':
                i += 1
                while i < b and src[i] != '"':
                    i += 2 if src[i] == "\\" else 1
            elif src.startswith("//", i):
                while i < b and src[i] != "\n":
                    i += 1
                continue
            elif c in "([{":
                depth += 1
            elif c in ")]}":
                depth -= 1
            elif c == "\n" and depth <= 0:
                line = src.count("\n", 0, i) + 1
                raise LineError(f"a line ending ends {what}: continue it inside ( ) or [ ], or join the lines", line)
            i += 1

    def visit(t):
        if not isinstance(t, Tree):
            return
        d = t.data
        if d in SEQUENCES:
            prev = None
            kids = [k for k in t.children if isinstance(k, (Tree, Token)) and _span(k)]
            if d in ("match_expr", "handle_expr", "icon_section"):
                kids = kids[1:]  # the selector or the size of the grid is part of the header
            for k in kids:
                sp = _span(k)
                if prev is not None and sp[0] == prev[1]:
                    col = k.column if isinstance(k, Token) else k.meta.column
                    raise LineError("two statements or items share a line: start a new line", sp[0], col)
                prev = sp
        if d in LEAVES or (d == "binding" and not (isinstance(t.children[-1], Tree) and t.children[-1].data in STRUCTURES)):
            sp = _span(t)
            flat(sp[2], sp[3], "a statement")
            return
        if d == "binding":  # targets = structure: one line up to the structure keyword
            flat(_span(t)[2], _span(t.children[-1])[2], "a statement")
        if d in ("match_arm", "handle_arm"):  # pattern => on the line of the start of the body
            flat(_span(t)[2], _span(t.children[-1])[2], "an arm")
        for k in t.children:
            visit(k)

    visit(tree)


HEADER = re.compile(r"[ \t]*labcraft[ \t]+1[ \t]*(//(?!/)[^\n]*)?(\n|\Z)")


def parse(src):
    """Parses a file, with its header and line endings; raises a LarkError subclass on error."""
    if not HEADER.match(src):  # spec 14.2; Lark ignores what surrounds the header, as it ignores line endings
        raise LineError("the first line must be the header `labcraft 1`", 1)
    tree = PARSER.parse(src)
    check_lines(src, tree)
    return tree


class Scope:
    def __init__(self, parent=None):
        self.parent, self.names = parent, {}

    def lookup(self, name):
        s = self
        while s:
            if name in s.names:
                return s.names[name]
            s = s.parent
        return None


class Builder:
    """Builds the ELK graph of a function from its syntax tree."""

    def __init__(self, src):
        self.src, self.n = src, 0
        self.root = {"id": "root", "children": [], "edges": []}
        self.nodes = {"root": self.root}

    def tx(self, t):
        if isinstance(t, Token):
            return str(t)
        return self.src[t.meta.start_pos:t.meta.end_pos]

    def node(self, parent, label, kind, compound=False):
        self.n += 1
        nid = f"n{self.n}"
        lines = label.split("\n")
        d = {"id": nid, "kind": kind, "label": label, "children": []}
        if compound:
            d["layoutOptions"] = {"elk.padding": "[top=28,left=12,bottom=12,right=12]"}
        else:
            d["width"] = max(len(l) for l in lines) * 7 + 18
            d["height"] = 16 * len(lines) + 12
        self.nodes[parent]["children"].append(d)
        self.nodes[nid] = d
        return nid

    def edge(self, src, dst, label=None, kind="data"):
        self.n += 1
        e = {"id": f"e{self.n}", "sources": [src], "targets": [dst], "kind": kind}
        if label:
            e["labels"] = [{"text": label, "width": len(label) * 6 + 4, "height": 12}]
        self.root["edges"].append(e)

    # Values ---------------------------------------------------------------------------

    def source(self, operand, scope, container):
        """Node that produces `operand`, and the wire label (output or field)."""
        if isinstance(operand, Tree) and operand.data == "label_ref":  # after: #close
            hit = scope.lookup("#" + str(operand.children[0]))
            if hit:
                return hit[1], None
        if isinstance(operand, Tree) and operand.data == "path":
            parts = [str(c) for c in operand.children]
            hit = scope.lookup(parts[0])
            if hit:
                nid, lbl = self.deref(hit)
                field = ".".join(parts[1:])
                return nid, (field or lbl)
            # Variant, error category or unit (R-1): constant.
            return self.node(container, ".".join(parts), "const"), None
        return self.node(container, self.tx(operand), "const"), None

    def deref(self, hit):
        if hit[0] == "node":
            return hit[1], hit[2]
        _, operand, scope, container = hit
        return self.source(operand, scope, container)

    def wire(self, operand, dst, scope, container, label=None, param=None):
        src, lbl = self.source(operand, scope, container)
        name = self.tx(operand).split(".")[0]
        kind = "err" if param == "err" or ERR_NAME.match(name) else "data"
        self.edge(src, dst, label or lbl, kind)

    # Diagram -------------------------------------------------------------------------

    def block(self, stmts, container, scope, ctx):
        bindings = []
        for st in stmts:  # Pass 1: declare the names (the order of lines has no meaning).
            if st.data not in ("binding", "call_stmt"):  # a call alone has no target
                continue
            kids = list(st.children)
            label = kids.pop(0).children[0] if kids and isinstance(kids[0], Tree) and kids[0].data == "label" else None
            rhs = kids.pop()
            targets = kids
            nid = None
            if isinstance(rhs, Tree) and rhs.data in ("call", "struct_lit", "match_expr", "loop_expr", "for_expr", "handle_expr"):
                compound = rhs.data.endswith("_expr")
                nid = self.node(container, self.title(rhs, label), rhs.data, compound)
                if label:
                    scope.names["#" + str(label)] = ("node", nid, None)
                named = [t for t in targets if t.data != "t_skip"]
                for t in targets:
                    if t.data == "t_name":
                        nm = str(t.children[0])
                        if not (ctx.get("outputs") and nm in ctx["outputs"] and nm in ctx["inputs"]):
                            scope.names[nm] = ("node", nid, nm if len(named) > 1 else None)
            elif len(targets) == 1 and targets[0].data == "t_name":
                nm = str(targets[0].children[0])
                if not (ctx.get("outputs") and nm in ctx["outputs"]):
                    scope.names[nm] = ("alias", rhs, scope, container)
            bindings.append((targets, rhs, nid))

        for targets, rhs, nid in bindings:  # Pass 2: the wires.
            if nid is not None:
                self.inside(rhs, nid, container, scope, ctx, targets)
            for i, t in enumerate(targets):
                tname = self.tx(t.children[0]) if t.children else None
                many = len(targets) > 1
                if t.data == "t_next" and ctx.get("loop"):
                    reg = self.node(ctx["loop"], f"⟲ next {tname}", "reg")
                    if nid is None:
                        self.wire(rhs, reg, scope, container)
                    else:
                        self.edge(nid, reg, tname if many else None, self.kind_of(tname))
                elif t.data == "t_name" and ctx.get("outputs") and tname in ctx["outputs"]:
                    out = ctx["outputs"][tname]
                    if nid is None:
                        self.wire(rhs, out, scope, container)
                    else:
                        self.edge(nid, out, tname if many else None, self.kind_of(tname))

        for st in stmts:
            if st.data in ("until_stmt", "collect_stmt"):
                word = st.data.split("_")[0]
                term = self.node(container, word, "term")
                self.wire(st.children[0], term, scope, container)
            elif st.data == "yield_stmt":
                term = self.node(container, "yield", "term")
                names = ctx.get("yield_names") or []
                for i, op in enumerate(st.children):
                    lbl = names[i] if i < len(names) else None
                    self.wire(op, term, scope, container, label=lbl, param="err" if lbl and ERR_NAME.match(lbl) else None)

    def kind_of(self, name):
        return "err" if name and ERR_NAME.match(name) else "data"

    def title(self, rhs, label):
        if rhs.data == "call":
            t = self.tx(rhs.children[0])
        elif rhs.data == "struct_lit":
            t = self.tx(rhs.children[0]) + " ◂"
        elif rhs.data == "match_expr":
            t = "match " + self.tx(rhs.children[0])
        elif rhs.data == "handle_expr":
            t = "handle " + self.tx(rhs.children[0])
        elif rhs.data == "loop_expr":
            t = "loop" + (" continue_on_error" if any(isinstance(c, Token) and c.type == "CONTINUE_ON_ERROR" for c in rhs.children) else "")
        else:
            t = "for" + (" parallel" if any(isinstance(c, Token) and c.type == "PARALLEL" for c in rhs.children) else "")
        return t + (f"\n#{label}" if label else "")

    def inside(self, rhs, nid, container, scope, ctx, targets):
        names = [self.tx(t.children[0]) if t.children else "_" for t in targets]
        if rhs.data == "call":
            for a in rhs.children[1:]:
                if isinstance(a, Tree) and a.data == "after_arg":  # order wires
                    for c in a.children:
                        src, _ = self.source(c, scope, container)
                        self.edge(src, nid, "after", "err")
                    continue
                if not isinstance(a, Tree) or a.data != "arg":
                    continue
                if len(a.children) == 2:
                    self.wire(a.children[1], nid, scope, container, label=str(a.children[0]), param=str(a.children[0]))
                else:
                    self.wire(a.children[0], nid, scope, container)
        elif rhs.data == "struct_lit":
            for f in rhs.children[1:]:
                self.wire(f.children[1], nid, scope, container, label=str(f.children[0]))
        elif rhs.data in ("match_expr", "handle_expr"):
            self.wire(rhs.children[0], nid, scope, container)
            for arm in rhs.children[1:]:
                *pats, body = arm.children
                arm_id = self.node(nid, ", ".join(self.tx(pat) for pat in pats), "arm", compound=True)
                inner = Scope(scope)
                for pat in pats:
                    for c in pat.children if isinstance(pat, Tree) else []:
                        if isinstance(c, Token) and c.type == "NAME" and pat.data in ("variant_pat", "h_category", "h_code", "h_other"):
                            inner.names[str(c)] = ("node", self.node(arm_id, f"{c}", "reg"), None)
                stmts = [c for c in body.children if isinstance(c, Tree)]
                self.block(stmts, arm_id, inner, {"loop": ctx.get("loop"), "yield_names": names})
        else:  # loop_expr, for_expr
            inner = Scope(scope)
            body = rhs.children[-1]
            for c in rhs.children[:-1]:
                if not isinstance(c, Tree):
                    continue
                if c.data == "carry":
                    nm = str(c.children[0])
                    reg = self.node(nid, f"⟲ {nm}", "reg")
                    self.wire(c.children[-1], reg, scope, container, param="err" if ERR_NAME.match(nm) else None)
                    inner.names[nm] = ("node", reg, None)
                elif c.data == "for_in":
                    nm = str(c.children[0])
                    idx = self.node(nid, f"▸ {nm}", "reg")
                    self.wire(c.children[1], idx, scope, container)
                    inner.names[nm] = ("node", idx, None)
            self.block(body.children, nid, inner, {"loop": nid})

    def function(self, fn):
        kids = [c for c in fn.children if c is not None]
        params = [c for c in kids if isinstance(c, Tree) and c.data in ("param", "variadic_param")]
        outputs = [c for c in kids if isinstance(c, Tree) and c.data == "output"]
        body = kids[-1]
        scope = Scope()
        for p in params:
            nm = str(p.children[0])
            label = nm + (f" = {self.tx(p.children[2])}" if len(p.children) > 2 and p.children[2] is not None else "")
            scope.names[nm] = ("node", self.node("root", label, "in"), None)
        outs = {str(o.children[0]): self.node("root", str(o.children[0]), "out") for o in outputs}
        ctx = {"outputs": outs, "inputs": {str(p.children[0]) for p in params}}
        self.block(body.children, "root", scope, ctx)
        return self.root


def view(path):
    src = path.read_text(encoding="utf-8")
    item = {"file": path.name, "source": src}
    try:
        tree = parse(src)
    except LarkError as e:
        item["error"] = str(e).strip().splitlines()[0] + _where(e)
        return item
    decl = next(c for c in tree.children if isinstance(c, Tree) and c.data in ("fn_decl", "extern_decl", "type_decl"))
    if decl.data == "fn_decl":
        try:
            item["graph"] = Builder(src).function(decl)
        except Exception as e:  # throwaway tool: show the error instead of failing
            item["error"] = f"syntax OK, but the graph could not be built: {e!r}"
    return item


def _where(e):
    line, col = getattr(e, "line", None), getattr(e, "column", None)
    return f" (line {line}, column {col})" if line else ""


STATEMENT = re.compile(r"^[#\w][\w#:, ]*=[^=>]")


def spec_units(path):
    """Code blocks of the spec to check: (line of the block in the spec, offset, full text).

    A block that contains "…" is an illustrative fragment and is not checked. The others are
    completed to form a file: header added, statements placed in a dummy function,
    type declarations split (one per file).
    """
    lines = path.read_text(encoding="utf-8").splitlines()
    blocks, cur = [], None
    for i, l in enumerate(lines, 1):
        s = l.strip()
        if cur is None and s.startswith("```"):
            cur, lang, start = [], s[3:].strip(), i + 1
        elif cur is not None and s == "```":
            if lang in ("", "text"):
                blocks.append((start, cur))
            cur = None
        elif cur is not None:
            cur.append(l)
    units, skipped = [], 0
    for start, body in blocks:
        text = "\n".join(body)
        first = next((l.strip() for l in body if l.strip() and not l.strip().startswith("//")), "")
        if "…" in text:
            skipped += 1
        elif re.match(r"^labcraft \d+$", first):
            units.append((start, 0, text))
        elif first.startswith("fn ") and "{" in text:
            units.append((start, 1, "labcraft 1\n" + text))
        elif first.startswith("type "):
            part, part_start = [], None
            for j, l in enumerate(body):
                if l.startswith("type ") and part:
                    units.append((start + part_start, 1, "labcraft 1\n" + "\n".join(part)))
                    part = []
                if not part:
                    part_start = j
                part.append(l)
            units.append((start + part_start, 1, "labcraft 1\n" + "\n".join(part)))
        elif first.startswith("layout") or first.startswith("icon"):
            units.append((start, 2, "labcraft 1\nfn spec_check() -> () {}\n" + text))
        elif STATEMENT.match(first):
            units.append((start, 2, "labcraft 1\nfn spec_check() -> () {\n" + text + "\n}"))
        else:
            skipped += 1
    return units, skipped


def markdown_files(paths):
    """Each path, a folder giving its .md files at any depth, in a fixed order."""
    files = []
    for p in paths:
        files += sorted(p.rglob("*.md")) if p.is_dir() else [p]
    return files


def check_spec(path):
    units, skipped = spec_units(path)
    errors = 0
    for start, offset, text in units:
        try:
            parse(text)
        except LarkError as e:
            errors += 1
            line = getattr(e, "line", None)
            where = f"line {start + line - 1 - offset}" if line else f"block at line {start}"
            print(f"ERROR {path.as_posix()} {where}: {str(e).strip().splitlines()[0]}")
    print(f"spec: {len(units)} blocks checked, {errors} with errors, {skipped} fragments skipped")
    return errors


PAGE = """<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>LabCraft lcf-view</title>
<style>
 :root { --bg:#fff; --fg:#1f2328; --muted:#59636e; --line:#d1d9e0; }
 body { margin:0; padding:16px; background:var(--bg); color:var(--fg); font:14px/1.45 system-ui,sans-serif; }
 h1 { font-size:20px; margin:0 0 6px; } h2 { font-size:16px; margin:26px 0 6px; border-top:1px solid var(--line); padding-top:14px; }
 .ok { color:#1a7f37; } .ko { color:#cf222e; white-space:pre-wrap; }
 .row { display:flex; gap:16px; flex-wrap:wrap; align-items:flex-start; }
 .graph { overflow:auto; max-width:100%; } pre { background:#f6f8fa; padding:10px; font-size:12px; overflow:auto; max-width:100%; margin:0; }
 p { color:var(--muted); max-width:900px; }
</style></head><body>
<h1>LabCraft: syntax check and diagrams (throwaway tool)</h1>
<p>Syntax checked against the grammar of spec v0 (section 14). Automatic elkjs layout; this is not the IDE's rendering.
Solid wire: data. Dashed: error. Blue: inputs and outputs. Yellow: constants. Purple: carries and local variables. Gray: structure terminals.</p>
<div id="items"></div>
<script src="https://cdn.jsdelivr.net/npm/elkjs@0.9.3/lib/elk.bundled.js"></script>
<script>
const ITEMS = __DATA__;
const OPTS = {"elk.algorithm":"layered","elk.direction":"RIGHT","elk.hierarchyHandling":"INCLUDE_CHILDREN",
  "elk.edgeRouting":"ORTHOGONAL","elk.layered.spacing.nodeNodeBetweenLayers":"36","elk.spacing.nodeNode":"18",
  "elk.spacing.edgeLabel":"2","elk.edgeLabels.inline":"false"};
const FILL = {in:"#e8f0fe",out:"#e8f0fe",const:"#fff4d6",reg:"#efe7fb",term:"#eceff1",call:"#ffffff",
  struct_lit:"#ffffff",match_expr:"#fafbfc",handle_expr:"#fafbfc",loop_expr:"#f7f9fc",for_expr:"#f7f9fc",arm:"#ffffff"};
const NS = "http://www.w3.org/2000/svg";
function el(tag, attrs, parent){ const e=document.createElementNS(NS,tag); for(const k in attrs) e.setAttribute(k,attrs[k]); parent.appendChild(e); return e; }
function esc(s){ const d=document.createElement("div"); d.textContent=s; return d.innerHTML; }
const elk = new ELK();
async function draw(g, host){
  g.layoutOptions = OPTS;
  const res = await elk.layout(g);
  const abs = {};
  (function walk(n, ox, oy){ abs[n.id]={x:ox+(n.x||0), y:oy+(n.y||0)}; (n.children||[]).forEach(c=>walk(c, abs[n.id].x, abs[n.id].y)); })(res,0,0);
  const svg = el("svg",{width:res.width+20,height:res.height+20,viewBox:`-10 -10 ${res.width+20} ${res.height+20}`,"font-family":"system-ui,sans-serif","font-size":"12"},host);
  (function nodes(n){ for(const c of (n.children||[])){ const p=abs[c.id]; const compound=(c.children||[]).length>0||c.kind.endsWith("_expr")||c.kind==="arm";
      el("rect",{x:p.x,y:p.y,width:c.width,height:c.height,rx:c.kind==="const"?9:4,fill:FILL[c.kind]||"#fff",
        stroke:compound?"#8c959f":(c.kind==="in"||c.kind==="out"?"#4a6fd1":c.kind==="const"?"#c99a1e":c.kind==="reg"?"#7b52c4":"#57606a"),
        "stroke-width":compound?1.2:1,"stroke-dasharray":c.kind==="arm"?"4 3":""},svg);
      const lines=c.label.split("\\n");
      lines.forEach((l,i)=>{ const t=el("text",compound?{x:p.x+8,y:p.y+16+i*14,"font-weight":"600"}:{x:p.x+c.width/2,y:p.y+17+i*16,"text-anchor":"middle"},svg); t.textContent=l; });
      nodes(c); } })(res);
  for(const e of (res.edges||[])){
    const off = e.container && abs[e.container] ? abs[e.container] : {x:0,y:0};
    for(const s of (e.sections||[])){
      const pts=[s.startPoint,...(s.bendPoints||[]),s.endPoint].map(q=>`${q.x+off.x},${q.y+off.y}`).join(" ");
      el("polyline",{points:pts,fill:"none",stroke:e.kind==="err"?"#9a6700":"#c0641a","stroke-width":e.kind==="err"?1.4:1.8,"stroke-dasharray":e.kind==="err"?"5 3":""},svg);
      const end=s.endPoint; el("circle",{cx:end.x+off.x,cy:end.y+off.y,r:2.5,fill:e.kind==="err"?"#9a6700":"#c0641a"},svg);
    }
    for(const l of (e.labels||[])){ const t=el("text",{x:l.x+off.x,y:l.y+off.y+10,"font-size":"10",fill:"#59636e"},svg); t.textContent=l.text; }
  }
}
const host = document.getElementById("items");
for(const it of ITEMS){
  const sec=document.createElement("section");
  sec.innerHTML=`<h2>${esc(it.file)} ${it.error?'<span class="ko">✗</span>':'<span class="ok">✓ syntax</span>'}</h2>`+
    (it.error?`<p class="ko">${esc(it.error)}</p>`:"")+`<div class="row"><div class="graph"></div><pre>${esc(it.source)}</pre></div>`;
  host.appendChild(sec);
  if(it.graph) draw(it.graph, sec.querySelector(".graph")).catch(err=>{ sec.querySelector(".graph").textContent="Layout error: "+err; });
}
</script></body></html>
"""


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("paths", nargs="*", help="files (.lcf/.lct) or folders")
    ap.add_argument("-o", "--out", default="lcf-view.html")
    ap.add_argument("--spec", type=Path, action="append", default=[],
                    help="also check the code blocks of a Markdown file, or of each .md file of a folder "
                         "(the spec, the skills); can be given several times")
    args = ap.parse_args()
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    spec_errors = sum(check_spec(md) for md in markdown_files(args.spec))
    if not args.paths:
        sys.exit(1 if spec_errors else 0)
    files = []
    for p in map(Path, args.paths):
        files += sorted(p.rglob("*.lc[ft]")) if p.is_dir() else [p]
    items = [view(f) for f in files]
    for it in items:
        print(("ERROR " if "error" in it else "ok    ") + it["file"] + (f": {it['error']}" if "error" in it else ""))
    data = json.dumps(items, ensure_ascii=False).replace("</", "<\\/")
    Path(args.out).write_text(PAGE.replace("__DATA__", data), encoding="utf-8")
    print(f"-> {args.out}")
    sys.exit(1 if spec_errors or any("error" in it for it in items) else 0)


if __name__ == "__main__":
    main()
