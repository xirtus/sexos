# SPDX-License-Identifier: GPL-3.0-only
"""polyamour: scripting Polyamour, SexOS's 3D suite, with a bpy-like API.

    import polyamour as pa
    pa.ops.mesh.primitive_cube_add(size=2, location=(0, 0, 1))
    cube = pa.context.active_object
    cube.rotation_euler.z = 0.5
    red = pa.data.materials.new('Red')
    red.base_color = (1, 0, 0, 1)
    cube.data.materials.append(red)
    cube.modifiers.new('Smooth', type='SUBSURF').levels = 2

`import bpy` gives the same module, so simple Blender scripts run unchanged.

A script runs in its own `python` process. Every change is written as a polyops
command (see polyamour.polyops) to the file named by POLYAMOUR_OPS, or to
standard output; Polyamour applies the whole stream as one undoable step when
the script ends, or nothing if it raised. What the script reads (names,
locations, selection...) comes from a mirror of the scene: Polyamour writes its
current scene as polyops text to the file named by POLYAMOUR_SCENE, which is
loaded here at import. Without it the script starts from an empty scene.
"""
import os

from . import polyops
from . import types
from .polyops import Raw, stream
from .types import (
    Camera, Collection, Color, Euler, Light, Material, Mesh, Modifier, Object, Vector,
    quat_to_euler, state, unique_name,
)

__all__ = ['data', 'context', 'ops', 'types', 'Vector', 'Euler', 'Color', 'flush', 'polyops']


def flush():
    """Push the commands written so far to the stream's file."""
    stream.flush()


# --- adding and removing objects --------------------------------------------------------

def _place(obj, location=None, rotation=None, scale=None):
    if location is not None:
        obj._loc._assign(types._vec3(location, 'location'))
    if rotation is not None:
        obj._rot._assign(types._vec3(rotation, 'rotation'))
    if scale is not None:
        s = [float(c) for c in scale] if not isinstance(scale, (int, float)) else [float(scale)] * 3
        obj._scale._assign(s)


def _placement(obj):
    return {'location': list(obj._loc), 'rotation': list(obj._rot), 'scale': list(obj._scale)}


def _register(obj):
    """Mirror Polyamour's add rule: the new object is active and the only selected one."""
    obj._collection = state.collections[0]
    state.objects.append(obj)
    state.selected[:] = [obj]
    state.active = obj
    return obj


def _new_object(name, data):
    obj = Object(unique_name(str(name), state.object_names()), data)
    obj._handle = stream.handle()
    return obj


def _add_mesh_object(command, kind, name, opts, location, rotation, scale):
    obj = _new_object(name, None)
    _place(obj, location, rotation, scale)
    mesh = Mesh(unique_name(obj.name, {m.name for m in state.meshes}))
    mesh._owner = obj
    mesh._users.append(obj)
    state.meshes.append(mesh)
    obj.data = mesh
    stream.emit(command, Raw(kind), name=obj.name, as_=obj._handle, **opts, **_placement(obj))
    return _register(obj)


def _link_object(name, data, location=None, rotation=None, scale=None):
    """bpy.data.objects.new: add the object at once (Polyamour has no unlinked objects)."""
    if isinstance(data, (Light, Camera)) and data._object is not None:
        raise ValueError('%s is already used by %s; Polyamour lights and cameras have one user' % (data.name, data._object.name))
    obj = _new_object(name, data)
    _place(obj, location, rotation, scale)
    where = _placement(obj)
    if data is None:
        stream.emit('add_empty', obj.name, as_=obj._handle, **where)
    elif isinstance(data, Mesh):
        stream.emit('add_object', obj.name, data=Raw(data._ref()), as_=obj._handle, **where)
        data._users.append(obj)
        for mat in data.materials:
            obj._slots.append(mat)
            stream.emit('material_assign', obj._handle, Raw(mat._ref()))
    elif isinstance(data, Light):
        stream.emit('light_add', Raw(Light._TYPES[data.type]), name=obj.name, as_=obj._handle, **data._opts(), **where)
        data._object = obj
    elif isinstance(data, Camera):
        stream.emit('camera_add', name=obj.name, as_=obj._handle, **data._opts(), **where)
        data._object = obj
        if state.camera is None:
            state.camera = obj
    else:
        raise TypeError('object data must be a Mesh, Light, Camera or None, not %r' % (data,))
    return _register(obj)


