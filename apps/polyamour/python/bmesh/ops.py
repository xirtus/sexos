# SPDX-License-Identifier: GPL-3.0-only
"""bmesh.ops: the common BMesh operators, in pure Python. Each takes the
BMesh and keyword arguments and returns a dict of the new elements, as
Blender's do."""
import math

from mathutils import Matrix, Vector

from .types import BMEdge, BMFace, BMLoopUV, BMVert


def _verts(geom):
    return [g for g in geom if isinstance(g, BMVert)]


def _edges(geom):
    return [g for g in geom if isinstance(g, BMEdge)]


def _faces(geom):
    return [g for g in geom if isinstance(g, BMFace)]


def _m(matrix):
    return Matrix(matrix) if matrix is not None else Matrix.Identity(4)


def _space(space, fn):
    if space is None:
        return fn
    s = Matrix(space)
    si = s.inverted()
    return lambda co: si @ fn(s @ co)


# --- primitives --------------------------------------------------------------------------------

def _uv(bm, faces, uvs):
    lay = bm.loops.layers.uv.verify()
    for f, fuv in zip(faces, uvs):
        for l, uv in zip(f.loops, fuv):
            l[lay] = BMLoopUV(uv)


def create_cube(bm, size=1.0, matrix=None, calc_uvs=False):
    m = _m(matrix)
    h = size / 2
    co = [(-h, -h, -h), (-h, -h, h), (-h, h, -h), (-h, h, h), (h, -h, -h), (h, -h, h), (h, h, -h), (h, h, h)]
    vs = [bm.verts.new(m @ Vector(c)) for c in co]
    quads = [(0, 1, 3, 2), (2, 3, 7, 6), (6, 7, 5, 4), (4, 5, 1, 0), (2, 6, 4, 0), (7, 3, 1, 5)]
    faces = [bm.faces.new([vs[i] for i in q]) for q in quads]
    if calc_uvs:
        _uv(bm, faces, [[(0, 0), (1, 0), (1, 1), (0, 1)]] * 6)
    return {'verts': vs}


def create_grid(bm, x_segments=1, y_segments=1, size=1.0, matrix=None, calc_uvs=False):
    m = _m(matrix)
    nx, ny = max(1, int(x_segments)), max(1, int(y_segments))
    vs = []
    for j in range(ny + 1):
        for i in range(nx + 1):
            vs.append(bm.verts.new(m @ Vector((size * (2 * i / nx - 1), size * (2 * j / ny - 1), 0.0))))
    faces = []
    for j in range(ny):
        for i in range(nx):
            a = j * (nx + 1) + i
            faces.append(bm.faces.new([vs[a], vs[a + 1], vs[a + nx + 2], vs[a + nx + 1]]))
    if calc_uvs:
        _uv(bm, faces, [[(i / nx, j / ny), ((i + 1) / nx, j / ny), ((i + 1) / nx, (j + 1) / ny), (i / nx, (j + 1) / ny)]
                        for j in range(ny) for i in range(nx)])
    return {'verts': vs}


def create_uvsphere(bm, u_segments=32, v_segments=16, radius=1.0, matrix=None, calc_uvs=False, diameter=None):
    if diameter is not None:
        radius = diameter
    m = _m(matrix)
    u, v = max(3, int(u_segments)), max(3, int(v_segments))
    top = bm.verts.new(m @ Vector((0, 0, radius)))
    rings = []
    for j in range(1, v):
        th = math.pi * j / v
        ring = []
        for i in range(u):
            ph = 2 * math.pi * i / u
            ring.append(bm.verts.new(m @ Vector((radius * math.sin(th) * math.cos(ph), radius * math.sin(th) * math.sin(ph), radius * math.cos(th)))))
        rings.append(ring)
    bot = bm.verts.new(m @ Vector((0, 0, -radius)))
    for i in range(u):
        bm.faces.new([top, rings[0][i], rings[0][(i + 1) % u]])
    for j in range(len(rings) - 1):
        for i in range(u):
            a, b = rings[j], rings[j + 1]
            bm.faces.new([a[i], b[i], b[(i + 1) % u], a[(i + 1) % u]])
    for i in range(u):
        bm.faces.new([bot, rings[-1][(i + 1) % u], rings[-1][i]])
    return {'verts': [top] + [x for r in rings for x in r] + [bot]}


