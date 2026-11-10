"""Give browser pixel fixtures a visible program area through real Shell input."""
import re
import time
from web_surface_geometry import insets,canvas_box,latest_paint,latest_frame


def full_viewport(guest, root, surface, width=410, height=330):
    from quil_workflow_gate import palette_commands, palette_select
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


def chrome_metrics(text):
    # A serial tail may stop halfway through an otherwise valid-looking record.
    complete = text[:text.rfind('\n') + 1]
    records = re.findall(r'^\[display.chrome.metrics\] ([^\r\n]+)\r?$', complete, re.MULTILINE)
    assert records, 'missing completed chrome geometry publication'
    fields = dict(re.findall(r'(\w+)=(\d+)', records[-1]))
    required = ('scale', 'bar', 'title', 'inset', 'body_top', 'target', 'width', 'height', 'publication')
    assert all(key in fields for key in required), ('malformed chrome geometry', fields)
    result = {key: int(fields[key]) for key in required}
    scale = result['scale']
    assert scale in (1, 2, 3) and result['publication'] == 1, result
    # Independent approved logical goldens; do not accept a self-consistent but
    # wrong geometry announcement as permission to move the pixel assertions.
    for key, logical in (('bar', 28), ('title', 26), ('inset', 8), ('body_top', 34), ('target', 24)):
        assert result[key] == logical * scale, ('chrome contract changed', key, result)
    assert 0 < result['width'] <= 8192 and 0 < result['height'] <= 4320, result
    return result


def content_rect(text, tile):
    metric = chrome_metrics(text)
    x, y, width, height = tile
    inset, top = metric['inset'], metric['body_top']
    assert width >= 2 * inset and height >= top + inset, ('empty program body', tile, metric)
    return x + inset, y + top, width - 2 * inset, height - top - inset


def clock_rect(text):
    m=chrome_metrics(text);ui=m['scale']
    assert m['width']>=320*ui,'unsupported narrow bar'
    return m['width']-152*ui,2*ui,144*ui,24*ui


def workspace_point(text,index):
    m=chrome_metrics(text);ui=m['scale'];width=m['width']
    assert 0<=index<9 and width>=600*ui,'compact mode exposes Atlas, not nine workspace targets'
    start=min((width-216*ui)//2,width-488*ui)
    return start+(24*index+12)*ui,14*ui
