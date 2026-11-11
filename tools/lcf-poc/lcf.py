#!/usr/bin/env python3
"""LabCraft PoC: check and run .lcf functions against simulated instruments (throwaway).

    python tools/lcf-poc/lcf.py check examples
    python tools/lcf-poc/lcf.py fmt --check --spec docs/language-spec-v0.md examples docs/rfc
    python tools/lcf-poc/lcf.py list examples
    python tools/lcf-poc/lcf.py run examples check_voltage --sim libs/visa/sim/dmm-230V.toml \\
        --arg resource=SIM::DMM0 [--trace]
    python tools/lcf-poc/lcf.py run examples two_benches --sim libs/visa/sim/two-dmm.toml \\
        --set stop_switch=true@450ms [--schedule-seed N] [--timeout 300ms] [--real-time]
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
    f = sub.add_parser("fmt", help="canonical order of the lines and layout of the text of .lcf and .lct files "
                                       "and of the code blocks of the spec (spec 5.2); the PoC only checks, "
                                       "it never rewrites; messages give FILE:LINE")
    f.add_argument("paths", nargs="*", help="files (.lcf, .lct) or folders")
    f.add_argument("--spec", metavar="PATH", action="append", default=[],
                   help="also check the layout of the complete code blocks of a Markdown file, or of each .md "
                        "file of a folder (blocks with `…` and the reference listings are skipped); can be "
                        "given several times")
    f.add_argument("--check", action="store_true", required=True, help="report the order of lines and the layout rules that are broken")
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
    r.add_argument("--set", action="append", default=[], metavar="ELEMENT=VALUE@TIME",
                   help="set a panel element of the function at a time of the run (cli.md 6.4)")
    r.add_argument("--schedule-seed", type=int, metavar="N",
                   help="draw the order among the nodes ready at the same time from N (deterministic mode)")
    r.add_argument("--timeout", metavar="DURATION", help="abort the run after DURATION on its clock")
    r.add_argument("--real-time", action="store_true", help="real time instead of the simulated clock")
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
        if not a.paths and not a.spec:
            ap.error("fmt: give files or folders, or --spec PATH")
        return fmt_check(a.paths, a.spec)
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
    settings = []
    for item in a.set:
        head, at_sign, at = item.rpartition("@")  # the time is after the last `@` (cli.md 6.4)
        name, eq, value = head.partition("=")
        if not at_sign or not eq:
            print(f"--set expects ELEMENT=VALUE@TIME, got '{item}'", file=sys.stderr)
            return 2
        settings.append({"element": name.strip(), "value": value, "at": at})
    if a.schedule_seed is not None and (a.real_time or not 0 <= a.schedule_seed < 2 ** 64):
        print("--schedule-seed: an integer from 0 to 2^64-1, in deterministic mode only", file=sys.stderr)
        return 2
    rep = api.run(a.folder, a.function, a.sim, args, a.trace, settings, max_iter=a.max_iterations,
                  schedule_seed=a.schedule_seed, timeout=a.timeout, real_time=a.real_time)
    print(json.dumps(rep, ensure_ascii=False, indent=2))
    return EXIT.get(rep.get("status"), 1)


EXIT = {"ok": 0, "error": 1, "refused": 1, "aborted": 3, "fault": 4}  # cli.md, section 6


def fmt_check(paths, specs=()):
    """Print the problems of each file (and of the code blocks of each Markdown file of `specs`); 1 if any, else 0."""
    from lcfpoc.canon import check_fn
    from lcfpoc.layout import check_layout, check_spec_layout
    from lcfpoc.ir import LowerError, lower_file
    from lcfpoc.library import MAX_FILE
    from lcfpoc.library import parse
    from lark.exceptions import LarkError
    files, bad = [], 0
    for p in map(Path, paths):
        if p.is_dir():
            files += sorted(f for f in [*p.rglob("*.lcf"), *p.rglob("*.lct")]
                            if not any(part.startswith(".") for part in f.relative_to(p).parts))
        elif p.exists():
            files.append(p)
        else:
            print(f"{p.as_posix()}: no such file or folder")
            bad += 1
    for f in files:
        name = f.as_posix()
        try:
            if f.stat().st_size > MAX_FILE:
                print(f"{name}: file too large (> {MAX_FILE} bytes): not checked")
                bad += 1
                continue
        except OSError as e:
            print(f"{name}: cannot read: {e.strerror or e}")
            bad += 1
            continue
        try:
            raw = f.read_bytes().decode("utf-8")
        except UnicodeDecodeError as e:
            line = f.read_bytes()[:e.start].count(10) + 1
            print(f"{name}:{line}: not UTF-8 text (spec 2): {e.reason} at byte {e.start}")
            bad += 1
            continue
        except OSError as e:
            print(f"{name}: cannot read: {e.strerror or e}")
            bad += 1
            continue
        src = raw.replace(chr(13) + chr(10), chr(10))
        try:
            fn, _, _, _ = lower_file(src, f.name, parse(src))
        except (LarkError, LowerError) as e:
            lines = str(e).strip().splitlines()
            line = getattr(e, "line", 0) or 0
            if line < 1 and getattr(e, "line", 0) == -1:  # unexpected end of input: the last line
                line = max(src.rstrip(chr(10)).count(chr(10)) + 1, 1)
            print(f"{name}:{line}: syntax error: {lines[0] if lines else type(e).__name__}" if line
                  else f"{name}: syntax error: {lines[0] if lines else type(e).__name__}")
            bad += 1
            continue
        except Exception as e:  # never a traceback on hostile input
            print(f"{name}: cannot be analyzed: {type(e).__name__}")
            bad += 1
            continue
        # an `extern` file has no body: only its layout is checked
        problems = sorted((check_fn(fn) if fn and not fn.extern else []) + check_layout(raw), key=lambda p: p[0])
        for line, msg in problems:
            print(f"{name}:{line}: {msg}")
        bad += bool(problems)
        if not problems:
            print(f"ok    {name}")
    if isinstance(specs, (str, Path)):
        specs = [specs]
    markdown = []
    for spec in specs:
        markdown += sorted(Path(spec).rglob("*.md")) if Path(spec).is_dir() else [Path(spec)]
    for spec in markdown:
        sp = Path(spec).as_posix()
        try:
            if Path(spec).stat().st_size > MAX_FILE:
                print(f"{sp}: file too large (> {MAX_FILE} bytes): not checked")
                return 1
            problems, done, skipped = check_spec_layout(spec)
        except UnicodeDecodeError as e:
            print(f"{sp}: not UTF-8 text (spec 2): {e.reason} at byte {e.start}")
            return 1
        except OSError as e:
            print(f"{sp}: cannot read: {e.strerror or e}")
            return 1
        for line, msg in problems:
            print(f"{sp}:{line}: {msg}")
        bad += bool(problems)
        print(f"{'ok    ' if not problems else ''}{sp}: {done} code blocks checked, {skipped} skipped, "
              f"{len(problems)} problem(s)")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
