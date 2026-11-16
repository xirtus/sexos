# SPDX-License-Identifier: GPL-3.0-only
"""bpy.path: Blender-style path helpers (`//` means the scene file's folder,
the working directory when there is no file)."""
import os
import re
import string


def _base(start=None):
    return start if start is not None else os.getcwd()


def abspath(path, start=None, library=None):
    if isinstance(path, bytes):
        path = path.decode()
    if path.startswith('//'):
        return os.path.join(_base(start), path[2:])
    return path


def relpath(path, start=None):
    if path.startswith('//'):
        return path
    try:
        return '//' + os.path.relpath(path, _base(start))
    except ValueError:
        return path


def is_subdir(path, directory):
    path = os.path.normpath(os.path.abspath(path))
    directory = os.path.normpath(os.path.abspath(directory))
    return path == directory or path.startswith(directory + os.sep)


def basename(path):
    return os.path.basename(path[2:] if path.startswith('//') else path)


def clean_name(name, replace='_'):
    ok = set(string.ascii_letters + string.digits + '_-.')
    return ''.join(c if c in ok else replace for c in name)


def display_name(name, has_ext=True, title_case=True):
    if has_ext:
        name = os.path.splitext(basename(name))[0]
    name = name.replace('_', ' ')
    if title_case and name.lower() == name:
        name = name.title()
    return name


def display_name_from_filepath(name):
    return os.path.splitext(basename(name))[0]


def display_name_to_filepath(name):
    for c in '\\/:*?"<>|':
        name = name.replace(c, '_')
    return name


def ensure_ext(filepath, ext, case_sensitive=False):
    if case_sensitive:
        return filepath if filepath.endswith(ext) else filepath + ext
    return filepath if filepath.lower().endswith(ext.lower()) else filepath + ext


def native_pathsep(path):
    return path.replace('/', os.sep)


def resolve_ncase(path):
    return path


def reduce_dirs(dirs):
    dirs = sorted(set(os.path.normpath(d) for d in dirs))
    out = []
    for d in dirs:
        if not any(is_subdir(d, o) for o in out):
            out.append(d)
    return out


def module_names(path, recursive=False, package=''):
    out = []
    try:
        for n in sorted(os.listdir(path)):
            full = os.path.join(path, n)
            if n.endswith('.py') and n != '__init__.py':
                out.append((package + n[:-3], full))
            elif os.path.isfile(os.path.join(full, '__init__.py')):
                out.append((package + n, os.path.join(full, '__init__.py')))
    except OSError:
        pass
    return out


extensions_image = {'.png', '.jpg', '.jpeg', '.bmp', '.tga', '.exr', '.hdr', '.tif', '.tiff', '.webp'}
extensions_movie = {'.mp4', '.mov', '.avi', '.mkv', '.webm'}
extensions_audio = {'.wav', '.mp3', '.ogg', '.flac'}

_ = re
