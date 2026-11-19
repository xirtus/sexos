"""Validate the complete admitted Quil text and caret against native glyph pixels."""
import re
import time
import hashlib
from pathlib import Path
import unicodedata
from check_desktop_pixels import glyph, cell_coverage, chrome_geometry
from native_text_metrics import source_metrics, font_sources
from native_scalar_cells import coverage as scalar_coverage
from check_disk_persistence import ppm_rgb

def view(text,target=9):
 matches=list(re.finditer(rf'\[quil.ui.view\] target={target} surface=(\d+) paint=(\d+) w=(\d+) h=(\d+) top=(\d+) cols=(\d+) rows=(\d+) cursor=(\d+) view=(\d+)',text))
 if not matches:raise ValueError('no Quil view')
 m=matches[-1];v=dict(zip(['surface','paint','w','h','top','cols','rows','cursor','view'],map(int,m.groups())))
 admissions=re.findall(r'\[quil.ui.admitted\] paint=(\d+) bytes=(\d+) presentation=unconfirmed',text[m.start():])
 if not admissions or int(admissions[-1][0])!=v['paint']:raise ValueError('latest Quil paint not fully admitted')
 chunks=re.findall(rf'\[quil.ui.text\] paint={v["paint"]} off=(\d+) hex=([0-9a-f]+)',text[m.start():])
 raw=bytearray()
 for off,hex_ in chunks:
  if int(off)!=len(raw):raise ValueError('missing/duplicate text chunk')
  raw.extend(bytes.fromhex(hex_))
 if len(raw)!=int(admissions[-1][1]):raise ValueError('wrong admitted length')
 if not raw or len(raw)>6144:raise ValueError('empty/oversize text')
 v['text']=raw.decode('utf8')
 v['applied']=re.search(rf'\[quil.ui.applied\] paint={v["paint"]} bytes={len(raw)} pixels=unconfirmed',text[m.start():])is not None
 published=re.findall(rf'\[shell.document.viewport\] target={target} task=[1-9]\d* surface={v["surface"]} sequence=[1-9]\d* x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+) enqueued=1',text)
 if published:
  x,y,w,h=map(int,published[-1]);v['x']=x;v['y']=y
  if(v['w'],v['h'])!=(w,h):raise ValueError('view not current published viewport')
 elif target==9:
  g=re.findall(r'\[shell.app.viewport\] target=9 sid=156 x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',text)
  if not g:raise ValueError('missing current owned-content geometry')
  x,y,w,h=map(int,g[-1]);v['x']=x;v['y']=y
  if(v['w'],v['h'])!=(w,h):raise ValueError('view not current content geometry')
 else:
  g=re.findall(rf'\[shell.tile.apply\] sid={v["surface"]} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',text)
  if not g:raise ValueError('missing successor frame geometry')
  x,y,w,h=map(int,g[-1]);v['x']=x;v['y']=y
  if(v['w'],v['h'])!=(w,h):raise ValueError('view not current successor geometry')
 return v

def wait_view(text, timeout=90, target=9):
 # A requested view may still be behind an immutable in-flight transaction.
 # Keep exact admission/geometry checks; absence must fail at the deadline.
 end=time.monotonic()+timeout;last='no view'
 while time.monotonic()<end:
  try:return view(text(),target)
  except ValueError as e:last=str(e)
  time.sleep(.05)
 raise RuntimeError('Quil view did not settle: '+last)

def source_contract(root=None):
 root=Path(root) if root is not None else Path(__file__).resolve().parents[1]
 metrics=source_metrics(root);hashes=font_sources(root)
 style=(root/'crates/sex-pdx/src/desktop_style.rs').read_text()
 def constant(name,expected):
  values=re.findall(r'pub const '+name+r'\s*:\s*u32\s*=\s*(0x[0-9a-fA-F]+|[0-9]+)\s*;',style)
  if len(values)!=1 or int(values[0],0)!=expected:raise ValueError('approved Quil metric/color changed: '+name)
  return int(values[0],0)
 c=dict(metrics=metrics,color=constant('TEXT',0xe0f0ff),background=constant('NATIVE_CANVAS',0x0c1420),text_top=constant('TEXT_TOP',12))
 names=('crates/sex-shape/src/cells.rs','crates/sex-shape/src/wide_table.rs','servers/quil/src/display_delivery.rs','crates/sex-pdx/src/document_view.rs')
 for name in names:hashes[name]=hashlib.sha256((root/name).read_bytes()).hexdigest()
 shape=re.sub(r'\s+','',(root/names[0]).read_text())
 glyphs=re.sub(r'\s+','',(root/'servers/sexdisplay/src/glyphs.rs').read_text())
 if '||fallback().is_some_and(|f|f.draw_cell(scalar,w,h,acc(),out))'not in glyphs:raise ValueError('native scalar fallback selection changed')
 if 'leti=table.partition_point(|&(_,last)|last<scalar);'not in shape or 'i<table.len()&&table[i].0<=scalar'not in shape:raise ValueError('wide cell mapping changed')
 if 'color: sex_pdx::desktop_style::TEXT,'not in (root/names[2]).read_text():raise ValueError('Quil text color source changed')
 c['wide']=[tuple(int(n,16)for n in pair)for pair in re.findall(r'\((0x[0-9a-fA-F]+),\s*(0x[0-9a-fA-F]+)\)',(root/names[1]).read_text())]
 if len(c['wide'])!=129 or any(a>b for a,b in c['wide'])or any(c['wide'][i-1][1]>=a for i,(a,_)in enumerate(c['wide'])if i):raise ValueError('invalid source wide table')
 catalog=(root/'crates/sex-pdx/src/app_catalog.rs').read_text()
 c['quil']={int(target):(int(surface),frame!='None')for target,surface,frame in re.findall(r'^\s*quil\(\d+,\s*(\d+),\s*(\d+),\s*(None|Some\(\d+\))',catalog,re.M)}
 if len(c['quil'])!=16:raise ValueError('missing exact Quil catalog identities')
 c.update(source_sha256=hashes,root=root);return c

