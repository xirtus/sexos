# SPDX-License-Identifier: GPL-3.0-only
"""Real Shell geometry and screendump helpers for program gates."""
import re
from pathlib import Path
from quil_workflow_gate import palette_commands, palette_select
ROOT = Path(__file__).resolve().parents[1]


def colours(data, tile):
    """The distinct colours inside a window tile of a binary PPM (P6) screendump."""
    fields = []
    i = 0
    while len(fields) < 4:
        while data[i : i + 1].isspace():
            i += 1
        if data[i : i + 1] == b'#':
            i = data.index(b'\n', i)
            continue
        j = i
        while not data[j : j + 1].isspace():
            j += 1
        fields.append(data[i:j])
        i = j
    assert fields[0] == b'P6' and int(fields[3]) < 256, fields
    width, height = int(fields[1]), int(fields[2])
    i += 1
    x, y, w, h = tile
    seen = set()
    for row in range(max(0, y + 36), min(height, y + h - 8)):
        base = i + (row * width) * 3
        line = data[base + max(0, x + 8) * 3 : base + min(width, x + w - 8) * 3]
        seen.update(line[k : k + 3] for k in range(0, len(line), 3))
    return seen


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
