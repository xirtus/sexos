# SPDX-License-Identifier: GPL-3.0-only
"""The Python kernel: the long-running process Polyamour starts to run
scripts, console lines, add-ons, panel draws and handlers (the other end of
crates/sex-3d/src/script/live.rs).

Protocol (one line each way, polyops quoting):

    host -> kernel   call ID KIND ARGS...
    kernel -> host   @N COMMAND ...          (a request; the host answers @N ok/err)
    kernel -> host   out TEXT | err TEXT     (what the code printed)
    kernel -> host   done ID ok [ARGS...] | done ID json {...} | done ID error MESSAGE line=N traceback=TEXT

Kinds: exec NAME SOURCE, console SOURCE, complete TEXT, draw PANEL [SPACE],
panels, menu MENU, operator IDNAME invoke|execute|poll PROPS-JSON,
modal EVENT-JSON, set_prop PANEL REF VALUE-JSON, handler NAME [ARGS],
frame FRAME, timers, addons [DIRS], addon_enable MODULE, addon_disable MODULE,
reset. `quit` ends the process.
"""
import codeop
import io
import json
import os
import re
import sys
import traceback

from . import polyops as P


class _Printer(io.TextIOBase):
    """sys.stdout / sys.stderr: each write becomes an `out`/`err` line."""

    def __init__(self, kernel, kind):
        self._k = kernel
        self._kind = kind

    def writable(self):
        return True

    def write(self, s):
        if s:
            self._k.send('%s %s' % (self._kind, P.quote(s)))
        return len(s)

    def flush(self):
        pass

    def isatty(self):
        return False

    @property
    def encoding(self):
        return 'utf-8'


