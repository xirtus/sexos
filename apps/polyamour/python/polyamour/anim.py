# SPDX-License-Identifier: GPL-3.0-only
"""Actions, F-Curves and keyframes (bpy.types.Action, FCurve, Keyframe) and
the keyframe_insert/keyframe_delete behind objects and pose bones."""
import re

from mathutils import Vector

from . import polyops as P
from ._base import ID, Collection_, bpy_struct, first, req


def _dump(name):
    return req('dump_action', name)


class Keyframe(bpy_struct):
    def __init__(self, fc, k):
        self._fc = fc
        self._k = k

    def _key(self):
        return ('key', self._fc._a, self._fc.data_path, self._fc.array_index, self._k)

    def _j(self):
        return self._fc._j()['keys'][self._k]

    def _set(self, **opts):
        req('key_set', self._fc._a, self._fc.data_path, self._fc.array_index, self._k, **opts)

    def _vec(self, key):
        def write(v):
            self._set(**{key: [v[0], v[1]]})
        return Vector(self._j()[key], _owner=write)

    @property
    def co(self):
        return self._vec('co')

    @co.setter
    def co(self, v):
        self._set(co=[float(v[0]), float(v[1])])

    @property
    def co_ui(self):
        return self.co

    @co_ui.setter
    def co_ui(self, v):
        self.co = v

    @property
    def handle_left(self):
        return self._vec('handle_left')

    @handle_left.setter
    def handle_left(self, v):
        self._set(handle_left=[float(v[0]), float(v[1])])

    @property
    def handle_right(self):
        return self._vec('handle_right')

    @handle_right.setter
    def handle_right(self, v):
        self._set(handle_right=[float(v[0]), float(v[1])])

    @property
    def interpolation(self):
        return self._j()['interpolation']

    @interpolation.setter
    def interpolation(self, v):
        self._set(interpolation=P.Raw(str(v)))

    @property
    def handle_left_type(self):
        return self._j()['handle_type']

    @handle_left_type.setter
    def handle_left_type(self, v):
        self._set(handle_type=P.Raw(str(v)))

    handle_right_type = handle_left_type

    @property
    def select_control_point(self):
        return self._j()['select']

    @select_control_point.setter
    def select_control_point(self, v):
        self._set(select=bool(v))

    select_left_handle = False
    select_right_handle = False
    easing = 'AUTO'
    type = 'KEYFRAME'
    back = 1.70158
    amplitude = 0.8
    period = 4.1


class KeyframePoints(Collection_):
    def __init__(self, fc):
        self._fc = fc

    def _items(self):
        return [Keyframe(self._fc, i) for i in range(len(self._fc._j()['keys']))]

    def _name_of(self, item):
        return None

    def insert(self, frame, value, options=set(), keyframe_type='KEYFRAME'):
        r = req('key_insert', self._fc._a, self._fc.data_path, self._fc.array_index, float(frame), float(value))
        return Keyframe(self._fc, int(first(r, 0)))

    def add(self, count=1):
        have = len(self._fc._j()['keys'])
        last = max([k['co'][0] for k in self._fc._j()['keys']] or [-1.0])
        for i in range(int(count)):
            req('key_insert', self._fc._a, self._fc.data_path, self._fc.array_index, last + 1 + i, 0.0, interpolation=P.Raw('BEZIER'))
        return None if count != 1 else Keyframe(self._fc, have)

    def remove(self, keyframe, fast=False):
        req('key_remove', self._fc._a, self._fc.data_path, self._fc.array_index, keyframe._k)

    def clear(self):
        for i in reversed(range(len(self))):
            req('key_remove', self._fc._a, self._fc.data_path, self._fc.array_index, i)

    def foreach_set(self, attr, seq):
        if attr == 'co':
            for i in range(len(self)):
                Keyframe(self._fc, i).co = (seq[2 * i], seq[2 * i + 1])
        else:
            Collection_.foreach_set(self, attr, seq)


