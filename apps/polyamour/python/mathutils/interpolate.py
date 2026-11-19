# SPDX-License-Identifier: GPL-3.0-only
"""mathutils.interpolate: poly_3d_calc (barycentric-style weights of a
point inside a polygon, Blender's mean value coordinates)."""
import math

from . import Vector


def poly_3d_calc(veclist, pt):
    pts = [Vector(v).to_3d() for v in veclist]
    p = Vector(pt).to_3d()
    n = len(pts)
    for i, v in enumerate(pts):
        if (v - p).length < 1e-9:
            return [1.0 if j == i else 0.0 for j in range(n)]
    w = []
    for i in range(n):
        prev, cur, nxt = pts[i - 1], pts[i], pts[(i + 1) % n]
        a0 = (prev - p).angle(cur - p, 0.0)
        a1 = (cur - p).angle(nxt - p, 0.0)
        d = (cur - p).length
        w.append((math.tan(a0 / 2) + math.tan(a1 / 2)) / d if d else 0.0)
    s = sum(w)
    return [x / s for x in w] if s else [1.0 / n] * n
