"""Acceptance tests of the PoC (criteria 1 to 4 and 6): python -m unittest discover tools/lcf-poc/tests"""
import json
import re
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "tools" / "lcf-poc"))
from lcfpoc import api  # noqa: E402

EX, INS = str(ROOT / "examples"), ROOT / "instruments"
SETTLE = {"dmm": "SIM::DMM0", "target": "230 V", "tolerance": "0.2 V", "period": "50 ms", "timeout": "1 s"}


def run(fn, sim, args, folder=EX, **kw):
    return api.run(folder, fn, str(INS / sim) if sim else None, args, **kw)


def lib_dir(td, files):
    for name, text in files.items():
        (Path(td) / name).write_text(text, encoding="utf-8", newline="\n")
    return td


class Acceptance(unittest.TestCase):
    def test_examples_check(self):
        rep = api.check(EX)
        self.assertTrue(rep["ok"], rep["diagnostics"])

    def test_check_voltage_pass_fail(self):
        r = run("check_voltage", "dmm-230V.toml", {"resource": "SIM::DMM0"})
        self.assertTrue(r["ok"], r)
        self.assertEqual(r["outputs"]["verdict"], "Verdict.Pass")
        r = run("check_voltage", "dmm-250V.toml", {"resource": "SIM::DMM0"})
        self.assertEqual(r["outputs"]["verdict"], "Verdict.Fail")
        self.assertEqual(r["sim"]["open_sessions"], [])

    def test_resource_absent_close_still_runs(self):
        r = run("check_voltage", "dmm-230V.toml", {"resource": "SIM::NOPE"}, trace=True)
        self.assertEqual(r["error"]["category"], "visa.resource_not_found")
        self.assertEqual(r["outputs"]["verdict"], "Verdict.Error")
        close = next(t for t in r["trace"] if t["op"] == "visa.close")
        self.assertNotIn("skipped", close)
        skipped = [t["op"] for t in r["trace"] if t.get("skipped")]
        self.assertEqual(skipped, ["scpi.query_f64"])

    def test_session_closed_after_failed_measure(self):
        r = run("check_voltage", "dmm-timeout.toml", {"resource": "SIM::DMM0"})
        self.assertEqual(r["error"]["category"], "io.timeout")
        self.assertEqual(r["outputs"]["verdict"], "Verdict.Error")
        self.assertEqual(r["sim"]["open_sessions"], [])

    def test_settle(self):
        r = run("settle", "dmm-settle-300ms.toml", SETTLE)
        self.assertTrue(r["ok"], r)
        self.assertAlmostEqual(r["outputs"]["value"]["value"], 230.0, delta=0.3)
        self.assertEqual(r["sim"]["time_ms"], 300.0)
        r = run("settle", "dmm-settle-3s.toml", SETTLE)
        self.assertEqual(r["error"]["category"], "io.timeout")

    def test_deterministic(self):
        a = run("settle", "dmm-settle-300ms.toml", SETTLE, trace=True)
        b = run("settle", "dmm-settle-300ms.toml", SETTLE, trace=True)
        self.assertEqual(json.dumps(a), json.dumps(b))

    def test_scpi_error_queue(self):
        with tempfile.TemporaryDirectory() as td:
            lib_dir(td, {"t.lcf": """labcraft 1

use visa
use scpi

fn t(err: error = none) -> (err: error) {
    s, e1 = visa.open("SIM::DMM0", err: err)
    s2, e2 = scpi.check_errors(s, err: e1)
    e3 = visa.close(s2, err: e2)
    err = e3
}
"""})
            r = run("t", "dmm-scpi-error.toml", {}, folder=td)
            self.assertEqual(r["error"]["category"], "scpi.instrument_error")
            self.assertEqual(r["error"]["code"], -221)
            self.assertEqual(len(r["error"]["others"]), 1)

    def test_station(self):
        bench = {"dmm": "SIM::DMM0", "relays": "SIM::RELAY0", "source": "SIM::PSU0"}
        steps = [{"name": "5V", "route": "R1", "voltage": 5.0, "low": 4.8, "high": 5.2},
                 {"name": "12V", "route": "R2", "voltage": 12.0, "low": 11.8, "high": 12.2}]
        r = run("station", "bench.toml", {"bench": bench, "steps": steps, "dut_count": 2})
        self.assertTrue(r["ok"], r)  # two products tested, then State.Stop
        self.assertEqual(len(r["results_log"]), 2)
        self.assertTrue(all(s["passed"] for batch in r["results_log"] for s in batch))
        r = run("station", "bench.toml", {"bench": bench, "steps": steps, "dut_count": 100})
        self.assertEqual(r["error"]["category"], "io.timeout")  # the product leaves, the wait expires

    def test_projectile(self):
        shot = {"vx0": "10 m/s", "vy0": "10 m/s", "mass": "0.1 kg"}
        r = run("projectile", None, shot)
        self.assertTrue(r["ok"], r)
        out = {k: v["value"] for k, v in r["outputs"].items()}
        self.assertEqual(r["outputs"]["range"]["unit"], "m")
        # without drag: the parabola, 2·v/g, 2·v²/g, v²/(2g), within 1 % (10 ms Euler step)
        self.assertAlmostEqual(out["flight_time"], 2 * 10 / 9.81, delta=0.02)
        self.assertAlmostEqual(out["range"], 2 * 100 / 9.81, delta=0.2)
        self.assertAlmostEqual(out["apex"], 100 / (2 * 9.81), delta=0.06)
        braked = run("projectile", None, dict(shot, drag="0.002 kg/m"))["outputs"]
        self.assertLess(braked["range"]["value"], out["range"] - 3)

    def test_projectile_unit_slip(self):
        # a velocity added to a position: rejected, not a wrong number
        src = (ROOT / "examples" / "projectile.lcf").read_text(encoding="utf-8")
        slip = src.replace("next x = add(x, dx)", "next x = add(x, vx)")
        self.assertNotEqual(src, slip)
        with tempfile.TemporaryDirectory() as td:
            lib_dir(td, {"projectile.lcf": slip})
            r = run("projectile", None, {"vx0": "10 m/s", "vy0": "10 m/s", "mass": "0.1 kg"}, folder=td)
        self.assertEqual(r["fault"]["category"], "lcf.unit_mismatch")
        self.assertEqual(r["fault"]["line"], slip.splitlines().index("        next x = add(x, vx)") + 1)


