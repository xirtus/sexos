"""The Geist Mono cells Display draws text with, for pixel oracles: a cell's fully
covered pixels must be exactly the text colour. The cells come from sex-font's own
renderer (examples/cells.rs), the code Display runs, built for the host."""
import subprocess
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
W,H=12,18
_cells=None
def cells():
 global _cells
 if _cells is None:
  out=subprocess.run(['cargo','run','-q','--manifest-path',str(ROOT/'Cargo.toml'),'-p','sex-font','--example','cells','--',str(W),str(H)],check=True,capture_output=True,text=True).stdout
  _cells=[bytes(map(int,row.split(','))) for row in out.split()]
  assert len(_cells)==95 and all(len(c)==W*H for c in _cells)
 return _cells
def solid(ch):
 """Pixels of `ch` with full coverage, as (dx, dy) from the cell's top left."""
 c=cells()[ord(ch)-32] if 32<=ord(ch)<=126 else bytes(W*H)
 return [(i%W,i//W) for i,a in enumerate(c) if a==255]
