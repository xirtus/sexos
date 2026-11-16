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


__all__ = ['app', 'context', 'data', 'flush', 'ops', 'props', 'types', 'Color', 'Euler', 'Matrix', 'Quaternion', 'Vector']
