# SPDX-License-Identifier: GPL-3.0-only
"""ImportHelper / ExportHelper mix-ins and axis conversion."""
import os

from mathutils import Matrix, Vector
from polyamour.rna import BoolProperty, EnumProperty, StringProperty


class ImportHelper:
    filepath: StringProperty(name='File Path', description='Filepath used for importing the file', maxlen=1024, subtype='FILE_PATH')
    filter_glob: StringProperty(default='*', options={'HIDDEN'})

    def invoke(self, context, _event):
        context.window_manager.fileselect_add(self)
        return {'RUNNING_MODAL'}

    def check(self, _context):
        return False


class ExportHelper:
    filepath: StringProperty(name='File Path', description='Filepath used for exporting the file', maxlen=1024, subtype='FILE_PATH')
    check_existing: BoolProperty(name='Check Existing', default=True, options={'HIDDEN'})
    filename_ext = ''

    def invoke(self, context, _event):
        if not self.filepath:
            self.filepath = 'untitled' + self.filename_ext
        context.window_manager.fileselect_add(self)
        return {'RUNNING_MODAL'}

    def check(self, _context):
        if self.filename_ext and not self.filepath.endswith(self.filename_ext):
            self.filepath = os.path.splitext(self.filepath)[0] + self.filename_ext
            return True
        return False


_AXES = (('X', 'X', ''), ('Y', 'Y', ''), ('Z', 'Z', ''), ('-X', '-X', ''), ('-Y', '-Y', ''), ('-Z', '-Z', ''))


def orientation_helper(axis_forward='Y', axis_up='Z'):
    def wrapper(cls):
        ann = cls.__dict__.get('__annotations__')
        if ann is None:
            ann = {}
            cls.__annotations__ = ann
        ann['axis_forward'] = EnumProperty(name='Forward', items=_AXES, default=axis_forward)
        ann['axis_up'] = EnumProperty(name='Up', items=_AXES, default=axis_up)
        return cls
    return wrapper


def _axis_vec(a):
    v = {'X': (1, 0, 0), 'Y': (0, 1, 0), 'Z': (0, 0, 1)}[a.lstrip('-')]
    return Vector(v) * (-1 if a.startswith('-') else 1)


def axis_conversion(from_forward='Y', from_up='Z', to_forward='Y', to_up='Z'):
    def basis(fw, up):
        f, u = _axis_vec(fw), _axis_vec(up)
        r = f.cross(u)
        return Matrix(((r[0], f[0], u[0]), (r[1], f[1], u[1]), (r[2], f[2], u[2])))
    return basis(to_forward, to_up) @ basis(from_forward, from_up).inverted()


def unpack_list(list_of_tuples):
    return [x for t in list_of_tuples for x in t]


def unpack_face_list(list_of_tuples):
    return unpack_list(list_of_tuples)


def poll_file_object_drop(context):
    return True
