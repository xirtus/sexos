# SPDX-License-Identifier: GPL-3.0-only
"""mathutils.geometry: intersections, distances, areas and polygon helpers,
with Blender's argument orders and return conventions (None when there is
no intersection)."""
import math

from . import Vector

_EPS = 1e-6


def _v(x):
    return x if isinstance(x, Vector) else Vector(x)


def _v3(x):
    v = _v(x)
    return v.to_3d() if len(v) == 2 else Vector(v[:3])


def intersect_line_line(v1, v2, v3, v4):
    """Closest points between line v1-v2 and line v3-v4: (on first, on second),
    or None when the lines are parallel."""
    p1, p2, p3, p4 = _v3(v1), _v3(v2), _v3(v3), _v3(v4)
    a = p2 - p1
    b = p4 - p3
    c = p3 - p1
    ab = a.cross(b)
    d = ab.length_squared
    if d < 1e-12:
        return None
    t = c.cross(b).dot(ab) / d
    s = c.cross(a).dot(ab) / d
    r1, r2 = p1 + a * t, p3 + b * s
    if len(_v(v1)) == 2:
        return r1.to_2d(), r2.to_2d()
    return r1, r2


def intersect_line_line_2d(lineA_p1, lineA_p2, lineB_p1, lineB_p2):
    """Intersection of two 2D segments, or None."""
    a1, a2, b1, b2 = _v(lineA_p1).to_2d(), _v(lineA_p2).to_2d(), _v(lineB_p1).to_2d(), _v(lineB_p2).to_2d()
    r = a2 - a1
    s = b2 - b1
    d = r.x * s.y - r.y * s.x
    if abs(d) < 1e-12:
        return None
    q = b1 - a1
    t = (q.x * s.y - q.y * s.x) / d
    u = (q.x * r.y - q.y * r.x) / d
    if 0 <= t <= 1 and 0 <= u <= 1:
        return a1 + r * t
    return None


def intersect_ray_tri(v1, v2, v3, ray, orig, clip=True):
    """Möller-Trumbore: where the ray from `orig` along `ray` meets the
    triangle, or None. With clip=False the infinite plane of the triangle is
    used."""
    a, b, c = _v3(v1), _v3(v2), _v3(v3)
    d, o = _v3(ray), _v3(orig)
    e1 = b - a
    e2 = c - a
    p = d.cross(e2)
    det = e1.dot(p)
    if abs(det) < 1e-12:
        return None
    inv = 1.0 / det
    t = o - a
    u = t.dot(p) * inv
    q = t.cross(e1)
    v = d.dot(q) * inv
    if clip and (u < 0 or u > 1 or v < 0 or u + v > 1):
        return None
    lam = e2.dot(q) * inv
    if clip and lam < 0:
        return None
    return o + d * lam


def intersect_line_plane(line_a, line_b, plane_co, plane_no, no_flip=False):
    a, b = _v3(line_a), _v3(line_b)
    co, no = _v3(plane_co), _v3(plane_no)
    u = b - a
    d = no.dot(u)
    if abs(d) < 1e-12:
        return None
    t = no.dot(co - a) / d
    return a + u * t


def intersect_plane_plane(plane_a_co, plane_a_no, plane_b_co, plane_b_no):
    a_co, a_no, b_co, b_no = _v3(plane_a_co), _v3(plane_a_no), _v3(plane_b_co), _v3(plane_b_no)
    direction = a_no.cross(b_no)
    det = direction.length_squared
    if det < 1e-12:
        return None, None
    da, db = a_no.dot(a_co), b_no.dot(b_co)
    point = (b_no.cross(direction) * da + direction.cross(a_no) * db) / det
    return point, direction.normalized()


def intersect_line_sphere(line_a, line_b, sphere_co, sphere_radius, clip=True):
    a, b, c = _v3(line_a), _v3(line_b), _v3(sphere_co)
    d = b - a
    f = a - c
    A = d.dot(d)
    B = 2 * f.dot(d)
    C = f.dot(f) - sphere_radius * sphere_radius
    disc = B * B - 4 * A * C
    if A == 0 or disc < 0:
        return None, None
    s = math.sqrt(disc)
    t1, t2 = (-B - s) / (2 * A), (-B + s) / (2 * A)
    p1 = a + d * t1 if not clip or 0 <= t1 <= 1 else None
    p2 = a + d * t2 if not clip or 0 <= t2 <= 1 else None
    return p1, p2


