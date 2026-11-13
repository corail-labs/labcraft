"""Scenario 1 (docs/acceptance-m0.md): the checks of each session that scenario-1.sh left in OUT_DIR.

usage: python tests/acceptance/agent/scenario-1.py OUT_DIR CLI

Reads OUT_DIR/session-N.jsonl (the transcript) and OUT_DIR/session-N/ (the files the agent wrote), and checks with the
CLI, not with anything the agent said. Required (points 1 to 5): `check --deny-warnings` without error or warning; PASS
against dmm-230V and FAIL against dmm-250V, as an enumeration or a string; an error and no PASS against no-instrument;
`resources.open` empty in the three runs; at most 20 calls to the MCP tools, no tool but Read and Skill besides, no Read
outside the folder of the session (even refused) and no skill but those of LabCraft. The runs give the program the
inputs of the agent's last `run_start`. Recorded: canonical form, the calls, the `check` failures before the first clean
one, and whether the limits are inclusive, against noise-free multimeters at 219.99, 220, 240 and 240.01 V written into
OUT_DIR/exact/ (dmm-220V-exact and dmm-240V-exact are not shipped). Writes OUT_DIR/summary.md and OUT_DIR/summary.json;
exit 0 when at least four sessions succeed (point 6).
"""

import json
import os
import re
import subprocess
import sys
from pathlib import Path

MAX_CALLS = 20
MCP = "mcp__labcraft__"
EXACT = {"219.99": "fail", "220.0": "pass", "240.0": "pass", "240.01": "fail"}
VERDICT = re.compile(r"(?:\w+\.)?(pass|fail)", re.I)


def cli(binary, *args):
    done = subprocess.run([str(binary), *args], capture_output=True, text=True, encoding="utf-8")
    try:
        return done.returncode, json.loads(done.stdout)
    except json.JSONDecodeError:
        return done.returncode, {"stdout": done.stdout, "stderr": done.stderr}


def read_transcript(transcript):
    """The folder the session ran in, the skills it was offered, and its calls in order: (name, input, result)."""
    cwd, skills, calls, results = None, [], [], {}
    for line in transcript.read_text(encoding="utf-8").splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if not isinstance(event, dict):
            continue
        if event.get("type") == "system" and event.get("subtype") == "init":
            cwd, skills = event.get("cwd"), event.get("skills") or []
        message = event.get("message")
        content = message.get("content") if isinstance(message, dict) else None
        for part in content if isinstance(content, list) else []:
            if not isinstance(part, dict):
                continue
            if part.get("type") == "tool_use":
                calls.append((part.get("id"), part.get("name", ""), part.get("input") or {}))
            elif part.get("type") == "tool_result":
                result = part.get("content")
                results[part.get("tool_use_id")] = result if isinstance(result, str) else json.dumps(result)
    return cwd, skills, [(name, given, results.get(call_id, "")) for call_id, name, given in calls]


def inside(path, folder):
    """Whether a path the agent gave lies in the folder of the session; paths are compared without case (Windows)."""
    if not folder or not isinstance(path, str):
        return False
    full = os.path.normcase(os.path.abspath(os.path.join(folder, path)))
    root = os.path.normcase(os.path.abspath(folder))
    return full == root or full.startswith(root.rstrip(os.sep) + os.sep)


def check_failures(mcp):
    """The reports of `check` with errors before the first clean one, not counting `LC0002`, a folder still empty."""
    failures = 0
    for name, _, result in mcp:
        if not name.endswith(("library_check", "file_write")):
            continue
        errors = re.search(r'\\?"errors\\?":\s*(\d+)', result)
        if not errors:
            continue
        if int(errors.group(1)) == 0:
            break
        if "LC0002" not in result:
            failures += 1
    return failures


def only_function(folder):
    """The name of the function of the folder, when it declares exactly one."""
    names = [found for file in folder.glob("*.lcf")
             for found in re.findall(r"^fn\s+(\w+)", file.read_text(encoding="utf-8"), re.M)]
    return names[0] if len(names) == 1 else None


def verdict_of(report):
    """The verdict an output gives, `pass` or `fail`: a string or a variant whose whole text is that word, never a bool."""
    for output in report.get("outputs") or []:
        if not isinstance(output, dict):
            continue
        value = output.get("value")
        if output.get("type") != "bool" and isinstance(value, str):
            found = VERDICT.fullmatch(value)
            if found:
                return output.get("type"), value, found.group(1).lower()
    return None, None, None


def run(binary, folder, function, sim, args):
    code, report = cli(binary, "run", "--json", "--no-trace", "--sim", sim, *args, str(folder), function)
    ty, value, word = verdict_of(report)
    resources = report.get("resources")
    return {
        "exit": code,
        "status": report.get("status"),
        "verdict": value,
        "word": word,
        "type": ty,
        "open": resources.get("open") if isinstance(resources, dict) else None,
    }


def exact_benches(out):
    """Noise-free multimeters at the limits and just beyond them."""
    folder = out / "exact"
    folder.mkdir(exist_ok=True)
    for volts in EXACT:
        (folder / f"dmm-{volts}V.toml").write_text(
            '[[visa.instrument]]\nresource = "SIM::DMM0"\nidn = "SIM,DMM,0,1.0"\n\n'
            '[visa.instrument.measure]\n"MEAS:VOLT:DC?" = "voltage"\n\n'
            f"[visa.instrument.quantity.voltage]\nvalue = {volts}\n",
            encoding="utf-8",
            newline="\n",
        )
    return folder


