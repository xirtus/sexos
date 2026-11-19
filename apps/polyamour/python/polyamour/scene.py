# SPDX-License-Identifier: GPL-3.0-only
"""Scenes, render settings, view layers, collections, the depsgraph, the
blend-data collections (bpy.data) and the context (bpy.context)."""
import contextlib

from mathutils import Vector

from . import polyops as P
from ._arrays import FloatArray
from ._base import ID, Collection_, bpy_struct, first, parse_value, req


def _scene_j():
    return req('dump_scene')


def _ids():
    return req('ids')


# --- settings stored by path -------------------------------------------------------------------

class _Settings(bpy_struct):
    """A group of scene settings kept under `PREFIX.name` (render.*, cycles.*...)."""

    _prefix = ''
    _defaults = {}

    def _key(self):
        return ('settings', self._prefix)

    def __getattr__(self, key):
        if key.startswith('_'):
            raise AttributeError(key)
        props = _scene_j()['props']
        full = self._prefix + '.' + key
        if full in props:
            d = self._defaults.get(key)
            v = props[full]
            if isinstance(d, bool):
                return v in ('1', 'true', 'True')
            if isinstance(d, str):
                return v
            if isinstance(d, (tuple, list)):
                return FloatArray(parse_value(v) if ',' in v else [float(v)], lambda x: setattr(self, key, x))
            p = parse_value(v)
            return p
        if key in self._defaults:
            d = self._defaults[key]
            if isinstance(d, (tuple, list)):
                return FloatArray(d, lambda x: setattr(self, key, x))
            return d
        raise AttributeError("'%s' has no attribute '%s'" % (type(self).__name__, key))

    def __setattr__(self, key, value):
        if key.startswith('_'):
            object.__setattr__(self, key, value)
            return
        if isinstance(value, bool):
            text = '1' if value else '0'
        elif isinstance(value, (int, float)):
            text = P.fmt_num(value)
        elif isinstance(value, str):
            text = value
        else:
            text = P.fmt_list(value)
        req('prop_set', self._prefix + '.' + key, text)


class ImageFormatSettings(_Settings):
    _prefix = 'render.image_settings'
    _defaults = {'file_format': 'PNG', 'color_mode': 'RGBA', 'color_depth': '8', 'compression': 15, 'quality': 90,
                 'exr_codec': 'ZIP', 'color_management': 'FOLLOW_SCENE'}


class FFmpegSettings(_Settings):
    _prefix = 'render.ffmpeg'
    _defaults = {'format': 'MPEG4', 'codec': 'H264', 'constant_rate_factor': 'MEDIUM'}


class RenderSettings(_Settings):
    _prefix = 'render'
    _defaults = {'resolution_x': 1920, 'resolution_y': 1080, 'resolution_percentage': 100, 'fps': 24, 'fps_base': 1.0,
                 'filepath': '/tmp/', 'engine': 'CYCLES', 'film_transparent': False, 'use_file_extension': True,
                 'use_overwrite': True, 'use_compositing': True, 'use_sequencer': True, 'pixel_aspect_x': 1.0,
                 'pixel_aspect_y': 1.0, 'use_motion_blur': False, 'motion_blur_shutter': 0.5, 'use_border': False,
                 'use_simplify': False, 'threads_mode': 'AUTO', 'use_persistent_data': False, 'dither_intensity': 1.0,
                 'filter_size': 1.5, 'use_freestyle': False, 'use_stamp': False, 'is_movie_format': False}

    @property
    def image_settings(self):
        return ImageFormatSettings()

    @property
    def ffmpeg(self):
        return FFmpegSettings()

    @property
    def views(self):
        return Collection_()

    def frame_path(self, frame=None, preview=False, view=''):
        from . import context
        f = context.scene.frame_current if frame is None else frame
        path = self.filepath
        if '#' in path:
            n = path.count('#')
            path = path.replace('#' * n, '%0*d' % (n, f), 1)
        else:
            path = path + '%04d' % f
        return path + '.png'


