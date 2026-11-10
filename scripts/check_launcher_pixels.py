"""Actual launcher text pixels; log intents alone cannot establish visibility."""
import re
from check_desktop_pixels import cell_errors,chrome_geometry
from check_disk_persistence import ppm_rgb

def layout(text):
    records=list(re.finditer(r'\[launcher.text.view\] ([^\r\n\[]*)',text))
    if not records:raise ValueError('missing launcher intent')
    record=records[-1];v={k:int(n) for k,n in re.findall(r'(\w+)=(-?\d+)',record[1])}
    required=('x','y','w','h','scale','selected','first','rows')
    if any(k not in v for k in required) or v['scale'] not in (1,2,3,4,6) or not 1<=v['rows']<=5:raise ValueError('invalid launcher bounds')
    rows=[(int(i),int(r),name,status) for i,r,name,status in re.findall(r'\[launcher.text.row\] index=(\d+) row=(\d+) name=([^\r\n\[]*?) status=([^\r\n\[]*)',text[record.end():])]
    if len(rows)!=v['rows'] or {r for _,r,_,_ in rows}!=set(range(v['rows'])) or {i for i,_,_,_ in rows}!=set(range(v['first'],v['first']+v['rows'])):raise ValueError('incomplete launcher rows')
    if v['selected'] not in {i for i,_,_,_ in rows}:raise ValueError('invisible launcher selection')
    lines=[(0,'APPS / ACTIONS'),(1,'J/K CHOOSE  ENTER OPEN  ESC CLOSE')]
    for index,row,name,status in rows:
        lines.extend([(2+2*row,('> 'if index==v['selected']else'  ')+name),(3+2*row,'  '+status)])
    return v,lines

def verify(text,ppm,font,resolution=None):
    try:
        v,lines=layout(text);width,height,pixels=ppm_rgb(ppm,min_width=1,min_height=1) if resolution else ppm_rgb(ppm)
        if resolution and (width,height)!=tuple(resolution):raise ValueError('wrong framebuffer resolution')
        x,y,w,h=(v[k] for k in ('x','y','w','h'));scale=v['scale']
        m=chrome_geometry(text);inset=m['inset'];top=m['body_top']
        if x<0 or y<m['bar'] or w<=2*inset or h<=top+inset or x+w>width or y+h>height:raise ValueError('launcher outside framebuffer')
        columns=min(80,(w-2*inset)//(6*scale));limit=min(columns,40)-1
        if limit<2:raise ValueError('launcher too narrow')
        errors=[]
        for row,line in lines:
            line=line[:limit];y0=y+top+row*9*scale
            if y0+7*scale>y+h:raise ValueError('launcher line outside surface')
            mismatch=0
            for col in range(columns):
                mismatch+=cell_errors(pixels,width,x+inset+col*6*scale,y0,line[col] if col<len(line) else ' ',scale,b'\xf0\xf8\xff',font)[0]
            if mismatch:errors.append(f'launcher line={row} glyph_mismatches={mismatch}')
        return errors
    except (ValueError,KeyError,OverflowError) as error:return [str(error)]
