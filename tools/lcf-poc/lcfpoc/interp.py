"""Dataflow interpreter (spec sections 5 to 8).

A node runs when all its inputs are ready. Among the ready nodes, the order is canonical
(by name, then by line): the same inputs give the same outputs and the same trace.
"""
import json
import re

from lark import Tree

from . import stdlib
from .ir import Call, For, FnIR, Handle, Loop, Match, Operand, StructLit
from .sim import SimError, World
from .stdlib import Builtin
from .values import (EnumVal, Fault, LcError, LcFail, PathConst, Q, Session, Struct, UnitConst, UnitError,
                     parse_unit, to_json)

SECONDS = parse_unit("s")[0]
INTS = {"i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64"}
FLOATS = {"f32", "f64"}


class ArgError(Exception):
    """Invalid run argument (CLI, MCP): reported as is."""


class Env:
    __slots__ = ("vars", "parent")

    def __init__(self, parent=None):
        self.vars, self.parent = {}, parent

    def lookup(self, name):
        e = self
        while e is not None:
            if name in e.vars:
                return True, e.vars[name]
            e = e.parent
        return False, None


class Ctx:
    """`next` carries of the enclosing loop."""

    def __init__(self, nexts=None):
        self.nexts = nexts if nexts is not None else {}


class Frame:
    def __init__(self, fn, env):
        self.fn, self.env = fn, env
        self.both = {p.name for p in fn.params} & {o[0] for o in fn.outputs}
        self.outputs = {}


def type_src(tree, src):
    return src[tree.meta.start_pos:tree.meta.end_pos]


