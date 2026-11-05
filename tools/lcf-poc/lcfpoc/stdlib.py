"""The primitives of the language (spec section 9): core, array, string, time, errors; and the native
bodies of `extern rust` functions, as the crates of the libraries will provide them. Each declares
its inputs, its outputs and their default values (used when the node is skipped because of an error)."""
import math
import re

from .values import (Fault, LcError, LcFail, PathConst, Q, Session, UnitConst, ZERO, dim_label, from_si, is_num,
                     parse_unit, UnitError)

REQ = object()


class P:
    """Parameter of a primitive. `kind`: value | unit (unit name) | category (error category)."""

    def __init__(self, name, default=REQ, kind="value", type=""):
        self.name, self.default, self.kind, self.type = name, default, kind, type


class Builtin:
    def __init__(self, qname, params, outs, defaults, fn, always, doc, template=False):
        self.qname, self.params, self.outs, self.defaults = qname, params, outs, defaults
        self.fn, self.always, self.doc, self.template = fn, always, doc, template
        self.decl = None  # the `extern` declaration it runs, whose signature is the contract (4.1)
        self.lib, _, self.short = qname.rpartition(".")

    def signature(self):
        ps = ", ".join(p.name + (f": {p.type}" if p.type else "") + ("" if p.default is REQ else " = …") for p in self.params)
        return f"{'always ' if self.always else ''}fn {self.qname}({ps}) -> ({', '.join(self.outs)})"


BUILTINS = {}
NATIVE = {}  # (crate, function) of an `extern rust` declaration -> its body in the proof of concept


def builtin(qname, params, outs, defaults=None, always=False, template=False):
    def deco(fn):
        BUILTINS[qname] = Builtin(qname, params, outs, defaults or (None,) * len(outs), fn, always,
                                  (fn.__doc__ or "").strip(), template)
        return fn
    return deco


def native(crate, qname, params, outs, defaults=None):
    """A native body. `always` comes from the manifest of the library, as in the spec (7.2)."""
    def deco(fn):
        NATIVE[crate, qname.rpartition(".")[2]] = Builtin(qname, params, outs, defaults or (None,) * len(outs), fn,
                                                          False, (fn.__doc__ or "").strip())
        return fn
    return deco


def unavailable(qname):
    """The body of an external function that does not run yet: `extern.unavailable` on `err` (4.1)."""
    def body(m, **_):
        raise LcFail("extern.unavailable", f"{qname}: this external function does not run yet (4.1)")
    return body


def libraries():
    return sorted({b.lib for b in BUILTINS.values() if b.lib})


# core --------------------------------------------------------------------------------------

def _kind(x):
    return dim_label(x.dim) if isinstance(x, Q) else "dimensionless"


def _need_nums(op, a, b):
    if not (is_num(a) and is_num(b)):
        raise Fault("lcf.type_error", f"{op}: numbers expected (got {type(a).__name__} and {type(b).__name__})")


def _same_dim(op, a, b):
    _need_nums(op, a, b)
    if isinstance(a, Q) != isinstance(b, Q) or (isinstance(a, Q) and a.dim != b.dim):
        raise Fault("lcf.unit_mismatch", f"{op}: incompatible dimensions ({_kind(a)} and {_kind(b)})")


def _div(x, y):
    if y == 0:
        return math.nan if x == 0 or math.isnan(x) else math.copysign(math.inf, x) * (math.copysign(1, y))
    return x / y


@builtin("core.add", [P("a"), P("b")], ["result"], (0.0,))
def add(m, a, b):
    """a + b; same dimensions required, the unit of the result is that of `a`."""
    _same_dim("add", a, b)
    return (a.with_si(a.si + b.si),) if isinstance(a, Q) else (a + b,)


@builtin("core.sub", [P("a"), P("b")], ["result"], (0.0,))
def sub(m, a, b):
    """a - b; same dimensions required."""
    _same_dim("sub", a, b)
    return (a.with_si(a.si - b.si),) if isinstance(a, Q) else (a - b,)


@builtin("core.mul", [P("a"), P("b")], ["result"], (0.0,))
def mul(m, a, b):
    """a × b; the dimension exponents add up (Ω × A = V)."""
    _need_nums("mul", a, b)
    if isinstance(a, Q) and isinstance(b, Q):
        return (from_si(a.si * b.si, tuple(x + y for x, y in zip(a.dim, b.dim))),)
    if isinstance(a, Q):
        return (a.with_si(a.si * b),)
    if isinstance(b, Q):
        return (b.with_si(a * b.si),)
    return (a * b,)


