"""Validate the complete admitted Quil text and caret against native glyph pixels."""
import re
import time
from check_desktop_pixels import glyph
from check_disk_persistence import ppm_rgb

def view(text,target=9):
 matches=list(re.finditer(rf'\[quil.ui.view\] target={target} surface=(\d+) paint=(\d+) w=(\d+) h=(\d+) top=(\d+) cols=(\d+) rows=(\d+) cursor=(\d+) view=(\d+)',text))
 if not matches:raise ValueError('no Quil view')
 m=matches[-1];v=dict(zip(['surface','paint','w','h','top','cols','rows','cursor','view'],map(int,m.groups())))
 admissions=re.findall(r'\[quil.ui.admitted\] paint=(\d+) bytes=(\d+) presentation=unconfirmed',text)
 if not admissions or int(admissions[-1][0])!=v['paint']:raise ValueError('latest Quil paint not fully admitted')
 chunks=re.findall(rf'\[quil.ui.text\] paint={v["paint"]} off=(\d+) hex=([0-9a-f]+)',text)
 raw=bytearray()
 for off,hex_ in chunks:
  if int(off)!=len(raw):raise ValueError('missing/duplicate text chunk')
  raw.extend(bytes.fromhex(hex_))
 if len(raw)!=int(admissions[-1][1]):raise ValueError('wrong admitted length')
 if not raw or len(raw)>512:raise ValueError('empty/oversize text')
 v['text']=raw.decode('ascii')
 if target==9:
  g=re.findall(r'\[shell.app.viewport\] target=9 sid=156 x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',text)
  if not g:raise ValueError('missing current owned-content geometry')
  x,y,w,h=map(int,g[-1]);v['x']=x;v['y']=y
  if(v['w'],v['h'])!=(w,h):raise ValueError('view not current content geometry')
 else:
  g=re.findall(r'\[shell.tile.apply\] sid=301 x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',text)
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

def verify(text,ppm,font,expected=None,target=9,resolution=None):
 try:
  v=view(text,target);width,height,pixels=ppm_rgb(ppm,min_width=1,min_height=1)
  if resolution and (width,height)!=tuple(resolution):raise ValueError('wrong actual framebuffer resolution')
  if expected and expected not in v['text']:raise ValueError('wrong visible text')
  scale=2 if v['w']>=256 else 1;color=b'\xe0\xf0\xff';mismatch=ink=0
  lines=v['text'].split('\n')
  for row,line in enumerate(lines):
   if len(line)>v['cols']:raise ValueError('line exceeds viewport')
   for col,ch in enumerate(line):
    for gy,bits in enumerate(glyph(font,ch)):
     for gx in range(6):
      on=gx<5 and bool(bits&(1<<(4-gx)))
      for dy in range(scale):
       for dx in range(scale):
        x=v['x']+8+(col*6+gx)*scale+dx;y=v['y']+v['top']+(row*9+gy)*scale+dy
        if not(0<=x<width and 0<=y<height and x<v['x']+v['w'] and y<v['y']+v['h']):raise ValueError('glyph clipped by viewport')
        mismatch+=(pixels[(y*width+x)*3:(y*width+x)*3+3]==color)!=on;ink+=on
  if not ink:raise ValueError('no glyph ink')
  return [] if not mismatch else [f'Quil glyph mismatch={mismatch}']
 except (ValueError,OverflowError,KeyError,IndexError,UnicodeError)as e:return[str(e)]
