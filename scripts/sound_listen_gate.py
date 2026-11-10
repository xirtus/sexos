#!/usr/bin/env python3
"""Programs hear the microphone only with the person's leave: `run play --listen 2`
asks; Bell tells it it waits and hears nothing until Spindle's `volume ROW mic on`;
then 48 kHz frames arrive at the card's pace (QEMU's silent input) for two seconds;
`volume ROW mic off` stops the microphone at once, and the program sees it."""
import argparse,os,re,shutil,time
from pathlib import Path
from install_program import install
ROOT=Path(__file__).resolve().parents[1]
KEYS={' ':'spc','-':'minus'}

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 out=a.out.resolve();out.mkdir(parents=True,exist_ok=True);disk=out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 install(disk,[('play',(ROOT/'iso_root/programs/play').read_bytes())])
 os.environ['SEXOS_GATE_DEVICES']='-audiodev none,id=snd0 -device virtio-sound-pci,audiodev=snd0'
 from quil_autosave_gate import boot
 g=boot(a.iso,disk,out,'listen',a.resolution);t=g['text'];q=g['q']
 def say(text,pattern,timeout=180):
  g['focus'](153);time.sleep(.5);s=len(t())
  for k in text:q.key(KEYS.get(k,k));time.sleep(.05)
  q.key('ret');return g['wait'](pattern,s,timeout)
 try:
  g['wait'](r'\[bell.mic.card\] input=(\d+)',0,120)
  g['wait'](r'\[app.private.retired\] target=41 ',0,300);time.sleep(2)
  row=say('run play --listen 2',r'\[play.listen.asked\] row=(\d+)')[1]
  g['wait'](rf'\[play.listen.state\] row={row} mic=1',0,60)
  # Without leave the microphone stays off.
  time.sleep(3)
  assert '[bell.mic] on=1' not in t() and '[bell.mic.first]' not in t(),'heard without leave'
  m=say(f'volume {row} mic on',rf'\[play.listen\] row={row} frames=(\d+) ms=(\d+) peak=(\d+) lost=(\d+) mic=2',120)
  frames,ms,lost=int(m[1]),int(m[2]),int(m[4])
  assert ms>=1900 and abs(frames-48*ms)<=0.1*48*ms+4096,f'{frames} frames in {ms} ms'
  assert lost==0,m[0]
  g['wait'](r'\[bell.mic\] on=1 ok=1',0,10)
  say(f'volume {row} mic off',rf'\[play.listen.end\] row={row} mic=1',60)
  g['wait'](r'\[bell.mic\] on=0 ok=1',0,30)
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
  print({'ok':True,'row':int(row),'frames':frames,'ms':ms})
 finally:g['stop']()
if __name__=='__main__':main()
