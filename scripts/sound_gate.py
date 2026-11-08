#!/usr/bin/env python3
"""SexOS plays sound: Spindle's `beep` reaches the Bell service, which drives a virtio
sound card through its BOOT lease. QEMU records the card's output to a WAV file, and
the notes must be there at the pitch and length asked for.

Without --no-card the gate also boots once with no card, where Bell must say the
card is absent and `beep` must leave the rest of the desktop running.
"""
import argparse,os,re,shutil,struct,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
NOTES=[(440,400),(1000,300)]
KEYS={' ':'spc'}

def segments(path,rate=48000):
 """Loud stretches of the left channel: (start s, length s, Hz by zero crossings)."""
 data=path.read_bytes();at=data.find(b'data')
 assert data[:4]==b'RIFF' and at>0,'not a WAV file'
 pcm=data[at+8:];samples=struct.unpack(f'<{len(pcm)//2}h',pcm[:len(pcm)//2*2])[::2]
 window=rate//100;loud=[max(abs(x)for x in samples[i:i+window])>2000 for i in range(0,len(samples)-window,window)]
 found=[];i=0
 while i<len(loud):
  if not loud[i]:i+=1;continue
  j=i
  while j<len(loud) and loud[j]:j+=1
  part=samples[i*window:j*window];ups=[k for k,(a,b) in enumerate(zip(part,part[1:])) if a<0<=b]
  hz=(len(ups)-1)*rate/(ups[-1]-ups[0]) if len(ups)>1 else 0
  found.append((i/100,(j-i)/100,hz));i=j
 return found

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');p.add_argument('--no-card',action='store_true');a=p.parse_args()
 out=a.out.resolve();out.mkdir(parents=True,exist_ok=True);disk=out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 wav=out/'sound.wav';wav.unlink(missing_ok=True)
 if not a.no_card:
  os.environ['SEXOS_GATE_DEVICES']=f'-audiodev wav,id=snd0,path={wav},out.frequency=48000,out.channels=2 -device virtio-sound-pci,audiodev=snd0'
 from quil_autosave_gate import boot
 g=boot(a.iso,disk,out,'sound',a.resolution);t=g['text'];q=g['q']
 def say(text,pattern,timeout=120):
  g['focus'](153);time.sleep(.5);s=len(t())
  for k in text:q.key(KEYS.get(k,k));time.sleep(.05)
  q.key('ret');return g['wait'](pattern,s,timeout)
 try:
  if a.no_card:
   g['wait'](r'\[bell.sound.absent\] reason=no_lease',0,60)
   g['wait'](r'\[app.private.retired\] target=41 ',0,300);time.sleep(2)
   say('beep',r'\[bell.sound.tone\] caller_pd=\d+ hz=880 ms=200 volume=30 queued=0 card=0')
   assert 'KERNEL PANIC' not in t();print({'ok':True,'card':False});return
  grant=g['wait'](r'\[kernel.cap.virtio.grant\] pd=(\d+) slot=20 product=0x1059 [^\n]*',0,60)
  g['wait'](r'\[bell.sound.ready\] stream=\d+ rate=48000 channels=2',0,120)
  g['wait'](r'\[app.private.retired\] target=41 ',0,300);time.sleep(2)
  for hz,ms in NOTES:
   s=len(t());say(f'beep {hz} {ms} 50',rf'\[bell.sound.tone\] caller_pd=\d+ hz={hz} ms={ms} volume=50 queued=1 card=1')
   g['wait'](r'\[bell.sound.drained\]',s,60);time.sleep(.5)
  assert 'KERNEL PANIC' not in t()
  q.cmd('quit');g['proc'].wait(30)
  found=segments(wav)
  assert len(found)==len(NOTES),f'expected {len(NOTES)} notes, heard {found}'
  for (hz,ms),(start,length,heard) in zip(NOTES,found):
   assert abs(heard-hz)<=hz*0.02,f'{hz} Hz note came out at {heard:.1f} Hz'
   assert abs(length*1000-ms)<=40,f'{ms} ms note lasted {length*1000:.0f} ms'
  print({'ok':True,'grant':grant[0],'notes':[dict(start=s,ms=round(l*1000),hz=round(h,1))for s,l,h in found]})
 finally:g['stop']()
if __name__=='__main__':main()
