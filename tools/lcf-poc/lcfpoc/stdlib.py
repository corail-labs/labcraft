"""Minimal library (spec section 9): core, array, time, errors, visa, scpi, dmm_driver, relays,
psu, results_log. Each primitive declares its inputs, its outputs and their default values
(used when the node is skipped because of an error)."""
import math

from .values import (Fault, LcError, LcFail, PathConst, Q, Session, UnitConst, ZERO, dim_label, from_si, is_num,
                     parse_unit, UnitError)

REQ = object()


class P:
    """Parameter of a primitive. `kind`: value | unit (unit name) | category (error category)."""

    def __init__(self, name, default=REQ, kind="value", type=""):
        self.name, self.default, self.kind, self.type = name, default, kind, type


class Builtin:
    def __init__(self, qname, params, outs, defaults, fn, always, doc):
        self.qname, self.params, self.outs, self.defaults = qname, params, outs, defaults
        self.fn, self.always, self.doc = fn, always, doc
        self.lib, _, self.short = qname.rpartition(".")

    def signature(self):
        ps = ", ".join(p.name + (f": {p.type}" if p.type else "") + ("" if p.default is REQ else " = …") for p in self.params)
        return f"{'always ' if self.always else ''}fn {self.qname}({ps}) -> ({', '.join(self.outs)})"


BUILTINS = {}


def builtin(qname, params, outs, defaults=None, always=False):
    def deco(fn):
        BUILTINS[qname] = Builtin(qname, params, outs, defaults or (None,) * len(outs), fn, always,
                                  (fn.__doc__ or "").strip())
        return fn
    return deco


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


@builtin("add", [P("a"), P("b")], ["result"], (0.0,))
def add(m, a, b):
    """a + b; same dimensions required, the unit of the result is that of `a`."""
    _same_dim("add", a, b)
    return (a.with_si(a.si + b.si),) if isinstance(a, Q) else (a + b,)


@builtin("sub", [P("a"), P("b")], ["result"], (0.0,))
def sub(m, a, b):
    """a - b; same dimensions required."""
    _same_dim("sub", a, b)
    return (a.with_si(a.si - b.si),) if isinstance(a, Q) else (a - b,)


@builtin("mul", [P("a"), P("b")], ["result"], (0.0,))
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


@builtin("div", [P("a"), P("b")], ["result"], (0.0,))
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


@builtin("abs", [P("x")], ["result"], (0.0,))
def abs_(m, x):
    """Absolute value."""
    return (x.with_si(abs(x.si)) if isinstance(x, Q) else abs(x),)


@builtin("neg", [P("x")], ["result"], (0.0,))
def neg(m, x):
    """Negation."""
    return (x.with_si(-x.si) if isinstance(x, Q) else -x,)


