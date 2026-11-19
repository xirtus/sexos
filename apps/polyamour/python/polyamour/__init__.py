# SPDX-License-Identifier: GPL-3.0-only
"""Polyamour's Python API, shaped like Blender's `bpy` (which is this
package under Blender's name).

Inside Polyamour every script, console line, add-on and handler runs in the
Python *kernel* (kernel.py) with a live channel: each read and write is a
request answered at once, so `bpy.ops.mesh.primitive_cube_add()` followed by
`bpy.context.active_object.data.vertices` sees the new cube. A plain
`python script.py` outside Polyamour gets the one-way batch stream instead
(writes only; see polyops.py).
"""
from mathutils import Color, Euler, Matrix, Quaternion, Vector  # noqa: F401

from . import polyops  # noqa: F401
from . import polyops as _P

# Classes registered by add-ons, by class name (bpy.types.<Name>).
_bpy_types_registry = {}

from . import types  # noqa: E402,F401
from . import app, rna  # noqa: E402,F401
from .ops import OpsRoot  # noqa: E402
from .scene import BlendData, Context  # noqa: E402

data = BlendData()
context = Context()
ops = OpsRoot()
props = rna  # bpy.props names live in rna (BoolProperty, ...)


def flush():
    """Send pending local changes (mesh element edits) now."""
    _P.get().flush()


def _decode_pairs(text):
    if not text:
        return []
    flat = _P.uints(text) if text.startswith('b64:') else [i for p in text.split(';') if p for i in _P.uints(p)]
    return [(flat[i], flat[i + 1]) for i in range(0, len(flat), 2)]


def mesh_get(target, evaluated=False):
    """Read a mesh (or an object's base/evaluated mesh) via the `mesh_get`
    polyops query. Returns a dict with `verts`, `faces`, `edges`, `uvs`,
    `smooth`, and `materials`."""
    name = getattr(target, 'name', target)
    opts = {'evaluated': True} if evaluated else {}
    raw = _P.request('mesh_get', str(name), **opts)
    if not isinstance(raw, dict):
        return {'verts': [], 'faces': [], 'edges': [], 'uvs': [], 'smooth': [], 'materials': []}
    flat_v = _P.floats(raw.get('verts', ''))
    verts = [tuple(flat_v[i:i + 3]) for i in range(0, len(flat_v), 3)]
    faces = [tuple(f) for f in _P.polygons(raw.get('faces', ''))]
    edges = _decode_pairs(raw.get('edges', ''))
    uv_flat = _P.floats(raw.get('uvs', '')) if 'uvs' in raw else []
    uvs = [tuple(uv_flat[i:i + 2]) for i in range(0, len(uv_flat), 2)]
    s = raw.get('smooth', '')
    if s in ('1', 'true'):
        smooth = [True] * len(faces)
    elif s in ('0', 'false', ''):
        smooth = [False] * len(faces)
    else:
        smooth = [c == '1' for c in s]
    materials = _P.uints(raw['mat']) if 'mat' in raw else [0] * len(faces)
    return {
        'verts': verts,
        'faces': faces,
        'edges': edges,
        'uvs': uvs,
        'smooth': smooth,
        'materials': materials,
    }


def ui_panel(panel_id, label=None, category='Tool'):
    """Register a declarative sidebar panel via the `ui_panel` polyops command."""
    return _P.request('ui_panel', str(panel_id), label=label, category=category)


def ui_prop(panel_id, key, kind='float', label=None, default=None, min=None, max=None):
    """Register a property or button inside a `ui_panel`."""
    default_s = str(default) if default is not None else None
    return _P.request(
        'ui_prop', str(panel_id), str(key),
        kind=kind, label=label, default=default_s, min=min, max=max,
    )


__all__ = [
    'app', 'context', 'data', 'flush', 'mesh_get', 'ops', 'props', 'types',
    'ui_panel', 'ui_prop', 'Color', 'Euler', 'Matrix', 'Quaternion', 'Vector',
]
