# SPDX-License-Identifier: GPL-3.0-only
"""Registration and UI: bpy.props, the classes add-ons subclass (Operator,
Panel, Menu, PropertyGroup, AddonPreferences, UIList, Header), the UILayout
recorder that turns a draw() into a description Polyamour renders with its
own widgets, and the window manager / preferences stand-ins."""
import sys

from . import polyops as P
from ._base import Collection_, ID, bpy_struct, req, _StructMeta, _idprops


# --- properties --------------------------------------------------------------------------------

_DEFAULTS = {'bool': False, 'int': 0, 'float': 0.0, 'string': '', 'enum': None, 'pointer': None, 'collection': None}


class _PropDef:
    """What bpy.props.*Property() returns: a deferred property that becomes a
    descriptor when its class is registered (or assigned to a bpy type)."""

    def __init__(self, kind, **kw):
        self.kind = kind
        self.kw = kw
        self.attr = None
        self.size = kw.get('size')

    @property
    def function(self):
        return getattr(sys.modules[__name__], {'bool': 'BoolProperty', 'int': 'IntProperty', 'float': 'FloatProperty',
                                               'string': 'StringProperty', 'enum': 'EnumProperty', 'pointer': 'PointerProperty',
                                               'collection': 'CollectionProperty'}[self.kind])

    @property
    def keywords(self):
        return dict(self.kw)

    def _bind_name(self, name):
        self.attr = name

    def __set_name__(self, owner, name):
        self.attr = name

    def __repr__(self):
        return '<_PropertyDeferred, %s, %r>' % (self.kind, self.kw)

    # descriptor
    def default(self, inst=None):
        kw = self.kw
        if self.kind == 'pointer':
            return None
        if self.kind == 'enum':
            items = self.enum_items(inst)
            d = kw.get('default')
            if kw.get('options') and 'ENUM_FLAG' in kw['options']:
                return set(d or ())
            if d is None:
                return items[0][0] if items else ''
            if isinstance(d, int) and items:
                for it in items:
                    if len(it) > 4 and it[4] == d:
                        return it[0]
                return items[min(d, len(items) - 1)][0]
            return d
        d = kw.get('default', _DEFAULTS[self.kind])
        if self.size or (self.kind in ('float', 'int', 'bool') and isinstance(d, (tuple, list))):
            n = self.size or len(d)
            if not isinstance(d, (tuple, list)):
                d = [d] * n
            return list(d)[:n] if isinstance(n, int) else list(d)
        return d

    def enum_items(self, inst=None):
        items = self.kw.get('items', ())
        if callable(items):
            try:
                from . import context
                items = items(inst, context) or ()
            except Exception:
                items = ()
        return [tuple(i) if i is not None else None for i in items if i is not None]

    def _store(self, inst):
        if hasattr(inst, '_props_store'):
            return inst._props_store()
        return _idprops.setdefault(inst._key(), {})

    def __get__(self, inst, owner):
        if inst is None:
            return self
        kw = self.kw
        if 'get' in kw and kw['get'] is not None:
            return kw['get'](inst)
        store = self._store(inst)
        if self.kind == 'pointer':
            t = kw.get('type')
            if t is not None and issubclass(t, PropertyGroup):
                if self.attr not in store or not isinstance(store[self.attr], PropertyGroup):
                    store[self.attr] = t._new_group()
                return store[self.attr]
            return store.get(self.attr)
        if self.kind == 'collection':
            if self.attr not in store or not isinstance(store[self.attr], PropCollection):
                store[self.attr] = PropCollection(kw.get('type'))
            return store[self.attr]
        if self.attr in store:
            v = store[self.attr]
        else:
            v = self.default(inst)
        if isinstance(v, list) and self.kind in ('float', 'int', 'bool'):
            from ._arrays import FloatArray, IntArray, BoolArray
            cls = {'float': FloatArray, 'int': IntArray, 'bool': BoolArray}[self.kind]
            sub = kw.get('subtype', '')
            if self.kind == 'float' and sub in ('TRANSLATION', 'DIRECTION', 'VELOCITY', 'ACCELERATION', 'XYZ', 'XYZ_LENGTH', 'EULER', 'QUATERNION', 'AXISANGLE', 'COLOR', 'COLOR_GAMMA'):
                import mathutils
                if sub == 'EULER':
                    return mathutils.Euler(v, _owner=lambda x: self.__set__(inst, list(x)))
                if sub == 'QUATERNION':
                    return mathutils.Quaternion(v, _owner=lambda x: self.__set__(inst, list(x)))
                if sub in ('COLOR', 'COLOR_GAMMA') and len(v) == 3:
                    return mathutils.Color(v, _owner=lambda x: self.__set__(inst, list(x)))
                if sub not in ('COLOR', 'COLOR_GAMMA'):
                    return mathutils.Vector(v, _owner=lambda x: self.__set__(inst, list(x)))
            return cls(v, lambda x: self.__set__(inst, x))
        return v

    def __set__(self, inst, value):
        kw = self.kw
        if 'set' in kw and kw['set'] is not None:
            kw['set'](inst, value)
        else:
            store = self._store(inst)
            store[self.attr] = self.coerce(value, inst)
        upd = kw.get('update')
        if upd is not None:
            from . import context
            upd(inst, context)

    def coerce(self, value, inst=None):
        kw = self.kw
        k = self.kind
        if k == 'bool':
            return [bool(x) for x in value] if self.size else bool(value)
        if k in ('int', 'float'):
            conv = int if k == 'int' else float

            def clamp(x):
                x = conv(x)
                if kw.get('min') is not None:
                    x = max(conv(kw['min']), x)
                if kw.get('max') is not None:
                    x = min(conv(kw['max']), x)
                return x
            if self.size or isinstance(value, (list, tuple)) or hasattr(value, '__len__'):
                return [clamp(x) for x in value]
            return clamp(value)
        if k == 'string':
            s = str(value)
            n = kw.get('maxlen', 0)
            return s[:n] if n else s
        if k == 'enum':
            ids = [i[0] for i in self.enum_items(inst)]
            if kw.get('options') and 'ENUM_FLAG' in kw['options']:
                v = set(value)
                bad = [x for x in v if ids and x not in ids]
                if bad:
                    raise TypeError('enum "%s" not found in %s' % (bad[0], tuple(ids)))
                return v
            if ids and value not in ids:
                raise TypeError('bpy_struct: item.attr = val: enum "%s" not found in %s' % (value, tuple(ids)))
            return value
        return value

    def info(self, inst=None):
        """JSON description of the property for a UI."""
        kw = self.kw
        k = self.kind
        d = {'kind': k, 'name': kw.get('name') or (self.attr or '').replace('_', ' ').title(), 'description': kw.get('description', '')}
        for key in ('min', 'max', 'soft_min', 'soft_max', 'step', 'precision', 'subtype', 'unit', 'size'):
            if kw.get(key) is not None:
                d[key] = kw[key]
        if k == 'enum':
            d['items'] = [[i[0], i[1] if len(i) > 1 else i[0], i[2] if len(i) > 2 else ''] for i in self.enum_items(inst)]
            d['flag'] = bool(kw.get('options') and 'ENUM_FLAG' in kw['options'])
        return d


