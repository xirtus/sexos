# SPDX-License-Identifier: GPL-3.0-only
"""Armatures, bones, edit bones, poses, constraints, vertex groups and
lattices (bpy.types.Armature, Bone, EditBone, Pose, PoseBone, Constraint,
VertexGroup, Lattice).

Polyamour has no separate edit-bone copy: `armature.edit_bones` edits the
bones directly (Edit Mode is not required, but works as in Blender)."""
import math

from mathutils import Matrix, Quaternion, Vector, Euler

from . import polyops as P
from ._base import ID, Collection_, bpy_struct, first, parse_opts, parse_value, req


def _m4(flat):
    """A column-major 16-number list as a Matrix."""
    return Matrix([[flat[c * 4 + r] for c in range(4)] for r in range(4)])


def _cols(m):
    return [m[r][c] for c in range(4) for r in range(4)]


def vec_roll_to_mat3(vec, roll):
    """Blender's bone matrix from a direction and a roll."""
    nor = Vector(vec).normalized()
    x, y, z = nor
    theta = 1.0 + y
    theta_alt = x * x + z * z
    safe, critical = 6.1e-3, 2.5e-4
    if theta > safe or theta_alt > critical * critical:
        if theta <= safe:
            theta = theta_alt * 0.5 + theta_alt * theta_alt * 0.125
        r = Matrix(((1 - x * x / theta, x, -x * z / theta),
                    (-x, y, -z),
                    (-x * z / theta, z, 1 - z * z / theta)))
    else:
        r = Matrix(((-1, 0, 0), (0, -1, 0), (0, 0, 1)))
    b = Matrix.Rotation(roll, 3, nor)
    return b @ r


def mat3_to_vec_roll(mat):
    vec = Vector((mat[0][1], mat[1][1], mat[2][1]))
    vecmat = vec_roll_to_mat3(vec, 0.0)
    rollmat = vecmat.inverted() @ mat
    return vec, math.atan2(rollmat[0][2], rollmat[2][2])


# --- armature data ------------------------------------------------------------------------

class Armature(ID):
    _kind = 'armature'
    _collection_name = 'armatures'

    def _j(self):
        return req('dump_armature', self._name)

    def _bone_j(self, name):
        for b in self._j()['bones']:
            if b['name'] == name:
                return b
        raise ReferenceError('bone "%s" was removed' % name)

    @property
    def bones(self):
        return ArmatureBones(self, Bone)

    @property
    def edit_bones(self):
        return ArmatureBones(self, EditBone)

    @property
    def collections(self):
        return [BoneCollection(self, n) for n in self._j()['collections']]

    @property
    def collections_all(self):
        return self.collections

    display_type = 'OCTAHEDRAL'
    show_names = False
    show_axes = False
    pose_position = 'POSE'

    def transform(self, matrix):
        for b in self.edit_bones:
            b.transform(matrix)


class BoneCollection(bpy_struct):
    def __init__(self, arm, name):
        self._a = arm
        self.name = name

    def assign(self, bone):
        req('bone_collection', self._a.name, self.name, bone.name)
        return True

    is_visible = True


