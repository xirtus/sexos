# SPDX-License-Identifier: GPL-3.0-only
"""A spiral staircase: 24 steps around a pole, three alternating materials, a
Subdivision modifier on every step, a helical handrail made with from_pydata,
and a camera and area light. Run inside Polyamour, or on any CPython 3.13:

    POLYAMOUR_OPS=stairs.polyops python spiral_staircase.py
"""
import math

import polyamour as pa

STEPS = 24
RISE = 0.2          # metres per step
TURN = math.tau / 12  # one full turn every 12 steps
RADIUS = 1.5

stairs = pa.data.collections.new('Staircase')
pa.context.scene.collection.children.link(stairs)

colours = [('Terracotta', (0.8, 0.3, 0.2, 1.0)), ('Sage', (0.45, 0.6, 0.45, 1.0)), ('Slate', (0.3, 0.35, 0.45, 1.0))]
materials = []
for name, rgba in colours:
    mat = pa.data.materials.new(name)
    mat.base_color = rgba
    mat.roughness = 0.35
    materials.append(mat)
brass = pa.data.materials.new('Brass')
brass.base_color = (0.9, 0.7, 0.3, 1.0)
brass.metallic = 1.0
brass.roughness = 0.2

height = STEPS * RISE + 0.5
pa.ops.mesh.primitive_cylinder_add(vertices=24, radius=0.15, depth=height, location=(0, 0, height / 2))
pole = pa.context.active_object
pole.name = 'Pole'
pole.data.materials.append(brass)
stairs.objects.link(pole)

for i in range(STEPS):
    angle = i * TURN
    # Parenting from Python keeps the child's local transform (as in Blender),
    # so the step is placed in the pole's space: the pole stands at z = height/2.
    pa.ops.mesh.primitive_cube_add(
        size=1.0,
        location=(RADIUS * math.cos(angle), RADIUS * math.sin(angle), i * RISE - height / 2),
        rotation=(0, 0, angle),
        scale=(1.2, 0.4, 0.08),
    )
    step = pa.context.active_object
    step.name = 'Step%02d' % i
    step.data.materials.append(materials[i % len(materials)])
    step.parent = pole
    smooth = step.modifiers.new('Rounded', type='SUBSURF')
    smooth.levels = 2
    stairs.objects.link(step)

# Handrail: a helix of loose edges, sent as a base64 array (it has > 64 points).
rail_points = []
for k in range(STEPS * 4 + 1):
    a = k * TURN / 4
    rail_points.append(((RADIUS + 0.55) * math.cos(a), (RADIUS + 0.55) * math.sin(a), k * RISE / 4 + 0.9))
rail_mesh = pa.data.meshes.new('Handrail')
rail_mesh.from_pydata(rail_points, [(k, k + 1) for k in range(len(rail_points) - 1)], [])
rail = pa.data.objects.new('Handrail', rail_mesh)
stairs.objects.link(rail)
rail.data.materials.append(brass)

# The top landing gets a name that needs quoting in polyops.
pa.ops.mesh.primitive_cylinder_add(vertices=32, radius=RADIUS + 0.7, depth=0.1, location=(0, 0, STEPS * RISE + 0.1))
landing = pa.context.active_object
landing.name = 'Top Landing "A"'
landing.active_material = brass

pa.ops.object.light_add(type='AREA', location=(0, 0, STEPS * RISE + 3))
lamp = pa.context.active_object
lamp.data.energy = 500
lamp.data.size = 3.0

pa.ops.object.camera_add(location=(9, -9, 6), rotation=(math.radians(65), 0, math.radians(45)))
pa.context.scene.camera = pa.context.active_object

pa.context.scene.world.color = (0.02, 0.02, 0.05)
pa.context.scene.render.resolution_x = 1280
pa.context.scene.render.resolution_y = 720
pa.context.scene.frame_end = 48

print('staircase: %d objects' % len(pa.data.objects))
