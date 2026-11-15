# SPDX-License-Identifier: GPL-3.0-only
"""Materials, node trees, nodes, sockets and links, the world, and images
(bpy.types.Material, NodeTree, Node, NodeSocket, NodeLink, World, Image)."""
import re

from mathutils import Vector

from . import polyops as P
from ._arrays import FloatArray
from ._base import ID, Collection_, bpy_struct, first, parse_value, req


# --- node trees ---------------------------------------------------------------------------

_SOCKET_TYPES = {
    'float': ('VALUE', 'NodeSocketFloat'), 'int': ('INT', 'NodeSocketInt'), 'bool': ('BOOLEAN', 'NodeSocketBool'),
    'vector': ('VECTOR', 'NodeSocketVector'), 'color': ('RGBA', 'NodeSocketColor'), 'string': ('STRING', 'NodeSocketString'),
    'shader': ('SHADER', 'NodeSocketShader'), 'geometry': ('GEOMETRY', 'NodeSocketGeometry'), 'object': ('OBJECT', 'NodeSocketObject'),
    'collection': ('COLLECTION', 'NodeSocketCollection'), 'image': ('IMAGE', 'NodeSocketImage'), 'material': ('MATERIAL', 'NodeSocketMaterial'),
    'texture': ('TEXTURE', 'NodeSocketTexture'), 'rotation': ('ROTATION', 'NodeSocketRotation'), 'matrix': ('MATRIX', 'NodeSocketMatrix'),
    'menu': ('MENU', 'NodeSocketMenu'), 'virtual': ('CUSTOM', 'NodeSocketVirtual'),
}
_IDNAME_TO_TYPE = {v[1]: k for k, v in _SOCKET_TYPES.items()}
_IDNAME_TO_TYPE.update({'NodeSocketFloatFactor': 'float', 'NodeSocketFloatAngle': 'float', 'NodeSocketFloatDistance': 'float',
                        'NodeSocketFloatUnsigned': 'float', 'NodeSocketVectorTranslation': 'vector', 'NodeSocketVectorDirection': 'vector',
                        'NodeSocketVectorXYZ': 'vector', 'NodeSocketVectorEuler': 'vector', 'NodeSocketIntUnsigned': 'int'})
_ENUM_TO_TYPE = {v[0]: k for k, v in _SOCKET_TYPES.items()}


def _owner_args(owner):
    return tuple(owner)


def _id_value(v):
    """Python value -> polyops value text for node_set."""
    if v is None:
        return P.Raw('none')
    if isinstance(v, ID):
        return v.name
    if isinstance(v, bool):
        return v
    if isinstance(v, (int, float)):
        return float(v)
    if isinstance(v, str):
        return v
    return [float(x) for x in v]


class NodeTree(ID):
    """A material's, the world's or a group's node tree."""

    _kind = 'node_group'
    _collection_name = 'node_groups'

    def __init__(self, name, owner=None):
        ID.__init__(self, name)
        self.__dict__['_owner'] = tuple(owner) if owner else ('group', name)

    def _key(self):
        return ('tree',) + self._owner

    def __repr__(self):
        if self._owner[0] == 'group':
            return "bpy.data.node_groups['%s']" % self._name
        if self._owner[0] == 'material':
            return "bpy.data.materials['%s'].node_tree" % self._owner[1]
        return 'bpy.data.worlds[0].node_tree'

    def _j(self):
        j = req('dump_tree', *self._owner)
        if j is None:
            raise ReferenceError('the node tree is gone (use_nodes is off)')
        return j

    def _node_j(self, name):
        for n in self._j()['nodes']:
            if n['name'] == name:
                return n
        raise ReferenceError('node "%s" was removed' % name)

    def _rename(self, new):
        if self._owner[0] != 'group':
            return first(req('node_tree', *self._owner, name=new))
        got = first(req('rename', 'node_group', self._name, new))
        self.__dict__['_owner'] = ('group', got or new)
        return got

    @property
    def nodes(self):
        return Nodes(self)

    @property
    def links(self):
        return NodeLinks(self)

    @property
    def bl_idname(self):
        kind = self._j().get('kind', 'Shader')
        return {'Shader': 'ShaderNodeTree', 'Geometry': 'GeometryNodeTree', 'Compositor': 'CompositorNodeTree'}.get(kind, kind + 'NodeTree')

    @property
    def type(self):
        return self.bl_idname.replace('NodeTree', '').upper()

    @property
    def interface(self):
        return NodeTreeInterface(self)

    @property
    def inputs(self):
        return _LegacySockets(self, 'input')

    @property
    def outputs(self):
        return _LegacySockets(self, 'output')

    @property
    def users(self):
        return 1

    @property
    def animation_data(self):
        return None

    def copy(self):
        raise NotImplementedError('copying a node tree is not supported yet')