class _BoneBase(bpy_struct):
    def __init__(self, arm, name):
        self._a = arm
        self.__dict__['_n'] = name

    def _key(self):
        return ('bone', self._a._name, self._n)

    def _j(self):
        return self._a._bone_j(self._n)

    def __repr__(self):
        return "bpy.data.armatures['%s'].%s['%s']" % (self._a._name, self._seq, self._n)

    def _set(self, **opts):
        req('bone', self._a._name, self._n, **opts)

    @property
    def name(self):
        return self._n

    @name.setter
    def name(self, new):
        got = first(req('rename', 'bone', '%s/%s' % (self._a._name, self._n), new))
        self.__dict__['_n'] = got or new

    def _v(self, key):
        def write(v):
            self._set(**{key: [v[0], v[1], v[2]]})
        return Vector(self._j()[key], _owner=write)

    @property
    def parent(self):
        p = self._j()['parent']
        return type(self)(self._a, p) if p else None

    @property
    def children(self):
        return [type(self)(self._a, c) for c in self._j()['children']]

    @property
    def children_recursive(self):
        out = []
        for c in self.children:
            out.append(c)
            out.extend(c.children_recursive)
        return out

    @property
    def parent_recursive(self):
        out = []
        p = self.parent
        while p is not None:
            out.append(p)
            p = p.parent
        return out

    @property
    def length(self):
        return self._j()['length']

    @property
    def vector(self):
        j = self._j()
        return Vector(j['tail']) - Vector(j['head'])

    @property
    def matrix_local(self):
        return _m4(self._j()['matrix_local'])

    @property
    def x_axis(self):
        return self.matrix_local.col[0].xyz

    @property
    def y_axis(self):
        return self.matrix_local.col[1].xyz

    @property
    def z_axis(self):
        return self.matrix_local.col[2].xyz

    @property
    def collections(self):
        return [BoneCollection(self._a, n) for n in self._j()['collections']]

    def _flag(key):
        return property(lambda self: self._j()[key], lambda self, v: self._set(**{key: bool(v)}))

    use_connect = _flag('use_connect')
    use_deform = _flag('use_deform')
    use_inherit_rotation = _flag('use_inherit_rotation')
    use_local_location = _flag('use_local_location')
    use_relative_parent = _flag('use_relative_parent')

    def _num(key, conv=float):
        return property(lambda self: self._j()[key], lambda self, v: self._set(**{key: conv(v)}))

    envelope_distance = _num('envelope_distance')
    envelope_weight = _num('envelope_weight')
    head_radius = _num('head_radius')
    tail_radius = _num('tail_radius')
    bbone_segments = _num('bbone_segments', int)
    del _flag, _num

    @property
    def inherit_scale(self):
        return self._j()['inherit_scale']

    @inherit_scale.setter
    def inherit_scale(self, v):
        self._set(inherit_scale=P.Raw(str(v).lower()))

    hide = False
    use_envelope_multiply = False

    @property
    def select(self):
        return self.get('_select', False)

    @select.setter
    def select(self, v):
        self['_select'] = bool(v)

    select_head = select
    select_tail = select


class Bone(_BoneBase):
    _seq = 'bones'

    @property
    def head(self):
        j = self._j()
        p = j['parent']
        if not p:
            return Vector(j['head'])
        pm = _m4(self._a._bone_j(p)['matrix_local'])
        return (pm.inverted() @ Vector(j['head']).to_4d()).xyz

    @property
    def tail(self):
        return self.head + Vector((0, self.length, 0))

    @property
    def head_local(self):
        return Vector(self._j()['head'])

    @property
    def tail_local(self):
        return Vector(self._j()['tail'])

    @property
    def matrix(self):
        j = self._j()
        m = _m4(j['matrix_local']).to_3x3()
        if j['parent']:
            pm = _m4(self._a._bone_j(j['parent'])['matrix_local']).to_3x3()
            return pm.inverted() @ m
        return m

    def convert_local_to_pose(self, matrix, matrix_local, parent_matrix=None, parent_matrix_local=None, invert=False):
        if parent_matrix is None:
            return matrix_local @ matrix
        return parent_matrix @ parent_matrix_local.inverted() @ matrix_local @ matrix