def intersect_line_sphere_2d(line_a, line_b, sphere_co, sphere_radius, clip=True):
    p1, p2 = intersect_line_sphere(_v(line_a).to_3d(), _v(line_b).to_3d(), _v(sphere_co).to_3d(), sphere_radius, clip)
    return (p1.to_2d() if p1 else None), (p2.to_2d() if p2 else None)


def intersect_sphere_sphere_2d(p_a, radius_a, p_b, radius_b):
    a, b = _v(p_a).to_2d(), _v(p_b).to_2d()
    d = (b - a).length
    if d > radius_a + radius_b or d < abs(radius_a - radius_b) or d == 0:
        return None, None
    x = (d * d - radius_b * radius_b + radius_a * radius_a) / (2 * d)
    h = math.sqrt(max(0.0, radius_a * radius_a - x * x))
    u = (b - a) / d
    mid = a + u * x
    perp = Vector((-u.y, u.x))
    return mid + perp * h, mid - perp * h


def intersect_point_line(pt, line_p1, line_p2):
    """The closest point on the line and its factor along line_p1->line_p2."""
    p, a, b = _v(pt), _v(line_p1), _v(line_p2)
    n = len(a)
    p = Vector(list(p[:n]))
    u = b - a
    d = u.length_squared
    if d == 0:
        return Vector(a), 0.0
    t = (p - a).dot(u) / d
    return a + u * t, t


def intersect_point_tri(pt, tri_p1, tri_p2, tri_p3):
    """The point projected onto the triangle when it lies inside it, else None."""
    p, a, b, c = _v3(pt), _v3(tri_p1), _v3(tri_p2), _v3(tri_p3)
    n = (b - a).cross(c - a)
    if n.length_squared == 0:
        return None
    n.normalize()
    q = p - n * (p - a).dot(n)
    for x, y in ((a, b), (b, c), (c, a)):
        if (y - x).cross(q - x).dot(n) < -_EPS:
            return None
    return q


def intersect_point_tri_2d(pt, tri_p1, tri_p2, tri_p3):
    """1 inside (counter-clockwise), -1 inside (clockwise), 0 outside."""
    p, a, b, c = (_v(x).to_2d() for x in (pt, tri_p1, tri_p2, tri_p3))

    def side(x, y):
        return (y.x - x.x) * (p.y - x.y) - (y.y - x.y) * (p.x - x.x)
    s = [side(a, b), side(b, c), side(c, a)]
    if all(x >= 0 for x in s):
        return 1
    if all(x <= 0 for x in s):
        return -1
    return 0


def intersect_point_quad_2d(pt, quad_p1, quad_p2, quad_p3, quad_p4):
    r = intersect_point_tri_2d(pt, quad_p1, quad_p2, quad_p3)
    return r or intersect_point_tri_2d(pt, quad_p1, quad_p3, quad_p4)


def intersect_tri_tri_2d(tri_a1, tri_a2, tri_a3, tri_b1, tri_b2, tri_b3):
    A = [tri_a1, tri_a2, tri_a3]
    B = [tri_b1, tri_b2, tri_b3]
    for i in range(3):
        for j in range(3):
            if intersect_line_line_2d(A[i], A[(i + 1) % 3], B[j], B[(j + 1) % 3]) is not None:
                return True
    return bool(intersect_point_tri_2d(A[0], *B) or intersect_point_tri_2d(B[0], *A))


def closest_point_on_tri(pt, tri_p1, tri_p2, tri_p3):
    p, a, b, c = _v3(pt), _v3(tri_p1), _v3(tri_p2), _v3(tri_p3)
    ab, ac, ap = b - a, c - a, p - a
    d1, d2 = ab.dot(ap), ac.dot(ap)
    if d1 <= 0 and d2 <= 0:
        return a
    bp = p - b
    d3, d4 = ab.dot(bp), ac.dot(bp)
    if d3 >= 0 and d4 <= d3:
        return b
    vc = d1 * d4 - d3 * d2
    if vc <= 0 and d1 >= 0 and d3 <= 0:
        return a + ab * (d1 / (d1 - d3))
    cp = p - c
    d5, d6 = ab.dot(cp), ac.dot(cp)
    if d6 >= 0 and d5 <= d6:
        return c
    vb = d5 * d2 - d1 * d6
    if vb <= 0 and d2 >= 0 and d6 <= 0:
        return a + ac * (d2 / (d2 - d6))
    va = d3 * d6 - d5 * d4
    if va <= 0 and (d4 - d3) >= 0 and (d5 - d6) >= 0:
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)))
    denom = 1.0 / (va + vb + vc)
    return a + ab * (vb * denom) + ac * (vc * denom)


