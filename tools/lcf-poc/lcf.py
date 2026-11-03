#!/usr/bin/env python3
"""LabCraft PoC: check and run .lcf functions against simulated instruments (throwaway).

    python tools/lcf-poc/lcf.py check examples
    python tools/lcf-poc/lcf.py fmt --check examples
    python tools/lcf-poc/lcf.py list examples
    python tools/lcf-poc/lcf.py run examples check_voltage --sim instruments/dmm-230V.toml \\
        --arg resource=SIM::DMM0 [--trace]
"""
import argparse
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from lcfpoc import api  # noqa: E402


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("check", help="check a library (folder)")
    c.add_argument("folder")
    c.add_argument("--json", action="store_true", help="JSON report")
    f = sub.add_parser("fmt", help="canonical order of the lines (spec 5.2); the PoC only checks it")
    f.add_argument("paths", nargs="+", help="files (.lcf) or folders")
    f.add_argument("--check", action="store_true", required=True, help="report the bodies that are not canonical")
    s = sub.add_parser("list", help="functions and types of a library")
    s.add_argument("folder")
    s.add_argument("--builtins", action="store_true", help="include the standard library")
    r = sub.add_parser("run", help="run a function against a simulated instrument")
    r.add_argument("folder")
    r.add_argument("function")
    r.add_argument("--sim", help="simulated instruments file (.toml)")
    r.add_argument("--arg", action="append", default=[], metavar="name=value")
    r.add_argument("--trace", action="store_true", help="execution order and wire values")
    r.add_argument("--max-iterations", type=int, default=10_000, help="iteration limit per loop")
    a = ap.parse_args(argv)
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    if a.cmd == "check":
        rep = api.check(a.folder)
        if a.json:
            print(json.dumps(rep, ensure_ascii=False, indent=2))
        else:
            from lcfpoc.library import Diag
            for d in rep["diagnostics"]:
                print(Diag(d))
            print(f"{a.folder}: {len(rep['functions'])} function(s), {len(rep['types'])} type(s), "
                  f"{rep['errors']} error(s), {rep['warnings']} warning(s)")
        return 0 if rep["ok"] else 1
    if a.cmd == "fmt":
        return fmt_check(a.paths)
    if a.cmd == "list":
        print(json.dumps(api.list_functions(a.folder, a.builtins), ensure_ascii=False, indent=2))
        return 0
    args = {}
    for item in a.arg:
        name, eq, value = item.partition("=")
        if not eq:
            print(f"--arg expects name=value, got '{item}'", file=sys.stderr)
            return 2
        args[name.strip()] = value
    rep = api.run(a.folder, a.function, a.sim, args, a.trace, max_iter=a.max_iterations)
    print(json.dumps(rep, ensure_ascii=False, indent=2))
    return 0 if rep["ok"] else 1


def fmt_check(paths):
    from lcfpoc.canon import check_fn
    from lcfpoc.ir import LowerError, lower_file
    from lcfpoc.library import parse
    from lark.exceptions import LarkError
    files, bad = [], 0
    for p in map(Path, paths):
        files += sorted(p.glob("*.lcf")) if p.is_dir() else [p]
    for f in files:
        src = f.read_text(encoding="utf-8")
        try:
            fn, _, _, _ = lower_file(src, f.name, parse(src))
        except (LarkError, LowerError) as e:
            print(f"{f.name}: syntax error: {str(e).strip().splitlines()[0]}")
            bad += 1
            continue
        problems = check_fn(fn) if fn else []
        for line, msg in problems:
            print(f"{f.name}:{line}: {msg}")
        bad += bool(problems)
        if not problems:
            print(f"ok    {f.name}")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