def create_icosphere(bm, subdivisions=1, radius=1.0, matrix=None, calc_uvs=False, diameter=None):
    if diameter is not None:
        radius = diameter
    m = _m(matrix)
    t = (1 + math.sqrt(5)) / 2
    pts = [(-1, t, 0), (1, t, 0), (-1, -t, 0), (1, -t, 0), (0, -1, t), (0, 1, t), (0, -1, -t), (0, 1, -t),
           (t, 0, -1), (t, 0, 1), (-t, 0, -1), (-t, 0, 1)]
    pts = [Vector(p).normalized() for p in pts]
    tris = [(0, 11, 5), (0, 5, 1), (0, 1, 7), (0, 7, 10), (0, 10, 11), (1, 5, 9), (5, 11, 4), (11, 10, 2), (10, 7, 6), (7, 1, 8),
            (3, 9, 4), (3, 4, 2), (3, 2, 6), (3, 6, 8), (3, 8, 9), (4, 9, 5), (2, 4, 11), (6, 2, 10), (8, 6, 7), (9, 8, 1)]
    for _ in range(max(0, int(subdivisions) - 1)):
        cache = {}

        def mid(a, b):
            k = (min(a, b), max(a, b))
            if k not in cache:
                pts.append(((pts[a] + pts[b]) / 2).normalized())
                cache[k] = len(pts) - 1
            return cache[k]
        new = []
        for a, b, c in tris:
            ab, bc, ca = mid(a, b), mid(b, c), mid(c, a)
            new += [(a, ab, ca), (b, bc, ab), (c, ca, bc), (ab, bc, ca)]
        tris = new
    vs = [bm.verts.new(m @ (p * radius)) for p in pts]
    for a, b, c in tris:
        bm.faces.new([vs[a], vs[b], vs[c]])
    return {'verts': vs}


def create_circle(bm, cap_ends=False, cap_tris=False, segments=32, radius=1.0, matrix=None, calc_uvs=False, diameter=None):
    if diameter is not None:
        radius = diameter
    m = _m(matrix)
    n = max(3, int(segments))
    vs = [bm.verts.new(m @ Vector((radius * math.cos(2 * math.pi * i / n), radius * math.sin(2 * math.pi * i / n), 0))) for i in range(n)]
    if cap_ends:
        if cap_tris:
            c = bm.verts.new(m @ Vector((0, 0, 0)))
            for i in range(n):
                bm.faces.new([c, vs[i], vs[(i + 1) % n]])
            vs.append(c)
        else:
            bm.faces.new(vs)
    else:
        for i in range(n):
            bm.edges.new((vs[i], vs[(i + 1) % n]))
    return {'verts': vs}


def create_cone(bm, cap_ends=True, cap_tris=False, segments=32, radius1=1.0, radius2=0.0, depth=2.0, matrix=None, calc_uvs=False,
                diameter1=None, diameter2=None):
    if diameter1 is not None:
        radius1 = diameter1
    if diameter2 is not None:
        radius2 = diameter2
    m = _m(matrix)
    n = max(3, int(segments))
    h = depth / 2

    def ring(r, z):
        if r <= 0:
            return [bm.verts.new(m @ Vector((0, 0, z)))]
        return [bm.verts.new(m @ Vector((r * math.cos(2 * math.pi * i / n), r * math.sin(2 * math.pi * i / n), z))) for i in range(n)]
    lo, hi = ring(radius1, -h), ring(radius2, h)
    for i in range(n):
        j = (i + 1) % n
        if len(lo) == 1:
            bm.faces.new([lo[0], hi[j], hi[i]])
        elif len(hi) == 1:
            bm.faces.new([lo[i], lo[j], hi[0]])
        else:
            bm.faces.new([lo[i], lo[j], hi[j], hi[i]])
    if cap_ends:
        if len(lo) > 1:
            bm.faces.new(list(reversed(lo)))
        if len(hi) > 1:
            bm.faces.new(hi)
    return {'verts': lo + hi}


def create_vert(bm, co=(0, 0, 0)):
    return {'vert': [bm.verts.new(co)]}


# --- transforms ---------------------------------------------------------------------------------

def translate(bm, vec=(0, 0, 0), space=None, verts=(), use_shapekey=False):
    d = Vector(vec)
    f = _space(space, lambda co: co + d)
    for v in verts:
        v.co = f(v.co)
    return {}


def scale(bm, vec=(1, 1, 1), space=None, verts=(), use_shapekey=False):
    s = Vector(vec)
    f = _space(space, lambda co: Vector((co.x * s.x, co.y * s.y, co.z * s.z)))
    for v in verts:
        v.co = f(v.co)
    return {}


def rotate(bm, cent=(0, 0, 0), matrix=None, verts=(), space=None, use_shapekey=False):
    c = Vector(cent)
    r = _m(matrix).to_3x3()
    f = _space(space, lambda co: r @ (co - c) + c)
    for v in verts:
        v.co = f(v.co)
    return {}


