"""Rebuild a Spindle pane's text from its serial record.

Spindle writes `[spindle.view.text] ... bytes=N base=P` for each paint, then one
`[spindle.view.bytes] ... off=O hex=...` line per 48-byte chunk. With base=0 (or no
base, older records) every chunk is written. Otherwise only the chunks that differ
from paint P of the same sequence are written, so a keystroke costs one or two
lines of serial output instead of a whole pane."""
import re


def pane(trace,target,sid,seq,paint):
 """The pane's bytes at `paint`, or ValueError when the record cannot rebuild them."""
 heads={};chunks={}
 for p,n,base in re.findall(rf'\[spindle.view.text\] target={target} sid={sid} sequence={seq} paint=(\d+) cols=\d+ rows=\d+ bytes=(\d+)(?: base=(\d+))?',trace):
  heads[int(p)]=(int(n),int(base or 0));chunks[int(p)]={}
 for p,off,data in re.findall(rf'\[spindle.view.bytes\] target={target} sid={sid} seq={seq} paint=(\d+) off=(\d+) hex=([0-9a-f]*)',trace):
  if int(p)in chunks:
   if int(off)in chunks[int(p)]:raise ValueError('duplicated viewport byte chunk')
   chunks[int(p)][int(off)]=bytes.fromhex(data)
 chain=[];p=int(paint)
 while True:
  if p not in heads:raise ValueError(f'missing Spindle paint {p} of sequence {seq}')
  chain.append(p);base=heads[p][1]
  if base==0:break
  if base>=p:raise ValueError('Spindle paint names a later base')
  p=base
 data=bytearray()
 for p in reversed(chain):
  n,base=heads[p]
  if base==0:
   data=bytearray()
   for off in sorted(chunks[p]):
    part=chunks[p][off]
    if off!=len(data) or len(part)!=min(48,n-off):raise ValueError('partial or duplicated viewport byte chunks')
    data.extend(part)
  else:
   if len(data)!=n:raise ValueError('Spindle delta paint changes the pane size')
   for off,part in chunks[p].items():
    if off%48 or len(part)!=min(48,n-off):raise ValueError('partial viewport delta chunk')
    data[off:off+len(part)]=part
  if len(data)!=n:raise ValueError('incomplete actual Spindle pane')
 return bytes(data)
