# SPDX-License-Identifier: GPL-3.0-only
"""Blender-flavoured data types (bpy.types) over the polyops stream.

Every object here mirrors part of Polyamour's scene. Reading an attribute reads
the mirror; assigning one updates the mirror and emits the polyops command that
makes the same change in Polyamour. The mirror starts from the scene snapshot
Polyamour passes in POLYAMOUR_SCENE (see polyamour.__init__), or empty.

Objects this script creates are addressed by handles ($1, $2 ...) so they stay
right even when Polyamour gives them a different unique name than predicted.
"""
import math

from .polyops import Raw, mesh_options, quote, stream


# --- math ------------------------------------------------------------------------

class Vector:
    """A small mathutils.Vector: x/y/z/w, indexing, arithmetic. Vectors handed
    out by a datablock (obj.location) write changes straight back to it."""

    __slots__ = ('_v', '_owner')

    def __init__(self, seq=(0.0, 0.0, 0.0), _owner=None):
        object.__setattr__(self, '_v', [float(c) for c in seq])
        object.__setattr__(self, '_owner', _owner)

    def _changed(self):
        if self._owner is not None:
            self._owner(self)

    def _assign(self, seq):
        """Replace the components without telling the owner."""
        vals = [float(c) for c in seq]
        if len(vals) != len(self._v):
            raise ValueError('expected %d values, got %d' % (len(self._v), len(vals)))
        self._v[:] = vals

    def __len__(self):
        return len(self._v)

    def __iter__(self):
        return iter(list(self._v))

    def __getitem__(self, i):
        if isinstance(i, slice):
            return tuple(self._v[i])
        return self._v[i]

    def __setitem__(self, i, value):
        if isinstance(i, slice):
            new = list(self._v)
            new[i] = [float(c) for c in value]
            self._assign(new)
        else:
            self._v[i] = float(value)
        self._changed()

    def _axis(i):
        def get(self):
            return self._v[i]

        def put(self, value):
            self[i] = value
        return property(get, put)

    x = _axis(0)
    y = _axis(1)
    z = _axis(2)
    w = _axis(3)
    r, g, b, a = x, y, z, w
    del _axis

    def __repr__(self):
        return '%s((%s))' % (type(self).__name__, ', '.join('%.4f' % c for c in self._v))

    def __eq__(self, other):
        try:
            return list(self._v) == [float(c) for c in other]
        except TypeError:
            return NotImplemented

    __hash__ = None

    def copy(self):
        return type(self)(self._v)

    def to_tuple(self, precision=-1):
        return tuple(self._v if precision < 0 else (round(c, precision) for c in self._v))

    def _zip(self, other):
        o = list(other)
        if len(o) != len(self._v):
            raise ValueError('vector sizes differ')
        return zip(self._v, o)

    def __add__(self, other):
        return Vector(a + b for a, b in self._zip(other))

    __radd__ = __add__

    def __sub__(self, other):
        return Vector(a - b for a, b in self._zip(other))

    def __rsub__(self, other):
        return Vector(b - a for a, b in self._zip(other))

    def __mul__(self, s):
        if isinstance(s, (int, float)):
            return Vector(c * s for c in self._v)
        return Vector(a * b for a, b in self._zip(s))

    __rmul__ = __mul__

    def __truediv__(self, s):
        return Vector(c / s for c in self._v)

    def __neg__(self):
        return Vector(-c for c in self._v)

    def dot(self, other):
        return sum(a * b for a, b in self._zip(other))

    def cross(self, other):
        a, b = self._v, list(other)
        return Vector((a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]))

    @property
    def length(self):
        return math.sqrt(sum(c * c for c in self._v))

    def normalized(self):
        n = self.length
        return Vector(c / n for c in self._v) if n else Vector(self._v)

    def normalize(self):
        n = self.length
        if n:
            self._assign([c / n for c in self._v])
            self._changed()


class Euler(Vector):
    """XYZ Euler angles in radians (Blender's default rotation mode)."""
    __slots__ = ()
    order = 'XYZ'


class Color(Vector):
    __slots__ = ()


def _vec3(v, what):
    vals = [float(c) for c in v]
    if len(vals) != 3:
        raise ValueError('%s needs 3 values, got %d' % (what, len(vals)))
    return vals


