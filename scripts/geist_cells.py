"""The Geist Mono cells Display draws text with, read from the generated table, for
pixel oracles: a cell's fully covered pixels must be exactly the text colour."""
import re
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
W,H=12,18
_cells=None
def cells():
 global _cells
 if _cells is None:
  src=(ROOT/'crates/sex-pdx/src/font_geist.rs').read_text()
  body=src[src.index('pub static ASCII'):src.index('\n];',src.index('pub static ASCII'))]
  _cells=[bytes(map(int,row.split(','))) for row in re.findall(r'\[([0-9,]+)\]',body)]
  assert len(_cells)==95 and all(len(c)==W*H for c in _cells)
 return _cells
def solid(ch):
 """Pixels of `ch` with full coverage, as (dx, dy) from the cell's top left."""
 c=cells()[ord(ch)-32] if 32<=ord(ch)<=126 else bytes(W*H)
 return [(i%W,i//W) for i,a in enumerate(c) if a==255]