def transform(bm, matrix=None, space=None, verts=(), use_shapekey=False):
    mm = _m(matrix)
    f = _space(space, lambda co: mm @ co)
    for v in verts:
        v.co = f(v.co)
    return {}


def smooth_vert(bm, verts=(), factor=0.5, mirror_clip_x=False, mirror_clip_y=False, mirror_clip_z=False, clip_dist=0.0,
                use_axis_x=True, use_axis_y=True, use_axis_z=True):
    new = {}
    for v in verts:
        nb = [e.other_vert(v).co for e in v.link_edges]
        if not nb:
            continue
        avg = sum(nb, Vector((0, 0, 0))) / len(nb)
        t = v.co.lerp(avg, factor)
        new[v] = Vector((t.x if use_axis_x else v.co.x, t.y if use_axis_y else v.co.y, t.z if use_axis_z else v.co.z))
    for v, c in new.items():
        v.co = c
    return {}


# --- topology -----------------------------------------------------------------------------------

def reverse_faces(bm, faces=(), flip_multires=False):
    for f in faces:
        f.normal_flip()
    return {}


def recalc_face_normals(bm, faces=()):
    faces = list(faces)
    fset = set(map(id, faces))
    seen = set()
    for start in faces:
        if id(start) in seen:
            continue
        comp = [start]
        seen.add(id(start))
        stack = [start]
        while stack:
            f = stack.pop()
            for l in f.loops:
                for o in l.edge.link_loops:
                    g = o.face
                    if id(g) in seen or id(g) not in fset:
                        continue
                    # neighbours walk the shared edge in opposite directions
                    if o.vert is l.vert:
                        g.normal_flip()
                    seen.add(id(g))
                    comp.append(g)
                    stack.append(g)
        vol = 0.0
        for f in comp:
            vs = [v.co for v in f.verts]
            for i in range(1, len(vs) - 1):
                vol += vs[0].dot(vs[i].cross(vs[i + 1]))
        if vol < 0:
            for f in comp:
                f.normal_flip()
        for f in comp:
            f.normal_update()
    return {}


def triangulate(bm, faces=(), quad_method='BEAUTY', ngon_method='BEAUTY'):
    from mathutils.geometry import tessellate_polygon
    out, fmap = [], {}
    for f in list(faces):
        vs = f.verts
        if len(vs) == 3:
            continue
        if len(vs) == 4:
            a, b, c, d = vs
            if quad_method in ('BEAUTY', 'SHORT_EDGE') and (a.co - c.co).length > (b.co - d.co).length:
                tris = [(b, c, d), (b, d, a)]
            else:
                tris = [(a, b, c), (a, c, d)]
        else:
            idx = tessellate_polygon([[v.co for v in vs]])
            tris = [(vs[i], vs[j], vs[k]) for i, j, k in idx]
        mat, smooth, sel = f.material_index, f.smooth, f.select
        bm.faces.remove(f)
        for t in tris:
            nf = bm.faces.new(t)
            nf.material_index, nf.smooth, nf.select = mat, smooth, sel
            out.append(nf)
            fmap[nf] = f
    return {'faces': out, 'edges': [], 'face_map': fmap, 'face_map_double': {}}


def remove_doubles(bm, verts=(), dist=0.0001, use_connected=False, use_centroid=False):
    verts = [v for v in verts if v.is_valid]
    target = {}
    for i, v in enumerate(verts):
        if v in target:
            continue
        for w in verts[i + 1:]:
            if w not in target and (w.co - v.co).length <= dist:
                target[w] = v
    weld_verts(bm, targetmap=target)
    return {}


def weld_verts(bm, targetmap=None):
    targetmap = targetmap or {}
    if not targetmap:
        return {}
    for f in list(bm.faces):
        vs = [targetmap.get(v, v) for v in f.verts]
        if not any(v in targetmap for v in f.verts):
            continue
        clean = []
        for v in vs:
            if not clean or clean[-1] is not v:
                clean.append(v)
        if len(clean) > 1 and clean[0] is clean[-1]:
            clean.pop()
        mat, smooth, sel = f.material_index, f.smooth, f.select
        bm.faces.remove(f)
        if len(clean) >= 3 and len(set(map(id, clean))) == len(clean) and bm.faces.get(clean) is None:
            nf = bm.faces.new(clean)
            nf.material_index, nf.smooth, nf.select = mat, smooth, sel
    for e in list(bm.edges):
        if not e.is_valid:
            continue
        a, b = (targetmap.get(v, v) for v in e.verts)
        if a is not e.verts[0] or b is not e.verts[1]:
            bm.edges.remove(e)
            if a is not b and bm.edges.get((a, b)) is None:
                bm.edges.new((a, b))
    for v in list(targetmap):
        if v.is_valid:
            bm.verts.remove(v)
    return {}