def quat_to_euler(w, x, y, z):
    """XYZ Euler angles of a (w, x, y, z) quaternion, as script.rs computes them."""
    n = math.sqrt(w * w + x * x + y * y + z * z) or 1.0
    w, x, y, z = w / n, x / n, y / n, z / n
    r00 = 1 - 2 * (y * y + z * z)
    r10 = 2 * (x * y + w * z)
    r20 = 2 * (x * z - w * y)
    r21 = 2 * (y * z + w * x)
    r22 = 1 - 2 * (x * x + y * y)
    sy = max(-1.0, min(1.0, -r20))
    if abs(sy) > 0.99999:
        r01 = 2 * (x * y - w * z)
        r11 = 1 - 2 * (x * x + z * z)
        return [math.atan2(math.copysign(1, sy) * r01, r11), math.asin(sy), 0.0]
    return [math.atan2(r21, r22), math.asin(sy), math.atan2(r10, r00)]


def unique_name(base, taken):
    """Blender-style unique name, the same rule as sex-3d's scene::unique_name."""
    if base not in taken:
        return base
    stem, _, num = base.rpartition('.')
    if not (stem and len(num) == 3 and num.isdigit()):
        stem = base
    i = 1
    while '%s.%03d' % (stem, i) in taken:
        i += 1
    return '%s.%03d' % (stem, i)


# --- the mirror ---------------------------------------------------------------------

class _State:
    """Everything the script can see of Polyamour's scene."""

    def __init__(self):
        self.objects = []        # Object, in Polyamour's order
        self.meshes = []         # Mesh datablocks
        self.materials = []
        self.collections = []    # Collection; [0] is the scene's root collection
        self.selected = []
        self.active = None
        self.camera = None
        self.scene_name = 'Scene'
        self.frame_start, self.frame_end, self.fps = 1, 250, 24.0
        self.world_color = [0.05, 0.05, 0.05]
        self.world_strength = 1.0
        self.render = {}

    def object_names(self):
        return {o.name for o in self.objects}


state = _State()


class ID:
    """A named datablock. `_handle` is set for what this script created."""

    _kind = 'ID'

    def __init__(self, name):
        self._name = name
        self._handle = None

    def _ref(self):
        return str(self._handle) if self._handle is not None else quote(self._name)

    @property
    def name(self):
        return self._name

    def __repr__(self):
        return '<%s %r>' % (type(self).__name__, self._name)

    @property
    def users(self):
        return 1


# --- materials -------------------------------------------------------------------------

class _Socket:
    def __init__(self, mat, attr, size):
        self._mat, self._attr, self._size = mat, attr, size

    @property
    def default_value(self):
        return getattr(self._mat, self._attr)

    @default_value.setter
    def default_value(self, v):
        if self._size == 3 and not isinstance(v, (int, float)):
            v = list(v)[:3]
        setattr(self._mat, self._attr, v)


class _Principled:
    """Just enough of a node tree for `nodes["Principled BSDF"].inputs[...]`."""

    _INPUTS = {
        'Base Color': ('base_color', 4), 'Metallic': ('metallic', 1), 'Roughness': ('roughness', 1),
        'IOR': ('ior', 1), 'Transmission': ('transmission', 1), 'Transmission Weight': ('transmission', 1),
        'Emission': ('emission_color', 3), 'Emission Color': ('emission_color', 3),
        'Emission Strength': ('emission_strength', 1), 'Alpha': ('alpha', 1),
    }
    name = type = 'Principled BSDF'

    def __init__(self, mat):
        self._mat = mat

    @property
    def inputs(self):
        mat = self._mat

        class Inputs:
            def __getitem__(_, key):
                try:
                    attr, size = _Principled._INPUTS[key]
                except KeyError:
                    raise KeyError('Polyamour materials have no %r input' % key) from None
                return _Socket(mat, attr, size)

            def get(self_, key, default=None):
                try:
                    return self_[key]
                except KeyError:
                    return default
        return Inputs()


class _NodeTree:
    def __init__(self, mat):
        self.nodes = {'Principled BSDF': _Principled(mat)}
        self.links = []


