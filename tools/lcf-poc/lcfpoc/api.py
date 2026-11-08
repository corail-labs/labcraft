"""Functions shared by the CLI and the MCP server: everything returns JSON dictionaries."""
import re

from . import stdlib
from .interp import run_function
from .library import SHIPPED, load_library


def _diags(lib):
    return [dict(d) for d in lib.diags]


def check(folder):
    lib = load_library(folder)
    errors, warnings = len(lib.errors), len(lib.diags) - len(lib.errors)
    return {"ok": errors == 0, "folder": str(lib.path), "files": lib.files, "functions": sorted(lib.functions),
            "types": sorted(lib.types), "errors": errors, "warnings": warnings, "diagnostics": _diags(lib)}


def list_functions(folder, builtins=False):
    lib = load_library(folder)
    fns = [{"name": f.name, "file": f.file, "doc": f.doc,
            "inputs": [{"name": p.name, "type": p.type_text, "required": p.default is None} for p in f.params],
            "outputs": [{"name": n, "type": t} for n, _, t in f.outputs]} for f in lib.functions.values()]
    out = {"functions": sorted(fns, key=lambda f: f["name"]),
           "types": [{"name": t.name, "definition": t.text, "file": t.file} for t in lib.types.values()],
           "errors": len(lib.errors)}
    if builtins:
        out["builtins"] = [{"name": b.qname, "signature": b.signature(), "doc": b.doc,
                            "library": b.lib} for b in stdlib.BUILTINS.values()]
        shipped = sorted(p for p in SHIPPED.iterdir() if p.is_dir()) if SHIPPED.is_dir() else []
        libs = {p.name: load_library(p) for p in shipped}
        libs.update({d.name: d for d in lib.loader.libs.values()} if lib.loader else {})
        for name, one in sorted(libs.items()):
            for f in one.functions.values():
                out["builtins"].append(_entry(name, one, f))
    return out


def _entry(name, lib, f):
    """A function of a library, as a caller writes it: `visa.session`, not the `session` of its file."""
    def typed(t):
        for ty in lib.types:
            t = re.sub(rf"(?<![\w.]){re.escape(ty)}(?![\w])", f"{name}.{ty}", t)
        return t
    inputs = ", ".join(f"{p.name}: {typed(p.type_text)}" + ("" if p.default is None else " = …") for p in f.params)
    outputs = ", ".join(f"{n}: {typed(t)}" for n, _, t in f.outputs)
    always = f.extern and lib.callee(f)[1].always
    return {"name": f"{name}.{f.name}", "library": name, "doc": f.doc,
            "signature": f"{'always ' if always else ''}fn {name}.{f.name}({inputs}) -> ({outputs})"}


def run(folder, function, sim=None, args=None, trace=False, settings=(), **limits):
    lib = load_library(folder)
    if lib.errors:
        return {"function": function, "status": "refused", "ok": False, "outputs": {}, "error": None,
                "fault": {"category": "lcf.check_failed",
                          "message": f"{len(lib.errors)} check error(s): fix them, then run `check` again"},
                "diagnostics": _diags(lib)}
    return run_function(lib, function, dict(args or {}), sim, trace, settings if settings is not None else [], **limits)
