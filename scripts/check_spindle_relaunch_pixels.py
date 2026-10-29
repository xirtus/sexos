"""Actual four-surface successor glyphs in the catalog's shell frame306."""
import re
from check_desktop_pixels import glyph
from check_disk_persistence import ppm_rgb

def verify(text,ppm,font,require_echo=True,target=17,require_all_ink=True,require_prompt=None):
    if re.search(rf'\[spindle.view.text\] target={target} ',text):
        from check_spindle_viewport_pixels import verify as viewport_verify
        return viewport_verify(text,ppm,font,target,require_prompt,require_echo,require_all_ink)
    try:
        sids={17:[302,303,304,305],12:[154,160,161,162]}[target]
        a,b,c,d=sids
        # The desktop-wide tile diagnostic has a finite boot budget. Bind to
        # all four unbudgeted native viewport intents for this exact catalog.
        views={int(sid):tuple(map(int,(x,y,w,h))) for sid,x,y,w,h in re.findall(
            rf'\[shell.app.viewport\] target={target} sid=(\d+) x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',text) if int(sid) in sids}
        if set(views)!=set(sids):raise ValueError('missing four fresh Spindle viewport intents')
        ax,ay,aw,ah=views[a];rx,ry,rw,rh=views[b];lx,ly,lw,lh=views[c]
        if (rx,ry,rh)!=(ax+aw+8,ay,ah) or (lx,ly,lw)!=(ax,ay+ah+8,aw) or views[d]!=(rx,ly,rw,lh):
            raise ValueError('inconsistent fresh Spindle quadrants')
        x,y,w,h=ax-8,ay-36,aw+rw+24,ah+lh+52
        if aw!=(w-24)//2 or ah!=(h-52)//2:raise ValueError('invalid fresh Spindle split')
        width,height,pixels=ppm_rgb(ppm)
        if x<0 or y<50 or w<=32 or h<=64 or x+w>width or y+h>height:raise ValueError('invalid fresh frame bounds')
        bw,bh=w-16,h-44;cw,ch=(bw-8)//2,(bh-8)//2
        texts={}
        for sid,row in re.findall(rf'\[spindle.grid.text\] target={target} sid=(\d+) text=([^\r\n]*)',text):
            if int(sid) not in sids:continue
            if len(row)!=120:raise ValueError('partial or malformed grid intent')
            texts[int(sid)]=row
        if set(texts)!=set(sids):raise ValueError('missing four exact successor grids')
        if require_echo and 'spindle17' not in ''.join(texts.values()):raise ValueError('missing command echo in native grid')
        if require_prompt and require_prompt not in texts[c]:raise ValueError("missing exact recalled prompt")
        errors=[]
        for index,sid in enumerate(sids):
            col,row=index%2,index//2;sx,sy=x+8+col*(cw+8),y+36+row*(ch+8)
            sw=cw if col==0 else bw-cw-8;sh=ch if row==0 else bh-ch-8
            scale=2 if sw>=256 else 1;mismatch=0;visible=0;clipped=0
            for offset,char in enumerate(texts[sid]):
                tx=sx+8+(offset%20)*6*scale;ty=sy+12+(offset//20)*9*scale
                if char!=' ' and (tx+5*scale>sx+sw or ty+7*scale>sy+sh):clipped+=1
                for gy,bits in enumerate(glyph(font,char)):
                    for gx in range(6):
                        on=gx<5 and bool(bits&(1<<(4-gx)))
                        for dy in range(scale):
                            for dx in range(scale):
                                px,py=tx+gx*scale+dx,ty+gy*scale+dy
                                if px>=sx+sw or py>=sy+sh:continue
                                actual=pixels[(py*width+px)*3:(py*width+px)*3+3]==b'\xe8\xff\xff'
                                visible+=on;mismatch+=on!=actual
            if clipped:errors.append(f'sid{sid}: clipped nonblank cells={clipped}')
            if (require_all_ink or texts[sid].strip()) and visible==0:errors.append(f'sid{sid}: no visible glyph evidence')
            if mismatch:errors.append(f'sid{sid}: glyph mismatch={mismatch}')
        return errors
    except (ValueError,KeyError,OverflowError) as exc:return [str(exc)]
