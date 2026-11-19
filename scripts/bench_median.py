#!/usr/bin/env python3
"""Prints the median of repeated bench runs (kernel study Phase 0). Each run directory is
named <config>-<n> and holds the bench.json written by program_bench_gate.py."""
import json,statistics,sys
from pathlib import Path
def medians(root):
 groups={}
 for f in sorted(Path(root).glob('*/bench.json')):
  groups.setdefault(f.parent.name.rsplit('-',1)[0],[]).append(json.loads(f.read_text()))
 out={}
 for name,runs in groups.items():
  keys=[k for k,v in runs[0].items() if isinstance(v,int)]
  out[name]={'runs':len(runs),**{k:int(statistics.median(r[k] for r in runs)) for k in keys},'spread':{k:[min(r[k] for r in runs),max(r[k] for r in runs)] for k in keys}}
 return out
if __name__=='__main__':
 m=medians(sys.argv[1])
 for name,row in m.items():print(f'[bench.kvm.median] {name} '+json.dumps(row))
 (Path(sys.argv[1])/'medians.json').write_text(json.dumps(m,indent=1))
