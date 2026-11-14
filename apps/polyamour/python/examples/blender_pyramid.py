# SPDX-License-Identifier: GPL-3.0-only
"""A plain Blender script: nothing here is Polyamour-specific, it runs the same
in Blender. Deletes the default cube, builds a golden pyramid with from_pydata
and a Principled BSDF, adds a smooth sphere with a Subdivision Surface modifier
and a mirrored copy of it, a sun lamp, and sets the render size."""
import math

import bpy
from mathutils import Vector

# Start from the default scene, minus the cube.
cube = bpy.data.objects.get('Cube')
if cube is not None:
    bpy.data.objects.remove(cube, do_unlink=True)

verts = [(-1, -1, 0), (1, -1, 0), (1, 1, 0), (-1, 1, 0), (0, 0, 1.5)]
faces = [(3, 2, 1, 0), (0, 1, 4), (1, 2, 4), (2, 3, 4), (3, 0, 4)]
mesh = bpy.data.meshes.new('Pyramid')
mesh.from_pydata(verts, [], faces)
mesh.update()
pyramid = bpy.data.objects.new('Pyramid', mesh)
bpy.context.collection.objects.link(pyramid)

gold = bpy.data.materials.new(name='Gold')
gold.use_nodes = True
bsdf = gold.node_tree.nodes['Principled BSDF']
bsdf.inputs['Base Color'].default_value = (1.0, 0.77, 0.34, 1.0)
bsdf.inputs['Metallic'].default_value = 1.0
bsdf.inputs['Roughness'].default_value = 0.25
pyramid.data.materials.append(gold)

bpy.ops.object.select_all(action='DESELECT')
bpy.ops.mesh.primitive_uv_sphere_add(radius=0.5, location=(3, 0, 0.5))
ball = bpy.context.active_object
ball.name = 'Ball'
mod = ball.modifiers.new(name='Subdivision', type='SUBSURF')
mod.levels = 2
bpy.ops.object.shade_smooth()
mirror = ball.modifiers.new(name='Mirror', type='MIRROR')
mirror.use_axis[0] = False
mirror.use_axis[1] = True
ball.location += Vector((0, 1, 0))

sun_data = bpy.data.lights.new(name='Sun', type='SUN')
sun_data.energy = 3.0
sun = bpy.data.objects.new(name='Sun', object_data=sun_data)
bpy.context.collection.objects.link(sun)
sun.rotation_euler = (math.radians(45), 0, math.radians(30))

scene = bpy.context.scene
scene.render.resolution_x = 1280
scene.render.resolution_y = 720
scene.render.engine = 'CYCLES'
bpy.context.view_layer.objects.active = pyramid
