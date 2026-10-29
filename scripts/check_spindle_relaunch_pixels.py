"""Actual four-surface successor glyphs in the catalog's shell frame306."""
import re
from check_desktop_pixels import glyph
from check_disk_persistence import ppm_rgb

def verify(text,ppm,font,require_echo=True):
    try:
        # The desktop-wide tile diagnostic has a finite boot budget. Bind to
        # all four unbudgeted native viewport intents for this exact catalog.
        views={int(sid):tuple(map(int,(x,y,w,h))) for sid,x,y,w,h in re.findall(
            r'\[shell.app.viewport\] target=17 sid=(30[2-5]) x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',text)}
        if set(views)!={302,303,304,305}:raise ValueError('missing four fresh Spindle viewport intents')
        ax,ay,aw,ah=views[302];rx,ry,rw,rh=views[303];lx,ly,lw,lh=views[304]
        if (rx,ry,rh)!=(ax+aw+8,ay,ah) or (lx,ly,lw)!=(ax,ay+ah+8,aw) or views[305]!=(rx,ly,rw,lh):
            raise ValueError('inconsistent fresh Spindle quadrants')
        x,y,w,h=ax-8,ay-36,aw+rw+24,ah+lh+52
        if aw!=(w-24)//2 or ah!=(h-52)//2:raise ValueError('invalid fresh Spindle split')
        width,height,pixels=ppm_rgb(ppm)
        if x<0 or y<50 or w<=32 or h<=64 or x+w>width or y+h>height:raise ValueError('invalid fresh frame bounds')
        bw,bh=w-16,h-44;cw,ch=(bw-8)//2,(bh-8)//2
        texts={}
        for sid,row in re.findall(r'\[spindle.grid.text\] target=17 sid=(30[2-5]) text=([^\r\n]*)',text):
            if len(row)!=120:raise ValueError('partial or malformed grid intent')
            texts[int(sid)]=row
        if set(texts)!={302,303,304,305}:raise ValueError('missing four exact successor grids')
        if require_echo and 'spindle17' not in ''.join(texts.values()):raise ValueError('missing command echo in native grid')
        errors=[]
        for index,sid in enumerate([302,303,304,305]):
            col,row=index%2,index//2;sx,sy=x+8+col*(cw+8),y+36+row*(ch+8)
            sw=cw if col==0 else bw-cw-8;sh=ch if row==0 else bh-ch-8
            scale=2 if sw>=256 else 1;mismatch=0;visible=0
            for offset,char in enumerate(texts[sid]):
                tx=sx+8+(offset%20)*6*scale;ty=sy+12+(offset//20)*9*scale
                for gy,bits in enumerate(glyph(font,char)):
                    for gx in range(6):
                        on=gx<5 and bool(bits&(1<<(4-gx)))
                        for dy in range(scale):
                            for dx in range(scale):
                                px,py=tx+gx*scale+dx,ty+gy*scale+dy
                                if px>=sx+sw or py>=sy+sh:continue
                                actual=pixels[(py*width+px)*3:(py*width+px)*3+3]==b'\xe8\xff\xff'
                                visible+=on;mismatch+=on!=actual
            if visible==0:errors.append(f'sid{sid}: no visible glyph evidence')
            if mismatch:errors.append(f'sid{sid}: glyph mismatch={mismatch}')
        return errors
    except (ValueError,KeyError,OverflowError) as exc:return [str(exc)]