def _prop(kind):
    def make(**kw):
        return _PropDef(kind, **kw)
    return make


BoolProperty = _prop('bool')
IntProperty = _prop('int')
FloatProperty = _prop('float')
StringProperty = _prop('string')
EnumProperty = _prop('enum')
PointerProperty = _prop('pointer')
CollectionProperty = _prop('collection')


def BoolVectorProperty(**kw):
    kw.setdefault('size', 3)
    kw.setdefault('default', [False] * (kw['size'] if isinstance(kw['size'], int) else 3))
    return _PropDef('bool', **kw)


def IntVectorProperty(**kw):
    kw.setdefault('size', 3)
    kw.setdefault('default', [0] * (kw['size'] if isinstance(kw['size'], int) else 3))
    return _PropDef('int', **kw)


def FloatVectorProperty(**kw):
    kw.setdefault('size', 3)
    kw.setdefault('default', [0.0] * (kw['size'] if isinstance(kw['size'], int) else 3))
    return _PropDef('float', **kw)


def RemoveProperty(cls, attr=''):
    if attr and attr in cls.__dict__:
        delattr(cls, attr)


def own_annotations(c):
    """A class's own annotations. Python 3.14 evaluates them lazily: the class
    dict then holds an annotate function and no `__annotations__` until asked."""
    ann = c.__dict__.get('__annotations__')
    if ann is None and ('__annotate_func__' in c.__dict__ or '__annotate__' in c.__dict__):
        try:
            import annotationlib
            ann = annotationlib.get_annotations(c)
        except Exception:
            ann = None
    return ann or {}


def _class_props(cls):
    """(name, _PropDef) of a class, its bases first, from annotations and attributes."""
    out = {}
    for c in reversed(cls.__mro__):
        for k, v in own_annotations(c).items():
            if isinstance(v, _PropDef):
                out[k] = v
        for k, v in c.__dict__.items():
            if isinstance(v, _PropDef):
                out[k] = v
    return out


def _bind_props(cls):
    for k, v in _class_props(cls).items():
        v.attr = k
        type.__setattr__(cls, k, v)


class PropCollection(list):
    """A CollectionProperty value: a list of PropertyGroup items."""

    def __init__(self, item_type):
        super().__init__()
        self._type = item_type

    def add(self):
        it = self._type._new_group() if self._type is not None else PropertyGroup._new_group()
        self.append(it)
        return it

    def remove(self, index):
        del self[index]

    def clear(self):
        del self[:]

    def move(self, a, b):
        self.insert(b, self.pop(a))

    def __getitem__(self, key):
        if isinstance(key, str):
            for it in self:
                if getattr(it, 'name', None) == key:
                    return it
            raise KeyError(key)
        return list.__getitem__(self, key)

    def get(self, key, default=None):
        try:
            return self[key]
        except (KeyError, IndexError):
            return default

    def find(self, key):
        for i, it in enumerate(self):
            if getattr(it, 'name', None) == key:
                return i
        return -1

    def keys(self):
        return [getattr(i, 'name', '') for i in self]

    def values(self):
        return list(self)

    def items(self):
        return [(getattr(i, 'name', ''), i) for i in self]

    def foreach_get(self, attr, seq):
        seq[:len(self)] = [getattr(i, attr) for i in self]

    def foreach_set(self, attr, seq):
        for i, v in zip(self, seq):
            setattr(i, attr, v)


# --- registrable classes --------------------------------------------------------------------------

class _Registrable(bpy_struct):
    is_registered = False

    def _props_store(self):
        d = self.__dict__.get('_store')
        if d is None:
            d = {}
            self.__dict__['_store'] = d
        return d

    def _key(self):
        return ('instance', id(self))

    @classmethod
    def bl_rna_get_subclass_py(cls, idname, default=None):
        return _registry_by_idname(idname) or default

    @classmethod
    def bl_rna_get_subclass(cls, idname, default=None):
        return cls.bl_rna_get_subclass_py(idname, default)


class PropertyGroup(_Registrable):
    name = StringProperty(name='Name')

    @classmethod
    def _new_group(cls):
        inst = cls.__new__(cls)
        inst.__dict__['_store'] = {}
        return inst

    def __init__(self):
        self.__dict__['_store'] = {}

    def __getitem__(self, key):
        return self._props_store()[key]

    def __setitem__(self, key, value):
        self._props_store()[key] = value

    def _idp(self):
        return self._props_store()


PropertyGroup.name.attr = 'name'


class OperatorProperties(bpy_struct):
    def __init__(self, op):
        self.__dict__['_op'] = op

    def __getattr__(self, k):
        return getattr(self._op, k)

    def __setattr__(self, k, v):
        setattr(self._op, k, v)


