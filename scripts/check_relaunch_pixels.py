"""Verify real successor Quil status glyphs in its latest shell-tiled bounds."""
import re
from check_desktop_pixels import glyph
from check_disk_persistence import ppm_rgb

def verify(text,ppm,font,status):
    try:
        matches=re.findall(r'\[shell.tile.apply\] sid=301 x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',text)
        if not matches:raise ValueError('missing successor geometry')
        x,y,w,h=map(int,matches[-1]);width,height,pixels=ppm_rgb(ppm)
        if x<0 or y<50 or w<256 or h<64 or x+w>width or y+h>height:raise ValueError('invalid successor bounds')
        if f'[quil.text.view] target=16 surface=301 status={status}' not in text:raise ValueError('missing successor text intent')
        # Common native frame has chrome36, glyph scale2, inset8. Require
        # every glyph bit and every dark interior bit of the complete status.
        mismatch=0
        for col,char in enumerate(status):
            for gy,bits in enumerate(glyph(font,char)):
                for gx in range(6):
                    on=gx<5 and bool(bits&(1<<(4-gx)))
                    for dy in range(2):
                        for dx in range(2):
                            px=x+8+(col*6+gx)*2+dx;py=y+36+gy*2+dy
                            if px>=x+w or py>=y+h:raise ValueError('status outside successor')
                            actual=pixels[(py*width+px)*3:(py*width+px)*3+3]==b'\xe0\xf0\xff'
                            mismatch+=actual!=on
        return [] if mismatch==0 else [f'successor status glyph mismatch={mismatch}']
    except (ValueError,OverflowError) as e:return [str(e)]
