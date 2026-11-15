# SPDX-License-Identifier: GPL-3.0-only
"""bpy.types.Object and what hangs off it (modifiers, material slots), lights
and cameras; the other ID types are in mesh.py, shading.py, rig.py, anim.py
and scene.py and re-exported here."""
import math

from mathutils import Euler, Matrix, Quaternion, Vector

from . import polyops as P
from ._arrays import BoolArray, FloatArray
from ._base import ID, Collection_, bpy_struct, first, parse_opts, parse_value, req
from .anim import Action, ActionFCurves, ActionGroup, AnimData, FCurve, Keyframe, KeyframePoints
from .mesh import (Attribute, Attributes, Key, Mesh, MeshEdge, MeshLoop, MeshLoopTriangle, MeshPolygon, MeshUVLoop,
                   MeshUVLoopLayer, MeshVertex, ShapeKey, VertexGroupElement)
from .rig import (Armature, Bone, BoneCollection, Constraint, Constraints, EditBone, Lattice, LatticePoint, Pose, PoseBone,
                  VertexGroup, VertexGroups, _m4, _cols)
from .shading import (ColorRamp, ColorRampElement, Image, Material, Node, NodeLink, NodeLinks, Nodes, NodeSocket, NodeTree,
                      NodeTreeInterface, World)


# --- modifiers --------------------------------------------------------------------------

# polyops modifier type -> Blender type
_TYPES = {
    'Mirror': 'MIRROR', 'Subdivision': 'SUBSURF', 'Array': 'ARRAY', 'Solidify': 'SOLIDIFY', 'Bevel': 'BEVEL',
    'Triangulate': 'TRIANGULATE', 'Weld': 'WELD', 'Displace': 'DISPLACE', 'Smooth': 'SMOOTH', 'Decimate': 'DECIMATE',
    'Remesh': 'REMESH', 'Screw': 'SCREW', 'Wireframe': 'WIREFRAME', 'EdgeSplit': 'EDGE_SPLIT', 'Build': 'BUILD',
    'Boolean': 'BOOLEAN', 'SimpleDeform': 'SIMPLE_DEFORM', 'Cast': 'CAST', 'Wave': 'WAVE', 'Shrinkwrap': 'SHRINKWRAP',
    'LaplacianSmooth': 'LAPLACIANSMOOTH', 'CorrectiveSmooth': 'CORRECTIVE_SMOOTH', 'WeightedNormal': 'WEIGHTED_NORMAL',
    'Armature': 'ARMATURE', 'Hook': 'HOOK', 'Lattice': 'LATTICE', 'Warp': 'WARP', 'VertexWeightEdit': 'VERTEX_WEIGHT_EDIT',
    'VertexWeightMix': 'VERTEX_WEIGHT_MIX', 'VertexWeightProximity': 'VERTEX_WEIGHT_PROXIMITY', 'Nodes': 'NODES',
}
_FROM_BLENDER = {v: k for k, v in _TYPES.items()}


def _f(x):
    return float(x)


def _b(x):
    return bool(x)


def _i(x):
    return int(x)


def _enum(pairs):
    """Blender enum <-> polyops word."""
    to_p = dict(pairs)
    to_b = {v: k for k, v in pairs}
    return (lambda v: P.Raw(to_p.get(str(v), str(v).lower())), lambda t: to_b.get(t, t.upper()))


def _obj():
    return (lambda v: v.name if v is not None else '', lambda t: Object(t) if t and t != 'none' else None)


_AX = _enum([('X', 'x'), ('Y', 'y'), ('Z', 'z')])

