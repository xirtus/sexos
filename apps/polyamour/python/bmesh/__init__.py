# SPDX-License-Identifier: GPL-3.0-only
"""bmesh for Polyamour: an editable linked mesh in pure Python with the
common operators (bmesh.ops), copied to and from bpy meshes.

Polyamour's Edit Mode works on the mesh itself, so from_edit_mesh() returns
a BMesh of the mesh and update_edit_mesh() writes it back."""
from . import geometry, ops, types, utils  # noqa: F401
from .types import BMesh

_edit = {}


def new(use_operators=True):
    return BMesh()


def from_edit_mesh(mesh):
    bm = _edit.get(mesh.name)
    if bm is None or not bm.is_valid:
        bm = BMesh()
        bm.from_mesh(mesh)
        bm.is_wrapped = True
        _edit[mesh.name] = bm
    return bm


def update_edit_mesh(mesh, loop_triangles=True, destructive=True):
    bm = _edit.get(mesh.name)
    if bm is not None:
        bm.to_mesh(mesh)
        mesh._flush_now()
        _edit.pop(mesh.name, None)


def _forget_edit_meshes():
    _edit.clear()
