# SPDX-License-Identifier: GPL-3.0-only
"""Region/3D conversions from the region's matrices (identity without a viewport)."""
from mathutils import Vector


def region_2d_to_vector_3d(region, rv3d, coord):
    vi = rv3d.view_matrix.inverted()
    return (vi.to_3x3() @ Vector((0, 0, -1))).normalized()


def region_2d_to_origin_3d(region, rv3d, coord, clamp=None):
    return rv3d.view_matrix.inverted().to_translation()


def region_2d_to_location_3d(region, rv3d, coord, depth_location):
    return Vector(depth_location)


def location_3d_to_region_2d(region, rv3d, coord, default=None):
    p = rv3d.perspective_matrix @ Vector(coord).to_4d()
    if p.w <= 0:
        return default
    return Vector(((p.x / p.w + 1) * region.width / 2, (p.y / p.w + 1) * region.height / 2))