def delete(bm, geom=(), context='VERTS'):
    geom = list(geom)
    if context == 'VERTS':
        for v in _verts(geom):
            if v.is_valid:
                bm.verts.remove(v)
    elif context == 'EDGES':
        es = _edges(geom)
        for e in es:
            if e.is_valid:
                bm.edges.remove(e)
        for v in set(v for e in es for v in e.verts):
            if v.is_valid and not v.link_edges:
                bm.verts.remove(v)
    elif context in ('FACES', 'FACES_KEEP_BOUNDARY'):
        fs = _faces(geom)
        es = set(e for f in fs for e in f.edges)
        vs = set(v for f in fs for v in f.verts)
        for f in fs:
            if f.is_valid:
                bm.faces.remove(f)
        if context == 'FACES':
            for e in es:
                if e.is_valid and not e.link_loops:
                    bm.edges.remove(e)
            for v in vs:
                if v.is_valid and not v.link_edges:
                    bm.verts.remove(v)
    elif context == 'FACES_ONLY':
        for f in _faces(geom):
            if f.is_valid:
                bm.faces.remove(f)
    elif context == 'EDGES_FACES':
        for e in _edges(geom):
            if e.is_valid:
                bm.edges.remove(e)
        for f in _faces(geom):
            if f.is_valid:
                bm.faces.remove(f)
    elif context == 'TAGGED_ONLY':
        for g in geom:
            if g.is_valid and g.tag:
                {BMVert: bm.verts, BMEdge: bm.edges, BMFace: bm.faces}[type(g)].remove(g)
    return {}


def duplicate(bm, geom=(), dest=None, use_select_history=False, use_edge_flip_from_face=False):
    dest = dest or bm
    geom = list(geom)
    vs = set(_verts(geom))
    for e in _edges(geom):
        vs.update(e.verts)
    for f in _faces(geom):
        vs.update(f.verts)
    vmap = {}
    for v in [v for v in bm.verts if v in vs]:
        nv = dest.verts.new(v.co, v)
        vmap[v] = nv
    out_e, out_f = [], []
    emap, fmap = {}, {}
    for f in _faces(geom):
        nf = dest.faces.new([vmap[v] for v in f.verts], f)
        for a, b in zip(f.loops, nf.loops):
            b._data = {k: (BMLoopUV(x.uv) if isinstance(x, BMLoopUV) else x) for k, x in a._data.items()}
        out_f.append(nf)
        fmap[f] = nf
    for e in _edges(geom):
        a, b = vmap[e.verts[0]], vmap[e.verts[1]]
        ne = dest.edges.get((a, b)) or dest.edges.new((a, b), e)
        out_e.append(ne)
        emap[e] = ne
    return {'geom': list(vmap.values()) + out_e + out_f, 'vert_map': vmap, 'edge_map': emap, 'face_map': fmap,
            'boundary_map': {}, 'isovert_map': {}, 'geom_orig': geom}


def contextual_create(bm, geom=(), mat_nr=0, use_smooth=False):
    vs = _verts(geom)
    es = _edges(geom)
    if len(vs) == 2 and not es:
        e = bm.edges.get(vs) or bm.edges.new(vs)
        return {'faces': [], 'edges': [e]}
    if not vs and es:
        order = [es[0].verts[0], es[0].verts[1]]
        rest = es[1:]
        while rest:
            for e in rest:
                if order[-1] in e.verts:
                    nxt = e.other_vert(order[-1])
                    rest.remove(e)
                    if nxt is not order[0]:
                        order.append(nxt)
                    break
            else:
                break
        vs = order
    if len(vs) >= 3:
        f = bm.faces.get(vs) or bm.faces.new(vs)
        f.material_index, f.smooth = mat_nr, use_smooth
        return {'faces': [f], 'edges': f.edges}
    return {'faces': [], 'edges': []}


def edgeloop_fill(bm, edges=(), mat_nr=0, use_smooth=False):
    return contextual_create(bm, geom=list(edges), mat_nr=mat_nr, use_smooth=use_smooth)


