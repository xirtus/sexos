# SPDX-License-Identifier: GPL-3.0-only
"""AddObjectHelper and object_data_add, as add-ons use them."""
from mathutils import Euler, Matrix, Vector
from polyamour.rna import BoolProperty, EnumProperty, FloatVectorProperty


class AddObjectHelper:
    align: EnumProperty(name='Align', items=(('WORLD', 'World', ''), ('VIEW', 'View', ''), ('CURSOR', '3D Cursor', '')), default='WORLD')
    location: FloatVectorProperty(name='Location', subtype='TRANSLATION')
    rotation: FloatVectorProperty(name='Rotation', subtype='EULER')
    scale: FloatVectorProperty(name='Scale', default=(1.0, 1.0, 1.0))


def object_add_grid_scale(context):
    return 1.0


def object_data_add(context, obdata, operator=None, name=None):
    import bpy
    obj = bpy.data.objects.new(name or getattr(obdata, 'name', 'Object'), obdata)
    if operator is not None:
        loc = Vector(getattr(operator, 'location', (0, 0, 0)))
        if getattr(operator, 'align', 'WORLD') == 'CURSOR':
            loc = loc + context.scene.cursor.location
        obj.location = loc
        rot = getattr(operator, 'rotation', None)
        if rot is not None:
            obj.rotation_euler = Euler(rot)
    else:
        obj.location = context.scene.cursor.location
    for o in context.selected_objects:
        o.select_set(False)
    obj.select_set(True)
    context.view_layer.objects.active = obj
    return obj


def world_to_camera_view(scene, obj, coord):
    cam = obj.matrix_world.normalized().inverted()
    co = cam @ Vector(coord)
    z = -co.z
    frame = obj.data.view_frame(scene=scene)
    if obj.data.type != 'ORTHO':
        frame = [-(v / (v.z / z)) for v in frame]
    min_x, max_x = frame[2].x, frame[1].x
    min_y, max_y = frame[1].y, frame[0].y
    x = (co.x - min_x) / (max_x - min_x) if max_x != min_x else 0.5
    y = (co.y - min_y) / (max_y - min_y) if max_y != min_y else 0.5
    return Vector((x, y, z))


_ = (BoolProperty, Matrix)
