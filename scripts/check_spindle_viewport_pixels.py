"""Native variable-size Spindle text: exact incarnation, geometry, payload and ink."""
import re
from pathlib import Path
from history_roster_evidence import source_spindles, verify_boot_roster
from check_desktop_pixels import glyph
from check_disk_persistence import ppm_rgb

SCALE_SOURCE='crates/sex-pdx/src/spindle_view.rs'

def _scale_threshold(root=None):
    root=Path(__file__).resolve().parents[1]if root is None else Path(root)
    try:source=(root/SCALE_SOURCE).read_text()
    except OSError as error:raise ValueError('missing bound Spindle scale source')from error
    source=re.sub(r'/\*.*?\*/|//[^\n]*','',source,flags=re.S)
    declarations=list(re.finditer(r'\bfn\s+scale\s*\(',source))
    # Deliberately support only the two sealed literal policies. No expression
    # evaluation or fallback to the host checkout for a supplied frozen root.
    pattern=r'\bpub\s+const\s+fn\s+scale\s*\(\s*width\s*:\s*u32\s*\)\s*->\s*u32\s*\{\s*if\s+width\s*>=\s*(128|256)\s*\{\s*2\s*\}\s*else\s*\{\s*1\s*\}\s*\}'
    matches=list(re.finditer(pattern,source))
    if len(declarations)!=1 or len(matches)!=1:raise ValueError('unsupported or ambiguous bound Spindle scale function')
    return int(matches[0][1])

def layout(w,h,root=None):
    bw=max(0,w-16);bh=max(0,h-44)
    threshold=_scale_threshold(root);scale=2 if bw>=threshold else 1
    cols=min(110,max(0,max(0,bw-16)//(6*scale)-1))
    rows=min(48,1+(bh-12-7*scale)//(9*scale))if bh>=12+7*scale and cols else 0
    empty=(8,36,bw,0)
    return [(8,36,bw,bh),empty,empty,empty],cols,[rows,0]

def _source_grid(root,target):
    rows=source_spindles(root)
    row=next((row for row in rows if row['target']==target),None)
    if row is None:raise ValueError('unknown catalog Spindle')
    source=(Path(root)/'crates/sex-pdx/src/app_catalog.rs').read_text()
    source=re.sub(r'/\*.*?\*/|//[^\n]*','',source,flags=re.S)
    blocks=[block for block in re.findall(r'Incarnation\s*\{([^{}]*)\}',source)
            if re.search(r'\bapp\s*:\s*App::Spindle\b',block)
            and re.search(r'\btarget\s*:\s*'+str(target)+r'\s*,',block)]
    if len(blocks)!=1:raise ValueError('missing exact literal Spindle grid')
    match=re.search(r'\bchildren\s*:\s*&\[\s*([0-9,\s]*)\]',blocks[0])
    if not match:raise ValueError('nonliteral Spindle grid children')
    children=[int(value)for value in match[1].split(',')if value.strip()]
    sids=[row['surface'],*children]
    if len(children)!=3 or len(set(sids))!=4 or row['frame']in sids or any(not 0<sid<(1<<64)for sid in sids):
        raise ValueError('Spindle grid requires three unique children')
    return sids

def verify(text,ppm,font,target=17,require_prompt=None,require_echo=True,require_all_ink=True,resolution=None,root=None,require_text=None):
    try:
        root=Path(__file__).resolve().parents[1]if root is None else Path(root)
        sids=_source_grid(root,target)
        roster=verify_boot_roster(root,text)
        task=next((member['task']for member in roster['participants']if member['target']==target),None)
        if task is None:raise ValueError('Spindle absent from held BOOT roster')
        accepted=re.findall(rf'\[spindle.view.accepted\] target={target} task=(\d+) sequence=(\d+) w=(\d+) h=(\d+) cols=(\d+) top_rows=(\d+) bottom_rows=(\d+) sender=3(?=\r?\n|$)',text)
        if not accepted:raise ValueError('no authenticated current-incarnation viewport')
        actual_task,seq,w,h,cols,top,bottom=map(int,accepted[-1])
        if actual_task!=task:raise ValueError('viewport task differs from held BOOT roster')
        if seq==0:raise ValueError('zero viewport sequence')
        panes,expected_cols,rows=layout(w,h,root)
        threshold=_scale_threshold(root)
        if (cols,[top,bottom])!=(expected_cols,rows):raise ValueError('wrong viewport text capacity')
        views={int(sid):tuple(map(int,(x,y,w,h)))for sid,x,y,w,h in re.findall(rf'\[shell.app.viewport\] target={target} sid=(\d+) x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',text)if int(sid)in sids}
        if sids[0]not in views:raise ValueError('missing catalog terminal viewport')
        x=views[sids[0]][0]-8;y=views[sids[0]][1]-36
        if views[sids[0]]!=(x+panes[0][0],y+panes[0][1],panes[0][2],panes[0][3]):raise ValueError('layout does not match shell geometry')
        width,height,pixels=ppm_rgb(ppm,**({'min_width':1,'min_height':1}if resolution else{}))
        if resolution and (width,height)!=tuple(resolution):raise ValueError('wrong actual framebuffer resolution')
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
        if sids[0]not in records or set(records)-{sids[0]}:raise ValueError('missing exact current-layout text payloads')
        rendered=[];errors=[]
        for index,sid in enumerate(sids[:1]):
            c,r,n,data=records[sid];px,py,pw,ph=panes[index];sx,sy=x+px,y+py;scale=2 if pw>=threshold else 1;expected_rows=rows[0]
            if expected_rows==0:
                if (c,r,n,data)!=(0,0,0,b''):raise ValueError('collapsed pane retained text')
                rendered.append([]);continue
            if (c,r,n)!=(cols,expected_rows,expected_rows*(cols+1))or len(data)!=n or n>6144:raise ValueError('wrong bounded pane payload')
            lines=data.split(b'\n')
            if lines[-1]!=b'' or len(lines)!=r+1 or any(len(line)!=c or any(not 32<=v<=126 for v in line)for line in lines[:-1]):raise ValueError('invalid explicit text rows')
            lines=lines[:-1];rendered.append(lines);ink=0;mismatch=0
            # Include unused physical columns and former rows: stale glyphs
            # after a shorter repaint must not masquerade as current content.
            physical_cols=min(110,max(0,pw-16)//(6*scale));physical_rows=min(48,max(0,1+(ph-12-7*scale)//(9*scale)))
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
        if require_text is not None:
            if not isinstance(require_text,str)or not require_text:raise ValueError('expected current text must be nonempty ASCII')
            needle=require_text.encode('ascii')
            current=rendered[0]
            if not any(needle in line for line in current):raise ValueError('missing expected current rendered text')
        prompt=rendered[0][-1].decode()if rendered[0]else''
        if require_prompt and require_prompt not in prompt:raise ValueError('missing exact visible prompt')
        return errors
    except (ValueError,KeyError,OverflowError)as e:return[str(e)]