def holes_fill(bm, edges=(), sides=0):
    bound = [e for e in edges if e.is_boundary]
    made = []
    used = set()
    for e in bound:
        if e in used:
            continue
        loop = [e]
        used.add(e)
        v = e.verts[1]
        start = e.verts[0]
        ok = True
        while v is not start:
            nxt = [x for x in v.link_edges if x.is_boundary and x not in used and x in bound]
            if not nxt:
                ok = False
                break
            used.add(nxt[0])
            loop.append(nxt[0])
            v = nxt[0].other_vert(v)
        if ok and (not sides or len(loop) <= sides):
            r = contextual_create(bm, geom=loop)
            made += r['faces']
    for f in made:
        recalc_face_normals(bm, faces=[f] + [g for e in f.edges for g in e.link_faces])
    return {'faces': made}


def poke(bm, faces=(), offset=0.0, center_mode='MEAN_WEIGHTED', use_relative_offset=False):
    out_v, out_f = [], []
    for f in list(faces):
        c = f.calc_center_median() + f.normal * offset
        vs, mat, smooth = f.verts, f.material_index, f.smooth
        bm.faces.remove(f)
        cv = bm.verts.new(c)
        out_v.append(cv)
        for i in range(len(vs)):
            nf = bm.faces.new([vs[i], vs[(i + 1) % len(vs)], cv])
            nf.material_index, nf.smooth = mat, smooth
            out_f.append(nf)
    return {'verts': out_v, 'faces': out_f}


# --- extrusion -----------------------------------------------------------------------------------

def extrude_face_region(bm, geom=(), edges_exclude=(), use_keep_orig=False, use_normal_flip=False, use_normal_from_adjacent=False,
                        use_dissolve_ortho_edges=False, use_select_history=False, skip_input_flip=False):
    faces = _faces(geom)
    fset = set(faces)
    if not faces:
        es = _edges(geom)
        if es:
            return extrude_edge_only(bm, edges=es)
        return extrude_vert_indiv(bm, verts=_verts(geom))
    vmap = {}
    for f in faces:
        for v in f.verts:
            if v not in vmap:
                vmap[v] = bm.verts.new(v.co, v)
    # Boundary edges of the region get side faces.
    side = []
    for f in faces:
        for l in f.loops:
            e = l.edge
            inside = [x for x in e.link_faces if x in fset]
            if len(inside) == 1:
                a, b = l.vert, l.link_loop_next.vert
                side.append((a, b, f))
    new_faces = []
    for f in faces:
        nf = bm.faces.new([vmap[v] for v in f.verts], f)
        for a, b in zip(f.loops, nf.loops):
            b._data = {k: (BMLoopUV(x.uv) if isinstance(x, BMLoopUV) else x) for k, x in a._data.items()}
        new_faces.append(nf)
    sides = []
    for a, b, f in side:
        sf = bm.faces.new([b, a, vmap[a], vmap[b]], f)
        sides.append(sf)
    if not use_keep_orig:
        for f in faces:
            bm.faces.remove(f)
        for v in list(vmap):
            if v.is_valid and not v.link_edges:
                bm.verts.remove(v)
    new_edges = []
    for nf in new_faces:
        for e in nf.edges:
            if e not in new_edges:
                new_edges.append(e)
    for sf in sides:
        for e in sf.edges:
            if e not in new_edges and all(v in vmap.values() or v in vmap for v in e.verts):
                new_edges.append(e)
    if use_normal_flip:
        for f in new_faces:
            f.normal_flip()
    return {'geom': list(vmap.values()) + new_edges + new_faces}


def extrude_discrete_faces(bm, faces=(), use_normal_flip=False, use_select_history=False):
    out = []
    for f in list(faces):
        r = extrude_face_region(bm, geom=[f], use_normal_flip=use_normal_flip)
        out += _faces(r['geom'])
    return {'faces': out}


def extrude_edge_only(bm, edges=(), use_normal_flip=False, use_select_history=False):
    vmap = {}
    for e in edges:
        for v in e.verts:
            if v not in vmap:
                vmap[v] = bm.verts.new(v.co, v)
    out_e, out_f = [], []
    for e in edges:
        a, b = e.verts
        if e.link_loops:
            l = e.link_loops[0]
            if l.vert is a:
                a, b = b, a
        f = bm.faces.new([a, b, vmap[b], vmap[a]])
        out_f.append(f)
        out_e.append(bm.edges.get((vmap[a], vmap[b])))
    return {'geom': list(vmap.values()) + out_e + out_f}


def extrude_vert_indiv(bm, verts=(), use_select_history=False):
    out_v, out_e = [], []
    for v in verts:
        nv = bm.verts.new(v.co, v)
        out_v.append(nv)
        out_e.append(bm.edges.new((v, nv)))
    return {'verts': out_v, 'edges': out_e}