class EditBone(_BoneBase):
    _seq = 'edit_bones'

    @property
    def head(self):
        return self._v('head')

    @head.setter
    def head(self, v):
        self._set(head=[float(c) for c in v])

    @property
    def tail(self):
        return self._v('tail')

    @tail.setter
    def tail(self, v):
        self._set(tail=[float(c) for c in v])

    @property
    def roll(self):
        return self._j()['roll']

    @roll.setter
    def roll(self, v):
        self._set(roll=float(v))

    @_BoneBase.parent.setter
    def parent(self, p):
        self._set(parent=p.name if p is not None else P.Raw('none'))

    @_BoneBase.length.setter
    def length(self, n):
        j = self._j()
        d = (Vector(j['tail']) - Vector(j['head'])).normalized()
        self._set(tail=list(Vector(j['head']) + d * float(n)))

    @property
    def matrix(self):
        return self.matrix_local

    @matrix.setter
    def matrix(self, m):
        m = Matrix(m)
        vec, roll = mat3_to_vec_roll(m.to_3x3())
        head = m.to_translation()
        tail = head + vec.normalized() * self.length
        self._set(head=list(head), tail=list(tail), roll=roll)

    def align_roll(self, vector):
        j = self._j()
        y = (Vector(j['tail']) - Vector(j['head'])).normalized()
        z = Vector(vector)
        z = (z - y * z.dot(y)).normalized()
        x = y.cross(z)
        m = Matrix(((x[0], y[0], z[0]), (x[1], y[1], z[1]), (x[2], y[2], z[2])))
        self.roll = mat3_to_vec_roll(m)[1]

    def transform(self, matrix, scale=True, roll=True):
        j = self._j()
        m = Matrix(matrix)
        self._set(head=list(m @ Vector(j['head'])), tail=list(m @ Vector(j['tail'])))

    @property
    def use_inherit_scale(self):
        return self.inherit_scale != 'NONE'

    def select_set(self, s):
        self.select = s


class ArmatureBones(Collection_):
    def __init__(self, arm, cls):
        self._a = arm
        self._cls = cls

    def _items(self):
        return [self._cls(self._a, b['name']) for b in self._a._j()['bones']]

    @property
    def active(self):
        n = self._a.get('_active_bone')
        return self._cls(self._a, n) if n and n in self else None

    @active.setter
    def active(self, b):
        self._a['_active_bone'] = b.name if b is not None else None

    def new(self, name):
        if self._cls is not EditBone:
            raise TypeError('bones are added with edit_bones.new()')
        have = set(self.keys())
        base, n, i = name, name, 0
        while n in have:
            i += 1
            n = '%s.%03d' % (base, i)
        got = first(req('bone', self._a._name, n, head=[0, 0, 0], tail=[0, 0, 0]))
        return EditBone(self._a, got or n)

    def remove(self, bone):
        req('bone_remove', self._a._name, bone.name)


# --- pose -----------------------------------------------------------------------------

_POSE_DEFAULTS = {'location': [0.0, 0.0, 0.0], 'rotation_mode': 'quaternion', 'quaternion': [1.0, 0.0, 0.0, 0.0],
                  'euler': [0.0, 0.0, 0.0], 'axis_angle': [0.0, 0.0, 1.0, 0.0], 'scale': [1.0, 1.0, 1.0]}


def _rig_lines(obj_name):
    d = req('dump_object', obj_name)
    return d, list(P.read_commands(d.get('rig') or ''))


class Pose(bpy_struct):
    def __init__(self, obj):
        self._o = obj

    def _key(self):
        return ('pose', self._o)

    @property
    def bones(self):
        return PoseBones(self._o)

    def apply_pose_from_action(self, action, evaluation_time=0.0):
        pass

    use_auto_ik = False
    use_mirror_x = False


class PoseBones(Collection_):
    def __init__(self, obj):
        self._o = obj

    def _items(self):
        d = req('dump_object', self._o)
        arm = Armature(d['data'])
        return [PoseBone(self._o, arm, b['name']) for b in arm._j()['bones']]