class ActionGroup(bpy_struct):
    def __init__(self, name):
        self.name = name

    def _key(self):
        return ('group', self.name)

    @property
    def channels(self):
        return []

    show_expanded = True
    select = False
    lock = False
    mute = False


def _bezier(k0, k1, x):
    """Value of the Bezier segment k0..k1 at frame x (handles in frames)."""
    p0, p1, p2, p3 = k0['co'], k0['handle_right'], k1['handle_left'], k1['co']
    lo, hi = 0.0, 1.0
    t = 0.5
    for _ in range(40):
        t = (lo + hi) / 2
        u = 1 - t
        bx = u * u * u * p0[0] + 3 * u * u * t * p1[0] + 3 * u * t * t * p2[0] + t * t * t * p3[0]
        if bx < x:
            lo = t
        else:
            hi = t
    u = 1 - t
    return u * u * u * p0[1] + 3 * u * u * t * p1[1] + 3 * u * t * t * p2[1] + t * t * t * p3[1]


class FCurve(bpy_struct):
    def __init__(self, action, data_path, index):
        self._a = action
        self.__dict__['data_path'] = data_path
        self.__dict__['array_index'] = index

    def _key(self):
        return ('fcurve', self._a, self.data_path, self.array_index)

    def _j(self):
        for c in _dump(self._a)['fcurves']:
            if c['data_path'] == self.data_path and c['array_index'] == self.array_index:
                return c
        raise ReferenceError('the F-Curve %s[%d] was removed' % (self.data_path, self.array_index))

    def __repr__(self):
        return "bpy.data.actions['%s'].fcurves.find('%s', index=%d)" % (self._a, self.data_path, self.array_index)

    @property
    def keyframe_points(self):
        return KeyframePoints(self)

    @property
    def group(self):
        g = self._j()['group']
        return ActionGroup(g) if g else None

    @property
    def mute(self):
        return self._j()['mute']

    @mute.setter
    def mute(self, v):
        pass

    lock = False
    hide = False
    select = True
    is_valid = True
    extrapolation = 'CONSTANT'
    color_mode = 'AUTO_RAINBOW'
    auto_smoothing = 'NONE'

    @property
    def modifiers(self):
        return []

    @property
    def sampled_points(self):
        return []

    def update(self):
        pass

    def range(self):
        keys = self._j()['keys']
        if not keys:
            return Vector((0.0, 0.0))
        return Vector((keys[0]['co'][0], keys[-1]['co'][0]))

    def evaluate(self, frame):
        keys = self._j()['keys']
        if not keys:
            return 0.0
        if frame <= keys[0]['co'][0]:
            return keys[0]['co'][1]
        if frame >= keys[-1]['co'][0]:
            return keys[-1]['co'][1]
        for a, b in zip(keys, keys[1:]):
            if a['co'][0] <= frame <= b['co'][0]:
                if a['interpolation'] == 'CONSTANT':
                    return a['co'][1]
                if a['interpolation'] == 'LINEAR' or b['co'][0] == a['co'][0]:
                    t = (frame - a['co'][0]) / (b['co'][0] - a['co'][0]) if b['co'][0] != a['co'][0] else 0.0
                    return a['co'][1] + (b['co'][1] - a['co'][1]) * t
                return _bezier(a, b, frame)
        return keys[-1]['co'][1]


class ActionFCurves(Collection_):
    def __init__(self, action):
        self._a = action

    def _items(self):
        return [FCurve(self._a, c['data_path'], c['array_index']) for c in _dump(self._a)['fcurves']]

    def _name_of(self, item):
        return item.data_path

    def new(self, data_path, index=0, action_group=''):
        req('fcurve_new', self._a, data_path, int(index), group=action_group or None)
        return FCurve(self._a, data_path, int(index))

    def find(self, data_path, index=0):
        for c in _dump(self._a)['fcurves']:
            if c['data_path'] == data_path and c['array_index'] == index:
                return FCurve(self._a, data_path, index)
        return None

    def remove(self, fcurve):
        req('fcurve_remove', self._a, fcurve.data_path, fcurve.array_index)

    def clear(self):
        for c in self._items():
            self.remove(c)


