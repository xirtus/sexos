#!/usr/bin/env python3
"""IPC v2 on a booted system: `run bench ipc2` creates an endpoint, serves it from a
worker with RECV / REPLY_RECV, checks badges, rights, one-way sends, notifications and
destroy, then times call round trips (syscall 105) and signal+poll (106)."""
import argparse,json,os,re,shutil,time
from pathlib import Path
os.environ.setdefault('SEXOS_GATE_BOUND','900')
from install_program import install
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 install(disk,[('bench',(ROOT/'iso_root/programs/bench').read_bytes())])
 g=boot(a.iso,disk,a.out,'bench',a.resolution);t=g['text'];q=g['q']
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  g['focus'](153);time.sleep(.5);s=len(t())
  for k in 'run bench ipc2':q.key('spc' if k==' ' else k);time.sleep(.05)
  q.key('ret')
  m=g['wait'](r'\[bench\.ipc2\.(done|fail)\][^\n]*\n',s,600);assert m[1]=='done',m[0]
  results={n:int(c) for n,c in re.findall(r'\[bench\] name=(ipc2_\w+) ops=\d+ cycles=(\d+)',t()[s:])}
  assert set(results)=={'ipc2_roundtrip','ipc2_signal_poll'},results
  (a.out/'ipc2.json').write_text(json.dumps(results));print({'ok':True,'cores':os.environ.get('SEXOS_GATE_CORES','1'),**results})
 finally:g['stop']()
if __name__=='__main__':main()