class _InterfaceSocket(bpy_struct):
    def __init__(self, tree, j, in_out, i):
        self._t = tree
        self._j = j
        self.in_out = in_out.upper()
        self.index = i

    @property
    def name(self):
        return self._j['name']

    @property
    def socket_type(self):
        return _SOCKET_TYPES.get(self._j['type'], ('', 'NodeSocketFloat'))[1]

    bl_socket_idname = socket_type

    @property
    def default_value(self):
        return parse_value(self._j.get('default'))

    item_type = 'SOCKET'
    min_value = -1e30
    max_value = 1e30
    description = ''


class NodeTreeInterface(bpy_struct):
    def __init__(self, tree):
        self._t = tree

    @property
    def items_tree(self):
        j = self._t._j()
        out = [_InterfaceSocket(self._t, s, 'input', i) for i, s in enumerate(j['inputs'])]
        out += [_InterfaceSocket(self._t, s, 'output', i) for i, s in enumerate(j['outputs'])]
        return out

    def new_socket(self, name, description='', in_out='INPUT', socket_type='NodeSocketFloat', parent=None):
        if self._t._owner[0] != 'group':
            raise TypeError('only node groups have an interface')
        ty = _IDNAME_TO_TYPE.get(socket_type, 'float')
        req('node_interface', 'group', self._t._name, P.Raw('input' if in_out == 'INPUT' else 'output'), name, P.Raw(ty))
        j = self._t._j()
        lst = j['inputs'] if in_out == 'INPUT' else j['outputs']
        return _InterfaceSocket(self._t, lst[-1], in_out, len(lst) - 1)

    def clear(self):
        raise NotImplementedError('removing group sockets is not supported yet')


class _LegacySockets(Collection_):
    """NodeTree.inputs / .outputs of the 3.x API."""

    def __init__(self, tree, in_out):
        self._t = tree
        self._io = in_out

    def _items(self):
        j = self._t._j()
        lst = j['inputs'] if self._io == 'input' else j['outputs']
        return [_InterfaceSocket(self._t, s, self._io, i) for i, s in enumerate(lst)]

    def new(self, type, name):
        return NodeTreeInterface(self._t).new_socket(name, in_out=self._io.upper(), socket_type=type)


def _node_type(idname):
    s = re.sub(r'^(ShaderNode|GeometryNode|CompositorNode|FunctionNode|TextureNode|NodeGroup|Node)', '', idname)
    if idname in ('ShaderNodeGroup', 'GeometryNodeGroup', 'CompositorNodeGroup'):
        return 'GROUP'
    if not s:
        return idname.upper()
    return re.sub(r'([a-z0-9])([A-Z])', r'\1_\2', s).upper()