# Blender type -> {Blender attribute: (polyops key, to polyops, from polyops text)}; attributes
# that span several polyops keys are handled in Modifier itself.
_ATTRS = {
    'SUBSURF': {'levels': ('levels', _i, int), 'subdivision_type': ('simple', lambda v: v == 'SIMPLE', lambda t: 'SIMPLE' if t == '1' else 'CATMULL_CLARK')},
    'MIRROR': {'use_mirror_merge': ('merge', _b, lambda t: t == '1'), 'merge_threshold': ('merge_distance', _f, float)},
    'ARRAY': {'count': ('count', _i, int), 'use_merge_vertices': ('merge', _b, lambda t: t == '1')},
    'SOLIDIFY': {'thickness': ('thickness', _f, float), 'offset': ('offset', _f, float)},
    'BEVEL': {'width': ('width', _f, float), 'segments': ('segments', _i, int), 'angle_limit': ('angle', _f, float)},
    'WELD': {'merge_threshold': ('distance', _f, float)},
    'DISPLACE': {'strength': ('strength', _f, float)},
    'SMOOTH': {'factor': ('factor', _f, float), 'iterations': ('iterations', _i, int)},
    'DECIMATE': {'decimate_type': ('mode',) + _enum([('COLLAPSE', 'collapse'), ('UNSUBDIV', 'unsubdivide'), ('DISSOLVE', 'planar')]),
                 'ratio': ('ratio', _f, float), 'iterations': ('iterations', _i, int), 'angle_limit': ('angle', _f, float),
                 'use_collapse_triangulate': ('triangulate', _b, lambda t: t == '1')},
    'REMESH': {'mode': ('mode',) + _enum([('BLOCKS', 'blocks'), ('SMOOTH', 'smooth'), ('SHARP', 'smooth'), ('VOXEL', 'voxel')]),
               'voxel_size': ('voxel_size', _f, float), 'octree_depth': ('octree_depth', _i, int), 'scale': ('scale', _f, float),
               'use_smooth_shade': ('smooth_shade', _b, lambda t: t == '1')},
    'SCREW': {'axis': ('axis',) + _AX, 'angle': ('angle', _f, float), 'steps': ('steps', _i, int), 'render_steps': ('steps', _i, int),
              'screw_offset': ('screw_offset', _f, float), 'iterations': ('iterations', _i, int),
              'use_merge_vertices': ('merge', _b, lambda t: t == '1'), 'merge_threshold': ('merge_distance', _f, float),
              'use_normal_flip': ('flip', _b, lambda t: t == '1'), 'use_smooth_shade': ('smooth_shade', _b, lambda t: t == '1')},
    'WIREFRAME': {'thickness': ('thickness', _f, float), 'offset': ('offset', _f, float), 'use_even_offset': ('even', _b, lambda t: t == '1'),
                  'use_relative_offset': ('relative', _b, lambda t: t == '1'), 'use_boundary': ('boundary', _b, lambda t: t == '1'),
                  'use_replace': ('replace', _b, lambda t: t == '1')},
    'EDGE_SPLIT': {'split_angle': ('angle', _f, float), 'use_edge_angle': ('use_angle', _b, lambda t: t == '1'), 'use_edge_sharp': None},
    'BUILD': {'frame_start': ('start', _f, float), 'frame_duration': ('length', _f, float), 'use_reverse': ('reverse', _b, lambda t: t == '1'),
              'use_random_order': ('random', _b, lambda t: t == '1'), 'seed': ('seed', _i, int)},
    'BOOLEAN': {'object': ('object',) + _obj(), 'operation': ('operation',) + _enum([('INTERSECT', 'intersect'), ('UNION', 'union'), ('DIFFERENCE', 'difference')]),
                'solver': None, 'operand_type': None},
    'SIMPLE_DEFORM': {'deform_method': ('mode',) + _enum([('TWIST', 'twist'), ('BEND', 'bend'), ('TAPER', 'taper'), ('STRETCH', 'stretch')]),
                      'deform_axis': ('axis',) + _AX, 'factor': ('factor', _f, float), 'angle': ('factor', _f, float),
                      'lock_x': ('lock_x', _b, lambda t: t == '1'), 'lock_y': ('lock_y', _b, lambda t: t == '1'), 'lock_z': ('lock_z', _b, lambda t: t == '1')},
    'CAST': {'cast_type': ('type',) + _enum([('SPHERE', 'sphere'), ('CYLINDER', 'cylinder'), ('CUBOID', 'cuboid')]), 'factor': ('factor', _f, float),
             'radius': ('radius', _f, float), 'size': ('size', _f, float), 'use_x': ('x', _b, lambda t: t == '1'), 'use_y': ('y', _b, lambda t: t == '1'),
             'use_z': ('z', _b, lambda t: t == '1'), 'use_radius_as_size': ('size_from_radius', _b, lambda t: t == '1')},
    'WAVE': {'use_x': ('x', _b, lambda t: t == '1'), 'use_y': ('y', _b, lambda t: t == '1'), 'use_cyclic': ('cyclic', _b, lambda t: t == '1'),
             'use_normal': ('normals', _b, lambda t: t == '1'), 'falloff_radius': ('falloff', _f, float), 'height': ('height', _f, float),
             'width': ('width', _f, float), 'narrowness': ('narrowness', _f, float), 'speed': ('speed', _f, float),
             'time_offset': ('time_offset', _f, float), 'lifetime': ('lifetime', _f, float), 'damping_time': ('damping', _f, float)},
    'SHRINKWRAP': {'target': ('target',) + _obj(), 'wrap_method': ('method',) + _enum([('NEAREST_SURFACEPOINT', 'nearest_surface'), ('PROJECT', 'project'),
                   ('NEAREST_VERTEX', 'nearest_vertex')]), 'offset': ('offset', _f, float), 'use_project_x': ('x', _b, lambda t: t == '1'),
                   'use_project_y': ('y', _b, lambda t: t == '1'), 'use_project_z': ('z', _b, lambda t: t == '1'),
                   'use_positive_direction': ('positive', _b, lambda t: t == '1'), 'use_negative_direction': ('negative', _b, lambda t: t == '1')},
    'LAPLACIANSMOOTH': {'iterations': ('iterations', _i, int), 'lambda_factor': ('factor', _f, float), 'lambda_border': ('border', _f, float),
                        'use_x': ('x', _b, lambda t: t == '1'), 'use_y': ('y', _b, lambda t: t == '1'), 'use_z': ('z', _b, lambda t: t == '1'),
                        'use_volume_preserve': ('preserve_volume', _b, lambda t: t == '1'), 'use_normalized': ('normalized', _b, lambda t: t == '1')},
    'CORRECTIVE_SMOOTH': {'factor': ('factor', _f, float), 'iterations': ('iterations', _i, int), 'scale': ('scale', _f, float),
                          'smooth_type': ('length_weighted', lambda v: v == 'LENGTH_WEIGHTED', lambda t: 'LENGTH_WEIGHTED' if t == '1' else 'SIMPLE'),
                          'use_only_smooth': ('only_smooth', _b, lambda t: t == '1')},
    'WEIGHTED_NORMAL': {'keep_sharp': ('keep_sharp', _b, lambda t: t == '1')},
}

# Attributes Blender has that Polyamour keeps only in Python (with Blender's defaults).
_LOCAL = {'show_viewport': True, 'show_render': True, 'show_in_editmode': False, 'show_on_cage': False, 'show_expanded': True,
          'is_active': False, 'use_pin_to_last': False, 'render_levels': 2, 'quality': 3, 'use_creases': True, 'uv_smooth': 'PRESERVE_BOUNDARIES',
          'boundary_smooth': 'ALL', 'use_limit_surface': True, 'use_bisect_axis': (False, False, False), 'use_bisect_flip_axis': (False, False, False),
          'mirror_object': None, 'use_clip': False, 'use_mirror_vertex_groups': True, 'use_mirror_u': False, 'use_mirror_v': False,
          'fit_type': 'FIXED_COUNT', 'fit_length': 0.0, 'curve': None, 'start_cap': None, 'end_cap': None, 'use_object_offset': False,
          'offset_object': None, 'use_even_offset': False, 'use_rim': True, 'use_rim_only': False, 'use_flip_normals': False,
          'limit_method': 'ANGLE', 'affect': 'EDGES', 'offset_type': 'OFFSET', 'profile': 0.5, 'harden_normals': False,
          'quad_method': 'BEAUTY', 'ngon_method': 'BEAUTY', 'min_vertices': 4, 'mode_weld': 'ALL', 'texture': None,
          'texture_coords': 'LOCAL', 'direction': 'NORMAL', 'mid_level': 0.5, 'solver': 'EXACT', 'operand_type': 'OBJECT',
          'use_self': False, 'use_hole_tolerant': False, 'use_edge_sharp': True, 'deform_axis': 'Z', 'use_remove_disconnected': True,
          'adaptivity': 0.0, 'threshold': 1.0, 'use_merge_vertices': False, 'merge_threshold': 0.01}


