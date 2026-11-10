"""Give browser pixel fixtures a visible program area through real Shell input."""
import re
import time
from quil_workflow_gate import palette_commands, palette_select
from web_surface_geometry import insets,canvas_box,latest_paint,latest_frame


def full_viewport(guest, root, surface, width=410, height=330):
    tile=latest_frame(guest['text'](),surface)
    left,top=insets(root)
    clipped=tile[2] < width + 2*left or tile[3] < height + top+left
    try:
        current_w,current_h=latest_paint(guest['text']())
        canvas_box(root,tile,current_w,current_h)
        clipped=current_w<width or current_h<height
    except (ValueError,AssertionError):pass
    if clipped:
        guest['focus'](surface)
        palette_select(guest['key'], 'ZoomToggle', palette_commands(root))
        start = len(guest['text']())
        guest['key']('ret', rf'\[shell.window.action\] action=ZoomToggle frame=\d+ sid={surface} ok=1')
        tile=latest_frame(guest['text']()[start:],surface)
        # Publication diagnostics have a finite log budget; the caller checks actual pixels.
        time.sleep(2)
    assert tile[2] >= width + 2*left and tile[3] >= height + top+left, ('browser fixture clipped', tile, width, height)
    return tile


def live_viewport(guest,root,surface,width=410,height=330):
    tile=full_viewport(guest,root,surface,width,height)
    # Move the real cursor to the protected bar, without masking any canvas pixel.
    guest['q'].cmd('input-send-event',{'events':[
        {'type':'abs','data':{'axis':'x','value':100}},
        {'type':'abs','data':{'axis':'y','value':100}}]})
    return painted_viewport(guest,root,surface,width,height,tile)


def painted_viewport(guest,root,surface,width=1,height=1,tile=None):
    """Await a complete paint fitted to current geometry, without changing the layout."""
    if tile is None:
        tile=latest_frame(guest['text'](),surface)
    deadline=time.monotonic()+60
    while True:
        try:
            w,h=latest_paint(guest['text']())
            canvas_box(root,tile,w,h)
            assert w>=width and h>=height
            time.sleep(.3)
            return tile,w,h
        except (ValueError,AssertionError):
            if time.monotonic()>deadline:raise
            time.sleep(.02)
