"""The corpora of tests/acceptance are sound: they are specifications for M0a, not run against Rust yet.

Each file is what it says it is, the expected reports use only what docs/cli.md defines, and the proof
of concept does not crash on any of them.
"""
import contextlib
import io
import json
import re
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "tools" / "lcf-poc"))
import lcf  # noqa: E402
from lcfpoc import api  # noqa: E402
from lcfpoc.layout import check_layout  # noqa: E402
from lcfpoc.library import parse  # noqa: E402

CORPUS = ROOT / "tests" / "acceptance"
CLI = (ROOT / "docs" / "cli.md").read_text(encoding="utf-8")
CODES = dict(re.findall(r"^\| `(LC\d{4})` \| (error|warning) \|", CLI, re.M))
S2 = set(re.findall(r"^\| `(LC\d{4})` \|.*\| S2 \|$", CLI, re.M))


def cases(family):
    return sorted(p for p in (CORPUS / family).iterdir() if p.is_dir())


class TypeErrors(unittest.TestCase):
    def test_one_case_per_code_of_scenario_2(self):
        codes = [json.loads((c / "expected.json").read_text(encoding="utf-8"))["diagnostics"][0]["code"]
                 for c in cases("type-errors")]
        self.assertEqual(sorted(codes), sorted(S2))
        self.assertEqual(len(S2), 9)

    def test_programs_parse_and_are_canonical(self):
        for case in cases("type-errors"):
            for f in [*case.glob("*.lcf"), *case.glob("*.lct")]:
                src = f.read_text(encoding="utf-8")
                parse(src)
                self.assertEqual(check_layout(src), [], f)

    def test_expected_reports(self):
        for case in cases("type-errors"):
            rep = json.loads((case / "expected.json").read_text(encoding="utf-8"))
            self.assertEqual((rep["schema"], rep["status"]), ("labcraft-check/1", "error"), case)
            self.assertEqual(rep["summary"], {"errors": 1, "warnings": 0}, case)
            self.assertEqual(len(rep["diagnostics"]), 1, case)
            d = rep["diagnostics"][0]
            self.assertEqual(CODES.get(d["code"]), d["severity"], case)
            lines = (case / d["file"]).read_text(encoding="utf-8").split("\n")
            for s in [d["span"], *[r["span"] for r in d.get("related", [])]]:
                self.assertEqual(s["start"]["line"], s["end"]["line"], case)
                text = lines[s["start"]["line"] - 1][s["start"]["column"] - 1:s["end"]["column"] - 1]
                self.assertTrue(text.strip(), case)
                self.assertEqual(text, text.strip(), case)
            self.assertTrue(any(d["node"]["text"] == line.strip() for line in lines), case)
            for w in d.get("wires", []):
                self.assertTrue(set(w) <= {"value", "from", "to"}, case)
                for t in (w.get("from"), w.get("to")):
                    if t:  # section 4.3 of docs/cli.md
                        self.assertRegex(t, r"^(#?\w+|@\d+|fn|next|collect|yield)(\.\w+)?$|^until$", case)

    def test_the_span_points_at_the_fault(self):
        want = {"wrong-input-type": "v", "incompatible-dimensions": "current",
                "output-of-the-wrong-dimension": "power", "implicit-narrowing": "x",
                "unitless-of-another-dimension": "[A]", "arms-that-disagree": 'yield "none"',
                "carry-whose-type-changes": "next n", "function-reference-of-the-wrong-signature": "scale",
                "variant-of-another-enumeration": "State.Init"}
        self.assertEqual(set(want), {c.name for c in cases("type-errors")})
        for name, text in want.items():
            case = CORPUS / "type-errors" / name
            d = json.loads((case / "expected.json").read_text(encoding="utf-8"))["diagnostics"][0]
            line = (case / d["file"]).read_text(encoding="utf-8").split("\n")[d["span"]["start"]["line"] - 1]
            self.assertEqual(line[d["span"]["start"]["column"] - 1:d["span"]["end"]["column"] - 1], text, name)


class Hostile(unittest.TestCase):
    FMT_CODES = {f"LC00{n:02}" for n in range(1, 11)} | {"LC0099", "LC0101", "LC0102", "LC1001", "LC1002",
                                                         "LC1003", "LC1004", "LC1005", "LC5004", "LC6001",
                                                         "LC6002", "LC6003"}  # section 2 of docs/cli.md

    def test_expected_reports_use_what_the_cli_defines(self):
        for case in cases("hostile"):
            rep = json.loads((case / "expected.json").read_text(encoding="utf-8"))
            for command in ("check", "fmt"):
                r = rep[command]
                errors = [c for c in r["codes"] if CODES.get(c) == "error"]
                for c in r["codes"]:
                    self.assertIn(c, CODES, case)
                # exit 1 exactly when an error is reported (section 1 of docs/cli.md)
                self.assertEqual(r["exit"], 1 if errors else 0, case)
            self.assertTrue(set(rep["fmt"]["codes"]) <= self.FMT_CODES, case)
            self.assertIn("changes", rep["fmt"], case)

    def test_bytes_are_stored_as_they_are(self):
        self.assertIn(b"\r\n", (CORPUS / "hostile" / "encoding-crlf" / "f.lcf").read_bytes())
        self.assertTrue((CORPUS / "hostile" / "encoding-bom" / "f.lcf").read_bytes().startswith(b"\xef\xbb\xbf"))
        self.assertEqual((CORPUS / "hostile" / "encoding-binary" / "f.lcf").read_bytes()[:2], b"\x00\xff")

    def test_the_proof_of_concept_does_not_crash(self):
        for case in cases("hostile"):
            with tempfile.TemporaryDirectory() as td:
                copy = Path(td) / "case"
                shutil.copytree(case, copy)
                (copy / "expected.json").unlink()
                before = {p.name: p.read_bytes() for p in copy.iterdir()}
                rep = api.check(str(copy))
                self.assertIn("diagnostics", rep, case)
                with contextlib.redirect_stdout(io.StringIO()):
                    flagged = lcf.fmt_check([str(copy)])
                self.assertIn(flagged, (0, 1), case)
                expected = json.loads((case / "expected.json").read_text(encoding="utf-8"))["fmt"]
                if expected["exit"] == 0:  # the file parses: the PoC sees the same changes as `fmt` makes
                    self.assertEqual(bool(flagged), expected["changes"], case)
                self.assertEqual({p.name: p.read_bytes() for p in copy.iterdir()}, before, case)


if __name__ == "__main__":
    unittest.main()