class NodeSocket(bpy_struct):
    def __init__(self, tree, node, is_output, index):
        self._t = tree
        self._n = node
        self.is_output = is_output
        self._i = index

    def _key(self):
        return ('socket', self._t._owner, self._n, self.is_output, self._i)

    def _j(self):
        n = self._t._node_j(self._n)
        return (n['outputs'] if self.is_output else n['inputs'])[self._i]

    def __repr__(self):
        return "bpy.data...nodes['%s'].%s[%d]" % (self._n, 'outputs' if self.is_output else 'inputs', self._i)

    @property
    def node(self):
        return Node(self._t, self._n)

    @property
    def name(self):
        return self._j()['name']

    @property
    def identifier(self):
        return self._j()['identifier']

    @property
    def type(self):
        return _SOCKET_TYPES.get(self._j()['type'], ('VALUE',))[0]

    @property
    def bl_idname(self):
        return _SOCKET_TYPES.get(self._j()['type'], ('', 'NodeSocketFloat'))[1]

    @property
    def is_linked(self):
        return self._j()['is_linked']

    @property
    def enabled(self):
        return self._j()['enabled']

    @property
    def hide(self):
        return self._j()['hide']

    @hide.setter
    def hide(self, v):
        req('node_set', *self._t._owner, self._n, P.Raw('hide:%s:%s' % ('out' if self.is_output else 'in', P.quote(self.identifier))), bool(v))

    @property
    def hide_value(self):
        return self._j()['hide_value']

    @property
    def is_multi_input(self):
        return False

    @property
    def links(self):
        out = []
        for l in NodeLinks(self._t):
            if (self.is_output and l.from_node.name == self._n and l.from_socket._i == self._i) or \
               (not self.is_output and l.to_node.name == self._n and l.to_socket._i == self._i):
                out.append(l)
        return out

    @property
    def default_value(self):
        j = self._j()
        v = j['value']
        ty = j['type']
        if v is None:
            return None
        if ty in ('float',):
            return float(v)
        if ty in ('int',):
            return int(float(v))
        if ty == 'bool':
            return v in ('1', 'true')
        if ty in ('vector', 'color', 'rotation'):
            vals = [float(x) for x in v.split(',')]

            def write(new):
                self.default_value = new
            return FloatArray(vals, write)
        if ty == 'string':
            return [t[1] for t in P.tokenize(v)][0] if v else ''
        if ty in ('object', 'image', 'material', 'collection'):
            name = [t[1] for t in P.tokenize(v)][0] if v and v != 'none' else None
            if name is None:
                return None
            from . import data
            return getattr(data, {'object': 'objects', 'image': 'images', 'material': 'materials', 'collection': 'collections'}[ty]).get(name)
        return v

    @default_value.setter
    def default_value(self, v):
        target = P.Raw('%s:%s' % ('out' if self.is_output else 'in', P.quote(self.identifier)))
        req('node_set', *self._t._owner, self._n, target, _id_value(v))
        # A material's own colour and numbers follow its Principled BSDF, as
        # Blender's viewport colour does for the material preview.
        flat = _PRINCIPLED_FLAT.get(self.identifier)
        if flat and not self.is_output and self._t._owner[0] == 'material':
            if self._t._node_j(self._n).get('bl_idname') == 'ShaderNodeBsdfPrincipled':
                if flat in ('base_color', 'emission'):
                    vals = [float(x) for x in v][:4 if flat == 'base_color' else 3]
                else:
                    vals = float(v)
                req('material_set', self._t._owner[1], **{flat: vals})

    @property
    def min_value(self):
        return self._j()['min']

    @property
    def max_value(self):
        return self._j()['max']

    def is_linked_to(self, other):
        return any(l.to_socket == other for l in self.links)


_PRINCIPLED_FLAT = {
    'Base Color': 'base_color', 'Metallic': 'metallic', 'Roughness': 'roughness', 'IOR': 'ior',
    'Transmission Weight': 'transmission', 'Emission Color': 'emission', 'Emission Strength': 'emission_strength',
}


