"""Functions shared by the CLI and the MCP server: everything returns JSON dictionaries."""
from . import stdlib
from .interp import run_function
from .library import load_library


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
                            "library": b.lib or "core (always available)"} for b in stdlib.BUILTINS.values()]
    return out


def run(folder, function, sim=None, args=None, trace=False, **limits):
    lib = load_library(folder)
    if lib.errors:
        return {"function": function, "ok": False, "outputs": {}, "error": None,
                "fault": {"category": "lcf.check_failed",
                          "message": f"{len(lib.errors)} check error(s): fix them, then run `check` again"},
                "diagnostics": _diags(lib)}
    return run_function(lib, function, dict(args or {}), sim, trace, **limits)
