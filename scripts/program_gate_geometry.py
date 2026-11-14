# SPDX-License-Identifier: GPL-3.0-only
"""Real Shell geometry for program gates; never click outside a tiled client.

Maximize once after launching a fresh program, then wait for that program's
updated layout/paint receipt before using client-relative control coordinates.
The helper does not treat the Shell resize admission as proof of client paint.
"""
import re
from pathlib import Path
from quil_workflow_gate import palette_commands, palette_select
ROOT = Path(__file__).resolve().parents[1]


def current_tile(text, surface, since=0):
    sid = int(surface)
    patterns = [
        rf'\[shell.tile.apply\] sid={sid} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',
        rf'\[silk.lifecycle.zoom.active\] frame=\d+ sid={sid} zx=(-?\d+) zy=(-?\d+) zw=(\d+) zh=(\d+)',
        rf'\[silk.lifecycle.unzoom.active\] frame=\d+ sid={sid} nx=(-?\d+) ny=(-?\d+) nw=(\d+) nh=(\d+)',
    ]
    matches = [m for p in patterns for m in re.finditer(p, text[since:])]
    if not matches:
        raise AssertionError(f'no actual Shell geometry for surface {sid}')
    return list(map(int, max(matches, key=lambda m: m.start()).groups()))


def maximize(g, surface, timeout=120):
    """Return (actual maximized tile, offset before resize) for a fresh window."""
    sid = int(surface)
    g['focus'](sid)
    palette_select(g['key'], 'ZoomToggle', palette_commands(ROOT))
    start = len(g['text']())
    g['key']('ret', rf'\[shell.window.action\] action=ZoomToggle frame=\d+ sid={sid} ok=1')
    m = g['wait'](rf'\[silk.lifecycle.zoom.active\] frame=\d+ sid={sid} zx=(-?\d+) zy=(-?\d+) zw=(\d+) zh=(\d+)', start, timeout)
    tile = list(map(int, m.groups()))
    assert tile[2] > 16 and tile[3] > 42, tile
    return tile, start