class Machine:
    def __init__(self, lib, world, trace=False, max_iter=10_000, max_steps=100_000, max_depth=24, max_trace=20_000):
        self.lib, self.world, self.want_trace = lib, world, trace
        self.max_iter, self.max_steps, self.max_depth, self.max_trace = max_iter, max_steps, max_depth, max_trace
        self.steps, self.trace, self.seq, self.truncated = 0, [], 0, False
        self.stack, self.iters, self.callers = [], [], []
        self.results_log = []

    def here(self):
        """The library of the function that is running: names of types are resolved there."""
        return self.stack[-1].lib if self.stack else self.lib

    # Types: default values and argument conversion --------------------------------------

    def default_for(self, t, src, depth=0, lib=None):
        if t is None or depth > 8 or not isinstance(t, Tree):
            return None
        lib = lib or self.here()
        if t.data == "array_type":
            return []
        if t.data == "struct_type":
            return Struct("struct", {str(f.children[0]): self.default_for(f.children[1], src, depth + 1, lib) for f in t.children})
        if t.data == "enum_type":
            v = t.children[0]
            return EnumVal("enum", str(v.children[0]))
        if t.data != "named_type":
            return None
        parts = [str(c) for c in t.children[0].children]
        name = ".".join(parts)
        unit = next((c for c in t.children if isinstance(c, Tree) and c.data == "unit_annot"), None)
        if name in FLOATS:
            if unit is not None:
                text = type_src(unit, src)[1:-1]
                try:
                    return Q.of(0.0, text)
                except UnitError:
                    return 0.0
            return 0.0
        if name in INTS:
            return 0
        if name == "bool":
            return False
        if name == "string":
            return ""
        if name == "bytes":
            return b""
        if name in ("duration", "timestamp"):
            return Q(0.0, SECONDS, "s", 1.0)
        decl = lib.find_type(parts)
        if decl is not None:
            if decl.tree.data == "enum_type":
                return EnumVal(decl.name, str(decl.tree.children[0].children[0]))
            if decl.tree.data == "struct_type":
                return Struct(decl.name, {str(f.children[0]): self.default_for(f.children[1], decl.src, depth + 1, decl.lib)
                                          for f in decl.tree.children})
            return None if lib.is_opaque(decl) else self.default_for(decl.tree, decl.src, depth + 1, decl.lib)
        return None

    def coerce(self, v, t, src, what):
        """Converts a run argument (text or JSON) to the declared type."""
        try:
            return self._coerce(v, t, src)
        except (ValueError, TypeError, UnitError) as e:
            raise ArgError(f"argument '{what}': {e}") from None

    def _coerce(self, v, t, src, lib=None):
        lib = lib or self.lib
        if t.data == "array_type":
            if isinstance(v, str):
                v = json.loads(v)
            if not isinstance(v, list):
                raise ValueError("array expected (JSON list)")
            return [self._coerce(x, t.children[0], src, lib) for x in v]
        if t.data != "named_type":
            raise ValueError("argument type not supported")
        parts = [str(c) for c in t.children[0].children]
        name = ".".join(parts)
        unit = next((c for c in t.children if isinstance(c, Tree) and c.data == "unit_annot"), None)
        utext = type_src(unit, src)[1:-1] if unit is not None else None
        if name == "string":
            if not isinstance(v, str):
                raise ValueError("string expected")
            return v
        if name == "bool":
            if isinstance(v, bool):
                return v
            if str(v).lower() in ("true", "false"):
                return str(v).lower() == "true"
            raise ValueError("boolean expected (true or false)")
        if name in INTS:
            if isinstance(v, bool) or (isinstance(v, float) and v != int(v)):
                raise ValueError("integer expected")
            return int(v)
        if name in FLOATS or name in ("duration", "timestamp"):
            if name in ("duration", "timestamp"):
                utext = "s"
            return self._number(v, utext)
        if name == "error":
            return None
        decl = lib.find_type(parts)
        if decl is not None and lib.is_opaque(decl):
            if not isinstance(v, str):
                raise ValueError("resource name expected (the PoC test bench opens the session)")
            try:
                return self.world.open(v, 2.0, external=True)
            except LcFail as f:
                raise ValueError(f"{f.category}: {f.message}") from None
        if decl is not None:
            tree = decl.tree
            if tree.data == "struct_type":
                if isinstance(v, str):
                    v = json.loads(v)
                if not isinstance(v, dict):
                    raise ValueError(f"JSON object expected for {decl.name}")
                fields = {}
                for f in tree.children:
                    fname = str(f.children[0])
                    if fname not in v:
                        raise ValueError(f"field '{fname}' missing in {decl.name}")
                    fields[fname] = self._coerce(v[fname], f.children[1], decl.src, decl.lib)
                extra = set(v) - set(fields)
                if extra:
                    raise ValueError(f"unknown fields in {decl.name}: {', '.join(sorted(extra))}")
                return Struct(decl.name, fields)
            if tree.data == "enum_type":
                var = str(v).split(".")[-1]
                if var not in decl.enum_variants():
                    raise ValueError(f"unknown variant of {decl.name}: {var}")
                return EnumVal(decl.name, var)
            return self._coerce(v, tree, decl.src, decl.lib)
        raise ValueError(f"type '{name}' is not supported for an argument")

    def _number(self, v, utext):
        if isinstance(v, bool):
            raise ValueError("number expected")
        if isinstance(v, (int, float)):
            return Q.of(float(v), utext) if utext else float(v)
        m = re.fullmatch(r"\s*(-?(?:\d[\d_]*(?:\.\d+)?(?:[eE]-?\d+)?|nan|inf))\s*(\S.*)?", str(v))
        if not m:
            raise ValueError(f"number expected, got '{v}'")
        num, given = float(m.group(1).replace("_", "")), (m.group(2) or "").strip()
        if given:
            if not utext:
                raise ValueError(f"no unit expected, got '{given}'")
            q = Q.of(num, given)
            if q.dim != parse_unit(utext)[0]:
                raise ValueError(f"incompatible dimension: expected {utext}, got {given}")
            return q
        return Q.of(num, utext) if utext else num

    # Execution -----------------------------------------------------------------------------------

    def run(self, fname, raw_args):
        """Runs a top-level function; returns a JSON report (never raises)."""
        report = {"function": fname, "ok": False, "outputs": {}, "error": None, "fault": None}
        try:
            fn = self.lib.functions.get(fname)
            if fn is None:
                raise ArgError(f"unknown function '{fname}' (available: {', '.join(sorted(self.lib.functions)) or 'none'})")
            if fn.extern:
                raise ArgError(f"'{fname}' is an external function: call it from a function of the library")
            unknown = set(raw_args) - {p.name for p in fn.params}
            if unknown:
                raise ArgError(f"unknown inputs: {', '.join(sorted(unknown))} (inputs: {', '.join(p.name for p in fn.params)})")
            args = {}
            for p in fn.params:
                if p.name in raw_args:
                    args[p.name] = self.coerce(raw_args[p.name], p.type, fn.src, p.name)
                elif p.default is None:
                    raise ArgError(f"required input '{p.name}' is missing")
            outs = self.call_fn(fn, args)
            report["outputs"] = {k: to_json(v) for k, v in outs.items()}
            names = [o[0] for o in fn.outputs]
            err = next((outs[n] for n in names if isinstance(outs[n], LcError)), None)
            report["error"] = to_json(err) if err else None
            report["ok"] = err is None
        except ArgError as e:
            report["fault"] = {"category": "lcf.bad_argument", "message": str(e)}
        except Fault as f:
            loc = getattr(f, "loc", None)
            report["fault"] = {"category": f.category, "message": f.message}
            if loc:
                report["fault"].update(file=loc[0], line=loc[1], function=loc[2], node=loc[3])
        except RecursionError:
            report["fault"] = {"category": "lcf.recursion_limit", "message": "recursion too deep"}
        except Exception as e:  # "never crash" rule: everything becomes a report
            report["fault"] = {"category": "lcf.internal", "message": f"PoC bug: {e!r}"}
        report["sim"] = {"time_ms": round(self.world.clock * 1000, 3), "open_sessions": self.world.open_sessions(),
                         "commands": self.world.commands}
        if self.results_log:
            report["results_log"] = [to_json(r) for r in self.results_log]
        if self.want_trace:
            report["trace"] = self.trace
            if self.truncated:
                report["trace_truncated"] = True
        return report

    def call_fn(self, fn, args):
        if len(self.stack) >= self.max_depth:
            raise Fault("lcf.recursion_limit", f"nested calls beyond {self.max_depth} levels")
        env = Env()
        for p in fn.params:
            env.vars[p.name] = args[p.name] if p.name in args else p.default.value
        frame = Frame(fn, env)
        self.stack.append(fn)
        try:
            self.run_body(fn.body, env, frame, Ctx(), top=True)
        finally:
            self.stack.pop()
        out = {}
        for name, tree, _ in fn.outputs:
            if name in frame.both:
                out[name] = frame.outputs.get(name, self.default_for(tree, fn.src, lib=fn.lib))
            else:
                out[name] = env.vars.get(name, self.default_for(tree, fn.src, lib=fn.lib))
        return out

    def run_body(self, body, env, frame, ctx, top=False):
        own = set(body.own) - (frame.both if top else set())
        pending = sorted(body.nodes, key=lambda n: (n.name, n.line))
        while pending:
            ready = [n for n in pending if all(b in env.vars for b in (n.free & own))]
            if not ready:
                names = ", ".join(n.name for n in pending)
                raise Fault("lcf.deadlock", f"nodes never ready (circular dependency?): {names}")
            for n in ready:
                self.exec_node(n, env, frame, ctx, top)
                pending.remove(n)

    def exec_node(self, node, env, frame, ctx, top):
        self.steps += 1
        if self.steps > self.max_steps:
            raise Fault("lcf.step_limit", f"more than {self.max_steps} nodes executed (endless loop?)")
        entry = self.begin_trace(node, frame)
        r = node.rhs
        try:
            if isinstance(r, Call):
                outs = self.do_call(node, r, env, frame, entry)
            elif isinstance(r, StructLit):
                outs = (self.make_struct(r, env),)
            elif isinstance(r, Match):
                outs = self.do_match(r, env, frame, ctx, entry)
            elif isinstance(r, Handle):
                outs = self.do_handle(r, env, frame, ctx, entry)
            elif isinstance(r, Loop):
                outs = self.do_loop(r, env, frame, entry)
            elif isinstance(r, For):
                outs = self.do_for(r, env, frame, entry)
            else:
                outs = (self.eval(r, env),)
            self.assign(node, outs, env, frame, ctx, top, entry)
        except Fault as f:
            if not hasattr(f, "loc"):
                f.loc = (frame.fn.file, node.line, frame.fn.name, node.describe())
            raise
        except RecursionError:
            raise
        except (TypeError, ValueError, KeyError, IndexError, AttributeError, ZeroDivisionError, OverflowError) as e:
            f = Fault("lcf.internal", f"{type(e).__name__}: {e}")
            f.loc = (frame.fn.file, node.line, frame.fn.name, node.describe())
            raise f from None

    def assign(self, node, outs, env, frame, ctx, top, entry):
        out_json = {}
        if node.label:  # the label is an order source: `after: #label` waits for this node
            env.vars["#" + node.label] = True
        for i, t in enumerate(node.targets):
            if t.kind == "skip":
                continue
            if i >= len(outs):
                raise Fault("lcf.arity", f"{len(node.targets)} targets for {len(outs)} output(s)")
            v = outs[i]
            if t.kind == "next":
                ctx.nexts[t.name] = v
                out_json["next " + t.name] = v
            elif top and t.name in frame.both:
                frame.outputs[t.name] = v
                out_json[t.name] = v
            else:
                env.vars[t.name] = v
                out_json[t.name] = v
        if entry is not None:
            entry["out"] = {k: to_json(v) for k, v in out_json.items()}

    # Trace ---------------------------------------------------------------------------------------

    def begin_trace(self, node, frame):
        if not self.want_trace:
            return None
        if len(self.trace) >= self.max_trace:
            self.truncated = True
            return None
        self.seq += 1
        r = node.rhs
        op = ".".join(r.path) if isinstance(r, Call) else ("=" if isinstance(r, Operand) else type(r).__name__.lower())
        e = {"seq": self.seq, "depth": len(self.stack) - 1, "fn": frame.fn.name, "node": node.name, "op": op,
             "line": node.line, "t_ms": round(self.world.clock * 1000, 3)}
        if self.iters:
            e["iter"] = list(self.iters)
        self.trace.append(e)
        return e

    # Values -------------------------------------------------------------------------------------

    def eval(self, op, env):
        k = op.kind
        if k == "const":
            return op.value
        if k == "array":
            return [self.eval(i, env) for i in op.items]
        if k == "unit":
            return UnitConst(op.value)
        found, v = env.lookup(op.parts[0])
        if found:
            for f in op.parts[1:]:
                v = self.field(v, f)
            return v
        if op.parts[0] in self.here().types and len(op.parts) == 2:
            return EnumVal(op.parts[0], op.parts[1])
        return PathConst(op.parts)

    def field(self, v, f):
        if isinstance(v, Struct):
            if f in v.fields:
                return v.fields[f]
            raise Fault("lcf.no_field", f"'{v.type_name}' has no field '{f}'")
        if isinstance(v, LcError) or v is None:
            e = v or LcError("", "", 0)
            if f in ("category", "code", "message", "origin"):
                return getattr(e, f)
        raise Fault("lcf.no_field", f"field '{f}' not found on {type(v).__name__}")

    def make_struct(self, lit, env):
        name = ".".join(lit.path)
        return Struct(name, {f: self.eval(o, env) for f, o in lit.fields})

    # Calls --------------------------------------------------------------------------------------

    def do_call(self, node, call, env, frame, entry):
        callee = call.callee
        if isinstance(callee, tuple):  # variant constructor: Type.Variant(args)
            return (EnumVal(callee[1], callee[2], [self.eval(a.op, env) for a in call.args if a.name is None]),)
        args = {pn: self.eval(op, env) for pn, (op, _) in call.bound.items()}
        for op in call.after or ():  # order inputs: ready by construction (the node waited for them)
            if op.kind != "label":
                self.eval(op, env)
        if isinstance(callee, FnIR):
            # A user function always runs its body: it is its nodes that are skipped one by one,
            # and its `always` nodes still run (spec 7.2).
            for p in callee.params:
                if p.name not in args:
                    args[p.name] = p.default.value
            always = True
        else:
            for p in callee.params:
                if p.name not in args:
                    args[p.name] = p.default
            always = callee.always
        if entry is not None:
            entry["in"] = {k: to_json(v) for k, v in args.items()}
        errin = args.get("err")
        names = [o[0] for o in callee.outputs] if isinstance(callee, FnIR) else list(callee.outs)
        if isinstance(errin, LcError) and not always:
            if entry is not None:
                entry["skipped"] = True
            outs = self.defaults(callee)
            self.passthrough(names, outs, args)
            if "err" in names:
                outs[names.index("err")] = errin
            return tuple(outs)
        if isinstance(callee, FnIR):
            self.callers.append((frame.fn, node))
            try:
                res = self.call_fn(callee, args)
            finally:
                self.callers.pop()
            outs = [res[n] for n in names]
        else:
            outs = list(self.call_builtin(callee, args, node, frame))
        if not isinstance(callee, FnIR) and always and isinstance(errin, LcError) and "err" in names:
            outs[names.index("err")] = errin
        return tuple(outs)

    @staticmethod
    def passthrough(names, outs, args):
        """An output named like an input passes that input through when the node is skipped or fails:
        the session goes through even on error, so `always visa.close` can still close it."""
        for i, n in enumerate(names):
            if n != "err" and n in args:
                outs[i] = args[n]

    def defaults(self, callee):
        fn = callee if isinstance(callee, FnIR) else callee.decl  # an `extern` declaration has typed outputs
        if fn is not None:
            return [self.default_for(o[1], fn.src, lib=fn.lib) for o in fn.outputs]
        return list(callee.defaults)

    def origin(self, node, frame, what):
        o = [f"{frame.fn.file}:{node.line} {frame.fn.name} › {node.describe()} → {what}"]
        for fn, n in reversed(self.callers):
            o.append(f"{fn.file}:{n.line} {fn.name} › {n.describe()}")
        return o

    def call_builtin(self, b, args, node, frame):
        try:
            res = b.fn(self, **args)
        except LcFail as f:
            if "err" not in b.outs:
                raise Fault(f.category, f.message) from None
            err = getattr(f, "error", None) or LcError(f.category, f.message, f.code)
            err.origin = self.origin(node, frame, b.qname)
            outs = self.defaults(b)
            self.passthrough(b.outs, outs, args)
            outs[b.outs.index("err")] = err
            return tuple(outs)
        if "err" in b.outs:
            e = res[b.outs.index("err")]
            if isinstance(e, LcError) and not e.origin:
                e.origin = self.origin(node, frame, b.qname)
        return tuple(res)

    # Structures ----------------------------------------------------------------------------------

    def sub_body(self, body, env, frame, ctx):
        self.run_body(body, env, frame, ctx)

    def do_match(self, m, env, frame, ctx, entry):
        sel = self.eval(m.sel, env)
        if entry is not None:
            entry["in"] = {"selector": to_json(sel)}
        for arm in m.arms:
            binds = self.match_pattern(arm.pat, sel)
            if binds is not None:
                break
        else:
            raise Fault("lcf.match_unmatched", f"no arm of the match handles {sel!r}")
        child = Env(env)
        child.vars.update(binds)
        self.sub_body(arm.body, child, frame, Ctx(ctx.nexts))
        return tuple(self.eval(o, child) for o in arm.body.yields[0])

    def match_pattern(self, p, sel):
        k = p.kind
        if k == "wild":
            return {p.bind: sel} if p.bind else {}
        if k == "variant" and len(p.binds) > 1:  # R.Two(a, b): one name per datum
            if not isinstance(sel, EnumVal) or (sel.type_name, sel.variant) != tuple(p.parts):
                return None
            if len(sel.args) != len(p.binds):
                raise Fault("lcf.arity", f"pattern binds {len(p.binds)} names, the variant holds {len(sel.args)} values")
            return dict(zip(p.binds, sel.args))
        if k == "lit":
            if isinstance(p.value, bool) != isinstance(sel, bool):
                return None
            return {} if sel == p.value else None
        if k == "range":
            if not isinstance(sel, int) or isinstance(sel, bool):
                return None
            return {} if p.value <= sel and (sel <= p.hi if p.inclusive else sel < p.hi) else None
        if k == "variant":
            if not isinstance(sel, EnumVal) or len(p.parts) != 2 or sel.type_name != p.parts[0] or sel.variant != p.parts[1]:
                return None
            if p.bind:
                return {p.bind: sel.args[0] if len(sel.args) == 1 else list(sel.args)}
            return {}
        return None

    def do_handle(self, h, env, frame, ctx, entry):
        sel = self.eval(h.sel, env)
        if entry is not None:
            entry["in"] = {"error": to_json(sel)}
        for arm in h.arms:
            p, binds = arm.pat, None
            if p.kind == "ok":
                binds = {} if sel is None else None
            elif p.kind == "wild":
                binds = {p.bind: sel} if p.bind else {}
            elif isinstance(sel, LcError):
                if p.kind == "cat":
                    c = ".".join(p.parts)
                    if sel.category == c or sel.category.startswith(c + "."):
                        binds = {p.bind: sel} if p.bind else {}
                elif p.kind == "code" and sel.code == p.value:
                    binds = {p.bind: sel} if p.bind else {}
            if binds is not None:
                break
        else:
            raise Fault("lcf.handle_unmatched", f"no arm of `handle` handles {sel!r} (`_` is required)")
        child = Env(env)
        child.vars.update(binds)
        self.sub_body(arm.body, child, frame, Ctx(ctx.nexts))
        return tuple(self.eval(o, child) for o in arm.body.yields[0])

    def iterate(self, body, regs, env, frame, extra, n_iter, entry_iter):
        child = Env(env)
        child.vars = dict(regs)
        child.vars.update(extra)
        ctx = Ctx({})
        self.iters.append(n_iter)
        try:
            self.run_body(body, child, frame, ctx)
        finally:
            self.iters.pop()
        return child, ctx

    def do_loop(self, lp, env, frame, entry):
        regs = {c.name: self.eval(c.init, env) for c in lp.carries}
        cols = [[] for _ in lp.body.collects]
        n = 0
        while True:
            n += 1
            if n > self.max_iter:
                raise Fault("lcf.iteration_limit", f"loop stopped after {self.max_iter} iterations (PoC limit)")
            child, ctx = self.iterate(lp.body, regs, env, frame, {}, n, entry)
            for i, c in enumerate(lp.body.collects):
                cols[i].append(self.eval(c, child))
            done = False
            if lp.body.untils:
                done = self.eval(lp.body.untils[0], child)
                if not isinstance(done, bool):
                    raise Fault("lcf.type_error", f"`until` expects a boolean, got {done!r}")
            regs.update({k: v for k, v in ctx.nexts.items() if k in regs})
            # Default stop when an error carry holds an error (Q-7); only values of type `error`
            # are LcError, so testing by value is equivalent to testing by type.
            if done or (not lp.continue_on_error and any(isinstance(v, LcError) for v in regs.values())):
                break
        if entry is not None:
            entry["iterations"] = n
        return tuple(regs[c.name] for c in lp.carries) + tuple(cols)

    def do_for(self, f, env, frame, entry):
        seqs = []
        for c in f.clauses:
            if c.hi is not None:
                lo, hi = self.eval(c.lo, env), self.eval(c.hi, env)
                if not (isinstance(lo, int) and isinstance(hi, int)) or isinstance(lo, bool) or isinstance(hi, bool):
                    raise Fault("lcf.type_error", f"`for` bounds: integers expected ({lo!r}..{hi!r})")
                if hi - lo > self.max_iter:
                    raise Fault("lcf.iteration_limit", f"`for` of {hi - lo} iterations exceeds the limit ({self.max_iter})")
                seqs.append((c.name, range(lo, hi)))
            else:
                arr = self.eval(c.lo, env)
                if not isinstance(arr, list):
                    raise Fault("lcf.type_error", f"`for {c.name} in …` expects an array, got {type(arr).__name__}")
                seqs.append((c.name, arr))
        total = min((len(s) for _, s in seqs), default=0)
        regs = {c.name: self.eval(c.init, env) for c in f.carries}
        cols = [[] for _ in f.body.collects]
        for i in range(total):
            child, ctx = self.iterate(f.body, regs, env, frame, {name: s[i] for name, s in seqs}, i, entry)
            for j, c in enumerate(f.body.collects):
                cols[j].append(self.eval(c, child))
            regs.update({k: v for k, v in ctx.nexts.items() if k in regs})
        if entry is not None:
            entry["iterations"] = total
        return tuple(regs[c.name] for c in f.carries) + tuple(cols)


def run_function(lib, fname, args, sim_path=None, trace=False, **limits):
    """Entry point shared by the CLI and the MCP server: always returns a report (dict)."""
    try:
        world = World.load(sim_path) if sim_path else World()
    except SimError as e:
        return {"function": fname, "ok": False, "outputs": {}, "error": None,
                "fault": {"category": "lcf.bad_simulation", "message": str(e)}}
    m = Machine(lib, world, trace=trace, **limits)
    return m.run(fname, args)