def session(out, binary, exact, number):
    folder = out / f"session-{number}"
    transcript = out / f"session-{number}.jsonl"
    row = {"session": number, "success": False}
    cwd, skills, calls = read_transcript(transcript) if transcript.exists() else (None, [], [])
    mcp = [call for call in calls if call[0].startswith(MCP)]
    row["mcp calls"] = len(mcp)
    row["calls"] = [call[0].removeprefix(MCP) for call in calls]
    row["other tools"] = sorted({call[0] for call in calls if not call[0].startswith(MCP)})
    row["labcraft skills offered"] = sorted(skill for skill in skills if skill.startswith("labcraft-"))
    row["reads outside"] = [given.get("file_path") for name, given, _ in calls
                            if name == "Read" and not inside(given.get("file_path"), cwd)]
    row["skills used"] = [given.get("skill") for name, given, _ in calls if name == "Skill"]
    row["check failures before the first clean"] = check_failures(mcp)
    starts = [given for name, given, _ in mcp if name.endswith("run_start") and given.get("function")]
    function = starts[-1]["function"] if starts else (only_function(folder) if folder.exists() else None)
    row["function"] = function
    if not folder.exists() or not function:
        row["why"] = "no program or no function"
        return row
    given = starts[-1].get("args") if starts else None
    args = []
    if isinstance(given, dict) and given:
        inputs = out / f"session-{number}.args.json"
        inputs.write_text(json.dumps(given), encoding="utf-8")
        args = ["--args", str(inputs)]
    row["args"] = given
    code, checked = cli(binary, "check", "--json", "--deny-warnings", str(folder))
    summary = checked.get("summary") or {}
    row["check"] = {"exit": code, "errors": summary.get("errors"), "warnings": summary.get("warnings")}
    row["canonical"] = cli(binary, "fmt", "--check", str(folder))[0] == 0
    outcomes = {sim: run(binary, folder, function, f"<labcraft>/visa/sim/{sim}.toml", args)
                for sim in ["dmm-230V", "dmm-250V", "no-instrument"]}
    row["runs"] = outcomes
    limits = {volts: run(binary, folder, function, str(exact / f"dmm-{volts}V.toml"), args)["word"] for volts in EXACT}
    row["limits"] = limits
    row["limits inclusive"] = limits == EXACT
    row["points"] = {
        "1": row["check"] == {"exit": 0, "errors": 0, "warnings": 0},
        "2": outcomes["dmm-230V"]["word"] == "pass" and outcomes["dmm-250V"]["word"] == "fail",
        "3": outcomes["no-instrument"]["status"] == "error" and outcomes["no-instrument"]["word"] != "pass",
        "4": all(outcome["open"] == [] for outcome in outcomes.values()),
        "5": len(mcp) <= MAX_CALLS and set(row["other tools"]) <= {"Read", "Skill"} and not row["reads outside"]
        and all(isinstance(skill, str) and skill.startswith("labcraft-") for skill in row["skills used"]),
    }
    row["success"] = all(row["points"].values())
    return row


def main():
    if len(sys.argv) != 3:
        print(__doc__, file=sys.stderr)
        return 2
    out, binary = Path(sys.argv[1]), Path(sys.argv[2])
    if not out.is_dir():
        print(f"no folder {out}", file=sys.stderr)
        return 2
    if not binary.is_file():
        print(f"no CLI at {binary}", file=sys.stderr)
        return 2
    exact = exact_benches(out)
    numbers = sorted(int(path.stem.split("-")[1]) for path in out.glob("session-*.jsonl") if path.stem.split("-")[1].isdigit())
    rows = [session(out, binary, exact, number) for number in numbers]
    versions = out / "versions.txt"
    versions = versions.read_text(encoding="utf-8").strip().replace("\n", "; ") if versions.exists() else "versions unknown"
    lines = ["# Scenario 1 by hand", "", versions + ".", "",
             "| Session | Success | Points 1-5 | MCP calls | Check failures first | Canonical | Limits inclusive "
             "| Function | 230V | 250V | no-instrument |",
             "|---|---|---|---|---|---|---|---|---|---|---|"]
    for row in rows:
        runs = row.get("runs", {})
        shown = lambda sim: f"{runs[sim]['status']} {runs[sim]['verdict']}" if sim in runs else "-"
        points = " ".join(f"{key}:{'ok' if ok else 'NO'}" for key, ok in row.get("points", {}).items()) or row.get("why", "")
        lines.append(f"| {row['session']} | {row['success']} | {points} | {row['mcp calls']} | "
                     f"{row['check failures before the first clean']} | {row.get('canonical')} | "
                     f"{row.get('limits inclusive')} | {row.get('function')} | {shown('dmm-230V')} | {shown('dmm-250V')} | "
                     f"{shown('no-instrument')} |")
    succeeded = sum(1 for row in rows if row["success"])
    lines += ["", f"**{succeeded} of {len(rows)} sessions succeed** (point 6 asks at least 4 of 5)."]
    (out / "summary.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
    (out / "summary.json").write_text(json.dumps(rows, indent=2) + "\n", encoding="utf-8")
    print("\n".join(lines))
    return 0 if succeeded >= 4 else 1


if __name__ == "__main__":
    sys.exit(main())
