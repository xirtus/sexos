# SPDX-License-Identifier: GPL-3.0-only
"""bmesh.types: an editable mesh with linked vertices, edges, faces and loops,
in pure Python. BMesh.from_mesh / to_mesh copy to and from a bpy Mesh."""
from mathutils import Vector


class _Elem:
    __slots__ = ('index', 'select', 'hide', 'tag', 'is_valid', '_data', '__weakref__')

    def _init(self):
        self.index = -1
        self.select = False
        self.hide = False
        self.tag = False
        self.is_valid = True
        self._data = {}

    def select_set(self, s):
        self.select = bool(s)

    def hide_set(self, h):
        self.hide = bool(h)

    def __getitem__(self, layer):
        return self._data.setdefault(layer.name, layer.default())

    def __setitem__(self, layer, value):
        self._data[layer.name] = value


class BMVert(_Elem):
    __slots__ = ('co', 'normal', 'link_edges', 'link_loops')

    def __init__(self, co):
        self._init()
        self.co = Vector(co).to_3d()
        self.normal = Vector((0.0, 0.0, 0.0))
        self.link_edges = []
        self.link_loops = []

    def __repr__(self):
        return '<BMVert(index=%d, co=%r)>' % (self.index, tuple(round(c, 4) for c in self.co))

    @property
    def link_faces(self):
        out = []
        for l in self.link_loops:
            if l.face not in out:
                out.append(l.face)
        return out

    @property
    def is_wire(self):
        return bool(self.link_edges) and not self.link_loops

    @property
    def is_boundary(self):
        return any(e.is_boundary for e in self.link_edges)

    @property
    def is_manifold(self):
        return all(e.is_manifold for e in self.link_edges)

    def calc_edge_angle(self, fallback=None):
        if len(self.link_edges) != 2:
            return fallback
        a, b = (e.other_vert(self).co - self.co for e in self.link_edges)
        return a.angle(b, 0.0)

    def calc_shell_factor(self):
        return 1.0

    def normal_update(self):
        n = Vector((0.0, 0.0, 0.0))
        for f in self.link_faces:
            n += f.normal
        self.normal = n.normalized() if n.length else Vector((0.0, 0.0, 1.0))

    def copy_from(self, other):
        self.co = other.co.copy()
        self._data = dict(other._data)


class BMEdge(_Elem):
    __slots__ = ('verts', 'link_loops', 'smooth', 'seam')

    def __init__(self, a, b):
        self._init()
        self.verts = [a, b]
        self.link_loops = []
        self.smooth = True
        self.seam = False

    def __repr__(self):
        return '<BMEdge(index=%d, verts=(%d, %d))>' % (self.index, self.verts[0].index, self.verts[1].index)

    @property
    def link_faces(self):
        return [l.face for l in self.link_loops]

    def other_vert(self, v):
        if v is self.verts[0]:
            return self.verts[1]
        if v is self.verts[1]:
            return self.verts[0]
        return None

    def calc_length(self):
        return (self.verts[0].co - self.verts[1].co).length

    def calc_face_angle(self, fallback=None):
        fs = self.link_faces
        if len(fs) != 2:
            return fallback
        return fs[0].normal.angle(fs[1].normal, 0.0)

    def calc_face_angle_signed(self, fallback=None):
        return self.calc_face_angle(fallback)

    @property
    def is_boundary(self):
        return len(self.link_loops) == 1

    @property
    def is_manifold(self):
        return len(self.link_loops) == 2

    @property
    def is_wire(self):
        return not self.link_loops

    @property
    def is_contiguous(self):
        return self.is_manifold

    @property
    def is_convex(self):
        return True

    @property
    def use_seam(self):
        return self.seam