class Operator(_Registrable):
    bl_idname = ''
    bl_label = ''
    bl_description = ''
    bl_options = {'REGISTER'}
    bl_undo_group = ''
    bl_translation_context = '*'
    bl_cursor_pending = 'DEFAULT'

    def __init__(self, *args, **kw):
        self.__dict__['_store'] = {}
        self.__dict__['layout'] = None

    @property
    def properties(self):
        return OperatorProperties(self)

    @property
    def options(self):
        class _O:
            is_invoke = False
            is_repeat = False
            is_repeat_last = False
            use_cancel_on_mouse_release = False
        return _O()

    @property
    def has_reports(self):
        return False

    def report(self, type, message):
        kind = sorted(type)[0] if type else 'INFO'
        message = str(message)
        if P.get().live:
            try:
                req('report', P.Raw(kind), message)
            except P.PolyopsError:
                pass
        _reports.append((kind, message))
        if kind in ('ERROR', 'ERROR_INVALID_INPUT', 'ERROR_INVALID_CONTEXT', 'ERROR_OUT_OF_MEMORY'):
            print('Error: %s' % message, file=sys.stderr)

    def is_property_set(self, name, ghost=True):
        return name in self._props_store()

    @classmethod
    def poll_message_set(cls, message, *args):
        pass

    def as_keywords(self, ignore=()):
        return {k: getattr(self, k) for k in _class_props(type(self)) if k not in ignore}


_reports = []


class Macro(Operator):
    @classmethod
    def define(cls, idname):
        return OperatorProperties(Operator())


class _Appendable(_Registrable):
    _extra_draw = None

    @classmethod
    def append(cls, draw_func):
        lst = cls.__dict__.get('_extra_draw')
        if lst is None:
            lst = []
            type.__setattr__(cls, '_extra_draw', lst)
        if draw_func not in lst:
            lst.append(draw_func)

    @classmethod
    def prepend(cls, draw_func):
        cls.append(draw_func)
        lst = cls.__dict__['_extra_draw']
        lst.insert(0, lst.pop())

    @classmethod
    def remove(cls, draw_func):
        lst = cls.__dict__.get('_extra_draw') or []
        if draw_func in lst:
            lst.remove(draw_func)

    @classmethod
    def is_extended(cls):
        return bool(cls.__dict__.get('_extra_draw'))

    def __init__(self):
        self.__dict__['layout'] = UILayout()


class Panel(_Appendable):
    bl_idname = ''
    bl_label = ''
    bl_space_type = 'VIEW_3D'
    bl_region_type = 'UI'
    bl_category = ''
    bl_context = ''
    bl_parent_id = ''
    bl_options = set()
    bl_order = 0
    bl_description = ''
    bl_ui_units_x = 0
    bl_owner_id = ''
    bl_translation_context = '*'
    is_popover = False
    use_pin = False

    @property
    def text(self):
        return self.__dict__.get('_text', self.bl_label)

    @text.setter
    def text(self, t):
        self.__dict__['_text'] = t


class Menu(_Appendable):
    bl_idname = ''
    bl_label = ''
    bl_description = ''
    bl_options = set()
    bl_owner_id = ''
    bl_translation_context = '*'

    def path_menu(self, searchpaths, operator, props_default=None, prop_filepath='filepath', filter_ext=None,
                  filter_path=None, display_name=None, add_operator=None):
        pass

    @classmethod
    def draw_preset(cls, layout, context):
        pass

    def draw_collapsible(self, context, layout):
        self.draw(context)


class Header(_Appendable):
    bl_idname = ''
    bl_space_type = 'VIEW_3D'
    bl_region_type = 'HEADER'


class UIList(_Registrable):
    bl_idname = ''
    layout_type = 'DEFAULT'
    filter_name = ''
    use_filter_show = False
    use_filter_sort_alpha = False
    use_filter_invert = False
    bitflag_filter_item = 1 << 30


class AddonPreferences(_Registrable):
    bl_idname = ''

    def __init__(self):
        self.__dict__['_store'] = {}
        self.__dict__['layout'] = UILayout()


class Gizmo(_Registrable):
    pass


class GizmoGroup(_Registrable):
    pass


class KeyingSetInfo(_Registrable):
    pass


class RenderEngine(_Registrable):
    bl_idname = ''
    bl_label = ''
    bl_use_preview = False


class OperatorFileListElement(PropertyGroup):
    pass


# --- the registry ---------------------------------------------------------------------------------

operators = {}      # 'module.name' -> class
panels = {}         # bl_idname -> class
menus = {}          # bl_idname -> class
headers = {}
uilists = {}
addon_prefs = {}    # module name -> class
other_classes = {}
keymaps = []        # (keymap name, space, idname, key, value, modifiers dict)


def op_idnames(idname):
    """('mod.name', 'MOD_OT_name') for either spelling."""
    if '_OT_' in idname:
        mod, name = idname.split('_OT_', 1)
        return mod.lower() + '.' + name, idname
    mod, _, name = idname.partition('.')
    return idname, mod.upper() + '_OT_' + name


def _class_id(cls):
    return getattr(cls, 'bl_idname', '') or cls.__name__


def _registry_by_idname(idname):
    for table in (operators, panels, menus, headers, uilists, other_classes):
        if idname in table:
            return table[idname]
    return None


def _send_register(kind, idname, **attrs):
    if not P.get().live:
        return
    clean = {}
    for k, v in attrs.items():
        if v is None or v == '' or v == set():
            continue
        if isinstance(v, (set, frozenset, list, tuple)):
            v = ','.join(sorted(str(x) for x in v))
        clean[k] = str(v) if not isinstance(v, (int, float)) or isinstance(v, bool) else v
    try:
        req('register', kind, idname, **clean)
    except P.PolyopsError:
        pass


