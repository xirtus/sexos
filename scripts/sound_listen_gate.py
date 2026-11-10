#!/usr/bin/env python3
"""Programs hear the microphone only with the person's leave: `run play --listen 2`
asks; Bell tells it it waits and hears nothing until Spindle's `volume ROW mic on`;
then 48 kHz frames arrive at the card's pace (QEMU's silent input) for two seconds;
when the program ends its leave lapses and what it heard is wiped, so the next asks anew;
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
  def ask():
   s0=len(t());m=say('run play --listen 2',r'\[bell.mic.asked\] row=(\d+) caller=(\d+) listen=1 allowed=0')
   row,pd=m[1],m[2]
   g['wait'](rf'\[play.listen.state\] row={row} mic=1',s0,60)
   surface=g['wait'](rf'\[display.text.committed\] target={pd} surface=(\d+)',s0,60)[1]
   # Without leave the microphone stays off.
   s1=len(t());time.sleep(3)
   assert '[bell.mic] on=1' not in t()[s1:] and '[bell.mic.first]' not in t()[s1:],'heard without leave'
   return row,pd,int(surface)
  def hear(row):
   m=say(f'volume {row} mic on',rf'\[play.listen\] row={row} frames=(\d+) ms=(\d+) peak=(\d+) lost=(\d+) mic=2',120)
   frames,ms,lost=int(m[1]),int(m[2]),int(m[4])
   assert ms>=1900 and abs(frames-48*ms)<=0.1*48*ms+4096,f'{frames} frames in {ms} ms'
   assert lost==0,m[0]
   return frames,ms
  row,pd,surface=ask();frames,ms=hear(row)
  # The program ends while allowed: the leave lapses and what it heard is wiped.
  s0=len(t());g['focus'](surface);time.sleep(.5);q.key('f11')
  g['wait'](rf'\[bell.mic.lapsed\] row={row} pd={pd}',s0,60)
  g['wait'](rf'\[bell.mic.wiped\] row={row} heard=\d+',s0,60)
  g['wait'](r'\[bell.mic\] on=0 ok=1',s0,30)
  # The next program asks anew, in whatever row it gets.
  row2,_,_=ask();hear(row2)
  s0=len(t())
  say(f'volume {row2} mic off',rf'\[play.listen.end\] row={row2} mic=1',60)
  g['wait'](r'\[bell.mic\] on=0 ok=1',s0,30)
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
  print({'ok':True,'rows':[int(row),int(row2)],'frames':frames,'ms':ms})
 finally:g['stop']()
if __name__=='__main__':main()
