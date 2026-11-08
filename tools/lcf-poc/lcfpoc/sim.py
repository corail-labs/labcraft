"""Simulated instruments (TOML files) and simulated clock.

The format is that of `libs/visa/simulation.md`: a file describes a bench, one table per library that
simulates; in M0 only `visa`, whose `[[visa.instrument]]` tables are read here. The shipped files are in
`libs/visa/sim/`. A problem of the file raises `SimError` with the code of section 7 (`visa:S005: …`).

    [[visa.instrument]]
    resource   = "SIM::DMM0"               # VISA resource name
    idn        = "SIM,DMM,0,1.0"           # response to *IDN?
    latency_ms = 5                         # simulated time consumed by each command (0 by default)
    accept     = ["ROUT:CLOS"]             # prefixes of writes accepted with no effect
    [visa.instrument.commands]             # query -> fixed response
    [visa.instrument.measure]              # query -> measured quantity
    "MEAS:VOLT:DC?" = "voltage"
    [visa.instrument.set]                  # write header ("VOLT 5") -> quantity being set
    [visa.instrument.quantity.voltage]     # value, initial, start_ms, settle_ms, noise, seed, source
    [visa.instrument.faults]               # timeout, error_on, error_queue
"""
import math
import re
import tomllib
import unicodedata

from .values import Fault, LcFail, Session

VI_ERROR_TMO = -1073807339
VI_ERROR_RSRC_NFOUND = -1073807343
VI_ERROR_INV_OBJECT = -1073807346

MASK = (1 << 64) - 1
CLOCK_END_NS = 1 << 62  # about 146 years: past it, fault LC8007 (cli.md, section 6.6)
NS_PER_MS = 1_000_000
DAY_MS = 86_400_000
MAX_VALUE = 1e100
QUEUE_SIZE = 32
MAX_TEXT = 1000
MAX_INSTRUMENTS, MAX_QUANTITIES, MAX_ENTRIES = 256, 1000, 10_000
INSTRUMENT_KEYS = {"resource", "idn", "latency_ms", "commands", "measure", "set", "accept", "quantity", "faults"}
QUANTITY_KEYS = {"value", "initial", "start_ms", "settle_ms", "noise", "seed", "source"}
FAULT_KEYS = {"timeout", "error_on", "error_queue"}
RESERVED = {"*IDN?", "SYST:ERR?", "*CLS", "*RST"}
NAME = re.compile(r"[A-Za-z_][A-Za-z0-9_]*\Z")
NRF = re.compile(r"[+-]?([0-9]+(\.[0-9]*)?|\.[0-9]+)([eE][+-]?[0-9]+)?\Z", re.ASCII)


class SimError(Exception):
    pass


def fail(code, message):
    raise SimError(f"visa:{code}: {message}")


class SplitMix64:
    """The generator of the noise of a quantity (libs/visa/simulation.md, section 3)."""

    def __init__(self, seed):
        self.state = seed & MASK

    def next(self):
        self.state = (self.state + 0x9E3779B97F4A7C15) & MASK
        z = self.state
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & MASK
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & MASK
        return z ^ (z >> 31)

    def gauss(self):
        """Sum of 12 uniform values, minus 6: one conversion to a float, then an exact product."""
        s = sum(self.next() >> 11 for _ in range(12))
        return float(s - 6 * (1 << 53)) * 2.0 ** -53


# Reading the file (section 7) -------------------------------------------------------------------

def check_keys(table, known, where):
    unknown = sorted(set(table) - known)
    if unknown:
        fail("S001", f"{where}: unknown key '{unknown[0]}' (the keys are {', '.join(sorted(known))})")


def number(table, key, where, default, low=0.0, high=float(DAY_MS)):
    v = table.get(key, default)
    if isinstance(v, bool) or not isinstance(v, (int, float)):
        fail("S002", f"{where}: '{key}' must be a number")
    if not math.isfinite(v) or not low <= v <= high:
        fail("S005", f"{where}: '{key}' = {v} is out of its range ({low:g} to {high:g})")
    return float(v)


def ms_to_ns(ms):
    return round(ms * NS_PER_MS)  # to the nearest nanosecond, ties to even


def text(v, where):
    if not isinstance(v, str):
        fail("S002", f"{where}: a text is expected")
    if len(v) > MAX_TEXT or any(unicodedata.category(c) == "Cc" for c in v):
        fail("S008", f"{where}: a text of at most {MAX_TEXT} characters, with no control character")
    return v


