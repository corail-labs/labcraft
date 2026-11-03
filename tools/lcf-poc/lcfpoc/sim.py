"""Simulated instruments (TOML files) and simulated clock.

File format (see instruments/*.toml): one or more `[[instrument]]` tables.

    [[instrument]]
    resource   = "SIM::DMM0"          # VISA resource name
    idn        = "SIM,DMM,0,1.0"      # response to *IDN?
    latency_ms = 5                    # simulated time consumed by each command (0 by default)
    accept     = ["ROUT:CLOS"]        # prefixes of writes accepted with no effect
    [instrument.commands]             # query -> fixed response
    [instrument.measure]              # query -> measured quantity
    "MEAS:VOLT:DC?" = "voltage"
    [instrument.set]                  # write prefix ("VOLT 5") -> quantity being set
    [instrument.quantity.voltage]     # value, initial, start_ms, settle_ms, noise, seed, source
    [instrument.faults]               # timeout = [queries with no response], error_on = {cmd = [code, "msg"]}
"""
import random
import tomllib

from .values import LcFail, Session

VI_ERROR_TMO = -1073807339
VI_ERROR_RSRC_NFOUND = -1073807343


class SimError(Exception):
    pass


class Quantity:
    def __init__(self, name, cfg, instrument, world):
        self.name, self.instrument, self.world = name, instrument, world
        self.source = cfg.get("source")
        value = float(cfg.get("value", 0.0))
        self.settle_s = float(cfg.get("settle_ms", 0.0)) / 1000
        # Ramp in progress: from `a` to `b` between t0 and t0 + dur.
        self.a = float(cfg.get("initial", value))
        self.b, self.t0, self.dur = value, float(cfg.get("start_ms", 0.0)) / 1000, self.settle_s
        self.noise = float(cfg.get("noise", 0.0))
        self.rng = random.Random(cfg.get("seed", 0))

    def true_value(self, t):
        if self.source:
            return self.world.quantity(self.source).true_value(t)
        if t < self.t0:
            return self.a
        if self.dur <= 0 or t >= self.t0 + self.dur:
            return self.b
        return self.a + (self.b - self.a) * (t - self.t0) / self.dur

    def read(self, t):
        v = self.true_value(t)
        return v + self.rng.gauss(0.0, self.noise) if self.noise else v

    def set(self, target, t):
        self.a, self.b, self.t0, self.dur = self.true_value(t), target, t, self.settle_s


class Instrument:
    def __init__(self, cfg, world):
        try:
            self.resource = str(cfg["resource"])
        except KeyError:
            raise SimError("instrument without a 'resource' key") from None
        self.world = world
        self.idn = cfg.get("idn", f"SIM,{self.resource},0,1.0")
        self.latency = float(cfg.get("latency_ms", 0.0)) / 1000
        self.commands = dict(cfg.get("commands", {}))
        self.measure = dict(cfg.get("measure", {}))
        self.setters = dict(cfg.get("set", {}))
        self.accept = list(cfg.get("accept", []))
        faults = cfg.get("faults", {})
        self.timeouts = set(faults.get("timeout", []))
        self.error_on = {k: tuple(v) for k, v in faults.get("error_on", {}).items()}
        self.queue = [tuple(e) for e in faults.get("error_queue", [])]
        self.quantities = {n: Quantity(n, q, self, world) for n, q in cfg.get("quantity", {}).items()}
        for kind, table in (("measure", self.measure), ("set", self.setters)):
            for cmd, q in table.items():
                if q not in self.quantities:
                    raise SimError(f"{self.resource}: '{cmd}' ({kind}) refers to the unknown quantity '{q}'")
        self.opened = 0

    def execute(self, cmd, timeout_s):
        """Executes a command; returns the response (text) or None. Consumes simulated time."""
        w = self.world
        w.clock += self.latency
        c = cmd.strip()
        response = None
        if c in self.timeouts:
            w.clock += timeout_s
            raise LcFail("io.timeout", f"{self.resource}: no response to '{c}' after {timeout_s:g} s", VI_ERROR_TMO)
        if c in self.error_on:
            self.queue.append(self.error_on[c])
        if c == "SYST:ERR?":
            code, msg = self.queue.pop(0) if self.queue else (0, "No error")
            response = f'{code},"{msg}"'
        elif c == "*IDN?":
            response = self.idn
        elif c == "*CLS":
            self.queue.clear()
        elif c == "*RST":
            pass
        elif c in self.commands:
            response = str(self.commands[c])
        elif c in self.measure:
            response = f"{self.quantities[self.measure[c]].read(w.clock):.6f}"
        else:
            head, _, arg = c.partition(" ")
            if head in self.setters:
                try:
                    self.quantities[self.setters[head]].set(float(arg), w.clock)
                except ValueError:
                    self.queue.append((-224, "Illegal parameter value"))
            elif any(c.startswith(p) for p in self.accept):
                pass
            else:
                self.queue.append((-113, "Undefined header"))
                if c.endswith("?"):
                    w.clock += timeout_s
                    w.log_cmd(self, c, "<no response>")
                    raise LcFail("io.timeout", f"{self.resource}: unknown query '{c}' (no response)", VI_ERROR_TMO)
        w.log_cmd(self, c, response)
        return response


class World:
    """Simulated instruments and clock. One run = one world; same inputs, same results."""

    def __init__(self):
        self.clock = 0.0  # simulated seconds
        self.instruments = {}
        self.sessions = []
        self.commands = []

    @classmethod
    def load(cls, path):
        world = cls()
        try:
            with open(path, "rb") as f:
                data = tomllib.load(f)
        except (OSError, tomllib.TOMLDecodeError, UnicodeDecodeError) as e:
            raise SimError(f"{path}: {e}") from None
        for cfg in data.get("instrument", []):
            if not isinstance(cfg, dict):
                raise SimError(f"{path}: [[instrument]] expected")
            try:
                inst = Instrument(cfg, world)
            except (TypeError, ValueError, AttributeError) as e:
                raise SimError(f"{path}: invalid instrument ({e})") from None
            if inst.resource in world.instruments:
                raise SimError(f"{path}: resource '{inst.resource}' defined twice")
            world.instruments[inst.resource] = inst
        for inst in world.instruments.values():
            for q in inst.quantities.values():
                if q.source:
                    world.quantity(q.source)
        return world

    def quantity(self, ref):
        res, _, name = ref.rpartition(".")
        inst = self.instruments.get(res)
        if inst is None or name not in inst.quantities:
            raise SimError(f"unknown quantity source '{ref}'")
        return inst.quantities[name]

    def open(self, resource, timeout_s, external=False):
        inst = self.instruments.get(resource)
        if inst is None:
            self.clock += timeout_s
            raise LcFail("visa.resource_not_found", f"resource not found: {resource}", VI_ERROR_RSRC_NFOUND)
        s = Session(resource, inst, external)
        s.timeout_s = timeout_s
        self.sessions.append(s)
        inst.opened += 1
        return s

    def log_cmd(self, inst, cmd, response):
        if len(self.commands) < 500:
            self.commands.append({"t_ms": round(self.clock * 1000, 3), "resource": inst.resource,
                                  "cmd": cmd, "response": response})

    def open_sessions(self):
        return [s.resource for s in self.sessions if s.is_open and not s.external]
