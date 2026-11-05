"""Loading a library (a folder) and static checking.

Every file error becomes a diagnostic (file, line, node): never a raw Python traceback.
"""
import difflib
import functools
import re
import sys
import tomllib
from pathlib import Path
from types import SimpleNamespace

from lark import Tree
from lark.exceptions import LarkError

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "lcf-view"))
from lcf_view import parse as _parse  # noqa: E402  (Lark grammar and line-ending rule shared with the viewer)

from . import stdlib  # noqa: E402
from .ir import (Call, For, FnIR, Handle, Loop, LowerError, Match, Operand, StructLit, TypeDecl,  # noqa: E402
                 line_of, lower_file)
from .values import UnitError, parse_unit  # noqa: E402

MAX_FILE = 1_000_000
SHIPPED = Path(__file__).resolve().parents[3] / "libs"  # the libraries shipped with LabCraft
BASE_TYPES = {"bool", "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "f32", "f64", "c64", "c128",
              "string", "bytes", "timestamp", "duration", "error"}
OPAQUE = {"queue", "ref", "waveform"}
ERR_OUT = "err"


class Diag(dict):
    def __str__(self):
        where = f"{self['file']}:{self['line']}" if self.get("line") else self["file"]
        ctx = " ".join(f"[{self[k]}]" for k in ("function", "node") if self.get(k))
        return f"{where}: {self['severity']}: {ctx + ' ' if ctx else ''}{self['message']}"


@functools.lru_cache(maxsize=512)
def parse(src):
    """Parsing is the slow part, and every load reads the same shipped libraries again."""
    return _parse(src)


class Library:
    def __init__(self, path, loader=None, name="", prefix="", diags=None):
        self.path, self.loader, self.name, self.prefix = Path(path), loader, name, prefix
        self.functions, self.types, self.diags, self.broken = {}, {}, ([] if diags is None else diags), set()
        self.files, self.deps = [], {}
        self.shipped = self.path.resolve().is_relative_to(SHIPPED)

    def diag(self, severity, file, line, message, fn=None, node=None):
        self.diags.append(Diag(severity=severity, file=self.prefix + file, line=line or 0, function=fn, node=node,
                               message=message))

    def find_type(self, parts):
        """The declaration of a type named here: `Verdict`, or `visa.session` for a library in `deps`."""
        if len(parts) == 1:
            return self.types.get(parts[0])
        dep = self.deps.get(".".join(parts[:-1]))
        return dep.types.get(parts[-1]) if dep else None

    def is_opaque(self, decl, depth=0):
        """`type session = opaque`, or an alias of such a type (3.2)."""
        t = decl.tree
        if depth > 8 or not (isinstance(t, Tree) and t.data == "named_type"):
            return False
        parts = [str(c) for c in t.children[0].children]
        if parts == ["opaque"]:
            return not [c for c in t.children[1:] if c is not None]  # not `opaque[V]`
        target = decl.lib.find_type(parts) if decl.lib else None
        return target is not None and self.is_opaque(target, depth + 1)

    def name_taken(self, name, uses):
        """R-1: a value cannot have the name of a library or of a type that appears in operands."""
        if name in uses:
            return True
        return name in self.types and not self.is_opaque(self.types[name])

    def callee(self, fn):
        if not fn.extern:
            return "fn", fn
        if fn.native is None:  # the declaration is the contract (4.1): the native body only runs it
            qname = f"{self.name}.{fn.name}" if self.name else fn.name
            base = self.native_body(fn)
            params = [stdlib.P(p.name, stdlib.REQ if p.default is None else p.default.value) for p in fn.params]
            outs = [o[0] for o in fn.outputs]
            fn.native = stdlib.Builtin(qname, params, outs, (None,) * len(outs),
                                       base.fn if base else stdlib.unavailable(qname), False, fn.doc)
            fn.native.decl = fn
        return "builtin", fn.native

    @staticmethod
    def native_body(fn):
        interface, args = fn.extern
        return stdlib.NATIVE.get((args.get("crate"), args.get("function"))) if interface == "rust" else None

    @property
    def errors(self):
        return [d for d in self.diags if d["severity"] == "error"]

    def resolve(self, path, uses):
        """Callee: ('fn', FnIR) | ('builtin', Builtin) | ('variant', type, variant) | ('nolib', lib) | None."""
        if len(path) == 1:
            if path[0] in self.functions:
                return self.callee(self.functions[path[0]])
            return None
        q = ".".join(path)
        if path[0] in stdlib.libraries():
            if path[0] not in uses:
                return "nolib", path[0]
            b = stdlib.BUILTINS.get(q)
            return ("builtin", b) if b else None
        name = ".".join(path[:-1])
        if name in uses and name in self.deps:
            fn = self.deps[name].functions.get(path[-1])
            return self.deps[name].callee(fn) if fn else None
        if path[0] in self.types and len(path) == 2:
            return "variant", path[0], path[1]
        if self.loader and self.loader.find(name, self):
            return "nolib", name
        return None


