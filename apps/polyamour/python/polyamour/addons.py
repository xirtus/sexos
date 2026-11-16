# SPDX-License-Identifier: GPL-3.0-only
"""Add-ons: Python modules or packages with `bl_info` (or a
blender_manifest.toml) and register()/unregister(), found in the add-on
folders Polyamour passes in (POLYAMOUR_ADDONS, os.pathsep separated, or the
`addons DIR` kernel call)."""
import ast
import importlib
import os
import sys
import traceback

dirs = []
enabled = []      # module names, in enabling order
_info = {}        # module name -> info dict


def set_dirs(paths):
    dirs[:] = [p for p in paths if p]
    for p in dirs:
        if p not in sys.path:
            sys.path.append(p)


def _read_bl_info(path):
    try:
        with open(path, encoding='utf-8') as f:
            tree = ast.parse(f.read(), path)
    except (OSError, SyntaxError, ValueError):
        return None
    for node in tree.body:
        if isinstance(node, ast.Assign) and any(isinstance(t, ast.Name) and t.id == 'bl_info' for t in node.targets):
            try:
                return ast.literal_eval(node.value)
            except ValueError:
                return None
    return None


def _read_manifest(path):
    info = {}
    try:
        with open(path, encoding='utf-8') as f:
            for line in f:
                line = line.split('#', 1)[0].strip()
                if '=' not in line or line.startswith('['):
                    continue
                k, _, v = line.partition('=')
                v = v.strip()
                if v.startswith('"') and v.endswith('"'):
                    v = v[1:-1]
                info[k.strip()] = v
    except OSError:
        return None
    return {'name': info.get('name', info.get('id', '')), 'description': info.get('tagline', ''),
            'author': info.get('maintainer', ''), 'version': tuple(int(x) for x in info.get('version', '0').split('.') if x.isdigit()),
            'category': 'Extension', 'blender': (4, 2, 0)}


def scan():
    """Every add-on in the folders: [{module, name, description, ...}]."""
    found = []
    seen = set()
    for d in dirs:
        try:
            names = sorted(os.listdir(d))
        except OSError:
            continue
        for n in names:
            full = os.path.join(d, n)
            info = None
            if n.endswith('.py') and os.path.isfile(full):
                mod = n[:-3]
                info = _read_bl_info(full)
            elif os.path.isdir(full) and os.path.isfile(os.path.join(full, '__init__.py')):
                mod = n
                info = _read_bl_info(os.path.join(full, '__init__.py'))
                if info is None and os.path.isfile(os.path.join(full, 'blender_manifest.toml')):
                    info = _read_manifest(os.path.join(full, 'blender_manifest.toml'))
            else:
                continue
            if info is None or mod in seen or not mod.isidentifier():
                continue
            seen.add(mod)
            info = dict(info)
            _info[mod] = info
            found.append({'module': mod, 'name': str(info.get('name', mod)), 'description': str(info.get('description', '')),
                          'author': str(info.get('author', '')), 'category': str(info.get('category', '')),
                          'version': '.'.join(str(x) for x in info.get('version', ())),
                          'location': str(info.get('location', '')), 'warning': str(info.get('warning', '')),
                          'path': full, 'enabled': mod in enabled})
    return found


def enable(module):
    if module in enabled:
        return sys.modules.get(module)
    mod = importlib.import_module(module)
    reg = getattr(mod, 'register', None)
    if reg is None:
        raise ImportError("add-on '%s' has no register()" % module)
    reg()
    enabled.append(module)
    return mod


def disable(module):
    mod = sys.modules.get(module)
    if module in enabled:
        enabled.remove(module)
    if mod is None:
        return
    try:
        unreg = getattr(mod, 'unregister', None)
        if unreg is not None:
            unreg()
    finally:
        for name in list(sys.modules):
            if name == module or name.startswith(module + '.'):
                del sys.modules[name]


def disable_all():
    for m in list(reversed(enabled)):
        try:
            disable(m)
        except Exception:
            traceback.print_exc()


def module_bl_info(mod, info_basis=None):
    return _info.get(getattr(mod, '__name__', mod), {})