class Modifier(bpy_struct):
    """A modifier on an object; `type` is Blender's. Attributes Polyamour
    implements are sent to it; others are kept in this Python."""

    def __init__(self, obj, index):
        self.__dict__['_o'] = obj
        self.__dict__['_i'] = index

    def _key(self):
        return ('modifier', self._o, self._i)

    def _j(self):
        return req('dump_object', self._o)['modifiers'][self._i]

    def _opts(self):
        return parse_opts(self._j()['opts'])

    def __repr__(self):
        return "bpy.data.objects['%s'].modifiers['%s']" % (self._o, self.name)

    @property
    def type(self):
        t = self._j()['type']
        return _TYPES.get(t, t.upper())

    @property
    def bl_rna(self):
        from ._base import _RNA
        return _RNA(self.type.title().replace('_', '') + 'Modifier')

    @property
    def name(self):
        names = _modifier_names(self._o)
        return names[self._i] if self._i < len(names) else self._j()['name']

    @name.setter
    def name(self, new):
        names = _modifier_names(self._o)
        names[self._i] = str(new)
        _custom_names[self._o] = names

    def _set(self, **opts):
        req('modifier_set', self._o, self._i, **opts)

    def __getattr__(self, key):
        if key.startswith('_'):
            raise AttributeError(key)
        t = self.type
        o = self._opts()
        spec = _ATTRS.get(t, {}).get(key)
        if spec:
            pk, _, from_p = spec
            if pk in o:
                return from_p(o[pk])
        if t == 'MIRROR' and key == 'use_axis':
            return BoolArray([o.get(k) == '1' for k in 'xyz'], lambda v: self._set(x=v[0], y=v[1], z=v[2]))
        if t == 'ARRAY':
            if key in ('use_relative_offset', 'use_constant_offset'):
                v = parse_value(o['relative' if key == 'use_relative_offset' else 'constant'])
                stored = self.get('_' + key)
                return stored if stored is not None else any(abs(x) > 0 for x in v)
            if key in ('relative_offset_displace', 'constant_offset_displace'):
                pk = 'relative' if key.startswith('relative') else 'constant'
                return FloatArray(parse_value(o[pk]), lambda v: self._set(**{pk: v}))
        if t == 'SIMPLE_DEFORM' and key == 'limits':
            return FloatArray(parse_value(o['limits']), lambda v: self._set(limits=v))
        if t == 'WAVE' and key in ('start_position_x', 'start_position_y'):
            return parse_value(o['start'])[0 if key.endswith('x') else 1]
        if key in o:
            # Rigging modifiers use Blender's names directly.
            v = o[key]
            if key in ('object', 'object_from', 'object_to', 'target'):
                return Object(v) if v and v != 'none' else None
            if key in ('vertex_group', 'subtarget', 'vertex_group_a', 'vertex_group_b', 'mask_vertex_group'):
                return v
            p = parse_value(v)
            if key == 'matrix_inverse' and isinstance(p, list):
                return _m4(p)
            if isinstance(p, str):
                return p.upper()
            if key.startswith('use_') or key.startswith('invert') or key == 'normalize':
                return bool(p)
            return p
        idp = self._idp()
        if key in idp:
            return idp[key]
        if key in _LOCAL:
            return _LOCAL[key]
        if key in ('vertex_group',):
            return ''
        if key in ('invert_vertex_group',):
            return False
        raise AttributeError("'%s' modifier has no attribute '%s'" % (t, key))

    def __setattr__(self, key, value):
        if key == 'name':
            Modifier.name.fset(self, value)
            return
        t = self.type
        spec = _ATTRS.get(t, {}).get(key)
        if spec:
            pk, to_p, _ = spec
            self._set(**{pk: to_p(value)})
            return
        if t == 'MIRROR' and key == 'use_axis':
            v = [bool(x) for x in value]
            self._set(x=v[0], y=v[1], z=v[2])
            return
        if t == 'ARRAY' and key in ('use_relative_offset', 'use_constant_offset', 'relative_offset_displace', 'constant_offset_displace'):
            o = self._opts()
            if key.startswith('use_'):
                self['_' + key] = bool(value)
                pk = 'relative' if 'relative' in key else 'constant'
                saved = self.get('_saved_' + pk) or ([1.0, 0.0, 0.0] if pk == 'relative' else [0.0, 0.0, 0.0])
                cur = parse_value(o[pk])
                if value:
                    self._set(**{pk: saved})
                else:
                    if any(abs(x) > 0 for x in cur):
                        self['_saved_' + pk] = cur
                    self._set(**{pk: [0.0, 0.0, 0.0]})
            else:
                pk = 'relative' if key.startswith('relative') else 'constant'
                use = self.get('_use_' + pk + ('_offset'), pk == 'relative')
                if use:
                    self._set(**{pk: [float(x) for x in value]})
                else:
                    self['_saved_' + pk] = [float(x) for x in value]
            return
        if t == 'SIMPLE_DEFORM' and key == 'limits':
            self._set(limits=[float(x) for x in value])
            return
        if t == 'WAVE' and key in ('start_position_x', 'start_position_y'):
            s = parse_value(self._opts()['start'])
            s[0 if key.endswith('x') else 1] = float(value)
            self._set(start=s)
            return
        o = self._opts()
        if key in o or key in ('vertex_group', 'invert_vertex_group', 'object', 'object_from', 'object_to', 'subtarget'):
            if isinstance(value, ID):
                value = value.name
            elif value is None:
                value = P.Raw('none')
            elif isinstance(value, str) and key not in ('vertex_group', 'subtarget', 'vertex_group_a', 'vertex_group_b', 'mask_vertex_group'):
                value = P.Raw(value.lower())
            elif hasattr(value, 'col') and hasattr(value, 'row'):
                value = _cols(value)
            self._set(**{key: value})
            return
        if key == 'node_group':
            self._set(node_group=value.name if value is not None else P.Raw('none'))
            return
        self._idp()[key] = value


