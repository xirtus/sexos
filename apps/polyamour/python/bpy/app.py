# SPDX-License-Identifier: GPL-3.0-only
"""bpy.app: version, handlers, timers, translations."""
import sys

from polyamour.app import *  # noqa: F401,F403
from polyamour.app import handlers, icons, timers, translations, version, version_string  # noqa: F401
from polyamour import app as _app

sys.modules[__name__ + '.handlers'] = handlers
sys.modules[__name__ + '.timers'] = timers
sys.modules[__name__ + '.translations'] = translations
sys.modules[__name__ + '.icons'] = icons


def __getattr__(name):
    return getattr(_app, name)