class NodeSockets(Collection_):
    def __init__(self, tree, node, is_output):
        self._t = tree
        self._n = node
        self._out = is_output

    def _items(self):
        n = self._t._node_j(self._n)
        lst = n['outputs'] if self._out else n['inputs']
        return [NodeSocket(self._t, self._n, self._out, i) for i in range(len(lst))]

    def __getitem__(self, key):
        if isinstance(key, (int, slice)):
            return self._items()[key]
        n = self._t._node_j(self._n)
        lst = n['outputs'] if self._out else n['inputs']
        for i, s in enumerate(lst):
            if s['name'] == key:
                return NodeSocket(self._t, self._n, self._out, i)
        for i, s in enumerate(lst):
            if s['identifier'] == key:
                return NodeSocket(self._t, self._n, self._out, i)
        raise KeyError('bpy_prop_collection[key]: key "%s" not found' % key)

    def new(self, type, name, identifier=''):
        ty = _IDNAME_TO_TYPE.get(type, 'float')
        req('node_socket', *self._t._owner, self._n, P.Raw('out' if self._out else 'in'), identifier or name, P.Raw(ty), name=name)
        return self[-1]


class ColorRampElement(bpy_struct):
    def __init__(self, ramp, i):
        self._r = ramp
        self._i = i

    @property
    def position(self):
        return self._r._parse()[2][self._i][0]

    @position.setter
    def position(self, v):
        interp, mode, els = self._r._parse()
        els[self._i][0] = float(v)
        self._r._write(interp, mode, els)

    @property
    def color(self):
        def write(c):
            interp, mode, els = self._r._parse()
            els[self._i][1:5] = list(c)
            self._r._write(interp, mode, els)
        return FloatArray(self._r._parse()[2][self._i][1:5], write)

    @color.setter
    def color(self, c):
        interp, mode, els = self._r._parse()
        els[self._i][1:5] = [float(x) for x in c]
        self._r._write(interp, mode, els)

    @property
    def alpha(self):
        return self.color[3]


class ColorRampElements(Collection_):
    def __init__(self, ramp):
        self._r = ramp

    def _items(self):
        return [ColorRampElement(self._r, i) for i in range(len(self._r._parse()[2]))]

    def _name_of(self, item):
        return None

    def new(self, position):
        interp, mode, els = self._r._parse()
        c = self._r.evaluate(position)
        els.append([float(position)] + list(c))
        els.sort(key=lambda e: e[0])
        self._r._write(interp, mode, els)
        for i, e in enumerate(els):
            if e[0] == float(position):
                return ColorRampElement(self._r, i)

    def remove(self, el):
        interp, mode, els = self._r._parse()
        els.pop(el._i)
        self._r._write(interp, mode, els)


class ColorRamp(bpy_struct):
    def __init__(self, node):
        self._node = node

    def _parse(self):
        t = self._node._j()['props'].get('color_ramp', 'LINEAR;RGB')
        t = [x[1] for x in P.tokenize(t)][0] if t.startswith('"') else t
        parts = t.split(';')
        els = [[float(x) for x in p.split(',')] for p in parts[2:] if p]
        return parts[0], parts[1] if len(parts) > 1 else 'RGB', els

    def _write(self, interp, mode, els):
        text = ';'.join([interp, mode] + [','.join('%.9g' % x for x in e) for e in els])
        req('node_set', *self._node._t._owner, self._node._n, P.Raw('prop:color_ramp'), text)

    @property
    def elements(self):
        return ColorRampElements(self)

    @property
    def interpolation(self):
        return self._parse()[0]

    @interpolation.setter
    def interpolation(self, v):
        _, mode, els = self._parse()
        self._write(str(v), mode, els)

    @property
    def color_mode(self):
        return self._parse()[1]

    @color_mode.setter
    def color_mode(self, v):
        interp, _, els = self._parse()
        self._write(interp, str(v), els)

    hue_interpolation = 'NEAR'

    def evaluate(self, position):
        interp, _, els = self._parse()
        if not els:
            return [0.0, 0.0, 0.0, 1.0]
        if position <= els[0][0]:
            return els[0][1:5]
        if position >= els[-1][0]:
            return els[-1][1:5]
        for a, b in zip(els, els[1:]):
            if a[0] <= position <= b[0]:
                if interp == 'CONSTANT':
                    return a[1:5]
                t = (position - a[0]) / (b[0] - a[0]) if b[0] != a[0] else 0.0
                return [a[k] + (b[k] - a[k]) * t for k in range(1, 5)]
        return els[-1][1:5]


