"""Use published shared chrome metrics and real Shell input for fixed browser fixtures."""
import re
import time


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


def full_viewport(guest, root, surface, width=410, height=330):
    from quil_workflow_gate import palette_commands, palette_select
    pattern = rf'\[shell.tile.apply\] sid={surface} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)'
    tile = tuple(map(int, re.findall(pattern, guest['text']())[-1]))
    if content_rect(guest['text'](), tile)[2] < width or content_rect(guest['text'](), tile)[3] < height:
        guest['focus'](surface)
        palette_select(guest['key'], 'ZoomToggle', palette_commands(root))
        start = len(guest['text']())
        guest['key']('ret', rf'\[shell.window.action\] action=ZoomToggle frame=\d+ sid={surface} ok=1')
        zoom = rf'\[silk.lifecycle.zoom.active\] frame=\d+ sid={surface} zx=(-?\d+) zy=(-?\d+) zw=(\d+) zh=(\d+)'
        tile = tuple(map(int, re.findall(zoom, guest['text']()[start:])[-1]))
        time.sleep(2)
    body = content_rect(guest['text'](), tile)
    assert body[2] >= width and body[3] >= height, ('browser fixture clipped', tile, body, width, height)
    return tile

def clock_rect(text):
    m=chrome_metrics(text);ui=m['scale']
    assert m['width']>=320*ui,'unsupported narrow bar'
    return m['width']-152*ui,2*ui,144*ui,24*ui

def workspace_point(text,index):
    m=chrome_metrics(text);ui=m['scale'];width=m['width']
    assert 0<=index<9 and width>=600*ui,'compact mode exposes Atlas, not nine workspace targets'
    start=min((width-216*ui)//2,width-488*ui)
    return start+(24*index+12)*ui,14*ui
