# SPDX-License-Identifier: GPL-3.0-only
"""bpy.types.Mesh and its element views (vertices, edges, polygons, loops,
loop triangles, UV layers, attributes, shape keys).

A mesh's arrays are fetched once (`dump_mesh`) and kept here; element writes
(`v.co.z += 1`, `p.use_smooth = True`, `uv.uv = ...`) change the local copy
and mark it dirty, and the whole change goes to Polyamour in one request
before the next request of any kind (or at the end of the script call), so a
loop over thousands of vertices costs one round trip."""
import base64
import math
import struct

from mathutils import Vector

from . import polyops as P
from ._base import ID, Collection_, bpy_struct, first, generation, mark_dirty, req


def _f32(text):
    if not text:
        return []
    raw = base64.b64decode(text)
    return list(struct.unpack('<%df' % (len(raw) // 4), raw))


def _u32(text):
    if not text:
        return []
    raw = base64.b64decode(text)
    return list(struct.unpack('<%dI' % (len(raw) // 4), raw))


class _Data:
    """The local copy of one mesh."""

    def __init__(self, j):
        self.gen = generation()
        self.dirty = set()
        self.dirty_keys = set()
        flat = _f32(j['verts'])
        self.co = [flat[i:i + 3] for i in range(0, len(flat), 3)]
        self.totals = _u32(j['loop_totals'])
        self.loops = _u32(j['loops'])
        self.starts = []
        s = 0
        for t in self.totals:
            self.starts.append(s)
            s += t
        e = _u32(j['edges'])
        self.edges = [(e[i], e[i + 1]) for i in range(0, len(e), 2)]
        self.smooth = [c == '1' for c in j.get('smooth', '')]
        self.mat = list(j.get('mat') or [0] * len(self.totals))
        uv = j.get('uvs')
        self.uvs = None
        if uv:
            f = _f32(uv)
            self.uvs = [f[i:i + 2] for i in range(0, len(f), 2)]
        self.seams = set(tuple(sorted(x)) for x in j.get('seams') or [])
        self.sharp = set(tuple(sorted(x)) for x in j.get('sharp') or [])
        self.groups = list(j.get('vertex_groups') or [])
        self.weights = {}
        w = _f32(j.get('weights'))
        for i in range(0, len(w), 3):
            self.weights.setdefault(int(w[i]), {})[int(w[i + 1])] = w[i + 2]
        self.keys = []
        for k in j.get('shape_keys') or []:
            kk = dict(k)
            f = _f32(k['co'])
            kk['co'] = [f[i:i + 3] for i in range(0, len(f), 3)]
            self.keys.append(kk)
        self.use_relative = j.get('use_relative', True)
        self.eval_time = j.get('eval_time', 0.0)
        self.sel_v = set(j.get('sel_v') or [])
        self.sel_e = set(tuple(sorted(x)) for x in j.get('sel_e') or [])
        self.sel_f = set(j.get('sel_f') or [])
        self.users = list(j.get('users') or [])
        self.is_editmode = bool(j.get('is_editmode'))
        self._derived = None

    # derived arrays (edge lookup, normals, triangles)
    def derived(self):
        if self._derived is None:
            d = {}
            d['edge_index'] = {tuple(sorted(e)): i for i, e in enumerate(self.edges)}
            fn, fc, fa = [], [], []
            for p in range(len(self.totals)):
                vs = self.face(p)
                nx = ny = nz = 0.0
                cx = cy = cz = 0.0
                for k, a in enumerate(vs):
                    b = vs[(k + 1) % len(vs)]
                    pa, pb = self.co[a], self.co[b]
                    nx += (pa[1] - pb[1]) * (pa[2] + pb[2])
                    ny += (pa[2] - pb[2]) * (pa[0] + pb[0])
                    nz += (pa[0] - pb[0]) * (pa[1] + pb[1])
                    cx += pa[0]
                    cy += pa[1]
                    cz += pa[2]
                ln = math.sqrt(nx * nx + ny * ny + nz * nz)
                n = len(vs) or 1
                fa.append(ln / 2.0)
                fn.append((nx / ln, ny / ln, nz / ln) if ln > 0 else (0.0, 0.0, 0.0))
                fc.append((cx / n, cy / n, cz / n))
            vn = [[0.0, 0.0, 0.0] for _ in self.co]
            for p in range(len(self.totals)):
                for v in self.face(p):
                    for k in range(3):
                        vn[v][k] += fn[p][k] * max(fa[p], 1e-12)
            for v in vn:
                ln = math.sqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2])
                if ln > 0:
                    v[0] /= ln
                    v[1] /= ln
                    v[2] /= ln
                else:
                    v[2] = 1.0
            d['fn'], d['fc'], d['fa'], d['vn'] = fn, fc, fa, vn
            self._derived = d
        return self._derived

    def face(self, p):
        s = self.starts[p]
        return self.loops[s:s + self.totals[p]]

    def touch(self, flag):
        self.dirty.add(flag)
        if flag in ('co', 'topo'):
            self._derived = None

    def faces(self):
        return [self.face(p) for p in range(len(self.totals))]


_cache = {}


class Mesh(ID):
    _kind = 'mesh'
    _collection_name = 'meshes'

    # --- the local copy
    def _d(self):
        c = _cache.get(self._name)
        if c is not None and (c.dirty or c.dirty_keys or c.gen == generation()):
            return c
        if not P.get().live:
            # Batch: the mesh as it was when the script started (or empty
            # for one made by the script), then the script's own edits.
            if c is None:
                try:
                    c = _Data(req('dump_mesh', self._name))
                except P.PolyopsError:
                    c = _Data({'verts': '', 'loop_totals': '', 'loops': '', 'edges': ''})
            c.gen = generation()
            _cache[self._name] = c
            return c
        c = _Data(req('dump_mesh', self._name))
        _cache[self._name] = c
        return c

    def _touch(self, flag):
        d = self._d()
        d.touch(flag)
        mark_dirty(('mesh', self._name), self)

    def _flush(self):
        d = _cache.get(self._name)
        if d is None:
            return
        name = self._name
        if 'topo' in d.dirty:
            opts = P.mesh_options(d.co, d.edges, d.faces())
            if d.uvs is not None and len(d.uvs) == len(d.loops):
                opts['uvs'] = P.b64_floats([c for uv in d.uvs for c in uv])
            if d.totals:
                opts['smooth'] = P.Raw(''.join('1' if s else '0' for s in d.smooth))
                opts['mat'] = [int(m) for m in d.mat]
            req('mesh_set', name, **opts)
            d.dirty.discard('co')
            d.dirty.discard('attr')
        if 'co' in d.dirty:
            req('mesh_attr', name, verts=P.b64_floats([c for v in d.co for c in v]))
        if 'attr' in d.dirty and d.totals:
            opts = {'smooth': P.Raw(''.join('1' if s else '0' for s in d.smooth)), 'mat': [int(m) for m in d.mat]}
            if d.uvs is not None and len(d.uvs) == len(d.loops):
                opts['uvs'] = P.b64_floats([c for uv in d.uvs for c in uv])
            req('mesh_attr', name, **opts)
        if 'edgeflags' in d.dirty:
            req('mesh_attr', name, seams=P.Raw(';'.join('%d,%d' % e for e in sorted(d.seams)) or ''),
                sharp=P.Raw(';'.join('%d,%d' % e for e in sorted(d.sharp)) or ''))
        if 'sel' in d.dirty:
            req('mesh_select', name, verts=P.Raw(','.join(map(str, sorted(d.sel_v)))),
                edges=P.Raw(','.join('%d,%d' % e for e in sorted(d.sel_e))),
                faces=P.Raw(','.join(map(str, sorted(d.sel_f)))))
        for k in list(d.dirty_keys):
            kb = next((x for x in d.keys if x['name'] == k), None)
            if kb is not None:
                req('shape_key', name, k, verts=P.b64_floats([c for v in kb['co'] for c in v]))
        d.dirty = set()
        d.dirty_keys = set()
        d.gen = generation()

    def _rename(self, new):
        self._flush_now()
        got = first(req('rename', 'mesh', self._name, new))
        _cache.pop(self._name, None)
        return got

    def _flush_now(self):
        ch = P.get()
        if ('mesh', self._name) in ch.pending:
            ch.pending.pop(('mesh', self._name))
            self._flush()

    # --- elements
    @property
    def vertices(self):
        return MeshVertices(self)

    @property
    def edges(self):
        return MeshEdges(self)

    @property
    def polygons(self):
        return MeshPolygons(self)

    @property
    def loops(self):
        return MeshLoops(self)

    @property
    def loop_triangles(self):
        return MeshLoopTriangles(self)

    @property
    def loop_triangle_polygons(self):
        return [t.polygon_index for t in MeshLoopTriangles(self)]

    @property
    def uv_layers(self):
        return UVLayers(self)

    @property
    def attributes(self):
        return Attributes(self)

    @property
    def vertex_colors(self):
        return Collection_()

    @property
    def color_attributes(self):
        return Collection_()

    @property
    def materials(self):
        return MeshMaterials(self)

    @property
    def shape_keys(self):
        d = self._d()
        return Key(self) if d.keys else None

    @property
    def users(self):
        return len(self._d().users)

    @property
    def is_editmode(self):
        return self._d().is_editmode

    @property
    def has_custom_normals(self):
        return False

    @property
    def use_auto_smooth(self):
        return self.get('_auto_smooth', False)

    @use_auto_smooth.setter
    def use_auto_smooth(self, v):
        self['_auto_smooth'] = bool(v)

    @property
    def auto_smooth_angle(self):
        return self.get('_auto_smooth_angle', math.radians(30))

    @auto_smooth_angle.setter
    def auto_smooth_angle(self, v):
        self['_auto_smooth_angle'] = float(v)

    @property
    def total_vert_sel(self):
        return len(self._d().sel_v)

    @property
    def total_edge_sel(self):
        return len(self._d().sel_e)

    @property
    def total_face_sel(self):
        return len(self._d().sel_f)

    # --- whole-mesh operations
    def from_pydata(self, vertices, edges, faces, shade_flat=True):
        opts = P.mesh_options(vertices, edges, faces)
        _cache.pop(self._name, None)
        P.get().pending.pop(('mesh', self._name), None)
        req('mesh_set', self._name, **opts)

    def clear_geometry(self):
        _cache.pop(self._name, None)
        P.get().pending.pop(('mesh', self._name), None)
        req('mesh_set', self._name, verts=P.Raw(''))

    def update(self, calc_edges=False, calc_edges_loose=False):
        self._flush_now()

    def validate(self, verbose=False, clean_customdata=True):
        self._flush_now()
        return False

    def calc_normals(self):
        pass

    def calc_normals_split(self):
        pass

    def free_normals_split(self):
        pass

    def calc_loop_triangles(self):
        pass

    def calc_tangents(self, uvmap=''):
        pass

    def shade_smooth(self):
        d = self._d()
        d.smooth = [True] * len(d.totals)
        self._touch('attr')

    def shade_flat(self):
        d = self._d()
        d.smooth = [False] * len(d.totals)
        self._touch('attr')

    def set_sharp_from_angle(self, angle=math.radians(30)):
        pass

    def flip_normals(self):
        self._flush_now()
        req('mesh_edit', self._name, P.Raw('flip_normals'))

    def transform(self, matrix, shape_keys=False):
        d = self._d()
        for i, c in enumerate(d.co):
            v = matrix @ Vector(c)
            d.co[i] = [v[0], v[1], v[2]]
        self._touch('co')

    def copy(self):
        self._flush_now()
        return Mesh(first(req('mesh_copy', self._name)))

    def user_clear(self):
        pass

    def to_mesh(self):
        return self

    def __getattr__(self, name):
        raise AttributeError("'Mesh' object has no attribute '%s'" % name)


# --- elements -----------------------------------------------------------------

class _Elem(bpy_struct):
    __slots__ = ('_m', 'index')

    def __init__(self, mesh, index):
        self._m = mesh
        self.index = index

    def _key(self):
        return (type(self).__name__, self._m._name, self.index)

    def __repr__(self):
        return "bpy.data.meshes['%s'].%s[%d]" % (self._m._name, self._seq, self.index)


class MeshVertex(_Elem):
    _seq = 'vertices'

    @property
    def co(self):
        d = self._m._d()
        i = self.index
        m = self._m

        def write(v):
            d.co[i] = [v[0], v[1], v[2]]
            m._touch('co')
        return Vector(d.co[i], _owner=write)

    @co.setter
    def co(self, v):
        d = self._m._d()
        d.co[self.index] = [float(v[0]), float(v[1]), float(v[2])]
        self._m._touch('co')

    @property
    def undeformed_co(self):
        return Vector(self._m._d().co[self.index])

    @property
    def normal(self):
        return Vector(self._m._d().derived()['vn'][self.index])

    @property
    def select(self):
        return self.index in self._m._d().sel_v

    @select.setter
    def select(self, s):
        d = self._m._d()
        (d.sel_v.add if s else d.sel_v.discard)(self.index)
        self._m._touch('sel')

    @property
    def hide(self):
        return False

    @hide.setter
    def hide(self, v):
        pass

    @property
    def groups(self):
        d = self._m._d()
        return [VertexGroupElement(g, w) for g, w in sorted(d.weights.get(self.index, {}).items())]


class VertexGroupElement:
    def __init__(self, group, weight):
        self.group = group
        self.weight = weight


class MeshEdge(_Elem):
    _seq = 'edges'

    @property
    def vertices(self):
        return list(self._m._d().edges[self.index])

    @vertices.setter
    def vertices(self, v):
        d = self._m._d()
        d.edges[self.index] = (int(v[0]), int(v[1]))
        self._m._touch('topo')

    @property
    def key(self):
        return tuple(sorted(self._m._d().edges[self.index]))

    @property
    def select(self):
        d = self._m._d()
        a, b = self.key
        return self.key in d.sel_e or (a in d.sel_v and b in d.sel_v)

    @select.setter
    def select(self, s):
        d = self._m._d()
        (d.sel_e.add if s else d.sel_e.discard)(self.key)
        if not s:
            for v in self.key:
                d.sel_v.discard(v)
        self._m._touch('sel')

    @property
    def use_seam(self):
        return self.key in self._m._d().seams

    @use_seam.setter
    def use_seam(self, s):
        d = self._m._d()
        (d.seams.add if s else d.seams.discard)(self.key)
        self._m._touch('edgeflags')

    @property
    def use_edge_sharp(self):
        return self.key in self._m._d().sharp

    @use_edge_sharp.setter
    def use_edge_sharp(self, s):
        d = self._m._d()
        (d.sharp.add if s else d.sharp.discard)(self.key)
        self._m._touch('edgeflags')

    @property
    def is_loose(self):
        d = self._m._d()
        k = self.key
        for f in d.faces():
            n = len(f)
            for i in range(n):
                if tuple(sorted((f[i], f[(i + 1) % n]))) == k:
                    return False
        return True

    hide = False


class MeshPolygon(_Elem):
    _seq = 'polygons'

    @property
    def vertices(self):
        return list(self._m._d().face(self.index))

    @vertices.setter
    def vertices(self, vs):
        d = self._m._d()
        s, t = d.starts[self.index], d.totals[self.index]
        if len(vs) != t:
            raise ValueError('a polygon keeps its corner count (%d)' % t)
        d.loops[s:s + t] = [int(v) for v in vs]
        self._m._touch('topo')

    @property
    def loop_start(self):
        return self._m._d().starts[self.index]

    @loop_start.setter
    def loop_start(self, v):
        self._m._d().starts[self.index] = int(v)
        self._m._touch('topo')

    @property
    def loop_total(self):
        return self._m._d().totals[self.index]

    @loop_total.setter
    def loop_total(self, v):
        self._m._d().totals[self.index] = int(v)
        self._m._touch('topo')

    @property
    def loop_indices(self):
        d = self._m._d()
        s = d.starts[self.index]
        return list(range(s, s + d.totals[self.index]))

    @property
    def edge_keys(self):
        vs = self.vertices
        n = len(vs)
        return [tuple(sorted((vs[i], vs[(i + 1) % n]))) for i in range(n)]

    @property
    def normal(self):
        return Vector(self._m._d().derived()['fn'][self.index])

    @property
    def center(self):
        return Vector(self._m._d().derived()['fc'][self.index])

    @property
    def area(self):
        return self._m._d().derived()['fa'][self.index]

    @property
    def use_smooth(self):
        return self._m._d().smooth[self.index]

    @use_smooth.setter
    def use_smooth(self, s):
        self._m._d().smooth[self.index] = bool(s)
        self._m._touch('attr')

    @property
    def material_index(self):
        return self._m._d().mat[self.index]

    @material_index.setter
    def material_index(self, i):
        self._m._d().mat[self.index] = int(i)
        self._m._touch('attr')

    @property
    def select(self):
        d = self._m._d()
        return self.index in d.sel_f or all(v in d.sel_v for v in d.face(self.index))

    @select.setter
    def select(self, s):
        d = self._m._d()
        if s:
            d.sel_f.add(self.index)
        else:
            d.sel_f.discard(self.index)
            for v in d.face(self.index):
                d.sel_v.discard(v)
        self._m._touch('sel')

    hide = False

    def flip(self):
        d = self._m._d()
        s, t = d.starts[self.index], d.totals[self.index]
        d.loops[s:s + t] = list(reversed(d.loops[s:s + t]))
        self._m._touch('topo')


class MeshLoop(_Elem):
    _seq = 'loops'

    @property
    def vertex_index(self):
        return self._m._d().loops[self.index]

    @vertex_index.setter
    def vertex_index(self, v):
        self._m._d().loops[self.index] = int(v)
        self._m._touch('topo')

    def _poly(self):
        d = self._m._d()
        for p, s in enumerate(d.starts):
            if s <= self.index < s + d.totals[p]:
                return p
        return -1

    @property
    def edge_index(self):
        d = self._m._d()
        p = self._poly()
        f = d.face(p)
        k = self.index - d.starts[p]
        e = tuple(sorted((f[k], f[(k + 1) % len(f)])))
        return d.derived()['edge_index'].get(e, -1)

    @property
    def normal(self):
        d = self._m._d()
        p = self._poly()
        if d.smooth and d.smooth[p]:
            return Vector(d.derived()['vn'][d.loops[self.index]])
        return Vector(d.derived()['fn'][p])

    @property
    def tangent(self):
        return Vector((1.0, 0.0, 0.0))


class MeshLoopTriangle(bpy_struct):
    def __init__(self, mesh, index, poly, loops):
        self._m = mesh
        self.index = index
        self.polygon_index = poly
        self.loops = loops

    @property
    def vertices(self):
        d = self._m._d()
        return [d.loops[l] for l in self.loops]

    @property
    def normal(self):
        a, b, c = (Vector(self._m._d().co[v]) for v in self.vertices)
        return (b - a).cross(c - a).normalized()

    @property
    def area(self):
        a, b, c = (Vector(self._m._d().co[v]) for v in self.vertices)
        return (b - a).cross(c - a).length / 2

    @property
    def split_normals(self):
        return [MeshLoop(self._m, l).normal[:] for l in self.loops]

    @property
    def material_index(self):
        return self._m._d().mat[self.polygon_index]

    @property
    def use_smooth(self):
        return self._m._d().smooth[self.polygon_index]


class _Elems(Collection_):
    _cls = None

    def __init__(self, mesh):
        self._m = mesh

    def _count(self):
        return 0

    def __len__(self):
        return self._count()

    def _items(self):
        return [self._cls(self._m, i) for i in range(self._count())]

    def __getitem__(self, i):
        if isinstance(i, slice):
            return self._items()[i]
        n = self._count()
        if i < 0:
            i += n
        if not 0 <= i < n:
            raise IndexError('index %d out of range' % i)
        return self._cls(self._m, i)

    def __iter__(self):
        return (self._cls(self._m, i) for i in range(self._count()))

    def _name_of(self, item):
        return None

    # fast paths for the common attributes
    def foreach_get(self, attr, seq):
        out = self._get_flat(attr)
        if out is None:
            return Collection_.foreach_get(self, attr, seq)
        seq[:len(out)] = out

    def foreach_set(self, attr, seq):
        if not self._set_flat(attr, list(seq)):
            Collection_.foreach_set(self, attr, seq)

    def _get_flat(self, attr):
        return None

    def _set_flat(self, attr, seq):
        return False


class MeshVertices(_Elems):
    _cls = MeshVertex

    def _count(self):
        return len(self._m._d().co)

    def add(self, count):
        d = self._m._d()
        d.co.extend([0.0, 0.0, 0.0] for _ in range(int(count)))
        self._m._touch('topo')

    def _get_flat(self, attr):
        d = self._m._d()
        if attr in ('co', 'undeformed_co'):
            return [c for v in d.co for c in v]
        if attr == 'normal':
            return [c for v in d.derived()['vn'] for c in v]
        if attr == 'select':
            return [i in d.sel_v for i in range(len(d.co))]
        return None

    def _set_flat(self, attr, seq):
        d = self._m._d()
        if attr == 'co':
            d.co = [[float(seq[i]), float(seq[i + 1]), float(seq[i + 2])] for i in range(0, len(d.co) * 3, 3)]
            self._m._touch('co' if 'topo' not in d.dirty else 'topo')
            return True
        if attr == 'select':
            d.sel_v = set(i for i, s in enumerate(seq) if s)
            self._m._touch('sel')
            return True
        return False


class MeshEdges(_Elems):
    _cls = MeshEdge

    def _count(self):
        return len(self._m._d().edges)

    def add(self, count):
        d = self._m._d()
        d.edges.extend((0, 0) for _ in range(int(count)))
        self._m._touch('topo')

    def _get_flat(self, attr):
        d = self._m._d()
        if attr == 'vertices':
            return [c for e in d.edges for c in e]
        return None

    def _set_flat(self, attr, seq):
        d = self._m._d()
        if attr == 'vertices':
            d.edges = [(int(seq[i]), int(seq[i + 1])) for i in range(0, len(d.edges) * 2, 2)]
            self._m._touch('topo')
            return True
        return False


class MeshPolygons(_Elems):
    _cls = MeshPolygon

    def _count(self):
        return len(self._m._d().totals)

    def add(self, count):
        d = self._m._d()
        n = int(count)
        d.starts.extend([0] * n)
        d.totals.extend([0] * n)
        d.smooth.extend([False] * n)
        d.mat.extend([0] * n)
        self._m._touch('topo')

    def _get_flat(self, attr):
        d = self._m._d()
        if attr == 'loop_start':
            return list(d.starts)
        if attr == 'loop_total':
            return list(d.totals)
        if attr == 'vertices':
            return list(d.loops)
        if attr == 'use_smooth':
            return list(d.smooth)
        if attr == 'material_index':
            return list(d.mat)
        if attr == 'normal':
            return [c for n in d.derived()['fn'] for c in n]
        if attr == 'center':
            return [c for n in d.derived()['fc'] for c in n]
        if attr == 'area':
            return list(d.derived()['fa'])
        return None

    def _set_flat(self, attr, seq):
        d = self._m._d()
        n = len(d.totals)
        if attr in ('loop_start', 'loop_total'):
            (d.starts if attr == 'loop_start' else d.totals)[:] = [int(x) for x in seq[:n]]
            self._m._touch('topo')
            return True
        if attr == 'vertices':
            d.loops = [int(x) for x in seq]
            self._m._touch('topo')
            return True
        if attr == 'use_smooth':
            d.smooth = [bool(x) for x in seq[:n]]
            self._m._touch('attr')
            return True
        if attr == 'material_index':
            d.mat = [int(x) for x in seq[:n]]
            self._m._touch('attr')
            return True
        return False


class MeshLoops(_Elems):
    _cls = MeshLoop

    def _count(self):
        return len(self._m._d().loops)

    def add(self, count):
        d = self._m._d()
        d.loops.extend([0] * int(count))
        if d.uvs is not None:
            d.uvs.extend([0.0, 0.0] for _ in range(int(count)))
        self._m._touch('topo')

    def _get_flat(self, attr):
        if attr == 'vertex_index':
            return list(self._m._d().loops)
        return None

    def _set_flat(self, attr, seq):
        if attr == 'vertex_index':
            d = self._m._d()
            d.loops = [int(x) for x in seq[:len(d.loops)]]
            self._m._touch('topo')
            return True
        return False


class MeshLoopTriangles(Collection_):
    def __init__(self, mesh):
        self._m = mesh

    def _items(self):
        d = self._m._d()
        out = []
        from mathutils.geometry import tessellate_polygon
        for p in range(len(d.totals)):
            s, t = d.starts[p], d.totals[p]
            if t == 3:
                tris = [(0, 1, 2)]
            elif t == 4:
                tris = [(0, 1, 2), (0, 2, 3)]
            else:
                tris = tessellate_polygon([[d.co[v] for v in d.loops[s:s + t]]])
            for tri in tris:
                out.append(MeshLoopTriangle(self._m, len(out), p, [s + k for k in tri]))
        return out

    def _name_of(self, item):
        return None


# --- UV layers ------------------------------------------------------------------

class MeshUVLoop(bpy_struct):
    def __init__(self, mesh, index):
        self._m = mesh
        self.index = index

    @property
    def uv(self):
        d = self._m._d()
        i, m = self.index, self._m

        def write(v):
            d.uvs[i] = [v[0], v[1]]
            m._touch('attr')
        return Vector(d.uvs[i], _owner=write)

    @uv.setter
    def uv(self, v):
        d = self._m._d()
        d.uvs[self.index] = [float(v[0]), float(v[1])]
        self._m._touch('attr')

    @property
    def vector(self):
        return self.uv

    @vector.setter
    def vector(self, v):
        self.uv = v

    select = False
    pin_uv = False


class _UVData(_Elems):
    _cls = MeshUVLoop

    def _count(self):
        d = self._m._d()
        return len(d.uvs) if d.uvs is not None else 0

    def _get_flat(self, attr):
        if attr in ('uv', 'vector'):
            return [c for uv in self._m._d().uvs for c in uv]
        return None

    def _set_flat(self, attr, seq):
        if attr in ('uv', 'vector'):
            d = self._m._d()
            d.uvs = [[float(seq[i]), float(seq[i + 1])] for i in range(0, len(d.uvs) * 2, 2)]
            self._m._touch('attr')
            return True
        return False


class MeshUVLoopLayer(bpy_struct):
    def __init__(self, mesh, name='UVMap'):
        self._m = mesh
        self.name = name

    def _key(self):
        return ('uv', self._m._name)

    @property
    def data(self):
        return _UVData(self._m)

    @property
    def uv(self):
        return _UVData(self._m)

    active = True
    active_render = True


class UVLayers(Collection_):
    def __init__(self, mesh):
        self._m = mesh

    def _items(self):
        d = self._m._d()
        return [MeshUVLoopLayer(self._m, self._m.get('_uv_name', 'UVMap'))] if d.uvs is not None else []

    def new(self, name='UVMap', do_init=True):
        d = self._m._d()
        if d.uvs is None:
            d.uvs = []
            for p in range(len(d.totals)):
                t = d.totals[p]
                corners = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]
                d.uvs.extend(list(corners[k % 4]) for k in range(t))
            self._m['_uv_name'] = name
            self._m._touch('attr')
        return MeshUVLoopLayer(self._m, self._m.get('_uv_name', name))

    def remove(self, layer):
        d = self._m._d()
        d.uvs = None
        self._m._flush_now()
        req('mesh_attr', self._m._name, clear_uvs=True)

    @property
    def active(self):
        items = self._items()
        return items[0] if items else None

    @active.setter
    def active(self, v):
        pass

    @property
    def active_index(self):
        return 0 if self._items() else -1

    @active_index.setter
    def active_index(self, v):
        pass


# --- generic attributes (lite) --------------------------------------------------------

class _AttrValue(bpy_struct):
    def __init__(self, attr, i):
        self._a = attr
        self.index = i

    @property
    def value(self):
        return self._a._get(self.index)

    @value.setter
    def value(self, v):
        self._a._set(self.index, v)

    vector = value
    color = value


class Attribute(bpy_struct):
    def __init__(self, mesh, name, data_type, domain):
        self._m = mesh
        self.name = name
        self.data_type = data_type
        self.domain = domain

    def _key(self):
        return ('attr', self._m._name, self.name)

    @property
    def data(self):
        n = {'POINT': len(self._m._d().co), 'FACE': len(self._m._d().totals), 'CORNER': len(self._m._d().loops),
             'EDGE': len(self._m._d().edges)}[self.domain]
        return [_AttrValue(self, i) for i in range(n)]

    def _get(self, i):
        d = self._m._d()
        if self.name == 'position':
            return MeshVertex(self._m, i).co
        if self.name == 'material_index':
            return d.mat[i]
        if self.name == 'sharp_face':
            return not d.smooth[i]
        store = self._m.get('_attr_' + self.name)
        return store[i] if store is not None else None

    def _set(self, i, v):
        d = self._m._d()
        if self.name == 'position':
            MeshVertex(self._m, i).co = v
        elif self.name == 'material_index':
            MeshPolygon(self._m, i).material_index = v
        elif self.name == 'sharp_face':
            MeshPolygon(self._m, i).use_smooth = not v
        else:
            store = self._m.get('_attr_' + self.name)
            if store is not None:
                store[i] = v


class Attributes(Collection_):
    """position, material_index, sharp_face and the UV map are Polyamour's
    own; other attributes made with new() live in this Python only."""

    def __init__(self, mesh):
        self._m = mesh

    def _items(self):
        d = self._m._d()
        out = [Attribute(self._m, 'position', 'FLOAT_VECTOR', 'POINT'), Attribute(self._m, 'material_index', 'INT', 'FACE'),
               Attribute(self._m, 'sharp_face', 'BOOLEAN', 'FACE')]
        if d.uvs is not None:
            out.append(Attribute(self._m, self._m.get('_uv_name', 'UVMap'), 'FLOAT2', 'CORNER'))
        for k, (t, dom) in self._m.get('_attrs', {}).items():
            out.append(Attribute(self._m, k, t, dom))
        return out

    def new(self, name, type, domain):
        d = self._m._d()
        n = {'POINT': len(d.co), 'FACE': len(d.totals), 'CORNER': len(d.loops), 'EDGE': len(d.edges)}[domain]
        zero = {'FLOAT_VECTOR': (0.0, 0.0, 0.0), 'FLOAT_COLOR': (0.0, 0.0, 0.0, 1.0), 'BYTE_COLOR': (0.0, 0.0, 0.0, 1.0),
                'FLOAT2': (0.0, 0.0), 'BOOLEAN': False, 'INT': 0}.get(type, 0.0)
        attrs = self._m.get('_attrs', {})
        attrs[name] = (type, domain)
        self._m['_attrs'] = attrs
        self._m['_attr_' + name] = [zero] * n
        return Attribute(self._m, name, type, domain)

    def remove(self, attr):
        attrs = self._m.get('_attrs', {})
        attrs.pop(attr.name, None)
        self._m.pop('_attr_' + attr.name, None)

    @property
    def active(self):
        return None

    @property
    def active_color(self):
        return None


# --- materials on a mesh ------------------------------------------------------------

_pending_materials = {}


class MeshMaterials(Collection_):
    """Polyamour keeps material slots on objects: these read the first user's
    slots and change every user's."""

    def __init__(self, mesh):
        self._m = mesh

    def _items(self):
        from .types import Material
        users = self._m._d().users
        if not users:
            return [Material(n) if n else None for n in _pending_materials.get(self._m._name, [])]
        d = req('dump_object', users[0])
        return [Material(n) if n else None for n in d['materials']]

    def _name_of(self, item):
        return item.name if item is not None else ''

    def append(self, mat):
        users = self._m._d().users
        if not users:
            _pending_materials.setdefault(self._m._name, []).append(mat.name if mat else None)
            return
        n = len(self._items())
        for u in users:
            req('material_slot', u, n, mat.name if mat else P.Raw('none'))

    def pop(self, index=-1):
        items = self._items()
        if not items:
            raise IndexError('no materials')
        i = index % len(items)
        for u in self._m._d().users:
            req('material_slot', u, i, remove=True)
        if not self._m._d().users:
            _pending_materials[self._m._name].pop(i)
        return items[i]

    def clear(self):
        for u in self._m._d().users:
            req('material_clear', u)
        _pending_materials.pop(self._m._name, None)

    def __setitem__(self, i, mat):
        for u in self._m._d().users:
            req('material_slot', u, i, mat.name if mat else P.Raw('none'))


# --- shape keys -----------------------------------------------------------------------

class ShapeKeyPoint(bpy_struct):
    def __init__(self, mesh, key, i):
        self._m = mesh
        self._k = key
        self.index = i

    def _kb(self):
        return next(x for x in self._m._d().keys if x['name'] == self._k)

    @property
    def co(self):
        kb = self._kb()
        i, m, k = self.index, self._m, self._k

        def write(v):
            kb['co'][i] = [v[0], v[1], v[2]]
            m._d().dirty_keys.add(k)
            mark_dirty(('mesh', m._name), m)
        return Vector(kb['co'][i], _owner=write)

    @co.setter
    def co(self, v):
        kb = self._kb()
        kb['co'][self.index] = [float(v[0]), float(v[1]), float(v[2])]
        self._m._d().dirty_keys.add(self._k)
        mark_dirty(('mesh', self._m._name), self._m)


class _KeyPoints(Collection_):
    def __init__(self, mesh, key):
        self._m = mesh
        self._k = key

    def _items(self):
        n = len(self._m._d().co)
        return [ShapeKeyPoint(self._m, self._k, i) for i in range(n)]

    def foreach_get(self, attr, seq):
        if attr == 'co':
            kb = next(x for x in self._m._d().keys if x['name'] == self._k)
            out = [c for v in kb['co'] for c in v]
            seq[:len(out)] = out
        else:
            Collection_.foreach_get(self, attr, seq)

    def foreach_set(self, attr, seq):
        if attr == 'co':
            kb = next(x for x in self._m._d().keys if x['name'] == self._k)
            kb['co'] = [[float(seq[i]), float(seq[i + 1]), float(seq[i + 2])] for i in range(0, len(kb['co']) * 3, 3)]
            self._m._d().dirty_keys.add(self._k)
            mark_dirty(('mesh', self._m._name), self._m)
        else:
            Collection_.foreach_set(self, attr, seq)


class ShapeKey(bpy_struct):
    def __init__(self, mesh, name):
        self._m = mesh
        self.__dict__['_n'] = name

    def _key(self):
        return ('shape_key', self._m._name, self._n)

    def __setattr__(self, k, v):
        if k in ShapeKey._SIMPLE:
            self._m._flush_now()
            if k == 'relative_key':
                v = v.name
            elif k == 'vertex_group':
                v = v or P.Raw('none')
            elif k == 'interpolation':
                v = P.Raw(str(v).lower().replace('key_', ''))
            req('shape_key', self._m._name, self._n, **{k: v})
        elif k == 'name':
            raise AttributeError('renaming a shape key is not supported yet')
        else:
            object.__setattr__(self, k, v)

    _SIMPLE = ('value', 'slider_min', 'slider_max', 'relative_key', 'vertex_group', 'mute', 'interpolation')

    def _kb(self):
        return next(x for x in self._m._d().keys if x['name'] == self._n)

    @property
    def name(self):
        return self._n

    @property
    def value(self):
        return self._kb()['value']

    @property
    def slider_min(self):
        return self._kb()['slider_min']

    @property
    def slider_max(self):
        return self._kb()['slider_max']

    @property
    def relative_key(self):
        r = self._kb()['relative_key']
        return ShapeKey(self._m, r) if r else self

    @property
    def vertex_group(self):
        return self._kb()['vertex_group']

    @property
    def mute(self):
        return self._kb()['mute']

    @property
    def interpolation(self):
        return self._kb()['interpolation']

    @property
    def frame(self):
        return self._kb()['frame']

    @property
    def data(self):
        return _KeyPoints(self._m, self._n)

    @property
    def points(self):
        return _KeyPoints(self._m, self._n)

    @property
    def id_data(self):
        return Key(self._m)

    def normals_vertex_get(self):
        return []


class _KeyBlocks(Collection_):
    def __init__(self, mesh):
        self._m = mesh

    def _items(self):
        return [ShapeKey(self._m, k['name']) for k in self._m._d().keys]

    def remove(self, key):
        self._m._flush_now()
        req('shape_key_remove', self._m._name, key.name)


class Key(ID):
    _kind = 'key'
    _collection_name = 'shape_keys'

    def __init__(self, mesh):
        ID.__init__(self, 'Key')
        self.__dict__['_m'] = mesh

    def _key(self):
        return ('key', self._m._name)

    @property
    def key_blocks(self):
        return _KeyBlocks(self._m)

    @property
    def reference_key(self):
        items = self.key_blocks._items()
        return items[0] if items else None

    @property
    def user(self):
        return self._m

    @property
    def use_relative(self):
        return self._m._d().use_relative

    @use_relative.setter
    def use_relative(self, v):
        self._m._flush_now()
        req('shape_key_settings', self._m._name, use_relative=bool(v))

    @property
    def eval_time(self):
        return self._m._d().eval_time

    @eval_time.setter
    def eval_time(self, v):
        self._m._flush_now()
        req('shape_key_settings', self._m._name, eval_time=float(v))

    @property
    def animation_data(self):
        return None