class Material(ID):
    """Principled material. `diffuse_color` and `base_color` are the same RGBA."""

    _kind = 'Material'

    def __init__(self, name):
        super().__init__(name)
        self._v = {'base_color': [0.8, 0.8, 0.8, 1.0], 'metallic': 0.0, 'roughness': 0.5,
                   'emission': [0.0, 0.0, 0.0], 'emission_strength': 0.0, 'ior': 1.5,
                   'transmission': 0.0, 'double_sided': False}
        self.use_nodes = True
        self.node_tree = _NodeTree(self)
        self.blend_method = 'OPAQUE'

    def _set(self, key, value):
        self._v[key] = value
        stream.emit('material_set', Raw(self._ref()), **{key: value})

    def _color(self):
        return Vector(self._v['base_color'], _owner=lambda v: self._set('base_color', list(v)))

    @property
    def base_color(self):
        return self._color()

    @base_color.setter
    def base_color(self, c):
        c = [float(x) for x in c]
        if len(c) == 3:
            c.append(self._v['base_color'][3])
        if len(c) != 4:
            raise ValueError('a colour needs 3 or 4 values')
        self._set('base_color', c)

    diffuse_color = base_color

    @property
    def alpha(self):
        return self._v['base_color'][3]

    @alpha.setter
    def alpha(self, a):
        self.base_color = self._v['base_color'][:3] + [float(a)]

    @property
    def emission_color(self):
        return Vector(self._v['emission'], _owner=lambda v: self._set('emission', list(v)))

    @emission_color.setter
    def emission_color(self, c):
        self._set('emission', _vec3(list(c)[:3], 'emission_color'))

    def _scalar(key, kind=float):
        def get(self):
            return self._v[key]

        def put(self, v):
            self._set(key, kind(v))
        return property(get, put)

    metallic = _scalar('metallic')
    roughness = _scalar('roughness')
    emission_strength = _scalar('emission_strength')
    ior = _scalar('ior')
    transmission = _scalar('transmission')
    double_sided = _scalar('double_sided', bool)
    use_backface_culling = property(lambda self: not self._v['double_sided'],
                                    lambda self, v: self._set('double_sided', not v))
    del _scalar

    @ID.name.setter
    def name(self, new):
        taken = {m.name for m in state.materials if m is not self}
        new = unique_name(str(new), taken)
        stream.emit('material_set', Raw(self._ref()), name=new)
        self._name = new


# --- meshes ----------------------------------------------------------------------------

class MeshVertex:
    def __init__(self, mesh, index):
        self._mesh, self.index = mesh, index

    @property
    def co(self):
        mesh, i = self._mesh, self.index

        def write(v):
            mesh._verts[i] = tuple(v)
            stream.emit('mesh_edit', Raw(mesh._ref()), Raw('set_verts'), indices=i, verts=list(v))
        return Vector(mesh._verts[i], _owner=write)

    @co.setter
    def co(self, v):
        v = _vec3(v, 'co')
        self._mesh._verts[self.index] = tuple(v)
        stream.emit('mesh_edit', Raw(self._mesh._ref()), Raw('set_verts'), indices=self.index, verts=v)


class MeshPolygon:
    def __init__(self, mesh, index):
        self._mesh, self.index = mesh, index

    @property
    def vertices(self):
        return tuple(self._mesh._faces[self.index])


class _MeshMaterials:
    """mesh.materials: Polyamour keeps slots on the objects that use the mesh,
    so changes here go to every user."""

    def __init__(self, mesh):
        self._mesh = mesh
        self._slots = []

    def append(self, mat):
        self._slots.append(mat)
        for o in self._mesh._users:
            o._slots.append(mat)
            stream.emit('material_assign', Raw(o._ref()), Raw(mat._ref()))

    def clear(self):
        self._slots.clear()
        for o in self._mesh._users:
            o._slots.clear()
            stream.emit('material_clear', Raw(o._ref()))

    def __getitem__(self, i):
        return self._slots[i]

    def __setitem__(self, i, mat):
        self._slots[i] = mat
        i = range(len(self._slots))[i]
        for o in self._mesh._users:
            while len(o._slots) <= i:
                o._slots.append(mat)
            o._slots[i] = mat
            stream.emit('material_assign', Raw(o._ref()), Raw(mat._ref()), slot=i)

    def __len__(self):
        return len(self._slots)

    def __iter__(self):
        return iter(list(self._slots))


