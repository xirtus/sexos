#!/usr/bin/env python3
"""Native resolution and screen scale: with no fixed mode in the boot menu, SexOS
comes up at the monitor's own preferred mode (QEMU's EDID names the VGA xres/yres),
fills the whole screen, writes only changed pixels for small updates, and
Ctrl+Alt+= / Ctrl+Alt+0 step the screen's scale (text and cursor grow, then return)."""
import argparse,re,shutil,time
from pathlib import Path
from quil_autosave_gate import boot
from check_disk_persistence import ppm_rgb
TEXT=(0xE0,0xF0,0xFF)
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path)
 p.add_argument('--resolution',default='2560x1600');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 width,height=map(int,a.resolution.split('x'))
 # A 2560x1600 mode needs more than QEMU's default 16 MiB of VGA memory.
 g=boot(a.iso,disk,a.out,'native',a.resolution+',vgamem_mb=64');t=g['text'];q=g['q']
 def shot(name):
  path=a.out/f'{name}.ppm';q.cmd('screendump',{'filename':str(path)});time.sleep(.5)
  w,h,px=ppm_rgb(path.read_bytes(),min_width=1,min_height=1);assert (w,h)==(width,height),(name,w,h)
  ink=sum(1 for i in range(0,len(px),3) if tuple(px[i:i+3])==TEXT)
  return w,h,px,ink
 def full_after(start,timeout=120):
  # The next complete frame published after `start`.
  return g['wait'](r'\[display\.batch\.published\] generation=\d+ content=full [^\n]* written=(\d+) ',start,timeout)
 def chord(key,pattern):
  s=len(t())
  q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':dn,'key':{'type':'qcode','data':x}}}for dn,ks in((True,('ctrl','alt',key)),(False,(key,'alt','ctrl')))for x in ks]})
  m=g['wait'](pattern,s,30);full_after(s);time.sleep(3);return m
 try:
  m=g['wait'](r'\[display\.mode\] width=(\d+) height=(\d+) modes=(\d+) largest=(\d+)x(\d+) width_mm=(\d+) height_mm=(\d+) scale=(\d)',0,60)
  assert (int(m[1]),int(m[2]))==(width,height),f'booted at {m[1]}x{m[2]}, not the monitor\'s {width}x{height}'
  assert int(m[3])>1,'firmware listed no other modes'
  g['wait'](rf'\[display\.boot\.geometry\] width={width} height={height} pitch=(\d+) boot=1',0,60)
  g['wait'](r'\[display\.scale\] scale=1 width_mm=\d+ height_mm=\d+ reason=boot',0,60)
  g['wait'](r'\[app.private.retired\] target=41 ',0,180);time.sleep(3)
  w,h,px,ink1=shot('scale1')
  # The desktop reaches the far corner: not left black past the old 1280x800.
  corner=[tuple(px[((h-20)*w+x)*3:((h-20)*w+x)*3+3]) for x in range(w-200,w-20)]
  assert any(c!=(0,0,0) for c in corner),'bottom-right of the screen never drawn'
  assert ink1>500,f'little text on screen: {ink1}'
  # Small updates write only what changed (the clock ticks each second).
  small=[int(x) for x in re.findall(r'content=(?:none|damage|cursor) strip=\d [^\n]* written=(\d+) ',t())]
  assert small and min(small)<w*h//8,f'small updates rewrote the screen: {small[:8]}'
  chord('equal',r'\[display\.scale\] scale=2 step=1 reason=keys')
  _,_,_,ink2=shot('scale2')
  assert ink2>ink1*2,f'text did not grow with the scale: {ink1} -> {ink2}'
  chord('0',r'\[display\.scale\] scale=1 step=0 reason=keys')
  _,_,_,ink3=shot('scale1-again')
  assert abs(ink3-ink1)<=ink1//20,f'scale 1 again differs: {ink1} vs {ink3}'
  assert 'KERNEL PANIC' not in t()
  print({'ok':True,'mode':f'{width}x{height}','modes':int(m[3]),'largest':f'{m[4]}x{m[5]}','ink':[ink1,ink2,ink3],'smallest_update':min(small),'screen':w*h})
 finally:g['stop']()
if __name__=='__main__':main()
