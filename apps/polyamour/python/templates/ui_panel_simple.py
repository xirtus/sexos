# SPDX-License-Identifier: GPL-3.0-only
# A panel: run this script, then open the 3D View sidebar (N) and its "Tool Kit" tab.
import bpy
from bpy.props import FloatProperty, IntProperty, EnumProperty, PointerProperty


class ToolKitSettings(bpy.types.PropertyGroup):
    count: IntProperty(name="Count", default=5, min=1, max=100)
    radius: FloatProperty(name="Radius", default=3.0, min=0.1)
    shape: EnumProperty(name="Shape", items=[('CUBE', "Cube", ""), ('SPHERE', "Sphere", "")], default='CUBE')


class OBJECT_OT_ring(bpy.types.Operator):
    """Add objects on a ring around the 3D cursor"""
    bl_idname = "object.tool_kit_ring"
    bl_label = "Add Ring"
    bl_options = {'REGISTER', 'UNDO'}

    def execute(self, context):
        import math
        kit = context.scene.tool_kit
        c = context.scene.cursor.location
        for i in range(kit.count):
            a = 2 * math.pi * i / kit.count
            loc = (c.x + kit.radius * math.cos(a), c.y + kit.radius * math.sin(a), c.z)
            if kit.shape == 'CUBE':
                bpy.ops.mesh.primitive_cube_add(size=0.5, location=loc)
            else:
                bpy.ops.mesh.primitive_uv_sphere_add(radius=0.3, location=loc)
        return {'FINISHED'}


class VIEW3D_PT_tool_kit(bpy.types.Panel):
    bl_label = "Ring"
    bl_space_type = 'VIEW_3D'
    bl_region_type = 'UI'
    bl_category = "Tool Kit"

    def draw(self, context):
        layout = self.layout
        kit = context.scene.tool_kit
        col = layout.column(align=True)
        col.prop(kit, "count")
        col.prop(kit, "radius")
        layout.prop(kit, "shape")
        layout.operator("object.tool_kit_ring")
        ob = context.active_object
        layout.label(text="Active: %s" % (ob.name if ob else "none"))


class OBJECT_PT_tool_kit(bpy.types.Panel):
    bl_label = "Tool Kit"
    bl_space_type = 'PROPERTIES'
    bl_region_type = 'WINDOW'
    bl_context = "object"

    def draw(self, context):
        ob = context.object
        row = self.layout.row()
        row.label(text="Name: " + ob.name)
        row = self.layout.row()
        row.prop(ob, "location")


classes = (ToolKitSettings, OBJECT_OT_ring, VIEW3D_PT_tool_kit, OBJECT_PT_tool_kit)


def register():
    for cls in classes:
        bpy.utils.register_class(cls)
    bpy.types.Scene.tool_kit = PointerProperty(type=ToolKitSettings)


def unregister():
    del bpy.types.Scene.tool_kit
    for cls in reversed(classes):
        bpy.utils.unregister_class(cls)


if __name__ == "__main__":
    register()