class CyclesSettings(_Settings):
    _prefix = 'cycles'
    _defaults = {'samples': 16, 'preview_samples': 16, 'max_bounces': 12, 'diffuse_bounces': 4, 'glossy_bounces': 4,
                 'transmission_bounces': 12, 'volume_bounces': 0, 'transparent_max_bounces': 8, 'use_denoising': False,
                 'use_preview_denoising': False, 'seed': 0, 'use_animated_seed': False, 'device': 'CPU', 'feature_set': 'SUPPORTED',
                 'use_adaptive_sampling': True, 'adaptive_threshold': 0.01, 'time_limit': 0.0, 'caustics_reflective': True,
                 'caustics_refractive': True, 'blur_glossy': 1.0, 'sample_clamp_direct': 0.0, 'sample_clamp_indirect': 10.0,
                 'denoiser': 'OPENIMAGEDENOISE', 'film_exposure': 1.0, 'pixel_filter_type': 'BLACKMAN_HARRIS'}


class EeveeSettings(_Settings):
    _prefix = 'eevee'
    _defaults = {'taa_render_samples': 64, 'taa_samples': 16, 'use_gtao': False, 'use_bloom': False, 'use_ssr': False}


class ViewSettings(_Settings):
    _prefix = 'view_settings'
    _defaults = {'view_transform': 'AgX', 'look': 'None', 'exposure': 0.0, 'gamma': 1.0, 'use_curve_mapping': False}


class DisplaySettings(_Settings):
    _prefix = 'display_settings'
    _defaults = {'display_device': 'sRGB'}


class UnitSettings(_Settings):
    _prefix = 'unit_settings'
    _defaults = {'system': 'METRIC', 'scale_length': 1.0, 'length_unit': 'METERS', 'system_rotation': 'RADIANS'}


class ToolSettings(_Settings):
    _prefix = 'tool_settings'
    _defaults = {'use_snap': False, 'use_proportional_edit': False, 'use_keyframe_insert_auto': False,
                 'use_mesh_automerge': False, 'transform_pivot_point': 'MEDIAN_POINT', 'snap_elements': set()}

    @property
    def mesh_select_mode(self):
        m = _scene_j()['select_mode']

        def write(v):
            words = [w for w, on in zip(('VERT', 'EDGE', 'FACE'), v) if on]
            req('mesh_select', _active_mesh(), mode=P.Raw(','.join(words) or 'VERT'))
        from ._arrays import BoolArray
        return BoolArray(m, write)

    @mesh_select_mode.setter
    def mesh_select_mode(self, v):
        words = [w for w, on in zip(('VERT', 'EDGE', 'FACE'), v) if on]
        req('mesh_select', _active_mesh(), mode=P.Raw(','.join(words) or 'VERT'))


def _active_mesh():
    a = _ids()['active']
    if a is None:
        raise RuntimeError('no active object')
    return a


class Cursor(bpy_struct):
    def _key(self):
        return ('cursor',)

    @property
    def location(self):
        return Vector(_scene_j()['cursor'], _owner=lambda v: req('prop_set', 'scene.cursor.location', P.fmt_list(v)))

    @location.setter
    def location(self, v):
        req('prop_set', 'scene.cursor.location', P.fmt_list([float(x) for x in v]))

    rotation_euler = Vector((0.0, 0.0, 0.0))
    rotation_mode = 'XYZ'

    @property
    def matrix(self):
        from mathutils import Matrix
        return Matrix.Translation(self.location)


# --- collections -----------------------------------------------------------------------------

class CollectionObjects(Collection_):
    def __init__(self, coll, recursive=False):
        self._c = coll
        self._r = recursive

    def _items(self):
        from .types import Object
        return [Object(n) for n in self._c._names(self._r)]

    def link(self, obj):
        req('link', obj.name, self._c._name)

    def unlink(self, obj):
        root = _scene_j()['collections'][0]['name']
        if root != self._c._name:
            req('link', obj.name, root)


class CollectionChildren(Collection_):
    def __init__(self, coll):
        self._c = coll

    def _items(self):
        return [Collection(n) for n in self._c._j()['children']]

    def link(self, child):
        req('collection_set', child.name, parent=self._c._name)

    def unlink(self, child):
        req('collection_set', child.name, parent=P.Raw('none'))


