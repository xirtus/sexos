#!/usr/bin/env python3
"""The person picks where sound plays: an Intel HD Audio controller with two codecs
(QEMU's hda-output, a line out, and hda-micro, a speaker), each recorded to its own
WAV file. `volume outputs` lists both; the first codec plays until `volume output 1`
moves sound to the second, and `volume output 0` moves it back. Each note must be
heard on the chosen output only."""
import argparse,os,re,shutil,time
from pathlib import Path
from sound_gate import segments
KEYS={' ':'spc'}
# (output chosen before the note, Hz, ms)
NOTES=[(None,440,400),(1,1000,300),(0,660,300)]

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 out=a.out.resolve();out.mkdir(parents=True,exist_ok=True);disk=out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 wavs=[out/'line.wav',out/'speaker.wav']
 for w in wavs:w.unlink(missing_ok=True)
 os.environ['SEXOS_GATE_DEVICES']=(f'-audiodev wav,id=a0,path={wavs[0]},out.frequency=48000,out.channels=2 '
  f'-audiodev wav,id=a1,path={wavs[1]},out.frequency=48000,out.channels=2 '
  '-device intel-hda -device hda-output,audiodev=a0 -device hda-micro,audiodev=a1')
 from quil_autosave_gate import boot
 g=boot(a.iso,disk,out,'outputs',a.resolution);t=g['text'];q=g['q']
 def say(text,pattern,timeout=120):
  g['focus'](153);time.sleep(.5);s=len(t())
  for k in text:q.key(KEYS.get(k,k));time.sleep(.05)
  q.key('ret');return g['wait'](pattern,s,timeout)
 try:
  g['wait'](r'\[bell.sound.ready\] card=hda stream=\d+ rate=48000 channels=2',0,300)
  g['wait'](r'\[app.private.retired\] target=41 ',0,300);time.sleep(2)
  w=int(say('volume outputs',r'\[spindle.volume.outputs\] reply=(0x[0-9a-f]+)')[1],16)
  count,on,kinds=w>>8&0xF,w&0xFF,[w>>(16+4*k)&0xF for k in range(2)]
  assert count==2 and on==1 and kinds==[0,1],(hex(w),count,on,kinds)
  for choose,hz,ms in NOTES:
   if choose is not None:say(f'volume output {choose}',rf'\[bell.outputs\] on={1<<choose:#x}')
   s=len(t());say(f'beep {hz} {ms} 50',rf'\[bell.sound.tone\] caller_pd=\d+ hz={hz} ms={ms} volume=50 queued=1 card=1')
   g['wait'](r'\[bell.sound.drained\]',s,60);time.sleep(.5)
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
  q.cmd('quit');g['proc'].wait(30)
  heard=[[round(h) for _,_,h in segments(w)] for w in wavs]
  want=[[440,660],[1000]]
  for k in range(2):
   assert len(heard[k])==len(want[k]) and all(abs(h-w)<=w*0.02 for h,w in zip(heard[k],want[k])),f'output {k} heard {heard[k]}, wanted {want[k]}'
  print({'ok':True,'outputs':hex(w),'heard':heard})
 finally:g['stop']()
if __name__=='__main__':main()