def inset_individual(bm, faces=(), thickness=0.0, depth=0.0, use_even_offset=True, use_interpolate=True, use_relative_offset=False):
    out = []
    for f in list(faces):
        c = f.calc_center_median()
        n = f.normal.copy()
        vs = f.verts
        inner = []
        for v in vs:
            d = c - v.co
            t = min(1.0, thickness / d.length) if d.length > 0 else 0.0
            inner.append(bm.verts.new(v.co.lerp(c, t) + n * depth))
        mat, smooth = f.material_index, f.smooth
        bm.faces.remove(f)
        k = len(vs)
        for i in range(k):
            sf = bm.faces.new([vs[i], vs[(i + 1) % k], inner[(i + 1) % k], inner[i]])
            sf.material_index, sf.smooth = mat, smooth
            out.append(sf)
        cf = bm.faces.new(inner)
        cf.material_index, cf.smooth = mat, smooth
        out.append(cf)
    return {'faces': out}


def inset_region(bm, faces=(), faces_exclude=(), use_boundary=True, use_even_offset=True, use_interpolate=True,
                 use_relative_offset=False, use_edge_rail=False, thickness=0.0, depth=0.0, use_outset=False):
    return inset_individual(bm, faces=faces, thickness=thickness, depth=depth)


# --- subdivision ------------------------------------------------------------------------------------

def subdivide_edges(bm, edges=(), smooth=0.0, smooth_falloff='SMOOTH', fractal=0.0, along_normal=0.0, cuts=1, seed=0,
                    custom_patterns=None, edge_percents=None, quad_corner_type='STRAIGHT_CUT', use_grid_fill=False,
                    use_single_edge=False, use_only_quads=False, use_sphere=False, use_smooth_even=False):
    cuts = max(1, int(cuts))
    edges = [e for e in edges if e.is_valid]
    eset = set(edges)
    split = {}   # edge -> list of new verts from verts[0] to verts[1]
    new_verts = []
    for e in edges:
        a, b = e.verts
        pts = [bm.verts.new(a.co.lerp(b.co, (k + 1) / (cuts + 1)), a) for k in range(cuts)]
        if use_sphere:
            r = (a.co.length + b.co.length) / 2
            for p in pts:
                p.co = p.co.normalized() * r
        split[e] = (a, pts)
        new_verts += pts

    def edge_points(u, v):
        e = bm.edges.get((u, v))
        if e not in split:
            return []
        a, pts = split[e]
        return pts if a is u else list(reversed(pts))
    inner_faces, new_faces = [], []
    for f in list(bm.faces):
        fe = f.edges
        if not any(e in split for e in fe):
            continue
        vs = f.verts
        n = len(vs)
        mat, smooth_f, sel = f.material_index, f.smooth, f.select
        full = all(e in eset for e in fe)
        made = []
        if full and use_grid_fill and n == 4:
            a, b, c, d = vs
            ab, bc, dc, ad = edge_points(a, b), edge_points(b, c), edge_points(d, c), edge_points(a, d)
            grid = []
            m = cuts + 2
            for j in range(m):
                row = []
                for i in range(m):
                    if j == 0:
                        row.append(([a] + ab + [b])[i])
                    elif j == m - 1:
                        row.append(([d] + dc + [c])[i])
                    elif i == 0:
                        row.append(ad[j - 1])
                    elif i == m - 1:
                        row.append(bc[j - 1])
                    else:
                        p0 = ad[j - 1].co
                        p1 = bc[j - 1].co
                        nv = bm.verts.new(p0.lerp(p1, i / (m - 1)))
                        new_verts.append(nv)
                        row.append(nv)
                grid.append(row)
            bm.faces.remove(f)
            for j in range(m - 1):
                for i in range(m - 1):
                    made.append(bm.faces.new([grid[j][i], grid[j][i + 1], grid[j + 1][i + 1], grid[j + 1][i]]))
            inner_faces += made
        elif full and use_grid_fill and n == 3:
            a, b, c = vs
            m = cuts + 1
            rows = [[a]]
            ab, ac = edge_points(a, b), edge_points(a, c)
            bc = edge_points(b, c)
            for j in range(1, m + 1):
                if j == m:
                    row = [b] + bc + [c]
                else:
                    l, r = ab[j - 1], ac[j - 1]
                    row = [l]
                    for i in range(1, j):
                        nv = bm.verts.new(l.co.lerp(r.co, i / j))
                        new_verts.append(nv)
                        row.append(nv)
                    row.append(r)
                rows.append(row)
            bm.faces.remove(f)
            for j in range(m):
                top, bot = rows[j], rows[j + 1]
                for i in range(len(top)):
                    made.append(bm.faces.new([top[i], bot[i], bot[i + 1]]))
                    if i + 1 < len(top):
                        made.append(bm.faces.new([top[i], bot[i + 1], top[i + 1]]))
            inner_faces += made
        else:
            ring = []
            for i, v in enumerate(vs):
                ring.append(v)
                ring += edge_points(v, vs[(i + 1) % n])
            bm.faces.remove(f)
            made.append(bm.faces.new(ring))
        for nf in made:
            nf.material_index, nf.smooth, nf.select = mat, smooth_f, sel
        new_faces += made
    for e in edges:
        if e.is_valid:
            a, pts = split[e]
            chain = [a] + pts + [e.other_vert(a)]
            bm.edges.remove(e) if not e.link_loops else None
            for u, v in zip(chain, chain[1:]):
                if bm.edges.get((u, v)) is None:
                    bm.edges.new((u, v))
    for e in [e for e in edges if e.is_valid and any(bm.edges.get((e.verts[0], p)) for p in split[e][1][:1])]:
        if not e.link_loops:
            bm.edges.remove(e)
    if smooth:
        smooth_vert(bm, verts=new_verts, factor=smooth)
    return {'geom_inner': new_verts + inner_faces, 'geom_split': new_verts, 'geom': new_verts + new_faces}