class Collection(ID):
    _kind = 'collection'
    _collection_name = 'collections'

    def _j(self):
        for c in _scene_j()['collections']:
            if c['name'] == self._name:
                return c
        raise KeyError('collection "%s" not found' % self._name)

    def _rename(self, new):
        req('collection_set', self._name, name=new)
        names = [c['name'] for c in _scene_j()['collections']]
        return new if new in names else None

    def _names(self, recursive):
        j = self._j()
        out = list(j['objects'])
        if recursive:
            for c in j['children']:
                out.extend(Collection(c)._names(True))
        return out

    @property
    def objects(self):
        return CollectionObjects(self)

    @property
    def all_objects(self):
        return CollectionObjects(self, True)

    @property
    def children(self):
        return CollectionChildren(self)

    @property
    def children_recursive(self):
        out = []
        for c in self.children:
            out.append(c)
            out.extend(c.children_recursive)
        return out

    @property
    def hide_viewport(self):
        return self._j()['hide_viewport']

    @hide_viewport.setter
    def hide_viewport(self, h):
        req('collection_set', self._name, visible=not h)

    hide_render = hide_viewport
    hide_select = False
    color_tag = 'NONE'

    @property
    def users(self):
        return 1


class SceneCollection(Collection):
    """scene.collection (the master collection: Polyamour's first one)."""


# --- view layers and the depsgraph -----------------------------------------------------------

class LayerObjects(Collection_):
    def _items(self):
        from .types import Object
        return [Object(n) for n in _ids()['objects']]

    @property
    def active(self):
        from .types import Object
        a = _ids()['active']
        return Object(a) if a else None

    @active.setter
    def active(self, obj):
        req('active', obj.name if obj is not None else P.Raw('none'))

    @property
    def selected(self):
        from .types import Object
        return [Object(n) for n in _ids()['selected']]


class LayerCollection(bpy_struct):
    def __init__(self, name):
        self._n = name

    def _key(self):
        return ('layer_collection', self._n)

    @property
    def name(self):
        return self._n

    @property
    def collection(self):
        return Collection(self._n)

    @property
    def children(self):
        return [LayerCollection(c) for c in Collection(self._n)._j()['children']]

    @property
    def exclude(self):
        return False

    @exclude.setter
    def exclude(self, v):
        Collection(self._n).hide_viewport = bool(v)

    @property
    def hide_viewport(self):
        return Collection(self._n).hide_viewport

    @hide_viewport.setter
    def hide_viewport(self, v):
        Collection(self._n).hide_viewport = bool(v)

    holdout = False
    indirect_only = False


class ViewLayer(bpy_struct):
    def _key(self):
        return ('view_layer',)

    name = 'ViewLayer'
    use = True
    samples = 0
    use_pass_combined = True
    use_pass_z = False
    use_pass_normal = False
    use_pass_mist = False

    @property
    def objects(self):
        return LayerObjects()

    @property
    def active_layer_collection(self):
        return LayerCollection(_scene_j()['collections'][0]['name'])

    @property
    def layer_collection(self):
        return LayerCollection(_scene_j()['collections'][0]['name'])

    @property
    def depsgraph(self):
        return Depsgraph()

    def update(self):
        P.get().flush()


class DepsgraphUpdate(bpy_struct):
    def __init__(self, id_):
        self.id = id_

    is_updated_geometry = True
    is_updated_transform = True
    is_updated_shading = True


class DepsgraphObjectInstance(bpy_struct):
    def __init__(self, obj):
        self.object = obj
        self.instance_object = obj
        self.parent = None
        self.is_instance = False
        self.matrix_world = obj.matrix_world
        self.persistent_id = (0,) * 8
        self.random_id = 0

    def _key(self):
        return ('instance', self.object.name)