class Language(unittest.TestCase):
    HEADER = "labcraft 1\n\nuse visa\nuse dmm_driver\nuse time\nuse errors\n\n"
    FNS = """fn sum_scaled(readings: [f64[V]]) -> (total: f64[V], scaled: [f64[V]]) {
    total, scaled = for x in readings, carry acc = 0 V {
        s = mul(x, 2.0)
        next acc = add(acc, x)
        collect s
    }
}

fn units() -> (a: f64[mV], p: f64[W], z: f64) {
    a = add(500 mV, 0.2 V)
    r = 10 Ω
    i = 2 A
    v = mul(r, i)
    p = mul(v, i)
    q = div(v, 4 V)
    z = q
}

fn bad_units() -> (x: f64[V]) {
    x = add(1 V, 2 A)
}

fn guard(resource: string, err: error = none) -> (value: f64[V], err: error) {
    dmm, e1 = visa.open(resource, err: err)
    v, dmm2, e2 = dmm_driver.read_dc_volts(dmm, err: e1)
    e3 = visa.close(dmm2, err: e2)
    value, e4 = handle e3 {
        ok => yield (v, none)
        io.timeout(x) => yield (nan V, none)
        _(e) => yield (0 V, e)
    }
    err = e4
}

fn by_code(resource: string) -> (hit: bool, err: error) {
    dmm, e1 = visa.open(resource)
    _, dmm2, e2 = dmm_driver.read_dc_volts(dmm, err: e1)
    e3 = visa.close(dmm2, err: e2)
    hit, e4 = handle e3 {
        code -1073807339 => yield (true, none)
        code 1073807339(e) => yield (false, e)
        _(e) => yield (false, e)
    }
    err = e4
}

fn classify(n: i32) -> (label: string) {
    label = match n {
        0 => yield "zero"
        1..=5 => yield "small"
        6..10 => yield "medium"
        _ => yield "large"
    }
}

fn count_to(limit: i32) -> (n: i32) {
    n = loop carry c = 0 {
        next c = add(c, 1)
        done = ge(c, limit)
        until done
    }
}

fn spin() -> (n: i32) {
    n = loop carry c = 0 {
        next c = add(c, 1)
        never = lt(c, 0)
        until never
    }
}

fn probe(err: error = none) -> (err: error) {
    t0 = time.now(after: err)
    _, err_out = loop carry n = 0, carry e = err {
        e1 = errors.raise_if(true, io.timeout, "boom", err: e)
        e2 = time.wait(50 ms, err: e1)
        t = time.elapsed(t0)
        done = gt(t, 1 s)
        next n = add(n, 1)
        next e = e2
        until done
    }
    err = err_out
}

fn probe_continue(err: error = none) -> (err: error) {
    t0 = time.now(after: err)
    _, err_out = loop continue_on_error carry n = 0, carry e = err {
        e1 = errors.raise_if(true, io.timeout, "boom", err: e)
        e2 = time.wait(50 ms, err: e1)
        t = time.elapsed(t0)
        done = gt(t, 1 s)
        next n = add(n, 1)
        next e = e2
        until done
    }
    err = err_out
}

fn strip() -> (a: f64, b: f64) {
    a = unitless(230 mV, [V])
    b = unitless(230 mV, [mV])
}

fn cleanup(dmm: visa.session, err: error = none) -> (err: error) {
    e1 = visa.close(dmm, err: err)
    err = e1
}

fn use_cleanup(resource: string) -> (err: error) {
    dmm, e1 = visa.open(resource)
    _, dmm2, e2 = dmm_driver.read_dc_volts(dmm, err: e1)
    e3 = cleanup(dmm2, err: e2)
    err = e3
}

fn forever(x: i32) -> (y: i32) {
    y = forever(x)
}

fn order() -> (a: i32, b: i32) {
    b = add(a, 1)
    a = 41
}
"""

    def setUp(self):
        self.td = tempfile.TemporaryDirectory()
        files = {}
        for block in re.split(r"\n(?=fn )", self.FNS):
            files[re.match(r"fn (\w+)", block).group(1) + ".lcf"] = self.HEADER + block.strip() + "\n"
        lib_dir(self.td.name, files)
        self.dir = self.td.name

    def tearDown(self):
        self.td.cleanup()

    def r(self, fn, sim=None, args=None, **kw):
        return run(fn, sim, args or {}, folder=self.dir, **kw)

    def test_check_ok(self):
        rep = api.check(self.dir)
        errs = [d for d in rep["diagnostics"] if d["severity"] == "error"]
        self.assertEqual(errs, [])

    def test_for_carry_collect(self):
        r = self.r("sum_scaled", args={"readings": "[1, 2, 3]"})
        self.assertEqual(r["outputs"]["total"], {"value": 6.0, "unit": "V"})
        self.assertEqual([x["value"] for x in r["outputs"]["scaled"]], [2.0, 4.0, 6.0])

    def test_units(self):
        r = self.r("units")
        self.assertEqual(r["outputs"]["a"]["unit"], "mV")
        self.assertAlmostEqual(r["outputs"]["a"]["value"], 700.0)
        self.assertEqual(r["outputs"]["p"]["unit"], "W")
        self.assertAlmostEqual(r["outputs"]["p"]["value"], 40.0)
        self.assertEqual(r["outputs"]["z"], 5.0)  # V ÷ V: dimensionless

    def test_unit_mismatch_is_reported(self):
        r = self.r("bad_units")
        self.assertEqual(r["fault"]["category"], "lcf.unit_mismatch")
        self.assertEqual(r["fault"]["function"], "bad_units")
        self.assertEqual(r["fault"]["line"], 9)

    def test_handle(self):
        r = self.r("guard", "dmm-230V.toml", {"resource": "SIM::DMM0"})
        self.assertTrue(r["ok"])
        r = self.r("guard", "dmm-timeout.toml", {"resource": "SIM::DMM0"})
        self.assertTrue(r["ok"], r)  # io.timeout handled: cleared
        self.assertEqual(r["outputs"]["value"]["value"], "nan")
        r = self.r("guard", "dmm-230V.toml", {"resource": "SIM::NOPE"})
        self.assertEqual(r["error"]["category"], "visa.resource_not_found")  # passed through by `_(e)`

    def test_handle_code_keeps_sign(self):
        r = self.r("by_code", "dmm-timeout.toml", {"resource": "SIM::DMM0"})
        self.assertTrue(r["ok"], r)
        self.assertIs(r["outputs"]["hit"], True)  # VI_ERROR_TMO = -1073807339: the sign matters

    def test_match(self):
        for n, label in ((0, "zero"), (3, "small"), (5, "small"), (6, "medium"), (10, "large"), (99, "large")):
            self.assertEqual(self.r("classify", args={"n": n})["outputs"]["label"], label, n)

    def test_loop_until(self):
        # `c` is the current value of the carry: `until` sees 7 at the 8th iteration, which writes 8.
        self.assertEqual(self.r("count_to", args={"limit": 7})["outputs"]["n"], 8)

    def test_order_of_lines_is_meaningless(self):
        self.assertEqual(self.r("order")["outputs"], {"a": 41, "b": 42})

    def test_endless_loop_is_bounded(self):
        r = self.r("spin", max_iter=500)
        self.assertEqual(r["fault"]["category"], "lcf.iteration_limit")

    def test_q7_error_in_loop(self):
        """Q-7: the loop stops by default when an error carry holds an error. With
        `continue_on_error`, the error skips `time.wait`, the clock stops advancing and `until` is never reached."""
        r = self.r("probe", max_iter=300)
        self.assertEqual(r["error"]["category"], "io.timeout")
        self.assertIsNone(r["fault"])
        self.assertEqual(self.r("probe_continue", max_iter=300)["fault"]["category"], "lcf.iteration_limit")

    def test_unitless_explicit_unit(self):
        r = self.r("strip")
        self.assertAlmostEqual(r["outputs"]["a"], 0.23)
        self.assertAlmostEqual(r["outputs"]["b"], 230.0)

    def test_always_inside_user_function(self):
        """A user function called with an error runs its body: its `visa.close` closes the session."""
        r = self.r("use_cleanup", "dmm-timeout.toml", {"resource": "SIM::DMM0"})
        self.assertEqual(r["error"]["category"], "io.timeout")
        self.assertEqual(r["sim"]["open_sessions"], [])

    def test_endless_recursion_is_bounded(self):
        r = self.r("forever", args={"x": 1})
        self.assertEqual(r["fault"]["category"], "lcf.recursion_limit")

    def test_bad_arguments(self):
        self.assertEqual(self.r("count_to", args={"limit": "abc"})["fault"]["category"], "lcf.bad_argument")
        self.assertEqual(self.r("count_to")["fault"]["category"], "lcf.bad_argument")
        self.assertEqual(self.r("nope")["fault"]["category"], "lcf.bad_argument")
        self.assertEqual(self.r("count_to", args={"limit": 1, "zzz": 2})["fault"]["category"], "lcf.bad_argument")
        self.assertEqual(run("count_to", "absent.toml", {"limit": 1}, folder=self.dir)["fault"]["category"],
                         "lcf.bad_simulation")