def table(cfg, key, where):
    v = cfg.get(key, {})
    if not isinstance(v, dict):
        fail("S002", f"{where}: '{key}' must be a table")
    if len(v) > MAX_ENTRIES:
        fail("S010", f"{where}: '{key}' has more than {MAX_ENTRIES} entries")
    return v


def array(cfg, key, where):
    v = cfg.get(key, [])
    if not isinstance(v, list):
        fail("S002", f"{where}: '{key}' must be an array")
    if len(v) > MAX_ENTRIES:
        fail("S010", f"{where}: '{key}' has more than {MAX_ENTRIES} entries")
    return v


def error_entry(v, where):
    if (not isinstance(v, list) or len(v) != 2 or isinstance(v[0], bool) or not isinstance(v[0], int)
            or not isinstance(v[1], str)):
        fail("S009", f"{where}: an error is [code, \"message\"]")
    if not -2**31 <= v[0] < 2**31:
        fail("S005", f"{where}: the error code {v[0]} is out of its range")
    return (v[0], text(v[1], where))


def command_key(c, where, query=False):
    text(c, where)
    if c != c.strip(" ") or not c:
        fail("S011", f"{where}: '{c}' has spaces at an end, or is empty")
    if c in RESERVED:
        fail("S011", f"{where}: '{c}' is answered by the simulator itself")
    if query and not c.endswith("?"):
        fail("S011", f"{where}: '{c}' is not a query (it ends with ?)")
    return c


def check_integers(v, depth=0):
    """TOML integers are 64-bit: a larger one is not TOML (LC8109 of the tool), refused before visa reads it."""
    stack = [v]
    while stack:
        x = stack.pop()
        if isinstance(x, bool):
            continue
        if isinstance(x, int) and not -2**63 <= x < 2**63:
            raise SimError(f"LC8109: the integer {str(x)[:20]}... does not fit in 64 bits, as TOML requires")
        if isinstance(x, dict):
            stack.extend(x.values())
        elif isinstance(x, list):
            stack.extend(x)


class Quantity:
    def __init__(self, name, cfg, instrument, world):
        where = f"{instrument.resource}: quantity '{name}'"
        if not isinstance(cfg, dict):
            fail("S002", f"{where}: a table is expected")
        if not NAME.match(name):
            fail("S008", f"{where}: a quantity name is an ASCII name")
        check_keys(cfg, QUANTITY_KEYS, where)
        self.name, self.instrument, self.world = name, instrument, world
        self.source, self.root = cfg.get("source"), None
        if self.source is not None:
            text(self.source, where)
            moving = sorted(k for k in ("value", "initial", "start_ms", "settle_ms") if k in cfg)
            if moving:
                fail("S007", f"{where}: a quantity with a 'source' has no '{moving[0]}'")
        value = number(cfg, "value", where, 0.0, -MAX_VALUE, MAX_VALUE)
        self.a = number(cfg, "initial", where, value, -MAX_VALUE, MAX_VALUE)
        self.b = value
        self.t0 = ms_to_ns(number(cfg, "start_ms", where, 0.0))
        self.settle = ms_to_ns(number(cfg, "settle_ms", where, 0.0))
        self.dur = self.settle
        self.noise = number(cfg, "noise", where, 0.0, 0.0, MAX_VALUE)
        if self.noise > 0 and "seed" not in cfg:
            fail("S003", f"{where}: a quantity with a noise needs its own 'seed'")
        seed = cfg.get("seed", 0)
        if isinstance(seed, bool) or not isinstance(seed, int):
            fail("S002", f"{where}: 'seed' must be an integer")
        if not 0 <= seed < 2**63:
            fail("S005", f"{where}: 'seed' is out of its range (0 to 2^63-1)")
        self.rng = SplitMix64(seed)

    def own_value(self, t):
        if t < self.t0:
            return self.a
        if self.dur <= 0 or t >= self.t0 + self.dur:
            return self.b
        return self.a + (self.b - self.a) * (float(t - self.t0) / float(self.dur))

    def true_value(self, t):
        # The end of the chain of sources, found once when the file is read (World.check_sources).
        return (self.root or self).own_value(t)

    def read(self, t):
        # Every read draws, noise 0 included, so that the draws depend only on the number of reads.
        return self.true_value(t) + self.noise * self.rng.gauss()

    def set(self, target, t):
        self.a, self.b, self.t0, self.dur = self.own_value(t), target, t, self.settle


