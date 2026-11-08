"""The Geist Mono cells Display draws text with, for pixel oracles: a cell's fully
covered pixels must be exactly the text colour. The cells come from sex-font's own
renderer (examples/cells.rs), the code Display runs, built for the host."""
import subprocess
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
W,H=12,18
_cells={}
def cells(w=W,h=H):
 """Cells for a w x h grid: Display's text scale s draws 6s x 9s cells."""
 if (w,h) not in _cells:
  out=subprocess.run(['cargo','run','-q','--manifest-path',str(ROOT/'Cargo.toml'),'-p','sex-font','--example','cells','--',str(w),str(h)],check=True,capture_output=True,text=True).stdout
  got=[bytes(map(int,row.split(','))) for row in out.split()]
  assert len(got)==95 and all(len(c)==w*h for c in got)
  _cells[(w,h)]=got
 return _cells[(w,h)]
def solid(ch,w=W,h=H):
 """Pixels of `ch` with full coverage, as (dx, dy) from the cell's top left."""
 c=cells(w,h)[ord(ch)-32] if 32<=ord(ch)<=126 else bytes(w*h)
 return [(i%w,i//w) for i,a in enumerate(c) if a==255]
