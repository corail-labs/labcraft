"""Dataflow interpreter (spec sections 5 to 8).

A node runs when all its inputs are ready. Each node that runs is a task (a generator); the scheduler
runs, among the tasks ready at the current simulated time, the first by position (spec 8, global
order), or one drawn from the schedule seed. A task that takes simulated time (a wait, a latency, a
timeout) sleeps until then, and the others go on: the waits of parallel branches overlap. The same
inputs give the same outputs and the same trace.
"""
import heapq
import json
import math
import re
import time

from lark import Tree

from . import canon, stdlib
from .ir import Call, For, FnIR, Handle, Loop, Match, Operand, StructLit
from .sim import SimError, SplitMix64, World
from .stdlib import Builtin
from .values import (EnumVal, Fault, LcError, LcFail, PathConst, Q, Session, Struct, UnitConst, UnitError,
                     parse_unit, to_json)

SECONDS = parse_unit("s")[0]
INTS = {"i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64"}
FLOATS = {"f32", "f64"}


class ArgError(Exception):
    """Invalid run argument (CLI, MCP): reported as is."""


FAULT_CODES = {"lcf.iteration_limit": "LC8001", "lcf.step_limit": "LC8002", "lcf.recursion_limit": "LC8003",
               "lcf.internal": "LC8004", "lcf.real_time_limit": "LC8006", "LC8007": "LC8007"}
SETTABLE = {"button", "switch", "led", "numeric", "text_box"}
KIND_DEFAULTS = {"button": False, "switch": False, "led": False, "text_box": "", "numeric": 0.0, "chart": []}
WAIT = object()  # what a body yields: it waits for one of its nodes to end


class Sleep:
    """What a task asks the scheduler: to be resumed at `until` (ns). `native`: a call in progress, which
    an abort lets end within the grace period; otherwise a `time.wait`, which an abort ends at once."""
    __slots__ = ("until", "native", "sessions")

    def __init__(self, until, native, sessions=()):
        self.until, self.native, self.sessions = until, native, sessions


class Task:
    """A node that runs, or the top-level call; `pos` orders it among the ready tasks (spec 8)."""
    __slots__ = ("gen", "pos", "parent", "seq", "stack", "iters", "callers", "done", "value", "sleep")


class Setting:
    __slots__ = ("element", "value", "at_ns", "applied")