_NODE_FIELDS = ('name', 'label', 'width', 'mute', 'hide', 'select', 'location', 'color', 'parent', 'use_custom_color')


class Node(bpy_struct):
    def __init__(self, tree, name):
        self.__dict__['_t'] = tree
        self.__dict__['_n'] = name

    def _key(self):
        return ('node', self._t._owner, self._n)

    def __repr__(self):
        return "%s.nodes['%s']" % (repr(self._t), self._n)

    def _j(self):
        return self._t._node_j(self._n)

    def _set(self, target, value):
        req('node_set', *self._t._owner, self._n, P.Raw(target), value)

    @property
    def name(self):
        return self._n

    @property
    def bl_idname(self):
        return self._j()['bl_idname']

    @property
    def type(self):
        return _node_type(self.bl_idname)

    @property
    def label(self):
        return self._j()['label']

    @property
    def location(self):
        def write(v):
            self._set('location', [v[0], v[1]])
        return Vector(self._j()['location'], _owner=write)

    @property
    def width(self):
        return self._j()['width']

    @property
    def dimensions(self):
        return Vector((self.width, 100.0))

    @property
    def mute(self):
        return self._j()['mute']

    @property
    def hide(self):
        return self._j()['hide']

    @property
    def select(self):
        return self._j()['select']

    @property
    def color(self):
        return Vector(self._j()['color'] or (0.6, 0.6, 0.6))

    @property
    def parent(self):
        p = self._j()['parent']
        return Node(self._t, p) if p else None

    @property
    def inputs(self):
        return NodeSockets(self._t, self._n, False)

    @property
    def outputs(self):
        return NodeSockets(self._t, self._n, True)

    @property
    def color_ramp(self):
        return ColorRamp(self)

    @property
    def id_data(self):
        return self._t

    @property
    def internal_links(self):
        return []

    use_custom_color = False
    show_options = True
    show_preview = False
    show_texture = False

    def __getattr__(self, key):
        if key.startswith('_'):
            raise AttributeError(key)
        props = self._j()['props']
        if key not in props:
            raise AttributeError("'%s' node has no attribute '%s'" % (self.bl_idname, key))
        v = props[key]
        if key in ('image', 'node_tree', 'object', 'material', 'collection'):
            name = [t[1] for t in P.tokenize(v)][0] if v and v != 'none' else None
            if name is None:
                return None
            from . import data
            coll = {'image': data.images, 'node_tree': data.node_groups, 'object': data.objects, 'material': data.materials,
                    'collection': data.collections}[key]
            return coll.get(name)
        if v.startswith('"'):
            return [t[1] for t in P.tokenize(v)][0]
        p = parse_value(v)
        if key.startswith('use_') or key.startswith('invert'):
            return bool(p)
        return p

    def __setattr__(self, key, value):
        if key in ('name',):
            got = first(req('node_set', *self._t._owner, self._n, P.Raw('name'), str(value)))
            self.__dict__['_n'] = got or str(value)
            return
        if key == 'location':
            self._set('location', [float(value[0]), float(value[1])])
            return
        if key == 'parent':
            self._set('parent', value.name if value is not None else P.Raw('none'))
            return
        if key in ('label',):
            self._set(key, str(value))
            return
        if key in ('width',):
            self._set(key, float(value))
            return
        if key in ('mute', 'hide', 'select'):
            self._set(key, bool(value))
            return
        if key == 'color':
            self._set('color', [float(c) for c in value][:3])
            return
        if key in ('use_custom_color', 'show_options', 'show_preview', 'show_texture'):
            self.__dict__[key] = value
            return
        self._set('prop:' + key, _id_value(value) if not isinstance(value, str) else P.Raw(value) if value.isidentifier() else value)


