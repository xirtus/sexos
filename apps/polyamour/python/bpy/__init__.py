# SPDX-License-Identifier: GPL-3.0-only
"""bpy for Polyamour: the `polyamour` module under Blender's name, so simple
Blender scripts (`import bpy`, `bpy.ops.mesh.primitive_cube_add(...)`,
`bpy.data.objects['Cube']`, `bpy.context.active_object`) run unchanged.

`import bpy.types` and `from bpy.types import Object` work too: the submodule
is registered under its Blender name."""
import sys

import polyamour as _pa
from polyamour import Color, Euler, Vector, context, data, flush, ops, polyops, types  # noqa: F401

__all__ = _pa.__all__

sys.modules[__name__ + '.types'] = _pa.types
