# SPDX-License-Identifier: GPL-3.0-only
"""mathutils.bvhtree: ray casts and nearest-point queries against polygons.

    bvh = BVHTree.FromPolygons(verts, polys)
    location, normal, index, distance = bvh.ray_cast(origin, direction)

`index` is the polygon index (the triangles of an n-gon share it). Misses
answer (None, None, None, None), like Blender.
"""
import math

from . import Vector
from .geometry import closest_point_on_tri

_MISS = (None, None, None, None)


class _Node:
    __slots__ = ('lo', 'hi', 'left', 'right', 'tris')


class BVHTree:
    def __init__(self, verts, tris, owner, epsilon=0.0):
        self._verts = [tuple(float(c) for c in v[:3]) for v in verts]
        self._tris = tris  # list of (a, b, c)
        self._owner = owner  # polygon index of each triangle
        self._eps = epsilon
        self._root = self._build(list(range(len(tris))))

    # --- construction
    @classmethod
    def FromPolygons(cls, vertices, polygons, all_triangles=False, epsilon=0.0):
        tris, owner = [], []
        verts = [tuple(v) for v in vertices]
        for pi, poly in enumerate(polygons):
            p = list(poly)
            if len(p) < 3:
                continue
            if len(p) == 3 or all_triangles:
                tris.append((p[0], p[1], p[2]))
                owner.append(pi)
                continue
            from .geometry import tessellate_polygon
            for t in tessellate_polygon([[verts[i] for i in p]]):
                tris.append((p[t[0]], p[t[1]], p[t[2]]))
                owner.append(pi)
        return cls(verts, tris, owner, epsilon)

    @classmethod
    def FromBMesh(cls, bmesh, epsilon=0.0):
        bmesh.verts.index_update()
        verts = [tuple(v.co) for v in bmesh.verts]
        polys = [[v.index for v in f.verts] for f in bmesh.faces]
        return cls.FromPolygons(verts, polys, epsilon=epsilon)

    @classmethod
    def FromObject(cls, object, depsgraph=None, deform=True, render=False, cage=False, epsilon=0.0):
        me = object.evaluated_get(depsgraph).to_mesh() if deform else object.data
        verts = [tuple(v.co) for v in me.vertices]
        polys = [list(p.vertices) for p in me.polygons]
        return cls.FromPolygons(verts, polys, epsilon=epsilon)

    def _bounds(self, ids):
        lo = [float('inf')] * 3
        hi = [float('-inf')] * 3
        for t in ids:
            for vi in self._tris[t]:
                v = self._verts[vi]
                for k in range(3):
                    if v[k] < lo[k]:
                        lo[k] = v[k]
                    if v[k] > hi[k]:
                        hi[k] = v[k]
        e = self._eps
        return [x - e for x in lo], [x + e for x in hi]

    def _build(self, ids):
        n = _Node()
        n.lo, n.hi = self._bounds(ids)
        n.left = n.right = None
        if len(ids) <= 4:
            n.tris = ids
            return n
        ax = max(range(3), key=lambda k: n.hi[k] - n.lo[k])

        def centre(t):
            a, b, c = (self._verts[i] for i in self._tris[t])
            return a[ax] + b[ax] + c[ax]
        ids.sort(key=centre)
        m = len(ids) // 2
        n.tris = None
        n.left = self._build(ids[:m])
        n.right = self._build(ids[m:])
        return n

    # --- queries
    def _tri(self, t):
        a, b, c = (Vector(self._verts[i]) for i in self._tris[t])
        return a, b, c

    @staticmethod
    def _ray_box(o, inv, lo, hi, tmax):
        t0, t1 = 0.0, tmax
        for k in range(3):
            if inv[k] is None:
                if o[k] < lo[k] or o[k] > hi[k]:
                    return False
                continue
            a = (lo[k] - o[k]) * inv[k]
            b = (hi[k] - o[k]) * inv[k]
            if a > b:
                a, b = b, a
            t0, t1 = max(t0, a), min(t1, b)
            if t0 > t1:
                return False
        return True

    def ray_cast(self, origin, direction, distance=float('inf')):
        o = Vector(origin).to_3d()
        d = Vector(direction).to_3d()
        if d.length == 0:
            return _MISS
        d = d.normalized()
        inv = [1.0 / x if abs(x) > 1e-30 else None for x in d]
        best = [distance, None]
        stack = [self._root]
        while stack:
            n = stack.pop()
            if n is None or not self._ray_box(o, inv, n.lo, n.hi, best[0]):
                continue
            if n.tris is None:
                stack.append(n.left)
                stack.append(n.right)
                continue
            for t in n.tris:
                a, b, c = self._tri(t)
                e1, e2 = b - a, c - a
                p = d.cross(e2)
                det = e1.dot(p)
                if abs(det) < 1e-14:
                    continue
                inv_det = 1.0 / det
                s = o - a
                u = s.dot(p) * inv_det
                if u < -1e-9 or u > 1 + 1e-9:
                    continue
                q = s.cross(e1)
                v = d.dot(q) * inv_det
                if v < -1e-9 or u + v > 1 + 1e-9:
                    continue
                dist = e2.dot(q) * inv_det
                if 0 <= dist < best[0]:
                    best[0], best[1] = dist, t
        if best[1] is None:
            return _MISS
        t = best[1]
        a, b, c = self._tri(t)
        nrm = (b - a).cross(c - a).normalized()
        return o + d * best[0], nrm, self._owner[t], best[0]

    def _box_dist2(self, p, lo, hi):
        s = 0.0
        for k in range(3):
            if p[k] < lo[k]:
                s += (lo[k] - p[k]) ** 2
            elif p[k] > hi[k]:
                s += (p[k] - hi[k]) ** 2
        return s

    def find_nearest(self, origin, distance=float('inf')):
        p = Vector(origin).to_3d()
        best = [distance * distance if distance != float('inf') else float('inf'), None, None]
        stack = [self._root]
        while stack:
            n = stack.pop()
            if n is None or self._box_dist2(p, n.lo, n.hi) > best[0]:
                continue
            if n.tris is None:
                stack.append(n.left)
                stack.append(n.right)
                continue
            for t in n.tris:
                a, b, c = self._tri(t)
                q = closest_point_on_tri(p, a, b, c)
                d2 = (q - p).length_squared
                if d2 < best[0]:
                    best = [d2, t, q]
        if best[1] is None:
            return _MISS
        a, b, c = self._tri(best[1])
        return best[2], (b - a).cross(c - a).normalized(), self._owner[best[1]], math.sqrt(best[0])

    def find_nearest_range(self, origin, distance=float('inf')):
        p = Vector(origin).to_3d()
        r2 = distance * distance
        out = {}
        stack = [self._root]
        while stack:
            n = stack.pop()
            if n is None or self._box_dist2(p, n.lo, n.hi) > r2:
                continue
            if n.tris is None:
                stack.append(n.left)
                stack.append(n.right)
                continue
            for t in n.tris:
                a, b, c = self._tri(t)
                q = closest_point_on_tri(p, a, b, c)
                d2 = (q - p).length_squared
                if d2 <= r2:
                    pi = self._owner[t]
                    if pi not in out or d2 < out[pi][3] ** 2:
                        out[pi] = (q, (b - a).cross(c - a).normalized(), pi, math.sqrt(d2))
        return sorted(out.values(), key=lambda r: r[3])

    def overlap(self, other_tree):
        """Pairs (polygon here, polygon there) whose triangles intersect."""
        pairs = set()
        for t in range(len(self._tris)):
            a, b, c = self._tri(t)
            for u in range(len(other_tree._tris)):
                d, e, f = other_tree._tri(u)
                if _tri_tri(a, b, c, d, e, f):
                    pairs.add((self._owner[t], other_tree._owner[u]))
        return sorted(pairs)


def _seg_tri(p, q, a, b, c):
    d = q - p
    e1, e2 = b - a, c - a
    h = d.cross(e2)
    det = e1.dot(h)
    if abs(det) < 1e-14:
        return False
    f = 1.0 / det
    s = p - a
    u = f * s.dot(h)
    if u < 0 or u > 1:
        return False
    qq = s.cross(e1)
    v = f * d.dot(qq)
    if v < 0 or u + v > 1:
        return False
    t = f * e2.dot(qq)
    return 0 <= t <= 1


def _tri_tri(a, b, c, d, e, f):
    for p, q in ((a, b), (b, c), (c, a)):
        if _seg_tri(p, q, d, e, f):
            return True
    for p, q in ((d, e), (e, f), (f, d)):
        if _seg_tri(p, q, a, b, c):
            return True
    return False