class PoseBone(bpy_struct):
    def __init__(self, obj, arm, name):
        self._o = obj
        self._a = arm
        self.__dict__['_n'] = name

    def _key(self):
        return ('pose_bone', self._o, self._n)

    def __repr__(self):
        return "bpy.data.objects['%s'].pose.bones['%s']" % (self._o, self._n)

    def _opts(self):
        d, lines = _rig_lines(self._o)
        out = dict(_POSE_DEFAULTS)
        for cmd, pos, opts in lines:
            if cmd == 'pose_bone' and len(pos) >= 2 and pos[1] == self._n:
                for k, v in opts.items():
                    out[k] = v if k == 'rotation_mode' else parse_value(v)
        return d, out

    def _set(self, **opts):
        req('pose_bone', self._o, self._n, **opts)

    @property
    def name(self):
        return self._n

    @property
    def bone(self):
        return Bone(self._a, self._n)

    @property
    def parent(self):
        p = self._a._bone_j(self._n)['parent']
        return PoseBone(self._o, self._a, p) if p else None

    @property
    def children(self):
        return [PoseBone(self._o, self._a, c) for c in self._a._bone_j(self._n)['children']]

    @property
    def id_data(self):
        from .types import Object
        return Object(self._o)

    def _vec(self, key, cls=Vector):
        v = self._opts()[1][key]
        v = v if isinstance(v, list) else [v]

        def write(x):
            self._set(**{key: list(x)})
        return cls(v, _owner=write)

    @property
    def location(self):
        return self._vec('location')

    @location.setter
    def location(self, v):
        self._set(location=[float(c) for c in v])

    @property
    def scale(self):
        return self._vec('scale')

    @scale.setter
    def scale(self, v):
        self._set(scale=[float(c) for c in v])

    @property
    def rotation_quaternion(self):
        return self._vec('quaternion', Quaternion)

    @rotation_quaternion.setter
    def rotation_quaternion(self, q):
        self._set(quaternion=[float(c) for c in q])

    @property
    def rotation_euler(self):
        o = self._opts()[1]
        mode = o['rotation_mode'].upper()

        def write(e):
            self._set(euler=list(e))
        return Euler(o['euler'], mode if len(mode) == 3 else 'XYZ', _owner=write)

    @rotation_euler.setter
    def rotation_euler(self, e):
        self._set(euler=[float(c) for c in e])

    @property
    def rotation_axis_angle(self):
        v = self._opts()[1]['axis_angle']

        def write(x):
            self._set(axis_angle=list(x))
        from ._arrays import FloatArray
        return FloatArray(v, write)

    @rotation_axis_angle.setter
    def rotation_axis_angle(self, v):
        self._set(axis_angle=[float(c) for c in v])

    @property
    def rotation_mode(self):
        return self._opts()[1]['rotation_mode'].upper()

    @rotation_mode.setter
    def rotation_mode(self, m):
        self._set(rotation_mode=P.Raw(str(m).lower()))

    @property
    def matrix(self):
        d = req('dump_object', self._o)
        return _m4(d['pose_matrices'][self._n])

    @matrix.setter
    def matrix(self, m):
        # Pose-space matrix -> basis: undo the rest and the parent's pose.
        m = Matrix(m)
        rest = self.bone.matrix_local
        p = self.parent
        if p is not None:
            pr = p.bone.matrix_local
            basis = (rest.inverted() @ pr @ p.matrix.inverted() @ m)
        else:
            basis = rest.inverted() @ m
        self.matrix_basis = basis

    @property
    def matrix_basis(self):
        o = self._opts()[1]
        mode = o['rotation_mode'].upper()
        if mode == 'QUATERNION':
            r = Quaternion(o['quaternion']).to_matrix()
        elif mode == 'AXIS_ANGLE':
            a = o['axis_angle']
            r = Matrix.Rotation(a[0], 3, Vector(a[1:4]))
        else:
            r = Euler(o['euler'], mode).to_matrix()
        return Matrix.LocRotScale(Vector(o['location']), r, Vector(o['scale']))

    @matrix_basis.setter
    def matrix_basis(self, m):
        loc, rot, sc = Matrix(m).decompose()
        mode = self.rotation_mode
        opts = {'location': list(loc), 'scale': list(sc)}
        if mode == 'QUATERNION':
            opts['quaternion'] = list(rot)
        elif mode == 'AXIS_ANGLE':
            ax, an = rot.to_axis_angle()
            opts['axis_angle'] = [an, ax[0], ax[1], ax[2]]
        else:
            opts['euler'] = list(rot.to_euler(mode))
        self._set(**opts)

    @property
    def head(self):
        return self.matrix.to_translation()

    @property
    def tail(self):
        m = self.matrix
        return m.to_translation() + m.col[1].xyz * self.bone.length

    @property
    def length(self):
        return self.bone.length

    @property
    def constraints(self):
        return Constraints(self._o, self._n)

    # IK settings
    def _ik(key, i):
        def get(self):
            v = self._opts()[1].get(key)
            if v is None:
                return {'ik_lock': 0, 'ik_limit': 0, 'ik_stiffness': 0.0, 'ik_min': -math.pi, 'ik_max': math.pi}[key]
            return v[i] if isinstance(v, list) else v

        def put(self, x):
            o = self._opts()[1]
            dflt = {'ik_lock': [0, 0, 0], 'ik_limit': [0, 0, 0], 'ik_stiffness': [0.0] * 3, 'ik_min': [-math.pi] * 3, 'ik_max': [math.pi] * 3}[key]
            cur = o.get(key, dflt)
            cur = list(cur) if isinstance(cur, list) else dflt
            cur[i] = float(x) if key in ('ik_stiffness', 'ik_min', 'ik_max') else int(bool(x))
            self._set(**{key: cur})
        return property(get, put)

    lock_ik_x, lock_ik_y, lock_ik_z = _ik('ik_lock', 0), _ik('ik_lock', 1), _ik('ik_lock', 2)
    use_ik_limit_x, use_ik_limit_y, use_ik_limit_z = _ik('ik_limit', 0), _ik('ik_limit', 1), _ik('ik_limit', 2)
    ik_stiffness_x, ik_stiffness_y, ik_stiffness_z = _ik('ik_stiffness', 0), _ik('ik_stiffness', 1), _ik('ik_stiffness', 2)
    ik_min_x, ik_min_y, ik_min_z = _ik('ik_min', 0), _ik('ik_min', 1), _ik('ik_min', 2)
    ik_max_x, ik_max_y, ik_max_z = _ik('ik_max', 0), _ik('ik_max', 1), _ik('ik_max', 2)
    del _ik

    @property
    def ik_stretch(self):
        return self._opts()[1].get('ik_stretch', 0.0)

    @ik_stretch.setter
    def ik_stretch(self, v):
        self._set(ik_stretch=float(v))

    custom_shape = None
    lock_location = (False, False, False)
    lock_rotation = (False, False, False)
    lock_rotation_w = False
    lock_scale = (False, False, False)

    def keyframe_insert(self, data_path, index=-1, frame=None, group='', options=set(), keytype='KEYFRAME'):
        from .anim import keyframe_insert
        from .types import Object
        return keyframe_insert(Object(self._o), 'pose.bones["%s"].%s' % (self._n, data_path), index, frame, group or self._n)

    def keyframe_delete(self, data_path, index=-1, frame=None, group=''):
        from .anim import keyframe_delete
        from .types import Object
        return keyframe_delete(Object(self._o), 'pose.bones["%s"].%s' % (self._n, data_path), index, frame)

    @property
    def bone_group(self):
        return None

    def evaluate_envelope(self, point):
        return 0.0