_custom_names = {}


def _modifier_names(obj):
    mods = req('dump_object', obj)['modifiers']
    names = list(_custom_names.get(obj, []))
    names = names[:len(mods)]
    used = set(n for n in names if n)
    while len(names) < len(mods):
        base = mods[len(names)]['name']
        n, k = base, 0
        while n in used:
            k += 1
            n = '%s.%03d' % (base, k)
        used.add(n)
        names.append(n)
    return names


class ObjectModifiers(Collection_):
    def __init__(self, obj):
        self._o = obj

    def _items(self):
        return [Modifier(self._o, i) for i in range(len(req('dump_object', self._o)['modifiers']))]

    def new(self, name, type):
        ptype = _FROM_BLENDER.get(type)
        if ptype is None:
            raise TypeError("modifiers.new(): type '%s' is not supported by Polyamour" % type)
        names = _modifier_names(self._o)
        i = int(first(req('modifier_add', self._o, P.Raw(ptype))))
        used = set(names)
        n, k = name, 0
        while n in used:
            k += 1
            n = '%s.%03d' % (name, k)
        _custom_names[self._o] = names + [n]
        return Modifier(self._o, i)

    def remove(self, modifier):
        names = _modifier_names(self._o)
        req('modifier_remove', self._o, modifier._i)
        names.pop(modifier._i)
        _custom_names[self._o] = names

    def clear(self):
        req('modifier_clear', self._o)
        _custom_names[self._o] = []

    def move(self, from_index, to_index):
        names = _modifier_names(self._o)
        req('modifier_move', self._o, int(from_index), int(to_index))
        names.insert(int(to_index), names.pop(int(from_index)))
        _custom_names[self._o] = names

    @property
    def active(self):
        items = self._items()
        return items[-1] if items else None


# --- material slots -------------------------------------------------------------------------

class MaterialSlot(bpy_struct):
    def __init__(self, obj, i):
        self._o = obj
        self._i = i

    def _key(self):
        return ('slot', self._o, self._i)

    @property
    def material(self):
        n = req('dump_object', self._o)['materials'][self._i]
        return Material(n) if n else None

    @material.setter
    def material(self, mat):
        req('material_slot', self._o, self._i, mat.name if mat is not None else P.Raw('none'))

    @property
    def name(self):
        m = self.material
        return m.name if m else ''

    @property
    def slot_index(self):
        return self._i

    link = 'OBJECT'


class MaterialSlots(Collection_):
    def __init__(self, obj):
        self._o = obj

    def _items(self):
        return [MaterialSlot(self._o, i) for i in range(len(req('dump_object', self._o)['materials']))]


# --- lights and cameras ---------------------------------------------------------------------

_LIGHT_KEYS = {'energy': 'power', 'color': 'color', 'shadow_soft_size': 'radius', 'spot_size': 'angle', 'spot_blend': 'blend',
               'size': 'size', 'size_y': 'size_y', 'shape': 'shape', 'use_shadow': 'cast_shadow', 'spread': 'spread',
               'angle': 'angle', 'max_bounces': 'max_bounces'}
_LIGHT_TYPES = ('POINT', 'SUN', 'SPOT', 'AREA')


class Light(ID):
    """Light data. Polyamour keeps a light inside its object: a light made by
    bpy.data.lights.new() waits here until bpy.data.objects.new() uses it."""

    _kind = 'light'
    _collection_name = 'lights'

    def __init__(self, name, obj=None, pending=None):
        ID.__init__(self, name)
        self.__dict__['_obj'] = obj
        self.__dict__['_pending'] = pending

    def _j(self):
        return req('dump_object', self._obj)['light']

    def _rename(self, new):
        if self._obj is None:
            return new
        return Object(self._obj)._rename(new)

    @property
    def type(self):
        if self._obj is None:
            return self._pending['type']
        return self._j()['type']

    @type.setter
    def type(self, t):
        if self._obj is None:
            self._pending['type'] = t
        else:
            raise NotImplementedError('changing an existing light type is not supported yet')

    def __getattr__(self, key):
        if key.startswith('_'):
            raise AttributeError(key)
        if self._obj is None:
            p = self._pending
            if key in p:
                return p[key]
            return {'energy': 10.0 if p['type'] != 'SUN' else 1.0, 'color': (1.0, 1.0, 1.0), 'shadow_soft_size': 0.25,
                    'spot_size': math.radians(45), 'spot_blend': 0.15, 'size': 0.25, 'use_shadow': True}.get(key, 0.0)
        j = self._j()
        if key == 'use_shadow':
            return j['cast_shadow']
        if key == 'color':
            return FloatArray(j['color'], lambda v: setattr(self, 'color', v))
        if key == 'angle' and j['type'] == 'SUN':
            return j['angle']
        if key in j:
            return j[key]
        if key in ('use_nodes', 'cycles'):
            return False
        raise AttributeError("'Light' object has no attribute '%s'" % key)

    def __setattr__(self, key, value):
        if key in ('name', 'type'):
            object.__setattr__(self, key, value)
            return
        if self._obj is None:
            self._pending[key] = value
            return
        pk = _LIGHT_KEYS.get(key)
        if pk is None:
            self._idp()[key] = value
            return
        if key == 'angle' and self.type == 'SUN':
            pk = 'radius'
        if key == 'shape':
            value = P.Raw(str(value).lower())
        req('set', P.Raw(P.quote(self._obj) + '.' + pk), *(value if isinstance(value, (list, tuple, FloatArray, Vector)) else [value]))

    def _create(self, obj_name):
        p = self._pending
        opts = {}
        for k, v in p.items():
            if k == 'type':
                continue
            pk = _LIGHT_KEYS.get(k)
            if pk and pk in ('power', 'color', 'radius', 'angle', 'blend', 'size'):
                opts[pk] = list(v) if isinstance(v, (list, tuple, FloatArray, Vector)) else v
        return first(req('light_add', P.Raw(p['type'].lower()), name=obj_name, **opts))