class Loader:
    """The libraries of a run: `use lib` finds a folder of the root library, then a shipped library."""

    def __init__(self, root):
        self.root, self.libs = root, {}

    def find(self, name, lib):
        """A library shipped with LabCraft sees only the others that are shipped (spec 4)."""
        places = ((self.root.path, ""), (SHIPPED, "<labcraft>/"))
        for base, prefix in places[1:] if lib.shipped else places:
            p = base.joinpath(*name.split("."))
            if p.is_dir() and (any(p.glob("*.lcf")) or any(p.glob("*.lct"))):
                return p, f"{prefix}{name.replace('.', '/')}/"
        return None

    def get(self, name, by):
        """The library `name` as `by` sees it, read with what it uses; None if there is none."""
        found = self.find(name, by)
        if not found:
            return None
        key = str(found[0].resolve())
        if key in self.libs:
            return self.libs[key]
        lib = Library(found[0], self, name, found[1], self.root.diags)
        self.libs[key] = lib  # before reading it: a cycle is reported, not followed for ever
        read_library(lib)
        return lib


def load_library(path):
    lib = Library(path)
    lib.loader = Loader(lib)
    root = lib.path
    if not root.is_dir():
        lib.diag("error", str(root), 0, "folder not found (a library is a folder)")
        return lib
    read_library(lib)
    for sub in sorted(x for x in root.iterdir() if x.is_dir() and not x.is_symlink() and not x.name.startswith(".")):
        lib.loader.get(sub.name, lib)  # a sub-folder that holds files is a sub-library, and is checked too
    _cycles(lib.loader.libs.values())
    for one in [lib, *lib.loader.libs.values()]:
        for ty in one.types.values():
            if not one.is_opaque(ty):  # the file's own `use` lines decide what it can name
                who = SimpleNamespace(uses=ty.uses, file=ty.file, name=ty.name, line=ty.line)
                Checker(one, who).check_type(ty.tree, ty.line)
        for fn in list(one.functions.values()):
            try:
                Checker(one, fn).run()
            except Exception as e:  # the checker must never crash the tool
                one.diag("error", fn.file, fn.line, f"check aborted (PoC bug): {e!r}", fn=fn.name)
    return lib


def _cycles(libs):
    """Libraries cannot use each other in a cycle (spec 4): report each cycle once."""
    done, seen = set(), set()

    def visit(lib, path):
        if lib in path:
            cycle = path[path.index(lib):]
            names = " → ".join(x.name for x in cycle + [lib])
            if frozenset(cycle) not in seen:
                seen.add(frozenset(cycle))
                lib.diag("error", lib.files[0] if lib.files else "", 0, f"libraries use each other: {names}")
            return
        if lib in done:
            return
        for dep in lib.deps.values():
            visit(dep, path + [lib])
        done.add(lib)

    for lib in libs:
        visit(lib, [])


def read_library(lib):
    root = lib.path
    files = sorted(list(root.glob("*.lcf")) + list(root.glob("*.lct")))
    if not files and not lib.name:
        lib.diag("warning", str(root), 0, "no .lcf or .lct file in this folder")
    parsed = []
    for f in files:
        lib.files.append(f.name)
        try:
            if f.stat().st_size > MAX_FILE:
                lib.diag("error", f.name, 0, f"file too large (> {MAX_FILE} bytes)")
                continue
            raw = f.read_bytes()
        except OSError as e:
            lib.diag("error", f.name, 0, f"cannot read file: {e}")
            continue
        try:
            src = raw.decode("utf-8")
        except UnicodeDecodeError as e:
            lib.diag("error", f.name, 0, f"invalid UTF-8 (byte {e.start})")
            continue
        parsed.append(_load_file(lib, f, src))
    for item in parsed:
        if item:
            _register(lib, *item)
    _read_manifest(lib)


