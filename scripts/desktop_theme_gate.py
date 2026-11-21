#!/usr/bin/env python3
"""The desktop follows the theme (slice S6a).

Programs draw with Settings' `appearance.theme` and `appearance.accent`
(`sex_theme`); the bar, window chrome, menus and desktop cards now do too. On a
fresh disk the theme is the default, light: once the Shell has read the
settings it shows the look once (`[shell.theme] mode=light ... status=ok`).
Settings (installed on the disk) finds Theme by search and steps it to Dark;
the Shell sends the look again with the new theme and Display repaints
(`[display.theme] mode=dark accent=d6336c`), and the bar on screen turns
darker. Stepping back to Light brings `[display.theme] mode=light` and a
lighter bar again."""
import argparse,json,re,shutil,time
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot
from settings_persist_gate import first_start
ROOT=Path(__file__).resolve().parents[1]
ACCENT='d6336c'
def ppm(path):
 data=path.read_bytes();parts=data.split(b'\n',3);w,h=map(int,parts[1].split());return w,h,parts[3]
def bar_light(img):
 """Mean luminance of the bar's middle rows (0..255), away from its edges."""
 w,h,px=img;total=n=0
 for y in range(6,18):
  for x in range(w//4,3*w//4,2):
   i=(y*w+x)*3;r,g,b=px[i:i+3];total+=.2126*r+.7152*g+.0722*b;n+=1
 return total/n
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 install(disk,[('tatami',(ROOT/'iso_root/programs/tatami').read_bytes())])
 g=boot(a.iso,disk,a.out,'theme',a.resolution);t=g['text'];q=g['q'];steps=[]
 def step(pattern,start,timeout=30):
  m=g['wait'](pattern,start,timeout);steps.append(m[0]);return m
 def shot(name):time.sleep(1.5);f=a.out/f'{name}.ppm';q.cmd('screendump',{'filename':str(f)});time.sleep(.4);return ppm(f)
 try:
  step(r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  # The first settings walk shows the look once, in the default theme.
  step(r'\[shell.config.current\] generation=\d+',0,60)
  step(rf'\[shell.theme\] mode=light accent={ACCENT} status=ok',0,60)
  if re.search(r'\[display.theme\] mode=(dark|high-contrast)',t()):raise RuntimeError('Display left the default theme by itself')
  g['focus'](153);time.sleep(.5);s=len(t())
  for k in 'run tatami':q.key('spc' if k==' ' else k);time.sleep(.05)
  q.key('ret');first_start(g,q,t,s);step(r'\[settings.open\] generation=\d+',s,120);time.sleep(1)
  light=shot('00-light')
  # Search finds Theme; one step on is Dark.
  s=len(t())
  for k in 'theme':q.key(k);time.sleep(.1)
  q.key('right');step(r'\[settings.set\] key=24 value=1 status=0',s,30)
  # Any key wakes the Shell, which then sees the new generation.
  time.sleep(.6);q.key('down')
  step(rf'\[shell.theme\] mode=dark accent={ACCENT} status=ok',s,30)
  step(rf'\[display.theme\] mode=dark accent={ACCENT}',s,30)
  dark=shot('01-dark')
  lo,hi=bar_light(dark),bar_light(light)
  if not lo+20<hi:raise RuntimeError(f'the bar did not darken: light {hi:.0f}, dark {lo:.0f}')
  # And back to Light.
  s=len(t());q.key('left');step(r'\[settings.set\] key=24 value=0 status=0',s,30)
  time.sleep(.6);q.key('down')
  step(rf'\[display.theme\] mode=light accent={ACCENT}',s,30)
  back=shot('02-light-again')
  if not bar_light(back)>lo+20:raise RuntimeError(f'the bar stayed dark: {bar_light(back):.0f}')
  for bad in('KERNEL PANIC','panicked'):
   if bad in t():raise RuntimeError('guest reported '+bad)
  result=dict(ok=True,bar_light=round(hi),bar_dark=round(lo),bar_light_again=round(bar_light(back)),steps=steps)
  (a.out/'result.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result))
 except Exception as e:
  (a.out/'result.json').write_text(json.dumps(dict(ok=False,error=str(e),steps=steps))+'\n');evidence(t());raise
 finally:g['stop']()
def evidence(text):
 """On failure, the serial lines this gate is about, so CI's log shows them."""
 keep=re.compile(r'theme|look|settings|files\.config|shell\.config|PANIC|panic|fault|exception|#PF|killed',re.I)
 for line in [l for l in text.splitlines() if keep.search(l)][-150:]:print('[serial]',line[:200])
if __name__=='__main__':main()
