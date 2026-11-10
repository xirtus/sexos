#!/usr/bin/env python3
"""Programs play sound at their own rate: Play's test tone (`run play --tone HZ RATE`)
writes two seconds of a sine at 44.1, 22.05 and 96 kHz, and Bell converts each to its
48 kHz mix. QEMU records the virtio sound card to a WAV file: each tone must be heard
for about two seconds at its own pitch."""
import argparse,os,shutil,time
from pathlib import Path
from sound_gate import segments
from install_program import install
ROOT=Path(__file__).resolve().parents[1]
KEYS={' ':'spc','-':'minus'}
TONES=[(440,44100),(660,22050),(880,96000)]

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 out=a.out.resolve();out.mkdir(parents=True,exist_ok=True);disk=out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 install(disk,[('play',(ROOT/'iso_root/programs/play').read_bytes())])
 wav=out/'sound.wav';wav.unlink(missing_ok=True)
 os.environ['SEXOS_GATE_DEVICES']=f'-audiodev wav,id=snd0,path={wav},out.frequency=48000,out.channels=2 -device virtio-sound-pci,audiodev=snd0'
 from quil_autosave_gate import boot
 g=boot(a.iso,disk,out,'rate',a.resolution);t=g['text'];q=g['q']
 def say(text,pattern,timeout=180):
  g['focus'](153);time.sleep(.5);s=len(t())
  for k in text:q.key(KEYS.get(k,k));time.sleep(.05)
  q.key('ret');return g['wait'](pattern,s,timeout)
 try:
  g['wait'](r'\[bell.sound.ready\] card=\S+ stream=\d+ rate=48000 channels=2',0,120)
  g['wait'](r'\[app.private.retired\] target=41 ',0,300);time.sleep(2)
  for hz,rate in TONES:
   m=say(f'run play --tone {hz} {rate}',rf'\[play.tone\] hz={hz} rate={rate} frames=(\d+) played=(\d+)')
   assert int(m[1])==2*rate and int(m[2])==2*rate,m[0]
   time.sleep(.5)
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
  q.cmd('quit');g['proc'].wait(30)
  found=segments(wav)
  heard=[]
  for hz,rate in TONES:
   near=[f for f in found if abs(f[2]-hz)<hz*0.02]
   long=sum(f[1] for f in near)
   assert 1.9<=long<=2.1,f'{hz} Hz written at {rate} Hz: heard {long:.2f} s, not 2 s: {found}'
   heard.append((hz,rate,round(long,3)))
  print({'ok':True,'tones':heard})
 finally:g['stop']()
if __name__=='__main__':main()
