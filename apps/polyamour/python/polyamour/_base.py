# SPDX-License-Identifier: GPL-3.0-only
"""Shared pieces of the bpy proxies: requests, ID properties, collections and
the conversions between polyops text and Python values."""
from . import polyops as P
from .polyops import Raw, quote, tokenize


def req(command, *args, **opts):
    """One request to Polyamour (see polyops.LiveChannel.request)."""
    return P.get().request(command, *args, **opts)


def ref(name, prop=None):
    """`NAME` or `NAME.prop` as a raw token (the polyops `set` path form)."""
    q = quote(name)
    return Raw(q + '.' + prop) if prop else Raw(q)


def generation():
    return P.get().generation


def mark_dirty(key, obj):
    P.get().pending[key] = obj


def first(reply, default=None):
    if isinstance(reply, list) and reply:
        return reply[0]
    return default


def parse_value(text):
    """A polyops value text as a Python value: number, list of numbers, or text."""
    if text is None:
        return None
    t = str(text)
    try:
        if ',' in t:
            return [float(x) for x in t.split(',')]
        if t.lstrip('-').isdigit():
            return int(t)
        return float(t)
    except ValueError:
        return t


def parse_opts(text):
    """' a=1 b=x' -> {'a': '1', 'b': 'x'} (texts)."""
    return {k: v for k, v, _ in tokenize(text or '') if k is not None}


def enum_out(v):
    return str(v).upper()


def enum_in(v):
    return Raw(str(v).lower())


# --- RNA-ish base ----------------------------------------------------------------

# ID properties (`obj["key"] = 1`) and registered properties live here, keyed by
# (kind, name) so every proxy of the same data-block sees them.
_idprops = {}


class _StructMeta(type):
    """Lets `bpy.types.Object.my_prop = IntProperty()` bind the property's name."""

    def __setattr__(cls, key, value):
        bind = getattr(value, '_bind_name', None)
        if bind is not None and not isinstance(value, type):
            bind(key)
        super().__setattr__(key, value)


class bpy_struct(metaclass=_StructMeta):
    _kind = 'struct'

    def _key(self):
        return (self._kind, id(self))

    def _idp(self):
        return _idprops.setdefault(self._key(), {})

    # ID properties
    def __getitem__(self, key):
        return self._idp()[key]

    def __setitem__(self, key, value):
        self._idp()[key] = value

    def __delitem__(self, key):
        del self._idp()[key]

    def __contains__(self, key):
        return key in self._idp()

    def get(self, key, default=None):
        return self._idp().get(key, default)

    def keys(self):
        return self._idp().keys()

    def values(self):
        return self._idp().values()

    def items(self):
        return self._idp().items()

    def pop(self, key, *default):
        return self._idp().pop(key, *default)

    def id_properties_ensure(self):
        return self._idp()

    def is_property_set(self, name, ghost=True):
        return name in self._idp()

    def property_unset(self, name):
        self._idp().pop(name, None)

    def as_pointer(self):
        return id(self)

    def path_from_id(self, prop=''):
        return prop

    def keyframe_insert(self, data_path, index=-1, frame=None, group='', options=set(), keytype='KEYFRAME'):
        raise TypeError('%s does not support keyframes yet' % type(self).__name__)

    @property
    def bl_rna(self):
        return _RNA(type(self).__name__)

    @property
    def rna_type(self):
        return self.bl_rna

    def __eq__(self, other):
        return type(other) is type(self) and other._key() == self._key()

    def __ne__(self, other):
        return not self.__eq__(other)

    def __hash__(self):
        return hash(self._key())


class _RNA:
    def __init__(self, identifier):
        self.identifier = identifier
        self.name = identifier
        self.description = ''
        self.properties = {}
        self.functions = {}


class ID(bpy_struct):
    """A named data-block. Subclasses define _kind and the requests."""

    _kind = 'id'

    def __init__(self, name):
        self.__dict__['_name'] = name

    def _key(self):
        return (self._kind, self._name)

    def __repr__(self):
        return "bpy.data.%s['%s']" % (self._collection_name, self._name)

    _collection_name = 'ids'

    @property
    def name(self):
        return self._name

    @name.setter
    def name(self, new):
        new = str(new)
        if new == self._name:
            return
        got = self._rename(new)
        old_key = self._key()
        self.__dict__['_name'] = got if got else new
        if old_key in _idprops:
            _idprops[self._key()] = _idprops.pop(old_key)

    def _rename(self, new):
        return first(req('rename', self._kind, self._name, new))

    @property
    def name_full(self):
        return self._name

    @property
    def is_evaluated(self):
        return False

    @property
    def original(self):
        return self

    @property
    def library(self):
        return None

    @property
    def use_fake_user(self):
        return self.get('_fake_user', False)

    @use_fake_user.setter
    def use_fake_user(self, v):
        self['_fake_user'] = bool(v)

    def user_clear(self):
        pass

    def update_tag(self, refresh=set()):
        pass

    def evaluated_get(self, depsgraph):
        return self

    def animation_data_create(self):
        return None

    def animation_data_clear(self):
        pass

    @property
    def tag(self):
        return False

    @tag.setter
    def tag(self, v):
        pass

    def __bool__(self):
        return True


class Collection_(bpy_struct):
    """A read-only sequence view with Blender's prop-collection methods.
    Subclasses implement _items() (a list) and may override _find()."""

    def _items(self):
        return []

    def __iter__(self):
        return iter(self._items())

    def __len__(self):
        return len(self._items())

    def __bool__(self):
        return True

    def _name_of(self, item):
        return getattr(item, 'name', None)

    def __getitem__(self, key):
        items = self._items()
        if isinstance(key, slice):
            return items[key]
        if isinstance(key, int):
            return items[key]
        for it in items:
            if self._name_of(it) == key:
                return it
        raise KeyError('%s[key]: key "%s" not found' % (type(self).__name__, key))

    def __contains__(self, key):
        if isinstance(key, str):
            return any(self._name_of(i) == key for i in self._items())
        return key in self._items()

    def get(self, key, default=None):
        try:
            return self[key]
        except (KeyError, IndexError):
            return default

    def find(self, key):
        for i, it in enumerate(self._items()):
            if self._name_of(it) == key:
                return i
        return -1

    def keys(self):
        return [self._name_of(i) for i in self._items()]

    def values(self):
        return list(self._items())

    def items(self):
        return [(self._name_of(i), i) for i in self._items()]

    def foreach_get(self, attr, seq):
        out = []
        for it in self._items():
            v = getattr(it, attr)
            if hasattr(v, '__len__') and not isinstance(v, str):
                out.extend(v)
            else:
                out.append(v)
        seq[:len(out)] = out

    def foreach_set(self, attr, seq):
        items = self._items()
        if not items:
            return
        first_v = getattr(items[0], attr)
        n = len(first_v) if hasattr(first_v, '__len__') and not isinstance(first_v, str) else 1
        for i, it in enumerate(items):
            v = seq[i * n:(i + 1) * n] if n > 1 else seq[i]
            setattr(it, attr, v)

    def __repr__(self):
        return '<bpy_collection[%d], %s>' % (len(self), type(self).__name__)