def _read_manifest(lib):
    """Only the libraries shipped with LabCraft have a manifest until M2 (spec, section 9)."""
    path = lib.path / "labcraft.toml"
    if not lib.shipped or not path.is_file():
        return
    try:
        with open(path, "rb") as f:
            data = tomllib.load(f)
    except (OSError, tomllib.TOMLDecodeError, UnicodeDecodeError) as e:
        lib.diag("error", path.name, 0, f"cannot read the manifest: {e}")
        return
    always = data.get("always", [])
    if not (isinstance(always, list) and all(isinstance(x, str) for x in always)):
        lib.diag("error", path.name, 0, "`always` is a list of function names")
        return
    for name in always:
        fn = lib.functions.get(name)
        kind, b = lib.callee(fn) if fn else (None, None)
        if kind != "builtin":
            lib.diag("error", path.name, 0, f"`always` can only mark a native function: '{name}' (7.2)")
        else:
            b.always = True


def _load_file(lib, f, src):
    try:
        tree = parse(src)
    except LarkError as e:
        first = str(e).strip().splitlines()[0] if str(e).strip() else type(e).__name__
        line = getattr(e, "line", 0) or 0
        if isinstance(e, __import__("lark").exceptions.UnexpectedEOF):
            first = "unexpected end of file (truncated file?)"
        else:
            exp = sorted(getattr(e, "expected", None) or getattr(e, "allowed", None) or [])
            if exp:
                first += " — expected: " + ", ".join(exp[:8]) + (" …" if len(exp) > 8 else "")
        lib.diag("error", f.name, line, f"syntax error: {first}")
        lib.broken.update(re.findall(r"^\s*fn\s+(\w+)", src, re.M))
        lib.broken.update(re.findall(r"^\s*type\s+(\w+)", src, re.M))
        return None
    except RecursionError:
        lib.diag("error", f.name, 0, "nesting too deep")
        return None
    try:
        fn, ty, use_lines, version = lower_file(src, f.name, tree)
    except LowerError as e:
        lib.diag("error", f.name, e.line, e.msg)
        lib.broken.update(re.findall(r"^\s*fn\s+(\w+)", src, re.M))
        return None
    if version != 1:
        lib.diag("error", f.name, 1, f"format 'labcraft {version}' is not supported (this PoC reads version 1)")
        return None
    if fn and f.suffix != ".lcf":
        lib.diag("error", f.name, fn.line, "a function must be declared in a .lcf file")
    if ty and f.suffix != ".lct":
        lib.diag("error", f.name, ty.line, "a type must be declared in a .lct file")
    return fn, ty, use_lines


def _register(lib, fn, ty, use_lines):
    for name, line in use_lines.items():
        if name in stdlib.libraries():
            continue
        dep = lib.loader.get(name, lib)
        if dep is None:
            shipped = sorted(p.name for p in SHIPPED.iterdir() if p.is_dir()) if SHIPPED.is_dir() else []
            lib.diag("error", fn.file if fn else ty.file, line, f"unknown library '{name}' "
                     f"(available: {', '.join(stdlib.libraries() + shipped)})")
        else:
            lib.deps[name] = dep
    if fn:
        fn.lib = lib
        if fn.name in lib.functions:
            old = lib.functions[fn.name]
            lib.diag("error", fn.file, fn.line, f"function '{fn.name}' defined twice (already in {old.file}:{old.line})")
        elif fn.name in lib.types:
            lib.diag("error", fn.file, fn.line, f"'{fn.name}' is already a type of the library")
        else:
            lib.functions[fn.name] = fn
    if ty:
        ty.lib, ty.uses = lib, list(use_lines)
        if ty.name in lib.types:
            old = lib.types[ty.name]
            lib.diag("error", ty.file, ty.line, f"type '{ty.name}' defined twice (already in {old.file}:{old.line})")
        else:
            lib.types[ty.name] = ty


# Checking a function --------------------------------------------------------------------