class Aborted(Exception):
    def __init__(self, info, leaked):
        super().__init__(info["by"])
        self.info, self.leaked = info, leaked


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
    def __init__(self, lib, world, trace=False, max_iter=10_000, max_steps=100_000, max_depth=24, max_trace=20_000,
                 schedule_seed=None, timeout_ns=None, real_time=False, max_real_s=600.0, grace_ns=5_000_000_000):
        self.lib, self.world, self.want_trace = lib, world, trace
        self.max_iter, self.max_steps, self.max_depth, self.max_trace = max_iter, max_steps, max_depth, max_trace
        self.steps, self.trace, self.seq, self.truncated = 0, [], 0, False
        self.stack, self.iters, self.callers = [], [], []  # those of the task that runs
        self.results_log = []
        self.rng = SplitMix64(schedule_seed) if schedule_seed is not None else None
        self.timeout_ns, self.real_time, self.max_real_s, self.grace_ns = timeout_ns, real_time, max_real_s, grace_ns
        self.ready, self.sleepers, self.seq_task, self.now, self.cur = [], [], 0, 0, None
        self.t0 = time.perf_counter()
        self.positions, self.active, self.settings, self.set_i, self.top_panel, self.ui_frame = {}, {}, [], 0, {}, None

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

    def run(self, fname, raw_args, settings=()):
        """Runs a top-level function; returns a JSON report (never raises)."""
        report = {"function": fname, "status": "refused", "ok": False, "outputs": {}, "error": None, "fault": None}
        released = leaked = None
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
            self.settings = self.read_settings(fn, settings)
            self.top_panel = self.make_panel(fn, args)
            outs = self.schedule(self.call_fn(fn, args, self.top_panel))
            report["outputs"] = {k: to_json(v) for k, v in outs.items()}
            names = [o[0] for o in fn.outputs]
            err = next((outs[n] for n in names if isinstance(outs[n], LcError)), None)
            report["error"] = to_json(err) if err else None
            report["ok"] = err is None
            report["status"] = "error" if err else "ok"
        except ArgError as e:
            report["fault"] = {"category": "lcf.bad_argument", "message": str(e)}
        except Aborted as a:
            report["status"] = "aborted"
            report["abort"] = a.info
            released, leaked = self.release(a.leaked)
        except Fault as f:
            loc = getattr(f, "loc", None)
            report["status"] = "fault"
            report["fault"] = {"category": f.category, "message": f.message}
            if f.category in FAULT_CODES:
                report["fault"]["code"] = FAULT_CODES[f.category]
            if loc:
                report["fault"].update(file=loc[0], line=loc[1], function=loc[2], node=loc[3])
            released, leaked = self.release(self.wind_down())  # a fault stops the run like an abort
        except RecursionError:
            report["status"] = "fault"
            report["fault"] = {"category": "lcf.recursion_limit", "code": "LC8003", "message": "recursion too deep"}
            released, leaked = self.release(())
        except Exception as e:  # "never crash" rule: everything becomes a report
            report["status"] = "fault"
            report["fault"] = {"category": "lcf.internal", "code": "LC8004", "message": f"PoC bug: {e!r}"}
            released, leaked = self.release(())
        report["sim"] = {"time_ms": round(self.world.clock_ns / 1e6, 3), "time_ns": self.world.clock_ns,
                         "open_sessions": self.world.open_sessions(), "commands": self.world.commands}
        if released:
            report["sim"]["released"] = released  # closed by the runtime after an abort or a fault (8.2)
        if leaked:
            report["sim"]["leaked"] = leaked
        if self.settings:
            report["panel"] = [{"element": s.element, "value": to_json(s.value), "at_ms": s.at_ns / 1e6, "by": "set",
                                "applied": s.applied} for s in self.settings]
        if self.results_log:
            report["results_log"] = [to_json(r) for r in self.results_log]
        if self.want_trace:
            report["trace"] = self.trace
            if self.truncated:
                report["trace_truncated"] = True
        return report

    def release(self, leaked):
        """After an abort or a fault, the runtime closes the sessions the run opened, without running a node."""
        released = []
        for s in self.world.sessions:
            if s.is_open and not s.external and s not in leaked:
                s.is_open = False
                released.append(s.resource)
        return released, [s.resource for s in leaked]

    # Scheduler (spec 8): tasks, discrete-event clock, global order ------------------------------

    def schedule(self, root_gen):
        """Runs the top-level call. Among the tasks ready at the current time, the first by position
        runs (or one drawn from the schedule seed); when none is ready, the clock jumps to the next wake-up."""
        self.now = self.world.clock_ns
        self.t0 = time.perf_counter()
        root = self.spawn(root_gen, (), None)
        n = 0
        while not root.done:
            self.apply_settings()
            if self.ready:
                n += 1
                if not self.real_time and n % 1000 == 0 and time.perf_counter() - self.t0 > self.max_real_s:
                    raise Fault("lcf.real_time_limit", f"the run took more than {self.max_real_s} s of real time "
                                                       "(this limit alone depends on the speed of the machine)")
                self.step(self.pick(), None)
                continue
            if not self.sleepers:
                raise Fault("lcf.internal", "no task is ready or waiting, and the run has not ended")
            t = self.sleepers[0][0]
            if self.timeout_ns is not None and t >= self.timeout_ns:
                self.now = self.timeout_ns
                self.apply_settings()  # the settings of that time come first (cli.md 6.2)
                self.abort("timeout")
            self.advance_to(t)
            while self.sleepers and self.sleepers[0][0] == t:
                _, pos, seq, task = heapq.heappop(self.sleepers)
                self.make_ready((pos, seq, task))
        return root.value

    def spawn(self, gen, pos, parent):
        t = Task()
        t.gen, t.pos, t.parent, t.done, t.value, t.sleep = gen, pos, parent, False, None, None
        self.seq_task += 1
        t.seq = self.seq_task
        t.stack, t.iters, t.callers = list(self.stack), list(self.iters), list(self.callers)
        self.make_ready((pos, t.seq, t))
        return t

    def make_ready(self, entry):
        if self.rng is None:
            heapq.heappush(self.ready, entry)  # by position: the global order of spec 8
        else:
            self.ready.append(entry)  # drawn at random by `pick`

    def pick(self):
        if self.rng is None:
            return heapq.heappop(self.ready)[2]
        i = self.rng.next() % len(self.ready)
        e = self.ready[i]
        self.ready[i] = self.ready[-1]
        self.ready.pop()
        return e[2]

    def step(self, task, value):
        self.cur = task
        self.stack, self.iters, self.callers = task.stack, task.iters, task.callers
        self.world.clock_ns = self.now
        try:
            req = task.gen.send(value)
        except StopIteration as e:
            task.done, task.value = True, e.value
            if task.parent is not None:
                self.step(task.parent, None)  # its body takes the nodes that become ready
            return
        if req is not WAIT:
            task.sleep = req
            heapq.heappush(self.sleepers, (req.until, task.pos, task.seq, task))

    def advance_to(self, t):
        if self.real_time:
            delay = self.t0 + t / 1e9 - time.perf_counter()
            if delay > 0:
                time.sleep(delay)
        self.now = t

    def wind_down(self):
        """After an abort or a fault (spec 8.2): no node starts any more; waits end at once; a native call
        in progress ends within the grace period, or is abandoned when the period ends, and its session
        reported as leaked. The grace period counts on the clock of the run. Returns the leaked sessions."""
        at = self.now
        end, leaked = at, []
        for until, _, _, task in self.sleepers:
            if task.sleep.native:
                if until <= at + self.grace_ns:
                    end = max(end, until)
                else:
                    end = max(end, at + self.grace_ns)
                    leaked += [s for s in task.sleep.sessions if s not in leaked]
        self.sleepers = []
        self.advance_to(end)
        self.world.clock_ns = end
        return leaked

    def abort(self, by):
        at = self.now
        leaked = self.wind_down()
        loops = sorted(self.active.values(), key=lambda i: i["pos"])
        info = {"by": by, "at_ms": at / 1e6,  # the clock of the run (`sim`) says when it ended
                "loops": [{k: v for k, v in i.items() if k != "pos"} for i in loops],
                "warning": "the instruments are in an unknown state: closing a session does not switch a supply off"}
        raise Aborted(info, leaked)

    # Panel (spec 10) ------------------------------------------------------------------------------

    def make_panel(self, fn, args):
        """The state of the panel of a call: an element that is an input starts with its value."""
        inputs = {p.name: p for p in fn.params}
        panel = {}
        for it in fn.panel:
            if it.name in inputs:
                p = inputs[it.name]
                v = args[it.name] if it.name in args else p.default.value
            elif it.default is not None:
                v = it.default.value
            else:
                v = KIND_DEFAULTS.get(it.kind)
            panel[it.name] = [it.kind, v]
        return panel

    def panel_get(self, name):
        st = self.ui_frame.panel.get(name)
        if st is None:
            raise Fault("lcf.internal", f"no panel element '{name}'")
        value = st[1]
        if st[0] == "button" and value is True:
            st[1] = False  # latching: one read sees the press (spec 10)
        return value

    def panel_put(self, name, value):
        st = self.ui_frame.panel.get(name)
        if st is None:
            raise Fault("lcf.internal", f"no panel element '{name}'")
        st[1] = value

    def read_settings(self, fn, raw):
        """`--set ELEMENT=VALUE@TIME` (cli.md 6.4): checked against the panel, sorted by time, stable."""
        items = {it.name: it for it in fn.panel}
        ins, outs = {p.name for p in fn.params}, {o[0] for o in fn.outputs}
        out = []
        if not isinstance(raw, (list, tuple)):
            raise ArgError(f"LC8104: the settings are a list of objects with `element`, `value` and `at`, got {raw!r}")
        for r in raw:
            if not isinstance(r, dict) or not isinstance(r.get("element"), str):
                raise ArgError(f"LC8104: a setting is an object with `element`, `value` and `at`, got {r!r}")
            name, value, at = r.get("element"), r.get("value"), r.get("at")
            if name not in items:
                raise ArgError(f"LC8108: '{name}' is not an element of the panel of '{fn.name}' "
                               f"(elements: {', '.join(items) or 'none'})")
            if name in ins:
                raise ArgError(f"LC8108: '{name}' is an input of '{fn.name}': it is given as an argument")
            if name in outs:
                raise ArgError(f"LC8108: '{name}' is an output of '{fn.name}': the function writes it")
            kind = items[name].kind
            if kind not in SETTABLE:
                raise ArgError(f"LC8108: '{name}' is a `{kind}`: the proof of concept sets only "
                               f"{', '.join(sorted(SETTABLE))} elements, whose type it knows from their kind")
            v = self.element_value(kind, value, name)
            if kind == "button" and v is False:
                raise ArgError(f"LC8108: a button set to false: what it does depends on the mode of the button (Q-30)")
            s = Setting()
            s.element, s.value, s.at_ns, s.applied = name, v, parse_duration(at, f"time of the setting of '{name}'"), False
            out.append(s)
        out.sort(key=lambda s: s.at_ns)
        self.set_i = 0
        return out

    def element_value(self, kind, v, name):
        try:
            if kind in ("button", "switch", "led"):
                if isinstance(v, bool):
                    return v
                if str(v).lower() in ("true", "false"):
                    return str(v).lower() == "true"
                raise ValueError("boolean expected (true or false)")
            if kind == "numeric":
                return self._number(v, None)
            if kind == "text_box":
                if not isinstance(v, str):
                    raise ValueError("string expected")
                return v
            return v
        except (ValueError, UnitError) as e:
            raise ArgError(f"LC8104: setting of '{name}': {e}") from None

    def apply_settings(self):
        while self.set_i < len(self.settings) and self.settings[self.set_i].at_ns <= self.now:
            s = self.settings[self.set_i]
            self.top_panel[s.element][1] = s.value
            s.applied = True
            self.set_i += 1

    # Bodies and nodes ---------------------------------------------------------------------------

    def call_fn(self, fn, args, panel=None):
        if len(self.stack) >= self.max_depth:
            raise Fault("lcf.recursion_limit", f"nested calls beyond {self.max_depth} levels")
        env = Env()
        for p in fn.params:
            env.vars[p.name] = args[p.name] if p.name in args else p.default.value
        frame = Frame(fn, env)
        frame.panel = panel if panel is not None else self.make_panel(fn, args)  # panels belong to their call
        stack = self.stack
        stack.append(fn)
        try:
            yield from self.run_body(fn.body, env, frame, Ctx(), self.cur.pos, top=True)
        finally:
            stack.pop()
        out = {}
        for name, tree, _ in fn.outputs:
            if name in frame.both:
                out[name] = frame.outputs.get(name, self.default_for(tree, fn.src, lib=fn.lib))
            else:
                out[name] = env.vars.get(name, self.default_for(tree, fn.src, lib=fn.lib))
        return out

    def ordered(self, body, fn, top):
        """The nodes of a body with their canonical position (spec 5.2), computed once per body."""
        key = id(body)
        if key not in self.positions:
            outs = [o[0] for o in fn.outputs]
            both = frozenset({p.name for p in fn.params} & set(outs))
            lines = canon.canonical(canon._lines(body, outs if top else None, both if top else frozenset()))
            index = {ln.line: i for i, ln in enumerate(lines)}
            self.positions[key] = sorted(((index.get(n.line, len(lines)), n) for n in body.nodes), key=lambda p: p[0])
        return self.positions[key]

    def run_body(self, body, env, frame, ctx, base, top=False):
        """Starts each node of the body as a task as soon as its inputs are there, then waits for them."""
        own = set(body.own) - (frame.both if top else set())
        pending = list(self.ordered(body, frame.fn, top))
        me, running = self.cur, 0
        while pending or running:
            still = []
            for k, n in pending:
                if all(b in env.vars for b in (n.free & own)):
                    self.spawn(self.exec_node(n, env, frame, ctx, top), base + (k,), me)
                    running += 1
                else:
                    still.append((k, n))
            pending = still
            if not running:
                names = ", ".join(n.name for _, n in pending)
                raise Fault("lcf.deadlock", f"nodes never ready (circular dependency?): {names}")
            yield WAIT
            running -= 1

    def exec_node(self, node, env, frame, ctx, top):
        self.steps += 1
        if self.steps > self.max_steps:
            raise Fault("lcf.step_limit", f"more than {self.max_steps} nodes executed (endless loop?)")
        entry = self.begin_trace(node, frame)
        r = node.rhs
        try:
            if isinstance(r, Call):
                outs = yield from self.do_call(node, r, env, frame, entry)
            elif isinstance(r, StructLit):
                outs = (self.make_struct(r, env),)
            elif isinstance(r, Match):
                outs = yield from self.do_match(r, env, frame, ctx, entry)
            elif isinstance(r, Handle):
                outs = yield from self.do_handle(r, env, frame, ctx, entry)
            elif isinstance(r, Loop):
                outs = yield from self.do_loop(node, r, env, frame, entry)
            elif isinstance(r, For):
                outs = yield from self.do_for(r, env, frame, entry)
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
             "line": node.line, "t_ms": round(self.world.clock_ns / 1e6, 3)}
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
        # a parameter of kind element names a panel element, even one named like an input (R-1)
        args = {pn: (op.parts[0] if kind == "element" else self.eval(op, env)) for pn, (op, kind) in call.bound.items()}
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
            callers = self.callers
            callers.append((frame.fn, node))
            try:
                res = yield from self.call_fn(callee, args)
            finally:
                callers.pop()
            outs = [res[n] for n in names]
        else:
            outs = list((yield from self.call_builtin(callee, args, node, frame)))
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
        """A primitive or a native body. What it does happens at once; the simulated time it takes
        (a wait, a latency, a timeout) is then waited by this task alone, so that other branches overlap
        with it. `visa` locks its session for the duration of a call (libs/visa/README.md); the runtime
        itself locks nothing (spec 8.1)."""
        sessions = [v for v in args.values() if isinstance(v, Session)]
        for s in (sessions if b.lib == "visa" else ()):
            while getattr(s, "busy_until", 0) > self.now:
                yield Sleep(s.busy_until, False)  # not started yet: an abort ends this wait at once
        start = self.world.clock_ns
        self.ui_frame = frame
        failed = None
        try:
            res = b.fn(self, **args)
        except LcFail as f:
            failed = f
        end = self.world.clock_ns
        if end > start:
            for s in (sessions if b.lib == "visa" else ()):
                s.busy_until = end
            self.world.clock_ns = start
            yield Sleep(end, b.qname != "time.wait", sessions)
        if failed is not None:
            if "err" not in b.outs:
                raise Fault(failed.category, failed.message) from None
            err = getattr(failed, "error", None) or LcError(failed.category, failed.message, failed.code)
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
        yield from self.run_body(body, env, frame, ctx, self.cur.pos)

    def do_match(self, m, env, frame, ctx, entry):
        sel = self.eval(m.sel, env)
        if entry is not None:
            entry["in"] = {"selector": to_json(sel)}
        # `_` stands for all the others, wherever it is written (6.1): it is tried last.
        for arm, p in sorted(((a, p) for a in m.arms for p in a.pats), key=lambda pair: pair[1].kind == "wild"):
            binds = self.match_pattern(p, sel)
            if binds is not None:
                break
        else:
            raise Fault("lcf.match_unmatched", f"no arm of the match handles {sel!r}")
        child = Env(env)
        child.vars.update(binds)
        yield from self.sub_body(arm.body, child, frame, Ctx(ctx.nexts))
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
        yield from self.sub_body(arm.body, child, frame, Ctx(ctx.nexts))
        return tuple(self.eval(o, child) for o in arm.body.yields[0])

    def iterate(self, body, regs, env, frame, extra, n_iter, base):
        """One iteration: its nodes are placed after the loop, then by iteration number (spec 8)."""
        child = Env(env)
        child.vars = dict(regs)
        child.vars.update(extra)
        ctx = Ctx({})
        iters = self.iters
        iters.append(n_iter)
        try:
            yield from self.run_body(body, child, frame, ctx, base)
        finally:
            iters.pop()
        return child, ctx

    def do_loop(self, node, lp, env, frame, entry):
        regs = {c.name: self.eval(c.init, env) for c in lp.carries}
        cols = [[] for _ in lp.body.collects]
        n = 0
        info = {"function": frame.fn.name, "node": node.describe(), "line": node.line, "iteration": 0,
                "pos": self.cur.pos}
        self.active[id(info)] = info  # where each loop is, for the report of an abort
        try:
            while True:
                n += 1
                info["iteration"] = n - 1  # iterations count from 0, as in a `for` (commands.md 7, 8)
                if n > self.max_iter:
                    raise Fault("lcf.iteration_limit", f"loop stopped after {self.max_iter} iterations (PoC limit)")
                child, ctx = yield from self.iterate(lp.body, regs, env, frame, {}, n - 1, self.cur.pos + (n - 1,))
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
        finally:
            self.active.pop(id(info), None)
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
        if f.parallel and not f.carries:
            # `for parallel`: each iteration is a task of its own, placed by its index (spec 6.2, 8)
            results = [None] * total
            for i in range(total):
                self.spawn(self.parallel_iteration(f.body, regs, env, frame, {name: s[i] for name, s in seqs}, i,
                                                   results), self.cur.pos + (i,), self.cur)
            for _ in range(total):
                yield WAIT
            for child, _ in results:
                for j, c in enumerate(f.body.collects):
                    cols[j].append(self.eval(c, child))
        else:
            for i in range(total):
                child, ctx = yield from self.iterate(f.body, regs, env, frame, {name: s[i] for name, s in seqs}, i,
                                                     self.cur.pos + (i,))
                for j, c in enumerate(f.body.collects):
                    cols[j].append(self.eval(c, child))
                regs.update({k: v for k, v in ctx.nexts.items() if k in regs})
        if entry is not None:
            entry["iterations"] = total
        return tuple(regs[c.name] for c in f.carries) + tuple(cols)

    def parallel_iteration(self, body, regs, env, frame, extra, i, results):
        results[i] = yield from self.iterate(body, regs, env, frame, extra, i, self.cur.pos)


