"""Native variable-size Spindle text: exact incarnation, geometry, payload and ink."""
import re
from check_desktop_pixels import glyph
from check_disk_persistence import ppm_rgb

def layout(w,h):
    bw=max(0,w-16);bh=max(0,h-44);left=max(0,bw-8)//2;right=max(0,bw-left-8)
    scale=lambda width:2 if width>=256 else 1
    largest=max(scale(left),scale(right));split=bh>=2*(12+7*largest)+8
    top=max(0,bh-8)//2 if split else 0;bottom=max(0,bh-top-8)if split else bh;by=36+(top+8 if split else 0)
    cols=min(20,max(0,min(max(0,left-16)//(6*scale(left)),max(0,right-16)//(6*scale(right)))-1))
    def rows(height):return min(6,1+(height-12-7*largest)//(9*largest))if height>=12+7*largest and cols else 0
    return [(8,36,left,top),(16+left,36,right,top),(8,by,left,bottom),(16+left,by,right,bottom)],cols,[rows(top),rows(bottom)]

def verify(text,ppm,font,target=17,require_prompt=None,require_echo=True,require_all_ink=True):
    try:
        sids={12:[154,160,161,162],17:[302,303,304,305]}[target]
        accepted=re.findall(rf'\[spindle.view.accepted\] target={target} task={target} sequence=(\d+) w=(\d+) h=(\d+) cols=(\d+) top_rows=(\d+) bottom_rows=(\d+) sender=3',text)
        if not accepted:raise ValueError('no authenticated current-incarnation viewport')
        seq,w,h,cols,top,bottom=map(int,accepted[-1]);panes,expected_cols,rows=layout(w,h)
        if (cols,[top,bottom])!=(expected_cols,rows):raise ValueError('wrong viewport text capacity')
        views={int(sid):tuple(map(int,(x,y,w,h)))for sid,x,y,w,h in re.findall(rf'\[shell.app.viewport\] target={target} sid=(\d+) x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',text)if int(sid)in sids}
        if set(views)!=set(sids):raise ValueError('missing catalog viewport quadrants')
        x=views[sids[0]][0]-8;y=views[sids[0]][1]-36
        if any(views[sid]!=(x+px,y+py,pw,ph)for sid,(px,py,pw,ph)in zip(sids,panes)):raise ValueError('layout does not match shell geometry')
        width,height,pixels=ppm_rgb(ppm)
        if x<0 or y<50 or x+w>width or y+h>height:raise ValueError('viewport outside framebuffer')
        records={}
        for sid,sequence,paint,c,r,n in re.findall(rf'\[spindle.view.text\] target={target} sid=(\d+) sequence=(\d+) paint=(\d+) cols=(\d+) rows=(\d+) bytes=(\d+)',text):
            if int(sid)not in sids or int(sequence)!=seq:continue
            chunks=[(int(off),bytes.fromhex(data))for off,data in re.findall(rf'\[spindle.view.bytes\] target={target} sid={sid} seq={seq} paint={paint} off=(\d+) hex=([0-9a-f]*)',text)]
            joined=bytearray()
            for off,data in chunks:
                if off!=len(joined) or len(data)!=min(48,int(n)-off):raise ValueError('partial or duplicated viewport byte chunks')
                joined.extend(data)
            records[int(sid)]=(int(c),int(r),int(n),bytes(joined))
        if set(records)!=set(sids):raise ValueError('missing exact current-layout text payloads')
        rendered=[];errors=[]
        for index,sid in enumerate(sids):
            c,r,n,data=records[sid];px,py,pw,ph=panes[index];sx,sy=x+px,y+py;scale=2 if pw>=256 else 1;expected_rows=rows[index//2]
            if expected_rows==0:
                if (c,r,n,data)!=(0,0,0,b''):raise ValueError('collapsed pane retained text')
                rendered.append([]);continue
            if (c,r,n)!=(cols,expected_rows,expected_rows*(cols+1))or len(data)!=n or n>126:raise ValueError('wrong bounded pane payload')
            lines=data.split(b'\n')
            if lines[-1]!=b'' or len(lines)!=r+1 or any(len(line)!=c or any(not 32<=v<=126 for v in line)for line in lines[:-1]):raise ValueError('invalid explicit text rows')
            lines=lines[:-1];rendered.append(lines);ink=0;mismatch=0
            # Include unused physical columns and former rows: stale glyphs
            # after a shorter repaint must not masquerade as current content.
            physical_cols=min(80,max(0,pw-16)//(6*scale));physical_rows=min(7,max(0,1+(ph-12-7*scale)//(9*scale)))
            for row in range(physical_rows):
                for col in range(physical_cols):
                    char=chr(lines[row][col])if row<r and col<c else' '
                    if char!=' 'and(8+col*6*scale+5*scale>pw or 12+row*9*scale+7*scale>ph):raise ValueError('clipped nonblank viewport glyph')
                    for gy,bits in enumerate(glyph(font,char)):
                        for gx in range(6):
                            on=gx<5 and bool(bits&(1<<(4-gx)))
                            for dy in range(scale):
                                for dx in range(scale):
                                    xx=sx+8+col*6*scale+gx*scale+dx;yy=sy+12+row*9*scale+gy*scale+dy
                                    if xx>=sx+pw or yy>=sy+ph:continue
                                    actual=pixels[(yy*width+xx)*3:(yy*width+xx)*3+3]==b'\xe8\xff\xff';mismatch+=actual!=on;ink+=on
            if mismatch:errors.append(f'sid{sid}: viewport glyph mismatch={mismatch}')
            if (require_all_ink or data.strip())and not ink:errors.append(f'sid{sid}: no viewport ink')
        if require_echo and not any(b'spindle17'in line for lines in rendered for line in lines):raise ValueError('missing native command echo')
        prompt=(rendered[2][-1]+rendered[3][-1]).decode()if rendered[2]and rendered[3]else''
        if require_prompt and require_prompt not in prompt:raise ValueError('missing exact visible prompt')
        return errors
    except (ValueError,KeyError,OverflowError)as e:return[str(e)]
