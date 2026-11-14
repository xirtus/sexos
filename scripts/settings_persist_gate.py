#!/usr/bin/env python3
"""Settings survive a restart (workstream B1, `sex-config`).

First start: the disk has no settings file, so Files serves defaults. Super+B and
Super+Shift+N twice change the wallpaper and night light; the Shell sends both to
Files, which writes /system/settings.toml. Second start on the same disk: Files reads
the file, the Shell asks for the changes and shows Dusk with night light 2 again,
with no key pressed. Then the Settings program (installed on the disk) lowers night
light to 1 from its Desktop page; the Shell applies it from Files, and a third start
shows it. Volume keys on the first start lower the volume to 90%, which the Shell keeps
in settings and gives Bell again on the third start. The key repeat delay set in
Settings reaches the Shell at once and again after the restart."""
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
 g=boot(a.iso,disk,a.out,'settings-1',a.resolution);t=g['text'];q=g['q']
 def send(down,keys):q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':down,'key':{'type':'qcode','data':k}}}for k in keys]});time.sleep(.06)
 def press(*keys):
  for k in keys:send(True,[k])
  for k in reversed(keys):send(False,[k])
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  g['wait'](r'\[files.config.defaults\]',0,60)
  # The Shell's key repeat follows the settings' defaults (400 ms, 25 per second).
  g['wait'](r'\[shell.keyboard.settings\] delay_ms=400 interval_ms=40 caps_as_ctrl=0',0,60)
  s=len(t());press('meta_l','b');g['wait'](r'\[shell.look\] wallpaper=1 name=Dusk night=0 ok=1',s,30)
  g['wait'](r'\[files.config.set\] caller=3 key=2 value=1 status=0',s,30)
  for level in (1,2):
   s=len(t());press('meta_l','shift','n');g['wait'](rf'\[files.config.set\] caller=3 key=3 value={level} status=0',s,30)
  g['wait'](r'\[shell.config.saved\] key=3 value=2 status=0',s,30)
  # Two presses of the volume down key: the Shell keeps 90% in settings.
  s=len(t())
  for _ in range(2):q.key('volumedown');time.sleep(.4)
  g['wait'](r'\[files.config.set\] caller=3 key=10 value=90 status=0',s,30)
  assert 'KERNEL PANIC' not in t()
 except Exception:evidence(t());raise
 finally:g['stop']()
 g=boot(a.iso,disk,a.out,'settings-2',a.resolution);t=g['text'];q=g['q']
 try:
  loaded=g['wait'](r'\[files.config.loaded\] bytes=(\d+) taken=(\d+) unknown=0 invalid=0',0,120)
  g['wait'](r'\[shell.config.changed\] key=2 value=1',0,60)
  g['wait'](r'\[shell.config.changed\] key=3 value=2',0,60)
  g['wait'](r'\[shell.look\] wallpaper=1 name=Dusk night=2 ok=1',0,60)
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  g['focus'](153);time.sleep(.5);s=len(t())
  for k in 'run settings':q.key('spc' if k==' ' else k);time.sleep(.05)
  q.key('ret');first_start(g,q,t,s);g['wait'](r'\[settings.open\] generation=\d+',s,120);time.sleep(1)
  # Appearance -> Desktop page, down to night light, one step lower.
  s=len(t());q.key('pgdn');time.sleep(.3);q.key('down');time.sleep(.3);q.key('left')
  g['wait'](r'\[settings.set\] key=3 value=1 status=0',s,30)
  g['wait'](r'\[files.config.set\] caller=\d+ key=3 value=1 status=0',s,30)
  # Any key wakes the Shell, which then sees the new generation.
  time.sleep(.6);q.key('down');g['wait'](r'\[shell.look\] wallpaper=1 name=Dusk night=1 ok=1',s,30)
  # Search finds the time zone; one step on is Europe/London, and the bar follows.
  s=len(t())
  for k in 'zone':q.key(k);time.sleep(.1)
  q.key('right');g['wait'](r'\[settings.set\] key=11 value=1 status=0',s,30)
  time.sleep(.6);q.key('esc');g['wait'](r'\[display.clock.zone\] zone=1 name=Europe/London',s,30)
  # Search finds the repeat delay; one step longer, and the Shell's key repeat follows.
  s=len(t())
  for k in 'delay':q.key(k);time.sleep(.1)
  q.key('right');m=g['wait'](r'\[settings.set\] key=7 value=(\d+) status=0',s,30);delay=int(m[1]);assert delay>400,delay
  time.sleep(.6);q.key('esc');g['wait'](rf'\[shell.keyboard.settings\] delay_ms={delay} interval_ms=40 ',s,30)
  assert 'KERNEL PANIC' not in t()
 except Exception:evidence(t());raise
 finally:g['stop']()
 g=boot(a.iso,disk,a.out,'settings-3',a.resolution);t=g['text']
 try:
  g['wait'](r'\[shell.look\] wallpaper=1 name=Dusk night=1 ok=1',0,120)
  g['wait'](r'\[display.clock.zone\] zone=1 name=Europe/London',0,60)
  # The volume came back from settings: Bell set to 90% with no key pressed.
  g['wait'](rf'\[shell.keyboard.settings\] delay_ms={delay} ',0,60)
  g['wait'](r'\[shell.volume.restore\] volume=90 ok=1',0,60);g['wait'](r'\[bell.mixer\] action=1 master=90 ',0,60)
  assert 'KERNEL PANIC' not in t()
  print({'ok':True,'loaded':loaded.group(0)})
 except Exception:evidence(t());raise
 finally:g['stop']()
def evidence(text):
 """On failure, the serial lines this gate is about, so CI's log shows them."""
 keep=re.compile(r'settings|bell\.mixer|files\.config|files\.password|shell|focus|input|hid|display\.screen|display\.clock\.zone|PANIC|panic|fault|exception|#PF|killed|retired|finish|exit',re.I)
 drop=re.compile(r'clock\.civil')
 for line in [l for l in text.splitlines() if keep.search(l) and not drop.search(l)][-150:]:print('[serial]',line[:200])
if __name__=='__main__':main()