class Mesh(ID):
    """Mesh datablock. Geometry is known for meshes made with from_pydata and
    for those in the scene snapshot; primitives added by operators report none."""

    _kind = 'Mesh'

    def __init__(self, name):
        super().__init__(name)
        self._verts, self._edges, self._faces = [], [], []
        self._users = []
        self._owner = None   # the object a primitive's mesh is addressed through
        self.materials = _MeshMaterials(self)

    def _ref(self):
        if self._handle is None and self._owner is not None:
            return self._owner._ref()
        return super()._ref()

    def from_pydata(self, vertices, edges, faces, shade_flat=True):
        opts = mesh_options(vertices, edges, faces)
        self._verts = [tuple(float(c) for c in v) for v in vertices]
        self._edges = [tuple(int(i) for i in e) for e in edges]
        self._faces = [tuple(int(i) for i in f) for f in faces]
        stream.emit('mesh_set', Raw(self._ref()), **opts)

    def update(self, *args, **kwargs):
        pass

    def validate(self, *args, **kwargs):
        return False

    @property
    def vertices(self):
        return [MeshVertex(self, i) for i in range(len(self._verts))]

    @property
    def polygons(self):
        return [MeshPolygon(self, i) for i in range(len(self._faces))]

    @property
    def edges(self):
        return list(self._edges)

    def _edit(self, op, *args, **opts):
        stream.emit('mesh_edit', Raw(self._ref()), Raw(op), *args, **opts)

    def shade_smooth(self):
        self._edit('shade_smooth')

    def shade_flat(self):
        self._edit('shade_flat')

    def flip_normals(self):
        self._edit('flip_normals')
        self._faces = [tuple(reversed(f)) for f in self._faces]

    def translate(self, offset):
        d = _vec3(offset, 'offset')
        self._edit('translate_verts', *d)
        self._verts = [tuple(a + b for a, b in zip(v, d)) for v in self._verts]

    @property
    def users(self):
        return len(self._users)


# --- lights and cameras ----------------------------------------------------------------------

class Light(ID):
    """Light data; in Polyamour it lives inside one object (the first user)."""

    _kind = 'Light'
    _TYPES = {'POINT': 'point', 'SUN': 'sun', 'SPOT': 'spot', 'AREA': 'area'}

    def __init__(self, name, type='POINT'):
        super().__init__(name)
        if type not in self._TYPES:
            raise ValueError('light type must be one of %s' % ', '.join(self._TYPES))
        object.__setattr__(self, 'type', type)
        self._object = None
        self._v = {'power': 1.0 if type == 'SUN' else 1000.0, 'color': [1.0, 1.0, 1.0], 'radius': 0.1,
                   'angle': math.radians(45), 'blend': 0.15, 'size': 1.0}

    def _set(self, key, value):
        self._v[key] = value
        if self._object is not None:
            stream.emit('set', Raw(self._object._ref() + '.' + key), value)

    def __setattr__(self, key, value):
        if key == 'type' and getattr(self, '_object', None) is not None:
            raise AttributeError('Polyamour cannot change the type of a light in use')
        object.__setattr__(self, key, value)

    def _opts(self):
        o = {'power': self._v['power'], 'color': self._v['color'], 'radius': self._v['radius']}
        if self.type == 'SPOT':
            o.update(angle=self._v['angle'], blend=self._v['blend'])
        if self.type == 'AREA':
            o['size'] = self._v['size']
        return o

    def _prop(key, kind=float):
        def get(self):
            return self._v[key]

        def put(self, v):
            self._set(key, kind(v))
        return property(get, put)

    energy = _prop('power')
    shadow_soft_size = _prop('radius')
    spot_size = _prop('angle')
    spot_blend = _prop('blend')
    size = _prop('size')
    del _prop

    @property
    def color(self):
        return Color(self._v['color'], _owner=lambda v: self._set('color', list(v)))

    @color.setter
    def color(self, c):
        self._set('color', _vec3(c, 'color'))


