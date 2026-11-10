"""The Geist Mono cells Display draws text with, for pixel oracles: a cell's fully
covered pixels must be exactly the text colour. The cells come from sex-font's own
renderer (examples/cells.rs), the code Display runs, built for the host."""
import subprocess
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
W,H=12,18
_cells={}
def cells(w=W,h=H,font="mono",root=ROOT):
 """Cells for a w x h grid: Display's text scale s draws 6s x 9s cells."""
 root=Path(root).resolve()
 if (w,h,font,root) not in _cells:
  out=subprocess.run(['cargo','run','-q','--manifest-path',str(root/'Cargo.toml'),'-p','sex-font','--example','cells','--',str(w),str(h),font],check=True,capture_output=True,text=True).stdout
  got=[bytes(map(int,row.split(','))) for row in out.split()]
  assert len(got)==95 and all(len(c)==w*h for c in got)
  _cells[(w,h,font,root)]=got
 return _cells[(w,h,font,root)]
def solid(ch,w=W,h=H):
 """Pixels of `ch` with full coverage, as (dx, dy) from the cell's top left."""
 c=cells(w,h)[ord(ch)-32] if 32<=ord(ch)<=126 else bytes(w*h)
 return [(i%w,i//w) for i,a in enumerate(c) if a==255]

def captions(root=ROOT):
 """Natural 9px Sans metrics and four coverage phases, independent of Display's cache."""
 root=Path(root).resolve();key=('caption',root)
 if key not in _cells:
  out=subprocess.run(['cargo','run','-q','--manifest-path',str(root/'Cargo.toml'),'-p','sex-font','--example','captions'],check=True,capture_output=True,text=True).stdout
  lines=[list(map(int,row.split(',')))for row in out.split()]
  assert len(lines)==476 and len(lines[0])==95
  assert all(len(row)==95 for row in lines[1:96])
  assert all(len(row)==162 and all(0<=v<=255 for v in row)for row in lines[96:])
  _cells[key]=dict(advances=lines[0],kerning=lines[1:96],coverage=[[lines[96+c*4+p]for p in range(4)]for c in range(95)])
 return _cells[key]

def caption_bitmap(atlas,text,w=80,h=9,left=2):
 """Compose natural advances and quarter-pixel glyphs; no Display layout/cache code."""
 raw=[0]*(w*h);pen=0;previous=None
 for ch in text:
  c=ord(ch)-32;assert 0<=c<95
  if previous is not None:pen+=atlas['kerning'][previous][c]
  x0=left+(pen>>8)-2;phase=(pen&255)>>6;glyph=atlas['coverage'][c][phase]
  for y in range(min(9,h)):
   for x in range(18):
    xx=x0+x
    if 0<=xx<w:raw[y*w+xx]=min(255,raw[y*w+xx]+glyph[y*18+x])
  pen+=atlas['advances'][c];previous=c
 return raw
