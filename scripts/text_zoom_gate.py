#!/usr/bin/env python3
"""Alt+= and Alt+- size a window's text: a program's text is drawn in Geist at each
step, every fully covered pixel exactly the text colour, and Alt+0 restores it."""
import argparse,re,shutil,time
from pathlib import Path
import program_launch_gate as launch
from quil_autosave_gate import boot
from quil_workflow_gate import palette_select,palette_commands
from check_desktop_pixels import font_from_source
ROOT=Path(__file__).resolve().parents[1]
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 font=font_from_source((ROOT/'servers/sexdisplay/src/main.rs').read_text())
 g=boot(a.iso,disk,a.out,'zoom',a.resolution);t=g['text'];q=g['q'];commands=palette_commands(ROOT)
 def alt(key,scale):
  s=len(t());q.cmd('send-key',{'keys':[{'type':'qcode','data':'alt'},{'type':'qcode','data':key}],'hold-time':80})
  g['wait'](rf'\[display\.text\.zoom\] surface={sid} step=-?\d+ zoom=-?\d+ scale={scale}',s,30);time.sleep(1.5)
 def shot(name,scale):
  x,y,w,h=map(int,re.findall(rf'\[shell.tile.apply\] sid={sid} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',t())[-1])
  path=a.out/f'{name}.ppm';q.cmd('screendump',{'filename':str(path)});time.sleep(.5)
  ink,missing=launch.text_ink(path,(x,y,w,h),font,6*scale,9*scale)
  assert ink>200 and missing==0,f'{name}: scale {scale} ink {ink} missing {missing}'
  return dict(scale=scale,ink=ink)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  palette_select(g['key'],'FocusProgram',commands)
  s=len(t());q.key('ret');m=g['wait'](rf'\[hello.ready\] target=(\d+) task=(\d+) surface=(\d+) bytes={len(launch.TEXT)}',s,60)
  sid=m[3];g['wait'](rf'\[display.text.committed\] target={m[1]} ',s,60);time.sleep(2)
  g['focus'](int(sid));time.sleep(.5)
  steps=[shot('default',2)]
  alt('equal',3);steps.append(shot('larger',3))
  alt('equal',4);steps.append(shot('largest-shown',4))
  alt('minus',3);steps.append(shot('smaller',3))
  alt('0',2);steps.append(shot('reset',2))
  assert steps[1]['ink']>steps[0]['ink'] and steps[2]['ink']>steps[1]['ink'],steps
  assert steps[-1]['ink']==steps[0]['ink'],steps
  assert 'KERNEL PANIC'not in t()
  print({'ok':True,'steps':steps})
 finally:g['stop']()
if __name__=='__main__':main()