class Nodes(Collection_):
    def __init__(self, tree):
        self._t = tree

    def _items(self):
        return [Node(self._t, n['name']) for n in self._t._j()['nodes']]

    def new(self, type):
        name = first(req('node_add', *self._t._owner, P.Raw(type)))
        return Node(self._t, name)

    def remove(self, node):
        req('node_remove', *self._t._owner, node.name)

    def clear(self):
        for n in self._items():
            self.remove(n)

    @property
    def active(self):
        a = self._t._j().get('active')
        return Node(self._t, a) if a else None

    @active.setter
    def active(self, node):
        req('node_tree', *self._t._owner, active=node.name if node is not None else P.Raw('none'))


class NodeLink(bpy_struct):
    def __init__(self, tree, j):
        self._t = tree
        self._lj = j

    def _key(self):
        j = self._lj
        return ('link', self._t._owner, j['from_node'], j['from_index'], j['to_node'], j['to_index'])

    @property
    def from_node(self):
        return Node(self._t, self._lj['from_node'])

    @property
    def to_node(self):
        return Node(self._t, self._lj['to_node'])

    @property
    def from_socket(self):
        return NodeSocket(self._t, self._lj['from_node'], True, self._lj['from_index'])

    @property
    def to_socket(self):
        return NodeSocket(self._t, self._lj['to_node'], False, self._lj['to_index'])

    @property
    def is_muted(self):
        return self._lj['muted']

    is_valid = True
    is_hidden = False


class NodeLinks(Collection_):
    def __init__(self, tree):
        self._t = tree

    def _items(self):
        return [NodeLink(self._t, l) for l in self._t._j()['links']]

    def _name_of(self, item):
        return None

    def new(self, input, output, verify_limits=True):
        a, b = input, output
        if not a.is_output and b.is_output:
            a, b = b, a
        req('node_link', *self._t._owner, a._n, a.identifier, b._n, b.identifier)
        for l in self._items():
            if l.to_node.name == b._n and l.to_socket._i == b._i:
                return l
        return None

    def remove(self, link):
        req('node_unlink', *self._t._owner, link._lj['to_node'], link._lj['to_socket'])

    def clear(self):
        for l in self._items():
            self.remove(l)


# --- materials ----------------------------------------------------------------------------

class Material(ID):
    _kind = 'material'
    _collection_name = 'materials'

    def _j(self):
        return req('dump_material', self._name)

    def _set(self, **opts):
        req('material_set', self._name, **opts)

    @property
    def diffuse_color(self):
        def write(v):
            self._set(base_color=list(v))
        c = list(self._j()['base_color'])
        return FloatArray((c + [1.0])[:4], write)

    @diffuse_color.setter
    def diffuse_color(self, v):
        self._set(base_color=[float(x) for x in v])

    base_color = diffuse_color

    def _num(key):
        return property(lambda self: self._j()[key], lambda self, v: self._set(**{key: float(v)}))

    metallic = _num('metallic')
    roughness = _num('roughness')
    ior = _num('ior')
    transmission = _num('transmission')
    emission_strength = _num('emission_strength')
    del _num

    @property
    def emission_color(self):
        def write(v):
            self._set(emission=list(v)[:3])
        return FloatArray(self._j()['emission'], write)

    @emission_color.setter
    def emission_color(self, v):
        self._set(emission=[float(x) for x in v][:3])

    @property
    def pass_index(self):
        return self._j()['pass_index']

    @pass_index.setter
    def pass_index(self, v):
        self._set(pass_index=int(v))

    @property
    def use_backface_culling(self):
        return not self._j()['double_sided']

    @use_backface_culling.setter
    def use_backface_culling(self, v):
        self._set(double_sided=not v)

    @property
    def use_nodes(self):
        return self._j()['use_nodes']

    @use_nodes.setter
    def use_nodes(self, on):
        req('use_nodes', 'material', self._name, bool(on))

    @property
    def node_tree(self):
        if not self._j()['use_nodes']:
            return None
        return NodeTree('Shader Nodetree', ('material', self._name))

    @property
    def users(self):
        return self._j()['users']

    def _local(key, default):
        return property(lambda self: self.get('_' + key, default), lambda self, v: self.__setitem__('_' + key, v))

    specular_intensity = _local('specular_intensity', 0.5)
    specular_color = _local('specular_color', (1.0, 1.0, 1.0))
    blend_method = _local('blend_method', 'OPAQUE')
    shadow_method = _local('shadow_method', 'OPAQUE')
    surface_render_method = _local('surface_render_method', 'DITHERED')
    preview_render_type = _local('preview_render_type', 'SPHERE')
    show_transparent_back = _local('show_transparent_back', True)
    use_screen_refraction = _local('use_screen_refraction', False)
    line_color = _local('line_color', (0.0, 0.0, 0.0, 1.0))
    del _local

    is_grease_pencil = False

    @property
    def animation_data(self):
        return None

    def copy(self):
        return Material(first(req('material_copy', self._name)))