class BMLoop(_Elem):
    __slots__ = ('vert', 'edge', 'face', 'link_loop_next', 'link_loop_prev')

    def __init__(self, vert, edge, face):
        self._init()
        self.vert = vert
        self.edge = edge
        self.face = face
        self.link_loop_next = None
        self.link_loop_prev = None

    @property
    def link_loop_radial_next(self):
        ls = self.edge.link_loops
        return ls[(ls.index(self) + 1) % len(ls)]

    link_loop_radial_prev = link_loop_radial_next

    @property
    def link_loops(self):
        return [l for l in self.edge.link_loops if l is not self]

    def calc_angle(self):
        a = self.link_loop_prev.vert.co - self.vert.co
        b = self.link_loop_next.vert.co - self.vert.co
        return a.angle(b, 0.0)

    def calc_normal(self):
        a = self.link_loop_prev.vert.co - self.vert.co
        b = self.link_loop_next.vert.co - self.vert.co
        return b.cross(a).normalized()

    def calc_tangent(self):
        return (self.link_loop_next.vert.co - self.vert.co).normalized()


class BMFace(_Elem):
    __slots__ = ('loops', 'normal', 'material_index', 'smooth', '_bm')

    def __init__(self):
        self._init()
        self.loops = []
        self.normal = Vector((0.0, 0.0, 1.0))
        self.material_index = 0
        self.smooth = False
        self._bm = None

    def __repr__(self):
        return '<BMFace(index=%d, totverts=%d)>' % (self.index, len(self.loops))

    @property
    def verts(self):
        return [l.vert for l in self.loops]

    @property
    def edges(self):
        return [l.edge for l in self.loops]

    def calc_center_median(self):
        vs = self.verts
        c = Vector((0.0, 0.0, 0.0))
        for v in vs:
            c += v.co
        return c / max(1, len(vs))

    calc_center_median_weighted = calc_center_median

    def calc_center_bounds(self):
        vs = [v.co for v in self.verts]
        lo = Vector([min(v[k] for v in vs) for k in range(3)])
        hi = Vector([max(v[k] for v in vs) for k in range(3)])
        return (lo + hi) / 2

    def _newell(self):
        n = Vector((0.0, 0.0, 0.0))
        vs = [v.co for v in self.verts]
        for i, a in enumerate(vs):
            b = vs[(i + 1) % len(vs)]
            n.x += (a.y - b.y) * (a.z + b.z)
            n.y += (a.z - b.z) * (a.x + b.x)
            n.z += (a.x - b.x) * (a.y + b.y)
        return n

    def calc_area(self):
        return self._newell().length / 2

    def calc_perimeter(self):
        return sum(e.calc_length() for e in self.edges)

    def calc_tangent_edge(self):
        return (self.loops[1].vert.co - self.loops[0].vert.co).normalized()

    def normal_update(self):
        n = self._newell()
        self.normal = n.normalized() if n.length else Vector((0.0, 0.0, 1.0))

    def normal_flip(self):
        _reverse_face(self)
        self.normal = -self.normal

    def copy(self, verts=True, edges=True):
        raise NotImplementedError('BMFace.copy is not supported yet')

    def copy_from(self, other):
        self.material_index = other.material_index
        self.smooth = other.smooth

    def copy_from_face_interp(self, face, vert=True):
        pass

    def is_valid_face(self):
        return self.is_valid


def _link_face(face, verts, bm):
    loops = []
    n = len(verts)
    for i, v in enumerate(verts):
        w = verts[(i + 1) % n]
        e = bm.edges.get((v, w)) or bm.edges.new((v, w))
        l = BMLoop(v, e, face)
        loops.append(l)
        v.link_loops.append(l)
        e.link_loops.append(l)
    for i, l in enumerate(loops):
        l.link_loop_next = loops[(i + 1) % n]
        l.link_loop_prev = loops[i - 1]
    face.loops = loops


def _unlink_face(face):
    for l in face.loops:
        if l in l.vert.link_loops:
            l.vert.link_loops.remove(l)
        if l in l.edge.link_loops:
            l.edge.link_loops.remove(l)
        l.is_valid = False


def _reverse_face(face):
    bm = face._bm
    verts = list(reversed(face.verts))
    data = [l._data for l in reversed(face.loops)]
    # Keep each corner's data on its vertex: rotate so loop i starts at verts[i].
    _unlink_face(face)
    _link_face(face, verts, bm)
    for l, d in zip(face.loops, data):
        l._data = d



