"""End-to-end test of the MCP server: a stdio client plays the agent loop
(write, check, see the error, fix, run). python tools/lcf-poc/tests/mcp_smoke.py"""
import asyncio
import json
import sys
import tempfile
from pathlib import Path

from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

ROOT = Path(__file__).resolve().parents[3]

FIRST_DRAFT = """labcraft 1

use core
use visa
use scpi

/// Checks that a 5 V supply is between 4.75 V and 5.25 V.
fn check_5v(resource: string, err: error = none) -> (ok: bool, voltage: f64[V], err: error) {
    dmm, e1 = visa.open(resource, err: err)
    raw, dmm2, e2 = scpi.query_f64(dmm, "MEAS:VOLT:DC?", err: e1)
    voltage = core.with_unit(raw, [V])
    above = core.ge(voltage, 4.75 V)
    below = core.le(voltge, 5.25 V)
    ok = core.and(above, below)
    #close: e3 = visa.close(dmm2, err: e2)
    err = e3
}
"""


async def call(session, tool, **kw):
    res = await session.call_tool(tool, kw)
    return json.loads(res.content[0].text)


async def main():
    params = StdioServerParameters(command=sys.executable, args=[str(ROOT / "tools/lcf-poc/mcp_server.py")])
    with tempfile.TemporaryDirectory() as td:
        sim = Path(td) / "dmm-5v.toml"
        sim.write_text('[[visa.instrument]]\nresource = "SIM::DMM0"\n[visa.instrument.measure]\n"MEAS:VOLT:DC?" = "v"\n'
                       '[visa.instrument.quantity.v]\nvalue = 5.02\nnoise = 0.01\nseed = 3\n', encoding="utf-8")
        async with stdio_client(params) as (r, w), ClientSession(r, w) as s:
            await s.initialize()
            tools = [t.name for t in (await s.list_tools()).tools]
            print("tools:", tools)
            (Path(td) / "check_5v.lcf").write_text(FIRST_DRAFT, encoding="utf-8", newline="\n")
            rep = await call(s, "check", folder=td)
            print("1. check:", [f"{d['line']}: {d['message']}" for d in rep["diagnostics"]])
            assert not rep["ok"] and "voltge" in rep["diagnostics"][0]["message"]
            (Path(td) / "check_5v.lcf").write_text(FIRST_DRAFT.replace("voltge", "voltage"), encoding="utf-8", newline="\n")
            rep = await call(s, "check", folder=td)
            assert rep["ok"], rep
            out = await call(s, "run", folder=td, function="check_5v", sim=str(sim), args={"resource": "SIM::DMM0"})
            print("2. run:", out["ok"], out["outputs"])
            assert out["ok"] and out["outputs"]["ok"] is True
            lst = await call(s, "list_functions", folder=td, builtins=True)
            assert any(b["name"] == "scpi.query_f64" for b in lst["builtins"])
    print("MCP: OK")


asyncio.run(main())
