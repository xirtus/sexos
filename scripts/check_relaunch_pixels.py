"""Verify real successor Quil status glyphs in its latest shell-tiled bounds."""
import re
from check_desktop_pixels import cell_errors
from check_disk_persistence import ppm_rgb

def verify(text,ppm,font,status):
    try:
        matches=re.findall(r'\[shell.tile.apply\] sid=301 x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',text)
        if not matches:raise ValueError('missing successor geometry')
        x,y,w,h=map(int,matches[-1]);width,height,pixels=ppm_rgb(ppm)
        if x<0 or y<50 or w<256 or h<64 or x+w>width or y+h>height:raise ValueError('invalid successor bounds')
        if f'[quil.text.view] target=16 surface=301 status={status}' not in text:raise ValueError('missing successor text intent')
        # Common native frame has chrome36, text scale 2 (Geist Mono), inset8. Require
        # every covered and every uncovered pixel of the complete status.
        mismatch=0
        for col,char in enumerate(status):
            px,py=x+8+col*12,y+36
            if px+12>x+w or py+18>y+h:raise ValueError('status outside successor')
            mismatch+=cell_errors(pixels,width,px,py,char,2,b'\xe0\xf0\xff',font)[0]
        return [] if mismatch==0 else [f'successor status glyph mismatch={mismatch}']
    except (ValueError,OverflowError) as e:return [str(e)]