class _Layer:
    def __init__(self, name, default):
        self.name = name
        self._default = default

    def default(self):
        d = self._default
        return d() if callable(d) else d


class BMLoopUV:
    __slots__ = ('uv', 'pin_uv', 'select', 'select_edge')

    def __init__(self, uv=(0.0, 0.0)):
        self.uv = Vector(uv)
        self.pin_uv = False
        self.select = False
        self.select_edge = False


class BMLayerCollection:
    def __init__(self, default):
        self._layers = {}
        self._default = default
        self.active = None

    def new(self, name=''):
        name = name or 'UVMap'
        lay = _Layer(name, self._default)
        self._layers[name] = lay
        if self.active is None:
            self.active = lay
        return lay

    def verify(self):
        return self.active or self.new()

    def get(self, name, default=None):
        return self._layers.get(name, default)

    def __getitem__(self, k):
        if isinstance(k, int):
            return list(self._layers.values())[k]
        return self._layers[k]

    def __contains__(self, k):
        return k in self._layers

    def __len__(self):
        return len(self._layers)

    def __iter__(self):
        return iter(self._layers.values())

    def keys(self):
        return list(self._layers)

    def values(self):
        return list(self._layers.values())

    def items(self):
        return list(self._layers.items())

    def remove(self, layer):
        self._layers.pop(layer.name, None)
        if self.active is layer:
            self.active = next(iter(self._layers.values()), None)


class BMLayerAccess:
    def __init__(self, default):
        self.uv = BMLayerCollection(lambda: BMLoopUV())
        self.float = BMLayerCollection(0.0)
        self.int = BMLayerCollection(0)
        self.string = BMLayerCollection(b'')
        self.color = BMLayerCollection(lambda: Vector((1.0, 1.0, 1.0, 1.0)))
        self.float_color = self.color
        self.deform = BMLayerCollection(dict)
        self.shape = BMLayerCollection(lambda: Vector((0.0, 0.0, 0.0)))
        self.float_vector = BMLayerCollection(lambda: Vector((0.0, 0.0, 0.0)))
        self.skin = BMLayerCollection(None)


class _Seq:
    def __init__(self, bm):
        self._bm = bm
        self._items = []
        self._table = False

    def __iter__(self):
        return iter(list(self._items))

    def __len__(self):
        return len(self._items)

    def __getitem__(self, i):
        return self._items[i]

    def __contains__(self, x):
        return x in self._items

    def ensure_lookup_table(self):
        self._table = True

    def index_update(self):
        for i, x in enumerate(self._items):
            x.index = i

    def sort(self, key=None, reverse=False):
        self._items.sort(key=key or (lambda x: x.index), reverse=reverse)

    active = None


class BMVertSeq(_Seq):
    def __init__(self, bm):
        super().__init__(bm)
        self.layers = BMLayerAccess(None)

    def new(self, co=(0.0, 0.0, 0.0), example=None):
        v = BMVert(co)
        if example is not None:
            v._data = dict(example._data)
        self._items.append(v)
        return v

    def remove(self, v):
        for e in list(v.link_edges):
            self._bm.edges.remove(e)
        if v in self._items:
            self._items.remove(v)
        v.is_valid = False


class BMEdgeSeq(_Seq):
    def __init__(self, bm):
        super().__init__(bm)
        self.layers = BMLayerAccess(None)
        self._lookup = {}

    def new(self, verts, example=None):
        a, b = verts
        if a is b:
            raise ValueError('edges.new(): the two vertices are the same')
        k = (id(a), id(b)) if id(a) < id(b) else (id(b), id(a))
        if k in self._lookup:
            raise ValueError('edges.new(): this edge exists')
        e = BMEdge(a, b)
        a.link_edges.append(e)
        b.link_edges.append(e)
        self._lookup[k] = e
        self._items.append(e)
        if example is not None:
            e.smooth, e.seam = example.smooth, example.seam
        return e

    def get(self, verts, fallback=None):
        a, b = verts
        k = (id(a), id(b)) if id(a) < id(b) else (id(b), id(a))
        return self._lookup.get(k, fallback)

    def remove(self, e):
        for l in list(e.link_loops):
            self._bm.faces.remove(l.face)
        for v in e.verts:
            if e in v.link_edges:
                v.link_edges.remove(e)
        a, b = e.verts
        k = (id(a), id(b)) if id(a) < id(b) else (id(b), id(a))
        self._lookup.pop(k, None)
        if e in self._items:
            self._items.remove(e)
        e.is_valid = False