class Instrument:
    def __init__(self, cfg, world):
        if not isinstance(cfg, dict):
            fail("S002", "[[visa.instrument]] expected")
        resource = cfg.get("resource")
        if resource is None:
            fail("S003", "an instrument without 'resource'")
        self.resource = text(resource, "resource")
        if not resource.startswith("SIM::"):
            fail("S003", f"the resource '{resource}' does not start with SIM::")
        where = self.resource
        check_keys(cfg, INSTRUMENT_KEYS, where)
        self.world = world
        self.idn = text(cfg.get("idn", f"SIM,{self.resource},0,1.0"), f"{where}: idn")
        self.latency = ms_to_ns(number(cfg, "latency_ms", where, 0.0))
        self.commands = {}
        for c, answer in table(cfg, "commands", where).items():
            self.commands[command_key(c, f"{where}: commands", query=True)] = text(answer, f"{where}: commands '{c}'")
        self.measure = {command_key(c, f"{where}: measure", query=True): text(q, f"{where}: measure '{c}'")
                        for c, q in table(cfg, "measure", where).items()}
        self.setters = {command_key(c, f"{where}: set"): text(q, f"{where}: set '{c}'")
                        for c, q in table(cfg, "set", where).items()}
        self.accept = [text(p, f"{where}: accept") for p in array(cfg, "accept", where)]
        for c in self.commands.keys() & self.measure.keys():
            fail("S011", f"{where}: '{c}' is both in commands and in measure")
        faults = table(cfg, "faults", where)
        check_keys(faults, FAULT_KEYS, f"{where}: faults")
        self.timeouts = {command_key(c, f"{where}: faults.timeout", query=True) for c in array(faults, "timeout", where)}
        self.error_on = {command_key(k, f"{where}: faults.error_on"): error_entry(v, f"{where}: faults.error_on '{k}'")
                         for k, v in table(faults, "error_on", where).items()}
        self.queue = [error_entry(e, f"{where}: faults.error_queue") for e in array(faults, "error_queue", where)]
        if len(self.queue) > QUEUE_SIZE:
            fail("S010", f"{where}: faults.error_queue holds more than {QUEUE_SIZE} errors, the size of the queue")
        quantities = table(cfg, "quantity", where)
        if len(quantities) > MAX_QUANTITIES:
            fail("S010", f"{where}: more than {MAX_QUANTITIES} quantities")
        self.quantities = {n: Quantity(n, q, self, world) for n, q in quantities.items()}
        for kind, entries in (("measure", self.measure), ("set", self.setters)):
            for cmd, q in entries.items():
                if q not in self.quantities:
                    fail("S006", f"{where}: '{cmd}' ({kind}) refers to the unknown quantity '{q}'")
        for cmd, q in self.setters.items():
            if self.quantities[q].source:
                fail("S006", f"{where}: '{cmd}' (set) sets '{q}', which takes its value from a source")

    def push_error(self, entry):
        if len(self.queue) < QUEUE_SIZE:
            self.queue.append(entry)
        else:
            self.queue[-1] = (-350, "Queue overflow")

    def execute(self, cmd):
        """Executes a command (section 4). Returns its answer, or None: a write, or a query never answered.
        Consumes the latency; the caller waits for the timeout when it reads no answer."""
        w = self.world
        w.advance(self.latency)
        c = command_text(cmd)
        response = None
        if c in self.timeouts:
            w.log_cmd(self, c, "<no response>")
            return None
        if c in self.error_on:
            self.push_error(self.error_on[c])
        if c == "SYST:ERR?":
            code, msg = self.queue.pop(0) if self.queue else (0, "No error")
            quoted = msg.replace('"', '""')  # a quote in a SCPI string is doubled
            response = f'{code},"{quoted}"'
        elif c == "*IDN?":
            response = self.idn
        elif c == "*CLS":
            self.queue.clear()
        elif c == "*RST":
            pass
        elif c in self.commands:
            response = self.commands[c]
        elif c in self.measure:
            response = f"{self.quantities[self.measure[c]].read(w.clock_ns):.6f}"
        else:
            head, _, arg = c.partition(" ")
            if head in self.setters and not c.endswith("?"):
                arg = arg.strip(" ")
                if not NRF.match(arg):
                    self.push_error((-224, "Illegal parameter value"))
                else:
                    x = float(arg)
                    if not math.isfinite(x) or abs(x) > MAX_VALUE:
                        self.push_error((-222, "Data out of range"))
                    else:
                        self.quantities[self.setters[head]].set(x, w.clock_ns)
            elif not c.endswith("?") and any(c.startswith(p) for p in self.accept):
                pass
            else:
                self.push_error((-113, "Undefined header"))
        w.log_cmd(self, c, response if response is not None or not c.endswith("?") else "<no response>")
        return response