# --- world ----------------------------------------------------------------------------------

class World(ID):
    _kind = 'world'
    _collection_name = 'worlds'

    def _rename(self, new):
        return new

    def _j(self):
        return req('dump_scene')['world']

    @property
    def color(self):
        def write(v):
            req('world', color=list(v))
        return FloatArray(self._j()['color'], write)

    @color.setter
    def color(self, v):
        req('world', color=[float(x) for x in v][:3])

    @property
    def use_nodes(self):
        return self._j()['use_nodes']

    @use_nodes.setter
    def use_nodes(self, on):
        req('use_nodes', 'world', bool(on))

    @property
    def node_tree(self):
        return NodeTree('Shader Nodetree', ('world',)) if self._j()['use_nodes'] else None

    @property
    def light_settings(self):
        return bpy_struct()

    @property
    def animation_data(self):
        return None

    mist_settings = None


# --- images ---------------------------------------------------------------------------------

class _Pixels(FloatArray):
    def foreach_set(self, seq):
        self[:] = list(seq)

    def foreach_get(self, seq):
        seq[:len(self)] = list(self)


class Image(ID):
    _kind = 'image'
    _collection_name = 'images'

    def _j(self, pixels=False):
        return req('dump_image', self._name, pixels=True if pixels else None)

    @property
    def size(self):
        return list(self._j()['size'])

    @property
    def generated_width(self):
        return self.size[0]

    @property
    def generated_height(self):
        return self.size[1]

    @property
    def pixels(self):
        j = self._j(True)

        def write(v):
            req('image_pixels', self._name, P.b64_floats([float(x) for x in v]))
        return _Pixels(P.floats('b64:' + j['pixels']), write)

    @pixels.setter
    def pixels(self, v):
        req('image_pixels', self._name, P.b64_floats([float(x) for x in v]))

    channels = 4
    depth = 32
    has_data = True
    source = 'GENERATED'
    type = 'IMAGE'
    is_dirty = False

    def _local(key, default):
        return property(lambda self: self.get('_' + key, default), lambda self, v: self.__setitem__('_' + key, v))

    filepath = _local('filepath', '')
    filepath_raw = _local('filepath_raw', '')
    file_format = _local('file_format', 'PNG')
    alpha_mode = _local('alpha_mode', 'STRAIGHT')
    use_fake_user = _local('use_fake_user', False)
    is_float = _local('is_float', False)
    del _local

    @property
    def colorspace_settings(self):
        img = self

        class _CS:
            @property
            def name(self):
                return img.get('_colorspace', 'sRGB')

            @name.setter
            def name(self, v):
                img['_colorspace'] = v
        return _CS()

    def save(self, filepath=None, quality=90):
        path = filepath or self.filepath_raw or self.filepath
        if not path:
            raise RuntimeError('Error: Image "%s" does not have any image data' % self._name)
        req('image_save', self._name, path)

    def save_render(self, filepath, scene=None, quality=0):
        req('image_save', self._name, filepath)

    def update(self):
        pass

    def reload(self):
        pass

    def pack(self, data=None, data_len=0):
        pass

    def unpack(self, method='USE_LOCAL'):
        pass

    def gl_load(self, frame=0, layer=0, pass_index=0):
        return 0

    def scale(self, width, height, frame=0, tile_index=0):
        raise NotImplementedError('Image.scale is not supported yet')

    @property
    def users(self):
        return 1