class Camera(ID):
    _kind = 'Camera'

    def __init__(self, name):
        super().__init__(name)
        self._object = None
        self._v = {'fov': 2 * math.atan(18.0 / 50.0 * 9.0 / 16.0), 'near': 0.1, 'far': 1000.0, 'ortho_scale': None}

    def _set(self, key, value):
        self._v[key] = value
        if self._object is not None:
            stream.emit('set', Raw(self._object._ref() + '.' + key), Raw('none') if value is None else value)

    @property
    def angle(self):
        return self._v['fov']

    @angle.setter
    def angle(self, a):
        self._set('fov', float(a))

    @property
    def lens(self):
        return 18.0 * 9.0 / 16.0 / math.tan(self._v['fov'] / 2)

    @lens.setter
    def lens(self, mm):
        self._set('fov', 2 * math.atan(18.0 / float(mm) * 9.0 / 16.0))

    clip_start = property(lambda self: self._v['near'], lambda self, v: self._set('near', float(v)))
    clip_end = property(lambda self: self._v['far'], lambda self, v: self._set('far', float(v)))

    @property
    def type(self):
        return 'PERSP' if self._v['ortho_scale'] is None else 'ORTHO'

    @type.setter
    def type(self, t):
        if t == 'ORTHO':
            self._set('ortho_scale', self._v['ortho_scale'] or 6.0)
        elif t == 'PERSP':
            self._set('ortho_scale', None)
        else:
            raise ValueError("camera type must be 'PERSP' or 'ORTHO'")

    @property
    def ortho_scale(self):
        return self._v['ortho_scale'] or 6.0

    @ortho_scale.setter
    def ortho_scale(self, s):
        self._set('ortho_scale', float(s))

    def _opts(self):
        return {'fov': self._v['fov'], 'near': self._v['near'], 'far': self._v['far'], 'ortho_scale': self._v['ortho_scale']}


# --- modifiers -------------------------------------------------------------------------------

# Blender modifier type -> (polyops type, {Blender attribute: default}).
_MODIFIERS = {
    'SUBSURF': ('Subdivision', {'levels': 1, 'render_levels': 2, 'subdivision_type': 'CATMULL_CLARK'}),
    'MIRROR': ('Mirror', {'use_axis': [True, False, False], 'use_mirror_merge': True, 'merge_threshold': 0.001}),
    'ARRAY': ('Array', {'count': 2, 'use_relative_offset': True, 'relative_offset_displace': [1.0, 0.0, 0.0],
                        'use_constant_offset': False, 'constant_offset_displace': [0.0, 0.0, 0.0],
                        'use_merge_vertices': False}),
    'SOLIDIFY': ('Solidify', {'thickness': 0.01, 'offset': -1.0}),
    'BEVEL': ('Bevel', {'width': 0.1, 'segments': 1, 'angle_limit': math.radians(30)}),
    'TRIANGULATE': ('Triangulate', {}),
    'WELD': ('Weld', {'merge_threshold': 0.001}),
    'DISPLACE': ('Displace', {'strength': 1.0}),
    'SMOOTH': ('Smooth', {'factor': 0.5, 'iterations': 1}),
}
_POLYOPS_TO_BLENDER = {v[0].lower(): k for k, v in _MODIFIERS.items()}


def _modifier_options(kind, p):
    """polyops options for every Blender attribute of a modifier."""
    if kind == 'SUBSURF':
        return {'levels': int(p['levels']), 'simple': p['subdivision_type'] == 'SIMPLE'}
    if kind == 'MIRROR':
        x, y, z = (bool(a) for a in p['use_axis'])
        return {'x': x, 'y': y, 'z': z, 'merge': bool(p['use_mirror_merge']), 'merge_distance': float(p['merge_threshold'])}
    if kind == 'ARRAY':
        rel = list(p['relative_offset_displace']) if p['use_relative_offset'] else [0.0, 0.0, 0.0]
        const = list(p['constant_offset_displace']) if p['use_constant_offset'] else [0.0, 0.0, 0.0]
        return {'count': int(p['count']), 'relative': rel, 'constant': const, 'merge': bool(p['use_merge_vertices'])}
    if kind == 'SOLIDIFY':
        return {'thickness': float(p['thickness']), 'offset': float(p['offset'])}
    if kind == 'BEVEL':
        return {'width': float(p['width']), 'segments': int(p['segments']), 'angle': float(p['angle_limit'])}
    if kind == 'WELD':
        return {'distance': float(p['merge_threshold'])}
    if kind == 'DISPLACE':
        return {'strength': float(p['strength'])}
    if kind == 'SMOOTH':
        return {'factor': float(p['factor']), 'iterations': int(p['iterations'])}
    return {}


class _AxisFlags(list):
    """Mirror's use_axis: setting an item updates the modifier."""

    def __init__(self, values, mod):
        super().__init__(bool(v) for v in values)
        self._mod = mod

    def __setitem__(self, i, v):
        super().__setitem__(i, bool(v))
        self._mod._push()