def _remove_object(obj):
    if obj not in state.objects:
        raise ReferenceError('%r was removed already' % (obj,))
    stream.emit('delete', Raw(obj._ref()))
    for child in state.objects:
        if child._parent is obj:
            child._parent = obj._parent
    state.objects.remove(obj)
    if obj in state.selected:
        state.selected.remove(obj)
    if state.active is obj:
        state.active = None
    if state.camera is obj:
        state.camera = None
    if isinstance(obj.data, Mesh) and obj in obj.data._users:
        obj.data._users.remove(obj)


# --- bpy.data --------------------------------------------------------------------------------

class _IDList:
    """Name-indexed list of datablocks (bpy.data.objects and friends)."""

    def __init__(self, items):
        self._items = items

    def _all(self):
        return self._items() if callable(self._items) else self._items

    def get(self, name, default=None):
        return next((x for x in self._all() if x.name == name), default)

    def __getitem__(self, key):
        if isinstance(key, int):
            return self._all()[key]
        x = self.get(key)
        if x is None:
            raise KeyError('%s not found' % key)
        return x

    def __contains__(self, key):
        return any(x is key or x.name == key for x in self._all())

    def __iter__(self):
        return iter(list(self._all()))

    def __len__(self):
        return len(self._all())

    def keys(self):
        return [x.name for x in self._all()]

    def values(self):
        return list(self._all())

    def items(self):
        return [(x.name, x) for x in self._all()]

    def find(self, name):
        return next((i for i, x in enumerate(self._all()) if x.name == name), -1)


class _Objects(_IDList):
    def new(self, name, object_data):
        return _link_object(name, object_data)

    def remove(self, obj, do_unlink=True, do_id_user=True, do_ui_user=True):
        _remove_object(obj)


class _Meshes(_IDList):
    def new(self, name):
        mesh = Mesh(unique_name(str(name), {m.name for m in state.meshes}))
        mesh._handle = stream.handle()
        stream.emit('mesh_new', mesh.name, as_=mesh._handle)
        state.meshes.append(mesh)
        return mesh


class _Materials(_IDList):
    def new(self, name):
        mat = Material(unique_name(str(name), {m.name for m in state.materials}))
        mat._handle = stream.handle()
        stream.emit('material_new', mat.name, as_=mat._handle)
        state.materials.append(mat)
        return mat


class _Collections(_IDList):
    def new(self, name):
        coll = Collection(unique_name(str(name), {c.name for c in state.collections}))
        coll._handle = stream.handle()
        stream.emit('collection_new', coll.name, as_=coll._handle)
        state.collections.append(coll)
        return coll


class _Lights(_IDList):
    def new(self, name, type='POINT'):
        return Light(str(name), type)


class _Cameras(_IDList):
    def new(self, name):
        return Camera(str(name))


class BlendData:
    """bpy.data."""

    def __init__(self):
        self.objects = _Objects(lambda: state.objects)
        self.meshes = _Meshes(lambda: state.meshes)
        self.materials = _Materials(lambda: state.materials)
        self.collections = _Collections(lambda: state.collections)
        self.lights = _Lights(lambda: [o.data for o in state.objects if isinstance(o.data, Light)])
        self.cameras = _Cameras(lambda: [o.data for o in state.objects if isinstance(o.data, Camera)])
        self.scenes = _IDList(lambda: [context.scene])


data = BlendData()


# --- bpy.context ----------------------------------------------------------------------------

class RenderSettings:
    """scene.render: any setting is stored by Polyamour as text (`render k=v`)."""

    _DEFAULTS = {'resolution_x': 1920, 'resolution_y': 1080, 'resolution_percentage': 100,
                 'engine': 'CYCLES', 'filepath': '/tmp/', 'fps': 24}

    def __getattr__(self, key):
        if key.startswith('_'):
            raise AttributeError(key)
        return state.render.get(key, self._DEFAULTS.get(key))

    def __setattr__(self, key, value):
        if isinstance(value, (list, tuple, Vector)):
            value = Raw(polyops.fmt_list(value))
        state.render[key] = value
        stream.emit('render', **{key: value})


