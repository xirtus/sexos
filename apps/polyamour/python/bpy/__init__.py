# SPDX-License-Identifier: GPL-3.0-only
"""bpy for Polyamour: the `polyamour` package under Blender's module names,
so Blender scripts and add-ons (`import bpy`, `bpy.ops.mesh.primitive_cube_add()`,
`bpy.data.objects['Cube']`, `from bpy.types import Operator`,
`from bpy.props import IntProperty`, `bpy.utils.register_class`) run unchanged."""
import sys

import polyamour as _pa
from polyamour import Color, Euler, Matrix, Quaternion, Vector, context, data, flush, ops, polyops  # noqa: F401

from . import app, msgbus, path, props, types, utils  # noqa: F401

sys.modules[__name__ + '.ops'] = ops

__all__ = ['app', 'context', 'data', 'msgbus', 'ops', 'path', 'props', 'types', 'utils']