class BMFaceSeq(_Seq):
    def __init__(self, bm):
        super().__init__(bm)
        self.layers = BMLayerAccess(None)
        self.active = None

    def new(self, verts, example=None):
        verts = list(verts)
        if len(verts) < 3:
            raise ValueError('faces.new(): needs at least 3 vertices')
        if len(set(map(id, verts))) != len(verts):
            raise ValueError('faces.new(): found the same vertex used multiple times')
        if self.get(verts) is not None:
            raise ValueError('faces.new(): face already exists')
        f = BMFace()
        f._bm = self._bm
        _link_face(f, verts, self._bm)
        if example is not None:
            f.material_index, f.smooth = example.material_index, example.smooth
        f.normal_update()
        self._items.append(f)
        return f

    def get(self, verts, fallback=None):
        ids = set(map(id, verts))
        v0 = list(verts)[0]
        for l in v0.link_loops:
            f = l.face
            if len(f.loops) == len(ids) and set(map(id, f.verts)) == ids:
                return f
        return fallback

    def remove(self, f):
        _unlink_face(f)
        if f in self._items:
            self._items.remove(f)
        f.is_valid = False


class BMesh:
    def __init__(self):
        self.verts = BMVertSeq(self)
        self.edges = BMEdgeSeq(self)
        self.faces = BMFaceSeq(self)
        self.loops = _Loops(self)
        self.select_mode = {'VERT'}
        self.select_history = _SelectHistory()
        self.is_wrapped = False
        self.is_valid = True
        self._mesh = None

    def free(self):
        self.is_valid = False

    def clear(self):
        self.__init__()

    def copy(self):
        bm = BMesh()
        m = {}
        for v in self.verts:
            nv = bm.verts.new(v.co)
            nv.select, nv._data = v.select, dict(v._data)
            m[id(v)] = nv
        for e in self.edges:
            ne = bm.edges.get((m[id(e.verts[0])], m[id(e.verts[1])])) or bm.edges.new((m[id(e.verts[0])], m[id(e.verts[1])]))
            ne.seam, ne.smooth, ne.select = e.seam, e.smooth, e.select
        for f in self.faces:
            nf = bm.faces.new([m[id(v)] for v in f.verts])
            nf.material_index, nf.smooth, nf.select = f.material_index, f.smooth, f.select
            for a, b in zip(f.loops, nf.loops):
                b._data = {k: (BMLoopUV(v.uv) if isinstance(v, BMLoopUV) else v) for k, v in a._data.items()}
        for name in self.loops.layers.uv.keys():
            bm.loops.layers.uv.new(name)
        return bm

    def normal_update(self):
        for f in self.faces:
            f.normal_update()
        for v in self.verts:
            v.normal_update()

    def select_flush(self, select):
        for e in self.edges:
            if all(v.select for v in e.verts):
                e.select = select or e.select
        for f in self.faces:
            if all(v.select for v in f.verts):
                f.select = select or f.select

    def select_flush_mode(self):
        self.select_flush(True)

    def transform(self, matrix, filter=None):
        for v in self.verts:
            v.co = matrix @ v.co

    def calc_volume(self, signed=False):
        vol = 0.0
        for f in self.faces:
            vs = [v.co for v in f.verts]
            for i in range(1, len(vs) - 1):
                vol += vs[0].dot(vs[i].cross(vs[i + 1])) / 6.0
        return vol if signed else abs(vol)

    def calc_loop_triangles(self):
        out = []
        for f in self.faces:
            ls = f.loops
            for i in range(1, len(ls) - 1):
                out.append((ls[0], ls[i], ls[i + 1]))
        return out

    # --- bpy Mesh
    def from_mesh(self, mesh, face_normals=True, vertex_normals=True, use_shape_key=False, shape_key_index=0):
        d = mesh._d()
        base = len(self.verts)
        vs = [self.verts.new(c) for c in d.co]
        for i in d.sel_v:
            if i < len(vs):
                vs[i].select = True
        for i, f in enumerate(d.faces()):
            try:
                face = self.faces.new([vs[k] for k in f])
            except ValueError:
                continue
            face.smooth = d.smooth[i] if i < len(d.smooth) else False
            face.material_index = d.mat[i] if i < len(d.mat) else 0
            face.select = i in d.sel_f
        for a, b in d.edges:
            e = self.edges.get((vs[a], vs[b])) or self.edges.new((vs[a], vs[b]))
            e.seam = tuple(sorted((a, b))) in d.seams
            e.smooth = tuple(sorted((a, b))) not in d.sharp
            e.select = tuple(sorted((a, b))) in d.sel_e or (vs[a].select and vs[b].select)
        if d.uvs is not None:
            lay = self.loops.layers.uv.new(mesh.get('_uv_name', 'UVMap'))
            k = 0
            faces = list(self.faces)[-len(d.totals):] if d.totals else []
            for f in faces:
                for l in f.loops:
                    if k < len(d.uvs):
                        l[lay] = BMLoopUV(d.uvs[k])
                    k += 1
        dl = self.verts.layers.deform.verify() if d.groups else None
        if dl is not None:
            for vi, w in d.weights.items():
                if vi < len(vs):
                    vs[vi][dl] = dict(w)
        self.index_all()
        self._mesh = mesh
        del base

    def index_all(self):
        self.verts.index_update()
        self.edges.index_update()
        self.faces.index_update()
        i = 0
        for f in self.faces:
            for l in f.loops:
                l.index = i
                i += 1

    def to_mesh(self, mesh):
        self.index_all()
        d = mesh._d()
        d.co = [[v.co.x, v.co.y, v.co.z] for v in self.verts]
        faces = [[v.index for v in f.verts] for f in self.faces]
        d.totals = [len(f) for f in faces]
        d.loops = [i for f in faces for i in f]
        d.starts = []
        s = 0
        for t in d.totals:
            d.starts.append(s)
            s += t
        d.edges = [(e.verts[0].index, e.verts[1].index) for e in self.edges]
        d.smooth = [f.smooth for f in self.faces]
        d.mat = [f.material_index for f in self.faces]
        lay = self.loops.layers.uv.active
        if lay is not None:
            d.uvs = [[l[lay].uv[0], l[lay].uv[1]] for f in self.faces for l in f.loops]
        else:
            d.uvs = None
        d.sel_v = set(v.index for v in self.verts if v.select)
        d.sel_e = set(tuple(sorted((e.verts[0].index, e.verts[1].index))) for e in self.edges if e.select)
        d.sel_f = set(f.index for f in self.faces if f.select)
        d.seams = set(tuple(sorted((e.verts[0].index, e.verts[1].index))) for e in self.edges if e.seam)
        d.sharp = set(tuple(sorted((e.verts[0].index, e.verts[1].index))) for e in self.edges if not e.smooth)
        d.keys = []
        mesh._touch('topo')
        mesh._touch('attr')
        mesh._touch('sel')
        mesh._touch('edgeflags')

    def from_object(self, obj, depsgraph, cage=False, face_normals=True, vertex_normals=True):
        self.from_mesh(obj.evaluated_get(depsgraph).to_mesh())


class _Loops:
    def __init__(self, bm):
        self._bm = bm
        self.layers = BMLayerAccess(None)


class _SelectHistory(list):
    @property
    def active(self):
        return self[-1] if self else None

    def add(self, elem):
        self.append(elem)

    def validate(self):
        pass

    def discard(self, elem):
        if elem in self:
            self.remove(elem)


BMEditSelSeq = _SelectHistory
BMLayerItem = _Layer
BMElemSeq = _Seq