def register_class(cls):
    if getattr(cls, '_registered_here', False) and cls.__dict__.get('_registered_here'):
        raise ValueError("register_class(...): already registered as a subclass '%s'" % cls.__name__)
    _bind_props(cls)
    for k, v in _class_props(cls).items():
        t = v.kw.get('type')
        if v.kind in ('pointer', 'collection') and t is not None and isinstance(t, type) and issubclass(t, PropertyGroup):
            _bind_props(t)
    if issubclass(cls, Operator):
        if not cls.bl_idname:
            raise ValueError('register_class(...): %s needs bl_idname' % cls.__name__)
        py, c = op_idnames(cls.bl_idname)
        operators[py] = cls
        _send_register('operator', py, label=cls.bl_label or py, description=(cls.__doc__ or cls.bl_description or '').strip(),
                       options=getattr(cls, 'bl_options', set()), has_invoke=hasattr(cls, 'invoke'), modal=hasattr(cls, 'modal'))
    elif issubclass(cls, Panel):
        pid = _class_id(cls)
        if cls.bl_parent_id and cls.bl_parent_id not in panels:
            raise RuntimeError("register_class(...): parent panel '%s' is not registered" % cls.bl_parent_id)
        panels[pid] = cls
        _send_register('panel', pid, label=cls.bl_label, space=cls.bl_space_type, region=cls.bl_region_type,
                       category=cls.bl_category or None, context=cls.bl_context or None, parent=cls.bl_parent_id or None,
                       order=getattr(cls, 'bl_order', 0), options=getattr(cls, 'bl_options', set()))
    elif issubclass(cls, Menu):
        mid = _class_id(cls)
        menus[mid] = cls
        _send_register('menu', mid, label=cls.bl_label)
    elif issubclass(cls, Header):
        headers[_class_id(cls)] = cls
        _send_register('header', _class_id(cls), space=cls.bl_space_type)
    elif issubclass(cls, UIList):
        uilists[_class_id(cls)] = cls
    elif issubclass(cls, AddonPreferences):
        addon_prefs[cls.bl_idname] = cls
    else:
        other_classes[_class_id(cls)] = cls
    type.__setattr__(cls, 'is_registered', True)
    type.__setattr__(cls, '_registered_here', True)
    from . import _bpy_types_registry
    _bpy_types_registry[cls.__name__] = cls
    reg = cls.__dict__.get('register')
    if isinstance(reg, classmethod):
        cls.register()


def unregister_class(cls):
    if not cls.__dict__.get('_registered_here'):
        raise RuntimeError("unregister_class(...): missing bl_rna attribute from '%s' instance (may not be registered)" % cls.__name__)
    unreg = cls.__dict__.get('unregister')
    if isinstance(unreg, classmethod):
        cls.unregister()
    kind = None
    if issubclass(cls, Operator):
        py, _ = op_idnames(cls.bl_idname)
        operators.pop(py, None)
        kind, ident = 'operator', py
    elif issubclass(cls, Panel):
        panels.pop(_class_id(cls), None)
        kind, ident = 'panel', _class_id(cls)
    elif issubclass(cls, Menu):
        menus.pop(_class_id(cls), None)
        kind, ident = 'menu', _class_id(cls)
    elif issubclass(cls, Header):
        headers.pop(_class_id(cls), None)
        kind, ident = 'header', _class_id(cls)
    elif issubclass(cls, UIList):
        uilists.pop(_class_id(cls), None)
    elif issubclass(cls, AddonPreferences):
        addon_prefs.pop(cls.bl_idname, None)
    else:
        other_classes.pop(_class_id(cls), None)
    type.__setattr__(cls, 'is_registered', False)
    type.__setattr__(cls, '_registered_here', False)
    from . import _bpy_types_registry
    _bpy_types_registry.pop(cls.__name__, None)
    if kind and P.get().live:
        try:
            req('unregister', kind, ident)
        except P.PolyopsError:
            pass


def reset_registry():
    for table in (operators, panels, menus, headers, uilists, addon_prefs, other_classes):
        for cls in list(table.values()):
            try:
                type.__setattr__(cls, 'is_registered', False)
                type.__setattr__(cls, '_registered_here', False)
            except TypeError:
                pass
        table.clear()
    del keymaps[:]


# --- UI layout recorder ---------------------------------------------------------------------------

def _jsonable(v):
    if v is None or isinstance(v, (bool, int, float, str)):
        return v
    if isinstance(v, ID):
        return v.name
    if isinstance(v, (set, frozenset)):
        return sorted(str(x) for x in v)
    if hasattr(v, '__len__') and hasattr(v, '__getitem__'):
        try:
            return [_jsonable(x) for x in v]
        except Exception:
            return str(v)
    return str(v)


class _Refs:
    """The (data, property) pairs a draw referenced, by number."""

    def __init__(self):
        self.items = []

    def add(self, data, prop):
        self.items.append((data, prop))
        return len(self.items) - 1