def distance_point_to_plane(pt, plane_co, plane_no):
    """Signed distance (positive on the side the normal points to)."""
    no = _v3(plane_no).normalized()
    return (_v3(pt) - _v3(plane_co)).dot(no)


def area_tri(v1, v2, v3):
    a, b, c = _v(v1), _v(v2), _v(v3)
    if len(a) == 2:
        return abs((b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y)) / 2.0
    return (_v3(b) - _v3(a)).cross(_v3(c) - _v3(a)).length / 2.0


def volume_tetrahedron(v1, v2, v3, v4):
    a, b, c, d = _v3(v1), _v3(v2), _v3(v3), _v3(v4)
    return abs((b - a).cross(c - a).dot(d - a)) / 6.0


def normal(*vectors):
    """The normal of a polygon (Newell's method; for 3 points the cross product)."""
    if len(vectors) == 1 and not isinstance(vectors[0], Vector) and len(vectors[0]) and hasattr(vectors[0][0], '__len__'):
        vectors = vectors[0]
    pts = [_v3(v) for v in vectors]
    if len(pts) < 3:
        raise ValueError('normal(): at least 3 points are needed')
    if len(pts) == 3:
        n = (pts[1] - pts[0]).cross(pts[2] - pts[0])
    else:
        n = Vector((0.0, 0.0, 0.0))
        for i, p in enumerate(pts):
            q = pts[(i + 1) % len(pts)]
            n.x += (p.y - q.y) * (p.z + q.z)
            n.y += (p.z - q.z) * (p.x + q.x)
            n.z += (p.x - q.x) * (p.y + q.y)
    l = n.length
    return n / l if l > 0 else Vector((0.0, 0.0, 0.0))


def barycentric_transform(point, tri_a1, tri_a2, tri_a3, tri_b1, tri_b2, tri_b3):
    p, a1, a2, a3 = _v3(point), _v3(tri_a1), _v3(tri_a2), _v3(tri_a3)
    b1, b2, b3 = _v3(tri_b1), _v3(tri_b2), _v3(tri_b3)
    v0, v1, v2 = a2 - a1, a3 - a1, p - a1
    d00, d01, d11 = v0.dot(v0), v0.dot(v1), v1.dot(v1)
    d20, d21 = v2.dot(v0), v2.dot(v1)
    den = d00 * d11 - d01 * d01
    if den == 0:
        return Vector(b1)
    v = (d11 * d20 - d01 * d21) / den
    w = (d00 * d21 - d01 * d20) / den
    u = 1 - v - w
    na = (a2 - a1).cross(a3 - a1)
    nb = (b2 - b1).cross(b3 - b1)
    h = (p - a1).dot(na.normalized()) if na.length else 0.0
    out = b1 * u + b2 * v + b3 * w
    if nb.length and na.length:
        out += nb.normalized() * h * math.sqrt(nb.length / na.length)
    return out


def interpolate_bezier(knot1, handle1, handle2, knot2, resolution):
    """`resolution` points along the cubic Bezier from knot1 to knot2."""
    k1, h1, h2, k2 = _v(knot1), _v(handle1), _v(handle2), _v(knot2)
    if resolution < 2:
        return [Vector(k1)]
    out = []
    for i in range(resolution):
        t = i / (resolution - 1)
        u = 1 - t
        out.append(k1 * (u * u * u) + h1 * (3 * u * u * t) + h2 * (3 * u * t * t) + k2 * (t * t * t))
    return out


def _project_2d(pts):
    n = normal(pts) if len(pts) >= 3 else Vector((0, 0, 1))
    ax = max(range(3), key=lambda i: abs(n[i]))
    i, j = [(1, 2), (2, 0), (0, 1)][ax]
    flip = n[ax] < 0
    out = [(p[i], p[j]) for p in pts]
    if flip:
        out = [(x, -y) for x, y in out]
    return out