def command_text(cmd):
    """The command without the termination a program may have written, then without spaces at the ends."""
    if cmd.endswith("\r\n"):
        cmd = cmd[:-2]
    elif cmd.endswith("\n"):
        cmd = cmd[:-1]
    return cmd.strip(" ")


class World:
    """Simulated instruments and clock. One run = one world; same inputs, same results.
    The clock counts whole nanoseconds; `clock` gives it in seconds for the rest of the proof of concept."""

    def __init__(self):
        self.clock_ns = 0
        self.instruments = {}
        self.sessions = []
        self.commands = []

    @property
    def clock(self):
        return self.clock_ns / 1e9

    @clock.setter
    def clock(self, seconds):
        self.clock_ns = round(seconds * 1e9)

    def advance(self, ns):
        if self.clock_ns + ns >= CLOCK_END_NS:
            raise Fault("LC8007", "the clock of the run would pass 2^62 ns (about 146 years)")
        self.clock_ns += ns

    def wait(self, seconds):
        if not math.isfinite(seconds) or seconds * 1e9 >= CLOCK_END_NS:
            raise Fault("LC8007", f"a wait of {seconds} s would take the clock of the run past 2^62 ns (about 146 years)")
        self.advance(round(seconds * 1e9))

    @classmethod
    def load(cls, path):
        world = cls()
        try:
            with open(path, "rb") as f:
                data = tomllib.load(f)
        except (OSError, UnicodeDecodeError) as e:
            raise SimError(f"{path}: {e}") from None
        except (tomllib.TOMLDecodeError, ValueError, OverflowError) as e:
            raise SimError(f"{path}: LC8109: not TOML ({e})") from None
        except RecursionError:
            raise SimError(f"{path}: LC0007: nested too deep") from None
        try:
            check_integers(data)
        except SimError as e:
            raise SimError(f"{path}: {e}") from None
        others = sorted(set(data) - {"visa"})
        if others:
            raise SimError(f"{path}: LC8110: the key '{others[0]}' belongs to no library that simulates (in M0: visa)")
        try:
            section = data.get("visa", {})
            if not isinstance(section, dict):
                fail("S002", "[[visa.instrument]] expected")
            check_keys(section, {"instrument"}, "visa")
            instruments = array(section, "instrument", "visa")
            if len(instruments) > MAX_INSTRUMENTS:
                fail("S010", f"more than {MAX_INSTRUMENTS} instruments")
            for cfg in instruments:
                inst = Instrument(cfg, world)
                if inst.resource in world.instruments:
                    fail("S004", f"resource '{inst.resource}' defined twice")
                world.instruments[inst.resource] = inst
            world.check_sources()
        except SimError as e:
            raise SimError(f"{path}: {e}") from None
        return world

    def check_sources(self):
        """Each chain of sources ends: followed without recursion, a loop is refused."""
        for inst in self.instruments.values():
            for q in inst.quantities.values():
                path, on_path, cur = [], set(), q
                while cur.source and cur.root is None:
                    if id(cur) in on_path:
                        fail("S007", f"{inst.resource}: the sources of '{q.name}' loop")
                    on_path.add(id(cur))
                    path.append(cur)
                    cur = self.quantity(cur.source)
                end = cur.root or cur
                for p in path:
                    p.root = end

    def quantity(self, ref):
        res, _, name = ref.rpartition(".")
        inst = self.instruments.get(res)
        if inst is None or name not in inst.quantities:
            fail("S007", f"unknown quantity source '{ref}'")
        return inst.quantities[name]

    def open(self, resource, timeout_s, external=False):
        inst = self.instruments.get(resource)
        if inst is None:
            self.wait(timeout_s)
            raise LcFail("visa.resource_not_found", f"resource not found: {resource}", VI_ERROR_RSRC_NFOUND)
        s = Session(resource, inst, external)
        s.timeout_s = timeout_s
        self.sessions.append(s)
        return s

    def log_cmd(self, inst, cmd, response):
        if len(self.commands) < 500:
            self.commands.append({"t_ms": round(self.clock_ns / 1e6, 3), "resource": inst.resource,
                                  "cmd": cmd, "response": response})

    def open_sessions(self):
        return [s.resource for s in self.sessions if s.is_open and not s.external]