class UILayout(bpy_struct):
    def __init__(self, kind='column', refs=None, **attrs):
        d = self.__dict__
        d['_kind'] = kind
        d['_children'] = []
        d['_refs'] = refs if refs is not None else _Refs()
        d['_attrs'] = dict(attrs)
        d['_flags'] = {'active': True, 'enabled': True, 'alert': False, 'scale_x': 1.0, 'scale_y': 1.0, 'alignment': 'EXPAND',
                       'use_property_split': False, 'use_property_decorate': True, 'operator_context': 'INVOKE_DEFAULT',
                       'emboss': 'NORMAL', 'activate_init': False, 'ui_units_x': 0.0, 'ui_units_y': 0.0}

    def _key(self):
        return ('layout', id(self))

    def __getattr__(self, k):
        if k.startswith('_'):
            raise AttributeError(k)
        flags = self.__dict__['_flags']
        if k in flags:
            return flags[k]
        raise AttributeError("'UILayout' object has no attribute '%s'" % k)

    def __setattr__(self, k, v):
        self._flags[k] = v

    def _add(self, node):
        self._children.append(node)
        return node

    def _sub(self, kind, **attrs):
        sub = UILayout(kind, self._refs, **attrs)
        sub._flags.update({k: self._flags[k] for k in ('use_property_split', 'use_property_decorate', 'operator_context', 'enabled', 'active')})
        self._children.append(sub)
        return sub

    def to_json(self):
        out = {'type': self._kind}
        out.update(self._attrs)
        changed = {k: v for k, v in self._flags.items() if k in ('active', 'enabled', 'alert', 'alignment', 'scale_x', 'scale_y', 'use_property_split')
                   and v != {'active': True, 'enabled': True, 'alert': False, 'alignment': 'EXPAND', 'scale_x': 1.0, 'scale_y': 1.0,
                             'use_property_split': False}[k]}
        out.update(changed)
        out['items'] = [c.to_json() if isinstance(c, UILayout) else c for c in self._children]
        return out

    # containers
    def row(self, align=False, heading='', heading_ctxt='', translate=True):
        return self._sub('row', align=align, heading=heading)

    def column(self, align=False, heading='', heading_ctxt='', translate=True):
        return self._sub('column', align=align, heading=heading)

    def box(self):
        return self._sub('box')

    def split(self, factor=0.0, align=False):
        return self._sub('split', factor=factor, align=align)

    def column_flow(self, columns=0, align=False):
        return self._sub('column', align=align)

    def grid_flow(self, row_major=False, columns=0, even_columns=False, even_rows=False, align=False):
        return self._sub('grid', columns=columns, align=align)

    def panel(self, idname, default_closed=False):
        header = self._sub('row', role='panel_header', panel=idname)
        body = self._sub('column', role='panel_body', panel=idname, closed=default_closed)
        return header, body

    def panel_prop(self, data, property):
        return self.panel(property)

    def menu_pie(self):
        return self._sub('pie')

    # items
    def label(self, text='', text_ctxt='', translate=True, icon='NONE', icon_value=0):
        self._add({'type': 'label', 'text': str(text), 'icon': icon})

    def separator(self, factor=1.0, type='AUTO'):
        self._add({'type': 'separator', 'factor': factor})

    def separator_spacer(self):
        self._add({'type': 'spacer'})

    def prop(self, data, property, text=None, text_ctxt='', translate=True, icon='NONE', placeholder='', expand=False, slider=False,
             toggle=-1, icon_only=False, event=False, full_event=False, emboss=True, index=-1, icon_value=0, invert_checkbox=False):
        node = {'type': 'prop', 'prop': property, 'icon': icon, 'expand': expand, 'slider': slider, 'toggle': toggle,
                'icon_only': icon_only, 'index': index, 'emboss': emboss}
        try:
            value = getattr(data, property)
        except AttributeError:
            node.update({'type': 'label', 'text': 'property not found: %s.%s' % (type(data).__name__, property), 'alert': True})
            self._add(node)
            return
        pd = getattr(type(data), property, None)
        if isinstance(pd, _PropDef):
            node.update(pd.info(data))
        else:
            node['kind'] = ('bool' if isinstance(value, bool) else 'int' if isinstance(value, int) else 'float' if isinstance(value, float)
                            else 'string' if isinstance(value, str) else 'pointer' if isinstance(value, ID) or value is None else 'float')
            node['name'] = property.replace('_', ' ').title()
        node['text'] = node.get('name', property) if text is None else str(text)
        node['value'] = _jsonable(value)
        node['ref'] = self._refs.add(data, property)
        self._add(node)

    def props_enum(self, data, property):
        self.prop(data, property, expand=True)

    def prop_enum(self, data, property, value, text='', text_ctxt='', translate=True, icon='NONE', icon_value=0):
        self.prop(data, property, text=text or value, icon=icon)
        self._children[-1]['enum_value'] = value

    def prop_menu_enum(self, data, property, text='', text_ctxt='', translate=True, icon='NONE'):
        self.prop(data, property, text=text or None, icon=icon)

    def prop_tabs_enum(self, data, property, data_highlight=None, property_highlight='', icon_only=False):
        self.prop(data, property, expand=True)

    def prop_search(self, data, property, search_data, search_property, text='', text_ctxt='', translate=True, icon='NONE', results_are_suggestions=False):
        self.prop(data, property, text=text or None, icon=icon)
        if self._children and isinstance(self._children[-1], dict):
            try:
                coll = getattr(search_data, search_property)
                self._children[-1]['choices'] = [getattr(i, 'name', str(i)) for i in coll]
            except Exception:
                pass

    def prop_decorator(self, data, property, index=-1):
        pass

    def prop_with_popover(self, data, property, text='', icon='NONE', panel=''):
        self.prop(data, property, text=text or None, icon=icon)

    def prop_with_menu(self, data, property, text='', icon='NONE', menu=''):
        self.prop(data, property, text=text or None, icon=icon)

    def operator(self, operator, text=None, text_ctxt='', translate=True, icon='NONE', emboss=True, depress=False, icon_value=0, search_weight=0.0):
        props = _OpButton(operator)
        node = {'type': 'operator', 'operator': operator, 'text': text if text is not None else _op_label(operator), 'icon': icon,
                'emboss': emboss, 'depress': depress, 'properties': props._values, 'context': self._flags['operator_context']}
        self._add(node)
        return props

    def operator_enum(self, operator, property, icon_only=False):
        self._add({'type': 'operator_enum', 'operator': operator, 'prop': property})

    def operator_menu_enum(self, operator, property, text='', text_ctxt='', translate=True, icon='NONE'):
        self._add({'type': 'operator_menu_enum', 'operator': operator, 'prop': property, 'text': text})
        return _OpButton(operator)

    def operator_menu_hold(self, operator, **kw):
        return self.operator(operator, **{k: v for k, v in kw.items() if k in ('text', 'icon', 'emboss', 'depress')})

    def menu(self, menu, text='', text_ctxt='', translate=True, icon='NONE', icon_value=0):
        label = text or getattr(menus.get(menu), 'bl_label', '') or menu
        self._add({'type': 'menu', 'menu': menu, 'text': label, 'icon': icon})

    def menu_contents(self, menu):
        self._add({'type': 'menu_contents', 'menu': menu})

    def popover(self, panel, text='', text_ctxt='', translate=True, icon='NONE', icon_value=0):
        self._add({'type': 'popover', 'panel': panel, 'text': text, 'icon': icon})

    def popover_group(self, space_type, region_type, context, category):
        pass

    def context_pointer_set(self, name, data):
        self._attrs.setdefault('context', {})[name] = _jsonable(data)

    def context_string_set(self, name, value):
        self._attrs.setdefault('context', {})[name] = value

    def template_list(self, listtype_name, list_id, dataptr, propname, active_dataptr, active_propname, item_dyntip_propname='',
                      rows=5, maxrows=5, type='DEFAULT', columns=9, sort_reverse=False, sort_lock=False):
        try:
            items = getattr(dataptr, propname)
            names = [getattr(i, 'name', str(i)) for i in items]
        except Exception:
            names = []
        node = {'type': 'list', 'list': listtype_name, 'items': names, 'rows': rows}
        try:
            node['active'] = getattr(active_dataptr, active_propname)
            node['ref'] = self._refs.add(active_dataptr, active_propname)
        except Exception:
            pass
        self._add(node)

    def template_ID(self, data, property, new='', open='', unlink='', filter='ALL', live_icon=False, text='', text_ctxt='', translate=True):
        self.prop(data, property, text=text or None)

    template_ID_preview = template_ID
    template_any_ID = template_ID

    def template_modifiers(self):
        self._add({'type': 'template', 'template': 'modifiers'})

    def template_constraints(self, use_bone_constraints=True):
        self._add({'type': 'template', 'template': 'constraints'})

    def template_color_ramp(self, data, property, expand=False):
        self._add({'type': 'template', 'template': 'color_ramp'})

    def template_curve_mapping(self, data, property, **kw):
        self._add({'type': 'template', 'template': 'curve_mapping'})

    def template_node_view(self, ntree, node, socket):
        self._add({'type': 'template', 'template': 'node_view'})

    def template_icon(self, icon_value=0, scale=1.0):
        pass

    def template_icon_view(self, data, property, show_labels=False, scale=6.0, scale_popup=5.0):
        self.prop(data, property)

    def template_header(self):
        pass

    def template_preview(self, id, show_buttons=True, **kw):
        pass

    def template_image(self, data, property, image_user, compact=False, multiview=False):
        self.prop(data, property)

    def template_running_jobs(self):
        pass

    def template_operator_search(self):
        pass

    def template_search(self, data, property, search_data, search_property, **kw):
        self.prop_search(data, property, search_data, search_property)

    def template_reports_banner(self):
        pass

    def template_input_status(self):
        pass

    def template_status_info(self):
        pass

    def introspect(self):
        return self.to_json()

    @staticmethod
    def icon(data):
        return 0

    @staticmethod
    def enum_item_name(data, property, identifier):
        pd = getattr(type(data), property, None)
        if isinstance(pd, _PropDef):
            for i in pd.enum_items(data):
                if i[0] == identifier:
                    return i[1]
        return identifier

    enum_item_description = enum_item_name


