#!/usr/bin/env python3
"""USB sound cards. QEMU has a virtio sound card and a USB Audio Class 1 card
(usb-audio on the xHCI controller), each recorded to its own WAV. SexUSB drives the
USB card and Bell plays through it, as headphones plugged in take over: Play's 1 kHz
tone is heard on the USB card, steady, and not on the virtio card.

Needs ffmpeg on the host and Play built (scripts/build_play.sh)."""
import argparse,http.server,math,os,re,shutil,struct,subprocess,tempfile,threading,time
from pathlib import Path
from install_program import install
ROOT=Path(__file__).resolve().parents[1]

def tone(hz,seconds=30):
 d=Path(tempfile.mkdtemp());out=d/f'{hz}.mp4'
 subprocess.run(['ffmpeg','-v','error','-y','-f','lavfi','-i',f'color=c=gray:size=160x120:rate=10:duration={seconds}',
  '-f','lavfi','-i',f'sine=frequency={hz}:sample_rate=48000:duration={seconds}','-c:v','libx264','-pix_fmt','yuv420p',
  '-c:a','aac','-b:a','96k','-shortest',str(out)],check=True)
 return out.read_bytes()

def goertzel(x,hz,rate=48000):
 w=2*math.pi*hz/rate;c=2*math.cos(w);s1=s2=0.0
 for v in x:s0=v+c*s1-s2;s2,s1=s1,s0
 return math.sqrt(max(s1*s1+s2*s2-c*s1*s2,0))*2/len(x)

def levels(path,rate=48000,step=0.25):
 data=path.read_bytes();at=data.find(b'data');pcm=data[at+8:]
 left=struct.unpack(f'<{len(pcm)//2}h',pcm[:len(pcm)//2*2])[::2];n=int(rate*step)
 return [(goertzel(left[i:i+n],440),goertzel(left[i:i+n],880)) for i in range(0,len(left)-n,n)]

class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 out=a.out.resolve();out.mkdir(parents=True,exist_ok=True);disk=out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 play=os.environ.get('SEXOS_PLAY',str(ROOT/'target/play/x86_64-sex-app/release/play'))
 install(disk,[('play',Path(play).read_bytes())])
 files={'/tone.mp4':tone(1000,8)}
 class Handler(http.server.BaseHTTPRequestHandler):
  def do_GET(self):
   body=files.get(self.path)
   if body is None:self.send_response(404);self.end_headers();return
   self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
  def log_message(self,*a):pass
 server=Quiet(('127.0.0.1',0),Handler);port=server.server_address[1]
 threading.Thread(target=server.serve_forever,daemon=True).start()
 card=out/'virtio.wav';usb=out/'usb.wav'
 for w in (card,usb):w.unlink(missing_ok=True)
 os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
 os.environ['SEXOS_GATE_DEVICES']=(f'-audiodev wav,id=snd0,path={card},out.frequency=48000,out.channels=2 -device virtio-sound-pci,audiodev=snd0 '
  f'-audiodev wav,id=snd1,path={usb},out.frequency=48000,out.channels=2 -device usb-audio,bus=xhci.0,audiodev=snd1')
 from quil_autosave_gate import boot
 g=boot(a.iso,disk,out,'usbaudio',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k=='@':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'2'}],'hold-time':80})
   elif k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 def say(text,pattern,timeout=180):
  g['focus'](153);time.sleep(.5);s=len(t());keys(text);q.key('ret');return g['wait'](pattern,s,timeout)
 try:
  g['wait'](r'\[bell.sound.ready\] card=virtio',0,120)
  dev=g['wait'](r'\[sexusb.audio\] slot=\d+ uac=1 speed=full .* mps=(\d+) interval_us=1000 frames_per_packet=48 .* ok=1',0,120)
  g['wait'](r'\[app.private.retired\] target=41 ',0,300);time.sleep(2)
  m=say(f'fetch http://10.0.2.2:{port}/tone.mp4 tone.mp4',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',600);assert m[1].startswith('Saved'),m[1]
  s=len(t());say('run play @tone.mp4',r'\[play.opened\] ',300)
  g['wait'](r'\[bell.usb.card\] vendor=0x46f4 product=0x0002 uac=1 generation=1 parked=1',0,60)
  g['wait'](r'\[play.done\] ',s,120)
  sent=g['wait'](r'\[sexusb.audio.sent\] packets=(\d+) frames=(\d+)',s,60)
  time.sleep(2)
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
  q.cmd('quit');g['proc'].wait(30)
 finally:g['stop']()
 def tone_levels(path):
  if not path.exists():return []
  data=path.read_bytes();at=data.find(b'data');pcm=data[at+8:]
  left=struct.unpack(f'<{len(pcm)//2}h',pcm[:len(pcm)//2*2])[::2];n=12000
  return [goertzel(left[i:i+n],1000) for i in range(0,len(left)-n,n)]
 u=tone_levels(usb);v=tone_levels(card)
 top=max(u,default=0);steady=[x for x in u if x>0.5*top]
 spread=(max(steady)-min(steady))/top if steady else 1
 print({'ok':top>1000 and len(steady)>=24 and spread<0.1 and max(v,default=0)<0.05*top,'usb_peak':round(top),'usb_loud_quarters':len(steady),'usb_spread':round(spread,3),'virtio_peak':round(max(v,default=0)),'mps':int(dev[1])})
 assert top>1000,f'no 1 kHz tone on the USB card (peak {top:.0f})'
 assert len(steady)>=24,f'only {len(steady)} quarter seconds of tone on the USB card'
 assert spread<0.1,f'USB tone level wanders {spread:.2f} (gaps or repeats)'
 assert max(v,default=0)<0.05*top,'the tone also played on the virtio card'
if __name__=='__main__':main()
