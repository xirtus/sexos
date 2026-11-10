"""Verify real successor Quil status glyphs in its latest shell-tiled bounds."""
import re
from check_desktop_pixels import cell_errors,chrome_geometry
from check_disk_persistence import ppm_rgb

def current_dirty_status(text,original):
    complete=text[:text.rfind('\n')+1]
    statuses=re.findall(r'^\[quil.text.view\] target=16 surface=301 status=([^\r\n]*)\r?$',complete,re.MULTILINE)
    name,separator,_=original.rpartition(' * ')
    if statuses and separator:
        current_name,current_separator,_=statuses[-1].rpartition(' * ')
        if current_separator and current_name==name:return statuses[-1]
    return None

def verify(text,ppm,font,status):
    try:
        matches=re.findall(r'\[shell.tile.apply\] sid=301 x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',text)
        if not matches:raise ValueError('missing successor geometry')
        x,y,w,h=map(int,matches[-1]);width,height,pixels=ppm_rgb(ppm)
        metric=chrome_geometry(text);ui=metric['scale'];inset=metric['inset'];top=metric['body_top'];scale=(2 if w>=256 else 1)*ui
        if x<0 or y<metric['bar'] or w<256 or h<top+inset or x+w>width or y+h>height:raise ValueError('invalid successor bounds')
        if f'[quil.text.view] target=16 surface=301 status={status}' not in text:raise ValueError('missing successor text intent')
        # Use the captured shared chrome and actual scaled Geist Mono cells. Require
        # every covered and every uncovered pixel of the complete status.
        mismatch=0
        for col,char in enumerate(status):
            px,py=x+inset+col*6*scale,y+top
            if px+6*scale>x+w or py+9*scale>y+h:raise ValueError('status outside successor')
            mismatch+=cell_errors(pixels,width,px,py,char,scale,b'\xe0\xf0\xff',font)[0]
        return [] if mismatch==0 else [f'successor status glyph mismatch={mismatch}']
    except (ValueError,OverflowError,AssertionError) as e:return [str(e)]
