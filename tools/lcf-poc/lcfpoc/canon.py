"""Canonical order of the lines of a diagram (spec 5.2, B2): `lcf.py fmt --check`.

Algorithm, applied to each body (function, arm, loop) on its own:

- The inputs of a line are the names it reads (in its own sub-bodies too, and in `after:`) that
  another line of the same body defines; a label (`#close`) counts as a name.
- Repeatedly emit, among the lines whose inputs are all emitted, the one with the smallest key:
  1. ordinary lines, by name (Unicode order): the name of a line is its first named output that
     is neither `_` nor `next x`, or else its label;
  2. `next` lines (first output that is not `_` is `next x`), by carry name;
  3. `collect` lines, in their written order (it is the order of the outputs);
  4. the `until` line;
  5. at function level, the lines that only bind a function output to a value (`err = e3`), in
     signature order.
  A `yield` is last by grammar. Ties (equal keys) keep the written order.

The check only compares orders; comments move with their statement (spec 5.2) and are ignored here.
"""
from .ir import Operand


def _refs(op):
    if op.kind in ("ref", "label"):
        return {op.parts[0]}
    if op.kind == "array":
        return set().union(*[_refs(i) for i in op.items]) if op.items else set()
    return set()


class Line:
    def __init__(self, kind, line, deps, defines, key, text):
        self.kind, self.line, self.deps, self.defines, self.key, self.text = kind, line, deps, defines, key, text


def _lines(body, outputs=None, both=frozenset()):
    """Lines of a body, in written order. `outputs`: outputs of the function (function body only)."""
    items = []
    for n in body.nodes:
        defines = {t.name for t in n.targets if t.kind == "name" and t.name not in both}
        if n.label:
            defines.add("#" + n.label)
        named = [t for t in n.targets if t.kind != "skip"]
        if (outputs is not None and isinstance(n.rhs, Operand) and len(n.targets) == 1
                and n.targets[0].kind == "name" and n.targets[0].name in outputs):
            key = (4, outputs.index(n.targets[0].name))
        elif named and named[0].kind == "next":
            key = (1, named[0].name)
        else:
            first = next((t.name for t in named if t.kind == "name"), None)
            key = (0, first if first is not None else (n.label or ""))
        items.append(Line("node", n.line, set(n.free), defines, key, n.describe()))
    for i, op in enumerate(body.collects):
        items.append(Line("collect", op.line, _refs(op), set(), (2, i), f"collect {op.text}"))
    for op in body.untils:
        items.append(Line("until", op.line, _refs(op), set(), (3, 0), f"until {op.text}"))
    items.sort(key=lambda x: x.line)
    return items


def canonical(items):
    return _canon(items)[0]


def _canon(items):
    """(lines in canonical order, True if the lines contain a cycle)."""
    definer = {}
    for it in items:
        for name in it.defines:
            definer[name] = it
    out, done = [], set()
    pending = list(enumerate(items))
    while pending:
        ready = [(i, it) for i, it in pending
                 if all(definer[d] is it or id(definer[d]) in done for d in it.deps if d in definer)]
        if not ready:  # cycle: reported by `check`; keep the written order
            out += [it for _, it in pending]
            return out, True
        i, it = min(ready, key=lambda p: (p[1].key, p[0]))
        out.append(it)
        done.add(id(it))
        pending.remove((i, it))
    return out, False


def _strict_refs(op):
    """Names an operand reads that must be defined in the diagram. A bare unit (`[V]`) and a dotted
    path (`Verdict.Pass`, `io.timeout`: a variant or a category) are not values of the diagram."""
    if op.kind == "ref":
        return {op.parts[0]} if len(op.parts) == 1 else set()
    if op.kind == "array":
        return set().union(*[_strict_refs(i) for i in op.items]) if op.items else set()
    return _refs(op) if op.kind == "label" else set()


def _reads(body):
    """[(names read directly by the body, sub-body, names it binds)] flattened: names read by the
    body itself (not by its sub-bodies), then its sub-bodies as (body, bound names)."""
    from .ir import Call, For, Handle, Loop, Match, StructLit
    own, subs = set(), []
    for n in body.nodes:
        r = n.rhs
        if isinstance(r, Call):
            for a in r.args:
                own |= _strict_refs(a.op)
        elif isinstance(r, StructLit):
            for _, o in r.fields:
                own |= _strict_refs(o)
        elif isinstance(r, (Match, Handle)):
            own |= _strict_refs(r.sel)
            subs += [(a.body, set(a.pat.binds)) for a in r.arms]
        elif isinstance(r, Loop):
            for c in r.carries:
                own |= _strict_refs(c.init)
            subs.append((r.body, {c.name for c in r.carries}))
        elif isinstance(r, For):
            for c in r.clauses:
                own |= _strict_refs(c.lo) | (_strict_refs(c.hi) if c.hi else set())
            for c in r.carries:
                own |= _strict_refs(c.init)
            subs.append((r.body, {c.name for c in r.clauses} | {c.name for c in r.carries}))
        elif isinstance(r, Operand):
            own |= _strict_refs(r)
    for op in body.untils + body.collects + [o for y in body.yields for o in y]:
        own |= _strict_refs(op)
    return own, subs


def _defined(body):
    names = set()
    for n in body.nodes:
        names |= {t.name for t in n.targets if t.kind in ("name", "next")}
        if n.label:
            names.add("#" + n.label)
    return names


def _unknown(body, scope):
    """True if the body, or a body inside it, reads a name that nobody defines."""
    scope = scope | _defined(body)
    own, subs = _reads(body)
    if own - scope:
        return True
    return any(_unknown(b, scope | binds) for b, binds in subs)


def _bodies(body):
    from .ir import For, Handle, Loop, Match
    for n in body.nodes:
        r = n.rhs
        if isinstance(r, (Match, Handle)):
            for arm in r.arms:
                yield arm.body
        elif isinstance(r, (Loop, For)):
            yield r.body


def _subs(body):
    return _reads(body)[1]


def check_fn(fn):
    """[(line, message)] for each body of `fn` that is not in canonical order."""
    outs = [o[0] for o in fn.outputs]
    both = frozenset({p.name for p in fn.params} & set(outs))
    problems = []

    def walk(body, top, scope):
        items = _lines(body, outs if top else None, both if top else frozenset())
        want, cyclic = _canon(items)
        scope = scope | _defined(body)
        # a body with an error at edit time (cycle, unknown name) keeps its written order (5.2)
        for got, exp in zip(items, want):
            if cyclic or _unknown(body, scope):
                break
            if got is not exp:
                order = "; ".join(x.text for x in want)
                problems.append((got.line, f"not in canonical order (5.2): expected `{exp.text}` here, "
                                           f"found `{got.text}`. Canonical order: {order}"))
                break
        for sub, binds in _subs(body):
            walk(sub, False, scope | binds)

    walk(fn.body, True, {p.name for p in fn.params})
    return problems
