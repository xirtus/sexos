# SPDX-License-Identifier: GPL-3.0-only
"""bmesh.geometry."""
from mathutils.geometry import intersect_point_tri


def intersect_face_point(face, point):
    vs = [v.co for v in face.verts]
    for i in range(1, len(vs) - 1):
        if intersect_point_tri(point, vs[0], vs[i], vs[i + 1]) is not None:
            return True
    return False