class Syntax(unittest.TestCase):
    """New syntax of the v0 grammar: one test per feature (spec section 14)."""

    HEADER = "labcraft 1\n\nuse time\nuse errors\nuse array\n\n"

    def lib(self, files):
        td = tempfile.TemporaryDirectory()
        self.addCleanup(td.cleanup)
        lib_dir(td.name, {k: (self.HEADER + v if k.endswith(".lcf") else v) for k, v in files.items()})
        return td.name

    def fn(self, src, fn=None, args=None, **files):
        folder = self.lib({"f.lcf": src, **files})
        name = fn or re.search(r"fn (\w+)", src).group(1)
        return api.check(folder), run(name, None, args or {}, folder=folder)

    def diag(self, rep, needle):
        hits = [d for d in rep["diagnostics"] if needle in d["message"]]
        self.assertTrue(hits, rep["diagnostics"])
        return hits[0]

    def test_unit_argument_in_brackets(self):
        _, r = self.fn("""fn u(x: f64) -> (v: f64[m/s], n: i32) {
    n = array.len([y])
    v = with_unit(x, [m/s])
    y = add(x, 1)
}
""", args={"x": 3})
        self.assertEqual(r["outputs"]["v"], {"value": 3.0, "unit": "m/s"})
        self.assertEqual(r["outputs"]["n"], 1)  # `[x]` given to a value parameter: an array (R-5)
        rep, _ = self.fn("fn u(x: f64) -> (v: f64[V]) {\n    v = with_unit(x, V)\n}\n")
        self.diag(rep, "written in brackets")

    def test_contextual_words(self):
        rep, r = self.fn("""fn c(x: f64) -> (ok: bool, code: i32, low: f64) {
    e1 = errors.raise_if(true, io.timeout, "boom")
    ok = errors.is_none(e1)
    code = e1.code
    low = max(x, -inf)
}
""", args={"x": 2})
        self.assertTrue(rep["ok"], rep["diagnostics"])
        self.assertEqual(r["outputs"], {"ok": False, "code": 0, "low": 2.0})
        rep, _ = self.fn("fn c(x: f64) -> (y: f64) {\n    inf = 1\n    y = x\n}\n")
        self.diag(rep, "syntax error")  # `nan` and `inf` are reserved (14.1)

    def test_after_label_and_list(self):
        rep, r = self.fn("""fn a() -> (t: f64[s], t2: f64[s]) {
    e1 = time.wait(20 ms)
    #pause: e2 = time.wait(50 ms)
    t = time.now(after: #pause)
    t2 = time.now(after: (#pause, e1))
}
""")
        self.assertTrue(rep["ok"], rep["diagnostics"])
        self.assertAlmostEqual(r["outputs"]["t"]["value"], 0.07)
        self.assertAlmostEqual(r["outputs"]["t2"]["value"], 0.07)
        rep, _ = self.fn("fn a() -> (t: f64[s]) {\n    t = time.now(after: #nope)\n}\n")
        self.diag(rep, "unknown label '#nope'")

    def test_after_on_a_node_with_err(self):
        rep, r = self.fn("""fn a(err: error = none) -> (t: f64[s], err: error) {
    e1 = time.wait(10 ms, err: err)
    e2 = time.wait(30 ms, after: e1)
    t = time.now(after: e2)
    err = e2
}
""")
        self.assertTrue(rep["ok"], rep["diagnostics"])
        self.assertAlmostEqual(r["outputs"]["t"]["value"], 0.04)

    def test_call_statement_without_output(self):
        rep, r = self.fn("""fn s() -> (t: f64[s]) {
    #pause: time.wait(50 ms)
    time.wait(20 ms)
    t = time.now(after: #pause)
}
""")
        self.assertEqual(rep["errors"], 0, rep["diagnostics"])
        self.diag(rep, "`err` output of 'time.wait' is not wired")  # 7.5 applies (R-16)
        self.assertEqual(r["sim"]["time_ms"], 70.0)

    def test_two_statements_on_one_line_are_rejected(self):
        rep, _ = self.fn("fn o(a: i32) -> (b: i32) {\n    x = add(a, 1) b = x\n}\n")
        d = self.diag(rep, "share a line")
        self.assertEqual(d["line"], 8)
        rep, _ = self.fn("fn o(a: i32) -> (b: i32) {\n    b =\n        add(a, 1)\n}\n")
        self.diag(rep, "a line ending ends a statement")
        rep, _ = self.fn("fn o(a: i32) -> (b: i32) {\n    b = add(a,\n            1)\n}\n")  # inside ( ): ignored
        self.assertTrue(rep["ok"], rep["diagnostics"])

    def test_variant_pattern_binds_several_names(self):
        _, r = self.fn("""fn v(a: i32, b: i32) -> (s: i32) {
    p = Pair.Two(a, b)
    s = match p {
        Pair.Two(x, y) => {
            t = add(x, y)
            yield t
        }
        Pair.Empty => yield 0
    }
}
""", args={"a": 2, "b": 3}, **{"pair.lct": "labcraft 1\n\ntype Pair = enum { Two(i32, i32), Empty }\n"})
        self.assertEqual(r["outputs"]["s"], 5)

    def test_literals_and_patterns(self):
        _, r = self.fn("""fn l(n: i32) -> (x: f64, label: string) {
    x = 1_000.5_0
    label = match n {
        -5..=5 => yield "near"
        _ => yield "far"
    }
}
""", args={"n": -3})
        self.assertEqual(r["outputs"], {"x": 1000.5, "label": "near"})

    def test_grammar_restrictions(self):
        loop = "    b = loop carry n = 0 {{\n{}\n    }}\n"
        bad = {
            "next in a function body": "    next b = a",
            "`until` in a `for`": "    b = for x in [a] {\n        until true\n    }",
            "`loop` without `until`": loop.format("        next n = add(n, 1)"),
            "two `until`": loop.format("        next n = add(n, 1)\n        until true\n        until true"),
            "`next` in an arm": loop.format("        x = match a {\n            _ => {\n                next n = 1\n"
                                            "                yield true\n            }\n        }\n        until x"),
            "`for` without `in`": "    b = for carry c = 0 {\n        next c = 1\n    }",
            "two `yield`": "    b = match a {\n        _ => {\n            yield 1\n            yield 2\n        }\n    }",
            "`///` inside a body": "    /// doc\n    b = a",
            "name starting with `_`": "    _x = a\n    b = _x",
            "unknown escape": '    s = string.len("a\\qb")\n    b = a',
            "line ending in a string": '    s = string.len("a\nb")\n    b = a',
        }
        for what, body in bad.items():
            rep, _ = self.fn("fn g(a: i32) -> (b: i32) {\n" + body + "\n}\n")
            self.assertTrue([d for d in rep["diagnostics"] if "syntax error" in d["message"]], what)

    def test_one_next_per_carry(self):
        rep, _ = self.fn("fn g(a: i32) -> (b: i32) {\n    b = loop carry n = 0 {\n        until true\n    }\n}\n")
        self.diag(rep, "carry 'n' has 0 `next`")

    def test_layout_wire_terminals(self):
        rep, _ = self.fn("""fn w(a: i32) -> (b: i32) {
    #inc: b = add(a, 1)
}

layout {
    node #inc at (40, 80)
    wire a.a -> #inc.a via (20, 80)
}
""")
        self.assertTrue(rep["ok"], rep["diagnostics"])

    def test_canonical_order(self):
        from lcfpoc.canon import check_fn
        from lcfpoc.ir import lower_file
        from lcfpoc.library import parse
        for f in sorted(Path(EX).glob("*.lcf")):
            src = f.read_text(encoding="utf-8")
            fn, _, _, _ = lower_file(src, f.name, parse(src))
            self.assertEqual(check_fn(fn), [], f.name)
        src = (Path(EX) / "check_voltage.lcf").read_text(encoding="utf-8")
        swapped = src.replace("    above = ge(voltage, low)\n    below = le(voltage, high)\n",
                              "    below = le(voltage, high)\n    above = ge(voltage, low)\n")
        self.assertNotEqual(src, swapped)
        fn, _, _, _ = lower_file(swapped, "x.lcf", parse(swapped))
        (line, msg), = check_fn(fn)
        self.assertIn("expected `above = ge(…)`", msg)