_CAM_KEYS = {'lens': 'lens', 'angle': 'fov', 'clip_start': 'near', 'clip_end': 'far', 'ortho_scale': 'ortho_scale',
             'sensor_width': 'sensor_width', 'sensor_height': 'sensor_height', 'sensor_fit': 'sensor_fit',
             'shift_x': 'shift_x', 'shift_y': 'shift_y'}


class _CameraDof(bpy_struct):
    def __init__(self, cam):
        self._c = cam

    def __getattr__(self, key):
        if key.startswith('_'):
            raise AttributeError(key)
        j = self._c._j() if self._c._obj else {}
        k = {'use_dof': 'dof', 'focus_distance': 'focus_distance', 'aperture_fstop': 'fstop', 'aperture_blades': 'blades',
             'aperture_rotation': 'aperture_rotation', 'aperture_ratio': 'aperture_ratio'}.get(key, key)
        if k in j:
            return parse_value(j[k]) if isinstance(j[k], str) else j[k]
        return {'use_dof': False, 'focus_distance': 10.0, 'aperture_fstop': 2.8, 'focus_object': None}.get(key, 0.0)

    def __setattr__(self, key, value):
        if key.startswith('_'):
            object.__setattr__(self, key, value)
            return
        k = {'use_dof': 'dof', 'focus_distance': 'focus_distance', 'aperture_fstop': 'fstop', 'aperture_blades': 'blades',
             'aperture_rotation': 'aperture_rotation', 'aperture_ratio': 'aperture_ratio'}.get(key)
        if key == 'focus_object':
            req('set', P.Raw(P.quote(self._c._obj) + '.focus_object'), value.name if value is not None else P.Raw('none'))
        elif k:
            req('set', P.Raw(P.quote(self._c._obj) + '.' + k), value)


class Camera(ID):
    _kind = 'camera'
    _collection_name = 'cameras'

    def __init__(self, name, obj=None, pending=None):
        ID.__init__(self, name)
        self.__dict__['_obj'] = obj
        self.__dict__['_pending'] = pending

    def _j(self):
        return req('dump_object', self._obj)['camera']

    def _rename(self, new):
        if self._obj is None:
            return new
        return Object(self._obj)._rename(new)

    @property
    def dof(self):
        return _CameraDof(self)

    def __getattr__(self, key):
        if key.startswith('_'):
            raise AttributeError(key)
        if self._obj is None:
            p = self._pending
            if key in p:
                return p[key]
            return {'lens': 50.0, 'clip_start': 0.1, 'clip_end': 1000.0, 'type': 'PERSP', 'ortho_scale': 6.0, 'sensor_width': 36.0,
                    'sensor_fit': 'AUTO', 'shift_x': 0.0, 'shift_y': 0.0}.get(key, 0.0)
        j = self._j()
        if key == 'angle_y':
            return j['angle']
        if key == 'angle_x':
            return 2 * math.atan(j['sensor_width'] / (2 * j['lens']))
        if key in j:
            return j[key]
        if key in ('display_size',):
            return 1.0
        if key in ('passepartout_alpha',):
            return 0.5
        if key in ('show_passepartout', 'show_limits', 'show_name'):
            return False
        raise AttributeError("'Camera' object has no attribute '%s'" % key)

    def __setattr__(self, key, value):
        if key == 'name':
            object.__setattr__(self, key, value)
            return
        if self._obj is None:
            self._pending[key] = value
            return
        if key == 'type':
            if value == 'ORTHO':
                req('set', P.Raw(P.quote(self._obj) + '.ortho_scale'), self._j()['ortho_scale'] or 6.0)
            elif value == 'PERSP':
                req('set', P.Raw(P.quote(self._obj) + '.ortho_scale'), P.Raw('none'))
            elif value == 'PANO':
                req('set', P.Raw(P.quote(self._obj) + '.panorama'), P.Raw('equirectangular'))
            return
        pk = _CAM_KEYS.get(key)
        if pk is None:
            self._idp()[key] = value
            return
        if pk == 'sensor_fit':
            value = P.Raw(str(value).lower())
        req('set', P.Raw(P.quote(self._obj) + '.' + pk), value)

    def _create(self, obj_name):
        p = self._pending
        opts = {}
        if 'lens' in p:
            opts['lens'] = float(p['lens'])
        if 'angle' in p:
            opts['fov'] = float(p['angle'])
        if 'clip_start' in p:
            opts['near'] = float(p['clip_start'])
        if 'clip_end' in p:
            opts['far'] = float(p['clip_end'])
        if p.get('type') == 'ORTHO':
            opts['ortho_scale'] = float(p.get('ortho_scale', 6.0))
        return first(req('camera_add', name=obj_name, **opts))

    def view_frame(self, scene=None):
        j = self._j()
        aspect = 16.0 / 9.0
        if scene is not None:
            r = scene.render
            aspect = r.resolution_x / max(1, r.resolution_y)
        if j['type'] == 'ORTHO':
            hw = j['ortho_scale'] / 2
            hh = hw / aspect
            d = -1.0
        else:
            hw = j['sensor_width'] / 2 / j['lens']
            hh = hw / aspect
            d = -1.0
        return (Vector((hw, hh, d)), Vector((hw, -hh, d)), Vector((-hw, -hh, d)), Vector((-hw, hh, d)))


# --- objects -----------------------------------------------------------------------------------

