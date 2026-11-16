# SPDX-License-Identifier: GPL-3.0-only
"""bpy.types: Polyamour's classes under Blender's names. Built-in menus,
panels and headers (`bpy.types.VIEW3D_MT_mesh_add`, ...) exist on demand so
add-ons can `.append()` their entries; classes an add-on registered are
found here by class name, as in Blender."""
from polyamour.types import *  # noqa: F401,F403
from polyamour import types as _t
from polyamour import rna as _rna
from polyamour import _bpy_types_registry

_stubs = {}


def __getattr__(name):
    if name.startswith('__'):
        raise AttributeError(name)
    if name in _bpy_types_registry:
        return _bpy_types_registry[name]
    if hasattr(_t, name):
        return getattr(_t, name)
    if name in _stubs:
        return _stubs[name]
    base = None
    if '_MT_' in name:
        base = _rna.Menu
    elif '_PT_' in name:
        base = _rna.Panel
    elif '_HT_' in name:
        base = _rna.Header
    elif '_UL_' in name:
        base = _rna.UIList
    if base is None:
        raise AttributeError("module 'bpy.types' has no attribute '%s'" % name)
    cls = type(name, (base,), {'bl_idname': name, 'bl_label': name.split('_', 2)[-1].replace('_', ' ').title(),
                               'draw': lambda self, context: None})
    _stubs[name] = cls
    return cls


def __dir__():
    return sorted(set(dir(_t)) | set(_bpy_types_registry) | set(_stubs))