class Depsgraph(bpy_struct):
    def _key(self):
        return ('depsgraph',)

    def update(self):
        P.get().flush()

    @property
    def scene(self):
        return Scene(_ids()['scene'])

    scene_eval = scene

    @property
    def view_layer(self):
        return ViewLayer()

    view_layer_eval = view_layer

    @property
    def objects(self):
        from .types import EvaluatedObject
        return [EvaluatedObject(n) for n in _ids()['objects']]

    @property
    def object_instances(self):
        return [DepsgraphObjectInstance(o) for o in self.objects]

    @property
    def ids(self):
        return self.objects

    @property
    def updates(self):
        return []

    mode = 'VIEWPORT'

    def id_eval_get(self, id_):
        return id_.evaluated_get(self)

    def id_type_updated(self, id_type):
        return True


# --- scene ----------------------------------------------------------------------------------

class Scene(ID):
    _kind = 'scene'
    _collection_name = 'scenes'

    def _rename(self, new):
        req('scene', name=new)
        return new

    def _j(self):
        return _scene_j()

    @property
    def frame_current(self):
        return int(self._j()['frame_current'])

    @frame_current.setter
    def frame_current(self, f):
        req('prop_set', 'scene.frame_current', str(int(f)))

    @property
    def frame_start(self):
        return self._j()['frame_start']

    @frame_start.setter
    def frame_start(self, f):
        req('scene', frame_start=int(f))

    @property
    def frame_end(self):
        return self._j()['frame_end']

    @frame_end.setter
    def frame_end(self, f):
        req('scene', frame_end=int(f))

    frame_step = 1
    frame_preview_start = property(lambda self: self.frame_start)
    frame_preview_end = property(lambda self: self.frame_end)
    use_preview_range = False
    frame_subframe = 0.0

    def frame_set(self, frame, subframe=0.0):
        from . import app
        app._frame_change(self, int(frame), float(subframe))

    @property
    def render(self):
        return RenderSettings()

    @property
    def cycles(self):
        return CyclesSettings()

    @property
    def eevee(self):
        return EeveeSettings()

    @property
    def view_settings(self):
        return ViewSettings()

    @property
    def display_settings(self):
        return DisplaySettings()

    @property
    def unit_settings(self):
        return UnitSettings()

    @property
    def tool_settings(self):
        return ToolSettings()

    @property
    def cursor(self):
        return Cursor()

    @property
    def camera(self):
        from .types import Object
        c = self._j()['camera']
        return Object(c) if c else None

    @camera.setter
    def camera(self, obj):
        req('active_camera', obj.name if obj is not None else P.Raw('none'))

    @property
    def world(self):
        from .shading import World
        return World('World')

    @world.setter
    def world(self, w):
        pass

    @property
    def objects(self):
        return LayerObjects()

    @property
    def collection(self):
        return SceneCollection(self._j()['collections'][0]['name'])

    @property
    def view_layers(self):
        return _Single(ViewLayer())

    @property
    def use_nodes(self):
        return self._j()['use_nodes']

    @use_nodes.setter
    def use_nodes(self, on):
        req('use_nodes', 'compositor', bool(on))

    @property
    def node_tree(self):
        from .shading import NodeTree
        return NodeTree('Compositing Nodetree', ('compositor',)) if self.use_nodes else None

    @property
    def keying_sets(self):
        return Collection_()

    @property
    def timeline_markers(self):
        return Collection_()

    @property
    def sequence_editor(self):
        return None

    @property
    def rigidbody_world(self):
        return None

    @property
    def animation_data(self):
        return None

    use_gravity = True
    gravity = Vector((0.0, 0.0, -9.81))
    sync_mode = 'AUDIO_SYNC'

    def statistics(self, view_layer=None):
        j = _ids()
        return 'Objects %d' % len(j['objects'])

    def ray_cast(self, depsgraph, origin, direction, distance=1.70141e38):
        from .types import Object
        best = None
        for name in _ids()['objects']:
            o = Object(name)
            if o.type != 'MESH' or o.hide_viewport:
                continue
            mw = o.matrix_world
            inv = mw.inverted()
            lo = inv @ Vector(origin)
            ld = (inv.to_3x3() @ Vector(direction)).normalized()
            hit, loc, nrm, idx = o.ray_cast(lo, ld, depsgraph=depsgraph)
            if hit:
                wl = mw @ loc
                d = (wl - Vector(origin)).length
                if d <= distance and (best is None or d < best[0]):
                    best = (d, wl, (mw.to_3x3().inverted().transposed() @ nrm).normalized(), idx, o, mw)
        if best is None:
            return False, Vector((0, 0, 0)), Vector((0, 0, 0)), -1, None, None
        return True, best[1], best[2], best[3], best[4], best[5]