# --- constraints ---------------------------------------------------------------------------

_TEXT_KEYS = {'name', 'target', 'subtarget', 'pole_target', 'pole_subtarget', 'space_object', 'space_subtarget', 'bone', 'vertex_group'}
_OBJECT_KEYS = {'target', 'pole_target', 'space_object'}


class Constraint(bpy_struct):
    def __init__(self, obj, bone, index, ctype, opts):
        d = self.__dict__
        d['_o'], d['_b'], d['_i'], d['_t'], d['_opts'] = obj, bone, index, ctype, opts

    def _key(self):
        return ('constraint', self._o, self._b, self._i)

    def __repr__(self):
        return "<Constraint %s '%s'>" % (self.type, self.name)

    @property
    def type(self):
        return self._t.upper()

    @property
    def name(self):
        return self._opts.get('name') or self._t.replace('_', ' ').title()

    @property
    def is_valid(self):
        return True

    @property
    def targets(self):
        return _ArmatureTargets(self)

    def __getattr__(self, key):
        if key.startswith('_'):
            raise AttributeError(key)
        opts = self.__dict__['_opts']
        if key not in opts:
            if key in ('influence',):
                return 1.0
            if key in ('mute', 'is_override_data_editable', 'active', 'show_expanded'):
                return False
            if key in _OBJECT_KEYS:
                return None
            if key in _TEXT_KEYS:
                return ''
            raise AttributeError("'%s' constraint has no attribute '%s'" % (self.type, key))
        v = opts[key]
        if key in _OBJECT_KEYS:
            from .types import Object
            return Object(v) if v and v != 'none' else None
        if key in _TEXT_KEYS:
            return v
        p = parse_value(v)
        if isinstance(p, str):
            return p.upper()
        if isinstance(p, list) and key == 'inverse_matrix':
            return _m4(p)
        if key.startswith('use_') or key.startswith('invert') or key in ('mute',):
            if isinstance(p, list):
                return [bool(x) for x in p]
            return bool(p)
        return p

    def __setattr__(self, key, value):
        if key.startswith('_'):
            self.__dict__[key] = value
            return
        if key in _OBJECT_KEYS or hasattr(value, 'name') and not isinstance(value, str):
            value = value.name if value is not None else P.Raw('none')
        elif isinstance(value, str) and key not in _TEXT_KEYS:
            value = P.Raw(value.lower())
        elif hasattr(value, 'col') and hasattr(value, 'row'):
            value = _cols(value)
        req('constraint_set', self._o, self._i, bone=self._b, **{key: value})
        self.__dict__['_opts'][key] = P.token(value) if not isinstance(value, str) else value