class World:
    """scene.world: background colour and strength."""

    name = 'World'
    use_nodes = False

    @property
    def color(self):
        return Color(state.world_color, _owner=lambda v: self._set(color=list(v)))

    @color.setter
    def color(self, c):
        self._set(color=types._vec3(list(c)[:3], 'color'))

    @property
    def strength(self):
        return state.world_strength

    @strength.setter
    def strength(self, s):
        self._set(strength=float(s))

    def _set(self, color=None, strength=None):
        if color is not None:
            state.world_color = list(color)
        if strength is not None:
            state.world_strength = strength
        stream.emit('world', color=color, strength=strength)


class _Cursor:
    def __init__(self):
        self.location = Vector((0, 0, 0))
        self.rotation_euler = Euler((0, 0, 0))


class Scene:
    """bpy.context.scene (Polyamour documents hold one scene)."""

    def __init__(self):
        self.render = RenderSettings()
        self.world = World()
        self.cursor = _Cursor()
        self.collection = Collection('Scene Collection', _is_master=True)

    @property
    def name(self):
        return state.scene_name

    @name.setter
    def name(self, n):
        state.scene_name = str(n)
        stream.emit('scene', name=state.scene_name)

    @property
    def objects(self):
        return data.objects

    @property
    def camera(self):
        return state.camera

    @camera.setter
    def camera(self, obj):
        if obj is not None and obj.type != 'CAMERA':
            raise TypeError('%s is not a camera' % obj.name)
        state.camera = obj
        stream.emit('active_camera', Raw('none') if obj is None else Raw(obj._ref()))

    def _frame(key):
        def get(self):
            return getattr(state, key)

        def put(self, v):
            v = float(v) if key == 'fps' else int(v)
            setattr(state, key, v)
            stream.emit('scene', **{key: v})
        return property(get, put)

    frame_start = _frame('frame_start')
    frame_end = _frame('frame_end')
    del _frame


class _LayerObjects:
    @property
    def active(self):
        return state.active

    @active.setter
    def active(self, obj):
        state.active = obj
        stream.emit('active', Raw('none') if obj is None else Raw(obj._ref()))

    def __iter__(self):
        return iter(list(state.objects))

    def __len__(self):
        return len(state.objects)


class _ViewLayer:
    def __init__(self):
        self.objects = _LayerObjects()

    def update(self):
        pass


class Context:
    """bpy.context."""

    def __init__(self):
        self.scene = Scene()
        self.view_layer = _ViewLayer()

    @property
    def active_object(self):
        return state.active

    @active_object.setter
    def active_object(self, obj):
        self.view_layer.objects.active = obj

    object = active_object

    @property
    def selected_objects(self):
        return list(state.selected)

    @property
    def collection(self):
        return self.scene.collection

    @property
    def mode(self):
        return 'OBJECT'


context = Context()


# --- bpy.ops ---------------------------------------------------------------------------------

_IGNORED_OP_ARGS = {'enter_editmode', 'align', 'calc_uvs', 'generate_uvs', 'end_fill_type', 'fill_type'}


def _check(kw):
    bad = set(kw) - _IGNORED_OP_ARGS
    if bad:
        raise TypeError('unexpected keyword argument(s): %s' % ', '.join(sorted(bad)))


