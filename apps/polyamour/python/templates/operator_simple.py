# SPDX-License-Identifier: GPL-3.0-only
# An operator: run this script, then press F3 and search "Simple Object Operator".
import bpy


def main(context):
    for ob in context.scene.objects:
        print(ob.name, tuple(round(v, 3) for v in ob.location))


class SimpleOperator(bpy.types.Operator):
    """Print the name and location of every object"""
    bl_idname = "object.simple_operator"
    bl_label = "Simple Object Operator"
    bl_options = {'REGISTER', 'UNDO'}

    @classmethod
    def poll(cls, context):
        return context.active_object is not None

    def execute(self, context):
        main(context)
        self.report({'INFO'}, "Listed %d objects" % len(context.scene.objects))
        return {'FINISHED'}


def menu_func(self, context):
    self.layout.operator(SimpleOperator.bl_idname, text=SimpleOperator.bl_label)


def register():
    bpy.utils.register_class(SimpleOperator)
    bpy.types.VIEW3D_MT_object.append(menu_func)


def unregister():
    bpy.types.VIEW3D_MT_object.remove(menu_func)
    bpy.utils.unregister_class(SimpleOperator)


if __name__ == "__main__":
    register()
    # Call it right away as well:
    bpy.ops.object.simple_operator()