class _ArmatureTargets(Collection_):
    def __init__(self, con):
        self._c = con

    def new(self):
        return _ArmTarget(self._c)


class _ArmTarget:
    def __init__(self, con):
        self._c = con
        self._target = None
        self._sub = ''
        self.weight = 1.0

    @property
    def target(self):
        return self._target

    @target.setter
    def target(self, o):
        self._target = o
        self._push()

    @property
    def subtarget(self):
        return self._sub

    @subtarget.setter
    def subtarget(self, s):
        self._sub = s
        self._push()

    def _push(self):
        if self._target is not None:
            req('constraint_target', self._c._o, self._c._i, self._target.name, subtarget=self._sub, weight=self.weight, bone=self._c._b)


class Constraints(Collection_):
    def __init__(self, obj, bone=None):
        self._o = obj
        self._b = bone

    def _items(self):
        d, lines = _rig_lines(self._o)
        out = []
        for cmd, pos, opts in lines:
            if cmd != 'constraint_add':
                continue
            b = opts.pop('bone', None)
            if b != self._b:
                continue
            out.append(Constraint(self._o, self._b, len(out), pos[1], opts))
        return out

    def new(self, type):
        i = first(req('constraint_add', self._o, P.Raw(str(type).lower()), bone=self._b))
        return self._items()[int(i)]

    def remove(self, con):
        req('constraint_remove', self._o, con._i, bone=self._b)

    def clear(self):
        for c in reversed(self._items()):
            self.remove(c)

    @property
    def active(self):
        items = self._items()
        return items[-1] if items else None


# --- vertex groups ------------------------------------------------------------------------

class VertexGroup(bpy_struct):
    def __init__(self, obj, mesh, name, index):
        self._o = obj
        self._m = mesh
        self.__dict__['_n'] = name
        self.index = index

    def _key(self):
        return ('vgroup', self._m, self._n)

    @property
    def name(self):
        return self._n

    lock_weight = False

    def _mesh(self):
        from .mesh import Mesh
        return Mesh(self._m)

    def add(self, index, weight, type):
        m = self._mesh()
        m._flush_now()
        idx = [int(i) for i in index]
        if not idx:
            return
        if type == 'REPLACE':
            ws = [float(weight)] * len(idx)
        else:
            d = m._d()
            gi = d.groups.index(self._n)
            cur = [d.weights.get(i, {}).get(gi, 0.0) for i in idx]
            sign = 1.0 if type == 'ADD' else -1.0
            ws = [min(1.0, max(0.0, c + sign * float(weight))) for c in cur]
        req('vgroup', self._m, self._n, indices=idx, weights=ws)

    def remove(self, index):
        m = self._mesh()
        m._flush_now()
        req('vgroup', self._m, self._n, remove=[int(i) for i in index])

    def weight(self, index):
        d = self._mesh()._d()
        gi = d.groups.index(self._n)
        w = d.weights.get(index, {})
        if gi not in w:
            raise RuntimeError('Error: Vertex not in group')
        return w[gi]


