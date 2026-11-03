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
            break
        i, it = min(ready, key=lambda p: (p[1].key, p[0]))
        out.append(it)
        done.add(id(it))
        pending.remove((i, it))
    return out


def _bodies(body):
    from .ir import For, Handle, Loop, Match
    for n in body.nodes:
        r = n.rhs
        if isinstance(r, (Match, Handle)):
            for arm in r.arms:
                yield arm.body
        elif isinstance(r, (Loop, For)):
            yield r.body


def check_fn(fn):
    """[(line, message)] for each body of `fn` that is not in canonical order."""
    outs = [o[0] for o in fn.outputs]
    both = frozenset({p.name for p in fn.params} & set(outs))
    problems = []

    def walk(body, top):
        items = _lines(body, outs if top else None, both if top else frozenset())
        want = canonical(items)
        for got, exp in zip(items, want):
            if got is not exp:
                order = "; ".join(x.text for x in want)
                problems.append((got.line, f"not in canonical order (5.2): expected `{exp.text}` here, "
                                           f"found `{got.text}`. Canonical order: {order}"))
                break
        for sub in _bodies(body):
            walk(sub, False)

    walk(fn.body, True)
    return problems
