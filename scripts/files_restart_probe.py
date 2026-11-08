#!/usr/bin/env python3
"""Files falls over and comes back: what was saved before is still there, and new work saves. Needs an ISO built with SEXOS_FILES_CRASH_PROBE=1."""
import argparse,re,shutil,time
from pathlib import Path
from quil_autosave_gate import boot
from quil_workflow_gate import palette_select,palette_commands
ROOT=Path(__file__).resolve().parents[1]
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 g=boot(a.iso,disk,a.out,'files',a.resolution);t=g['text'];q=g['q']
 def expect(text,pattern,timeout=60):
  g['focus'](153);time.sleep(.5);s=len(t())
  for k in text:q.key(KEYS.get(k,k));time.sleep(.05)
  q.key('ret');return g['wait'](pattern,s,timeout)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  expect('write keep.txt saved before',r'\[spindle.fs.result\] kind=8 status=0')
  expect('cat crash.probe',r'\[files.crash.probe\]')
  m=g['wait'](r'\[service.restarted\] pd=(\d+) task=\d+ restarts=1 ',0,60)
  g['wait'](r'\[spindle.fs.expired\]',0,60)
  expect('cat keep.txt',r'\[spindle.fs.cat\] saved before',90)
  expect('write after.txt written after',r'\[spindle.fs.result\] kind=8 status=0',90)
  expect('cat after.txt',r'\[spindle.fs.cat\] written after',90)
  # Programs still start and close: their cleanup needs the new run's acknowledgement.
  palette_select(g['key'],'FocusProgram',palette_commands(ROOT));s=len(t());q.key('ret')
  r=g['wait'](r'\[hello.ready\] target=(\d+) task=(\d+)',s,60);time.sleep(2)
  g['focus'](int(re.findall(rf'\[shell.program.launch\] target={r[1]} task={r[2]} surface=(\d+)',t())[-1]))
  s=len(t());q.key('f11');g['wait'](rf'\[app.private.retired\] target={r[1]} task={r[2]} ',s,90)
  assert '[files.fs.mounted]' in t() and t().count('[files.fs.formatted]')<=1,'the filesystem was formatted again'
  assert 'KERNEL PANIC' not in t()
  print({'ok':True,'files_pd':int(m[1])})
 finally:g['stop']()
if __name__=='__main__':main()