class _Single(Collection_):
    def __init__(self, item):
        self._it = item

    def _items(self):
        return [self._it]


# --- bpy.data ----------------------------------------------------------------------------------

class _IDCollection(Collection_):
    _ids_key = None
    _cls = None

    def _items(self):
        return [self._make(n) for n in _ids()[self._ids_key]]

    def _make(self, n):
        return self._cls(n)

    def __getitem__(self, key):
        if isinstance(key, (int, slice)):
            return self._items()[key]
        if key in _ids()[self._ids_key]:
            return self._make(key)
        raise KeyError('bpy_prop_collection[key]: key "%s" not found' % key)

    def __contains__(self, key):
        if isinstance(key, str):
            return key in _ids()[self._ids_key]
        return getattr(key, 'name', None) in _ids()[self._ids_key]

    def __len__(self):
        return len(_ids()[self._ids_key])

    def tag(self, value):
        pass

    def remove(self, id_, do_unlink=True, do_id_user=True, do_ui_user=True):
        req('remove', self._remove_kind, id_.name)

    _remove_kind = None


class BlendDataObjects(_IDCollection):
    _ids_key = 'objects'

    def _make(self, n):
        from .types import Object
        return Object(n)

    def new(self, name, object_data):
        from .types import Object, Light, Camera
        from .mesh import Mesh, _pending_materials
        from .rig import Armature, Lattice
        if object_data is None:
            got = first(req('add_empty', name))
        elif isinstance(object_data, Mesh):
            # Blender keeps materials on the mesh: a new user gets the slots
            # its other users have.
            mats = list(_pending_materials.pop(object_data.name, []))
            if not mats:
                users = object_data._d().users
                if users:
                    try:
                        mats = list(req('dump_object', users[0]).get('materials') or [])
                    except P.PolyopsError:
                        mats = []
            got = first(req('add_object', name, data=object_data.name))
            for i, m in enumerate(mats):
                req('material_slot', got, i, m if m else P.Raw('none'))
        elif isinstance(object_data, Light):
            got = object_data._create(name)
        elif isinstance(object_data, Camera):
            got = object_data._create(name)
        elif isinstance(object_data, Armature):
            got = first(req('add_armature', name, data=object_data.name))
        elif isinstance(object_data, Lattice):
            got = first(req('add_lattice', name, data=object_data.name))
        else:
            raise TypeError('objects.new(): unsupported data %r' % (object_data,))
        # Blender's objects.new() neither selects nor activates the object and
        # links it to no collection; Polyamour has every object in the scene,
        # so keep the previous selection and active object.
        return Object(got)

    def remove(self, obj, do_unlink=True, do_id_user=True, do_ui_user=True):
        req('delete', obj.name)


class BlendDataMeshes(_IDCollection):
    _ids_key = 'meshes'
    _remove_kind = 'mesh'

    def _make(self, n):
        from .mesh import Mesh
        return Mesh(n)

    def new(self, name):
        from .mesh import Mesh
        return Mesh(first(req('mesh_new', name)))

    def new_from_object(self, obj, preserve_all_data_layers=False, depsgraph=None):
        from .mesh import Mesh
        src = obj.evaluated_get(depsgraph).to_mesh()
        m = Mesh(first(req('mesh_new', obj.name)))
        d = src._d()
        req('mesh_set', m.name, **P.mesh_options(d.co, d.edges, d.faces()))
        return m


class BlendDataMaterials(_IDCollection):
    _ids_key = 'materials'
    _remove_kind = 'material'

    def _make(self, n):
        from .shading import Material
        return Material(n)

    def new(self, name):
        from .shading import Material
        return Material(first(req('material_new', name)))

    def create_gpencil_data(self, material):
        pass


class BlendDataCollections(_IDCollection):
    _ids_key = 'collections'
    _remove_kind = 'collection'
    _cls = Collection

    def new(self, name):
        root = _scene_j()['collections'][0]['name']
        return Collection(first(req('collection_new', name, parent=root)))