def coverage(font,char,scale,span,c):
 if 32<=ord(char)<=126:return cell_coverage(font,char,scale)
 w=6*scale*span;h=9*scale
 if char!='\t':
  result=scalar_coverage(ord(char),w,h,c['root'])
  if result is not None:return result
 # Display's explicit recovery bitmap is used only when no font has the scalar.
 bitmap=glyph(font,char)
 return bytes(255 if y//scale<7 and x//scale<5 and bitmap[y//scale]&(1<<(4-x//scale))else 0 for y in range(h)for x in range(w))

def verify(text,ppm,font,expected=None,target=9,resolution=None):
 try:
  v=view(text,target);width,height,pixels=ppm_rgb(ppm,min_width=1,min_height=1)
  if resolution and (width,height)!=tuple(resolution):raise ValueError('wrong actual framebuffer resolution')
  if not v['applied']:raise ValueError('latest Quil paint not application-acknowledged')
  if expected and expected not in v['text']:raise ValueError('wrong visible text')
  c=source_contract();m=c['metrics'];ui=chrome_geometry(text)['scale'];scale=(2 if v['w']>=m['scale_threshold']else 1)*ui
  if ui not in (1,2,3):raise ValueError('unsupported Quil UI scale')
  if target not in c['quil']or v['surface']!=c['quil'][target][0]:raise ValueError('Quil catalog surface identity differs')
  top=(c['text_top']if c['quil'][target][1]else m['top'])*ui
  if v['top']!=top:raise ValueError('Quil source text origin changed')
  if not(0<=v['x']<width and 0<=v['y']<height and v['x']+v['w']<=width and v['y']+v['h']<=height):raise ValueError('Quil viewport clipped')
  color=c['color'].to_bytes(3,'big');background=c['background'].to_bytes(3,'big')
  body_h=v['h']-v['top'];body_w=v['w']
  if body_h<=0:raise ValueError('empty Quil body')
  wanted=bytearray(background*(body_w*body_h));occupied=bytearray(body_w*body_h);ink=0
  for row,line in enumerate(v['text'].split('\n')):
   column=0
   for ch in line:
    # These fixtures use logical left-to-right text. Bidi/shaped-script proof
    # requires a visual-order oracle rather than pretending scalar order suffices.
    if unicodedata.bidirectional(ch)in ('R','AL','AN','LRE','RLE','LRO','RLO','PDF','LRI','RLI','FSI','PDI','BN'):raise ValueError('Quil bidi text requires visual-order oracle')
    span=2 if any(a<=ord(ch)<=b for a,b in c['wide'])else 1
    if column+span>v['cols']:raise ValueError('line exceeds viewport')
    cell=coverage(font,ch,scale,span,c);cw=m['advance']*scale*span
    x0=m['inset']*ui+column*m['advance']*scale;y0=row*m['line']*scale
    for i,alpha in enumerate(cell):
     x=x0+i%cw;y=y0+i//cw
     if x>=body_w or y>=body_h:
      if alpha:raise ValueError('glyph clipped by viewport')
      continue
     at=y*body_w+x;occupied[at]=1
     wanted[at*3:at*3+3]=bytes((fg*alpha+bg*(255-alpha))//255 for fg,bg in zip(color,background));ink+=alpha>0
    column+=span
  if not ink:raise ValueError('no glyph ink')
  mismatch=stray=0
  for y in range(body_h):
   start=((v['y']+v['top']+y)*width+v['x'])*3
   actual=pixels[start:start+body_w*3];want=wanted[y*body_w*3:(y+1)*body_w*3]
   if actual==want:continue
   for x in range(body_w):
    if actual[x*3:x*3+3]!=want[x*3:x*3+3]:
     if occupied[y*body_w+x]:mismatch+=1
     else:stray+=1
  return ([]if not mismatch else[f'Quil glyph mismatch={mismatch}'])+([]if not stray else[f'Quil stray text pixels={stray}'])
 except (ValueError,OverflowError,KeyError,IndexError,UnicodeError)as e:return[str(e)]