def subdivide_edgering(bm, edges=(), interp_mode='LINEAR', smooth=0.0, cuts=1, profile_shape='SMOOTH', profile_shape_factor=0.0):
    return subdivide_edges(bm, edges=edges, cuts=cuts, use_grid_fill=True)


def unsubdivide(bm, verts=(), iterations=2):
    return {}


# --- spin, mirror ------------------------------------------------------------------------------------

def spin(bm, geom=(), cent=(0, 0, 0), axis=(0, 0, 1), dvec=(0, 0, 0), angle=0.0, space=None, steps=0, use_merge=False,
         use_normal_flip=False, use_duplicate=False):
    steps = max(1, int(steps))
    c = Vector(cent)
    ax = Vector(axis).normalized()
    d = Vector(dvec)
    cur = list(geom)
    last = cur
    for k in range(steps):
        rot = Matrix.Rotation(angle / steps, 3, ax)
        if use_duplicate:
            r = duplicate(bm, geom=cur)
            new = r['geom']
        else:
            r = extrude_face_region(bm, geom=cur)
            new = r['geom']
        for v in _verts(new):
            v.co = rot @ (v.co - c) + c + d
        last = [g for g in new if not isinstance(g, BMVert) or True]
        cur = [g for g in new if isinstance(g, (BMVert, BMEdge)) or isinstance(g, BMFace)]
        if not _faces(cur):
            cur = _edges(new) or _verts(new)
    if use_merge:
        remove_doubles(bm, verts=list(bm.verts), dist=1e-5)
    return {'geom_last': [g for g in last if g.is_valid]}


def mirror(bm, geom=(), matrix=None, merge_dist=0.0, axis='X', mirror_u=False, mirror_v=False, mirror_udim=False, use_shapekey=False):
    k = {'X': 0, 'Y': 1, 'Z': 2}[axis]
    r = duplicate(bm, geom=geom)
    mm = _m(matrix)
    mi = mm.inverted()
    for v in _verts(r['geom']):
        co = mi @ v.co
        co[k] = -co[k]
        v.co = mm @ co
    for f in _faces(r['geom']):
        f.normal_flip()
    if merge_dist > 0:
        remove_doubles(bm, verts=list(bm.verts), dist=merge_dist)
    return {'geom': [g for g in r['geom'] if g.is_valid]}


def dissolve_verts(bm, verts=(), use_face_split=False, use_boundary_tear=False):
    for v in list(verts):
        if not v.is_valid:
            continue
        fs = v.link_faces
        if len(fs) >= 2 and not v.is_boundary:
            dissolve_faces(bm, faces=fs)
            for f in list(v.link_faces):
                vs = [x for x in f.verts if x is not v]
                mat = f.material_index
                bm.faces.remove(f)
                if len(vs) >= 3:
                    bm.faces.new(vs).material_index = mat
        if v.is_valid:
            bm.verts.remove(v)
    return {}