class BlendDataImages(_IDCollection):
    _ids_key = 'images'
    _remove_kind = 'image'

    def _make(self, n):
        from .shading import Image
        return Image(n)

    def new(self, name, width, height, alpha=False, float_buffer=False, stereo3d=False, is_data=False, tiled=False):
        from .shading import Image
        img = Image(first(req('image_new', name, int(width), int(height))))
        if float_buffer:
            img['_is_float'] = True
        return img

    def load(self, filepath, check_existing=False):
        from .shading import Image
        if check_existing:
            for i in self._items():
                if i.filepath == filepath:
                    return i
        img = Image(first(req('image_load', filepath)))
        img['_filepath'] = filepath
        return img


class BlendDataNodeGroups(_IDCollection):
    _ids_key = 'node_groups'
    _remove_kind = 'node_group'

    def _make(self, n):
        from .shading import NodeTree
        return NodeTree(n)

    def new(self, name, type='ShaderNodeTree'):
        from .shading import NodeTree
        if type not in ('ShaderNodeTree', 'GeometryNodeTree'):
            raise TypeError("node_groups.new(): type '%s' is not supported" % type)
        opts = {} if type == 'ShaderNodeTree' else {'kind': P.Raw('geometry')}
        try:
            got = first(req('node_tree', 'group', name, **opts))
        except P.PolyopsError:
            if opts:
                raise TypeError('geometry node groups are not supported by this Polyamour')
            raise
        return NodeTree(got or name)


class BlendDataArmatures(_IDCollection):
    _ids_key = 'armatures'
    _remove_kind = 'armature'

    def _make(self, n):
        from .rig import Armature
        return Armature(n)

    def new(self, name):
        from .rig import Armature
        return Armature(first(req('armature_new', name)))


class BlendDataLattices(_IDCollection):
    _ids_key = 'lattices'
    _remove_kind = 'lattice'

    def _make(self, n):
        from .rig import Lattice
        return Lattice(n)

    def new(self, name):
        from .rig import Lattice
        return Lattice(first(req('lattice_new', name)))


class BlendDataActions(_IDCollection):
    _ids_key = 'actions'
    _remove_kind = 'action'

    def _make(self, n):
        from .anim import Action
        return Action(n)

    def new(self, name):
        from .anim import Action
        return Action(first(req('action_new', name)))


_pending_lights = {}


class BlendDataLights(_IDCollection):
    _ids_key = 'lights'

    def _make(self, n):
        from .types import Light
        return Light(n, obj=n)

    def new(self, name, type):
        from .types import Light
        return Light(name, pending={'type': type})

    def remove(self, light, do_unlink=True, do_id_user=True, do_ui_user=True):
        if light._obj:
            req('delete', light._obj)


class BlendDataCameras(_IDCollection):
    _ids_key = 'cameras'

    def _make(self, n):
        from .types import Camera
        return Camera(n, obj=n)

    def new(self, name):
        from .types import Camera
        return Camera(name, pending={})

    def remove(self, cam, do_unlink=True, do_id_user=True, do_ui_user=True):
        if cam._obj:
            req('delete', cam._obj)


class BlendDataScenes(Collection_):
    def _items(self):
        return [Scene(_ids()['scene'])]

    def new(self, name):
        raise NotImplementedError('Polyamour has one scene per file')


class BlendDataWorlds(Collection_):
    def _items(self):
        from .shading import World
        return [World('World')]

    def new(self, name):
        from .shading import World
        return World('World')


class _Texts(Collection_):
    """bpy.data.texts: the Text Editor's texts, when the app shares them."""

    def __init__(self):
        self._t = {}

    def _items(self):
        from .rna import Text
        return [Text(n, self) for n in self._t]

    def new(self, name):
        from .rna import Text
        n, k = name, 0
        while n in self._t:
            k += 1
            n = '%s.%03d' % (name, k)
        self._t[n] = ''
        return Text(n, self)

    def remove(self, text):
        self._t.pop(text.name, None)

    def load(self, filepath, internal=False):
        import os
        with open(filepath, encoding='utf-8') as f:
            t = self.new(os.path.basename(filepath))
            t.write(f.read())
        return t


