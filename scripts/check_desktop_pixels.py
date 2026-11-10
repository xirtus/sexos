#!/usr/bin/env python3
"""Check actual Linen filename glyphs against its final committed view intent.

A log record alone cannot pass: the framebuffer must contain each complete
visible name, and its selected marker, using the renderer's source-bound font.
"""
from pathlib import Path
import argparse
import re
from check_disk_persistence import ppm_rgb


def font_from_source(source):
    match=re.search(r'const FONT_ASCII_5X7:.*?= \[(.*?)\n\];',source,re.S)
    if not match:raise ValueError('missing renderer font')
    rows=[]
    for row in re.findall(r'\[([^\[\]]+)\]',re.sub(r'//[^\n]*','',match[1])):
        bits=re.findall(r'0b[01]+|0x[0-9a-fA-F]+|[0-9]+',row)
        if len(bits)!=7:raise ValueError('invalid renderer glyph')
        rows.append(tuple(int(bit,0) for bit in bits))
    if len(rows)!=95 or any(not 0<=bit<32 for row in rows for bit in row):raise ValueError('invalid printable-ASCII renderer font')
    if any(rows[0]) or any(not any(row)for row in rows[1:]):raise ValueError('missing printable glyph ink')
    extra=re.search(r'const FONT_UNICODE_5X7:.*?= &\[(.*?)\n\];',source,re.S)
    fallback=re.search(r'const FONT_UNSUPPORTED_5X7: \[u8; 7\] = \[([^\]]+)\];',source)
    if not extra or not fallback:raise ValueError('missing scalar renderer font')
    def row(bits):
        values=tuple(int(v.strip(),0)for v in bits.split(','))
        if len(values)!=7 or any(not 0<=v<32 for v in values):raise ValueError('invalid scalar glyph')
        return values
    unicode={}
    entries=re.findall(r'\((0x[0-9A-Fa-f]+), \[([^\]]+)\]\)',re.sub(r'//[^\n]*','',extra[1]))
    for code,bits in entries:
        scalar=int(code,0)
        if not 128<=scalar<=0x10ffff or 0xd800<=scalar<=0xdfff or str(scalar)in unicode:raise ValueError('invalid scalar repertoire')
        unicode[str(scalar)]=row(bits)
    if not unicode or len(entries)!=len(unicode):raise ValueError('missing scalar repertoire')
    unsupported=row(fallback[1])
    if not any(unsupported):raise ValueError('invisible unsupported scalar')
    if 'FONT_UNICODE_5X7.iter().find'not in source or 'map_or(FONT_UNSUPPORTED_5X7'not in source:raise ValueError('scalar font not bound to renderer')
    # Archive contracts without this field retain the old scale-2 threshold.
    # Bind the changed default to the actual scalar shader, not today's checker.
    shader=source.split('fn surface_text_fg_at(',1)[-1].split('fn civil_clock_date(',1)[0]
    shader=re.sub(r'//[^\n]*','',shader)
    minimum=1 if 'glyphs::mono_cell'in shader and not re.search(r'if\s+scale\s*>=\s*2\s*\{',shader) else 2
    return dict(ascii=rows,unicode=unicode,fallback=unsupported,geist_min_scale=minimum)


def glyph(font,char):
    # Independent scalar-to-ROM mapping. A finite repertoire and fallback are
    # read from the actual renderer source and survive JSON contract freezing.
    if len(char)!=1 or char=='\n':raise ValueError('expected one non-newline scalar')
    if isinstance(font,list):
        # Historical ASCII contracts have no source-bound Unicode fallback.
        if len(font)!=95 or not 32<=ord(char)<=126:raise ValueError('unsupported historical ASCII glyph')
        return font[ord(char)-32]
    if len(font['ascii'])!=95:raise ValueError('invalid ASCII font')
    if char=='\t':return (0,)*7
    if 32<=ord(char)<=126:return font['ascii'][ord(char)-32]
    return font['unicode'].get(str(ord(char)),font['fallback'])


def text_cells(raw):
    """Independent byte spans; malformed read-only bytes each show one box."""
    result=[];at=0
    while at<len(raw):
        first=raw[at];size=1 if first<128 else 2 if 0xc2<=first<=0xdf else 3 if 0xe0<=first<=0xef else 4 if 0xf0<=first<=0xf4 else 0
        try:
            if not size or at+size>len(raw):raise UnicodeError('malformed scalar')
            scalar=raw[at:at+size].decode('utf8')
        except UnicodeError:scalar='\ufffd';size=1
        result.append((scalar,at,at+size));at+=size
    return result


