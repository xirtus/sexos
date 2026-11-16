# SPDX-License-Identifier: GPL-3.0-only
# bmesh: build a mesh with operators, then edit the active mesh in Edit Mode.
import bpy
import bmesh
from mathutils import Matrix

# A new mesh: a cube with its top pulled up and shrunk.
bm = bmesh.new()
bmesh.ops.create_cube(bm, size=1.0)
top = [f for f in bm.faces if f.normal.z > 0.5]
ret = bmesh.ops.extrude_face_region(bm, geom=top)
moved = [e for e in ret["geom"] if isinstance(e, bmesh.types.BMVert)]
bmesh.ops.translate(bm, vec=(0, 0, 1), verts=moved)
bmesh.ops.scale(bm, vec=(0.5, 0.5, 1), verts=moved)
bmesh.ops.subdivide_edges(bm, edges=bm.edges[:], cuts=1, use_grid_fill=True)
me = bpy.data.meshes.new("Spire")
bm.to_mesh(me)
bm.free()
ob = bpy.data.objects.new("Spire", me)
bpy.context.collection.objects.link(ob)
ob.location.x = 3
print("Spire:", len(me.vertices), "vertices,", len(me.polygons), "faces")

# Edit Mode: lift every selected vertex of the active mesh.
bpy.context.view_layer.objects.active = ob
bpy.ops.object.mode_set(mode='EDIT')
bm = bmesh.from_edit_mesh(ob.data)
for v in bm.verts:
    if v.co.z > 1.0:
        v.co.z += 0.5
bmesh.update_edit_mesh(ob.data)
bpy.ops.object.mode_set(mode='OBJECT')
