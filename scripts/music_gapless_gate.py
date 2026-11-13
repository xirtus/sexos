#!/usr/bin/env python3
"""Music plays an album without a gap. Two FLAC tracks, a 440 Hz tone (track 1) then
an 880 Hz tone (track 2), 2 s each, are given in the wrong order; Music sorts them by
their tags and plays them one after the other. QEMU records the sound card: from the
first sound to the last there is no silence longer than 5 ms, and 440 Hz comes first.

Needs ffmpeg on the host and Music built (scripts/build_music.sh)."""
import argparse,http.server,os,re,shutil,struct,subprocess,tempfile,threading,time
from pathlib import Path
from install_program import install
ROOT=Path(__file__).resolve().parents[1]

def track(hz,n,title):
 d=Path(tempfile.mkdtemp());out=d/'t.flac'
 subprocess.run(['ffmpeg','-v','error','-y','-f','lavfi','-i',f'sine=frequency={hz}:sample_rate=48000:duration=2',
  '-ac','2','-metadata',f'title={title}','-metadata','artist=Gate','-metadata','album=Gapless','-metadata',f'track={n}',str(out)],check=True)
 return out.read_bytes()

def crossings(x):return sum(1 for a,b in zip(x,x[1:]) if a<0<=b)

class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 out=a.out.resolve();out.mkdir(parents=True,exist_ok=True);disk=out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 music=os.environ.get('SEXOS_MUSIC',str(ROOT/'target/music/x86_64-sex-app/release/music'))
 install(disk,[('music',Path(music).read_bytes())])
 files={'/one.flac':track(440,1,'Low'),'/two.flac':track(880,2,'High')}
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
 g=boot(a.iso,disk,out,'music',a.resolution);t=g['text'];q=g['q']
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
  for n in ('one.flac','two.flac'):
   m=say(f'fetch http://10.0.2.2:{port}/{n} {n}',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',600);assert m[1].startswith('Saved'),m[1]
  s=len(t());say('run music @two.flac @one.flac',r'\[music.library\] tracks=2',300)
  g['wait'](r'\[music.library.track\] n=0 artist=Gate album=Gapless number=1 title=Low',s,30)
  g['wait'](r'\[music.play\] n=0 title=Low',s,60)
  nxt=g['wait'](r'\[music.next\] n=1 at_frame=(\d+) title=High',s,120)
  g['wait'](r'\[music.end\] ',s,120)
  g['wait'](r'\[music.hearing\] n=1 ',s,60);time.sleep(4)
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
  q.cmd('quit');g['proc'].wait(30)
 finally:g['stop']()
 data=wav.read_bytes();at=data.find(b'data');pcm=data[at+8:]
 left=struct.unpack(f'<{len(pcm)//2}h',pcm[:len(pcm)//2*2])[::2]
 loud=[i for i,v in enumerate(left) if abs(v)>50];first,last=loud[0],loud[-1]
 gap=run=0
 for v in left[first:last]:
  run=run+1 if abs(v)<=50 else 0;gap=max(gap,run)
 heard=(last-first)/48000
 a440=crossings(left[first+4800:first+4800+24000])*2;a880=crossings(left[last-4800-24000:last-4800])*2
 print({'ok':gap<=240 and 3.9<=heard<=4.1,'longest_quiet_ms':round(gap/48,2),'heard_s':round(heard,3),'first_hz':a440,'last_hz':a880,'next_at_frame':int(nxt[1])})
 assert gap<=240,f'silence of {gap/48:.1f} ms between tracks'
 assert 3.9<=heard<=4.1,f'heard {heard:.3f} s of sound for 4 s of tracks'
 assert abs(a440-440)<20 and abs(a880-880)<20,f'order wrong: {a440} Hz then {a880} Hz'
if __name__=='__main__':main()
