#!/usr/bin/env python3
"""MCP server (stdio) of the LabCraft PoC: the agent writes a .lcf, checks it, runs it, reads the result.

    claude mcp add labcraft-poc -- python <repo>/tools/lcf-poc/mcp_server.py

Relative paths are resolved from the client's working directory (the open project).
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from mcp.server.fastmcp import FastMCP  # noqa: E402

from lcfpoc import api  # noqa: E402

mcp = FastMCP(
    "labcraft-poc",
    instructions=(
        "LabCraft v0: a dataflow language for test benches. A library is a folder of "
        ".lcf files (one function per file) and .lct files (one type per file). Working loop: "
        "list_functions (with builtins=true) to learn the primitives, write the file, `check` "
        "until zero errors, `run` against a simulated instrument (a .toml file from the libs/visa/sim/ folder), "
        "read `outputs`, `error` and `fault`, fix. `trace=true` gives the execution order and the value "
        "of each wire. Spec: docs/language-spec-v0.md; examples: examples/."
    ),
)


@mcp.tool()
def check(folder: str) -> dict:
    """Checks a library (folder of .lcf/.lct files): syntax, names, inputs/outputs, literal units,
    cycles, match exhaustiveness. Returns the diagnostics (file, line, node, message)."""
    return api.check(folder)


@mcp.tool()
def list_functions(folder: str, builtins: bool = False) -> dict:
    """Lists the functions and types of a library (inputs, outputs, documentation). With
    builtins=true, adds the available primitives (core, array, time, errors, visa, scpi, …)."""
    return api.list_functions(folder, builtins)


@mcp.tool()
def run(folder: str, function: str, sim: str | None = None, args: dict | None = None, trace: bool = False,
        max_iterations: int = 10000) -> dict:
    """Runs a function against simulated instruments (deterministic, simulated clock).

    sim: instruments .toml file (e.g. libs/visa/sim/dmm-230V.toml). args: input values by name;
    numbers with a unit as text ("230 V", "50 ms"), arrays and structs as JSON, session = resource
    name ("SIM::DMM0"). Returns outputs, error (error that reached the `err` output), fault (run-time
    fault or invalid file), sim (simulated time, sessions still open, commands exchanged)
    and, with trace=true, the executed nodes in order with the wire values."""
    return api.run(folder, function, sim, args or {}, trace, max_iter=max_iterations)


if __name__ == "__main__":
    mcp.run()
