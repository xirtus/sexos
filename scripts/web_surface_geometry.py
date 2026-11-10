"""Actual Web paint sizes and source-selected program insets for pixel oracles.

Client receipts select reference dimensions; the callers still verify real pixels.
No crop-to-410 fallback, hidden-area exemption, or altered pixel threshold.
"""
import re


def insets(root):
    style=(root/'crates/sex-pdx/src/desktop_style.rs').read_text()
    pixel=(root/'crates/sex-pdx/src/pixel_region.rs').read_text()
    def constant(name):
        return int(re.search(r'pub const '+name+r'\s*:\s*u32\s*=\s*(\d+)\s*;',style)[1])
    names={'crate::desktop_style::INSET':constant('INSET'),
           'crate::desktop_style::chrome::TITLE':constant('TITLE'),
           'crate::desktop_style::chrome::RESIZE':constant('RESIZE')}
    def expression(name):
        value=re.search(r'pub const '+name+r'\s*:\s*u32\s*=\s*([^;]+);',pixel)[1]
        for key,number in names.items():value=value.replace(key,str(number))
        assert re.fullmatch(r'\s*\d+(?:\s*\+\s*\d+)*\s*',value),('unsupported inset expression',value)
        return sum(map(int,value.split('+')))
    return expression('LEFT'),expression('TOP')


def canvas_box(root,tile,width,height):
    x,y,w,h=tile
    left,top=insets(root)
    gap=w-width
    assert gap>0 and gap%(2*left)==0,('paint not fitted to frame',tile,width,height)
    scale=gap//(2*left)
    assert 1<=scale<=3 and h-height==(top+left)*scale,('paint/body contract mismatch',tile,width,height,scale)
    return x+left*scale,y+top*scale,width,height


def latest_paint(text):
    complete=text[:text.rfind('\n')+1]
    found=re.findall(r'\[web.shown\] w=(\d+) h=(\d+) scroll=-?\d+ focus=-?\d+ items=\d+ ticks=\d+(?: canvases=\d+)?\n',complete)
    if not found:raise ValueError('no complete Web paint receipt')
    w,h=map(int,found[-1])
    assert w>0 and h>0,('empty paint',w,h)
    return w,h


def latest_frame(text,surface):
    """Use the latest real tiling/zoom publication for this surface."""
    complete=text[:text.rfind('\n')+1]
    patterns=(rf'\[shell.tile.apply\] sid={surface} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)\n',
              rf'\[silk.lifecycle.zoom.active\] frame=\d+ sid={surface} zx=(-?\d+) zy=(-?\d+) zw=(\d+) zh=(\d+)\n',
              rf'\[silk.lifecycle.zoom.restore\] frame=\d+ sid={surface} nx=(-?\d+) ny=(-?\d+) nw=(\d+) nh=(\d+)\n')
    found=[(m.start(),tuple(map(int,m.groups())))for p in patterns for m in re.finditer(p,complete)]
    if not found:raise ValueError('no complete geometry publication for surface')
    return max(found,key=lambda item:item[0])[1]
