# SPDX-License-Identifier: GPL-3.0-only
"""Moan paint/layout receipts, fenced after actual Shell maximization."""
import re
import time
from program_gate_geometry import current_tile, maximize
from lua_gate import qcodes


def chord(g, *keys):
    # Serialize make/release on the same QMP queue; delivery timing is not proof.
    for down, group in ((True, keys), (False, tuple(reversed(keys)))):
        g['q'].cmd('input-send-event', {'events': [
            {'type': 'key', 'data': {'down': down, 'key': {'type': 'qcode', 'data': key}}}
            for key in group]})
        time.sleep(.04 if down else .12)


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
    # A viewport alone can finish a burst that started before this command.
    # Require a fresh burst's first timeline record, then its closing viewport.
    opened = g['wait'](r'\[moan.timeline\] x=-?\d+ y=-?\d+ w=\d+ h=\d+ start=\d+ span=\d+', since, 120)
    since = opened.start()
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


def discard(g, surface, resolution, since):
    """Click the actual current Weft confirmation; Shell owns raw Tab."""
    g['focus'](surface)
    m = g['wait'](r'\[moan.dialog\] action=Discard x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)', since, 60)
    tile, size = viewport(g, surface, since)
    x, y, w, h = map(int, m.groups())
    assert w > 0 and h > 0 and 0 <= x < size[0] and 0 <= y < size[1], (m[0], size)
    ox, oy = origin(tile, size)
    x += ox; y += oy
    rw, rh = resolution
    assert 0 <= x < rw and 0 <= y < rh, (x, y, resolution)
    q = g['q']
    q.cmd('input-send-event', {'events': [
        {'type': 'abs', 'data': {'axis': 'x', 'value': round(x * 32767 / (rw - 1))}},
        {'type': 'abs', 'data': {'axis': 'y', 'value': round(y * 32767 / (rh - 1))}}]})
    time.sleep(.2)
    for down in (True, False):
        q.cmd('input-send-event', {'events': [{'type': 'btn', 'data': {'down': down, 'button': 'left'}}]})
        time.sleep(.15)


def type_text(g, value):
    """Use one QMP queue for shifted and plain text, with release spacing."""
    for ch in value:
        g['q'].cmd('send-key', {'keys': [{'type': 'qcode', 'data': k} for k in qcodes(ch)], 'hold-time': 40})
        time.sleep(.12)


def spindle(g, line):
    """Acknowledge each real Shell command character before admitting the next."""
    assert line and len(line) < 256
    g['focus'](153)
    time.sleep(.5)
    start = len(g['text']())
    base = None
    for n, ch in enumerate(line):
        since = len(g['text']())
        type_text(g, ch)
        m = g['wait'](rf'\[spindle\.input\.recv\] key=printable ch={re.escape(ch)} len=(\d+)\n', since, 30)
        if base is None:
            base = int(m[1]) - n
        assert int(m[1]) == base + n, (line[:n+1], m[0])
    since = len(g['text']())
    g['q'].key('ret')
    g['wait'](rf'\[spindle\.input\.recv\] key=enter len={base + len(line) - 1}\n', since, 30)
    return start