def final_view(text):
    groups=list(re.finditer(r'\[linen\.list\.text\] ([^\r\n\[]*)',text))
    if not groups:raise ValueError('missing filename view intent')
    record=groups[-1]
    fields=dict(re.findall(r'(\w+)=([^\s]+)',record[1]))
    labels=[]
    for row in re.finditer(r'\[linen\.list\.label\] id=(\d+) row=(\d+) text=([^\r\n]*)',text[record.end():]):
        labels.append((int(row[1]),int(row[2]),row[3]))
    if fields.get('committed_view')!='1':raise ValueError('view was not committed')
    values={key:int(fields[key],0) for key in ['x','y','w','h','scale','text_color','rows','selected','row_y','row_h']}
    if not 1<=values['rows']<=5 or values['scale'] not in (1,2,3,4,6):raise ValueError('invalid view bounds')
    if len(labels)!=values['rows'] or {r for _,r,_ in labels}!=set(range(values['rows'])):raise ValueError('incomplete name intents')
    if len({i for i,_,_ in labels})!=len(labels) or not all(i>0 and name for i,_,name in labels):raise ValueError('invalid name identities')
    if values['selected'] not in {i for i,_,_ in labels}:raise ValueError('selected object is not visible')
    return values,labels


def cell_errors(pixels,width,x0,y0,char,scale,color,font,clip=None):
    """(mismatches, ink) for one text cell at (x0, y0). Windows at text scale 2 and up
    draw Geist Mono: fully covered pixels must be the text colour, uncovered ones must
    not be. Compact cells compare their complete antialias coverage; archived contracts
    retain their source-bound fallback threshold. `clip` (x_end, y_end) skips
    pixels a pane cuts off."""
    inside=lambda x,y:clip is None or (x<clip[0] and y<clip[1])
    mismatch=ink=0
    minimum=font.get('geist_min_scale',2)if isinstance(font,dict)else 2
    if scale>=minimum and (char==' ' or 32<=ord(char)<=126):
        import geist_cells
        w,h=6*scale,9*scale
        cell=geist_cells.cells(w,h)[ord(char)-32]
        # Compact Geist cells may have no fully opaque pixel. Their background
        # is the uniform native canvas/selection, observed from zero-coverage
        # pixels; compare every antialiased pixel instead of accepting no ink.
        background=None
        if scale==1:
            from collections import Counter
            empty=[bytes(pixels[((y0+i//w)*width+x0+i%w)*3:((y0+i//w)*width+x0+i%w)*3+3])for i,a in enumerate(cell)if a==0 and inside(x0+i%w,y0+i//w)]
            if not empty:return 1,0
            background=Counter(empty).most_common(1)[0][0]
        for i,a in enumerate(cell):
            if scale!=1 and 0<a<255:continue
            x=x0+i%w;y=y0+i//w
            if not inside(x,y):continue
            actual=pixels[(y*width+x)*3:(y*width+x)*3+3]
            if scale==1:
                expected=bytes((fg*a+bg*(255-a))//255 for fg,bg in zip(color,background))
                mismatch+=actual!=expected;ink+=a>0 and actual!=background
            else:
                mismatch+=(actual==color)!=(a==255);ink+=a==255
        return mismatch,ink
    bitmap=glyph(font,char) if char!=' ' else (0,)*7
    for gy,bits in enumerate(bitmap):
        for gx in range(6):
            on=gx<5 and bool(bits&(1<<(4-gx)))
            for dy in range(scale):
                for dx in range(scale):
                    x=x0+gx*scale+dx;y=y0+gy*scale+dy
                    if not inside(x,y):continue
                    actual=pixels[(y*width+x)*3:(y*width+x)*3+3]==color
                    mismatch+=actual!=on;ink+=on
    return mismatch,ink


def cell_coverage(font,char,scale):
    """Independent source-selected Mono raster, or the finite recovery bitmap."""
    minimum=font.get('geist_min_scale',2)if isinstance(font,dict)else 2
    w,h=6*scale,9*scale
    if scale>=minimum and 32<=ord(char)<=126:
        import geist_cells
        return geist_cells.cells(w,h)[ord(char)-32]
    bitmap=glyph(font,char)if char!=' 'else(0,)*7
    return bytes(255 if y//scale<7 and x//scale<5 and bitmap[y//scale]&(1<<(4-x//scale))else 0 for y in range(h)for x in range(w))

def draw_cell(pixels,width,x0,y0,char,scale,color,font,clip=None):
    """Synthetic fixtures paint complete AA cells from the pinned font atlas."""
    cell=cell_coverage(font,char,scale);w=6*scale
    for i,alpha in enumerate(cell):
        x=x0+i%w;y=y0+i//w
        if clip is not None and not(clip[0]<=x<clip[2]and clip[1]<=y<clip[3]):continue
        at=(y*width+x)*3;back=pixels[at:at+3]
        pixels[at:at+3]=bytes((fg*alpha+bg*(255-alpha))//255 for fg,bg in zip(color,back))


def chrome_geometry(text):
    """Archived traces retain their original 50/36/8 geometry contract."""
    if '[display.chrome.metrics]' not in text:
        return dict(scale=1,bar=50,body_top=36,inset=8)
    from browser_gate_viewport import chrome_metrics
    return chrome_metrics(text)


def verify(text,ppm,font,resolution=None):
    errors=[]
    try:
        view,labels=final_view(text)
        width,height,pixels=ppm_rgb(ppm,**({'min_width':1,'min_height':1}if resolution else{}))
        if resolution and (width,height)!=tuple(resolution):raise ValueError('wrong actual framebuffer resolution')
        metric=chrome_geometry(text)
        if view['x']<0 or view['y']<metric['bar'] or view['w']<=0 or view['h']<=0 or view['x']+view['w']>width or view['y']+view['h']>height:raise ValueError('view outside framebuffer')
        color=view['text_color'].to_bytes(3,'big');scale=view['scale']
        # Match the renderer's width-derived wrap bound. A correct prefix is
        # insufficient: an older, longer filename must not survive to its right.
        columns=min(80,(view['w']-2*metric['inset'])//(6*scale))
        for identity,row,name in labels:
            rendered=('> ' if identity==view['selected'] else '  ')+name
            x0=view['x']+metric['inset'];y0=view['y']+view['row_y']+row*view['row_h']
            if len(rendered)>columns or y0<view['y'] or y0+7*scale>view['y']+view['h']:raise ValueError('label outside view')
            mismatch=0;ink=0
            for column in range(columns):
                m,i=cell_errors(pixels,width,x0+column*6*scale,y0,rendered[column] if column<len(rendered) else ' ',scale,color,font)
                mismatch+=m;ink+=i
            if mismatch or ink==0:errors.append(f'name_id={identity} glyph_mismatches={mismatch} ink={ink}')
        return errors
    except (ValueError,KeyError,OverflowError,UnicodeError) as exc:
        return [str(exc)]


def verify_hidden(previous_view_text,ppm):
    """Require the previously visible Linen text color to disappear on hide."""
    try:
        view,_=final_view(previous_view_text)
        width,height,pixels=ppm_rgb(ppm)
        x,y,w,h=(view[k] for k in ('x','y','w','h'))
        metric=chrome_geometry(previous_view_text)
        if x<0 or y<metric['bar'] or w<=0 or h<=0 or x+w>width or y+h>height:
            raise ValueError('view outside framebuffer')
        color=view['text_color'].to_bytes(3,'big')
        ink=sum(pixels[(row*width+col)*3:(row*width+col)*3+3]==color
                for row in range(y,y+h) for col in range(x,x+w))
        return [] if ink==0 else [f'hidden Linen retains text_color_pixels={ink}']
    except (ValueError,KeyError,OverflowError) as exc:
        return [str(exc)]


def verify_kaleido_layout(text,ppm,font,resolution=None):
    """Match the native app title/status in the latest complete tiled bounds.

    The caller separately waits for the app's authenticated presentation reply.
    This checks rendered geometry and glyphs, not just presentation log markers.
    """
    try:
        views=[]
        for record in re.finditer(r'\[shell\.tile\.apply\] ([^\[\r\n]*)',text):
            fields=dict(re.findall(r'(\w+)=([^\s]+)',record[1]))
            if fields.get('sid')=='300' and all(k in fields for k in ('x','y','w','h')):
                views.append(fields)
        if not views:raise ValueError('missing complete Kaleido tile intent')
        x,y,w,h=(int(views[-1][k],0) for k in ('x','y','w','h'))
        width,height,pixels=ppm_rgb(ppm,**({'min_width':1,'min_height':1}if resolution else{}))
        if resolution and (width,height)!=tuple(resolution):raise ValueError('wrong actual framebuffer resolution')
        metric=chrome_geometry(text)
        if x<0 or y<metric['bar'] or w<=0 or h<=0 or x+w>width or y+h>height:
            raise ValueError('Kaleido tile outside framebuffer')
        scale=(2 if w>=256 else 1)*metric['scale']
        columns=min(80,(w-2*metric['inset'])//(6*scale))
        lines=('Kaleidoscope','Cached / sample network text')
        errors=[]
        for row,line in enumerate(lines):
            y0=y+metric['body_top']+row*9*scale
            if len(line)>columns or y0+7*scale>y+h:
                raise ValueError('Kaleido title/status outside tile')
            mismatch=0
            for column in range(columns):
                m,_=cell_errors(pixels,width,x+metric['inset']+column*6*scale,y0,line[column] if column<len(line) else ' ',scale,b'\xcd\xd6\xf4',font)
                mismatch+=m
            if mismatch:errors.append(f'Kaleido line={row} glyph_mismatches={mismatch}')
        return errors
    except (ValueError,KeyError,OverflowError,UnicodeError) as exc:
        return [str(exc)]


def main():
    p=argparse.ArgumentParser();p.add_argument('log',type=Path);p.add_argument('screenshot',type=Path);p.add_argument('--font-source',type=Path,default=Path(__file__).resolve().parents[1]/'servers/sexdisplay/src/main.rs');a=p.parse_args()
    try:errors=verify(a.log.read_text(errors='replace'),a.screenshot.read_bytes(),font_from_source(a.font_source.read_text()))
    except (ValueError,OSError) as exc:errors=[str(exc)]
    for error in errors:print('[desktop.filename.pixels] FAIL',error)
    if not errors:print('[desktop.filename.pixels] PASS actual_names=1 selected_marker=1 font=production_framebuffer')
    return bool(errors)
if __name__=='__main__':raise SystemExit(main())
