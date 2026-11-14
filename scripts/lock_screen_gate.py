#!/usr/bin/env python3
"""Lock screen and idle (workstream B6/B7).

With no password set, Super+Esc locks and Enter unlocks. Settings sets a password
(Ctrl+P, typed twice). Locked again: keys meant for Settings never reach it, a wrong
password is refused, the right one unlocks. Then Settings sets "dim after" to one
minute and the screen dims after a quiet minute; a key wakes it."""
import argparse,re,shutil,time
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
def first_start(g,q,t,s):
 """Settings opens with first-start setup on a fresh disk: keep the defaults, no
 password, never idle."""
 g['wait'](r'\[settings.setup\] step=start',s,120);time.sleep(1)
 for step in ('Layout','Zone','Password'):
  q.key('ret');g['wait'](rf'\[settings.setup\] step={step} ok=1',s,120)
 q.key('down');time.sleep(.2);q.key('down');time.sleep(.2);q.key('ret');g['wait'](r'\[settings.setup\] step=Idle ok=1',s,60)
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 install(disk,[('settings',(ROOT/'iso_root/programs/settings').read_bytes())])
 g=boot(a.iso,disk,a.out,'lock',a.resolution);t=g['text'];q=g['q']
 def send(down,keys):q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':down,'key':{'type':'qcode','data':k}}}for k in keys]});time.sleep(.06)
 def press(*keys):
  for k in keys:send(True,[k])
  for k in reversed(keys):send(False,[k])
 def typ(text):
  for k in text:q.key('spc' if k==' ' else k);time.sleep(.08)
 def shot(name):time.sleep(1.5);q.cmd('screendump',{'filename':str(a.out/f'{name}.ppm')})
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  # No password yet: Enter unlocks.
  s=len(t());press('meta_l','esc');g['wait'](r'\[shell.lock\] locked=1',s,30);g['wait'](r'\[display.screen\] mode=0 locked=1 typed=0 say=3',s,30)
  shot('00-locked-no-password')
  s=len(t());q.key('ret');g['wait'](r'\[shell.lock.check\] status=2',s,60);g['wait'](r'\[shell.lock\] locked=0',s,30)
  # Settings sets a password.
  g['focus'](153);time.sleep(.5);s=len(t());typ('run settings');q.key('ret');first_start(g,q,t,s);g['wait'](r'\[settings.open\] generation=\d+',s,120);time.sleep(1)
  s=len(t());press('ctrl','p');typ('opensesame');q.key('ret');typ('opensesame');q.key('ret')
  g['wait'](r'\[files.password\] caller=\d+ verb=3 status=0',s,120);g['wait'](r'\[settings.password\] status=0',s,60)
  # Locked: a key meant for Settings changes nothing.
  s=len(t());press('meta_l','esc');g['wait'](r'\[display.screen\] mode=0 locked=1 typed=0 say=0',s,30)
  q.key('right');time.sleep(1);assert '[settings.set]' not in t()[s:],'a key reached Settings while locked'
  typ('wrong');g['wait'](r'\[display.screen\] mode=0 locked=1 typed=6 say=0',s,30);shot('01-locked-typing')
  s=len(t());q.key('ret');g['wait'](r'\[shell.lock.check\] status=1',s,120);g['wait'](r'\[display.screen\] mode=0 locked=1 typed=0 say=1',s,30)
  shot('02-wrong-password')
  s=len(t());typ('opensesame');q.key('ret');g['wait'](r'\[shell.lock.check\] status=0',s,120);g['wait'](r'\[shell.lock\] locked=0',s,30)
  # Idle: dim after one minute.
  s=len(t());q.key('esc');typ('dim');q.key('right');g['wait'](r'\[settings.set\] key=14 value=1 status=0',s,30)
  time.sleep(1);q.key('esc');s=len(t())
  g['wait'](r'\[shell.idle\] state=dim idle_ms=\d+',s,150);g['wait'](r'\[display.screen\] mode=1 locked=0',s,30);shot('03-dimmed')
  s=len(t());q.key('shift');g['wait'](r'\[shell.idle\] state=awake',s,30);g['wait'](r'\[display.screen\] mode=0 locked=0',s,30)
  assert 'KERNEL PANIC' not in t()
  print({'ok':True})
 except Exception:evidence(t());raise
 finally:g['stop']()
def evidence(text):
 """On failure, the serial lines this gate is about, so CI's log shows them."""
 keep=re.compile(r'settings|files\.config|files\.password|shell|focus|input|hid|display\.screen|display\.clock\.zone|PANIC|panic|fault|exception|#PF|killed|retired|finish|exit',re.I)
 drop=re.compile(r'clock\.civil')
 for line in [l for l in text.splitlines() if keep.search(l) and not drop.search(l)][-150:]:print('[serial]',line[:200])
if __name__=='__main__':main()