class Modifier:
    """A modifier on an object. Known Blender attributes are sent to Polyamour;
    others (show_viewport, quality, ...) are kept here and otherwise ignored."""

    def __init__(self, obj, name, kind, params):
        d = self.__dict__
        d['_object'], d['name'], d['type'] = obj, name, kind
        d['_p'] = dict(params)
        if kind == 'MIRROR':
            d['_p']['use_axis'] = _AxisFlags(params['use_axis'], self)

    @property
    def _index(self):
        return self._object.modifiers._list.index(self)

    def _push(self):
        stream.emit('modifier_set', Raw(self._object._ref()), self._index, **_modifier_options(self.type, self._p))

    def __getattr__(self, key):
        p = self.__dict__['_p']
        if key in p:
            return p[key]
        raise AttributeError(key)

    def __setattr__(self, key, value):
        if key in ('name', 'show_viewport', 'show_render', 'show_in_editmode', 'show_on_cage', 'quality') or key not in self._p:
            self.__dict__[key] = value
            return
        if key == 'use_axis':
            value = _AxisFlags(value, self)
        elif isinstance(value, (list, tuple, Vector)):
            value = [float(c) for c in value]
        self._p[key] = value
        self._push()

    def __repr__(self):
        return '<Modifier %r %s>' % (self.name, self.type)


class ObjectModifiers:
    def __init__(self, obj):
        self._object = obj
        self._list = []

    def new(self, name, type):
        kind = str(type).upper()
        if kind not in _MODIFIERS:
            kind = _POLYOPS_TO_BLENDER.get(str(type).lower(), kind)
        if kind not in _MODIFIERS:
            raise TypeError('Polyamour has no %r modifier (it has %s)' % (type, ', '.join(_MODIFIERS)))
        polyops_type, defaults = _MODIFIERS[kind]
        stream.emit('modifier_add', Raw(self._object._ref()), Raw(polyops_type))
        m = Modifier(self._object, name, kind, {k: list(v) if isinstance(v, list) else v for k, v in defaults.items()})
        self._list.append(m)
        return m

    def remove(self, mod):
        i = self._list.index(mod)
        stream.emit('modifier_remove', Raw(self._object._ref()), i)
        del self._list[i]

    def clear(self):
        stream.emit('modifier_clear', Raw(self._object._ref()))
        self._list.clear()

    def get(self, name, default=None):
        return next((m for m in self._list if m.name == name), default)

    def __getitem__(self, key):
        if isinstance(key, str):
            m = self.get(key)
            if m is None:
                raise KeyError(key)
            return m
        return self._list[key]

    def __len__(self):
        return len(self._list)

    def __iter__(self):
        return iter(list(self._list))


# --- objects -------------------------------------------------------------------------------

class MaterialSlot:
    def __init__(self, obj, i):
        self._obj, self._i = obj, i

    @property
    def material(self):
        return self._obj._slots[self._i]

    @material.setter
    def material(self, mat):
        self._obj._slots[self._i] = mat
        stream.emit('material_assign', Raw(self._obj._ref()), Raw(mat._ref()), slot=self._i)


