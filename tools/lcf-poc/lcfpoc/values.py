"""Run-time values: quantities with units, structs, enumerations, errors."""
import math
import re

DIMS = ("m", "kg", "s", "A", "K", "mol", "cd")
ZERO = (0,) * 7


def _d(**kw):
    return tuple(kw.get(k, 0) for k in DIMS)


# name -> (dimension, factor to SI). `kg` is the base: `g` is 1e-3.
UNIT_TABLE = {
    "m": (_d(m=1), 1.0), "kg": (_d(kg=1), 1.0), "g": (_d(kg=1), 1e-3), "s": (_d(s=1), 1.0),
    "A": (_d(A=1), 1.0), "K": (_d(K=1), 1.0), "mol": (_d(mol=1), 1.0), "cd": (_d(cd=1), 1.0),
    "Hz": (_d(s=-1), 1.0), "N": (_d(kg=1, m=1, s=-2), 1.0), "Pa": (_d(kg=1, m=-1, s=-2), 1.0),
    "J": (_d(kg=1, m=2, s=-2), 1.0), "W": (_d(kg=1, m=2, s=-3), 1.0), "C": (_d(A=1, s=1), 1.0),
    "V": (_d(kg=1, m=2, s=-3, A=-1), 1.0), "F": (_d(kg=-1, m=-2, s=4, A=2), 1.0),
    "Ω": (_d(kg=1, m=2, s=-3, A=-2), 1.0), "ohm": (_d(kg=1, m=2, s=-3, A=-2), 1.0),
    "S": (_d(kg=-1, m=-2, s=3, A=2), 1.0), "H": (_d(kg=1, m=2, s=-2, A=-2), 1.0),
}
PREFIXES = {"p": 1e-12, "n": 1e-9, "µ": 1e-6, "u": 1e-6, "m": 1e-3, "k": 1e3, "M": 1e6, "G": 1e9}
# Name given to the result of a computation: the first in the list with that dimension.
DISPLAY_ORDER = ["V", "A", "Ω", "W", "Hz", "F", "J", "N", "Pa", "C", "S", "H", "s", "m", "kg", "K", "mol", "cd"]
NAME_OF_DIM = {}
for _n in DISPLAY_ORDER:
    NAME_OF_DIM.setdefault(UNIT_TABLE[_n][0], _n)


class UnitError(Exception):
    pass


def _lookup(name):
    if name in UNIT_TABLE:
        return UNIT_TABLE[name]
    if name in ("°C", "°F"):
        raise UnitError(f"offset unit '{name}' is not supported by the PoC (Q-3)")
    if len(name) > 1 and name[0] in PREFIXES and name[1:] in UNIT_TABLE and name[1:] != "kg":
        dim, scale = UNIT_TABLE[name[1:]]
        return dim, scale * PREFIXES[name[0]]
    raise UnitError(f"unknown unit '{name}'")


_TOK = re.compile(r"\s*([A-Za-zµΩ°][A-Za-z0-9µΩ°_]*|\d+|[*·/^()\-])")


def parse_unit(text):
    """`mV`, `V/s`, `kg·m^2·s^-2`, `(V/A)` -> (dimension, SI factor)."""
    toks, pos, text = [], 0, text.strip()
    while pos < len(text):
        m = _TOK.match(text, pos)
        if not m:
            raise UnitError(f"unreadable unit '{text}'")
        toks.append(m.group(1))
        pos = m.end()
    i = 0

    def peek():
        return toks[i] if i < len(toks) else None

    def term():
        nonlocal i
        t = peek()
        if t is None:
            raise UnitError(f"incomplete unit '{text}'")
        i += 1
        if t == "(":
            dim, sc = expr()
            if peek() != ")":
                raise UnitError(f"unclosed parenthesis in '{text}'")
            i += 1
        elif t == "1":
            dim, sc = ZERO, 1.0
        else:
            dim, sc = _lookup(t)
        if peek() == "^":
            i += 1
            sign = 1
            if peek() == "-":
                sign, i = -1, i + 1
            e = peek()
            if e is None or not e.isdigit():
                raise UnitError(f"exponent expected in '{text}'")
            i += 1
            n = sign * int(e)
            dim, sc = tuple(d * n for d in dim), sc ** n
        return dim, sc

    def expr():
        nonlocal i
        dim, sc = term()
        while peek() in ("*", "·", "/"):
            op = peek()
            i += 1
            d2, s2 = term()
            if op == "/":
                dim, sc = tuple(a - b for a, b in zip(dim, d2)), sc / s2
            else:
                dim, sc = tuple(a + b for a, b in zip(dim, d2)), sc * s2
        return dim, sc

    dim, sc = expr()
    if i != len(toks):
        raise UnitError(f"unreadable unit '{text}'")
    return dim, sc


SUP = str.maketrans("-0123456789", "⁻⁰¹²³⁴⁵⁶⁷⁸⁹")