class Kernel:
    def __init__(self, rfile, wfile):
        self._w = wfile
        self.channel = P.LiveChannel(rfile, wfile)
        P.channel = self.channel
        import polyamour
        self.pa = polyamour
        self.console_ns = None
        self.console_buf = []
        self.compiler = codeop.CommandCompiler()
        self.drawn = {}     # panel id -> refs of its last draw
        self.modal = []     # running modal operators

    def send(self, line):
        self._w.write(line + '\n')
        self._w.flush()

    # --- console
    def _console_namespace(self):
        if self.console_ns is None:
            import bpy
            import mathutils
            ns = {'__name__': '__console__', '__builtins__': __builtins__, 'bpy': bpy, 'C': bpy.context, 'D': bpy.data}
            for mod in (mathutils,):
                for k in dir(mod):
                    if not k.startswith('_'):
                        ns[k] = getattr(mod, k)
            import math
            for k in dir(math):
                if not k.startswith('_'):
                    ns.setdefault(k, getattr(math, k))
            self.console_ns = ns
        return self.console_ns

    def console(self, source):
        ns = self._console_namespace()
        self.console_buf.append(source)
        text = '\n'.join(self.console_buf)
        try:
            code = self.compiler(text, '<console>', 'single')
        except (SyntaxError, OverflowError, ValueError):
            self.console_buf = []
            self._print_exc(skip=0, console=True)
            return ['error']
        if code is None:
            return ['more']
        self.console_buf = []
        try:
            exec(code, ns)
        except SystemExit:
            pass
        except BaseException:
            self._print_exc(skip=1, console=True)
            return ['error']
        return ['ok']

    def _print_exc(self, skip=0, console=False):
        kind, value, tb = sys.exc_info()
        for _ in range(skip):
            if tb is not None:
                tb = tb.tb_next
        sys.stderr.write(''.join(traceback.format_exception(kind, value, tb)))

    def complete(self, text):
        ns = self._console_namespace()
        m = re.search(r"""([A-Za-z_][\w\.]*(?:\[[^\]]*\])*(?:\.[A-Za-z_]\w*)*)\[(['"])([^'"]*)$""", text)
        if m:
            base, quote, part = m.group(1), m.group(2), m.group(3)
            try:
                obj = eval(base, ns)
                keys = [k for k in obj.keys() if isinstance(k, str) and k.startswith(part)]
            except Exception:
                keys = []
            head = text[:m.start(3)]
            return {'prefix': part, 'completions': sorted(head + k + quote + ']' for k in keys)}
        m = re.search(r'([A-Za-z_][\w]*(?:(?:\.[A-Za-z_]\w*)|(?:\[[^\]]*\]))*)\.(\w*)$', text)
        out = []
        if m:
            base, part = m.group(1), m.group(2)
            try:
                obj = eval(base, ns)
                names = [n for n in dir(obj) if n.startswith(part) and (part.startswith('_') or not n.startswith('_'))]
            except Exception:
                names = []
            head = text[:m.start(2)]
            for n in sorted(set(names)):
                try:
                    v = getattr(obj, n)
                    suffix = '(' if callable(v) and not isinstance(v, type) else ''
                except Exception:
                    suffix = ''
                out.append(head + n + suffix)
            return {'prefix': part, 'completions': out}
        m = re.search(r'([A-Za-z_]\w*)$', text)
        if m:
            part = m.group(1)
            import builtins
            import keyword
            names = set(k for k in ns if k.startswith(part)) | set(k for k in dir(builtins) if k.startswith(part)) | \
                set(k for k in keyword.kwlist if k.startswith(part))
            head = text[:m.start(1)]
            return {'prefix': part, 'completions': sorted(head + n for n in names if not n.startswith('_') or part.startswith('_'))}
        return {'prefix': '', 'completions': []}

    # --- scripts
    def run_exec(self, name, source):
        ns = {'__name__': '__main__', '__file__': name, '__builtins__': __builtins__}
        try:
            code = compile(source, name, 'exec')
        except SyntaxError as e:
            return self._error(e, name, syntax=True)
        try:
            exec(code, ns)
        except SystemExit as e:
            if e.code not in (None, 0):
                return self._error(e, name)
        except BaseException as e:
            return self._error(e, name)
        return None

    def _error(self, e, name, syntax=False):
        kind, value, tb = sys.exc_info()
        line = None
        if syntax:
            line = getattr(e, 'lineno', None)
        else:
            t = tb
            while t is not None:
                if t.tb_frame.f_code.co_filename == name:
                    line = t.tb_lineno
                t = t.tb_next
        if tb is not None and tb.tb_frame.f_code.co_filename == __file__:
            tb = tb.tb_next
        text = ''.join(traceback.format_exception(kind, value, tb))
        sys.stderr.write(text)
        msg = '%s: %s' % (kind.__name__, value) if str(value) else kind.__name__
        if isinstance(e, P.PolyopsError):
            msg = str(value)
        return msg, line, text

    # --- UI
    def draw(self, panel_id, space=None):
        from . import rna
        cls = rna.panels.get(panel_id) or rna.menus.get(panel_id) or rna.headers.get(panel_id)
        if cls is None:
            raise KeyError("no panel, menu or header '%s' is registered" % panel_id)
        rna.set_current_area(space or getattr(cls, 'bl_space_type', 'VIEW_3D'))
        try:
            ctx = self.pa.context
            poll = getattr(cls, 'poll', None)
            if poll is not None and not poll(ctx):
                return {'id': panel_id, 'poll': False}
            inst = cls()
            refs = rna._Refs()
            inst.__dict__['layout'] = rna.UILayout('column', refs)
            out = {'id': panel_id, 'poll': True, 'label': getattr(cls, 'bl_label', '')}
            if hasattr(inst, 'draw_header'):
                hl = rna.UILayout('row', refs)
                inst.__dict__['layout'] = hl
                inst.draw_header(ctx)
                out['header'] = hl.to_json()
                inst.__dict__['layout'] = rna.UILayout('column', refs)
            body = inst.__dict__['layout']
            if hasattr(inst, 'draw'):
                inst.draw(ctx)
            for fn in (type(inst).__dict__.get('_extra_draw') or []):
                fn(inst, ctx)
            out['layout'] = body.to_json()
            if 'Panel' in [c.__name__ for c in type(inst).__mro__] and inst.__dict__.get('_text'):
                out['label'] = inst.__dict__['_text']
            self.drawn[panel_id] = refs
            return out
        finally:
            rna.set_current_area(None)

    def menu(self, menu_id):
        """A built-in menu's add-on entries (Menu.append), or a registered menu."""
        from . import rna
        from . import _bpy_types_registry
        import bpy
        cls = rna.menus.get(menu_id) or _bpy_types_registry.get(menu_id) or getattr(bpy.types, menu_id, None)
        if cls is None:
            return {'id': menu_id, 'layout': {'type': 'column', 'items': []}}
        inst = cls()
        refs = rna._Refs()
        inst.__dict__['layout'] = rna.UILayout('column', refs)
        ctx = self.pa.context
        if menu_id in rna.menus and hasattr(inst, 'draw'):
            inst.draw(ctx)
        for fn in (cls.__dict__.get('_extra_draw') or []):
            fn(inst, ctx)
        self.drawn[menu_id] = refs
        return {'id': menu_id, 'label': getattr(cls, 'bl_label', ''), 'layout': inst.layout.to_json()}

    def panels(self):
        from . import rna
        out = []
        for pid, cls in rna.panels.items():
            out.append({'id': pid, 'label': cls.bl_label, 'space': cls.bl_space_type, 'region': cls.bl_region_type,
                        'category': cls.bl_category, 'context': cls.bl_context, 'parent': cls.bl_parent_id,
                        'order': getattr(cls, 'bl_order', 0), 'options': sorted(getattr(cls, 'bl_options', set()))})
        ops = []
        for oid, cls in rna.operators.items():
            ops.append({'id': oid, 'label': cls.bl_label or oid, 'description': (cls.__doc__ or cls.bl_description or '').strip(),
                        'options': sorted(getattr(cls, 'bl_options', set()))})
        menus = [{'id': mid, 'label': cls.bl_label} for mid, cls in rna.menus.items()]
        import bpy
        extended = [name for name, cls in vars(bpy.types).items() if isinstance(cls, type) and cls.__dict__.get('_extra_draw')]
        stubs = getattr(sys.modules.get('bpy.types'), '_stubs', {})
        for name, cls in list(self.pa._bpy_types_registry.items()) + list(stubs.items()):
            if isinstance(cls, type) and cls.__dict__.get('_extra_draw') and name not in extended:
                extended.append(name)
        return {'panels': out, 'operators': ops, 'menus': menus, 'extended_menus': sorted(extended),
                'keymaps': [list(k[:5]) + [k[5]] for k in rna.keymaps]}

    def set_prop(self, panel_id, ref, value):
        refs = self.drawn.get(panel_id)
        if refs is None:
            raise KeyError("panel '%s' was not drawn" % panel_id)
        data, prop = refs.items[int(ref)]
        cur = getattr(data, prop)
        if isinstance(value, str) and isinstance(cur, (int, float)) and not isinstance(cur, bool):
            value = type(cur)(value)
        if isinstance(value, list) and isinstance(cur, set):
            value = set(value)
        if isinstance(value, str) and hasattr(cur, 'name') and not isinstance(cur, str):
            # a pointer chosen by name
            from . import data as D
            for coll in (D.objects, D.materials, D.meshes, D.collections, D.images, D.node_groups, D.actions):
                if value in coll:
                    value = coll[value]
                    break
        setattr(data, prop, value)

    def operator(self, idname, mode, props):
        from . import rna
        ops = sys.modules["polyamour.ops"]
        py, _ = rna.op_idnames(idname)
        cls = rna.operators.get(py)
        mod, _, name = py.partition('.')
        call = ops._OpCall(mod, name)
        if mode == 'poll':
            return ['1' if call.poll() else '0']
        if cls is None:
            r = call('INVOKE_DEFAULT' if mode == 'invoke' else 'EXEC_DEFAULT', **props)
            return sorted(r)
        wm = rna.window_manager
        before = list(wm.modal_ops)
        r = ops.run_python_operator(cls, 'INVOKE_DEFAULT' if mode == 'invoke' else 'EXEC_DEFAULT', props)
        if 'RUNNING_MODAL' in r:
            for op in wm.modal_ops:
                if op not in before:
                    self.modal.append(op)
        return sorted(r)

    def modal_event(self, ev):
        """Send one event to the running modal operators; answers how many still run."""
        from . import rna
        event = rna.Event(ev.get('type', 'TIMER'), ev.get('value', 'NOTHING'), **{k: v for k, v in ev.items() if k not in ('type', 'value')})
        still = []
        for op in self.modal:
            try:
                r = op.modal(self.pa.context, event)
            except Exception:
                traceback.print_exc()
                r = {'CANCELLED'}
            if 'RUNNING_MODAL' in r or 'PASS_THROUGH' in r and 'FINISHED' not in r and 'CANCELLED' not in r:
                still.append(op)
            elif op in rna.window_manager.modal_ops:
                rna.window_manager.modal_ops.remove(op)
        self.modal = still
        timers = rna.window_manager.timers
        return {'running': len(still), 'timer': min((t.time_step for t in timers), default=None) if still else None}

    # --- the loop
    def handle(self, line):
        toks = P.tokenize(line)
        if not toks:
            return True
        if toks[0][1] == 'quit':
            return False
        if toks[0][1] != 'call' or len(toks) < 3:
            return True
        cid, kind = toks[1][1], toks[2][1]
        args = [t[1] for t in toks[3:]]
        gen0 = self.channel.generation
        result = None
        error = None
        try:
            result = self.dispatch(kind, args)
        except SystemExit:
            raise
        except BaseException as e:
            error = self._error(e, '<kernel>')
        if error is None and isinstance(result, tuple) and len(result) == 3 and kind == 'exec':
            error, result = result, None
        try:
            self.channel.flush()
            if error is None and self.channel.generation != gen0 and kind in ('exec', 'console', 'operator', 'set_prop', 'addon_enable', 'modal'):
                from . import app
                app._run_handlers('depsgraph_update_post', self.pa.context.scene, self.pa.context.evaluated_depsgraph_get())
                self.channel.flush()
        except BaseException as e:
            if error is None:
                error = self._error(e, '<kernel>')
        if error is not None:
            msg, ln, tb = error
            parts = ['done', cid, 'error', P.quote(msg)]
            if ln:
                parts.append('line=%d' % ln)
            parts.append('traceback=' + P.quote(tb))
            self.send(' '.join(parts))
        elif isinstance(result, (dict, list)) and not (isinstance(result, list) and all(isinstance(x, str) for x in result)):
            self.send('done %s json %s' % (cid, json.dumps(result)))
        else:
            self.send(' '.join(['done', cid, 'ok'] + [P.quote(x) for x in (result or [])]))
        return True

    def dispatch(self, kind, args):
        from . import addons, app, rna
        if kind == 'exec':
            return self.run_exec(args[0], args[1])
        if kind == 'console':
            return self.console(args[0])
        if kind == 'complete':
            return self.complete(args[0])
        if kind == 'draw':
            return self.draw(args[0], args[1] if len(args) > 1 else None)
        if kind == 'menu':
            return self.menu(args[0])
        if kind == 'panels':
            return self.panels()
        if kind == 'operator':
            props = json.loads(args[2]) if len(args) > 2 and args[2] else {}
            return self.operator(args[0], args[1], props)
        if kind == 'modal':
            return self.modal_event(json.loads(args[0]) if args else {})
        if kind == 'set_prop':
            self.set_prop(args[0], args[1], json.loads(args[2]))
            return []
        if kind == 'handler':
            name = args[0]
            ctx = self.pa.context
            if name.startswith('load') or name.startswith('save'):
                app._run_handlers(name, args[1] if len(args) > 1 else '')
            else:
                app._run_handlers(name, ctx.scene, ctx.evaluated_depsgraph_get())
            return []
        if kind == 'frame':
            app._frame_change(self.pa.context.scene, int(float(args[0])), float(args[1]) if len(args) > 1 else 0.0)
            return []
        if kind == 'timers':
            nxt = app.timers.run_due()
            return {'next': nxt, 'modal': len(self.modal)}
        if kind == 'addons':
            if args:
                addons.set_dirs([p for a in args for p in a.split(os.pathsep)])
            return {'addons': addons.scan(), 'enabled': list(addons.enabled)}
        if kind == 'addon_enable':
            addons.enable(args[0])
            return []
        if kind == 'addon_disable':
            addons.disable(args[0])
            return []
        if kind == 'reset':
            addons.disable_all()
            rna.reset_registry()
            app.clear_non_persistent_handlers()
            app.timers.clear()
            self.console_ns = None
            self.console_buf = []
            self.modal = []
            return []
        if kind == 'ping':
            return ['pong']
        raise ValueError("unknown call kind '%s'" % kind)

    def loop(self, rfile):
        while True:
            line = rfile.readline()
            if not line:
                return
            if not self.handle(line.rstrip('\r\n')):
                return


def main():
    # The protocol owns the original standard input and output; the code the
    # kernel runs gets an empty stdin and prints through `out`/`err` lines.
    wfd = os.dup(1)
    rfd = os.dup(0)
    os.dup2(2, 1)
    try:
        nul = os.open(os.devnull, os.O_RDONLY)
        os.dup2(nul, 0)
        os.close(nul)
    except OSError:
        pass
    wfile = os.fdopen(wfd, 'w', encoding='utf-8', newline='\n')
    rfile = os.fdopen(rfd, 'r', encoding='utf-8', newline='\n')
    k = Kernel(rfile, wfile)
    sys.stdout = _Printer(k, 'out')
    sys.stderr = _Printer(k, 'err')
    from . import addons
    env = os.environ.get('POLYAMOUR_ADDONS')
    if env:
        addons.set_dirs(env.split(os.pathsep))
    k.send('hello polyops %d python=%d.%d.%d' % ((P.VERSION,) + tuple(sys.version_info[:3])))
    k.loop(rfile)