class Scope:
    def __init__(self, names, parent=None):
        self.names, self.parent = set(names), parent

    def has(self, n):
        s = self
        while s:
            if n in s.names:
                return True
            s = s.parent
        return False

    def all(self):
        out, s = set(), self
        while s:
            out |= s.names
            s = s.parent
        return out


def callee_params(c):
    """[(name, required, kind)] of a callee (user function or primitive)."""
    if isinstance(c, FnIR):
        return [(p.name, p.default is None, "value") for p in c.params]
    return [(p.name, p.default is stdlib.REQ, p.kind) for p in c.params]


def callee_outs(c):
    return [o[0] for o in c.outputs] if isinstance(c, FnIR) else list(c.outs)


class Checker:
    def __init__(self, lib, fn):
        self.lib, self.fn, self.used, self.err_targets = lib, fn, set(), []
        self.generics = re.compile(r"^[A-Z]\w?$")

    def d(self, sev, line, msg, node=None):
        self.lib.diag(sev, self.fn.file, line, msg, fn=self.fn.name, node=node.describe() if node else None)

    def run(self):
        fn, lib = self.fn, self.lib
        names = [p.name for p in fn.params]
        if fn.extern and ERR_OUT not in [o[0] for o in fn.outputs]:
            self.d("error", fn.line, "an external function has an `err` output (R-13)")
        for n in {x for x in names if names.count(x) > 1}:
            self.d("error", fn.line, f"input '{n}' declared twice")
        outs = [o[0] for o in fn.outputs]
        for n in {x for x in outs if outs.count(x) > 1}:
            self.d("error", fn.line, f"output '{n}' declared twice")
        for p in fn.params:
            self.check_type(p.type, fn.line)
            if p.default is not None and p.default.kind not in ("const", "array"):
                self.d("error", p.default.line or fn.line, f"the default value of '{p.name}' must be a literal")
        for _, ty, _ in fn.outputs:
            self.check_type(ty, fn.line)
        for n in names + outs:
            if lib.name_taken(n, fn.uses):
                self.d("error", fn.line, f"'{n}' is the name of a library or a type (R-1)")
        if fn.extern:
            base = lib.native_body(fn)
            if base and ([p.name for p in base.params] != names or list(base.outs) != outs):
                self.d("error", fn.line, "the declaration differs from its native body (4.1): "
                                         f"{base.signature()}")
            return
        both = set(names) & set(outs)
        self.labels = {}
        scope = Scope(names)
        self.block(fn.body, scope, kind="fn", top_outputs=(outs, both))
        for tname, node in self.err_targets:
            if tname not in self.used and tname not in outs:
                self.d("warning", node.line, f"error '{tname}' is never read (7.5)", node)

    # Types -------------------------------------------------------------------------------

    def check_type(self, t, line):
        if not isinstance(t, Tree):
            return
        if t.data == "named_type":
            path = t.children[0]
            parts = [str(c) for c in path.children]
            full = ".".join(parts)
            dep = self.lib.deps.get(".".join(parts[:-1]))
            if len(parts) > 1 and self.lib.loader and self.lib.loader.find(".".join(parts[:-1]), self.lib) \
                    and ".".join(parts[:-1]) not in self.fn.uses:
                self.d("error", line_of(t) or line, f"type '{full}': library '{'.'.join(parts[:-1])}' is not imported "
                                                    f"(add 'use {'.'.join(parts[:-1])}')")
            elif not (full in BASE_TYPES or full in OPAQUE or parts[0] in self.lib.types or full in self.lib.broken
                      or parts[0] in self.lib.broken or self.generics.match(full) or self.lib.find_type(parts)
                      or (dep and parts[-1] in dep.broken)):
                near = difflib.get_close_matches(full, sorted(BASE_TYPES | set(self.lib.types)), 1)
                self.d("error", line_of(t) or line, f"unknown type '{full}'" + (f" (did you mean '{near[0]}'?)" if near else ""))
            for c in t.children[1:]:
                self.check_type(c, line)
            return
        if t.data == "unit_annot":
            return
        for c in t.children:
            self.check_type(c, line)

    # Blocs -------------------------------------------------------------------------------

    def block(self, body, parent, kind, bind=(), carries=None, top_outputs=None):
        """kind: fn | arm | loop | for. `carries`: carries visible to `next` (enclosing loop)."""
        own_list = [(t.name, n) for n in body.nodes for t in n.targets if t.kind == "name"]
        for n in body.nodes:
            if n.label:
                if n.label in self.labels:
                    self.d("error", n.line, f"label '#{n.label}' used twice (line {self.labels[n.label].line})", n)
                self.labels.setdefault(n.label, n)
        seen, outs, both = {}, (top_outputs or ([], set()))[0], (top_outputs or ([], set()))[1]
        for name, node in own_list:
            if kind == "fn" and name in both:
                continue
            if name in seen:
                self.d("error", node.line, f"value '{name}' defined twice (a value is defined only once, line {seen[name].line})", node)
            seen[name] = node
            if kind == "fn" and parent.has(name) and name not in both:
                self.d("error", node.line, f"'{name}' is already an input of the function", node)
            if self.lib.name_taken(name, self.fn.uses):
                self.d("error", node.line, f"a value cannot have the name of a library or a type: '{name}' (R-1)", node)
        if kind == "fn":
            for o in outs:
                n = sum(1 for name, _ in own_list if name == o)
                if n == 0:
                    self.d("error", self.fn.line, f"output '{o}' is not bound (add '{o} = …')")
                elif n > 1:
                    self.d("error", self.fn.line, f"output '{o}' bound {n} times")
        labels = {"#" + n.label for n in body.nodes if n.label}
        scope = Scope(set(seen) | set(bind) | labels | (set(outs) - both if kind == "fn" else set()), parent)
        if kind == "fn":  # R-2: a bound output can be read back
            scope.names |= {o for o in outs}
        loop_carries = carries
        for node in body.nodes:
            self.node(node, scope, loop_carries, kind)
        # terminators
        if body.untils and kind != "loop":
            self.d("error", body.line, "`until` is allowed only in a `loop`")
        if len(body.untils) > 1:
            self.d("error", body.line, "only one `until` per loop (R-6)")
        if body.collects and kind not in ("loop", "for"):
            self.d("error", body.line, "`collect` is allowed only in a loop")
        if body.yields and kind in ("fn", "loop", "for"):
            self.d("error", body.line, "`yield` is allowed only in an arm of `match` or `handle`")
        if kind == "arm" and len(body.yields) != 1:
            self.d("error", body.line, f"an arm must end with exactly one `yield` ({len(body.yields)} found)")
        for op in body.untils + body.collects + [o for y in body.yields for o in y]:
            self.operand(op, scope)
        self.cycles(body, kind, both)
        return scope

    def cycles(self, body, kind, both):
        definer = {}
        for n in body.nodes:
            for t in n.targets:
                if t.kind == "name" and not (kind == "fn" and t.name in both):
                    definer[t.name] = n
            if n.label:
                definer["#" + n.label] = n
        state = {}

        def visit(n, path):
            state[id(n)] = 1
            for name in sorted(n.free):
                d = definer.get(name)
                if d is None:
                    continue
                if state.get(id(d)) == 1:
                    cyc = path[path.index(d):] + [d] if d in path else [n, d]
                    names = " → ".join(x.name for x in cyc)
                    self.d("error", d.line, f"dependency cycle outside a loop: {names}", d)
                    return True
                if id(d) not in state and visit(d, path + [d]):
                    return True
            state[id(n)] = 2
            return False

        for n in body.nodes:
            if id(n) not in state and visit(n, [n]):
                break

    # Nodes -------------------------------------------------------------------------------

    def node(self, node, scope, carries, kind):
        r = node.rhs
        for t in node.targets:
            if t.kind == "next":
                if carries is None or t.name not in carries:
                    self.d("error", node.line, f"'next {t.name}': no carry '{t.name}' in the enclosing loop", node)
        if isinstance(r, Call):
            self.call(node, r, scope)
        elif isinstance(r, StructLit):
            self.struct_lit(node, r, scope)
        elif isinstance(r, Match):
            self.operand(r.sel, scope, node)
            self.arms(node, r, scope, carries, self.match_arms)
        elif isinstance(r, Handle):
            self.operand(r.sel, scope, node)
            self.arms(node, r, scope, carries, self.handle_arms)
        elif isinstance(r, Loop):
            self.loop(node, r, scope)
        elif isinstance(r, For):
            self.for_(node, r, scope)
        else:
            self.operand(r, scope, node)
            if len(node.targets) > 1:
                self.d("error", node.line, "a value can be bound to only one target", node)

    def operand(self, op, scope, node=None, kind="value"):
        if op.kind == "array":
            for i in op.items:
                self.operand(i, scope, node)
            return
        if op.kind == "label":
            if not scope.has(op.parts[0]):
                self.d("error", op.line, f"unknown label '{op.text}' in `after:`", node)
            return
        if op.kind == "unit":
            try:
                parse_unit(op.value)
            except UnitError as e:
                self.d("error", op.line, str(e), node)
            return
        if kind == "unit":
            self.d("error", op.line, f"a unit argument is written in brackets: `[{op.text}]` (R-5)", node)
            return
        if op.kind != "ref":
            return
        first = op.parts[0]
        self.used.add(first)
        if scope.has(first):
            return
        if first in self.lib.types:
            variants = self.lib.types[first].enum_variants()
            if variants is None or len(op.parts) != 2:
                self.d("error", op.line, f"'{op.text}': '{first}' is not an enumeration with this variant", node)
            elif op.parts[1] not in variants:
                self.d("error", op.line, f"unknown variant '{op.text}' (variants of {first}: {', '.join(variants)})", node)
            return
        if kind == "category" and len(op.parts) > 1:
            return
        if first in self.lib.broken:
            return
        near = difflib.get_close_matches(first, sorted(scope.all()), 1, 0.6)
        self.d("error", op.line, f"unknown value '{op.text}'" + (f" (did you mean '{near[0]}'?)" if near else ""), node)

    def call(self, node, call, scope):
        res = self.lib.resolve(call.path, self.fn.uses)
        name = ".".join(call.path)
        dep = self.lib.deps.get(".".join(call.path[:-1]))
        if res is None:
            if call.path[0] not in self.lib.broken and name not in self.lib.broken \
                    and not (dep and call.path[-1] in dep.broken):
                pool = sorted(set(self.lib.functions) | {q for q in stdlib.BUILTINS if q.split(".")[0] in self.fn.uses})
                core = {q.split(".")[1]: q for q in stdlib.BUILTINS if q.startswith("core.")}
                near = difflib.get_close_matches(name, [*pool, *core], 1, 0.6)
                near = [core.get(near[0], near[0])] if near else []
                self.d("error", node.line, f"unknown function '{name}'" + (f" (did you mean '{near[0]}'?)" if near else ""), node)
            for a in call.args:
                self.operand(a.op, scope, node)
            return
        if res[0] == "nolib":
            self.d("error", node.line, f"'{name}': library '{res[1]}' is not imported (add 'use {res[1]}')", node)
            for a in call.args:
                self.operand(a.op, scope, node)
            return
        if res[0] == "variant":
            _, tname, vname = res
            variants = self.lib.types[tname].enum_variants()
            if variants is None or vname not in variants:
                self.d("error", node.line, f"unknown variant '{name}'", node)
            call.callee = res
            for a in call.args:
                if a.op.kind == "unit":
                    a.op = self.unit_as_array(a.op, name, "value", node)
                self.operand(a.op, scope, node)
            if len(node.targets) > 1:
                self.d("error", node.line, f"'{name}' produces a single value", node)
            return
        callee = res[1]
        call.callee = callee
        params = callee_params(callee)
        pnames = [p[0] for p in params]
        pos = [a for a in call.args if a.name is None]
        named = [a for a in call.args if a.name is not None]
        seen_named = False
        for a in call.args:  # positional arguments first
            if a.name is None and seen_named:
                self.d("error", a.op.line or node.line, "a positional argument cannot follow a named argument", node)
            seen_named = seen_named or a.name is not None
        if len(pos) > len(params):
            self.d("error", node.line, f"'{name}' takes {len(params)} input(s), {len(pos)} positional arguments given", node)
        for p, a in zip(params, pos):
            call.bound[p[0]] = (a.op, p[2])
        for a in named:
            if a.name == "after":  # order input of any node (B9); the error wire is preferred when there is one
                call.after = (call.after or []) + [a.op]
                self.operand(a.op, scope, node)
                continue
            if getattr(callee, "template", False) and a.name not in pnames:
                call.bound[a.name] = (a.op, "value")  # an input per field of the template (5.4)
                continue
            if a.name not in pnames:
                near = difflib.get_close_matches(a.name, pnames, 1)
                self.d("error", a.op.line or node.line, f"'{name}' has no input '{a.name}'" +
                       (f" (did you mean '{near[0]}'?)" if near else f" (inputs: {', '.join(pnames)})"), node)
            elif a.name in call.bound:
                self.d("error", a.op.line or node.line, f"input '{a.name}' wired twice", node)
            else:
                call.bound[a.name] = (a.op, params[pnames.index(a.name)][2])
        for pn, required, _ in params:
            if required and pn not in call.bound:
                self.d("error", node.line, f"required input '{pn}' of '{name}' is not wired", node)
        if getattr(callee, "template", False) and "template" in call.bound:
            self.template(node, call, name)
        for pn, (op, kind) in list(call.bound.items()):
            if op.kind == "unit" and kind != "unit":
                op = self.unit_as_array(op, name, pn, node)
                call.bound[pn] = (op, kind)
            self.operand(op, scope, node, kind)
        outs = callee_outs(callee)
        if len(node.targets) > len(outs):
            self.d("error", node.line, f"'{name}' has {len(outs)} output(s) ({', '.join(outs)}), {len(node.targets)} targets given", node)
        if ERR_OUT in outs:
            k = outs.index(ERR_OUT)
            if k >= len(node.targets) or node.targets[k].kind == "skip":
                self.d("warning", node.line, f"`err` output of '{name}' is not wired: an error would be lost (7.5)", node)
            elif node.targets[k].kind == "name":
                self.err_targets.append((node.targets[k].name, node))

    def template(self, node, call, name):
        """R-12: the template is a literal; its fields and the named arguments match exactly."""
        op = call.bound["template"][0]
        if op.kind != "const" or not isinstance(op.value, str):
            self.d("error", node.line, f"'{name}': the template is a literal (R-12)", node)
            return
        fields, given = stdlib.template_fields(op.value), set(call.bound) - {"template"}
        for f in sorted(set(fields) - given):
            self.d("error", node.line, f"'{name}': field '{f}' of the template has no argument (R-12)", node)
        for a in sorted(given - set(fields)):
            self.d("error", node.line, f"'{name}': argument '{a}' has no field in the template (R-12)", node)

    def unit_as_array(self, op, name, pn, node):
        """`[x]` given to a value parameter: an array literal of one value (R-5)."""
        if re.fullmatch(r"[^\W\d_]\w*", op.value):
            return Operand("array", items=[Operand("ref", parts=(op.value,), line=op.line, text=op.value)],
                           line=op.line, text=op.text)
        self.d("error", op.line, f"'{name}': input '{pn}' takes a value, not the unit {op.text}", node)
        return Operand("const", value=None, line=op.line, text=op.text)

    def struct_lit(self, node, lit, scope):
        for _, op in lit.fields:
            self.operand(op, scope, node)
        name = ".".join(lit.path)
        if len(lit.path) > 1 and ".".join(lit.path[:-1]) not in self.fn.uses:
            self.d("error", node.line, f"'{name}': library '{'.'.join(lit.path[:-1])}' is not imported "
                                       f"(add 'use {'.'.join(lit.path[:-1])}')", node)
        decl = self.lib.find_type(list(lit.path))
        dep = self.lib.deps.get(".".join(lit.path[:-1]))
        if decl is None:
            if name not in self.lib.broken and not (dep and lit.path[-1] in dep.broken):
                self.d("error", node.line, f"unknown type '{name}' in struct construction", node)
            return
        t = decl.tree
        if not (isinstance(t, Tree) and t.data == "struct_type"):
            self.d("error", node.line, f"'{name}' is not a struct", node)
            return
        declared = [str(f.children[0]) for f in t.children]
        given = [f for f, _ in lit.fields]
        for f in given:
            if f not in declared:
                self.d("error", node.line, f"'{name}' has no field '{f}' (fields: {', '.join(declared)})", node)
        for f in declared:
            if f not in given:
                self.d("error", node.line, f"field '{f}' of '{name}' is not set", node)
        if len(node.targets) > 1:
            self.d("error", node.line, "a struct produces a single value", node)

    def arms(self, node, r, scope, carries, fn):
        counts = set()
        for arm in r.arms:
            self.block(arm.body, scope, "arm", bind=list(arm.pat.binds), carries=carries)
            if arm.body.yields:
                counts.add(len(arm.body.yields[0]))
        if len(counts) > 1:
            self.d("error", node.line, f"the arms do not produce the same number of values ({sorted(counts)}) (R-7)", node)
        elif counts and len(node.targets) > next(iter(counts)):
            self.d("error", node.line, f"{len(node.targets)} targets for {next(iter(counts))} value(s) produced", node)
        if not r.arms:
            self.d("error", node.line, "structure without an arm", node)
        fn(node, r)

    def match_arms(self, node, m):
        if any(a.pat.kind == "wild" for a in m.arms):
            return
        variants = [a.pat for a in m.arms if a.pat.kind == "variant"]
        lits = [a.pat.value for a in m.arms if a.pat.kind == "lit"]
        if variants:
            tname = variants[0].parts[0] if len(variants[0].parts) > 1 else None
            decl = self.lib.types.get(tname)
            allv = decl.enum_variants() if decl else None
            if allv is None:
                self.d("error", node.line, f"unknown pattern '{'.'.join(variants[0].parts)}'", node)
                return
            for v in variants:
                if v.parts[0] != tname or len(v.parts) != 2 or v.parts[1] not in allv:
                    self.d("error", node.line, f"pattern '{'.'.join(v.parts)}' is unknown for enumeration {tname}", node)
            missing = [v for v in allv if (tname, v) not in {(p.parts[0], p.parts[-1]) for p in variants}]
            if missing:
                self.d("error", node.line, f"non-exhaustive match: missing variants {', '.join(missing)} (R-8)", node)
        elif lits and all(isinstance(x, bool) for x in lits):
            if set(lits) != {True, False}:
                self.d("error", node.line, "non-exhaustive boolean match: handle `true` and `false`, or add `_` (R-8)", node)
        else:
            self.d("error", node.line, "non-exhaustive match: add a `_` arm (R-8)", node)

    def handle_arms(self, node, h):
        if sum(1 for a in h.arms if a.pat.kind == "ok") > 1:
            self.d("error", node.line, "more than one `ok` arm", node)
        if not any(a.pat.kind == "wild" for a in h.arms):
            self.d("error", node.line, "handle without a `_` arm: it is required (7.3)", node)

    def loop(self, node, lp, scope):
        names = [c.name for c in lp.carries]
        for n in {x for x in names if names.count(x) > 1}:
            self.d("error", node.line, f"carry '{n}' declared twice", node)
        for c in lp.carries:
            self.operand(c.init, scope, node)
            if c.type is not None:
                self.check_type(c.type, node.line)
        self.block(lp.body, scope, "loop", bind=names, carries=set(names))
        self.one_next(node, lp.carries, lp.body)
        if not lp.body.untils:
            self.d("warning", node.line, "loop without `until`: it will stop only on an error or at the iteration limit", node)

    def for_(self, node, f, scope):
        if f.parallel and f.carries:
            self.d("error", node.line, "`for parallel` is rejected when there is a `carry` (6.2)", node)
        if not f.clauses:
            self.d("error", node.line, "`for` without an `x in …` clause", node)
        names = [c.name for c in f.carries] + [c.name for c in f.clauses]
        for c in f.clauses:  # clause operands are evaluated outside
            self.operand(c.lo, scope, node)
            if c.hi is not None:
                self.operand(c.hi, scope, node)
        for c in f.carries:
            self.operand(c.init, scope, node)
        self.block(f.body, scope, "for", bind=names, carries={c.name for c in f.carries})
        self.one_next(node, f.carries, f.body)

    def one_next(self, node, carries, body):
        """Each carry has exactly one `next` (R-6)."""
        nexts = [t.name for n in body.nodes for t in n.targets if t.kind == "next"]
        for c in carries:
            k = nexts.count(c.name)
            if k != 1:
                self.d("error", node.line, f"carry '{c.name}' has {k} `next` (exactly one is required, R-6)", node)
