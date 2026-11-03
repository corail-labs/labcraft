"""Bridge to `figcraft-cli`: JSON command scripts (one command per line), windowless execution, export."""
import json
import os
import subprocess
import tempfile
from pathlib import Path

DEFAULT_CLI = r"C:\Program Files\ArtCraft\FigCraft\figcraft-cli.exe"


class FigCraftError(Exception):
    pass


def cli_path():
    p = os.environ.get("FIGCRAFT_CLI", DEFAULT_CLI)
    if not Path(p).exists():
        raise FigCraftError(f"figcraft-cli not found ({p}): set FIGCRAFT_CLI")
    return p


def num(x):
    """Deterministic number: at most 2 decimals, never "-0"."""
    r = round(float(x), 2)
    if r == 0:
        r = 0.0
    return int(r) if r == int(r) else r


class Script:
    """List of commands; `add` returns a `{"$ref": name}` reference usable in later commands."""

    def __init__(self):
        self.steps = []
        self.n = 0

    def add(self, command, params=None, ref=True):
        step = {"command": command, "params": params or {}}
        out = None
        if ref:
            self.n += 1
            name = f"r{self.n}"
            step["as"] = name
            out = {"$ref": name}
        self.steps.append(step)
        return out

    def dumps(self):
        """One command per line, sorted keys: identical output from one run to the next."""
        return "".join(json.dumps(s, ensure_ascii=False, sort_keys=True, separators=(",", ":")) + "\n"
                       for s in self.steps)


def run(script, src=None, out=None, text=None):
    """Runs a script (Script, or JSONL text); returns the list of results (one dict per command)."""
    text = text if text is not None else script.dumps()
    with tempfile.NamedTemporaryFile("w", suffix=".jsonl", delete=False, encoding="utf-8", newline="\n") as f:
        f.write(text)
        tmp = f.name
    try:
        cmd = [cli_path(), "run", tmp]
        if src:
            cmd += ["--in", str(src)]
        if out:
            cmd += ["--out", str(out)]
        p = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8")
    finally:
        os.unlink(tmp)
    res = []
    for line in p.stdout.splitlines():
        line = line.strip()
        if line.startswith("{"):
            try:
                res.append(json.loads(line))
            except json.JSONDecodeError:
                pass
    if p.returncode != 0:
        raise FigCraftError((p.stderr or p.stdout).strip()[:1500])
    return res


def export(figfile, outdir, fmt, node=None, page=None, scale=None):
    cmd = [cli_path(), "export", str(figfile), "-o", str(outdir), "--format", fmt]
    if node:
        cmd += ["--node", node]
    if page:
        cmd += ["--page", page]
    if scale:
        cmd += ["--scale", str(scale)]
    p = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8")
    if p.returncode != 0:
        raise FigCraftError((p.stderr or p.stdout).strip()[:1500])
    return p.stdout