def dim_label(dim):
    if dim in NAME_OF_DIM:
        return NAME_OF_DIM[dim]
    parts = [f"{b}{'' if e == 1 else str(e).translate(SUP)}" for b, e in zip(DIMS, dim) if e]
    return "·".join(parts) or "1"


class Q:
    """Quantity: SI value, dimension, display unit and its factor."""

    __slots__ = ("si", "dim", "unit", "scale")

    def __init__(self, si, dim, unit, scale):
        self.si, self.dim, self.unit, self.scale = float(si), dim, unit, scale

    @classmethod
    def of(cls, value, unit_text):
        dim, scale = parse_unit(unit_text)
        return cls(value * scale, dim, unit_text.strip(), scale)

    @property
    def value(self):
        return self.si / self.scale

    def __repr__(self):
        return f"{self.value:g} {self.unit}"

    def __eq__(self, o):
        return isinstance(o, Q) and self.dim == o.dim and self.si == o.si

    def __hash__(self):
        return hash((self.si, self.dim))

    def with_si(self, si):
        return Q(si, self.dim, self.unit, self.scale)


def from_si(si, dim):
    """Result of a computation: a dimensionless number, or a quantity in the named unit of its dimension."""
    if dim == ZERO:
        return si
    return Q(si, dim, dim_label(dim), 1.0)


def is_num(x):
    return isinstance(x, (int, float, Q)) and not isinstance(x, bool)


class PathConst:
    """Path that is not a value: an error category (`io.timeout`) or a unit (`V`)."""

    def __init__(self, parts):
        self.parts = tuple(parts)

    @property
    def text(self):
        return ".".join(self.parts)

    def __repr__(self):
        return self.text


class UnitConst:
    """Unit passed as an argument, written in brackets: `with_unit(x, [V])` (spec R-5)."""

    def __init__(self, text):
        self.text = text

    def __repr__(self):
        return f"[{self.text}]"


class Struct:
    def __init__(self, type_name, fields):
        self.type_name, self.fields = type_name, fields

    def __eq__(self, o):
        return isinstance(o, Struct) and self.type_name == o.type_name and self.fields == o.fields

    def __repr__(self):
        return f"{self.type_name}{self.fields}"


class EnumVal:
    def __init__(self, type_name, variant, args=()):
        self.type_name, self.variant, self.args = type_name, variant, tuple(args)

    def __eq__(self, o):
        return (isinstance(o, EnumVal) and self.type_name == o.type_name
                and self.variant == o.variant and self.args == o.args)

    def __hash__(self):
        return hash((self.type_name, self.variant))

    def __repr__(self):
        return f"{self.type_name}.{self.variant}"


class LcError:
    """Error value (section 7.1). The absence of an error (`none`) is represented by `None`."""

    def __init__(self, category, message="", code=0, origin=None, others=None):
        self.category, self.message, self.code = category, message, code
        self.origin, self.others = list(origin or []), list(others or [])

    def __repr__(self):
        return f"error({self.category}: {self.message})"


class LcFail(Exception):
    """Failure of a primitive: becomes an `error` value on the node's `err` output."""

    def __init__(self, category, message="", code=0):
        super().__init__(message)
        self.category, self.message, self.code = category, message, code


class Fault(Exception):
    """Unrecoverable run-time fault (no error wire to carry it): reported, never propagated."""

    def __init__(self, category, message):
        super().__init__(message)
        self.category, self.message = category, message


class Session:
    """Connection handle (opaque type `visa.session`); copying the handle does not copy the connection."""

    def __init__(self, resource, instrument, external=False):
        self.resource, self.instrument = resource, instrument
        self.timeout_s, self.is_open, self.external = 2.0, True, external
        self.buffer = []  # responses waiting to be read

    def __repr__(self):
        return f"session({self.resource}, {'open' if self.is_open else 'closed'})"


def to_json(v, depth=0):
    if depth > 20:
        return "…"
    if v is None or isinstance(v, (bool, str, int)):
        return v
    if isinstance(v, float):
        return v if math.isfinite(v) else ("nan" if math.isnan(v) else ("inf" if v > 0 else "-inf"))
    if isinstance(v, Q):
        return {"value": to_json(v.value), "unit": v.unit}
    if isinstance(v, bytes):
        return v.hex()
    if isinstance(v, list):
        return [to_json(x, depth + 1) for x in v]
    if isinstance(v, Struct):
        return {k: to_json(x, depth + 1) for k, x in v.fields.items()}
    if isinstance(v, EnumVal):
        name = f"{v.type_name}.{v.variant}"
        return {name: [to_json(x, depth + 1) for x in v.args]} if v.args else name
    if isinstance(v, LcError):
        d = {"category": v.category, "code": v.code, "message": v.message, "origin": v.origin}
        if v.others:
            d["others"] = [to_json(o, depth + 1) for o in v.others]
        return d
    if isinstance(v, Session):
        return {"session": v.resource, "open": v.is_open}
    if isinstance(v, PathConst):
        return v.text
    return repr(v)
