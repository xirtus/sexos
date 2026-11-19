# SPDX-License-Identifier: GPL-3.0-only
"""mathutils.kdtree: a 3D k-d tree for nearest-point searches.

    kd = KDTree(len(verts))
    for i, co in enumerate(verts):
        kd.insert(co, i)
    kd.balance()
    co, index, dist = kd.find((0, 0, 0))
"""
import heapq
import math

from . import Vector


class KDTree:
    def __init__(self, size):
        self._size = int(size)
        self._items = []
        self._root = None
        self._balanced = False

    def insert(self, co, index):
        if len(self._items) >= self._size:
            raise ValueError('KDTree.insert(): the tree is full (size %d)' % self._size)
        c = [float(x) for x in co]
        if len(c) == 2:
            c.append(0.0)
        self._items.append((c[0], c[1], c[2], int(index)))
        self._balanced = False

    def balance(self):
        def build(items, depth):
            if not items:
                return None
            ax = depth % 3
            items.sort(key=lambda p: p[ax])
            m = len(items) // 2
            return (items[m], ax, build(items[:m], depth + 1), build(items[m + 1:], depth + 1))
        self._root = build(list(self._items), 0)
        self._balanced = True

    def _check(self):
        if not self._balanced:
            raise RuntimeError('KDTree: balance() must be called before searching')

    def _search(self, co, keep):
        """Visit candidates; `keep(dist2, item)` returns the current bound."""
        q = [float(x) for x in co] + [0.0] * (3 - len(co))
        stack = [self._root]
        bound = [float('inf')]

        def visit(node):
            if node is None:
                return
            p, ax, lo, hi = node
            d2 = (p[0] - q[0]) ** 2 + (p[1] - q[1]) ** 2 + (p[2] - q[2]) ** 2
            bound[0] = keep(d2, p)
            diff = q[ax] - p[ax]
            first, second = (lo, hi) if diff < 0 else (hi, lo)
            visit(first)
            if diff * diff <= bound[0]:
                visit(second)
        del stack
        visit(self._root)

    @staticmethod
    def _out(p, d2):
        return Vector((p[0], p[1], p[2])), p[3], math.sqrt(d2)

    def find(self, co, filter=None):
        self._check()
        best = [float('inf'), None]

        def keep(d2, p):
            if d2 < best[0] and (filter is None or filter(p[3])):
                best[0], best[1] = d2, p
            return best[0]
        self._search(co, keep)
        if best[1] is None:
            return None, None, None
        return self._out(best[1], best[0])

    def find_n(self, co, n):
        self._check()
        heap = []  # max-heap by -d2

        def keep(d2, p):
            if len(heap) < n:
                heapq.heappush(heap, (-d2, p[3], p))
            elif d2 < -heap[0][0]:
                heapq.heapreplace(heap, (-d2, p[3], p))
            return -heap[0][0] if len(heap) >= n else float('inf')
        if n > 0:
            self._search(co, keep)
        return [self._out(p, -d) for d, _, p in sorted(heap, key=lambda h: (-h[0], h[1]))]

    def find_range(self, co, radius):
        self._check()
        r2 = radius * radius
        hits = []

        def keep(d2, p):
            if d2 <= r2:
                hits.append((d2, p))
            return r2
        self._search(co, keep)
        hits.sort(key=lambda h: (h[0], h[1][3]))
        return [self._out(p, d2) for d2, p in hits]
