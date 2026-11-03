#!/usr/bin/env python3
"""LabCraft: renders a .lcf function as a FigCraft design, then exports it to SVG and PNG (throwaway tool).

    python tools/lcf-figcraft/lcf_figcraft.py examples --fn check_voltage --out build/

Writes build/<fn>.figcraft, build/<fn>.svg, build/<fn>.png. No LLM: the same .lcf always gives the same render.
The appearance (tokens, components, wires) is read from design/notation.figcraft.
"""
import argparse
import shutil
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from lcffc import emit, fc, layout, model  # noqa: E402
from lcffc.notation import Notation  # noqa: E402

DEFAULT_NOTATION = HERE.parents[1] / "design" / "notation.figcraft"


def build_script(folder, fn, notation_path=DEFAULT_NOTATION, theme=None):
    """Returns (jsonl, graph) for a function. Deterministic: no clock, no randomness."""
    N = Notation(notation_path)
    for _ in range(4):   # text measurement can change sizes: repeat while there is anything new
        model.Port._n = 0
        model.GNode._n = 0
        lib, f, root, body = model.load(folder, fn)
        lay = layout.Layouter(N)
        L = lay.layout_fn(root, body)
        if not N.resolve():
            break
    em = emit.Emitter(N, lay, theme)
    script = em.build(root, L, fn)
    return script, (lib, f, root, body, L)


def render(folder, fn, out_dir, notation_path=DEFAULT_NOTATION, theme=None, formats=("svg", "png"), scale=2):
    out = Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)
    script, graph = build_script(folder, fn, notation_path, theme)
    jsonl = out / f"{fn}.jsonl"
    jsonl.write_text(script.dumps(), encoding="utf-8", newline="\n")
    fig = out / f"{fn}.figcraft"
    res = fc.run(script, src=notation_path, out=fig)
    bad = [r for r in res if not r.get("ok", True)]
    if bad:
        raise fc.FigCraftError(f"command failed: {bad[0]}")
    for fmt in formats:
        with tempfile.TemporaryDirectory() as tmp:
            fc.export(fig, tmp, fmt, page="Render", scale=scale if fmt == "png" else None)
            files = sorted(Path(tmp).glob(f"*.{fmt}"))
            if not files:
                raise fc.FigCraftError(f"export {fmt}: no file")
            shutil.copyfile(files[0], out / f"{fn}.{fmt}")
    return out / f"{fn}.png", graph


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("folder", help="folder of a .lcf library (e.g. examples)")
    ap.add_argument("--fn", required=True, help="function to render")
    ap.add_argument("--out", default="build", help="output folder")
    ap.add_argument("--notation", default=str(DEFAULT_NOTATION), help="notation library (.figcraft)")
    ap.add_argument("--theme", default=None, help="mode of the token collection (Light, Dark)")
    ap.add_argument("--scale", type=float, default=2, help="PNG scale")
    a = ap.parse_args(argv)
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    png, _ = render(a.folder, a.fn, a.out, a.notation, a.theme, scale=a.scale)
    print(f"written {Path(a.out) / (a.fn + '.figcraft')}, .svg, .png")
    return 0


if __name__ == "__main__":
    sys.exit(main())