def dissolve_faces(bm, faces=(), use_verts=False):
    faces = list(faces)
    if len(faces) < 2:
        return {'region': faces}
    fset = set(faces)
    boundary = []
    for f in faces:
        for l in f.loops:
            if len([x for x in l.edge.link_faces if x in fset]) == 1:
                boundary.append((l.vert, l.link_loop_next.vert))
    nxt = {a: b for a, b in boundary}
    if not boundary:
        return {'region': []}
    start = boundary[0][0]
    ring = [start]
    v = nxt[start]
    while v is not start and v in nxt and len(ring) <= len(boundary):
        ring.append(v)
        v = nxt[v]
    mat = faces[0].material_index
    inner = set(v for f in faces for v in f.verts) - set(ring)
    for f in faces:
        bm.faces.remove(f)
    for e in list(bm.edges):
        if not e.link_loops and (e.verts[0] in inner or e.verts[1] in inner or (e.verts[0] in ring and e.verts[1] in ring and nxt.get(e.verts[0]) is not e.verts[1] and nxt.get(e.verts[1]) is not e.verts[0])):
            bm.edges.remove(e)
    for v in inner:
        if v.is_valid and not v.link_edges:
            bm.verts.remove(v)
    nf = bm.faces.new(ring)
    nf.material_index = mat
    return {'region': [nf]}


def bevel(bm, geom=(), offset=0.0, offset_type='OFFSET', profile_type='SUPERELLIPSE', segments=1, profile=0.5, affect='EDGES',
          clamp_overlap=False, material=-1, loop_slide=True, mark_seam=False, mark_sharp=False, harden_normals=False,
          face_strength_mode='NONE', miter_outer='SHARP', miter_inner='SHARP', spread=0.1, custom_profile=None,
          vmesh_method='ADJ'):
    """Runs Polyamour's own bevel on a temporary copy (needs a live Polyamour)."""
    from ._remote import run_on_copy
    edges = set(_edges(geom))
    verts = set(_verts(geom))
    for v in bm.verts:
        v.select = v in verts or any(e in edges for e in v.link_edges)
    for e in bm.edges:
        e.select = e in edges
    for f in bm.faces:
        f.select = False
    run_on_copy(bm, 'mesh.bevel', select_mode='VERT' if affect == 'VERTICES' else 'EDGE', offset=offset, segments=int(segments),
                profile=profile, affect=affect)
    return {'faces': [f for f in bm.faces if f.select], 'edges': [e for e in bm.edges if e.select], 'verts': [v for v in bm.verts if v.select]}


def convex_hull(bm, input=(), use_existing_faces=False):
    raise NotImplementedError('bmesh.ops.convex_hull is not supported yet')


def bridge_loops(bm, edges=(), use_pairs=False, use_cyclic=False, use_merge=False, merge_factor=0.0, twist_offset=0):
    raise NotImplementedError('bmesh.ops.bridge_loops is not supported yet')


def connect_verts(bm, verts=(), faces_exclude=(), check_degenerate=False):
    raise NotImplementedError('bmesh.ops.connect_verts is not supported yet')


def beautify_fill(bm, faces=(), edges=(), use_restrict_tag=False, method='AREA'):
    return {'geom': []}


def dissolve_limited(bm, angle_limit=0.0872665, use_dissolve_boundaries=False, verts=(), edges=(), delimit=set()):
    return {'region': []}


def dissolve_edges(bm, edges=(), use_verts=False, use_face_split=False):
    for e in list(edges):
        fs = e.link_faces
        if len(fs) == 2:
            dissolve_faces(bm, faces=fs)
        elif e.is_valid and not fs:
            bm.edges.remove(e)
    return {'region': []}


def split_edges(bm, edges=(), verts=(), use_verts=False):
    return {'edges': list(edges)}


def collapse(bm, edges=(), uvs=False):
    for e in list(edges):
        if not e.is_valid:
            continue
        a, b = e.verts
        mid = (a.co + b.co) / 2
        a.co = mid
        weld_verts(bm, targetmap={b: a})
    return {}


def pointmerge(bm, verts=(), merge_co=(0, 0, 0)):
    verts = [v for v in verts if v.is_valid]
    if not verts:
        return {}
    keep = verts[0]
    keep.co = Vector(merge_co)
    weld_verts(bm, targetmap={v: keep for v in verts[1:]})
    return {}


def join_triangles(bm, faces=(), cmp_seam=False, cmp_sharp=False, cmp_uvs=False, cmp_vcols=False, cmp_materials=False,
                   angle_face_threshold=0.698132, angle_shape_threshold=0.698132):
    tris = [f for f in faces if f.is_valid and len(f.loops) == 3]
    done = set()
    out = []
    for f in tris:
        if f in done or not f.is_valid:
            continue
        for e in f.edges:
            other = [g for g in e.link_faces if g is not f and g in tris and g not in done and g.is_valid]
            if other and f.normal.angle(other[0].normal, 0.0) <= angle_face_threshold:
                g = other[0]
                r = dissolve_faces(bm, faces=[f, g])
                done.update((f, g))
                out += r['region']
                break
    return {'faces': out}