class Hostile(unittest.TestCase):
    """Criterion 3: never a crash, always an error reported with file and line."""

    GOOD = """labcraft 1

fn ok(a: i32) -> (b: i32) {
    b = add(a, 1)
}
"""

    def diags(self, files):
        with tempfile.TemporaryDirectory() as td:
            lib_dir(td, files)
            rep = api.check(td)
            r = api.run(td, "ok", None, {"a": 1})
        return rep, r

    def assert_reported(self, files, needle):
        rep, r = self.diags(files)
        self.assertFalse(rep["ok"])
        hits = [d for d in rep["diagnostics"] if needle in d["message"]]
        self.assertTrue(hits, rep["diagnostics"])
        self.assertTrue(hits[0]["file"].endswith((".lcf", ".lct")))
        self.assertEqual(r["fault"]["category"], "lcf.check_failed")
        return hits[0]

    def test_truncated(self):
        d = self.assert_reported({"t.lcf": self.GOOD[:40]}, "syntax error")
        self.assertEqual(d["file"], "t.lcf")

    def test_unknown_value(self):
        d = self.assert_reported({"t.lcf": self.GOOD.replace("add(a, 1)", "add(aa, 1)")}, "unknown value")
        self.assertEqual(d["line"], 4)
        self.assertIn("'a'", d["message"])

    def test_unknown_function(self):
        self.assert_reported({"t.lcf": self.GOOD.replace("add(", "addd(")}, "unknown function")

    def test_unbound_output(self):
        self.assert_reported({"t.lcf": self.GOOD.replace("    b = add(a, 1)\n", "")}, "is not bound")

    def test_two_definitions(self):
        self.assert_reported({"ok.lcf": self.GOOD, "ok2.lcf": self.GOOD}, "defined twice")

    def test_value_defined_twice(self):
        self.assert_reported({"t.lcf": self.GOOD.replace("    b = add(a, 1)", "    b = add(a, 1)\n    b = add(a, 2)")},
                             "defined twice")

    def test_cycle(self):
        self.assert_reported({"t.lcf": self.GOOD.replace("    b = add(a, 1)", "    x = add(y, 1)\n    y = add(x, 1)\n    b = x")},
                             "cycle")

    def test_missing_input(self):
        self.assert_reported({"t.lcf": self.GOOD.replace("add(a, 1)", "add(a)")}, "required input")

    def test_handle_without_wildcard(self):
        src = "labcraft 1\n\nfn ok(a: i32, err: error = none) -> (b: i32) {\n    b = handle err {\n        ok => yield a\n    }\n}\n"
        self.assert_reported({"t.lcf": src}, "handle without a `_` arm")

    def test_unit_unknown(self):
        self.assert_reported({"t.lcf": self.GOOD.replace("add(a, 1)", "add(a, 1 qux)")}, "unknown unit")

    def test_binary_and_empty_and_deep(self):
        with tempfile.TemporaryDirectory() as td:
            Path(td, "bin.lcf").write_bytes(bytes(range(256)))
            Path(td, "empty.lcf").write_text("")
            Path(td, "deep.lcf").write_text("labcraft 1\nfn f() -> () {" + "x = match y {" * 300)
            Path(td, "big.lcf").write_text("x" * 1_100_000)
            rep = api.check(td)
            self.assertFalse(rep["ok"])
            self.assertEqual({d["file"] for d in rep["diagnostics"]}, {"bin.lcf", "empty.lcf", "deep.lcf", "big.lcf"})

    def test_missing_folder(self):
        self.assertFalse(api.check("/nonexistent/folder")["ok"])


if __name__ == "__main__":
    unittest.main()
