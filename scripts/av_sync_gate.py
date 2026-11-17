#!/usr/bin/env python3
"""Sound and pictures stay together. Play plays a clip that flashes white and beeps at
the start of every second, for five minutes by default.

- Pictures follow the sound: each flash Play shows is logged with how much sound the
  card had played then (`played`). A flash at k seconds of media time must appear when
  the card has played k seconds of sound, give or take a constant. Drift (the
  least-squares trend of that gap across the clip) stays within 40 ms; jitter (its
  spread, which includes the card's 21 ms periods) within 80 ms.
- The sound itself is whole: QEMU records the card, and every beep there starts 1 s
  after the one before (within 5 ms): nothing dropped or played twice.

The card's own clock is the reference, not the guest's ticks: under QEMU's emulation
the two run at slightly different rates (about 1.5%), as two crystals do on hardware.

Needs ffmpeg on the host and Play built (scripts/build_play.sh)."""
import argparse,http.server,os,re,shutil,struct,subprocess,tempfile,threading,time
from pathlib import Path
from install_program import install
ROOT=Path(__file__).resolve().parents[1]

def clip(seconds):
 d=Path(tempfile.mkdtemp());out=d/'sync.mp4'
 subprocess.run(['ffmpeg','-v','error','-y',
  '-f','lavfi','-i',f"color=c=black:size=160x120:rate=25:duration={seconds},drawbox=x=0:y=0:w=iw:h=ih:color=white:t=fill:enable='lt(mod(t,1),0.08)'",
  '-f','lavfi','-i',f"aevalsrc='if(lt(mod(t,1),0.05),0.5*sin(2*PI*1000*t),0)':s=48000:d={seconds}",
  '-c:v','libx264','-pix_fmt','yuv420p','-g','50','-c:a','aac','-b:a','96k','-shortest',str(out)],check=True)
 return out.read_bytes()

def onsets(path,rate=48000):
 """Seconds into the recording where each beep starts (after at least 0.5 s of quiet)."""
 data=path.read_bytes();at=data.find(b'data');pcm=data[at+8:]
 left=struct.unpack(f'<{len(pcm)//2}h',pcm[:len(pcm)//2*2])[::2]
 found=[];quiet=rate
 for i,v in enumerate(left):
  if abs(v)>3000:
   if quiet>=rate//2:found.append(i/rate)
   quiet=0
  else:quiet+=1
 return found

class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800')
 p.add_argument('--seconds',type=int,default=300);p.add_argument('--max-drift-ms',type=float,default=40);a=p.parse_args()
 os.environ.setdefault('SEXOS_GATE_BOUND',str(a.seconds*2+400))
 out=a.out.resolve();out.mkdir(parents=True,exist_ok=True);disk=out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 play=os.environ.get('SEXOS_PLAY',str(ROOT/'target/play/x86_64-sex-app/release/play'))
 install(disk,[('play',Path(play).read_bytes())])
 files={'/sync.mp4':clip(a.seconds)}
 class Handler(http.server.BaseHTTPRequestHandler):
  def do_GET(self):
   body=files.get(self.path)
   if body is None:self.send_response(404);self.end_headers();return
   self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
  def log_message(self,*a):pass
 server=Quiet(('127.0.0.1',0),Handler);port=server.server_address[1]
 threading.Thread(target=server.serve_forever,daemon=True).start()
 wav=out/'sound.wav';wav.unlink(missing_ok=True)
 os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
 os.environ['SEXOS_GATE_DEVICES']=f'-audiodev wav,id=snd0,path={wav},out.frequency=48000,out.channels=2 -device virtio-sound-pci,audiodev=snd0'
 from quil_autosave_gate import boot
 g=boot(a.iso,disk,out,'avsync',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k=='@':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'2'}],'hold-time':80})
   elif k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 def say(text,pattern,timeout=180):
  g['focus'](153);time.sleep(.5);s=len(t());keys(text);q.key('ret');return g['wait'](pattern,s,timeout)
 try:
  g['wait'](r'\[bell.sound.ready\] card=\S+ stream=\d+ rate=48000 channels=2',0,120)
  g['wait'](r'\[app.private.retired\] target=41 ',0,300);time.sleep(2)
  m=say(f'fetch http://10.0.2.2:{port}/sync.mp4 sync.mp4',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',600);assert m[1].startswith('Saved'),m[1]
  s=len(t());say('run play @sync.mp4',r'\[play.opened\] ',300)
  done=g['wait'](r'\[play.done\] frames=(\d+) ms=(\d+) .* late_ms=(\d+)',s,a.seconds*2+120)
  flashes=[(int(at),int(tick),int(pl)) for at,tick,pl in re.findall(r'\[play.flash\] at_ms=(\d+) tick=(\d+) played=(\d+)',t()[s:])]
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
  q.cmd('quit');g['proc'].wait(30)
 finally:g['stop']()
 beeps=onsets(wav)
 assert len(flashes)>=a.seconds-1,f'{len(flashes)} flashes for {a.seconds} s'
 gaps=[(b-a0)*1000 for a0,b in zip(beeps,beeps[1:])]
 assert len(beeps)>=a.seconds-1,f'{len(beeps)} beeps heard for {a.seconds} s'
 gap_err=max(abs(g-1000) for g in gaps)
 # Media time of each flash against the sound the card had played when it showed.
 errs=[at/1000-played/48000 for at,_,played in flashes][2:]  # start-up first
 n=len(errs);xs=[at/1000 for at,_,_ in flashes][2:];mx=sum(xs)/n;my=sum(errs)/n
 slope=sum((x-mx)*(y-my) for x,y in zip(xs,errs))/max(sum((x-mx)**2 for x in xs),1e-9)
 drift=abs(slope*(xs[-1]-xs[0]))*1000
 jitter=(max(errs)-min(errs))*1000
 tick_rate=(flashes[-1][1]-flashes[2][1])/(flashes[-1][0]-flashes[2][0])
 print({'ok':drift<=a.max_drift_ms and jitter<=80 and gap_err<=5,'seconds':a.seconds,'flashes':len(flashes),'beeps':len(beeps),'drift_ms':round(drift,1),'jitter_ms':round(jitter,1),
  'beep_gap_err_ms':round(gap_err,2),'guest_ticks_per_media_ms':round(tick_rate,4),'late_ms':int(done[3])})
 assert drift<=a.max_drift_ms,f'pictures drift {drift:.1f} ms from the sound over {len(errs)} s'
 assert jitter<=80,f'pictures jitter {jitter:.1f} ms around the sound'
 assert gap_err<=5,f'beeps not 1 s apart: worst {gap_err:.1f} ms'
if __name__=='__main__':main()