def _op_label(idname):
    py, _ = op_idnames(idname)
    cls = operators.get(py)
    if cls is not None:
        return cls.bl_label or py
    return idname.split('.')[-1].replace('_', ' ').title()


class _OpButton:
    """What layout.operator() returns: properties to call the operator with."""

    def __init__(self, idname):
        self.__dict__['_id'] = idname
        self.__dict__['_values'] = {}

    def __setattr__(self, k, v):
        self._values[k] = _jsonable(v)

    def __getattr__(self, k):
        if k.startswith('_'):
            raise AttributeError(k)
        if k in self._values:
            return self._values[k]
        py, _ = op_idnames(self._id)
        cls = operators.get(py)
        if cls is not None:
            pd = _class_props(cls).get(k)
            if pd is not None:
                return pd.default()
        return None


# --- window manager, windows, areas ---------------------------------------------------------------

class Event(bpy_struct):
    def __init__(self, type='NONE', value='NOTHING', **kw):
        self.type = type
        self.value = value
        self.mouse_x = kw.get('mouse_x', 0)
        self.mouse_y = kw.get('mouse_y', 0)
        self.mouse_region_x = kw.get('mouse_region_x', 0)
        self.mouse_region_y = kw.get('mouse_region_y', 0)
        self.mouse_prev_x = self.mouse_x
        self.mouse_prev_y = self.mouse_y
        self.shift = kw.get('shift', False)
        self.ctrl = kw.get('ctrl', False)
        self.alt = kw.get('alt', False)
        self.oskey = kw.get('oskey', False)
        self.ascii = kw.get('ascii', '')
        self.unicode = kw.get('unicode', '')
        self.is_repeat = kw.get('is_repeat', False)
        self.is_tablet = False
        self.pressure = 1.0
        self.tilt = (0.0, 0.0)
        self.direction = 'ANY'
        self.type_prev = 'NONE'
        self.value_prev = 'NOTHING'

    def _key(self):
        return ('event', id(self))


class Timer(bpy_struct):
    def __init__(self, step):
        self.time_step = step
        self.time_delta = 0.0
        self.time_duration = 0.0

    def _key(self):
        return ('timer', id(self))


class KeyMapItem(bpy_struct):
    def __init__(self, idname, type, value, **kw):
        self.idname = idname
        self.type = type
        self.value = value
        self.any = kw.get('any', False)
        self.shift = kw.get('shift', False)
        self.ctrl = kw.get('ctrl', False)
        self.alt = kw.get('alt', False)
        self.oskey = kw.get('oskey', False)
        self.active = True
        self.properties = _OpButton(idname)

    def _key(self):
        return ('kmi', id(self))


