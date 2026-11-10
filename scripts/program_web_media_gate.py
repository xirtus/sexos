#!/usr/bin/env python3
"""Video and sound in Web: a page served over the network has an autoplaying <video
controls> (MP4, H.264 and AAC, 440 Hz). Its script pauses it for 0.8 s through
HTMLMediaElement, plays it on, and when it ends plays an MP3 (880 Hz) through
`new Audio()`. QEMU records the virtio sound card to a WAV file: both tones must be in
it at their pitch, the video must show its 50 pictures, and the screen must show a
picture while it plays. The page's animated GIF must move, its WebP must decode, and
its AVIF (AV1 through dav1d) must decode to its colour.

Build FFmpeg first (scripts/build_ffmpeg.sh), then the ISO."""
import argparse,http.server,os,re,shutil,sys,threading,time
from pathlib import Path
from browser_gate_viewport import content_rect
from PIL import Image
sys.path.insert(0,str(Path(__file__).resolve().parent))
ROOT=Path(__file__).resolve().parents[1]
SITE=ROOT/'scripts/web_site'
CLIPS=ROOT/'apps/play/tests'
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  if self.path=='/web':f=ROOT/'iso_root/programs/web'
  elif self.path in('/site/clip-av.mp4','/site/tone.mp3'):f=CLIPS/self.path.removeprefix('/site/')
  elif self.path.startswith('/site/'):f=SITE/self.path.removeprefix('/site/')
  else:f=None
  if f is None or not f.is_file():self.send_response(404);self.end_headers();return
  body=f.read_bytes();self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
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
 wav=a.out/'web-media.wav';wav.unlink(missing_ok=True)
 os.environ['SEXOS_GATE_DEVICES']=f'-audiodev wav,id=snd0,path={wav},out.frequency=48000,out.channels=2 -device virtio-sound-pci,audiodev=snd0'
 base=f'http://10.0.2.2:{PORT}';url=f'{base}/site/media.html'
 g=boot(a.iso,disk,a.out,'webmedia',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
   elif k=='+':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'equal'}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 console=lambda pat,s,timeout=300:g['wait'](r'\[web.console\] level=0 '+pat,s,timeout)
 try:
  g['wait'](r'\[bell.sound.ready\]',0,180)
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'fetch {base}/web /bin/web');q.key('ret')
  m=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,600);assert m[1].startswith('Saved'),m[1]
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'run +net web {url}');q.key('ret')
  can=console(r'can (\S+ \S+ \S+)',s,1200);assert can[1]=='mp4=maybe webm=maybe avi=no',can[0]
  opened=g['wait'](r'\[web.media.opened\] node=\d+ format=\S+ video=h264 audio=aac w=320 h=240 ms=\d+ sound=1',s,600)
  meta=console(r'meta (\d+)x(\d+) (\S+)',s,120);assert meta.groups()==('320','240','2.0'),meta[0]
  m=g['wait'](r'\[web.ready\] target=(\d+) task=(\d+)',s,600);target,task=m[1],m[2]
  surface=int(g['wait'](rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',s,60)[1])
  paused=console(r'paused at (\S+) paused=(\w+)',s,120);assert paused[2]=='true',paused[0]
  # While it plays: the picture must be on screen, not black or blank.
  console(r'resumed',s,60);time.sleep(.5)
  shot=a.out/'playing.ppm';q.cmd('screendump',{'filename':str(shot)});time.sleep(.5)
  x,y,w,h=map(int,re.findall(rf'\[shell.tile.apply\] sid={surface} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',t())[-1])
  img=Image.open(shot).convert('RGB');bx,by,_,_=content_rect(t(),(x,y,w,h))
  region=img.crop((bx,by,bx+320,by+240));region.save(a.out/'playing.png')
  colours={region.getpixel((px,py)) for px in range(0,320,8) for py in range(0,220,8)}
  bright=sum(1 for px in range(0,320,8) for py in range(0,220,8) if sum(region.getpixel((px,py)))>150)
  assert len(colours)>20 and bright>200,f'no picture on screen: {len(colours)} colours, {bright} bright samples'
  pics=g['wait'](r'\[web.pictures\] count=(\d+) moving=(\d+)',s,60)
  assert int(pics[1])>=3 and pics[2]=='1',pics[0]
  avif=g['wait'](r'\[web.avif\] (w=(\d+) h=(\d+) bytes=\d+ centre=([0-9a-f]{6})|failed[^\n]*)',s,60)
  assert avif.group(2,3)==('96','48'),avif[0]
  rgb=[int(avif[4][i:i+2],16) for i in (0,2,4)]
  assert all(abs(c-e)<=12 for c,e in zip(rgb,(0xE0,0x70,0x20))),f'AVIF colour {avif[4]}, not e07020'
  g['wait'](r'\[web.picture.frames\] n=5',s,60)
  ended=g['wait'](r'\[web.media.ended\] node=\d+ frames=(\d+) sound_frames=(\d+) played=(\d+)',s,300)
  frames,heard,played=map(int,ended.groups())
  assert frames==50,f'{frames} pictures'
  assert 90000<=heard<=100000 and played>=heard-2048,ended[0]
  console(r'video ended at (\S+)',s,60)
  ends=console(r'audio ended (\d+)',s,300);assert ends[1]=='1',f'onended ran {ends[1]} times'
  assert 'play.sound.stalled' not in t()[s:] and 'web.media.failed' not in t()[s:]
  time.sleep(1)
  found=segments(wav,split=True)
  for hz in(440,880):
   assert [f for f in found if abs(f[2]-hz)<hz*0.02 and f[1]>=0.4],f'no {hz} Hz tone in {found}'
  long440=sum(f[1] for f in found if abs(f[2]-440)<9)
  assert 1.6<=long440<=2.4,f'440 Hz played {long440:.2f} s in all, not about 2 s'
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
  print({'ok':True,'avif':avif[0],'frames':frames,'sound_frames':heard,'colours':len(colours),'tones':[(round(s,2),round(l,2),round(h)) for s,l,h in found]})
 finally:g['stop']()
if __name__=='__main__':main()