class _MeshOps:
    """bpy.ops.mesh: the primitive operators. Each returns {'FINISHED'} and
    leaves the new object active, like Blender."""

    @staticmethod
    def primitive_cube_add(size=2.0, location=(0, 0, 0), rotation=(0, 0, 0), scale=(1, 1, 1), **kw):
        _check(kw)
        _add_mesh_object('add_primitive', 'cube', 'Cube', {'size': size}, location, rotation, scale)
        return {'FINISHED'}

    @staticmethod
    def primitive_plane_add(size=2.0, location=(0, 0, 0), rotation=(0, 0, 0), scale=(1, 1, 1), **kw):
        _check(kw)
        _add_mesh_object('add_primitive', 'plane', 'Plane', {'size': size}, location, rotation, scale)
        return {'FINISHED'}

    @staticmethod
    def primitive_grid_add(x_subdivisions=10, y_subdivisions=10, size=2.0, location=(0, 0, 0), rotation=(0, 0, 0), scale=(1, 1, 1), **kw):
        _check(kw)
        opts = {'x_subdivisions': int(x_subdivisions), 'y_subdivisions': int(y_subdivisions), 'size': size}
        _add_mesh_object('add_primitive', 'grid', 'Grid', opts, location, rotation, scale)
        return {'FINISHED'}

    @staticmethod
    def primitive_uv_sphere_add(segments=32, ring_count=16, radius=1.0, location=(0, 0, 0), rotation=(0, 0, 0), scale=(1, 1, 1), **kw):
        _check(kw)
        opts = {'segments': int(segments), 'rings': int(ring_count), 'radius': radius}
        _add_mesh_object('add_primitive', 'uv_sphere', 'Sphere', opts, location, rotation, scale)
        return {'FINISHED'}

    @staticmethod
    def primitive_ico_sphere_add(subdivisions=2, radius=1.0, location=(0, 0, 0), rotation=(0, 0, 0), scale=(1, 1, 1), **kw):
        _check(kw)
        _add_mesh_object('add_primitive', 'ico_sphere', 'Icosphere', {'subdivisions': int(subdivisions), 'radius': radius}, location, rotation, scale)
        return {'FINISHED'}

    @staticmethod
    def primitive_cylinder_add(vertices=32, radius=1.0, depth=2.0, location=(0, 0, 0), rotation=(0, 0, 0), scale=(1, 1, 1), **kw):
        _check(kw)
        _add_mesh_object('add_primitive', 'cylinder', 'Cylinder', {'segments': int(vertices), 'radius': radius, 'depth': depth}, location, rotation, scale)
        return {'FINISHED'}

    @staticmethod
    def primitive_cone_add(vertices=32, radius1=1.0, radius2=0.0, depth=2.0, location=(0, 0, 0), rotation=(0, 0, 0), scale=(1, 1, 1), **kw):
        _check(kw)
        opts = {'segments': int(vertices), 'radius1': radius1, 'radius2': radius2, 'depth': depth}
        _add_mesh_object('add_primitive', 'cone', 'Cone', opts, location, rotation, scale)
        return {'FINISHED'}

    @staticmethod
    def primitive_torus_add(major_segments=48, minor_segments=12, major_radius=1.0, minor_radius=0.25, location=(0, 0, 0), rotation=(0, 0, 0), scale=(1, 1, 1), **kw):
        _check(kw)
        opts = {'major_segments': int(major_segments), 'minor_segments': int(minor_segments),
                'major_radius': major_radius, 'minor_radius': minor_radius}
        _add_mesh_object('add_primitive', 'torus', 'Torus', opts, location, rotation, scale)
        return {'FINISHED'}


class _ObjectOps:
    """bpy.ops.object: adding lights, cameras and empties; selection; delete;
    shading; modifiers on the active object."""

    @staticmethod
    def light_add(type='POINT', radius=1.0, location=(0, 0, 0), rotation=(0, 0, 0), scale=(1, 1, 1), **kw):
        _check(kw)
        light = Light(str(type).capitalize(), type)
        _link_object(light.name, light, location, rotation, scale)
        return {'FINISHED'}

    @staticmethod
    def camera_add(location=(0, 0, 0), rotation=(0, 0, 0), scale=(1, 1, 1), **kw):
        _check(kw)
        _link_object('Camera', Camera('Camera'), location, rotation, scale)
        return {'FINISHED'}

    @staticmethod
    def empty_add(type='PLAIN_AXES', radius=1.0, location=(0, 0, 0), rotation=(0, 0, 0), scale=(1, 1, 1), **kw):
        _check(kw)
        _link_object('Empty', None, location, rotation, scale)
        return {'FINISHED'}

    @staticmethod
    def select_all(action='TOGGLE'):
        if action == 'TOGGLE':
            action = 'DESELECT' if state.selected else 'SELECT'
        if action == 'SELECT':
            state.selected[:] = list(state.objects)
            stream.emit('select_all')
        elif action == 'DESELECT':
            state.selected[:] = []
            stream.emit('deselect')
        elif action == 'INVERT':
            state.selected[:] = [o for o in state.objects if o not in state.selected]
            stream.emit('select', *[Raw(o._ref()) for o in state.selected])
        else:
            raise ValueError("action must be 'TOGGLE', 'SELECT', 'DESELECT' or 'INVERT'")
        return {'FINISHED'}

    @staticmethod
    def delete(use_global=False, confirm=True):
        for obj in list(state.selected):
            _remove_object(obj)
        return {'FINISHED'}

    @staticmethod
    def shade_smooth(**kw):
        for obj in state.selected:
            if isinstance(obj.data, Mesh):
                obj.data.shade_smooth()
        return {'FINISHED'}

    @staticmethod
    def shade_flat(**kw):
        for obj in state.selected:
            if isinstance(obj.data, Mesh):
                obj.data.shade_flat()
        return {'FINISHED'}

    @staticmethod
    def modifier_add(type='SUBSURF'):
        obj = state.active
        if obj is None:
            raise RuntimeError('modifier_add needs an active object')
        name = types._MODIFIERS.get(type, (type.capitalize(), None))[0]
        obj.modifiers.new(name, type)
        return {'FINISHED'}


