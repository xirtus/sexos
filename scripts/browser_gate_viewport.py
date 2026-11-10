"""Give fixed-size browser fixtures a full visible program viewport via real Shell input."""
import re
import time
from quil_workflow_gate import palette_commands, palette_select


def full_viewport(guest, root, surface, width=410, height=330):
    pattern = rf'\[shell.tile.apply\] sid={surface} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)'
    tile = tuple(map(int, re.findall(pattern, guest['text']())[-1]))
    # The content begins 8 pixels in and 36 down, with the remaining border below.
    if tile[2] < width + 16 or tile[3] < height + 44:
        guest['focus'](surface)
        palette_select(guest['key'], 'ZoomToggle', palette_commands(root))
        start = len(guest['text']())
        guest['key']('ret', rf'\[shell.window.action\] action=ZoomToggle frame=\d+ sid={surface} ok=1')
        zoom = rf'\[silk.lifecycle.zoom.active\] frame=\d+ sid={surface} zx=(-?\d+) zy=(-?\d+) zw=(\d+) zh=(\d+)'
        tile = tuple(map(int, re.findall(zoom, guest['text']()[start:])[-1]))
        # Publication diagnostics have a finite log budget; the caller checks actual pixels.
        time.sleep(2)
    assert tile[2] >= width + 16 and tile[3] >= height + 44, ('browser fixture clipped', tile, width, height)
    return tile
