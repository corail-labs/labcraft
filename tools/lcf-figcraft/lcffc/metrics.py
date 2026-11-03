"""Layout quality measures: wire crossings, label collisions (in absolute coordinates)."""


def collect(L, ox=0.0, oy=0.0, out=None):
    """Segments (source uid, p, q), node boxes and labels of all bodies, flattened."""
    out = out if out is not None else {"segs": [], "boxes": [], "labels": []}
    for e, pts in L.routes:
        for a, b in zip(pts, pts[1:]):
            out["segs"].append((e.src.uid, (a[0] + ox, a[1] + oy), (b[0] + ox, b[1] + oy)))
    for n, (nx, ny) in L.node_xy.items():
        if n.kind != "frame":
            out["boxes"].append((nx + ox, ny + oy, nx + ox + n.w, ny + oy + n.h))
        for item in getattr(n, "layouts", []):
            collect(item[0], ox + nx + item[1], oy + ny + item[2], out)
        for p, pts in getattr(n, "bus", []):
            for a, b in zip(pts, pts[1:]):
                out["segs"].append((p.uid, (a[0] + nx + ox, a[1] + ny + oy), (b[0] + nx + ox, b[1] + ny + oy)))
    for txt, x, y, fam, w, h in getattr(L, "label_boxes", []):
        out["labels"].append((txt, x + ox, y + oy, x + ox + w, y + oy + h))
    return out


def crossings(L):
    """Number of perpendicular crossings between wires from different sources."""
    segs = collect(L)["segs"]
    hs = [(s, a, b) for s, a, b in segs if abs(a[1] - b[1]) < 0.01 and abs(a[0] - b[0]) > 0.01]
    vs = [(s, a, b) for s, a, b in segs if abs(a[0] - b[0]) < 0.01 and abs(a[1] - b[1]) > 0.01]
    n = 0
    for sh, a, b in hs:
        x0, x1, y = min(a[0], b[0]), max(a[0], b[0]), a[1]
        for sv, c, d in vs:
            if sh == sv:
                continue
            x = c[0]
            y0, y1 = min(c[1], d[1]), max(c[1], d[1])
            if x0 + 0.5 < x < x1 - 0.5 and y0 + 0.5 < y < y1 - 0.5:
                n += 1
    return n


def label_collisions(L):
    """Wire labels that touch a wire (other than their own), a node or another label."""
    d = collect(L)
    bad = 0
    labs = d["labels"]
    for i, (txt, x0, y0, x1, y1) in enumerate(labs):
        hit = False
        for s, a, b in d["segs"]:
            sx0, sx1 = sorted((a[0], b[0]))
            sy0, sy1 = sorted((a[1], b[1]))
            if sx1 >= x0 and sx0 <= x1 and sy1 >= y0 and sy0 <= y1:
                hit = True
                break
        if not hit:
            for bx0, by0, bx1, by1 in d["boxes"]:
                if bx1 > x0 and bx0 < x1 and by1 > y0 and by0 < y1:
                    hit = True
                    break
        if not hit:
            for j, (t2, a0, b0, a1, b1) in enumerate(labs):
                if j != i and a1 > x0 and a0 < x1 and b1 > y0 and b0 < y1:
                    hit = True
                    break
        bad += hit
    return bad