class KeyMapItems(list):
    def __init__(self, km):
        super().__init__()
        self._km = km

    def new(self, idname, type, value, any=False, shift=0, ctrl=0, alt=0, oskey=0, key_modifier='NONE', direction='ANY', repeat=False, head=False):
        kmi = KeyMapItem(idname, type, value, any=any, shift=bool(shift), ctrl=bool(ctrl), alt=bool(alt), oskey=bool(oskey))
        self.append(kmi)
        keymaps.append((self._km.name, self._km.space_type, idname, type, value, {'shift': kmi.shift, 'ctrl': kmi.ctrl, 'alt': kmi.alt}))
        _send_register('keymap', '%s:%s' % (self._km.name, idname), key=type, value=value, shift=int(kmi.shift), ctrl=int(kmi.ctrl),
                       alt=int(kmi.alt), space=self._km.space_type)
        return kmi

    def new_modal(self, propvalue, type, value, **kw):
        return self.new(propvalue, type, value)

    def remove(self, item):
        list.remove(self, item)

    def find_from_operator(self, idname, properties=None, include={'ACTION', 'MOUSE', 'KEYBOARD', 'TWEAK'}, exclude=set()):
        for k in self:
            if k.idname == idname:
                return k
        return None


class KeyMap(bpy_struct):
    def __init__(self, name, space_type='EMPTY', region_type='WINDOW', modal=False):
        self.name = name
        self.space_type = space_type
        self.region_type = region_type
        self.is_modal = modal
        self.keymap_items = KeyMapItems(self)

    def _key(self):
        return ('keymap', self.name)

    def restore_to_default(self):
        pass


class KeyMaps(list):
    def new(self, name, space_type='EMPTY', region_type='WINDOW', modal=False, tool=False):
        for km in self:
            if km.name == name:
                return km
        km = KeyMap(name, space_type, region_type, modal)
        self.append(km)
        return km

    def find(self, name, space_type='EMPTY', region_type='WINDOW'):
        for km in self:
            if km.name == name:
                return km
        return self.new(name, space_type, region_type)

    def get(self, name, default=None):
        for km in self:
            if km.name == name:
                return km
        return default

    def __getitem__(self, k):
        if isinstance(k, str):
            return self.find(k)
        return list.__getitem__(self, k)


class KeyConfig(bpy_struct):
    def __init__(self, name):
        self.name = name
        self.keymaps = KeyMaps()

    def _key(self):
        return ('keyconfig', self.name)


class KeyConfigs(bpy_struct):
    def __init__(self):
        self.addon = KeyConfig('Polyamour addon')
        self.user = KeyConfig('Polyamour user')
        self.default = KeyConfig('Polyamour')
        self.active = self.default

    def _key(self):
        return ('keyconfigs',)

    def __iter__(self):
        return iter((self.default, self.addon, self.user))


class SpaceView3D(bpy_struct):
    type = 'VIEW_3D'

    def __init__(self):
        from mathutils import Matrix
        self.region_3d = _Region3D()
        self.shading = _Bag(type='SOLID', light='STUDIO', color_type='MATERIAL')
        self.overlay = _Bag(show_overlays=True, show_wireframes=False, show_floor=True)
        self.lens = 50.0
        self.clip_start = 0.01
        self.clip_end = 1000.0
        self.camera = None
        self.show_gizmo = True
        self.local_view = None

    def _key(self):
        return ('space', 'VIEW_3D')

    @classmethod
    def draw_handler_add(cls, callback, args, region_type, draw_type):
        return (callback, args)

    @classmethod
    def draw_handler_remove(cls, handler, region_type):
        pass


class _Region3D(bpy_struct):
    def __init__(self):
        from mathutils import Matrix, Vector, Quaternion
        self.view_matrix = Matrix.Identity(4)
        self.window_matrix = Matrix.Identity(4)
        self.perspective_matrix = Matrix.Identity(4)
        self.view_location = Vector((0, 0, 0))
        self.view_rotation = Quaternion()
        self.view_distance = 10.0
        self.view_perspective = 'PERSP'
        self.is_perspective = True
        self.is_orthographic_side_view = False

    def _key(self):
        return ('region_3d',)

    def update(self):
        pass


class _Bag(bpy_struct):
    def __init__(self, **kw):
        self.__dict__.update(kw)

    def _key(self):
        return ('bag', id(self))


class Space(bpy_struct):
    def __init__(self, type):
        self.type = type

    def _key(self):
        return ('space', self.type)

    @classmethod
    def draw_handler_add(cls, callback, args, region_type, draw_type):
        return (callback, args)

    @classmethod
    def draw_handler_remove(cls, handler, region_type):
        pass


SpaceProperties = Space


class Region(bpy_struct):
    def __init__(self, type='WINDOW', width=800, height=600):
        self.type = type
        self.width = width
        self.height = height
        self.x = 0
        self.y = 0

    def _key(self):
        return ('region', self.type)

    def tag_redraw(self):
        pass


class _Spaces(list):
    @property
    def active(self):
        return self[0] if self else None


class Area(bpy_struct):
    def __init__(self, type='VIEW_3D'):
        self.type = type
        self.ui_type = type
        self.width = 800
        self.height = 600
        self.x = 0
        self.y = 0
        self.spaces = _Spaces([SpaceView3D() if type == 'VIEW_3D' else Space(type)])
        self.regions = [Region('HEADER', 800, 26), Region('TOOLS', 50, 600), Region('UI', 250, 600), Region('WINDOW', 800, 600)]
        self.show_menus = True

    def _key(self):
        return ('area', self.type)

    def tag_redraw(self):
        pass

    def header_text_set(self, text=None):
        pass


_area = [None]


def current_area():
    return _area[0]


def set_current_area(space_type):
    _area[0] = Area(space_type) if space_type else None


class Screen(bpy_struct):
    def __init__(self):
        self.name = 'Layout'
        self.areas = [Area('VIEW_3D'), Area('PROPERTIES'), Area('OUTLINER'), Area('DOPESHEET_EDITOR')]
        self.is_animation_playing = False
        self.show_fullscreen = False

    def _key(self):
        return ('screen',)