@builtin("core.div", [P("a"), P("b")], ["result"], (0.0,))
def div(m, a, b):
    """a ÷ b; the exponents subtract (V ÷ A = Ω). Division by zero: inf or nan (IEEE)."""
    _need_nums("div", a, b)
    if isinstance(a, Q) and isinstance(b, Q):
        return (from_si(_div(a.si, b.si), tuple(x - y for x, y in zip(a.dim, b.dim))),)
    if isinstance(a, Q):
        return (a.with_si(_div(a.si, b)),)
    if isinstance(b, Q):
        return (from_si(_div(a, b.si), tuple(-x for x in b.dim)),)
    return (_div(a, b),)


@builtin("core.abs", [P("x")], ["result"], (0.0,))
def abs_(m, x):
    """Absolute value."""
    return (x.with_si(abs(x.si)) if isinstance(x, Q) else abs(x),)


@builtin("core.neg", [P("x")], ["result"], (0.0,))
def neg(m, x):
    """Negation."""
    return (x.with_si(-x.si) if isinstance(x, Q) else -x,)


@builtin("core.sqrt", [P("x")], ["result"], (0.0,))
def sqrt(m, x):
    """Square root; the dimension exponents are halved (rejected if any is odd)."""
    if isinstance(x, Q):
        if any(e % 2 for e in x.dim):
            raise Fault("lcf.unit_mismatch", f"sqrt: dimension {_kind(x)} has no square root")
        return (from_si(math.sqrt(x.si) if x.si >= 0 else math.nan, tuple(e // 2 for e in x.dim)),)
    return (math.sqrt(x) if x >= 0 else math.nan,)


@builtin("core.min", [P("a"), P("b")], ["result"], (0.0,))
def min_(m, a, b):
    """The smaller of the two."""
    _same_dim("min", a, b)
    return ((a if (a.si if isinstance(a, Q) else a) <= (b.si if isinstance(b, Q) else b) else b),)


@builtin("core.max", [P("a"), P("b")], ["result"], (0.0,))
def max_(m, a, b):
    """The larger of the two."""
    _same_dim("max", a, b)
    return ((a if (a.si if isinstance(a, Q) else a) >= (b.si if isinstance(b, Q) else b) else b),)


def _cmp(name, op):
    def f(m, a, b):
        if name in ("eq", "ne") and not (is_num(a) and is_num(b)):
            return ((a == b) if name == "eq" else (a != b),)
        _same_dim(name, a, b)
        x, y = (a.si, b.si) if isinstance(a, Q) else (a, b)
        return (op(x, y),)
    f.__doc__ = f"Comparison `{name}`; identical dimensions required."
    builtin("core." + name, [P("a"), P("b")], ["result"], (False,))(f)


for _n, _o in (("lt", lambda x, y: x < y), ("le", lambda x, y: x <= y), ("gt", lambda x, y: x > y),
               ("ge", lambda x, y: x >= y), ("eq", lambda x, y: x == y), ("ne", lambda x, y: x != y)):
    _cmp(_n, _o)


def _bool(op, x):
    if not isinstance(x, bool):
        raise Fault("lcf.type_error", f"{op}: boolean expected (got {type(x).__name__})")
    return x


@builtin("core.and", [P("a"), P("b")], ["result"], (False,))
def and_(m, a, b):
    """Logical AND."""
    return (_bool("and", a) and _bool("and", b),)


@builtin("core.or", [P("a"), P("b")], ["result"], (False,))
def or_(m, a, b):
    """Logical OR."""
    return (_bool("or", a) or _bool("or", b),)


@builtin("core.not", [P("a")], ["result"], (False,))
def not_(m, a):
    """Logical NOT."""
    return (not _bool("not", a),)


@builtin("core.select", [P("condition"), P("if_true"), P("if_false")], ["result"])
def select(m, condition, if_true, if_false):
    """`if_true` if the condition is true, otherwise `if_false`."""
    return (if_true if _bool("select", condition) else if_false,)


def _unit_of(u, who="with_unit"):
    if not isinstance(u, UnitConst):
        raise Fault("lcf.type_error", f"{who}: unit in brackets expected as the second argument (`[V]`)")
    try:
        return u.text, parse_unit(u.text)
    except UnitError as e:
        raise Fault("lcf.unit_unknown", f"{who}: {e}") from None


@builtin("core.with_unit", [P("x"), P("unit", kind="unit")], ["result"], (0.0,))
def with_unit(m, x, unit):
    """Attaches a unit to a number: `core.with_unit(raw, [V])`. Rejected if `x` already has one."""
    if x is None:  # the default value of a skipped generic node, which the PoC does not type (7.2)
        x = 0.0
    if not is_num(x) or isinstance(x, Q):
        raise Fault("lcf.type_error", "with_unit: unitless number expected")
    text, _ = _unit_of(unit)
    return (Q.of(float(x), text),)


@builtin("core.unitless", [P("x"), P("unit", kind="unit")], ["result"], (0.0,))
def unitless(m, x, unit):
    """Removes the unit by expressing the number in the requested unit: `core.unitless(230 mV, [V])` gives 0.23."""
    if x is None:  # the default value of a skipped generic node, which the PoC does not type (7.2)
        return (0.0,)
    if not isinstance(x, Q):
        raise Fault("lcf.type_error", "unitless: quantity with a unit expected")
    _, (dim, scale) = _unit_of(unit, "unitless")
    if dim != x.dim:
        raise Fault("lcf.unit_mismatch", f"unitless: {x!r} cannot be expressed in {unit.text}")
    return (x.si / scale,)


# array -------------------------------------------------------------------------------------

@builtin("array.len", [P("array")], ["length"], (0,))
def array_len(m, array):
    """Number of elements."""
    return (len(array),)


@builtin("array.get", [P("array"), P("index"), P("err", None)], ["item", "err"])
def array_get(m, array, index, err=None):
    """Element at `index` (from 0); `array.index_out_of_range` otherwise."""
    if not isinstance(index, int) or isinstance(index, bool) or not 0 <= index < len(array):
        raise LcFail("array.index_out_of_range", f"index {index!r} outside the array (size {len(array)})")
    return (array[index], None)


@builtin("array.push", [P("array"), P("item")], ["array"], ([],))
def array_push(m, array, item):
    """New array with `item` appended at the end."""
    return (list(array) + [item],)


# time --------------------------------------------------------------------------------------

def _seconds(x, who):
    if isinstance(x, Q) and x.dim == parse_unit("s")[0]:
        return x.si
    raise Fault("lcf.unit_mismatch", f"{who}: duration expected (a unit of time), got {x!r}")


@builtin("time.now", [], ["timestamp"], (Q(0.0, parse_unit("s")[0], "s", 1.0),))
def time_now(m):
    """Simulated clock (seconds since the start of the run). Order it with `after:`."""
    return (Q(m.world.clock, parse_unit("s")[0], "s", 1.0),)


@builtin("time.wait", [P("duration"), P("err", None)], ["err"])
def time_wait(m, duration, err=None):
    """Waits: advances the simulated clock without sleeping."""
    m.world.clock += max(_seconds(duration, "time.wait"), 0.0)
    return (None,)


@builtin("time.elapsed", [P("since")], ["duration"], (Q(0.0, parse_unit("s")[0], "s", 1.0),))
def time_elapsed(m, since):
    """Time elapsed since a `time.now` timestamp."""
    return (Q(m.world.clock - _seconds(since, "time.elapsed"), parse_unit("s")[0], "s", 1.0),)


# errors ------------------------------------------------------------------------------------

@builtin("errors.raise_if", [P("condition"), P("category", kind="category"), P("message", ""), P("err", None)], ["err"])
def raise_if(m, condition, category, message="", err=None):
    """Produces the error `category` if the condition is true (otherwise `none`)."""
    if not isinstance(category, PathConst):
        raise Fault("lcf.type_error", "errors.raise_if: category expected as the second argument")
    if _bool("errors.raise_if", condition):
        raise LcFail(category.text, message)
    return (None,)


@builtin("errors.make", [P("category", kind="category"), P("code", 0), P("message", "")], ["err"])
def errors_make(m, category, code=0, message=""):
    """An error of the given category and code."""
    if not isinstance(category, PathConst):
        raise Fault("lcf.type_error", "errors.make: category expected as the first argument")
    return (LcError(category.text, message, code),)


@builtin("errors.merge", [P("a"), P("b"), P("c", None), P("d", None), P("e", None), P("f", None)], ["err"])
def merge(m, a, b, c=None, d=None, e=None, f=None):
    """Merges errors: the first non-empty one becomes the main error, the others go in `others`."""
    errs = [x for x in (a, b, c, d, e, f) if isinstance(x, LcError)]
    if not errs:
        return (None,)
    first = errs[0]
    return (LcError(first.category, first.message, first.code, first.origin, first.others + errs[1:]),)


@builtin("errors.is_none", [P("err")], ["result"], (True,), always=True)
def is_none(m, err):
    """True if there is no error."""
    return (err is None,)


# string ------------------------------------------------------------------------------------

FIELD = re.compile(r"\{\{|\}\}|\{([^\W\d_]\w*)(?::([^{}]*))?\}")


def template_fields(template):
    """Names of the `{name}` fields of a `string.format` template (5.4)."""
    return [m.group(1) for m in FIELD.finditer(template) if m.group(1)]


def _format_field(v, spec):
    m = re.fullmatch(r"(?:\.(\d+))?\s*(.*)", spec or "")
    digits, unit = m.group(1), m.group(2).strip()
    number = v
    if isinstance(v, Q):
        if unit:
            try:
                dim, scale = parse_unit(unit)
            except UnitError as e:
                raise Fault("lcf.unit_unknown", f"string.format: {e}") from None
            if dim != v.dim:
                raise Fault("lcf.unit_mismatch", f"string.format: {v!r} cannot be expressed in {unit}")
            number = v.si / scale
        else:
            number, unit = v.value, v.unit
    elif unit:
        raise Fault("lcf.unit_mismatch", f"string.format: {v!r} has no unit to express in {unit}")
    if not is_num(number):
        return str(v)
    text = f"{number:.{digits}f}" if digits is not None else f"{number:g}"
    return f"{text} {unit}" if unit else text


@builtin("string.format", [P("template", type="string")], ["text"], ("",), template=True)
def string_format(m, /, template, **values):
    """Text from a literal template: each `{name}` field is a named input (5.4)."""
    def field(f):
        if f.group(0) in ("{{", "}}"):
            return f.group(0)[0]
        return _format_field(values[f.group(1)], f.group(2))
    return (FIELD.sub(field, template),)


@builtin("string.split", [P("text", type="string"), P("separator", type="string")], ["parts"], ([],))
def string_split(m, text, separator):
    """Splits a text at each separator."""
    if not separator:
        raise Fault("lcf.bad_argument", "string.split: empty separator")
    return (text.split(separator),)


@builtin("string.to_f64", [P("text", type="string"), P("err", None)], ["value", "err"], (0.0, None))
def string_to_f64(m, text, err=None):
    """Number read from a text; `string.parse` if it is not one."""
    t = text.strip()
    if not re.fullmatch(r"[+-]?(?:(?:[0-9]+(?:\.[0-9]*)?|\.[0-9]+)(?:[eE][+-]?[0-9]+)?|(?i:nan|inf))", t):
        raise LcFail("string.parse", f"not a number: {text!r}")
    return (float(t), None)


@builtin("string.to_i32", [P("text", type="string"), P("err", None)], ["value", "err"], (0, None))
def string_to_i32(m, text, err=None):
    """Integer read from a text; `string.parse` if it is not one."""
    t = text.strip()
    if not re.fullmatch(r"[+-]?[0-9]+", t) or not -2**31 <= int(t) < 2**31:
        raise LcFail("string.parse", f"not an i32: {text!r}")
    return (int(t), None)


# visa (crate labcraft-visa) -----------------------------------------------------------------

VISA = "labcraft-visa"


def _live(s):
    if not isinstance(s, Session) or not s.is_open:
        raise LcFail("visa.invalid_session", "session missing or closed")
    return s


@native(VISA, "visa.open", [P("resource", type="string"), P("timeout", Q(2.0, parse_unit("s")[0], "s", 1.0)), P("err", None)],
        ["session", "err"])
def visa_open(m, resource, timeout, err=None):
    """Opens a session on a resource (`SIM::…`: simulated instrument)."""
    return (m.world.open(resource, _seconds(timeout, "visa.open")), None)


@native(VISA, "visa.close", [P("session"), P("err", None)], ["err"])
def visa_close(m, session, err=None):
    """Closes the session. `always`: runs even if an error arrives on the input."""
    s = _live(session)
    s.is_open = False
    return (None,)


@native(VISA, "visa.write", [P("session"), P("command", type="string"), P("err", None)], ["session", "err"])
def visa_write(m, session, command, err=None):
    """Sends a command; any response waits for a `visa.read`."""
    s = _live(session)
    resp = s.instrument.execute(command, s.timeout_s)
    if resp is not None:
        s.buffer.append(resp)
    return (s, None)


@native(VISA, "visa.read", [P("session"), P("err", None)], ["text", "session", "err"], ("", None, None))
def visa_read(m, session, err=None):
    """Reads the pending response; `io.timeout` if there is none."""
    s = _live(session)
    if not s.buffer:
        m.world.clock += s.timeout_s
        raise LcFail("io.timeout", f"{s.resource}: nothing to read", -1073807339)
    return (s.buffer.pop(0), s, None)


@native(VISA, "visa.query", [P("session"), P("command", type="string"), P("err", None)], ["text", "session", "err"],
        ("", None, None))
def visa_query(m, session, command, err=None):
    """Writes, then reads."""
    s = _live(session)
    resp = s.instrument.execute(command, s.timeout_s)
    if resp is None:
        m.world.clock += s.timeout_s
        raise LcFail("io.timeout", f"{s.resource}: '{command}' is not a query", -1073807339)
    return (resp, s, None)


# results_log (crate of the example, section 13) ------------------------------------------------

@native("labcraft-results-log", "results_log.append_csv", [P("results"), P("err", None)], ["err"])
def results_append(m, results, err=None):
    """Appends the results to the log (in memory in the PoC, reported by `run`)."""
    m.results_log.append(list(results))
    return (None,)