def _ear_clip(poly2d, idx):
    """Ear clipping of one simple polygon; returns triangles of indices."""
    pts = list(idx)
    P = {k: poly2d[n] for n, k in enumerate(idx)}
    area = sum(P[pts[n]][0] * P[pts[(n + 1) % len(pts)]][1] - P[pts[(n + 1) % len(pts)]][0] * P[pts[n]][1] for n in range(len(pts)))
    if area < 0:
        pts.reverse()
    tris = []

    def cross(o, a, b):
        return (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])

    def inside(p, a, b, c):
        return cross(a, b, p) >= 0 and cross(b, c, p) >= 0 and cross(c, a, p) >= 0

    guard = 0
    while len(pts) > 3 and guard < 10000:
        guard += 1
        n = len(pts)
        for k in range(n):
            a, b, c = pts[k - 1], pts[k], pts[(k + 1) % n]
            if cross(P[a], P[b], P[c]) <= 1e-12:
                continue
            if any(inside(P[q], P[a], P[b], P[c]) for q in pts if q not in (a, b, c) and P[q] not in (P[a], P[b], P[c])):
                continue
            tris.append((a, b, c))
            pts.pop(k)
            break
        else:
            # Degenerate: fan the rest.
            for k in range(1, len(pts) - 1):
                tris.append((pts[0], pts[k], pts[k + 1]))
            return tris
    if len(pts) == 3:
        tris.append(tuple(pts))
    return tris


def tessellate_polygon(veclist_list):
    """Triangles (index triples into the concatenated point lists) filling
    each polyline of `veclist_list`."""
    tris = []
    base = 0
    for poly in veclist_list:
        pts = [_v3(p) for p in poly]
        n = len(pts)
        if n >= 3:
            p2 = _project_2d(pts)
            for t in _ear_clip(p2, list(range(base, base + n))):
                tris.append(t)
        base += n
    return tris


def convex_hull_2d(points):
    pts = sorted(set((float(p[0]), float(p[1]), i) for i, p in enumerate(points)))
    if len(pts) < 3:
        return [p[2] for p in pts]

    def cross(o, a, b):
        return (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
    lower, upper = [], []
    for p in pts:
        while len(lower) >= 2 and cross(lower[-2], lower[-1], p) <= 0:
            lower.pop()
        lower.append(p)
    for p in reversed(pts):
        while len(upper) >= 2 and cross(upper[-2], upper[-1], p) <= 0:
            upper.pop()
        upper.append(p)
    hull = lower[:-1] + upper[:-1]
    return [p[2] for p in hull]


def box_pack_2d(boxes):
    """Pack [x, y, w, h] boxes in place (shelf packing, tallest first);
    returns the (width, height) of the result."""
    order = sorted(range(len(boxes)), key=lambda i: -boxes[i][3])
    total = sum(b[2] * b[3] for b in boxes)
    side = math.sqrt(total) if total > 0 else 1.0
    x = y = shelf = 0.0
    w = h = 0.0
    for i in order:
        b = boxes[i]
        if x + b[2] > side and x > 0:
            x = 0.0
            y += shelf
            shelf = 0.0
        b[0], b[1] = x, y
        x += b[2]
        shelf = max(shelf, b[3])
        w = max(w, x)
        h = max(h, y + b[3])
    return w, h


def box_fit_2d(points):
    hull = [points[i] for i in convex_hull_2d(points)]
    best = (float('inf'), 0.0)
    for i in range(len(hull)):
        a, b = hull[i], hull[(i + 1) % len(hull)]
        ang = math.atan2(b[1] - a[1], b[0] - a[0])
        c, s = math.cos(-ang), math.sin(-ang)
        xs = [p[0] * c - p[1] * s for p in hull]
        ys = [p[0] * s + p[1] * c for p in hull]
        area = (max(xs) - min(xs)) * (max(ys) - min(ys))
        if area < best[0]:
            best = (area, -ang)
    return best[1]


def points_in_planes(planes, epsilon_coplanar=1e-4, epsilon_isect=1e-6):
    pts = []
    used = set()
    n = len(planes)
    for i in range(n):
        for j in range(i + 1, n):
            for k in range(j + 1, n):
                a, b, c = (Vector(planes[x][:3]) for x in (i, j, k))
                den = a.dot(b.cross(c))
                if abs(den) < 1e-12:
                    continue
                da, db, dc = (-planes[x][3] for x in (i, j, k))
                p = (b.cross(c) * da + c.cross(a) * db + a.cross(b) * dc) / den
                if all(Vector(pl[:3]).dot(p) + pl[3] <= epsilon_isect for pl in planes):
                    pts.append(p)
                    used.update((i, j, k))
    return pts, sorted(used)