class BlendData(bpy_struct):
    def _key(self):
        return ('data',)

    objects = BlendDataObjects()
    meshes = BlendDataMeshes()
    materials = BlendDataMaterials()
    collections = BlendDataCollections()
    images = BlendDataImages()
    node_groups = BlendDataNodeGroups()
    armatures = BlendDataArmatures()
    lattices = BlendDataLattices()
    actions = BlendDataActions()
    lights = BlendDataLights()
    cameras = BlendDataCameras()
    scenes = BlendDataScenes()
    worlds = BlendDataWorlds()
    texts = _Texts()
    curves = fonts = metaballs = grease_pencils = textures = brushes = palettes = linestyles = particles = Collection_()
    libraries = speakers = sounds = movieclips = masks = cache_files = paint_curves = workspaces = screens = Collection_()
    window_managers = volumes = pointclouds = lightprobes = hair_curves = shape_keys = Collection_()
    filepath = ''
    is_dirty = False
    is_saved = False
    use_autopack = False
    version = (4, 2, 0)

    def batch_remove(self, ids):
        for i in ids:
            if i.__class__.__name__ in ('Object', 'EvaluatedObject'):
                req('delete', i.name)
            else:
                req('remove', i._kind if i._kind != 'node_group' else 'node_group', i.name)

    def orphans_purge(self, do_local_ids=True, do_linked_ids=True, do_recursive=False):
        return 0

    def user_map(self, subset=None, key_types=None, value_types=None):
        return {}

    @contextlib.contextmanager
    def temp_data(self, filepath=None):
        yield self


# --- bpy.context ----------------------------------------------------------------------------------

_MODE_NAMES = {('EDIT', 'MESH'): 'EDIT_MESH', ('EDIT', 'ARMATURE'): 'EDIT_ARMATURE', ('EDIT', 'LATTICE'): 'EDIT_LATTICE',
               ('POSE', 'ARMATURE'): 'POSE', ('SCULPT', 'MESH'): 'SCULPT', ('WEIGHT_PAINT', 'MESH'): 'PAINT_WEIGHT',
               ('VERTEX_PAINT', 'MESH'): 'PAINT_VERTEX', ('TEXTURE_PAINT', 'MESH'): 'PAINT_TEXTURE'}