class _Ops:
    mesh = _MeshOps()
    object = _ObjectOps()


ops = _Ops()


# --- the scene snapshot -------------------------------------------------------------------

def _load_snapshot(text):
    """Build the mirror from polyops scene text (what scene_to_polyops writes)."""
    from .polyops import floats, polygons, read_commands, uints
    root = Collection('Collection')
    state.collections[:] = [root]
    objects = {}
    meshes = {}
    by_name = lambda items, n: next((x for x in items if x.name == n), None)

    def place(obj, o):
        if 'location' in o:
            obj._loc._assign(floats(o['location']))
        if 'quaternion' in o:
            obj._rot._assign(quat_to_euler(*floats(o['quaternion'])))
        if 'rotation' in o:
            obj._rot._assign(floats(o['rotation']))
        if 'scale' in o:
            s = floats(o['scale'])
            obj._scale._assign(s * 3 if len(s) == 1 else s)
        if o.get('visible') == '0':
            obj._visible = False
        obj._collection = by_name(state.collections, o.get('collection', root.name)) or root
        state.objects.append(obj)
        objects[obj.name] = obj

    for cmd, pos, o in read_commands(text):
        if cmd == 'scene':
            state.scene_name = o.get('name', state.scene_name)
            state.frame_start = int(o.get('frame_start', state.frame_start))
            state.frame_end = int(o.get('frame_end', state.frame_end))
            state.fps = float(o.get('fps', state.fps))
        elif cmd == 'world':
            if 'color' in o:
                state.world_color = floats(o['color'])
            if 'strength' in o:
                state.world_strength = float(o['strength'])
        elif cmd in ('collection_new', 'collection_set'):
            c = Collection(pos[0]) if cmd == 'collection_new' else by_name(state.collections, pos[0])
            if cmd == 'collection_new':
                state.collections.append(c)
            if c is None:
                continue
            if 'name' in o:
                c._name = o['name']
            if 'visible' in o:
                c._visible = o['visible'] not in ('0', 'false')
            if 'parent' in o:
                c._parent = by_name(state.collections, o['parent'])
        elif cmd == 'material_new':
            m = Material(pos[0])
            v = m._v
            for k in ('metallic', 'roughness', 'emission_strength', 'ior', 'transmission'):
                if k in o:
                    v[k] = float(o[k])
            if 'base_color' in o:
                c = floats(o['base_color'])
                v['base_color'] = c + [1.0] if len(c) == 3 else c
            if 'emission' in o:
                v['emission'] = floats(o['emission'])
            v['double_sided'] = o.get('double_sided') == '1'
            state.materials.append(m)
        elif cmd == 'mesh_new':
            me = Mesh(pos[0])
            me._verts = [tuple(c) for c in zip(*[iter(floats(o.get('verts', '')))] * 3)]
            me._faces = [tuple(f) for f in polygons(o.get('faces', ''))]
            e = o.get('edges', '')
            flat = uints(e) if e.startswith('b64:') else [i for p in e.split(';') if p for i in uints(p)]
            me._edges = list(zip(flat[0::2], flat[1::2]))
            state.meshes.append(me)
            meshes[me.name] = me
        elif cmd in ('add_object', 'add_empty'):
            me = meshes.get(o.get('data'))
            obj = Object(pos[0] if pos else o.get('name', 'Empty'), me)
            if me is not None:
                me._users.append(obj)
            place(obj, o)
        elif cmd == 'light_add':
            kind = (pos[0] if pos else 'point').upper()
            light = Light(o.get('name', kind.capitalize()), kind)
            for k in ('power', 'radius', 'angle', 'blend', 'size'):
                if k in o:
                    light._v[k] = float(o[k])
            if 'color' in o:
                light._v['color'] = floats(o['color'])
            obj = Object(light.name, light)
            light._object = obj
            place(obj, o)
        elif cmd == 'camera_add':
            cam = Camera(o.get('name', 'Camera'))
            for k in ('fov', 'near', 'far', 'ortho_scale'):
                if k in o:
                    cam._v[k] = float(o[k])
            obj = Object(cam.name, cam)
            cam._object = obj
            place(obj, o)
        elif cmd == 'parent' and pos[0] in objects:
            objects[pos[0]]._parent = objects.get(pos[1])
        elif cmd == 'material_assign' and pos[0] in objects:
            obj, mat = objects[pos[0]], by_name(state.materials, pos[1])
            slot = int(o.get('slot', len(obj._slots)))
            while len(obj._slots) <= slot:
                obj._slots.append(mat)
            obj._slots[slot] = mat
            if isinstance(obj.data, Mesh):
                obj.data.materials._slots = list(obj._slots)
        elif cmd == 'modifier_add' and pos[0] in objects:
            _load_modifier(objects[pos[0]], pos[1], o)
        elif cmd == 'select':
            state.selected[:] = [objects[n] for n in pos if n in objects]
        elif cmd == 'active':
            state.active = objects.get(pos[0]) if pos else None
        elif cmd == 'active_camera':
            state.camera = objects.get(pos[0]) if pos else None


