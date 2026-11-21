#!/usr/bin/env python3
"""Tatami's R3 pages each round-trip one setting (workstream R3).

Settings (installed on the disk with Mesh and Abacus) opens; PgUp walks back from the
first page. Startup programs: Right on Mesh puts it in `startup.programs` (key 33).
Shortcuts: Left on the overview's row wraps to Escape (key 36 = 29); the Shell picks it
up from Files and Super+Esc then opens the overview instead of locking. Notifications:
Right on Abacus hides its notifications (key 40); `run abacus` makes the Shell ask Files
which program the row runs, and Files names Abacus (program 3), hidden. About: the page
shows the kernel's numbers (cores, domains) and, from Files (`sex_pdx::facts`, Tatami
only), the boot image's SHA-256, hashed a step per request. Updates and Storage, the
pages after Privacy, show
Files' slot state and the SexOS filesystem's size and free blocks. The second start on
the same disk reads the settings back, and the Shell starts Mesh by itself, once."""
import argparse,re,shutil,time
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot
from settings_persist_gate import first_start,evidence
ROOT=Path(__file__).resolve().parents[1]
PROGRAMS=('tatami','mesh','abacus')

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 install(disk,[(n,(ROOT/'iso_root/programs'/n).read_bytes()) for n in PROGRAMS])
 g=boot(a.iso,disk,a.out,'tatami-1',a.resolution);t=g['text'];q=g['q']
 def typ(line):
  for k in line:q.key('spc' if k==' ' else k);time.sleep(.05)
  q.key('ret')
 def keys(*names):
  for k in names:s=len(t());q.key(k);g['wait'](r'\[settings\.key\] code=',s,30);time.sleep(.2)
 def chord(*ks):
  for down,seq in ((True,ks),(False,tuple(reversed(ks)))):
   for k in seq:q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':down,'key':{'type':'qcode','data':k}}}]});time.sleep(.06)
 out={}
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(1)
  g['focus'](153);time.sleep(.5);s=len(t());typ('run tatami');first_start(g,q,t,s);g['wait'](r'\[settings\.open\] generation=\d+',s,120);time.sleep(1)
  # Startup programs: six pages back from the first (Updates and Storage come last). Mesh is the third program.
  keys('pgup','pgup','pgup','pgup','pgup','pgup','down','down')
  s=len(t());q.key('right');g['wait'](r'\[settings\.set\] key=33 value=4 status=0',s,30)
  g['wait'](r'\[files\.config\.set\] caller=\d+ key=33 value=4 status=0',s,30)
  # Shortcuts: the overview is the third row; Left from "default" wraps to Escape.
  keys('pgdn','down','down')
  s=len(t());q.key('left');g['wait'](r'\[settings\.set\] key=36 value=29 status=0',s,30)
  # Notifications: Abacus is the fourth program.
  keys('pgdn','down','down','down')
  s=len(t());q.key('right');g['wait'](r'\[settings\.set\] key=40 value=8 status=0',s,30)
  # About: four pages back from Notifications.
  s=len(t());keys('pgup','pgup','pgup','pgup')
  m=g['wait'](r'\[settings\.about\] cores=(\d+) domains=(\d+) limit_pages=(\d+)',s,30)
  assert int(m[1])>=1 and int(m[2])>=5,m[0];out['about']=m[0]
  # Files hashes the boot image 1 MiB per request: Tatami asks again while it says
  # `hashing N%`, and the hash fills in. Re-checked until it does.
  g['wait'](r'\[settings\.facts\] page=setup status=0 slot=\S+ version=\S+ filesystem=- health=- image=(hashing \d+%|[0-9a-f]{64})\n',s,30)
  m=g['wait'](r'\[settings\.facts\] page=setup status=0 slot=(\S+) version=\S+ filesystem=- health=- image=([0-9a-f]{64})\n',s,300);out['image']=m[2]
  out['about_requests']=t()[s:].count('page=setup status=0')
  # Updates and Storage: six pages on from About, after Privacy. Files answers Tatami alone.
  s=len(t());keys('pgdn','pgdn','pgdn','pgdn','pgdn','pgdn')
  m=g['wait'](r'\[settings\.facts\] page=updates status=0 slot=(none|a|b) ',s,30);out['slot']=m[1]
  g['wait'](r'\[files\.facts\] topic=1 bytes=\d+',s,5)
  s=len(t());keys('pgdn')
  m=g['wait'](r'\[settings\.facts\] page=storage status=0 slot=- version=- filesystem=sexfs (\d+) (\d+) health=none image=-\n',s,30)
  assert int(m[1])>=int(m[2])>0,m[0];out['storage']=m[0]
  # The Shell has the new shortcut: Super+Esc opens the overview, and closes it again.
  time.sleep(.6);keys('down');g['wait'](r'\[shell\.config\.changed\] key=36 value=29',0,30)
  s=len(t());chord('meta_l','esc');g['wait'](r'\[shell\.wm\.action\] action=Overview ',s,30)
  assert '[shell.wm.action] action=Lock' not in t()[s:],'Super+Esc still locks'
  chord('meta_l','esc');time.sleep(.5)
  # Abacus runs: Files names the row's program to the Shell, hidden from notifications.
  g['wait'](r'\[shell\.config\.changed\] key=40 value=8',0,30)
  g['focus'](153);time.sleep(.5);s=len(t());typ('run abacus')
  m=g['wait'](r'\[files\.config\.program\] row=(\d+) program=3\n',s,120)
  g['wait'](rf'\[shell\.notify\.program\] row={m[1]} program=3 hidden=1\n',s,30)
  assert 'KERNEL PANIC' not in t()
 except Exception:evidence(t());raise
 finally:g['stop']()
 g=boot(a.iso,disk,a.out,'tatami-2',a.resolution);t=g['text']
 try:
  g['wait'](r'\[files\.config\.loaded\] bytes=\d+ taken=\d+ unknown=0 invalid=0',0,120)
  g['wait'](r'\[shell\.startup\.run\] program=mesh\n',0,180)
  m=g['wait'](r'\[files\.config\.program\] row=(\d+) program=2\n',0,120)
  g['wait'](r'\[shell\.startup\.done\] chosen=1\n',0,60)
  g['wait'](rf'\[shell\.notify\.program\] row={m[1]} program=2 hidden=0\n',0,30)
  assert t().count('[shell.startup.run]')==1,'started more than once'
  assert 'KERNEL PANIC' not in t()
  print({'ok':True,**out})
 except Exception:evidence(t());raise
 finally:g['stop']()

if __name__=='__main__':main()
