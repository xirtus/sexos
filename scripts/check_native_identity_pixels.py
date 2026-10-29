"""Actual Quil family appearance at each live BOOT incarnation; no authority inference."""
from check_quil_ui_pixels import view
from check_desktop_pixels import glyph
from check_disk_persistence import ppm_rgb

def verify(text,ppm,font,target,resolution):
    try:
        v=view(text,target);width,height,pixels=ppm_rgb(ppm,min_width=1,min_height=1)
        if (width,height)!=tuple(resolution):raise ValueError('wrong framebuffer resolution')
        if not v['applied']:raise ValueError('view not application acknowledged')
        x,y=(v['x']-8,v['y']-36) if target==9 else(v['x'],v['y'])
        errors=[];mismatch=0
        for col,ch in enumerate('Quil'):
            for gy,bits in enumerate(glyph(font,ch)):
                for gx in range(6):
                    on=gx<5 and bool(bits&(1<<(4-gx)))
                    for dy in range(2):
                        for dx in range(2):
                            px=x+62+(col*6+gx)*2+dx;py=y+7+gy*2+dy
                            if not(0<=px<width and 0<=py<height):raise ValueError('title clipped')
                            mismatch+=(pixels[(py*width+px)*3:(py*width+px)*3+3]==b'\xe8\xff\xff')!=on
        if mismatch:errors.append('Quil title glyph mismatch='+str(mismatch))
        # Sample empty owned content away from text, cursor and frame rims.
        px=v['x']+v['w']//2;py=v['y']+v['h']-48
        if pixels[(py*width+px)*3:(py*width+px)*3+3]!=b'\x0c\x14\x20':errors.append('inconsistent Quil canvas')
        if any(pixels[(yy*width+xx)*3:(yy*width+xx)*3+3]!=b'\xcb\xa6\xf7'for yy in range(22,26)for xx in range(155,159)):
            errors.append('wrong active-app indicator')
        return errors
    except (ValueError,KeyError,IndexError,OverflowError)as error:return[str(error)]
