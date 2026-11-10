#!/usr/bin/env python3
"""Notifications on a booted desktop: Spindle's `notify` posts text through
Bell, the panel shows it for a few seconds, Super+N shows the history.
Screens are saved for review."""
import argparse,re,shutil,time
from pathlib import Path
from quil_autosave_gate import boot
KEYS={' ':'spc','|':('shift','backslash'),'-':'minus','.':'dot'}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 g=boot(a.iso,disk,a.out,'notify',a.resolution);t=g['text'];q=g['q']
 def send(down,keys):q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':down,'key':{'type':'qcode','data':k}}}for k in keys]});time.sleep(.06)
 def press(*keys):
  for k in keys:send(True,[k])
  for k in reversed(keys):send(False,[k])
 def type_text(s):
  for ch in s:
   k=KEYS.get(ch,('shift',ch.lower()) if ch.isupper() else ch);press(*(k if isinstance(k,tuple)else(k,)))
 def shot(name):time.sleep(1.2);q.cmd('screendump',{'filename':str(a.out/f'{name}.ppm')});time.sleep(.4)
 def notify(summary,body):
  s=len(t());type_text(f'notify {summary} | {body}');press('ret')
  g['wait'](r'\[bell.post.ok\] caller_pd=\d+ event_id=\d+ lane=\d+ len=\d+ delivered=1',s,30)
  return g['wait'](r'\[shell.notify.show\] id=\d+ sender=\d+ app=(\w+) urgency=\d summary=([^\n]*?) body_len=(\d+) history=(\d+)',s,30)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(3)
  # Focus Spindle through the launcher.
  s=len(t());press('meta_l');g['wait'](r'\[shell.palette.open\]',s,30);time.sleep(.5)
  type_text('term');press('ret');g['wait'](r'\[command_palette.execute\] cmd=0 ',s,30);time.sleep(2)
  m=notify('Build done','3 warnings in sexnet');assert m[1]=='Spindle' and m[2]=='Build done' and m[3]=='20' and m[4]=='1',m.groups()
  shot('01-panel-notice')
  s=len(t());g['wait'](r'\[shell.notify.hide\] history=1',s,20)
  m=notify('Backup finished','12 files');assert m[4]=='2',m.groups()
  s=len(t());press('meta_l','n');g['wait'](r'\[shell.notify.history\] count=2 ',s,30)
  shot('02-history')
  assert 'KERNEL PANIC' not in t()
  print({'ok':True})
 finally:g['stop']()
if __name__=='__main__':main()
