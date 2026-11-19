# SPDX-License-Identifier: GPL-3.0-only
# An add-on: save this file into the add-ons folder (Preferences > Add-ons
# shows where) and enable it there, or run it here to try it out.
bl_info = {
    "name": "Star Mesh",
    "author": "You",
    "version": (1, 0),
    "blender": (4, 2, 0),
    "location": "View3D > Add > Mesh > Star",
    "description": "Adds a star-shaped mesh",
    "category": "Add Mesh",
}

import math

import bpy
from bpy.props import FloatProperty, IntProperty
from bpy_extras.object_utils import AddObjectHelper, object_data_add


def star(points, outer, inner, depth):
    verts = [(0, 0, depth / 2), (0, 0, -depth / 2)]
    for i in range(points * 2):
        a = math.pi * i / points
        r = outer if i % 2 == 0 else inner
        verts.append((r * math.cos(a), r * math.sin(a), 0))
    n = points * 2
    faces = []
    for i in range(n):
        a, b = 2 + i, 2 + (i + 1) % n
        faces.append((0, a, b))
        faces.append((1, b, a))
    return verts, faces


class MESH_OT_star_add(bpy.types.Operator, AddObjectHelper):
    """Add a star mesh"""
    bl_idname = "mesh.star_add"
    bl_label = "Star"
    bl_options = {'REGISTER', 'UNDO'}

    points: IntProperty(name="Points", default=5, min=3, max=64)
    outer: FloatProperty(name="Outer Radius", default=1.0, min=0.01)
    inner: FloatProperty(name="Inner Radius", default=0.4, min=0.01)
    depth: FloatProperty(name="Depth", default=0.3, min=0.0)

    def execute(self, context):
        verts, faces = star(self.points, self.outer, self.inner, self.depth)
        mesh = bpy.data.meshes.new("Star")
        mesh.from_pydata(verts, [], faces)
        mesh.update()
        object_data_add(context, mesh, operator=self)
        return {'FINISHED'}


def menu_func(self, context):
    self.layout.operator(MESH_OT_star_add.bl_idname, text="Star")


def register():
    bpy.utils.register_class(MESH_OT_star_add)
    bpy.types.VIEW3D_MT_mesh_add.append(menu_func)


def unregister():
    bpy.types.VIEW3D_MT_mesh_add.remove(menu_func)
    bpy.utils.unregister_class(MESH_OT_star_add)


if __name__ == "__main__":
    register()
    bpy.ops.mesh.star_add(points=6)
