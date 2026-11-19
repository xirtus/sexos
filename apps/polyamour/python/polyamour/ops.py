# SPDX-License-Identifier: GPL-3.0-only
"""bpy.ops: operators by Blender's idnames. Operators a script registered run
here in Python; the rest are Polyamour's own (`op IDNAME options...`)."""
import inspect

from . import polyops as P
from . import rna
from ._base import ID, req

_builtin = [None]


def builtin_operators():
    if _builtin[0] is None:
        if P.get().live:
            _builtin[0] = set(req('op_list'))
        else:
            return set()
    return _builtin[0]


def _flatten(kw):
    """Blender keyword arguments -> polyops options."""
    out = {}
    for k, v in kw.items():
        if isinstance(v, dict):
            # Macro sub-operator settings (TRANSFORM_OT_translate={...}).
            for k2, v2 in v.items():
                out.setdefault(k2, v2)
            continue
        out[k] = v
    opts = {}
    for k, v in out.items():
        if v is None:
            continue
        if isinstance(v, ID):
            opts[k] = v.name
        elif isinstance(v, (set, frozenset)):
            opts[k] = P.Raw(','.join(sorted(str(x) for x in v)) or 'none')
        elif isinstance(v, (bool, int, float, str)):
            opts[k] = v
        elif hasattr(v, '__len__'):
            items = list(v)
            if items and all(isinstance(x, (int, float)) for x in items):
                opts[k] = [float(x) for x in items]
            elif items and all(hasattr(x, '__len__') and not isinstance(x, str) for x in items):
                flat = [c for row in items for c in row]
                if all(isinstance(x, (int, float)) for x in flat):
                    opts[k] = [float(x) for x in flat]
            # anything else (file lists...) is not passed on
    return opts


def _call_handler(fn, *args):
    """Call `fn` with as many of `args` as it takes."""
    try:
        sig = inspect.signature(fn)
        params = list(sig.parameters.values())
        if any(p.kind == p.VAR_POSITIONAL for p in params):
            return fn(*args)
        n = len([p for p in params if p.kind in (p.POSITIONAL_ONLY, p.POSITIONAL_OR_KEYWORD)])
        return fn(*args[:n])
    except (TypeError, ValueError):
        return fn(*args)


class _OpCall:
    def __init__(self, module, name):
        self._mod = module
        self._name = name

    @property
    def _id(self):
        return '%s.%s' % (self._mod, self._name)

    def idname(self):
        return rna.op_idnames(self._id)[1]

    def idname_py(self):
        return self._id

    def __repr__(self):
        cls = rna.operators.get(self._id)
        label = cls.bl_label if cls else self._name.replace('_', ' ').title()
        return '# %s\nbpy.ops.%s()' % (label, self._id)

    @property
    def bl_options(self):
        cls = rna.operators.get(self._id)
        return getattr(cls, 'bl_options', {'REGISTER', 'UNDO'})

    def get_rna_type(self):
        cls = rna.operators.get(self._id)
        from ._base import _RNA
        r = _RNA(self.idname())
        if cls is not None:
            r.properties = {k: v for k, v in rna._class_props(cls).items()}
            r.name = cls.bl_label
            r.description = cls.bl_description or (cls.__doc__ or '').strip()
        return r

    def _split(self, args):
        ctx = 'EXEC_DEFAULT'
        override = None
        for a in args:
            if isinstance(a, str):
                ctx = a
            elif isinstance(a, dict):
                override = a
        return ctx, override

    def poll(self, *args):
        ctx, override = self._split(args)
        from . import context
        cls = rna.operators.get(self._id)
        if cls is not None:
            p = getattr(cls, 'poll', None)
            return bool(p(context)) if p is not None else True
        if self._id not in builtin_operators():
            return False
        return P.get().request('op_poll', self._id) == ['1']

    def __call__(self, *args, **kw):
        ctx, override = self._split(args)
        from . import context
        if override:
            with context.temp_override(**override):
                return self._run(ctx, kw)
        return self._run(ctx, kw)

    def _run(self, ctx, kw):
        cls = rna.operators.get(self._id)
        if cls is not None:
            return run_python_operator(cls, ctx, kw)
        if P.get().live and self._id not in builtin_operators():
            raise AttributeError('Calling operator "bpy.ops.%s" error, could not be found' % self._id)
        P.get().flush()
        names = None
        if self._id.startswith('object.modifier_') and isinstance(kw.get('modifier'), str):
            # Modifier names are kept on this side: pass the index.
            from . import context
            from .types import _custom_names, _modifier_names
            a = context.active_object
            if a is not None:
                names = _modifier_names(a.name)
                if kw['modifier'] in names:
                    kw = dict(kw)
                    idx = names.index(kw['modifier'])
                    kw['modifier'] = str(idx)
                    names = (a.name, names, idx)
                else:
                    names = None
        try:
            r = req('op', self._id, **_flatten(kw))
        except P.PolyopsError as e:
            raise RuntimeError('Error: %s' % e) from None
        if names is not None:
            obj, lst, idx = names
            if self._id in ('object.modifier_apply', 'object.modifier_remove'):
                lst.pop(idx)
            elif self._id == 'object.modifier_move_up' and idx > 0:
                lst.insert(idx - 1, lst.pop(idx))
            elif self._id == 'object.modifier_move_down' and idx + 1 < len(lst):
                lst.insert(idx + 1, lst.pop(idx))
            elif self._id == 'object.modifier_move_to_index':
                lst.insert(int(kw.get('index', idx)), lst.pop(idx))
            _custom_names[obj] = lst
        if self._id == 'render.render':
            from . import app
            app._run_handlers('render_complete', None)
        return {r[0]} if r else {'FINISHED'}


def run_python_operator(cls, ctx, kw, event=None):
    from . import context
    op = cls()
    for k, v in kw.items():
        if k not in rna._class_props(cls):
            raise TypeError('Converting py args to operator properties: keyword "%s" unrecognized' % k)
        setattr(op, k, v)
    poll = getattr(cls, 'poll', None)
    if poll is not None and not poll(context):
        raise RuntimeError('Operator bpy.ops.%s.poll() failed, context is incorrect' % rna.op_idnames(cls.bl_idname)[0])
    if ctx.startswith('INVOKE') and hasattr(op, 'invoke'):
        r = op.invoke(context, event or rna.Event())
    elif hasattr(op, 'execute'):
        r = op.execute(context)
    elif hasattr(op, 'invoke'):
        r = op.invoke(context, event or rna.Event())
    else:
        r = {'CANCELLED'}
    if r is None or not isinstance(r, (set, frozenset)):
        raise ValueError('Python operator %s returned %r, expected a set like {"FINISHED"}' % (cls.__name__, r))
    return set(r)


class _OpModule:
    def __init__(self, name):
        self._name = name

    def __getattr__(self, name):
        if name.startswith('__'):
            raise AttributeError(name)
        return _OpCall(self._name, name)

    def __dir__(self):
        names = set()
        for i in list(builtin_operators()) + list(rna.operators):
            m, _, n = i.partition('.')
            if m == self._name:
                names.add(n)
        return sorted(names)

    def __repr__(self):
        return '<module like class \'bpy.ops.%s\'>' % self._name


class OpsRoot:
    def __getattr__(self, name):
        if name.startswith('__'):
            raise AttributeError(name)
        return _OpModule(name)

    def __dir__(self):
        return sorted(set(i.partition('.')[0] for i in list(builtin_operators()) + list(rna.operators)))
