# SPDX-License-Identifier: GPL-3.0-only
"""bpy.app: version, handlers and timers."""
import sys
import time
import traceback

from . import polyops as P

version = (4, 2, 0)
version_string = '4.2.0 (Polyamour)'
version_file = (4, 2, 0)
version_cycle = 'release'
version_char = ''
binary_path = ''
binary_path_python = sys.executable
background = False
factory_startup = False
debug = False
debug_python = False
debug_value = 0
debug_wm = False
debug_depsgraph = False
debug_events = False
debug_ffmpeg = False
debug_handlers = False
build_branch = b'main'
build_platform = sys.platform.encode()
build_type = b'Release'
build_hash = b'polyamour'
build_date = b''
build_time = b''
build_options = type('build_options', (), {'cycles': True, 'opensubdiv': False, 'openvdb': False, 'fluid': False,
                                           'alembic': False, 'usd': False, 'ffmpeg': False, 'freestyle': False})()
online_access = False
online_access_override = False
autoexec_fail = False
autoexec_fail_message = ''
render_icon_size = 32
render_preview_size = 128
tempdir = '/tmp/'
driver_namespace = {}
use_userpref_skip_save_on_exit = False
use_event_simulate = False
alembic = usd = ocio = oiio = opensubdiv = openvdb = sdl = ffmpeg = type('lib', (), {'supported': False, 'version': (0, 0, 0),
                                                                                    'version_string': ''})()


def is_job_running(job_type):
    return False


def help_text(all=False):
    return 'Polyamour'


# --- handlers ------------------------------------------------------------------------------------

class _HandlerList(list):
    pass


class _Handlers:
    _names = ('frame_change_pre', 'frame_change_post', 'depsgraph_update_pre', 'depsgraph_update_post', 'load_pre', 'load_post',
              'load_post_fail', 'load_factory_preferences_post', 'load_factory_startup_post', 'save_pre', 'save_post',
              'save_post_fail', 'render_init', 'render_pre', 'render_post', 'render_stats', 'render_write', 'render_complete',
              'render_cancel', 'undo_pre', 'undo_post', 'redo_pre', 'redo_post', 'version_update', 'annotation_pre',
              'annotation_post', 'composite_pre', 'composite_post', 'composite_cancel', 'object_bake_pre',
              'object_bake_complete', 'object_bake_cancel', 'animation_playback_pre', 'animation_playback_post',
              'translation_update_post', 'xr_session_start_pre', 'blend_import_pre', 'blend_import_post')

    def __init__(self):
        for n in self._names:
            setattr(self, n, _HandlerList())

    @staticmethod
    def persistent(fn):
        fn._bpy_persistent = True
        return fn

    def __iter__(self):
        return (getattr(self, n) for n in self._names)


handlers = _Handlers()

_running = set()


def _run_handlers(name, *args):
    """Run bpy.app.handlers.NAME; errors are printed, not raised (like Blender)."""
    if name in _running:
        return
    lst = getattr(handlers, name, None)
    if not lst:
        return
    from .ops import _call_handler
    _running.add(name)
    try:
        for fn in list(lst):
            try:
                _call_handler(fn, *args)
            except Exception:
                traceback.print_exc()
    finally:
        _running.discard(name)


def _frame_change(scene, frame, subframe=0.0):
    from . import context
    dg = context.evaluated_depsgraph_get()
    _run_handlers('frame_change_pre', scene, dg)
    P.get().request('frame_set', frame, subframe)
    _run_handlers('frame_change_post', scene, dg)


def clear_non_persistent_handlers():
    for lst in handlers:
        lst[:] = [f for f in lst if getattr(f, '_bpy_persistent', False)]


# --- timers ----------------------------------------------------------------------------------------

class _Timers:
    def __init__(self):
        self._t = []  # [due time, function, persistent]

    def register(self, function, first_interval=0.0, persistent=False):
        self.unregister_silent(function)
        self._t.append([time.monotonic() + float(first_interval or 0.0), function, persistent])

    def unregister(self, function):
        if not self.is_registered(function):
            raise ValueError('Error: function is not registered')
        self.unregister_silent(function)

    def unregister_silent(self, function):
        self._t = [t for t in self._t if t[1] is not function]

    def is_registered(self, function):
        return any(t[1] is function for t in self._t)

    def run_due(self):
        """Run every due timer; returns seconds until the next one (or None)."""
        now = time.monotonic()
        for entry in list(self._t):
            if entry[0] <= now and entry in self._t:
                try:
                    r = entry[1]()
                except Exception:
                    traceback.print_exc()
                    r = None
                if r is None:
                    if entry in self._t:
                        self._t.remove(entry)
                else:
                    entry[0] = time.monotonic() + max(0.0, float(r))
        if not self._t:
            return None
        return max(0.0, min(t[0] for t in self._t) - time.monotonic())

    def clear(self, keep_persistent=False):
        self._t = [t for t in self._t if keep_persistent and t[2]]


timers = _Timers()


class _Translations:
    locale = 'en_US'
    locales = ('en_US',)

    @staticmethod
    def pgettext(msgid, msgctxt=None):
        return msgid

    pgettext_iface = pgettext_tip = pgettext_rpt = pgettext_data = pgettext_n = pgettext

    @staticmethod
    def register(module_name, translations_dict):
        pass

    @staticmethod
    def unregister(module_name):
        pass

    @staticmethod
    def locale_explode(locale):
        return (locale, None, None, locale, locale)

    contexts = type('contexts', (), {'default': '*', 'operator_default': 'Operator', 'id_windowmanager': 'WindowManager'})()


translations = _Translations()


class _Icons:
    def new_triangles(self, range, coords, colors):
        return 0

    def new_triangles_from_file(self, filepath):
        return 0

    def release(self, icon_id):
        pass


icons = _Icons()
