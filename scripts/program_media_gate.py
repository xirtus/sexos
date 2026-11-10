#!/usr/bin/env python3
"""Play plays sound with its pictures: an MP4 with H.264 and AAC, a WebM with VP9 and
Opus, an MP4 with AV1 and AAC, a WebM with 10-bit AV1 and Opus (AV1 through dav1d),
and an MP3 with no pictures. QEMU records the virtio sound card to a WAV file;
each clip's tone must be there at its pitch and for about its length, and every
picture must be shown. A longer clip then checks that Space pauses and plays and that
L seeks forward 5 s.

Build Play first: scripts/build_play.sh."""
import argparse,os,re,shutil,sys,threading,time,http.server
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
ROOT=Path(__file__).resolve().parents[1]
TESTS=ROOT/'apps/play/tests'
PLAY=os.environ.get('SEXOS_PLAY',str(ROOT/'iso_root/programs/play'))
# name, format, video codec, audio codec, pictures, tone Hz
CLIPS=[('clip-av.mp4','mov,mp4,m4a,3gp,3g2,mj2','h264','aac',50,440),
       ('clip.webm','matroska,webm','vp9','opus',50,660),
       ('clip-av1.mp4','mov,mp4,m4a,3gp,3g2,mj2','libdav1d','aac',50,770),
       ('clip-av1.webm','matroska,webm','libdav1d','opus',50,990),
       ('tone.mp3','mp3','','mp3float',0,880)]
LONG='clip-long.mp4'  # 8 s, 160x120, 550 Hz
FILES={'/play':Path(PLAY).read_bytes(),**{'/'+c[0]:(TESTS/c[0]).read_bytes() for c in CLIPS},'/'+LONG:(TESTS/LONG).read_bytes()}
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  body=FILES.get(self.path)
  if body is None:self.send_error(404);return
  self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
server=Quiet(('127.0.0.1',0),Handler);PORT=server.server_address[1]
threading.Thread(target=server.serve_forever,daemon=True).start()
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
from quil_autosave_gate import boot
from sound_gate import segments
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 wav=a.out/'media.wav';wav.unlink(missing_ok=True)
 os.environ['SEXOS_GATE_DEVICES']=f'-audiodev wav,id=snd0,path={wav},out.frequency=48000,out.channels=2 -device virtio-sound-pci,audiodev=snd0'
 g=boot(a.iso,disk,a.out,'media',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k=='@':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'2'}],'hold-time':80})
   elif k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 def expect(text,pattern,timeout=120):
  g['focus'](153);time.sleep(.5);s=len(t());keys(text);q.key('ret');return g['wait'](pattern,s,timeout)
 def fetched(url,name):
  m=expect(f'fetch http://10.0.2.2:{PORT}{url} {name}',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',600);assert m[1].startswith('Saved'),m[1]
 def close(m):
  s2=len(t());g['focus'](int(m[3]));time.sleep(.3);q.key('f11');g['wait'](rf'\[app.private.retired\] target={m[1]} task={m[2]} ',s2,90)
 results=[]
 try:
  g['wait'](r'\[bell.sound.ready\]',0,180)
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  fetched('/play','/bin/play')
  for c in CLIPS:fetched('/'+c[0],c[0])
  for name,fmt,vcodec,acodec,pictures,hz in CLIPS:
   s=len(t())
   opened=expect(f'run play @{name}',r'\[play.opened\] format=(\S+) codec=(\S*) w=(\d+) h=(\d+) .* audio=(\S+) rate=(\d+) channels=(\d+)',300)
   assert fmt.startswith(opened[1]) and (opened[2],opened[5])==(vcodec,acodec),opened[0]
   m=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,60)
   done=g['wait'](r'\[play.done\] frames=(\d+) ms=(\d+) w=\d+ h=\d+ sound_frames=(\d+) played=(\d+) late_ms=(-?\d+)',s,900)
   frames,ms,heard,played,late=map(int,done.groups())
   assert frames==pictures,f'{name}: {frames} pictures'
   assert 90000<=heard<=100000,f'{name}: {heard} sound frames'
   assert played>=heard-2048,f'{name}: played {played} of {heard}'
   assert 'play.sound.stalled' not in t()[s:],name
   assert late<500,f'{name}: a picture was {late} ms late'
   results.append({'clip':name,'pictures':frames,'ms':ms,'sound_frames':heard,'late_ms':late})
   close(m)
  # Pause, play and seek.
  fetched('/'+LONG,LONG)
  s=len(t())
  expect(f'run play @{LONG}',r'\[play.opened\] format=\S+ codec=h264 w=160 h=120',300)
  m=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,60)
  g['wait'](r'\[play.frame\] n=10 ',s,120)
  g['focus'](int(m[3]));time.sleep(.3);q.key('spc')
  g['wait'](r'\[play.paused\] 1',s,30);s2=len(t());time.sleep(1.5)
  assert '[play.frame]' not in t()[s2:],'pictures went on while paused'
  q.key('spc');g['wait'](r'\[play.paused\] 0',s2,30)
  q.key('l');to=int(g['wait'](r'\[play.seek\] to_ms=(\d+)',s2,30)[1])
  assert 5000<=to<=7900,f'seek went to {to} ms'
  first=int(g['wait'](r'\[play.frame\] n=\d+ at_ms=(\d+)',len(t()),60)[1])
  assert first>=to-40,f'first picture after the seek is at {first} ms, seek was to {to} ms'
  done=g['wait'](r'\[play.done\] frames=(\d+) ms=(\d+) ',s,120)
  assert int(done[1])<200,f'{done[1]} pictures: the seek skipped none'
  results.append({'clip':LONG,'seek_to':to,'first_after':first,'pictures':int(done[1]),'ms':int(done[2])})
  close(m)
  time.sleep(1)
  found=segments(wav)
  for name,_,_,_,_,hz in CLIPS:
   match=[f for f in found if abs(f[2]-hz)<hz*0.02 and 1.6<=f[1]<=2.4]
   assert match,f'{name}: no {hz} Hz tone of about 2 s in {found}'
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
  print({'ok':True,'clips':results,'tones':[(round(s,2),round(l,2),round(h)) for s,l,h in found]})
 finally:g['stop']()
if __name__=='__main__':main()