class Object(ID):
    _kind = 'object'
    _collection_name = 'objects'

    def _j(self):
        return req('dump_object', self._name)

    def _rename(self, new):
        req('set', P.Raw(P.quote(self._name) + '.name'), new)
        names = req('ids')['objects']
        if new in names:
            return new
        cands = [n for n in names if n.startswith(new + '.')]
        return cands[-1] if cands else new

    def _set(self, **opts):
        req('obj_set', self._name, **opts)

    @property
    def type(self):
        return self._j()['type']

    @property
    def data(self):
        j = self._j()
        t = j['type']
        if t == 'MESH':
            return Mesh(j['data'])
        if t == 'LIGHT':
            return Light(self._name, obj=self._name)
        if t == 'CAMERA':
            return Camera(self._name, obj=self._name)
        if t == 'ARMATURE':
            return Armature(j['data'])
        if t == 'LATTICE':
            return Lattice(j['data'])
        return None

    @data.setter
    def data(self, d):
        if isinstance(d, Mesh) and self.type == 'MESH':
            req('set', P.Raw(P.quote(self._name) + '.data'), d.name)
        else:
            raise NotImplementedError('only a mesh object can switch to another mesh')

    # transforms
    @property
    def location(self):
        return Vector(self._j()['location'], _owner=lambda v: self._set(location=list(v)))

    @location.setter
    def location(self, v):
        self._set(location=[float(c) for c in v])

    @property
    def rotation_euler(self):
        j = self._j()
        mode = j['rotation_mode']
        return Euler(j['rotation_euler'], mode if len(mode) == 3 else 'XYZ', _owner=lambda e: self._set(rotation_euler=list(e)))

    @rotation_euler.setter
    def rotation_euler(self, e):
        self._set(rotation_euler=[float(c) for c in e])

    @property
    def rotation_quaternion(self):
        return Quaternion(self._j()['rotation_quaternion'], _owner=lambda q: self._set(rotation_quaternion=list(q)))

    @rotation_quaternion.setter
    def rotation_quaternion(self, q):
        self._set(rotation_quaternion=[float(c) for c in q])

    @property
    def rotation_axis_angle(self):
        axis, angle = Quaternion(self._j()['rotation_quaternion']).to_axis_angle()

        def write(v):
            self._set(rotation_quaternion=list(Quaternion(Vector(v[1:4]), v[0])))
        return FloatArray([angle, axis[0], axis[1], axis[2]], write)

    @rotation_axis_angle.setter
    def rotation_axis_angle(self, v):
        self._set(rotation_quaternion=list(Quaternion(Vector(v[1:4]), v[0])))

    @property
    def rotation_mode(self):
        return self._j()['rotation_mode']

    @rotation_mode.setter
    def rotation_mode(self, m):
        self._set(rotation_mode=P.Raw(str(m)))

    @property
    def scale(self):
        return Vector(self._j()['scale'], _owner=lambda v: self._set(scale=list(v)))

    @scale.setter
    def scale(self, v):
        self._set(scale=[float(c) for c in v])

    def _delta(key, default):
        return property(lambda self: Vector(self.get('_' + key, default)), lambda self, v: self.__setitem__('_' + key, tuple(v)))

    delta_location = _delta('delta_location', (0.0, 0.0, 0.0))
    delta_rotation_euler = _delta('delta_rotation_euler', (0.0, 0.0, 0.0))
    delta_scale = _delta('delta_scale', (1.0, 1.0, 1.0))
    del _delta

    @property
    def matrix_world(self):
        return Matrix(_m4(self._j()['matrix_world']), _owner=lambda m: self._set(matrix_world=_cols(m)))

    @matrix_world.setter
    def matrix_world(self, m):
        self._set(matrix_world=_cols(Matrix(m)))

    @property
    def matrix_local(self):
        j = self._j()
        m = _m4(j['matrix_parent_world']).inverted() @ _m4(j['matrix_world'])
        return Matrix(m, _owner=lambda m: self._set(matrix_local=_cols(m)))

    @matrix_local.setter
    def matrix_local(self, m):
        self._set(matrix_local=_cols(Matrix(m)))

    matrix_basis = matrix_local

    @property
    def matrix_parent_inverse(self):
        return Matrix.Identity(4)

    @matrix_parent_inverse.setter
    def matrix_parent_inverse(self, m):
        pass

    @property
    def dimensions(self):
        b = self._j()['bounds']
        if not b:
            return Vector((0.0, 0.0, 0.0))
        s = self._j()['scale']
        return Vector([(b[1][k] - b[0][k]) * abs(s[k]) for k in range(3)])

    @dimensions.setter
    def dimensions(self, d):
        b = self._j()['bounds']
        if not b:
            return
        s = list(self._j()['scale'])
        for k in range(3):
            ext = b[1][k] - b[0][k]
            if ext > 0:
                s[k] = float(d[k]) / ext
        self._set(scale=s)

    @property
    def bound_box(self):
        b = self._j()['bounds'] or [[0, 0, 0], [0, 0, 0]]
        lo, hi = b
        return [(x, y, z) for x in (lo[0], hi[0]) for y in (lo[1], hi[1]) for z in (lo[2], hi[2])]

    # relations
    @property
    def parent(self):
        p = self._j()['parent']
        return Object(p) if p else None

    @parent.setter
    def parent(self, p):
        req('parent', self._name, p.name if p is not None else P.Raw('none'))

    @property
    def parent_type(self):
        return self._j()['parent_type']

    @parent_type.setter
    def parent_type(self, t):
        p = self.parent
        if p is not None:
            req('parent', self._name, p.name, type=P.Raw(str(t).lower()), bone=self.parent_bone or None)

    @property
    def parent_bone(self):
        return self._j()['parent_bone'] or ''

    @parent_bone.setter
    def parent_bone(self, b):
        p = self.parent
        if p is not None and b:
            req('parent', self._name, p.name, type=P.Raw('bone'), bone=b)

    @property
    def children(self):
        return tuple(Object(c) for c in self._j()['children'])

    @property
    def children_recursive(self):
        out = []
        for c in self.children:
            out.append(c)
            out.extend(c.children_recursive)
        return out

    @property
    def users_collection(self):
        from .scene import Collection
        c = self._j()['collection']
        return (Collection(c),) if c else ()

    @property
    def users_scene(self):
        from . import context
        return (context.scene,)

    @property
    def users(self):
        return 1

    # stacks
    @property
    def modifiers(self):
        return ObjectModifiers(self._name)

    @property
    def constraints(self):
        return Constraints(self._name)

    @property
    def vertex_groups(self):
        return VertexGroups(self._name)

    @property
    def material_slots(self):
        return MaterialSlots(self._name)

    @property
    def active_material(self):
        mats = self._j()['materials']
        i = min(self.active_material_index, len(mats) - 1)
        return Material(mats[i]) if mats and mats[i] else None

    @active_material.setter
    def active_material(self, mat):
        n = len(self._j()['materials'])
        i = min(self.active_material_index, max(n - 1, 0))
        req('material_slot', self._name, i, mat.name if mat is not None else P.Raw('none'))

    @property
    def active_material_index(self):
        return self.get('_active_material_index', 0)

    @active_material_index.setter
    def active_material_index(self, i):
        self['_active_material_index'] = int(i)

    @property
    def pose(self):
        return Pose(self._name) if self.type == 'ARMATURE' else None

    @property
    def animation_data(self):
        if self._j().get('action') or self.get('_animdata'):
            return AnimData(self._name)
        return None

    def animation_data_create(self):
        self['_animdata'] = True
        return AnimData(self._name)

    def animation_data_clear(self):
        req('action_assign', self._name, P.Raw('none'))
        self.pop('_animdata', None)

    def keyframe_insert(self, data_path, index=-1, frame=None, group='', options=set(), keytype='KEYFRAME'):
        from .anim import keyframe_insert
        return keyframe_insert(self, data_path, index, frame, group)

    def keyframe_delete(self, data_path, index=-1, frame=None, group=''):
        from .anim import keyframe_delete
        return keyframe_delete(self, data_path, index, frame)

    def shape_key_add(self, name='Key', from_mix=True):
        if self.type != 'MESH':
            raise TypeError('only mesh objects have shape keys here')
        m = self.data
        m._flush_now()
        have = [k['name'] for k in m._d().keys]
        n, k = name, 0
        while n in have:
            k += 1
            n = '%s.%03d' % (name, k)
        req('shape_key', m.name, n)
        from .mesh import ShapeKey
        return ShapeKey(Mesh(m.name), n)

    def shape_key_remove(self, key):
        req('shape_key_remove', self.data.name, key.name)

    def shape_key_clear(self):
        for k in list((self.data.shape_keys and self.data.shape_keys.key_blocks) or [])[::-1]:
            self.shape_key_remove(k)

    @property
    def active_shape_key(self):
        sk = self.data.shape_keys if self.type == 'MESH' else None
        if not sk:
            return None
        blocks = sk.key_blocks._items()
        i = min(self.get('_active_shape_key_index', len(blocks) - 1), len(blocks) - 1)
        return blocks[i]

    @property
    def active_shape_key_index(self):
        return self.get('_active_shape_key_index', 0)

    @active_shape_key_index.setter
    def active_shape_key_index(self, i):
        self['_active_shape_key_index'] = int(i)

    show_only_shape_key = False

    # visibility and selection
    @property
    def hide_viewport(self):
        return self._j()['hide_viewport']

    @hide_viewport.setter
    def hide_viewport(self, h):
        req('set', P.Raw(P.quote(self._name) + '.visible'), not h)

    @property
    def hide_render(self):
        return self._j()['hide_render']

    @hide_render.setter
    def hide_render(self, h):
        self._set(hide_render=bool(h))

    def hide_get(self, view_layer=None):
        return self.hide_viewport

    def hide_set(self, state, view_layer=None):
        self.hide_viewport = state

    hide_select = False

    def visible_get(self, view_layer=None, viewport=None):
        return not self.hide_viewport

    def select_get(self, view_layer=None):
        return self._j()['select']

    def select_set(self, state, view_layer=None):
        self._set(select=bool(state))

    def holdout_get(self, view_layer=None):
        return self._j()['holdout']

    def indirect_only_get(self, view_layer=None):
        return False

    def local_view_get(self, viewport):
        return False

    def visible_in_viewport_get(self, viewport):
        return not self.hide_viewport

    @property
    def pass_index(self):
        return self._j()['pass_index']

    @pass_index.setter
    def pass_index(self, v):
        req('set', P.Raw(P.quote(self._name) + '.pass_index'), int(v))

    def _vis(key):
        return property(lambda self: self._j()[key], lambda self, v: req('set', P.Raw(P.quote(self._name) + '.' + key), bool(v)))

    visible_camera = _vis('visible_camera')
    visible_diffuse = _vis('visible_diffuse')
    visible_glossy = _vis('visible_glossy')
    visible_transmission = _vis('visible_transmission')
    visible_volume_scatter = _vis('visible_volume_scatter')
    visible_shadow = _vis('visible_shadow')
    is_holdout = _vis('holdout')
    del _vis

    def _local(key, default):
        return property(lambda self: self.get('_' + key, default), lambda self, v: self.__setitem__('_' + key, v))

    display_type = _local('display_type', 'TEXTURED')
    show_name = _local('show_name', False)
    show_axis = _local('show_axis', False)
    show_wire = _local('show_wire', False)
    show_in_front = _local('show_in_front', False)
    color = _local('color', (1.0, 1.0, 1.0, 1.0))
    empty_display_type = _local('empty_display_type', 'PLAIN_AXES')
    empty_display_size = _local('empty_display_size', 1.0)
    instance_type = _local('instance_type', 'NONE')
    instance_collection = _local('instance_collection', None)
    track_axis = _local('track_axis', 'POS_Y')
    up_axis = _local('up_axis', 'Z')
    lock_location = _local('lock_location', (False, False, False))
    lock_rotation = _local('lock_rotation', (False, False, False))
    lock_scale = _local('lock_scale', (False, False, False))
    del _local

    @property
    def mode(self):
        from . import context
        a = context.active_object
        if a is not None and a.name == self._name:
            return context.object_mode
        return 'OBJECT'

    # evaluation
    def evaluated_get(self, depsgraph):
        return EvaluatedObject(self._name)

    def to_mesh(self, preserve_all_data_layers=False, depsgraph=None):
        if self.type != 'MESH':
            raise RuntimeError('Object does not have geometry data')
        return EvaluatedObject(self._name).to_mesh()

    def to_mesh_clear(self):
        pass

    def ray_cast(self, origin, direction, distance=1.70141e38, depsgraph=None):
        from mathutils.bvhtree import BVHTree
        me = self.evaluated_get(depsgraph).to_mesh()
        bvh = BVHTree.FromPolygons([v.co for v in me.vertices], [p.vertices for p in me.polygons])
        loc, nrm, idx, dist = bvh.ray_cast(origin, direction, distance)
        if loc is None:
            return False, Vector((0, 0, 0)), Vector((0, 0, 0)), -1
        return True, loc, nrm, idx

    def closest_point_on_mesh(self, origin, distance=1.84467e19, depsgraph=None):
        from mathutils.bvhtree import BVHTree
        me = self.evaluated_get(depsgraph).to_mesh()
        bvh = BVHTree.FromPolygons([v.co for v in me.vertices], [p.vertices for p in me.polygons])
        loc, nrm, idx, dist = bvh.find_nearest(origin, distance)
        if loc is None:
            return False, Vector((0, 0, 0)), Vector((0, 0, 0)), -1
        return True, loc, nrm, idx

    def copy(self):
        return Object(first(req('object_copy', self._name, linked=True)))

    def update_from_editmode(self):
        return False

    def convert_space(self, pose_bone=None, matrix=None, from_space='WORLD', to_space='WORLD'):
        m = Matrix(matrix) if matrix is not None else Matrix.Identity(4)
        mw = self.matrix_world
        if from_space == to_space:
            return m
        if from_space == 'WORLD':
            m = mw.inverted() @ m
        if to_space == 'WORLD':
            m = mw @ m
        return m

    def calc_matrix_camera(self, depsgraph, x=1, y=1, scale_x=1.0, scale_y=1.0):
        cam = self.data
        near, far = cam.clip_start, cam.clip_end
        fov = cam.angle
        aspect = (x * scale_x) / (y * scale_y)
        f = 1.0 / math.tan(fov / 2)
        return Matrix(((f / aspect if aspect >= 1 else f, 0, 0, 0), (0, f if aspect >= 1 else f * aspect, 0, 0),
                       (0, 0, (far + near) / (near - far), 2 * far * near / (near - far)), (0, 0, -1, 0)))