class VertexGroups(Collection_):
    def __init__(self, obj):
        self._o = obj

    def _mesh_name(self):
        d = req('dump_object', self._o)
        if d['type'] != 'MESH':
            return None
        return d['data']

    def _items(self):
        mn = self._mesh_name()
        if mn is None:
            return []
        from .mesh import Mesh
        groups = Mesh(mn)._d().groups
        return [VertexGroup(self._o, mn, g, i) for i, g in enumerate(groups)]

    def new(self, name='Group'):
        mn = self._mesh_name()
        if mn is None:
            raise TypeError('only mesh objects have vertex groups here')
        from .mesh import Mesh
        Mesh(mn)._flush_now()
        have = set(self.keys())
        n, i = name, 0
        while n in have:
            i += 1
            n = '%s.%03d' % (name, i)
        req('vgroup', mn, n)
        return VertexGroup(self._o, mn, n, len(have))

    def remove(self, group):
        req('vgroup_remove', group._m, group.name)

    def clear(self):
        for g in self._items():
            self.remove(g)

    @property
    def active(self):
        items = self._items()
        i = self.active_index
        return items[i] if 0 <= i < len(items) else None

    @property
    def active_index(self):
        from .types import Object
        n = len(self._items())
        return min(Object(self._o).get('_active_vgroup', n - 1), n - 1)

    @active_index.setter
    def active_index(self, i):
        from .types import Object
        Object(self._o)['_active_vgroup'] = int(i)


# --- lattices -----------------------------------------------------------------------------

class LatticePoint(bpy_struct):
    def __init__(self, lat, i):
        self._l = lat
        self.index = i

    @property
    def co(self):
        f = self._l._f('rest')
        return Vector(f[self.index * 3:self.index * 3 + 3])

    @property
    def co_deform(self):
        f = self._l._f('points')
        i = self.index

        def write(v):
            pts = self._l._f('points')
            pts[i * 3:i * 3 + 3] = [v[0], v[1], v[2]]
            req('lattice_set', self._l.name, verts=P.b64_floats(pts))
        return Vector(f[i * 3:i * 3 + 3], _owner=write)

    @co_deform.setter
    def co_deform(self, v):
        pts = self._l._f('points')
        pts[self.index * 3:self.index * 3 + 3] = [float(c) for c in v]
        req('lattice_set', self._l.name, verts=P.b64_floats(pts))

    @property
    def groups(self):
        return []

    select = True


class Lattice(ID):
    _kind = 'lattice'
    _collection_name = 'lattices'

    def _j(self):
        return req('dump_lattice', self._name)

    def _f(self, key):
        return P.floats('b64:' + self._j()[key])

    @property
    def points(self):
        n = self.points_u * self.points_v * self.points_w
        return [LatticePoint(self, i) for i in range(n)]

    def _pts(i):
        key = ('points_u', 'points_v', 'points_w')[i]

        def put(self, v):
            j = self._j()
            p = [j['points_u'], j['points_v'], j['points_w']]
            p[i] = int(v)
            req('lattice_set', self._name, points=p)
        return property(lambda self: self._j()[key], put)

    points_u, points_v, points_w = _pts(0), _pts(1), _pts(2)

    def _interp(i):
        def put(self, v):
            cur = [s.replace('KEY_', '').lower() for s in self._j()['interpolation']]
            cur[i] = str(v).replace('KEY_', '').lower()
            req('lattice_set', self._name, interpolation=P.Raw(','.join(cur)))
        return property(lambda self: self._j()['interpolation'][i], put)

    interpolation_type_u, interpolation_type_v, interpolation_type_w = _interp(0), _interp(1), _interp(2)
    del _pts, _interp

    @property
    def use_outside(self):
        return self._j()['use_outside']

    @use_outside.setter
    def use_outside(self, v):
        req('lattice_set', self._name, use_outside=bool(v))

    @property
    def shape_keys(self):
        return None