class Context(bpy_struct):
    """bpy.context. `temp_override(**kw)` swaps members for a `with` block."""

    def __init__(self):
        self.__dict__['_over'] = [{}]

    def _key(self):
        return ('context',)

    def _o(self, key):
        for d in reversed(self._over):
            if key in d:
                return True, d[key]
        return False, None

    def __getattribute__(self, key):
        if not key.startswith('_') and key not in ('temp_override', 'copy', 'evaluated_depsgraph_get', 'path_resolve'):
            over = object.__getattribute__(self, '_over')
            for d in reversed(over):
                if key in d:
                    return d[key]
        return object.__getattribute__(self, key)

    def __setattr__(self, key, value):
        raise AttributeError("bpy_struct: attribute \"%s\" from \"Context\" is read-only" % key)

    @contextlib.contextmanager
    def temp_override(self, **kw):
        self._over.append(kw)
        try:
            yield self
        finally:
            self._over.pop()

    def copy(self):
        keys = ('active_object', 'object', 'selected_objects', 'scene', 'view_layer', 'collection', 'mode')
        return {k: getattr(self, k) for k in keys}

    def evaluated_depsgraph_get(self):
        P.get().flush()
        return Depsgraph()

    def path_resolve(self, path, coerce=True):
        v = self
        for p in path.split('.'):
            v = getattr(v, p)
        return v

    @property
    def scene(self):
        return Scene(_ids()['scene'])

    @property
    def view_layer(self):
        return ViewLayer()

    @property
    def collection(self):
        return self.scene.collection

    @property
    def layer_collection(self):
        return self.view_layer.active_layer_collection

    @property
    def active_object(self):
        from .types import Object
        a = _ids()['active']
        return Object(a) if a else None

    object = active_object

    @property
    def edit_object(self):
        a = self.active_object
        return a if a is not None and _ids()['mode'] == 'EDIT' else None

    @property
    def selected_objects(self):
        from .types import Object
        return [Object(n) for n in _ids()['selected']]

    selected_editable_objects = selected_objects

    @property
    def visible_objects(self):
        from .types import Object
        return [o for o in (Object(n) for n in _ids()['objects']) if not o.hide_viewport]

    selectable_objects = visible_objects
    editable_objects = visible_objects

    @property
    def objects_in_mode(self):
        a = self.active_object
        return [a] if a is not None and _ids()['mode'] != 'OBJECT' else []

    objects_in_mode_unique_data = objects_in_mode

    @property
    def object_mode(self):
        return _ids()['mode']

    @property
    def mode(self):
        m = _ids()['mode']
        if m == 'OBJECT':
            return 'OBJECT'
        a = self.active_object
        return _MODE_NAMES.get((m, a.type if a is not None else ''), m)

    @property
    def active_bone(self):
        a = self.active_object
        if a is None or a.type != 'ARMATURE':
            return None
        bones = a.data.edit_bones if _ids()['mode'] == 'EDIT' else a.data.bones
        act = bones.active
        if act is None:
            items = bones._items()
            return items[0] if items else None
        return act

    @property
    def active_pose_bone(self):
        a = self.active_object
        if a is None or a.type != 'ARMATURE':
            return None
        b = self.active_bone
        return a.pose.bones[b.name] if b is not None else None

    @property
    def selected_pose_bones(self):
        a = self.active_object
        if a is None or a.type != 'ARMATURE':
            return []
        sel = set(req('dump_object', a.name).get('bone_select') or [])
        return [pb for pb in a.pose.bones if pb.name in sel]

    selected_pose_bones_from_active_object = selected_pose_bones
    visible_pose_bones = property(lambda self: list(self.active_object.pose.bones) if self.active_object and self.active_object.type == 'ARMATURE' else [])

    @property
    def selected_bones(self):
        a = self.active_object
        if a is None or a.type != 'ARMATURE':
            return []
        sel = set(req('dump_object', a.name).get('bone_select') or [])
        return [b for b in a.data.edit_bones if b.name in sel]

    selected_editable_bones = selected_bones

    @property
    def active_action(self):
        a = self.active_object
        ad = a.animation_data if a is not None else None
        return ad.action if ad else None

    @property
    def material(self):
        a = self.active_object
        return a.active_material if a is not None else None

    @property
    def mesh(self):
        a = self.active_object
        return a.data if a is not None and a.type == 'MESH' else None

    @property
    def armature(self):
        a = self.active_object
        return a.data if a is not None and a.type == 'ARMATURE' else None

    @property
    def camera(self):
        a = self.active_object
        return a.data if a is not None and a.type == 'CAMERA' else None

    @property
    def light(self):
        a = self.active_object
        return a.data if a is not None and a.type == 'LIGHT' else None

    @property
    def world(self):
        return self.scene.world

    @property
    def tool_settings(self):
        return ToolSettings()

    @property
    def blend_data(self):
        from . import data
        return data

    @property
    def preferences(self):
        from .rna import preferences
        return preferences

    @property
    def window_manager(self):
        from .rna import window_manager
        return window_manager

    @property
    def window(self):
        from .rna import window_manager
        return window_manager.windows[0]

    @property
    def screen(self):
        return self.window.screen

    @property
    def workspace(self):
        return None

    @property
    def area(self):
        from .rna import current_area
        return current_area()

    @property
    def region(self):
        a = self.area
        return a.regions[-1] if a is not None else None

    @property
    def space_data(self):
        a = self.area
        return a.spaces.active if a is not None else None

    @property
    def region_data(self):
        a = self.area
        return getattr(a.spaces.active, 'region_3d', None) if a is not None else None

    @property
    def engine(self):
        return 'CYCLES'

    @property
    def scene_collection(self):
        return self.scene.collection

    @property
    def edit_text(self):
        return None

    @property
    def active_operator(self):
        return None

    @property
    def gizmo_group(self):
        return None