class _EvaluatedMesh(Mesh):
    """The object's mesh with its modifiers applied (read-only copy)."""

    _kind = 'evaluated_mesh'

    def __init__(self, obj):
        Mesh.__init__(self, obj)
        from .mesh import _Data
        self.__dict__['_data'] = _Data(req('eval_mesh', obj))

    def _d(self):
        return self._data

    def _touch(self, flag):
        self._data.touch(flag)

    def _flush(self):
        pass

    def _flush_now(self):
        pass

    @property
    def name(self):
        return self._name

    @property
    def materials(self):
        return [Material(n) if n else None for n in req('dump_object', self._name)['materials']]


class EvaluatedObject(Object):
    """Object.evaluated_get(depsgraph): the same object, reading evaluated data."""

    @property
    def is_evaluated(self):
        return True

    @property
    def original(self):
        return Object(self._name)

    def to_mesh(self, preserve_all_data_layers=False, depsgraph=None):
        return _EvaluatedMesh(self._name)

    def to_mesh_clear(self):
        pass

    @property
    def data(self):
        if self.type == 'MESH':
            return _EvaluatedMesh(self._name)
        return Object.data.fget(self)


# The rest of bpy.types, for `isinstance` checks and registration targets.
from .scene import (Collection, Depsgraph, DepsgraphObjectInstance, DepsgraphUpdate, LayerCollection, RenderSettings, Scene,  # noqa: E402
                    ViewLayer, BlendData, Context)
