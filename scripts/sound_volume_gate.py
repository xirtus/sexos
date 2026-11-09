#!/usr/bin/env python3
"""The person's volume: the keyboard's volume and mute keys reach Shell, which asks
Bell to change the master volume; Spindle's `volume` command reads and sets it, and
sets one program row's volume. QEMU records the virtio sound card to a WAV file:
a note played at master 50% must come out at a quarter of the amplitude it has at
100% (the master follows a squared curve), and a note played while muted must not
be heard at all.
"""
import argparse,os,shutil,struct,time
from pathlib import Path
from sound_gate import segments
KEYS={' ':'spc','+':'shift-equal','-':'minus'}

def peaks(path,rate=48000):
 """(start s, length s, Hz, peak amplitude) of each loud stretch of the left channel."""
 data=path.read_bytes();at=data.find(b'data')
 pcm=data[at+8:];samples=struct.unpack(f'<{len(pcm)//2}h',pcm[:len(pcm)//2*2])[::2]
 out=[]
 for start,length,hz in segments(path,rate):
  a,b=int(start*rate),int((start+length)*rate)
  out.append((start,length,hz,max(abs(x) for x in samples[a:b])))
 return out

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 out=a.out.resolve();out.mkdir(parents=True,exist_ok=True);disk=out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 wav=out/'sound.wav';wav.unlink(missing_ok=True)
 os.environ['SEXOS_GATE_DEVICES']=f'-audiodev wav,id=snd0,path={wav},out.frequency=48000,out.channels=2 -device virtio-sound-pci,audiodev=snd0'
 from quil_autosave_gate import boot
 g=boot(a.iso,disk,out,'volume',a.resolution);t=g['text'];q=g['q']
 def say(text,pattern,timeout=120):
  g['focus'](153);time.sleep(.5);s=len(t())
  for k in text:
   if k=='+':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'equal'}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.05)
  q.key('ret');return g['wait'](pattern,s,timeout)
 def beep(hz):
  s=len(t());say(f'beep {hz} 500 50',rf'\[bell.sound.tone\] caller_pd=\d+ hz={hz} ms=500 volume=50 queued=1 card=1')
  g['wait'](r'\[bell.sound.drained\]',s,60);time.sleep(.5)
 def press(key,pattern):
  s=len(t());q.key(key);return g['wait'](pattern,s,30)
 try:
  g['wait'](r'\[bell.sound.ready\] stream=\d+ rate=48000 channels=2',0,120)
  g['wait'](r'\[app.private.retired\] target=41 ',0,300);time.sleep(2)
  say('volume',r'\[spindle.volume\] reply=0x264')   # 100%, card present
  beep(440)
  # Ten presses of the volume down key: 100% to 50%, a tick at each step.
  for n in range(10):
   press('volumedown',rf'\[bell.mixer\] action=3 master={95-5*n} muted=0')
   time.sleep(.15)
  g['wait'](r'\[bell.sound.drained\]',0,60);time.sleep(.5)
  beep(440)
  press('audiomute',r'\[bell.mixer\] action=2 master=50 muted=1')
  beep(660)
  say('volume',r'\[spindle.volume\] reply=0x332')   # 50%, muted, card
  say('volume unmute',r'\[spindle.volume\] reply=0x232')
  press('volumeup',r'\[bell.mixer\] action=3 master=55 muted=0')
  say('volume 100',r'\[spindle.volume\] reply=0x264')
  say('volume -20',r'\[spindle.volume\] reply=0x250')
  # One program row: its volume and mute are kept, whatever it is playing.
  row=int(say('volume 3 30',r'\[spindle.volume\] reply=0x([0-9a-f]+)')[1],16)
  assert row&0x1ff==30,hex(row)
  row=int(say('volume 3 mute',r'\[spindle.volume\] reply=0x([0-9a-f]+)')[1],16)
  assert row&0x1ff==0x100|100,hex(row)
  say('volume list',r'\[bell.mixer\] action=6 ')
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
  q.cmd('quit');g['proc'].wait(30)
  found=peaks(wav)
  notes=[f for f in found if f[1]>=0.4]
  low=[f for f in notes if abs(f[2]-440)<9]
  assert len(low)==2,f'expected two 440 Hz notes, heard {found}'
  ratio=low[1][3]/low[0][3]
  assert 0.2<=ratio<=0.3,f'at 50% the note is {ratio:.2f} of its loudness at 100%, not 0.25'
  assert not [f for f in notes if abs(f[2]-660)<14],f'a 660 Hz note was heard while muted: {found}'
  ticks=[f for f in found if abs(f[2]-988)<30]
  assert len(ticks)>=5,f'only {len(ticks)} volume ticks heard: {found}'
  print({'ok':True,'ratio':round(ratio,3),'peaks':[low[0][3],low[1][3]],'ticks':len(ticks)})
 finally:g['stop']()
if __name__=='__main__':main()
