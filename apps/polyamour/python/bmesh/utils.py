# SPDX-License-Identifier: GPL-3.0-only
"""bmesh.utils: a few element-level helpers."""
from .types import BMLoopUV


def face_flip(face):
    face.normal_flip()


def vert_separate(vert, edges):
    raise NotImplementedError('bmesh.utils.vert_separate is not supported yet')


def edge_split(edge, vert, fac):
    """Split `edge` at `fac` from `vert`: returns (new edge, new vert)."""
    bm = None
    for l in edge.link_loops:
        bm = l.face._bm
        break
    if bm is None:
        raise ValueError('edge_split needs an edge of a BMesh face here')
    other = edge.other_vert(vert)
    nv = bm.verts.new(vert.co.lerp(other.co, fac))
    for l in list(edge.link_loops):
        f = l.face
        vs = f.verts
        i = vs.index(l.vert)
        vs.insert(i + 1, nv)
        mat, smooth = f.material_index, f.smooth
        data = [x._data for x in f.loops]
        bm.faces.remove(f)
        nf = bm.faces.new(vs)
        nf.material_index, nf.smooth = mat, smooth
        for x, d in zip([x for x in nf.loops if x.vert is not nv], data):
            x._data = d
    if edge.is_valid:
        bm.edges.remove(edge)
    bm.edges.get((vert, nv)) or bm.edges.new((vert, nv))
    ne = bm.edges.get((nv, other)) or bm.edges.new((nv, other))
    return ne, nv


def face_split(face, vert_a, vert_b, coords=(), use_exist=True, example=None):
    bm = face._bm
    vs = face.verts
    i, j = vs.index(vert_a), vs.index(vert_b)
    if i > j:
        i, j = j, i
    a = vs[i:j + 1]
    b = vs[j:] + vs[:i + 1]
    mat, smooth = face.material_index, face.smooth
    bm.faces.remove(face)
    f1 = bm.faces.new(a)
    f2 = bm.faces.new(b)
    for f in (f1, f2):
        f.material_index, f.smooth = mat, smooth
    return f1, f1.loops[0]


def loop_separate(loop):
    raise NotImplementedError('bmesh.utils.loop_separate is not supported yet')


_ = BMLoopUV