@builtin("sqrt", [P("x")], ["result"], (0.0,))
def sqrt(m, x):
    """Square root; the dimension exponents are halved (rejected if any is odd)."""
    if isinstance(x, Q):
        if any(e % 2 for e in x.dim):
            raise Fault("lcf.unit_mismatch", f"sqrt: dimension {_kind(x)} has no square root")
        return (from_si(math.sqrt(x.si) if x.si >= 0 else math.nan, tuple(e // 2 for e in x.dim)),)
    return (math.sqrt(x) if x >= 0 else math.nan,)


@builtin("min", [P("a"), P("b")], ["result"], (0.0,))
def min_(m, a, b):
    """The smaller of the two."""
    _same_dim("min", a, b)
    return ((a if (a.si if isinstance(a, Q) else a) <= (b.si if isinstance(b, Q) else b) else b),)


@builtin("max", [P("a"), P("b")], ["result"], (0.0,))
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
    builtin(name, [P("a"), P("b")], ["result"], (False,))(f)


for _n, _o in (("lt", lambda x, y: x < y), ("le", lambda x, y: x <= y), ("gt", lambda x, y: x > y),
               ("ge", lambda x, y: x >= y), ("eq", lambda x, y: x == y), ("ne", lambda x, y: x != y)):
    _cmp(_n, _o)


def _bool(op, x):
    if not isinstance(x, bool):
        raise Fault("lcf.type_error", f"{op}: boolean expected (got {type(x).__name__})")
    return x


@builtin("and", [P("a"), P("b")], ["result"], (False,))
def and_(m, a, b):
    """Logical AND."""
    return (_bool("and", a) and _bool("and", b),)


@builtin("or", [P("a"), P("b")], ["result"], (False,))
def or_(m, a, b):
    """Logical OR."""
    return (_bool("or", a) or _bool("or", b),)


@builtin("not", [P("a")], ["result"], (False,))
def not_(m, a):
    """Logical NOT."""
    return (not _bool("not", a),)


@builtin("select", [P("condition"), P("if_true"), P("if_false")], ["result"])
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


@builtin("with_unit", [P("x"), P("unit", kind="unit")], ["result"], (0.0,))
def with_unit(m, x, unit):
    """Attaches a unit to a number: `with_unit(raw, [V])`. Rejected if `x` already has one."""
    if not is_num(x) or isinstance(x, Q):
        raise Fault("lcf.type_error", "with_unit: unitless number expected")
    text, _ = _unit_of(unit)
    return (Q.of(float(x), text),)


@builtin("unitless", [P("x"), P("unit", kind="unit")], ["result"], (0.0,))
def unitless(m, x, unit):
    """Removes the unit by expressing the number in the requested unit: `unitless(230 mV, [V])` gives 0.23."""
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


# visa / scpi -------------------------------------------------------------------------------

def _live(s):
    if not isinstance(s, Session) or not s.is_open:
        raise LcFail("visa.invalid_session", "session missing or closed")
    return s


@builtin("visa.open", [P("resource", type="string"), P("timeout", Q(2.0, parse_unit("s")[0], "s", 1.0)), P("err", None)],
         ["session", "err"])
def visa_open(m, resource, timeout, err=None):
    """Opens a session on a resource (`SIM::…`: simulated instrument)."""
    return (m.world.open(resource, _seconds(timeout, "visa.open")), None)


@builtin("visa.close", [P("session"), P("err", None)], ["err"], always=True)
def visa_close(m, session, err=None):
    """Closes the session. `always`: runs even if an error arrives on the input."""
    s = _live(session)
    s.is_open = False
    return (None,)


@builtin("visa.write", [P("session"), P("command", type="string"), P("err", None)], ["session", "err"])
def visa_write(m, session, command, err=None):
    """Sends a command; any response waits for a `visa.read`."""
    s = _live(session)
    resp = s.instrument.execute(command, s.timeout_s)
    if resp is not None:
        s.buffer.append(resp)
    return (s, None)


@builtin("visa.read", [P("session"), P("err", None)], ["text", "session", "err"], ("", None, None))
def visa_read(m, session, err=None):
    """Reads the pending response; `io.timeout` if there is none."""
    s = _live(session)
    if not s.buffer:
        m.world.clock += s.timeout_s
        raise LcFail("io.timeout", f"{s.resource}: nothing to read", -1073807339)
    return (s.buffer.pop(0), s, None)


@builtin("visa.query", [P("session"), P("command", type="string"), P("err", None)], ["text", "session", "err"], ("", None, None))
def visa_query(m, session, command, err=None):
    """Writes, then reads."""
    s = _live(session)
    resp = s.instrument.execute(command, s.timeout_s)
    if resp is None:
        m.world.clock += s.timeout_s
        raise LcFail("io.timeout", f"{s.resource}: '{command}' is not a query", -1073807339)
    return (resp, s, None)


def _query_float(m, session, command):
    text, s, _ = visa_query(m, session, command)
    try:
        return float(text), s
    except ValueError:
        raise LcFail("scpi.parse_error", f"non-numeric response to '{command}': {text!r}") from None


@builtin("scpi.query_f64", [P("session"), P("command", type="string"), P("err", None)], ["value", "session", "err"], (0.0, None, None))
def scpi_query_f64(m, session, command, err=None):
    """SCPI query whose response is a number."""
    v, s = _query_float(m, session, command)
    return (v, s, None)


@builtin("scpi.check_errors", [P("session"), P("err", None)], ["session", "err"])
def scpi_check_errors(m, session, err=None):
    """Drains the instrument error queue (`SYST:ERR?`); the first one becomes `scpi.instrument_error`."""
    s = _live(session)
    found = []
    for _ in range(20):
        text = s.instrument.execute("SYST:ERR?", s.timeout_s) or "0"
        code_text, _, msg = text.partition(",")
        try:
            code = int(code_text)
        except ValueError:
            raise LcFail("scpi.parse_error", f"unreadable SYST:ERR? response: {text!r}") from None
        if code == 0:
            break
        found.append((code, msg.strip('"')))
    if found:
        first = LcError("scpi.instrument_error", f"{s.resource}: {found[0][1]}", found[0][0])
        first.others = [LcError("scpi.instrument_error", f"{s.resource}: {msg}", c) for c, msg in found[1:]]
        raise _Carry(first)
    return (s, None)


class _Carry(LcFail):
    """LcFail that carries an already built error (with `others`)."""

    def __init__(self, error):
        super().__init__(error.category, error.message, error.code)
        self.error = error


@builtin("dmm_driver.read_dc_volts", [P("session"), P("err", None)], ["volts", "session", "err"],
         (Q(0.0, parse_unit("V")[0], "V", 1.0), None, None))
def read_dc_volts(m, session, err=None):
    """Multimeter driver: reads the DC voltage (`MEAS:VOLT:DC?`)."""
    v, s = _query_float(m, session, "MEAS:VOLT:DC?")
    return (Q.of(v, "V"), s, None)


# relays / psu / results_log (test station, section 13) --------------------------------------

@builtin("relays.apply", [P("bank"), P("route", type="string"), P("err", None)], ["bank", "err"])
def relays_apply(m, bank, route, err=None):
    """Closes the named route on the relay matrix (`ROUT:CLOS <route>`)."""
    s = _live(bank)
    s.instrument.execute(f"ROUT:CLOS {route}", s.timeout_s)
    return (s, None)


@builtin("psu.set_voltage", [P("source"), P("voltage"), P("err", None)], ["source", "err"])
def psu_set_voltage(m, source, voltage, err=None):
    """Sets the voltage setpoint of the power supply (`VOLT <value in V>`)."""
    s = _live(source)
    if not (isinstance(voltage, Q) and voltage.dim == parse_unit("V")[0]):
        raise Fault("lcf.unit_mismatch", f"psu.set_voltage: voltage expected, got {voltage!r}")
    s.instrument.execute(f"VOLT {voltage.si:.6f}", s.timeout_s)
    return (s, None)


@builtin("results_log.append_csv", [P("results"), P("err", None)], ["err"])
def results_append(m, results, err=None):
    """Appends the results to the log (in memory in the PoC, reported by `run`)."""
    m.results_log.append(list(results))
    return (None,)