def _load_modifier(obj, polyops_type, o):
    from .polyops import floats
    kind = types._POLYOPS_TO_BLENDER.get(polyops_type.lower())
    if kind is None:
        return
    p = {k: list(v) if isinstance(v, list) else v for k, v in types._MODIFIERS[kind][1].items()}
    b = lambda k, d: o.get(k, d) in ('1', 'true')
    if kind == 'SUBSURF':
        p['levels'] = int(o.get('levels', 1))
        p['subdivision_type'] = 'SIMPLE' if b('simple', '0') else 'CATMULL_CLARK'
    elif kind == 'MIRROR':
        p['use_axis'] = [b('x', '1'), b('y', '0'), b('z', '0')]
        p['use_mirror_merge'] = b('merge', '1')
        p['merge_threshold'] = float(o.get('merge_distance', 0.001))
    elif kind == 'ARRAY':
        p['count'] = int(o.get('count', 2))
        p['relative_offset_displace'] = floats(o.get('relative', '1,0,0'))
        p['constant_offset_displace'] = floats(o.get('constant', '0,0,0'))
        p['use_constant_offset'] = any(p['constant_offset_displace'])
        p['use_merge_vertices'] = b('merge', '0')
    elif kind == 'BEVEL':
        p.update(width=float(o.get('width', 0.1)), segments=int(o.get('segments', 1)), angle_limit=float(o.get('angle', p['angle_limit'])))
    elif kind == 'WELD':
        p['merge_threshold'] = float(o.get('distance', 0.001))
    elif kind == 'SMOOTH':
        p.update(factor=float(o.get('factor', 0.5)), iterations=int(o.get('iterations', 1)))
    else:
        for k in p:
            if k in o:
                p[k] = float(o[k])
    m = Modifier(obj, polyops_type, kind, p)
    obj.modifiers._list.append(m)


def _start():
    state.collections[:] = [Collection('Collection')]
    path = os.environ.get('POLYAMOUR_SCENE')
    if path:
        with open(path, encoding='utf-8') as f:
            _load_snapshot(f.read())
    # Open the stream now, so even a script that changes nothing produces a
    # complete (empty) program.
    stream._open()


_start()
