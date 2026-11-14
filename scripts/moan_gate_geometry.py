# SPDX-License-Identifier: GPL-3.0-only
"""Moan paint/layout receipts, fenced after actual Shell maximization."""
import re
from program_gate_geometry import current_tile, maximize


def chord(g, *keys):
    g['q'].cmd('input-send-event', {'events': [
        {'type': 'key', 'data': {'down': down, 'key': {'type': 'qcode', 'data': key}}}
        for down, group in ((True, keys), (False, tuple(reversed(keys)))) for key in group]})


def viewport(g, surface, since):
    tile = current_tile(g['text'](), surface)
    w, h = tile[2] - 16, tile[3] - 42
    # Exact dimensions reject older small resize events queued before ZoomToggle.
    g['wait'](rf'\[moan.viewport\] w={w} h={h} frame=[1-9]\d*', since, 120)
    return tile, (w, h)


def ready(g, surface):
    tile, since = maximize(g, surface)
    viewport(g, surface, since)
    return since


def refresh(g, surface):
    g['focus'](surface)
    since = len(g['text']())
    chord(g, 'ctrl', 'shift', 'l')
    tile, size = viewport(g, surface, since)
    return since, tile, size


def origin(tile, size):
    side = (tile[2] - size[0]) // 2
    return tile[0] + side, tile[1] + tile[3] - size[1] - side


def control(g, surface, name, resolution):
    since, tile, size = refresh(g, surface)
    positions = re.findall(rf'\[moan.control\] name={re.escape(name)} x=(-?\d+) y=(-?\d+)', g['text']()[since:])
    assert positions, name
    x, y = map(int, positions[-1])
    assert 0 <= x < size[0] and 0 <= y < size[1], (name, x, y, size)
    ox, oy = origin(tile, size)
    x += ox; y += oy
    rw, rh = resolution
    assert 0 <= x < rw and 0 <= y < rh, (name, x, y, resolution)
    return x, y