from .rna import (AddonPreferences, Gizmo, GizmoGroup, Header, KeyingSetInfo, Macro, Menu, Operator, OperatorFileListElement,  # noqa: E402
                  Panel, PropertyGroup, UIList, UILayout, WindowManager, Event, Timer, Area, Region, Space, SpaceView3D,
                  SpaceProperties, Window, Screen, Preferences, Addon, Addons, Text, RenderEngine)

bpy_prop_collection = Collection_
bpy_prop_array = FloatArray
Property = object
MeshVertices = __import__('polyamour.mesh', fromlist=['MeshVertices']).MeshVertices
MeshPolygons = __import__('polyamour.mesh', fromlist=['MeshPolygons']).MeshPolygons
MeshEdges = __import__('polyamour.mesh', fromlist=['MeshEdges']).MeshEdges
MeshLoops = __import__('polyamour.mesh', fromlist=['MeshLoops']).MeshLoops
BlendDataObjects = BlendDataMeshes = Collection_
Curve = TextCurve = GreasePencil = MetaBall = Volume = PointCloud = Speaker = LightProbe = ID
SubsurfModifier = MirrorModifier = ArrayModifier = SolidifyModifier = BevelModifier = Modifier
ArmatureModifier = BooleanModifier = DisplaceModifier = NodesModifier = Modifier
ShaderNode = ShaderNodeTree = GeometryNodeTree = CompositorNodeTree = NodeGroup = NodeSocketInterface = None
ShaderNodeTree = GeometryNodeTree = CompositorNodeTree = NodeTree
ShaderNode = NodeInternal = Node
ShaderNodeTexImage = ShaderNodeBsdfPrincipled = ShaderNodeGroup = GeometryNodeGroup = Node
ShaderNodeTexNoise = ShaderNodeMath = ShaderNodeMixRGB = ShaderNodeMix = ShaderNodeValToRGB = ShaderNodeOutputMaterial = Node
PointLight = SunLight = SpotLight = AreaLight = Light
Keyframe = Keyframe
bpy_struct = bpy_struct
ID = ID
Constraint = Constraint
CopyLocationConstraint = CopyRotationConstraint = TrackToConstraint = DampedTrackConstraint = KinematicConstraint = Constraint
ChildOfConstraint = ArmatureConstraint = Constraint