class Object(ID):
    _kind = 'Object'

    def __init__(self, name, data=None):
        super().__init__(name)
        self.data = data
        self._loc = Vector((0, 0, 0), _owner=lambda v: self._emit_set('location', v))
        self._rot = Euler((0, 0, 0), _owner=lambda v: self._emit_set('rotation_euler', v))
        self._scale = Vector((1, 1, 1), _owner=lambda v: self._emit_set('scale', v))
        self._parent = None
        self._collection = None
        self._visible = True
        self._slots = []
        self.modifiers = ObjectModifiers(self)
        self.rotation_mode = 'XYZ'

    def _emit_set(self, prop, value):
        stream.emit('set', Raw(self._ref() + '.' + prop), *[float(c) for c in value])

    @property
    def type(self):
        if isinstance(self.data, Mesh):
            return 'MESH'
        if isinstance(self.data, Light):
            return 'LIGHT'
        if isinstance(self.data, Camera):
            return 'CAMERA'
        return 'EMPTY'

    @ID.name.setter
    def name(self, new):
        new = str(new)
        if not new:
            raise ValueError('an object name cannot be empty')
        new = unique_name(new, {o.name for o in state.objects if o is not self})
        stream.emit('set', Raw(self._ref() + '.name'), new)
        self._name = new

    def _vector(attr, prop):
        def get(self):
            return getattr(self, attr)

        def put(self, value):
            vals = [float(c) for c in value]
            if prop == 'scale' and len(vals) == 1:
                vals = vals * 3
            getattr(self, attr)._assign(vals)
            self._emit_set(prop, vals)
        return property(get, put)

    location = _vector('_loc', 'location')
    rotation_euler = _vector('_rot', 'rotation_euler')
    scale = _vector('_scale', 'scale')
    del _vector

    @property
    def parent(self):
        return self._parent

    @parent.setter
    def parent(self, p):
        q = p
        while q is not None:
            if q is self:
                raise ValueError('parenting %s to %s would make a cycle' % (self.name, p.name))
            q = q._parent
        stream.emit('parent', Raw(self._ref()), Raw('none') if p is None else Raw(p._ref()))
        self._parent = p

    @property
    def children(self):
        return tuple(o for o in state.objects if o._parent is self)

    # visibility and selection

    def hide_set(self, state_):
        self._set_visible(not state_)

    def hide_get(self):
        return not self._visible

    def visible_get(self):
        return self._visible

    def _set_visible(self, v):
        self._visible = bool(v)
        stream.emit('set', Raw(self._ref() + '.visible'), self._visible)

    hide_viewport = property(lambda self: not self._visible, lambda self, v: self._set_visible(not v))
    hide_render = hide_viewport

    def select_set(self, flag):
        if flag and self not in state.selected:
            state.selected.append(self)
            stream.emit('select_add', Raw(self._ref()))
        elif not flag and self in state.selected:
            state.selected.remove(self)
            stream.emit('deselect', Raw(self._ref()))

    def select_get(self):
        return self in state.selected

    # materials

    @property
    def material_slots(self):
        return [MaterialSlot(self, i) for i in range(len(self._slots))]

    @property
    def active_material(self):
        return self._slots[0] if self._slots else None

    @active_material.setter
    def active_material(self, mat):
        if self._slots:
            self._slots[0] = mat
        else:
            self._slots.append(mat)
        stream.emit('material_assign', Raw(self._ref()), Raw(mat._ref()), slot=0)

    @property
    def users_collection(self):
        return (self._collection,) if self._collection is not None else ()


# --- collections ------------------------------------------------------------------------------

class _CollectionObjects:
    def __init__(self, coll):
        self._coll = coll

    def link(self, obj):
        """Put `obj` in this collection (a Polyamour object is in one collection)."""
        obj._collection = self._coll
        stream.emit('link', Raw(obj._ref()), Raw(self._coll._target()._ref()))

    def unlink(self, obj):
        if obj._collection is self._coll:
            root = state.collections[0]
            obj._collection = root
            stream.emit('link', Raw(obj._ref()), Raw(root._ref()))

    def __iter__(self):
        target = self._coll._target()
        return iter([o for o in state.objects if o._collection is not None and o._collection._target() is target])

    def __len__(self):
        return len(list(iter(self)))

    def __getitem__(self, key):
        for o in self:
            if o.name == key:
                return o
        raise KeyError(key)

    def __contains__(self, key):
        return any(o.name == key or o is key for o in self)


class _CollectionChildren:
    def __init__(self, coll):
        self._coll = coll

    def link(self, child):
        child._parent = self._coll if not self._coll._is_master else None
        stream.emit('collection_set', Raw(child._ref()), parent=Raw('none') if child._parent is None else Raw(child._parent._ref()))

    def unlink(self, child):
        child._parent = None
        stream.emit('collection_set', Raw(child._ref()), parent=Raw('none'))

    def __iter__(self):
        me = None if self._coll._is_master else self._coll
        return iter([c for c in state.collections[1:] if c._parent is me])

    def __len__(self):
        return len(list(iter(self)))


class Collection(ID):
    _kind = 'Collection'

    def __init__(self, name, _is_master=False):
        super().__init__(name)
        self._parent = None
        self._visible = True
        self._is_master = _is_master
        self.objects = _CollectionObjects(self)
        self.children = _CollectionChildren(self)

    def _target(self):
        """The Polyamour collection objects go to: the master collection is the
        scene's root collection."""
        return state.collections[0] if self._is_master else self

    @property
    def all_objects(self):
        return list(self.objects)

    @property
    def hide_viewport(self):
        return not self._visible

    @hide_viewport.setter
    def hide_viewport(self, v):
        self._visible = not v
        stream.emit('collection_set', Raw(self._target()._ref()), visible=self._visible)

    @ID.name.setter
    def name(self, new):
        if self._is_master:
            raise AttributeError('the scene collection cannot be renamed')
        new = unique_name(str(new), {c.name for c in state.collections if c is not self})
        stream.emit('collection_set', Raw(self._ref()), name=new)
        self._name = new
