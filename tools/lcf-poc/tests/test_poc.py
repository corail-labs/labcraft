"""Acceptance tests of the PoC (criteria 1 to 4 and 6): python -m unittest discover tools/lcf-poc/tests"""
import json
import re
import shutil
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
        # `scpi.query_f64` is written in LabCraft: its body runs and its native nodes are skipped (7.2)
        self.assertEqual(skipped, ["visa.query", "string.to_f64"])

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
        slip = src.replace("next x = core.add(x, dx)", "next x = core.add(x, vx)")
        self.assertNotEqual(src, slip)
        with tempfile.TemporaryDirectory() as td:
            lib_dir(td, {"projectile.lcf": slip})
            r = run("projectile", None, {"vx0": "10 m/s", "vy0": "10 m/s", "mass": "0.1 kg"}, folder=td)
        self.assertEqual(r["fault"]["category"], "lcf.unit_mismatch")
        self.assertEqual(r["fault"]["line"], slip.splitlines().index("        next x = core.add(x, vx)") + 1)


class Language(unittest.TestCase):
    HEADER = "labcraft 1\n\nuse core\nuse visa\nuse dmm_driver\nuse time\nuse errors\n\n"
    FNS = """fn sum_scaled(readings: [f64[V]]) -> (total: f64[V], scaled: [f64[V]]) {
    total, scaled = for x in readings, carry acc = 0 V {
        s = core.mul(x, 2.0)
        next acc = core.add(acc, x)
        collect s
    }
}

fn units() -> (a: f64[mV], p: f64[W], z: f64) {
    a = core.add(500 mV, 0.2 V)
    r = 10 Ω
    i = 2 A
    v = core.mul(r, i)
    p = core.mul(v, i)
    q = core.div(v, 4 V)
    z = q
}

fn bad_units() -> (x: f64[V]) {
    x = core.add(1 V, 2 A)
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
        next c = core.add(c, 1)
        done = core.ge(c, limit)
        until done
    }
}

fn spin() -> (n: i32) {
    n = loop carry c = 0 {
        next c = core.add(c, 1)
        never = core.lt(c, 0)
        until never
    }
}

fn probe(err: error = none) -> (err: error) {
    t0 = time.now(after: err)
    _, err_out = loop carry n = 0, carry e = err {
        e1 = errors.raise_if(true, io.timeout, "boom", err: e)
        e2 = time.wait(50 ms, err: e1)
        t = time.elapsed(t0)
        done = core.gt(t, 1 s)
        next n = core.add(n, 1)
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
        done = core.gt(t, 1 s)
        next n = core.add(n, 1)
        next e = e2
        until done
    }
    err = err_out
}

fn strip() -> (a: f64, b: f64) {
    a = core.unitless(230 mV, [V])
    b = core.unitless(230 mV, [mV])
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
    b = core.add(a, 1)
    a = 41
}
"""

    def setUp(self):
        self.td = tempfile.TemporaryDirectory()
        files = {}
        for block in re.split(r"\n(?=fn )", self.FNS):
            files[re.match(r"fn (\w+)", block).group(1) + ".lcf"] = self.HEADER + block.strip() + "\n"
        lib_dir(self.td.name, files)
        shutil.copytree(Path(EX) / "dmm_driver", Path(self.td.name) / "dmm_driver")
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
        self.assertEqual(r["fault"]["line"], 10)

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

    HEADER = "labcraft 1\n\nuse core\nuse time\nuse errors\nuse array\n\n"

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

    def test_own_function_named_like_core(self):
        rep, r = self.fn("fn f(a: i32) -> (x: i32, y: i32) {\n    x = add(a, 2)\n    y = core.add(a, 2)\n}\n",
                         args={"a": 5},
                         **{"add.lcf": "fn add(a: i32, b: i32) -> (result: i32) {\n    result = core.mul(a, b)\n}\n"})
        self.assertTrue(rep["ok"], rep["diagnostics"])
        self.assertEqual(r["outputs"], {"x": 10, "y": 7})

    def test_unit_argument_in_brackets(self):
        _, r = self.fn("""fn u(x: f64) -> (v: f64[m/s], n: i32) {
    n = array.len([y])
    v = core.with_unit(x, [m/s])
    y = core.add(x, 1)
}
""", args={"x": 3})
        self.assertEqual(r["outputs"]["v"], {"value": 3.0, "unit": "m/s"})
        self.assertEqual(r["outputs"]["n"], 1)  # `[x]` given to a value parameter: an array (R-5)
        rep, _ = self.fn("fn u(x: f64) -> (v: f64[V]) {\n    v = core.with_unit(x, V)\n}\n")
        self.diag(rep, "written in brackets")

    def test_contextual_words(self):
        rep, r = self.fn("""fn c(x: f64) -> (ok: bool, code: i32, low: f64) {
    e1 = errors.raise_if(true, io.timeout, "boom")
    ok = errors.is_none(e1)
    code = e1.code
    low = core.max(x, -inf)
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
        rep, _ = self.fn("fn o(a: i32) -> (b: i32) {\n    x = core.add(a, 1) b = x\n}\n")
        d = self.diag(rep, "share a line")
        self.assertEqual(d["line"], 9)
        rep, _ = self.fn("fn o(a: i32) -> (b: i32) {\n    b =\n        core.add(a, 1)\n}\n")
        self.diag(rep, "a line ending ends a statement")
        rep, _ = self.fn("fn o(a: i32) -> (b: i32) {\n    b = core.add(a,\n            1)\n}\n")  # inside ( ): ignored
        self.assertTrue(rep["ok"], rep["diagnostics"])

    def test_variant_pattern_binds_several_names(self):
        _, r = self.fn("""fn v(a: i32, b: i32) -> (s: i32) {
    p = Pair.Two(a, b)
    s = match p {
        Pair.Two(x, y) => {
            t = core.add(x, y)
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
            "`loop` without `until`": loop.format("        next n = core.add(n, 1)"),
            "two `until`": loop.format("        next n = core.add(n, 1)\n        until true\n        until true"),
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
    #inc: b = core.add(a, 1)
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
        swapped = src.replace("    above = core.ge(voltage, low)\n    below = core.le(voltage, high)\n",
                              "    below = core.le(voltage, high)\n    above = core.ge(voltage, low)\n")
        self.assertNotEqual(src, swapped)
        fn, _, _, _ = lower_file(swapped, "x.lcf", parse(swapped))
        (line, msg), = check_fn(fn)
        self.assertIn("expected `above = core.ge(…)`", msg)

    def test_canonical_skipped_on_edit_time_error(self):
        from lcfpoc.canon import check_fn
        from lcfpoc.ir import lower_file
        from lcfpoc.library import parse

        def probs(body):
            src = "labcraft 1\n\nfn f(a: i32) -> (x: i32) {\n" + body + "}\n"
            fn, _, _, _ = lower_file(src, "f.lcf", parse(src))
            return check_fn(fn)
        # a name the file does not define creates no dependency: the body is still ordered (5.2)
        self.assertEqual(len(probs("    z = core.add(q, 1)\n    b = core.add(a, 1)\n    x = b\n")), 1)
        # value defined twice: written order kept
        self.assertEqual(probs("    z = core.add(a, 1)\n    z = core.add(a, 2)\n    b = core.add(a, 1)\n    x = b\n"), [])
        # the same for an output that is also an input, and for a label
        self.assertEqual(probs("    x = a\n    x = core.add(a, 1)\n    b = core.add(a, 1)\n"), [])
        self.assertEqual(probs("    #l: z = core.add(a, 1)\n    #l: b = core.add(a, 1)\n    x = b\n"), [])
        # cycle: written order kept
        self.assertEqual(probs("    m = core.add(a, 1)\n    b = core.add(a, 1)\n    z = core.add(y, 1)\n"
                               "    y = core.add(z, b)\n    x = core.add(m, y)\n"), [])
        # an ordinary body that reads an input is still checked
        self.assertEqual(len(probs("    m = core.add(a, 2)\n    b = core.add(a, 1)\n    x = core.add(m, b)\n")), 1)

    def test_layout_items_of_panel_and_layout(self):
        head = "labcraft 1\n\nfn f(a: i32, b: i32) -> (x: i32, y: i32) {}\n\n"
        self.assertEqual(self.layout(head + "panel {\n    control a: numeric\n    control b: numeric\n"
                                            "    control z: button\n    indicator x: numeric\n"
                                            "    indicator y: numeric\n    indicator c: chart\n}\n"), [])
        for items in ("    control b: numeric\n    control a: numeric\n",  # signature order
                      "    control z: button\n    control a: numeric\n",  # terminals first
                      "    indicator x: numeric\n    control a: numeric\n",  # controls first
                      "    control z: button\n    control c: button\n"):  # others by name
            self.assertTrue(any("items of `panel`" in m for _, m in self.layout(head + "panel {\n" + items + "}\n")),
                            items)
        self.assertEqual(self.layout(head + "layout {\n    node #close at (1, -2)\n    node dmm at (3, 4)\n"
                                            "    wire a -> b\n    wire a -> c via (5, 6) (1, 1)\n}\n"), [])
        for items in ("    node dmm at (3, 4)\n    node #close at (1, 2)\n",
                      "    wire a -> b\n    node dmm at (3, 4)\n",
                      "    wire b -> a\n    wire a -> c\n",
                      "    wire a -> c\n    wire a -> b\n"):
            self.assertTrue(any("items of `layout`" in m for _, m in self.layout(head + "layout {\n" + items + "}\n")),
                            items)
        for value in ("1_000", "-0", "007", "1_", "1__0"):
            self.assertTrue(any("plain decimal" in m for _, m in
                                self.layout(head + f"layout {{\n    node a at ({value}, 2)\n}}\n")), value)
        for sect in ("panel {}\n", "layout {}\n", "panel {\n}\n"):
            self.assertTrue(any("empty" in m for _, m in self.layout(head + sect)), sect)
        # a comment is not an item: the section stays
        for sect in ("layout {\n    // later\n}\n", "panel {} // later\n", "layout {\n} // later\n",
                     "// later\npanel {}\n"):
            self.assertFalse(any("empty" in m for _, m in self.layout(head + sect)), sect)
        # any other coordinate is for `check`, and keeps its spelling
        self.assertFalse(any("plain decimal" in m for _, m in self.layout(head + "layout {\n    node a at (5.0, 1e3)\n}\n")))

    def test_layout_single_value_has_no_parentheses(self):
        src = "labcraft 1\n\nfn f(c: bool, a: i32) -> (x: i32) {\n    x = match c {\n        true => yield %s\n        false => yield a\n    }\n}\n"
        self.assertEqual(self.layout(src % "a"), [])
        self.assertTrue(any("parentheses" in m for _, m in self.layout(src % "(a)")))
        call = "labcraft 1\n\nfn f(e2: error) -> (t: f64) {\n    t = g(%s)\n}\n"
        self.assertEqual(self.layout(call % "after: e2"), [])
        self.assertEqual(self.layout(call % "after: (#b, e2)"), [])
        self.assertTrue(any("parentheses" in m for _, m in self.layout(call % "after: (e2)")))

    def test_layout_names_have_one_spelling(self):
        body = "labcraft 1\n\nfn f() -> (x: f64[%s]) {\n    x = 1 %s\n}\n"
        self.assertEqual(self.layout(body % ("\u00b5V", "\u00b5V")), [])
        self.assertEqual(self.layout(body % ("k\u03a9", "k\u03a9")), [])
        for spelled in ("\u03bcV", "k\u2126"):
            self.assertTrue(any("R-20" in m for _, m in self.layout(body % (spelled, spelled))), spelled)
        decomposed = "labcraft 1\n\nfn f(te\u0301: i32) -> (x: i32) {\n    x = te\u0301\n}\n"
        self.assertTrue(any("R-20" in m for _, m in self.layout(decomposed)))
        # strings and comments keep their text
        self.assertEqual(self.layout('labcraft 1\n\nfn f() -> (x: string) {\n    x = "\u03bc k\u2126" // \u03bc\n}\n'), [])

    def test_names_follow_the_lexicon(self):
        from lark.exceptions import LarkError
        from lcfpoc.library import parse
        src = "labcraft 1\n\nfn f(%s: i32) -> (x: i32) {\n    x = %s\n}\n"
        for name in ("te\u0301", "x\u20d7", "\u0915\u093f", "fn\u0300", "\u00b5", "\u03a9"):  # marks, letters
            parse(src % (name, name))
        for name in ("\u00b2x", "x\u00b2", "\u0663"):  # a superscript and a non-ASCII digit are not in a name
            with self.assertRaises(LarkError, msg=name):
                parse(src % (name, name))
        with self.assertRaises(LarkError):
            parse("labcraft 1\n\nfn f() -> (x: i32) {\n    x = \u0663\n}\n")

    def test_layout_fn_type_unary_minus_trailing_comment(self):
        g = "labcraft 1\n\nfn f(a: i32) -> (x: i32) {\n    x = core.add(a, -1)\n}\n"
        self.assertEqual(self.layout(g), [])
        self.assertBreaks(g.replace("-1", "- 1"), 4, "unary minus")
        t = ("labcraft 1\n\nfn f(m: fn(visa.session, err: error) -> (x: i32)) -> (x: i32) {\n"
             "    x = core.add(1, 2)\n}\n")
        self.assertEqual(self.layout(t), [])
        self.assertBreaks(t.replace("fn(visa", "fn (visa"), 3, "no space between `fn`")
        body = "    x = core.add(a, -1)\n"
        self.assertEqual(self.layout(g.replace(body, body + "    // trailing\n")), [])
        self.assertBreaks(g.replace(body, body + "// trailing\n"), 5, "indentation")
        self.assertBreaks(g.replace(body, body + "        // trailing\n"), 5, "indentation")
        m = ("labcraft 1\n\nfn f() -> () {\n    m = match 1 {\n        _ => {\n            q = core.add(1, 2)\n            yield q\n%s"
             "        }\n    }\n}\n")
        self.assertEqual(self.layout(m % "            // trailing\n"), [])
        self.assertBreaks(m % "        // trailing\n", 8, "indentation")

    def test_fmt_check_empty_file_line(self):
        import subprocess
        import sys
        import tempfile
        with tempfile.TemporaryDirectory() as d:
            for name, text in (("e.lcf", ""), ("h.lcf", "labcraft 1\n")):
                (Path(d) / name).write_text(text, encoding="utf-8", newline="\n")
            r = subprocess.run([sys.executable, str(Path(__file__).parent.parent / "lcf.py"), "fmt", "--check", d],
                               capture_output=True, text=True)
        self.assertEqual(r.returncode, 1)
        self.assertNotIn(":-1:", r.stdout)
        self.assertIn("e.lcf:1:", r.stdout)

    GOOD_LAYOUT = (
        "labcraft 1\n\nuse a\nuse b\n\n// note\n/// Doc.\nfn f(x: i32 = 1, y: f64[V] = 2 V) -> (z: i32) {\n"
        "    z = core.add(x, 1) // end\n    m = match z {\n        true => yield 1\n    }\n}\n")

    def layout(self, src):
        from lcfpoc.layout import check_layout
        return check_layout(src)

    def assertBreaks(self, src, line, text):
        got = [m for n, m in self.layout(src) if n == line]
        self.assertTrue(any(text in m for m in got), (text, self.layout(src)))

    def test_layout_examples_pass(self):
        from lcfpoc.layout import check_layout
        self.assertEqual(check_layout(self.GOOD_LAYOUT), [])
        for folder in (EX, str(ROOT / "docs" / "rfc")):
            for f in sorted(Path(folder).glob("*.lc[ft]")):
                self.assertEqual(check_layout(f.read_bytes().decode("utf-8")), [], f.name)

    def test_layout_file(self):
        g = self.GOOD_LAYOUT
        self.assertBreaks(g.replace("\n", "\r\n"), 1, "CR LF")
        self.assertBreaks(g[:-1], 13, "does not end with a line feed")
        self.assertBreaks(g + "\n", 14, "more than one line feed")
        self.assertBreaks(g.replace("core.add(x, 1) // end", "core.add(x, 1) // end "), 9, "end of the line")

    def test_layout_sections(self):
        g = self.GOOD_LAYOUT
        self.assertBreaks(g.replace("use a\n", "use a\n\n"), 5, "no blank line")
        self.assertBreaks(g.replace("use b\n\n", "use b\n"), 5, "separated by one blank line")
        self.assertBreaks(g.replace("// note\n", "// note\n\n"), 8, "no blank line")
        self.assertBreaks(g.replace("use b\n\n", "use b\n\n\n"), 7, "more than one blank")
        self.assertBreaks(g.replace("    z = core.add", "\n    z = core.add"), 9, "inside a body")
        self.assertBreaks(g.replace("use a\nuse b", "use b\nuse a"), 3, "not sorted")
        self.assertBreaks(g + "panel {}\n", 14, "one blank line")

    def test_layout_indentation(self):
        g = self.GOOD_LAYOUT
        self.assertBreaks(g.replace("    z = core.add", "  z = core.add"), 9, "expected 4")
        self.assertBreaks(g.replace("    z = core.add", "\tz = core.add"), 9, "tab")
        self.assertBreaks(g.replace("        true =>", "          true =>"), 11, "expected 8")
        self.assertBreaks(g.replace("// note", "  // note"), 6, "expected 0")

    def test_layout_spaces(self):
        g = self.GOOD_LAYOUT
        self.assertBreaks(g.replace("core.add(x, 1)", "core.add(x,1)"), 9, "after a comma")
        self.assertBreaks(g.replace("core.add(x, 1)", "core.add(x , 1)"), 9, "before a comma")
        self.assertBreaks(g.replace("x: i32", "x : i32"), 8, "before `:`")
        self.assertBreaks(g.replace("x: i32", "x:i32"), 8, "after `:`")
        self.assertBreaks(g.replace("z = core.add", "z= core.add"), 9, "each side of `=`")
        self.assertBreaks(g.replace("-> (z", "->(z"), 8, "each side of `->`")
        self.assertBreaks(g.replace("true =>", "true=>"), 11, "each side of `=>`")
        self.assertBreaks(g.replace("core.add(x, 1)", "core.add( x, 1)"), 9, "after `(`")
        self.assertBreaks(g.replace("core.add(x, 1)", "core.add(x, 1 )"), 9, "before `)`")
        self.assertBreaks(g.replace("f64[V]", "f64[ V]"), 8, "after `[`")
        self.assertBreaks(g + "type T = struct {a: i32 }\n", 14, "after `{`")
        self.assertBreaks(g + "type T = struct { a: i32}\n", 14, "before `}`")
        self.assertBreaks(g.replace("2 V", "2V"), 8, "number `2` and its unit")
        self.assertEqual(self.layout(g.replace("core.add(x, 1)", "core.add(x, 1e3)").replace("2 V", "4.7 kΩ")), [])
        self.assertEqual(self.layout(g.replace("core.add(x, 1)", 'core.add("a,b  :{(", 1)')), [])

    def test_layout_alignment_and_comments(self):
        g = self.GOOD_LAYOUT
        self.assertBreaks(g.replace("true => yield", "true  => yield"), 11, "alignment")
        self.assertBreaks(g.replace("1) // end", "1)  // end"), 9, "exactly one space")
        self.assertBreaks(g.replace("1) // end", "1)// end"), 9, "exactly one space")
        self.assertBreaks(g.replace("// end", "//end"), 9, "at least one space")
        self.assertBreaks(g.replace("/// Doc.", "///Doc."), 7, "at least one space")
        self.assertEqual(self.layout(g.replace("// end", "//")), [])

    def test_layout_width(self):
        g = self.GOOD_LAYOUT
        long = "a_long_parameter_name: f64[V] = 220 V"
        one = f"fn f({long}, {long.replace('a_', 'b_')}, {long.replace('a_', 'c_')}) -> (z: i32) {{\n"
        self.assertGreater(len(one), 100)
        src = g.replace(g.splitlines()[7] + "\n", one)
        self.assertBreaks(src, 8, "over 100")
        split = (one.replace("fn f(", "fn f(\n    ").replace(", ", ",\n    ").replace(") ->", ",\n) ->"))
        self.assertEqual(self.layout(g.replace(g.splitlines()[7] + "\n", split)), [])
        self.assertBreaks(g.replace("fn f(x: i32 = 1, y: f64[V] = 2 V)", "fn f(\n    x: i32 = 1,\n    y: f64[V] = 2 V,\n)"),
                          8, "fits on one line")
        merged = split.replace(",\n    b_", ", b_")
        self.assertBreaks(g.replace(g.splitlines()[7] + "\n", merged), 9, "one parameter per line")
        self.assertBreaks(g.replace("fn f(x: i32 = 1, y: f64[V] = 2 V)",
                                    "fn f(\n    x: i32 = 1 // c\n    y: f64[V] = 2 V,\n)"), 9, "ends with a comma")
        t = "type S = struct { a: i32, b: i32 }\n"
        self.assertEqual(self.layout("labcraft 1\n\n" + t), [])
        self.assertBreaks("labcraft 1\n\ntype S = struct {\n    a: i32,\n    b: i32,\n}\n", 3, "fits on one line")
        self.assertEqual(self.layout("labcraft 1\n\ntype S = struct {\n    a: i32, // c\n    b: i32,\n}\n"), [])
        self.assertEqual(self.layout("labcraft 1\n\ntype S = enum {\n    A, // c\n    B,\n}\n"), [])
        loop = ("    r = loop\n        carry a = 0,\n        carry b = 1\n    {\n        until a\n    }\n")
        body = "labcraft 1\n\nfn f() -> (r: i32) {\n" + loop + "}\n"
        self.assertBreaks(body, 4, "fit on one line")
        wide = body.replace("carry a = 0", "carry a" + "x" * 80 + " = 0")
        self.assertEqual(self.layout(wide), [])
        one_line = wide.replace("loop\n        carry", "loop carry").replace(",\n        carry", ", carry").replace("\n    {", " {")
        self.assertBreaks(one_line, 4, "over 100")
        self.assertBreaks(wide.replace("    {\n        until", "        {\n        until"), 7, "expected 4")
        self.assertBreaks(wide.replace("carry b = 1", "carry b = 1,"), 6, "no comma after the last")
        self.assertBreaks(wide.replace("= 0,", "= 0"), 5, "separated by commas")
        self.assertBreaks(g.replace("core.add(x, 1)", "core.add(x,\n        1)"), 9, "stays on one line")

    HDR = "labcraft 1\n\n"

    def body(self, stmts):
        return self.HDR + "fn f() -> (r: i32) {\n" + stmts + "}\n"

    def test_layout_struct_width_boundary(self):
        def one(k):
            return "type S = struct { a: i32, " + "é" * k + ": i32 }"

        def split(k):
            return "type S = struct {\n    a: i32,\n    " + "é" * k + ": i32,\n}"
        k = 100 - len(one(0))
        self.assertEqual(len(one(k)), 100)
        self.assertGreater(len(one(k).encode("utf-8")), 100)  # counted in code points, not bytes
        self.assertEqual(self.layout(self.HDR + one(k) + "\n"), [])
        self.assertBreaks(self.HDR + split(k) + "\n", 3, "fits on one line (100 characters)")
        self.assertBreaks(self.HDR + one(k + 1) + "\n", 3, "over 100")
        self.assertEqual(self.layout(self.HDR + split(k + 1) + "\n"), [])
        # the suggested one-line text is canonical: no comma before the brace
        got = [m for n, m in self.layout(self.HDR + split(0) + "\n")]
        self.assertTrue(any("expected `type S = struct { a: i32, : i32 }`" in m for m in got), got)
        self.assertEqual(self.layout(self.HDR + "type E = enum { A, B }\n"), [])
        self.assertBreaks(self.HDR + "type E = enum {\n    A,\n    B,\n}\n", 3, "fits on one line")

    def test_layout_trailing_comma_one_line(self):
        self.assertBreaks(self.HDR + "type S = struct { a: i32, b: i32, }\n", 3, "no comma before `}`")
        self.assertBreaks(self.HDR + "type E = enum { A, B, }\n", 3, "no comma before `}`")
        self.assertBreaks(self.body("    r = core.add(a, b,)\n"), 4, "no comma before `)`")
        self.assertEqual(self.layout(self.HDR + "type S = struct {}\n"), [])
        self.assertBreaks(self.HDR + "type S = struct { }\n", 3, "empty braces")

    def test_layout_signature_width_boundary(self):
        def one(k):
            return "fn f(a: i32) -> (" + "é" * k + ": i32) {"
        k = 100 - len(one(0))
        self.assertEqual(len(one(k)), 100)
        self.assertEqual(self.layout(self.HDR + one(k) + "\n}\n"), [])
        sp = "fn f(\n    a: i32,\n) -> (" + "é" * (k + 1) + ": i32) {\n}\n"
        self.assertBreaks(self.HDR + one(k + 1) + "\n}\n", 3, "over 100")
        self.assertEqual(self.layout(self.HDR + sp), [])
        self.assertBreaks(self.HDR + sp.replace("é" * (k + 1), "é" * k), 3, "fits on one line (100 characters)")

    def test_layout_output_split(self):
        o = "o" * 40
        good = f"fn f(\n    a: i32,\n) -> (\n    {o}1: i32,\n    {o}2: i32,\n) {{\n}}\n"
        self.assertEqual(self.layout(self.HDR + good), [])
        # `) -> (outputs) {` fits: the outputs are not split
        pa, pb = "a" * 50, "b" * 50
        short = f"fn f(\n    {pa}: i32,\n    {pb}: i32,\n) -> (\n    x: i32,\n    y: i32,\n) {{\n}}\n"
        self.assertBreaks(self.HDR + short, 6, "outputs fit on one line")
        # a comment in the outputs forces the split form
        com = "fn f(\n    a: i32,\n) -> (\n    x: i32, // c\n    y: i32,\n) {\n}\n"
        self.assertEqual(self.layout(self.HDR + com), [])
        # each output ends with a comma
        self.assertBreaks(self.HDR + good.replace(f"{o}2: i32,", f"{o}2: i32"), 7, "ends with a comma")

    def test_layout_zero_parameter_signature(self):
        o = "o" * 45
        long_one = f"fn f() -> ({o}1: i32, {o}2: i32) {{\n}}\n"
        self.assertGreater(len(long_one.splitlines()[0]), 100)
        self.assertBreaks(self.HDR + long_one, 3, "over 100")
        good = f"fn f() -> (\n    {o}1: i32,\n    {o}2: i32,\n) {{\n}}\n"
        self.assertEqual(self.layout(self.HDR + good), [])
        wrong = f"fn f(\n) -> (\n    {o}1: i32,\n    {o}2: i32,\n) {{\n}}\n"
        self.assertBreaks(self.HDR + wrong, 3, "without parameters")
        self.assertBreaks(self.HDR + "fn f() -> (\n    x: i32,\n) {\n}\n", 3, "fits on one line")

    def test_layout_extern(self):
        ex = 'extern python(module: "a", function: "b")\nfn spectrum(samples: [f64[V]]) -> (err: error)\n'
        self.assertEqual(self.layout(self.HDR + "/// Doc.\n" + ex), [])
        self.assertBreaks(self.HDR + ex.replace(")\nfn", ")\n\nfn"), 5, "directly by its `fn` line")
        # a bodyless function never reaches the next section
        self.assertEqual(self.layout(self.HDR + ex + "\npanel {\n    control samples: numeric\n}\n"), [])
        many = ex.replace("(samples: [f64[V]])", "(\n    samples: [f64[V]],\n)")
        self.assertBreaks(self.HDR + many, 4, "fits on one line")
        two = self.HDR + ex + 'extern c(library: "a", symbol: "b")\nfn g() -> (err: error)\n'
        self.assertBreaks(two, 5, "separated by one blank line")

    def test_layout_struct_construction(self):
        self.assertEqual(self.layout(self.body("    r = S { x: a, y: b }\n")), [])
        self.assertBreaks(self.body("    r = S {\n        x: a,\n    }\n"), 4, "struct construction")
        self.assertBreaks(self.body("    r = lib.S {\n        x: a,\n    }\n"), 4, "struct construction")
        wide = "    r = S { " + ", ".join(f"field{i}: value{i}" for i in range(14)) + " }\n"
        self.assertGreater(len(wide), 101)
        self.assertEqual(self.layout(self.body(wide)), [])  # whatever its length
        self.assertBreaks(self.body("    r = S {x: a }\n"), 4, "after `{`")

    def test_layout_for_and_loop_clauses(self):
        fits = "    r = for\n        x in a,\n        carry c = 0\n    {\n        collect x\n    }\n"
        self.assertBreaks(self.body(fits), 4, "fit on one line")
        one = "    r = for x in a, carry c = 0 {\n        collect x\n    }\n"
        self.assertEqual(self.layout(self.body(one)), [])
        wide = "    r = for\n        x in " + "x" * 70 + ",\n        carry c = 0\n    {\n        collect x\n    }\n"
        self.assertEqual(self.layout(self.body(wide)), [])
        wide_one = (wide.replace("for\n        x", "for x").replace(",\n        carry", ", carry")
                    .replace("\n    {", " {"))
        self.assertBreaks(self.body(wide_one), 4, "over 100")
        com = "    r = for\n        x in a, // c\n        carry c = 0\n    {\n        collect x\n    }\n"
        self.assertEqual(self.layout(self.body(com)), [])
        self.assertBreaks(self.body(wide.replace(",\n        carry", "\n        carry")), 5, "separated by commas")
        self.assertBreaks(self.body(wide.replace("carry c = 0", "carry c = 0,")), 6, "no comma after the last")
        self.assertBreaks(self.body(wide.replace("        carry", "      carry")), 6, "expected 8")
        self.assertBreaks(self.body(wide.replace("carry c = 0", "carry c = 0, carry d = 1")), 6, "one clause per line")
        # the comment after the `{` does not force the split form
        self.assertEqual(self.layout(self.body(one.replace("0 {", "0 { // c"))), [])
        # loop without clause, and `for parallel`
        self.assertEqual(self.layout(self.body("    r = loop {\n        until a\n    }\n")), [])
        self.assertEqual(self.layout(self.body("    r = for parallel x in a {\n        collect x\n    }\n")), [])

    def test_layout_trailing_comments_and_widths(self):
        # the comment is not counted in the width of the line
        o = "o" * 44
        sig = f"fn f(a: i32) -> ({o}: i32) {{ // a comment that makes the line longer than 100 code points\n}}\n"
        self.assertEqual(self.layout(self.HDR + sig), [])
        # a comment after the closing brace of a struct does not force the split
        self.assertEqual(self.layout(self.HDR + "type S = struct { a: i32 } // c\n"), [])
        self.assertBreaks(self.HDR + "type S = struct {\n    a: i32,\n} // c\n", 3, "fits on one line")
        self.assertEqual(self.layout(self.HDR + "type S = struct { // c\n    a: i32,\n}\n"), [])
        self.assertBreaks(self.HDR + "fn f(\n    a: i32,\n) -> (r: i32) { // c\n}\n", 3, "fits on one line")

    def test_layout_comments_spacing(self):
        g = self.GOOD_LAYOUT
        self.assertEqual(self.layout(g.replace("// end", "//")), [])
        self.assertEqual(self.layout(g.replace("/// Doc.", "///")), [])
        # at least one space: indented text in a comment is kept
        self.assertEqual(self.layout(g.replace("// end", "//    end")), [])
        self.assertEqual(self.layout(g.replace("/// Doc.", "///  Doc.")), [])
        self.assertBreaks(g.replace("// end", "//\tend"), 9, "at least one space")

    def test_layout_use(self):
        g = self.GOOD_LAYOUT
        self.assertBreaks(g.replace("use b", "use a"), 4, "appears twice")
        self.assertEqual(self.layout(g.replace("use a\nuse b", "use Z\nuse a")), [])  # code point order
        self.assertBreaks(g.replace("use a\nuse b", "use a\nuse Z"), 3, "not sorted")
        self.assertBreaks(g.replace("use a\nuse b", "use a\n// c\nuse b"), 4, "no comment between `use` lines")
        self.assertBreaks(g.replace("use b\n\n// note", "use b\n// note"), 5, "separated by one blank line")

    def test_layout_new_spaces(self):
        b = self.body
        self.assertEqual(self.layout(b("    r = for i in 0..5 {\n        collect i\n    }\n")), [])
        self.assertBreaks(b("    r = g(0 ..5)\n"), 4, "`..`")
        self.assertBreaks(b("    r = g(0.. 5)\n"), 4, "`..`")
        self.assertBreaks(b("    r = g(1 ..= 5)\n"), 4, "`..`")
        self.assertBreaks(b("    r = a . b\n"), 4, "around `.`")
        self.assertBreaks(b("    r = a. b\n"), 4, "around `.`")
        self.assertBreaks(b("    # close: r = g()\n"), 4, "after `#`")
        self.assertEqual(self.layout(b("    #close: r = g()\n")), [])
        self.assertBreaks(self.HDR + "fn f(parts: string ...) -> (r: i32) {\n}\n", 3, "before `...`")
        self.assertEqual(self.layout(self.HDR + "fn f(parts: string...) -> (r: i32) {\n}\n"), [])
        self.assertBreaks(self.HDR + "fn f(a: [f64;2]) -> (r: i32) {\n}\n", 3, "after `;`")
        self.assertBreaks(self.HDR + "fn f(a: [f64 ; 2]) -> (r: i32) {\n}\n", 3, "before `;`")
        self.assertEqual(self.layout(self.HDR + "fn f(a: [f64; 2]) -> (r: i32) {\n}\n"), [])
        self.assertBreaks(b("    r = g (a)\n"), 4, "between `g` and `(`")
        self.assertBreaks(b("    r = g.h [0]\n"), 4, "between `g.h` and `[`")
        self.assertBreaks(self.HDR + "fn mean <T: Number>(a: T) -> (r: T) {\n}\n", 3, "between `mean` and `<`")
        self.assertBreaks(self.HDR + "fn mean<T: Number >(a: T) -> (r: T) {\n}\n", 3, "before `>`")
        self.assertBreaks(self.HDR + "fn mean< T: Number>(a: T) -> (r: T) {\n}\n", 3, "after `<`")
        self.assertEqual(self.layout(self.HDR + "fn mean<T: Number>(a: T) -> (r: T) {\n}\n"), [])
        self.assertEqual(self.layout(b("    r = yield (a, b)\n")), [])
        self.assertBreaks(b("    r = g( )\n"), 4, "after `(`")
        self.assertBreaks(self.HDR + "fn f(a: f64[ ]) -> (r: i32) {\n}\n", 3, "after `[`")
        self.assertBreaks(self.HDR + "fn f(a: f64[m / s]) -> (r: i32) {\n}\n", 3, "inside a unit")
        self.assertBreaks(b("    g = 9.81 m/ s^2\n"), 4, "inside a unit")
        self.assertEqual(self.layout(b("    g = 9.81 m/s^2\n    k = 1 kg/(m*s)\n")), [])
        self.assertBreaks(b("    g = 9.81 m/(m * s)\n"), 4, "inside a unit")

    def test_layout_braces_of_a_body(self):
        b = self.body
        self.assertEqual(self.layout(self.HDR + "fn f() -> (r: i32) {}\n"), [])
        self.assertBreaks(self.HDR + "fn f() -> (r: i32) { r = 1 }\n", 3, "function body")
        self.assertBreaks(b("    r = match N { 1 => yield 1 }\n"), 4, "never written on one line")
        self.assertBreaks(b("    r = handle e { ok => yield 1 }\n"), 4, "never written on one line")
        self.assertBreaks(b("    r = match N {\n        1 => { yield 1 }\n    }\n"), 5, "arm")
        self.assertBreaks(b("    r = match N {\n        1 => {\n            yield 1\n        }\n    }\n"), 5,
                          "only a `yield`")
        self.assertEqual(self.layout(b("    r = match N {\n        1 => yield 1\n        _ => {\n"
                                       "            x = core.add(N, 1)\n            yield x\n        }\n    }\n")), [])
        self.assertBreaks(b("    r = for x in a { collect x }\n"), 4, "ends with `{`")
        self.assertBreaks(b("    r = match N {\n        1 => yield 1 }\n"), 5, "`}` stands alone")

    def test_layout_partial_splits(self):
        b = self.body
        self.assertBreaks(self.HDR + "type S = struct { a: i32,\n    b: i32 }\n", 3, "ends its line")
        self.assertBreaks(self.HDR + "type S = struct { a: i32,\n    b: i32 }\n", 4, "`}` stands alone")
        self.assertBreaks(b("    r = loop carry n = 0,\n        carry m = 1 {\n        until n\n    }\n"), 4, "`loop`")
        self.assertBreaks(b("    r = S { x: 1,\n        y: 2 }\n"), 4, "ends its line")
        self.assertBreaks(b("    r = s {\n        x: 1,\n    }\n"), 4, "struct construction")  # lowercase type

    def test_layout_false_positives(self):
        b = self.body
        self.assertEqual(self.layout(b("    r = handle e {\n        visa.* => yield (1, none)\n"
                                       "        _ => yield (2, none)\n    }\n")), [])
        self.assertEqual(self.layout(b("    r = loop carry n = N {\n        until n\n    }\n")), [])

    def test_layout_loop_keywords_may_follow(self):
        b = self.body
        wide = "x" * 80
        for kw, head in (("parallel", "for"), ("continue_on_error", "loop")):
            src = b(f"    r = {head} {kw}\n        carry c{wide} = 0,\n        carry d = 1\n    {{\n"
                    "        until c\n    }\n")
            self.assertEqual([m for n, m in self.layout(src) if "expected" in m or "alone" in m], [], kw)

    def test_layout_spaces_before_brackets(self):
        b = self.body
        self.assertBreaks(b("    r = yield(a, b)\n"), 4, "`yield` and `(`")
        self.assertEqual(self.layout(b("    r = core.add(a, b)\n    s = io.timeout(err)\n")), [])
        self.assertBreaks(b("    r = io.timeout (err)\n"), 4, "no space between")
        h = b("    r = handle e {\n        code -1073807339 (err) => yield 1\n        _ => yield 2\n    }\n")
        self.assertEqual(self.layout(h), [])
        bad = self.layout(h.replace("339 (err)", "339(err)"))
        self.assertTrue(any("and `(`" in m for n, m in bad if n == 5), bad)
        sect = self.HDR + "fn f() -> () {}\n\n"
        self.assertEqual(self.layout(sect + "layout {\n    node a at (40, 80)\n    wire a -> b via (1, 2) (3, 4)\n}\n"
                                     "\nicon 32 {\n    rect (2, 4) (30, 28) radius 4\n}\n"), [])
        self.assertBreaks(sect + "layout {\n    node a at(40, 80)\n}\n", 6, "layout")
        self.assertBreaks(sect + "layout {\n    wire a -> b via (1, 2)(3, 4)\n}\n", 6, "layout")
        self.assertBreaks(sect + "icon 32 {\n    rect(2, 4) (30, 28)\n}\n", 6, "layout")

    def test_layout_signature_outputs_only_with_split_parameters(self):
        src = self.HDR + "fn f(a: i32) -> (\n    x: i32, // c\n) {\n}\n"
        self.assertBreaks(src, 3, "only when the parameters are")

    def test_layout_is_linear(self):
        import time
        src = self.HDR + "fn f() -> (r: i32) {\n" + "    // c\n" * 20000 + "    r = 1\n}\n"
        t = time.time()
        self.assertEqual(self.layout(src), [])
        self.assertLess(time.time() - t, 2.0)
        t = time.time()
        self.layout(self.HDR + "fn f() -> (r: i32) {\n" + "    r = loop\n" * 20000 + "}\n")
        self.assertLess(time.time() - t, 2.0)
        t = time.time()
        self.layout("labcraft 1\n\n" + "use a\n// c\n" * 5000)
        self.assertLess(time.time() - t, 2.0)

    def test_fmt_check_size_limit_and_empty_message(self):
        import contextlib
        import io
        from unittest import mock
        import lcf
        from lcfpoc import library
        from lark.exceptions import LarkError
        with tempfile.TemporaryDirectory() as td:
            (Path(td) / "big.lcf").write_text("labcraft 1\n\n" + "// c\n" * 10, encoding="utf-8", newline="\n")
            buf = io.StringIO()
            with mock.patch.object(library, "MAX_FILE", 20), contextlib.redirect_stdout(buf):
                self.assertEqual(lcf.fmt_check([td]), 1)
            self.assertIn("file too large", buf.getvalue())
        with tempfile.TemporaryDirectory() as td:
            (Path(td) / "a.lcf").write_text("labcraft 1\n", encoding="utf-8", newline="\n")
            buf = io.StringIO()
            with mock.patch("lcfpoc.ir.lower_file", side_effect=LarkError("")), contextlib.redirect_stdout(buf):
                self.assertEqual(lcf.fmt_check([td]), 1)
            self.assertIn("syntax error", buf.getvalue())
            self.assertNotIn("Traceback", buf.getvalue())

    def test_extern_without_err_is_rejected(self):
        with tempfile.TemporaryDirectory() as td:
            lib_dir(td, {"f.lcf": 'labcraft 1\n\nextern rust(crate: "a", function: "f")\nfn f() -> (x: i32)\n'})
            self.assertTrue(any("R-13" in d["message"] for d in api.check(td)["diagnostics"]))

    def test_shipped_libraries_match_their_native_bodies(self):
        from lcfpoc.library import load_library
        from lcfpoc import stdlib
        for folder, names in ((ROOT / "libs" / "visa", {"open", "close", "write", "read", "query"}),
                              (ROOT / "examples" / "results_log", {"append_csv"})):
            lib = load_library(folder)
            self.assertEqual(lib.errors, [])
            self.assertEqual(set(lib.functions), names)
            for fn in lib.functions.values():
                b = stdlib.NATIVE[fn.extern[1]["crate"], fn.extern[1]["function"]]
                self.assertEqual([p.name for p in b.params], [p.name for p in fn.params], fn.name)
                self.assertEqual(list(b.outs), [o[0] for o in fn.outputs], fn.name)
        # the manifest marks `always`, not the native body
        visa = load_library(ROOT / "libs" / "visa")
        self.assertTrue(visa.callee(visa.functions["close"])[1].always)
        self.assertFalse(visa.callee(visa.functions["write"])[1].always)

    def test_shipped_library_ignores_a_folder_of_the_project(self):
        from lcfpoc.library import load_library
        with tempfile.TemporaryDirectory() as td:
            lib_dir(td, {"a.lcf": "labcraft 1\n\nuse scpi\nuse visa\n\nfn a() -> () {}\n"})
            (Path(td) / "visa").mkdir()
            lib_dir(Path(td) / "visa", {"fake.lcf": "labcraft 1\n\nfn fake() -> () {}\n"})
            lib = load_library(td)
            self.assertIn("fake", lib.deps["visa"].functions)  # the project replaces it for itself
            self.assertIn("query", lib.deps["scpi"].deps["visa"].functions)  # not for `scpi`

    def test_library_cycle(self):
        with tempfile.TemporaryDirectory() as td:
            for name, other in (("a", "b"), ("b", "a")):
                (Path(td) / name).mkdir()
                lib_dir(Path(td) / name, {"f.lcf": f"labcraft 1\n\nuse {other}\n\nfn f() -> () {{}}\n"})
            msgs = [d["message"] for d in api.check(td)["diagnostics"]]
            self.assertEqual(len([m for m in msgs if "use each other" in m]), 1, msgs)

    def test_type_files_are_checked(self):
        with tempfile.TemporaryDirectory() as td:
            lib_dir(td, {"a.lct": "labcraft 1\n\ntype a = nonexist.session\n",
                         "b.lct": "labcraft 1\n\ntype b = struct { x: opaque, y: nonsense }\n",
                         "c.lct": "labcraft 1\n\ntype c = [opaque]\n",
                         "d.lct": "labcraft 1\n\ntype d = opaque[V]\n",
                         "e.lct": "labcraft 1\n\ntype e = visa.session\n",
                         "f.lct": "labcraft 1\n\ntype f = opaque\n"})
            rep = api.check(td)
            files = {d["file"] for d in rep["diagnostics"] if d["severity"] == "error"}
            self.assertEqual(files, {"a.lct", "b.lct", "c.lct", "d.lct", "e.lct"}, rep["diagnostics"])

    def test_struct_literal_needs_use(self):
        with tempfile.TemporaryDirectory() as td:
            (Path(td) / "lib").mkdir()
            lib_dir(Path(td) / "lib", {"r.lct": "labcraft 1\n\ntype R = struct { a: i32 }\n"})
            lib_dir(td, {"a.lcf": "labcraft 1\n\nuse lib\n\nfn a() -> () {}\n",
                         "b.lcf": "labcraft 1\n\nfn b() -> (r: i32) {\n    x = lib.R { a: 1 }\n    r = x.a\n}\n"})
            self.assertTrue(any("'lib' is not imported" in d["message"] for d in api.check(td)["diagnostics"]))

    def test_declaration_is_the_contract(self):
        ext = 'labcraft 1\n\nextern rust(crate: "labcraft-visa", function: "%s")\n'
        with tempfile.TemporaryDirectory() as td:
            lib_dir(td, {"myclose.lcf": ext % "close" + "fn myclose(hnd: string) -> (err: error)\n",
                         "later.lcf": ext % "none_yet" + "fn later() -> (v: f64[V], err: error)\n",
                         "t.lcf": "labcraft 1\n\nuse core\n\nfn t() -> (w: f64[V], err: error) {\n    v, e = later()\n"
                                  "    w = core.add(v, 1 V)\n    err = e\n}\n"})
            msgs = [d["message"] for d in api.check(td)["diagnostics"]]
            self.assertTrue(any("differs from its native body" in m for m in msgs), msgs)
            (Path(td) / "myclose.lcf").unlink()
            r = api.run(td, "t", None, {})  # a typed default value, not a fault (7.2)
            self.assertIsNone(r["fault"], r)
            self.assertEqual(r["error"]["category"], "extern.unavailable")
            self.assertEqual(r["outputs"]["w"]["value"], 1.0)

    def test_skipped_generic_output_reaches_a_driver_as_an_error(self):
        with tempfile.TemporaryDirectory() as td:
            shutil.copytree(Path(EX) / "psu", Path(td) / "psu")
            lib_dir(td, {"t.lcf": """labcraft 1

use array
use psu
use visa

fn t(values: [f64[V]]) -> (err: error) {
    s, e0 = visa.open("SIM::PSU0")
    v, e1 = array.get(values, 5, err: e0)
    _, e2 = psu.set_voltage(s, v, err: e1)
    #close: e3 = visa.close(s, err: e2)
    err = e3
}
"""})
            r = run("t", "psu-1W5.toml", {"values": "[]"}, folder=td)
            self.assertIsNone(r["fault"], r)
            self.assertEqual(r["error"]["category"], "array.index_out_of_range")

    def test_bad_manifest_is_reported(self):
        from unittest import mock
        from lcfpoc import library
        with tempfile.TemporaryDirectory() as td:
            for text in ("always = 5\n", "always = [[]]\n", 'always = "close"\n', "not toml"):
                lib_dir(td, {"labcraft.toml": text, "f.lcf": "labcraft 1\n\nfn f() -> () {}\n"})
                with mock.patch.object(library, "SHIPPED", Path(td).resolve()):
                    lib = library.load_library(td)
                self.assertTrue(any("labcraft.toml" in d["file"] for d in lib.errors), text)

    def test_list_shows_the_libraries_a_caller_can_use(self):
        names = {b["name"]: b for b in api.list_functions(EX, True)["builtins"]}
        for q in ("relays.apply", "psu.set_voltage", "dmm_driver.read_dc_volts", "scpi.query_f64", "core.add"):
            self.assertIn(q, names)
        self.assertEqual(names["visa.close"]["signature"],
                         "always fn visa.close(session: visa.session, err: error = …) -> (err: error)")

    def test_number_conversions(self):
        with tempfile.TemporaryDirectory() as td:
            lib_dir(td, {"f.lcf": """labcraft 1

use string

fn f(t: string) -> (a: f64, b: i32, e1: error, e2: error) {
    a, e1 = string.to_f64(t)
    b, e2 = string.to_i32(t)
}
"""})
            def errs(text):
                o = api.run(td, "f", None, {"t": text})["outputs"]
                return (o["e1"] is not None, o["e2"] is not None)
            self.assertEqual(errs(" +12 "), (False, False))
            self.assertEqual(errs("+2.3E+02"), (False, True))
            self.assertEqual(errs("NaN"), (False, True))
            for bad in ("1_000", "Infinity", "", "x"):
                self.assertEqual(errs(bad), (True, True), bad)
            self.assertEqual(errs("99999999999"), (False, True))

    def test_library_not_imported_or_unknown(self):
        with tempfile.TemporaryDirectory() as td:
            lib_dir(td, {"f.lcf": "labcraft 1\n\nfn f(s: visa.session) -> (err: error) {\n"
                                  "    e = visa.close(s)\n    err = e\n}\n",
                         "g.lcf": "labcraft 1\n\nuse nothing\n\nfn g() -> () {}\n"})
            msgs = [d["message"] for d in api.check(td)["diagnostics"]]
            self.assertTrue(any("'visa' is not imported" in m and "visa.close" in m for m in msgs), msgs)
            self.assertTrue(any("type 'visa.session'" in m for m in msgs), msgs)
            self.assertTrue(any("unknown library 'nothing'" in m and "scpi" in m for m in msgs), msgs)

    def test_opaque_name_is_free_for_values_but_not_a_struct(self):
        with tempfile.TemporaryDirectory() as td:
            lib_dir(td, {"bank.lct": "labcraft 1\n\nuse visa\n\ntype bank = visa.session\n",
                         "s.lct": "labcraft 1\n\ntype s = struct { a: i32 }\n",
                         "f.lcf": "labcraft 1\n\nfn f(bank: bank, err: error = none) -> (bank: bank, err: error) {\n"
                                  "    bank = bank\n    err = err\n}\n",
                         "g.lcf": "labcraft 1\n\nfn g(s: i32) -> (x: i32) {\n    x = s\n}\n"})
            msgs = [d["message"] for d in api.check(td)["diagnostics"] if d["severity"] == "error"]
            self.assertEqual(len(msgs), 1, msgs)
            self.assertIn("'s' is the name of a library or a type", msgs[0])

    def test_string_primitives(self):
        with tempfile.TemporaryDirectory() as td:
            lib_dir(td, {"f.lcf": """labcraft 1

use array
use string

fn f(v: f64[mV], t: string) -> (a: string, b: string, n: f64, e: error) {
    a = string.format("{{{v}}} {v:.2 V} {x:.1}", v: v, x: 2.25)
    parts = string.split(t, ",")
    p0, e1 = array.get(parts, 0)
    b, e2 = array.get(parts, 1, err: e1)
    n, e3 = string.to_f64(p0, err: e2)
    e = e3
}
"""})
            r = api.run(td, "f", None, {"v": "230 mV", "t": "1.5,x"})
            self.assertTrue(r["ok"], r)
            self.assertEqual(r["outputs"]["a"], "{230 mV} 0.23 V 2.2")
            self.assertEqual((r["outputs"]["b"], r["outputs"]["n"]), ("x", 1.5))
            r = api.run(td, "f", None, {"v": "1 mV", "t": "no,x"})
            self.assertEqual(r["error"]["category"], "string.parse")
            lib_dir(td, {"f.lcf": """labcraft 1

use string

fn f(t: string) -> (a: string, b: string, c: string) {
    a = string.format(t)
    b = string.format("{x}")
    c = string.format("x", y: 1)
}
"""})
            msgs = [d["message"] for d in api.check(td)["diagnostics"] if "R-12" in d["message"]]
            self.assertEqual(len(msgs), 3, msgs)

    def test_extern_is_reported_cleanly(self):
        import contextlib
        import io
        import lcf
        spec = (ROOT / "docs" / "language-spec-v0.md").read_text(encoding="utf-8")
        block = spec[spec.index("labcraft 1\n\n/// Amplitude spectrum"):]
        block = block[:block.index("```")]
        with tempfile.TemporaryDirectory() as td:
            (Path(td) / "spectrum.lcf").write_text(block, encoding="utf-8", newline="\n")
            rep = api.check(td)
            self.assertTrue(rep["ok"], rep["diagnostics"])  # read and checked as a signature (4.1)
            lib_dir(td, {"caller.lcf": "labcraft 1\n\nfn caller(a: [f64[V]], r: f64[Hz]) -> (err: error) {\n"
                                       "    _, e = spectrum(a, r)\n    err = e\n}\n"})
            r = api.run(td, "caller", None, {"a": "[]", "r": "1 Hz"})
            self.assertEqual(r["error"]["category"], "extern.unavailable")
            (Path(td) / "caller.lcf").unlink()
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                self.assertEqual(lcf.fmt_check([td]), 0)  # its layout is still checked
            self.assertNotIn("StopIteration", buf.getvalue())

    def test_layout_cr(self):
        g = self.GOOD_LAYOUT
        got = self.layout(g.replace("z = core.add", "z =\r core.add"))
        self.assertTrue(any("not followed by a line feed" in m for n, m in got), got)
        self.assertFalse(any("CR LF" in m or "consecutive" in m for n, m in got), got)
        self.assertEqual([n for n, m in got if "CR" in m], [9])

    def test_fmt_check_hostile_files_and_paths(self):
        import contextlib
        import io
        import lcf
        with tempfile.TemporaryDirectory() as td:
            a, b = Path(td) / "a", Path(td) / "b"
            a.mkdir()
            b.mkdir()
            good = "labcraft 1\n\ntype Verdict = enum { Pass, Fail }\n"
            (a / "verdict.lct").write_text(good, encoding="utf-8", newline="\n")
            (b / "verdict.lct").write_text(good + "\n", encoding="utf-8", newline="\n")
            (b / "binary.lcf").write_bytes(b"labcraft 1\n\xff\xfe\x00\n")
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                code = lcf.fmt_check([str(a), str(b), str(Path(td) / "missing.lcf")])
            out = buf.getvalue()
            self.assertEqual(code, 1)
            self.assertNotIn("Traceback", out)
            self.assertIn(f"{a.as_posix()}/verdict.lct", out)
            self.assertIn(f"{b.as_posix()}/verdict.lct:4:", out)
            self.assertIn(f"{b.as_posix()}/binary.lcf:2: not UTF-8", out)
            self.assertIn("missing.lcf: no such file", out)
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                self.assertEqual(lcf.fmt_check([str(a)]), 0)

    def test_layout_spec_blocks(self):
        import contextlib
        import io
        import lcf
        from lcfpoc.layout import check_spec_layout
        problems, done, skipped = check_spec_layout(ROOT / "docs" / "language-spec-v0.md")
        self.assertEqual(problems, [])
        self.assertGreaterEqual(done, 15)
        self.assertGreaterEqual(skipped, 2)
        with tempfile.TemporaryDirectory() as td:
            md = Path(td) / "s.md"
            md.write_text(
                "# T\n\n```text\nlabcraft 1\n\nfn f() -> (r: i32) {\n    r = core.add(a,b)\n}\n```\n\n"
                "```text\nr = core.add(a, b) // c\nq = mul (r, 2)\n```\n\n"
                "```text\nfn g(x: i32) -> (r: i32)\n```\n\n"
                "```text\n[f64]   // reference listing: skipped\n```\n\n"
                "```text\nr = core.add(a,b) …\n```\n\n"
                "## Appendix A\n\n```text\nfn g(x:i32)\n```\n", encoding="utf-8", newline="\n")
            problems, done, skipped = check_spec_layout(md)
            self.assertEqual([n for n, m in problems], [7, 13], problems)
            self.assertEqual((done, skipped), (3, 2))
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                self.assertEqual(lcf.fmt_check([], str(md)), 1)
            self.assertIn(f"{md.as_posix()}:7:", buf.getvalue())


class Hostile(unittest.TestCase):
    """Criterion 3: never a crash, always an error reported with file and line."""

    GOOD = """labcraft 1

use core

fn ok(a: i32) -> (b: i32) {
    b = core.add(a, 1)
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
        d = self.assert_reported({"t.lcf": self.GOOD.replace("core.add(a, 1)", "core.add(aa, 1)")}, "unknown value")
        self.assertEqual(d["line"], 6)
        self.assertIn("'a'", d["message"])

    def test_unknown_function(self):
        self.assert_reported({"t.lcf": self.GOOD.replace("core.add(", "addd(")}, "unknown function")

    def test_name_of_core_without_the_prefix(self):
        d = self.assert_reported({"t.lcf": self.GOOD.replace("core.add(", "add(")}, "unknown function")
        self.assertIn("did you mean 'core.add'", d["message"])

    def test_core_without_use(self):
        self.assert_reported({"t.lcf": self.GOOD.replace("use core\n\n", "")}, "'core.add': library 'core' is not imported")

    def test_a_value_named_like_a_library_is_refused_only_if_imported(self):
        src = "labcraft 1\n\n%sfn f(core: i32) -> (x: i32) {\n    x = core\n}\n"
        with tempfile.TemporaryDirectory() as td:
            lib_dir(td, {"f.lcf": src % ""})
            self.assertTrue(api.check(td)["ok"], api.check(td)["diagnostics"])
        self.assert_reported({"f.lcf": src % "use core\n\n"}, "name of a library")

    def test_unbound_output(self):
        self.assert_reported({"t.lcf": self.GOOD.replace("    b = core.add(a, 1)\n", "")}, "is not bound")

    def test_two_definitions(self):
        self.assert_reported({"ok.lcf": self.GOOD, "ok2.lcf": self.GOOD}, "defined twice")

    def test_value_defined_twice(self):
        self.assert_reported({"t.lcf": self.GOOD.replace("    b = core.add(a, 1)", "    b = core.add(a, 1)\n    b = core.add(a, 2)")},
                             "defined twice")

    def test_cycle(self):
        self.assert_reported({"t.lcf": self.GOOD.replace("    b = core.add(a, 1)", "    x = core.add(y, 1)\n    y = core.add(x, 1)\n    b = x")},
                             "cycle")

    def test_missing_input(self):
        self.assert_reported({"t.lcf": self.GOOD.replace("core.add(a, 1)", "core.add(a)")}, "required input")

    def test_handle_without_wildcard(self):
        src = "labcraft 1\n\nfn ok(a: i32, err: error = none) -> (b: i32) {\n    b = handle err {\n        ok => yield a\n    }\n}\n"
        self.assert_reported({"t.lcf": src}, "handle without a `_` arm")

    def test_unit_unknown(self):
        self.assert_reported({"t.lcf": self.GOOD.replace("core.add(a, 1)", "core.add(a, 1 qux)")}, "unknown unit")

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