DURATION = re.compile(r"\s*(\d+(?:\.\d+)?(?:[eE][-+]?\d+)?)\s*(ns|us|µs|ms|s|min|h)\s*")
DURATION_NS = {"ns": 1, "us": 1_000, "µs": 1_000, "ms": 1_000_000, "s": 1_000_000_000, "min": 60_000_000_000,
               "h": 3_600_000_000_000}


def parse_duration(text, what):
    """A duration as the language writes it, the space before the unit optional (cli.md 6.2), in whole ns."""
    if (isinstance(text, (int, float)) and not isinstance(text, bool)) or re.fullmatch(r"\s*[-+]?[\d.]+\s*", str(text)):
        raise ArgError(f"{what}: a duration has a unit (`500 ms`, `5 s`): seconds are not assumed")
    m = DURATION.fullmatch(str(text or ""))
    if not m:
        raise ArgError(f"{what}: '{text}' is not a duration (`500 ms`, `5 s`)")
    ns = float(m.group(1)) * DURATION_NS[m.group(2)]
    if not math.isfinite(ns):
        raise ArgError(f"{what}: '{text}' is not finite")
    return round(ns)


def run_function(lib, fname, args, sim_path=None, trace=False, settings=(), **limits):
    """Entry point shared by the CLI and the MCP server: always returns a report (dict)."""
    try:
        world = World.load(sim_path) if sim_path else World()
    except SimError as e:
        return {"function": fname, "status": "refused", "ok": False, "outputs": {}, "error": None,
                "fault": {"category": "lcf.bad_simulation", "message": str(e)}}
    timeout = limits.pop("timeout", None)
    try:
        timeout_ns = parse_duration(timeout, "--timeout") if timeout is not None else None
        if timeout_ns is not None and timeout_ns <= 0:
            raise ArgError("--timeout: a duration above 0")
    except ArgError as e:
        return {"function": fname, "status": "refused", "ok": False, "outputs": {}, "error": None,
                "fault": {"category": "lcf.bad_argument", "message": str(e)}}
    m = Machine(lib, world, trace=trace, timeout_ns=timeout_ns, **limits)
    return m.run(fname, args, settings)