class Window(bpy_struct):
    def __init__(self):
        self.screen = Screen()
        self.scene = None
        self.view_layer = None
        self.workspace = _Bag(name='Layout')
        self.width = 1920
        self.height = 1080

    def _key(self):
        return ('window',)

    def cursor_set(self, cursor):
        pass

    def cursor_modal_set(self, cursor):
        pass

    def cursor_modal_restore(self):
        pass

    def cursor_warp(self, x, y):
        pass

    def event_simulate(self, type, value, **kw):
        return Event(type, value)


class WindowManager(bpy_struct):
    def __init__(self):
        self.keyconfigs = KeyConfigs()
        self.windows = [Window()]
        self._clipboard = ''
        self.modal_ops = []
        self.timers = []

    def _key(self):
        return ('window_manager',)

    @property
    def clipboard(self):
        return self._clipboard

    @clipboard.setter
    def clipboard(self, t):
        self._clipboard = str(t)

    def invoke_props_dialog(self, operator, width=300, title='', confirm_text='', cancel_default=False, text_ctxt='', translate=True):
        from . import context
        return operator.execute(context)

    invoke_props_popup = invoke_props_dialog

    def invoke_confirm(self, operator, event, title='', message='', confirm_text='', icon='NONE', text_ctxt='', translate=True):
        from . import context
        return operator.execute(context)

    def invoke_popup(self, operator, width=300):
        from . import context
        return operator.execute(context)

    def invoke_search_popup(self, operator):
        return {'CANCELLED'}

    def fileselect_add(self, operator):
        from . import context
        path = getattr(operator, 'filepath', '')
        if path:
            return operator.execute(context)
        return {'CANCELLED'}

    def modal_handler_add(self, operator):
        self.modal_ops.append(operator)
        return True

    def event_timer_add(self, time_step, window=None):
        t = Timer(time_step)
        self.timers.append(t)
        return t

    def event_timer_remove(self, timer):
        if timer in self.timers:
            self.timers.remove(timer)

    def progress_begin(self, min, max):
        pass

    def progress_update(self, value):
        pass

    def progress_end(self):
        pass

    def popup_menu(self, draw_func, title='', icon='NONE'):
        menu = Menu()
        menu.__dict__['layout'] = UILayout()
        from . import context
        draw_func(menu, context)
        popups.append({'title': title, 'layout': menu.layout.to_json()})

    def popup_menu_pie(self, event, draw_func, title='', icon='NONE'):
        self.popup_menu(draw_func, title, icon)

    def popover(self, draw_func, ui_units_x=0, keymap=None, from_active_button=False):
        self.popup_menu(draw_func)

    def tag_script_reload(self):
        pass

    def print_undo_steps(self):
        pass

    @property
    def operators(self):
        return []

    @property
    def addon_search(self):
        return ''


popups = []
window_manager = WindowManager()


# --- preferences --------------------------------------------------------------------------------

class Addon(bpy_struct):
    def __init__(self, module):
        self.module = module

    def _key(self):
        return ('addon', self.module)

    @property
    def preferences(self):
        cls = addon_prefs.get(self.module)
        if cls is None:
            return None
        store = _addon_pref_instances.get(self.module)
        if store is None:
            store = cls()
            _addon_pref_instances[self.module] = store
        return store


_addon_pref_instances = {}


class Addons(Collection_):
    """Enabled add-ons (kernel.py keeps the list)."""

    def _items(self):
        from . import addons
        return [Addon(m) for m in addons.enabled]

    def _name_of(self, item):
        return item.module

    def __getitem__(self, key):
        if isinstance(key, str):
            from . import addons
            if key in addons.enabled or key in addon_prefs:
                return Addon(key)
            raise KeyError('bpy_prop_collection[key]: key "%s" not found' % key)
        return self._items()[key]

    def __contains__(self, key):
        from . import addons
        return key in addons.enabled

    def new(self):
        return Addon('')


class Preferences(bpy_struct):
    def __init__(self):
        self.addons = Addons()
        self.view = _Bag(ui_scale=1.0, show_developer_ui=False, show_tooltips=True, show_tooltips_python=False, language='en_US',
                         show_splash=False, use_translate_interface=False)
        self.edit = _Bag(use_global_undo=True, undo_steps=32, keyframe_new_interpolation_type='BEZIER', use_enter_edit_mode=False)
        self.system = _Bag(dpi=72, pixel_size=1.0, memory_cache_limit=4096, use_region_overlap=True)
        self.inputs = _Bag(use_mouse_emulate_3_button=False, view_rotate_method='TURNTABLE')
        self.filepaths = _Bag(temporary_directory='', script_directories=[], render_output_directory='', use_relative_paths=True)
        self.themes = [_Bag(name='Default')]
        self.experimental = _Bag()
        self.extensions = _Bag(repos=[])
        self.is_dirty = False
        self.app_template = ''

    def _key(self):
        return ('preferences',)


preferences = Preferences()


# --- texts ----------------------------------------------------------------------------------------

class _Line:
    def __init__(self, body):
        self.body = body


class Text(ID):
    _kind = 'text'
    _collection_name = 'texts'

    def __init__(self, name, store=None):
        ID.__init__(self, name)
        self.__dict__['_s'] = store

    def _rename(self, new):
        self._s._t[new] = self._s._t.pop(self._name)
        return new

    def as_string(self):
        return self._s._t[self._name]

    def from_string(self, text):
        self._s._t[self._name] = str(text)

    def write(self, text):
        self._s._t[self._name] += str(text)

    def clear(self):
        self._s._t[self._name] = ''

    @property
    def lines(self):
        return [_Line(l) for l in self.as_string().split('\n')]

    @property
    def current_line(self):
        ls = self.lines
        return ls[-1] if ls else _Line('')

    filepath = ''
    is_dirty = False
    is_in_memory = True
    is_modified = False
    use_module = False

    def as_module(self):
        import types
        m = types.ModuleType(self._name)
        exec(compile(self.as_string(), self._name, 'exec'), m.__dict__)
        return m

    def cursor_set(self, line, character=0, select=False):
        pass
