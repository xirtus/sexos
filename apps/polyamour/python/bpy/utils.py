# SPDX-License-Identifier: GPL-3.0-only
"""bpy.utils: class registration and paths."""
import os
import sys

from polyamour.rna import register_class, unregister_class  # noqa: F401
from polyamour import addons as _addons


def register_classes_factory(classes):
    def register():
        for c in classes:
            register_class(c)

    def unregister():
        for c in reversed(classes):
            unregister_class(c)
    return register, unregister


def register_submodule_factory(module_name, submodule_names):
    module = None
    submodules = []

    def register():
        nonlocal module
        module = __import__(name=module_name, fromlist=submodule_names)
        submodules[:] = [getattr(module, name) for name in submodule_names]
        for mod in submodules:
            mod.register()

    def unregister():
        for mod in reversed(submodules):
            mod.unregister()
    return register, unregister


def register_tool(tool_cls, *, after=None, separator=False, group=False):
    pass


def unregister_tool(tool_cls):
    pass


def register_manual_map(fn):
    pass


def unregister_manual_map(fn):
    pass


def register_preset_path(path):
    return False


def unregister_preset_path(path):
    return False


def _user_dir():
    base = os.environ.get('POLYAMOUR_USER_DIR') or os.path.join(os.path.expanduser('~'), '.polyamour')
    return base


def resource_path(type, major=4, minor=2):
    return os.path.join(_user_dir(), type.lower())


def user_resource(resource_type, path='', create=False):
    p = os.path.join(_user_dir(), resource_type.lower(), path)
    if create:
        os.makedirs(p, exist_ok=True)
    return p


def extension_path_user(package, path='', create=False):
    p = os.path.join(_user_dir(), 'extensions', package, path)
    if create:
        os.makedirs(p, exist_ok=True)
    return p


def script_path_user():
    return os.path.join(_user_dir(), 'scripts')


def script_path_pref():
    return None


def script_paths(subdir=None, user_pref=True, check_all=False, use_user=True):
    return [p for p in _addons.dirs]


def script_paths_pref():
    return []


def refresh_script_paths():
    pass


def app_template_paths(path=None):
    return []


def blend_paths(absolute=False, packed=False, local=False):
    return []


def escape_identifier(s):
    return s.replace('\\', '\\\\').replace('"', '\\"')


def unescape_identifier(s):
    return s.replace('\\"', '"').replace('\\\\', '\\')


def flip_name(name, strip_digits=False):
    for a, b in (('.L', '.R'), ('_L', '_R'), ('.l', '.r'), ('_l', '_r'), ('Left', 'Right'), ('left', 'right')):
        if name.endswith(a):
            return name[:-len(a)] + b
        if name.endswith(b):
            return name[:-len(b)] + a
    return name


def smpte_from_seconds(time, fps=None, fps_base=None):
    fps = fps or 24
    h, rem = divmod(float(time), 3600)
    m, s = divmod(rem, 60)
    f = int(round((s - int(s)) * fps))
    return '%02d:%02d:%02d:%02d' % (h, m, int(s), f)


def smpte_from_frame(frame, fps=None, fps_base=None):
    fps = fps or 24
    return smpte_from_seconds(frame / fps, fps)


def time_from_frame(frame, fps=None, fps_base=None):
    import datetime
    return datetime.timedelta(seconds=frame / (fps or 24))


def time_to_frame(time, fps=None, fps_base=None):
    secs = time.total_seconds() if hasattr(time, 'total_seconds') else float(time)
    return secs * (fps or 24)


def keyconfig_set(filepath, report=None):
    return False


def modules_from_path(path, loaded_modules):
    return []


def execfile(filepath, mod=None):
    import types
    m = types.ModuleType('__main__')
    with open(filepath, encoding='utf-8') as f:
        exec(compile(f.read(), filepath, 'exec'), m.__dict__)
    return m


def load_scripts(reload_scripts=False, refresh_scripts=False):
    pass


class _ImagePreview:
    def __init__(self, name):
        self.name = name
        self.icon_id = abs(hash(name)) % 100000 + 1
        self.image_size = (0, 0)
        self.icon_size = (0, 0)
        self.image_pixels = []
        self.icon_pixels = []


class ImagePreviewCollection(dict):
    def new(self, name):
        p = _ImagePreview(name)
        self[name] = p
        return p

    def load(self, name, filepath, filetype, force_reload=False):
        return self.new(name)

    def close(self):
        self.clear()


class _Previews:
    ImagePreviewCollection = ImagePreviewCollection

    @staticmethod
    def new():
        return ImagePreviewCollection()

    @staticmethod
    def remove(pcoll):
        pcoll.close()


previews = _Previews()


class _Units:
    systems = ('NONE', 'METRIC', 'IMPERIAL')
    categories = ('NONE', 'LENGTH', 'AREA', 'VOLUME', 'MASS', 'ROTATION', 'TIME', 'VELOCITY', 'ACCELERATION', 'CAMERA', 'POWER')

    @staticmethod
    def to_value(unit_system, unit_category, str_input, str_ref_unit=None):
        return float(str_input.split()[0])

    @staticmethod
    def to_string(unit_system, unit_category, value, precision=3, split_unit=False, compatible_unit=False):
        return ('%.' + str(precision) + 'f') % value


units = _Units()

sys.modules[__name__ + '.previews'] = previews
sys.modules[__name__ + '.units'] = units
