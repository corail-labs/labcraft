"""The corpus of runs (tests/acceptance/runs/, docs/architecture.md, 10.11), run by the proof of concept.

Each case gives the parameters of `run.start` (`run.json`) and the parts of `labcraft-run/1` it asserts
(`expected.json`). The proof of concept has a report of its own: this runner turns it into those parts, as far
as it gives them, and compares. It gives no types, no `check` and no count of the resources opened, so those
are not compared here; the Rust runner of `tests/harness` compares everything.

The cases that `tests/acceptance/runs/poc-later.txt` names are rules the proof of concept does not follow: they
run all the same, and must give a report, never an exception.
"""

import json
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))

from lcfpoc import api  # noqa: E402

ROOT = HERE.parents[2]
CORPUS = ROOT / "tests" / "acceptance" / "runs"
SHIPPED = "<labcraft>/"
# `real_ms` depends on the machine, and the proof of concept does not measure it as the Rust runtime does.
NOT_COMPARED = {"how", "check", "type", "opened", "file", "span", "trace", "real_ms", "warning"}


def later():
    """The cases that the proof of concept does not follow, each with why."""
    names = {}
    for line in (CORPUS / "poc-later.txt").read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if line and not line.startswith("#"):
            name, _, why = line.partition(" ")
            names[name] = why.strip()
    return names


def as_text(value):
    """A value of `run.json` as the command line of the proof of concept takes it."""
    if isinstance(value, bool):
        return "true" if value else "false"
    return str(value)


def run_case(case):
    params = json.loads((case / "run.json").read_text(encoding="utf-8"))
    # Paths are those of `run.start`, from the root of the project, the repository (commands.md 2).
    folder = (ROOT / params.get("folder", ".")).resolve()
    sim = None
    if "sim" in params:
        given = params["sim"]
        sim = str((ROOT / "libs" / given[len(SHIPPED):]) if given.startswith(SHIPPED) else (ROOT / given).resolve())
    args = {name: as_text(value) for name, value in params.get("args", {}).items()}
    settings = [{"element": s["element"], "value": as_text(s["value"]), "at": s["at"]} for s in params.get("set", [])]
    limits = {}
    if "max_iterations" in params:
        limits["max_iter"] = params["max_iterations"]
    if "schedule_seed" in params:
        limits["schedule_seed"] = params["schedule_seed"]
    if "timeout" in params:
        limits["timeout"] = params["timeout"]
    if params.get("mode") == "real_time":
        limits["real_time"] = True
    return api.run(str(folder), params["function"], sim, args, False, settings, **limits)


def first_name(node):
    """The proof of concept names a line by its text (`n_a, sum_a, err_a`, `err = time.wait(…)`); the report by its first
    output."""
    return node.split(" = ")[0].split(",")[0].strip() if isinstance(node, str) else node


def value_of(value):
    """A number with a unit, which the proof of concept writes as an object, as section 5 of commands.md writes it."""
    if isinstance(value, dict) and set(value) == {"value", "unit"}:
        return f"{value['value']!r} {value['unit']}"
    return value


def translated(poc, expected):
    """The parts of `labcraft-run/1` that the report of the proof of concept gives."""
    out = {"status": poc["status"], "mode": poc.get("mode")}
    names = [output["name"] for output in expected.get("outputs", [])]
    out["outputs"] = [{"name": name, "value": value_of(poc["outputs"][name])} for name in names if name in poc["outputs"]]
    if poc.get("error") is not None:
        out["error"] = poc["error"]
    # A refused run of the proof of concept says why in `fault`; the report says it in `check`, which is not compared.
    if poc.get("fault") and poc["status"] != "refused":
        fault = dict(poc["fault"])
        fault["node"] = first_name(fault.get("node"))
        fault.setdefault("loops", [])
        out["fault"] = fault
    if poc.get("abort"):
        abort = dict(poc["abort"])
        abort["loops"] = [dict(at, node=first_name(at["node"])) for at in abort.get("loops", [])]
        out["abort"] = abort
    sim = poc.get("sim", {})
    out["clock"] = {"simulated_ms": sim.get("time_ms", 0.0)}
    out["resources"] = {"open": sim.get("open_sessions", []), "leaked": sim.get("leaked", [])}
    by_resource = {}
    for command in sim.get("commands", []):
        by_resource.setdefault(command["resource"], []).append(
            {"t_ms": command["t_ms"], "sent": command["cmd"],
             "answer": None if command.get("response") == "<no response>" else command.get("response")})
    if by_resource:
        out["instruments"] = [{"resource": name, "commands": by_resource[name]} for name in sorted(by_resource)]
    if poc.get("panel"):
        out["panel"] = poc["panel"]
    return out


def differences(expected, actual, at=""):
    """What `expected` gives that `actual` does not hold, as the Rust runner compares, without what is not compared."""
    found = []
    if isinstance(expected, dict) and isinstance(actual, dict):
        for key, value in expected.items():
            if key in NOT_COMPARED:
                continue
            if key not in actual:
                found.append(f"{at}.{key}: missing")
            else:
                found += differences(value, actual[key], f"{at}.{key}")
    elif isinstance(expected, list) and isinstance(actual, list):
        if len(expected) != len(actual):
            found.append(f"{at}: {len(actual)} elements, expected {len(expected)}")
        else:
            for index, (value, held) in enumerate(zip(expected, actual)):
                found += differences(value, held, f"{at}[{index}]")
    elif isinstance(expected, (int, float)) and isinstance(actual, (int, float)) and not isinstance(expected, bool):
        if float(expected) != float(actual):
            found.append(f"{at}: {actual!r}, expected {expected!r}")
    elif expected != actual:
        found.append(f"{at}: {actual!r}, expected {expected!r}")
    return found


class TestCorpusOfRuns(unittest.TestCase):
    def test_the_proof_of_concept_gives_the_expected_reports(self):
        skipped = later()
        cases = sorted(path for path in CORPUS.iterdir() if (path / "run.json").is_file())
        self.assertGreaterEqual(len(cases), 18)
        for name in skipped:
            self.assertTrue((CORPUS / name / "run.json").is_file(), f"poc-later.txt names {name}, which is no case")
        failed = []
        for case in cases:
            expected = json.loads((case / "expected.json").read_text(encoding="utf-8"))
            poc = run_case(case)
            self.assertIn(poc.get("status"), {"ok", "error", "aborted", "fault", "refused"}, case.name)
            if case.name in skipped:
                continue
            actual = translated(poc, expected)
            found = differences(expected, actual)
            # A field that says how the run ended is asserted absent when the case does not give it.
            found += [f".{key}: {actual[key]!r}, expected none" for key in ("error", "fault", "abort")
                      if key not in expected and key in actual]
            if found:
                failed.append(f"{case.name}: " + "; ".join(found))
        self.assertEqual(failed, [], "\n".join(failed))


if __name__ == "__main__":
    unittest.main()