class Action(ID):
    _kind = 'action'
    _collection_name = 'actions'

    @property
    def fcurves(self):
        return ActionFCurves(self._name)

    @property
    def groups(self):
        names = []
        for c in _dump(self._name)['fcurves']:
            if c['group'] and c['group'] not in names:
                names.append(c['group'])
        return [ActionGroup(n) for n in names]

    @property
    def frame_range(self):
        r = _dump(self._name)['frame_range']
        return Vector(r if r else (1.0, 1.0))

    @property
    def users(self):
        return _dump(self._name)['users']

    id_root = 'OBJECT'
    use_frame_range = False

    @property
    def pose_markers(self):
        return []

    def copy(self):
        new = first(req('action_new', self._name))
        for c in _dump(self._name)['fcurves']:
            req('fcurve_new', new, c['data_path'], c['array_index'], group=c['group'] or None)
            for k in c['keys']:
                req('key_insert', new, c['data_path'], c['array_index'], k['co'][0], k['co'][1], interpolation=P.Raw(k['interpolation']))
        return Action(new)


class AnimData(bpy_struct):
    def __init__(self, obj):
        self._o = obj

    def _key(self):
        return ('animdata', self._o)

    @property
    def action(self):
        a = req('dump_object', self._o).get('action')
        return Action(a) if a else None

    @action.setter
    def action(self, act):
        req('action_assign', self._o, act.name if act is not None else P.Raw('none'))

    @property
    def drivers(self):
        return Collection_()

    @property
    def nla_tracks(self):
        return Collection_()

    action_blend_type = 'REPLACE'
    action_influence = 1.0
    use_nla = True


# --- keyframe_insert -----------------------------------------------------------------

_POSE_PATH = re.compile(r'^pose\.bones\["(.+)"\]\.(\w+)$')

_DEFAULT_GROUPS = {'location': 'Object Transforms', 'rotation_euler': 'Object Transforms', 'rotation_quaternion': 'Object Transforms',
                   'scale': 'Object Transforms', 'rotation_axis_angle': 'Object Transforms'}


def path_values(obj, data_path):
    """Current values (a list) of `data_path` on object `obj`."""
    m = _POSE_PATH.match(data_path)
    target = obj
    prop = data_path
    if m:
        target = obj.pose.bones[m.group(1)]
        prop = m.group(2)
    v = target
    for part in prop.split('.'):
        v = getattr(v, part)
    if hasattr(v, '__len__') and not isinstance(v, str):
        return [float(x) for x in v]
    return [float(v)]


def keyframe_insert(obj, data_path, index=-1, frame=None, group='', interpolation=None):
    from . import context
    if frame is None:
        frame = context.scene.frame_current
    vals = path_values(obj, data_path)
    idx = range(len(vals)) if index is None or index < 0 else [index]
    m = _POSE_PATH.match(data_path)
    g = group or (m.group(1) if m else _DEFAULT_GROUPS.get(data_path, ''))
    for i in idx:
        req('keyframe', obj.name, data_path, i, float(frame), vals[i], group=g or None,
            interpolation=P.Raw(interpolation) if interpolation else None)
    return True


def keyframe_delete(obj, data_path, index=-1, frame=None):
    from . import context
    if frame is None:
        frame = context.scene.frame_current
    n = first(req('keyframe_delete', obj.name, data_path, -1 if index is None else int(index), float(frame)), '0')
    if int(n) == 0:
        raise RuntimeError('keyframe_delete: no keyframe on "%s" at frame %g' % (data_path, frame))
    return True
