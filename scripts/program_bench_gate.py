#!/usr/bin/env python3
"""Runs the bench program and prints its numbers: cycles per system call, yield, MiB
allocated, copied, written and read, and per file stat. It passes when every test
finished; the numbers are for the limits report and for comparing with
tools/bench/linux/run.sh on the same emulated machine."""
import argparse,json,os,re,shutil,time
from pathlib import Path
os.environ.setdefault('SEXOS_GATE_BOUND','1200')
from install_program import install
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');p.add_argument('--large-read',action='store_true');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 install(disk,[('bench',(ROOT/'iso_root/programs/bench').read_bytes())])
 g=boot(a.iso,disk,a.out,'bench',a.resolution);t=g['text'];q=g['q']
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  g['focus'](153);time.sleep(.5);s=len(t())
  for k in ('run bench read-large' if a.large_read else 'run bench'):q.key('spc' if k==' ' else 'minus' if k=='-' else k);time.sleep(.05)
  q.key('ret')
  if a.large_read:
   m=g['wait'](r'\[bench\.large\.(done|fail)\][^\n]*\n',s,900);assert m[1]=='done',m[0]
   assert re.search(r'\[bench.large.baseline\] requests=0 device_calls=0 device_bytes=0\r?\n',t()[s:])
   fields=('pass','bytes','requests','device_calls','device_bytes','read_cycles','elapsed_ticks')
   rows=[dict(zip(fields,map(int,v))) for v in re.findall(r'\[bench.large.read\] pass=(\d+) bytes=(\d+) requests=(\d+) device_calls=(\d+) device_bytes=(\d+) read_cycles=(\d+) elapsed_ticks=(\d+) verified=1\r?\n',t()[s:])]
   assert len(rows)==3 and [r['pass'] for r in rows]==[0,1,2],rows
   for r in rows:assert r['bytes']==r['device_bytes']==16*1024**2 and r['requests']==256 and r['device_calls']>0 and r['read_cycles']>0,r
   (a.out/'bench-large.json').write_text(json.dumps(rows,indent=2));print({'ok':True,'cores':os.environ.get('SEXOS_GATE_CORES','1'),'large_read':rows});return
  m=g['wait'](r'\[bench\.(done|fail)\][^\n]*\n',s,900);assert m[1]=='done',m[0]
  results={n:int(c) for n,c in re.findall(r'\[bench\] name=(\w+) ops=\d+ cycles=(\d+)',t()[s:])}
  assert len(results)==10,results
  (a.out/'bench.json').write_text(json.dumps(results));print({'ok':True,'cores':os.environ.get('SEXOS_GATE_CORES','1'),**results})
 finally:g['stop']()
if __name__=='__main__':main()
