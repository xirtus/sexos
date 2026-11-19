#!/usr/bin/env python3
"""YouTube-shaped streaming in Kaleidoscope (scripts/web_site/tube/yt.html): the formats
and request pattern YouTube's player used in the live embed runs, on local fixtures
(the cloud can't reach youtube.com). AV1 video in fragmented MP4 with a sidx and Opus
sound in WebM with Cues first go to two SourceBuffers; every piece is a POST for a
`&range=A-B`, read through `response.body.getReader()` and appended chunk by chunk, so
appends cut boxes and Clusters anywhere. At 4 s the video buffer changes codec
(changeType to VP9 WebM) and both move on with timestampOffset. All 200 pictures show,
and QEMU's recording of the sound card holds 600 Hz then 750 Hz, about 4 s each.

Build FFmpeg first (scripts/build_ffmpeg.sh), then the ISO. Fixtures: tube/yt/make.sh."""
import argparse,http.server,os,re,shutil,sys,threading,time
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
ROOT=Path(__file__).resolve().parents[1]
SITE=ROOT/'scripts/web_site/tube'
SEEN={'posts':0,'bodies':0}
class Handler(http.server.BaseHTTPRequestHandler):
 protocol_version='HTTP/1.1'
 def file(self):
  path=self.path.split('?')[0]
  if path=='/web':return ROOT/'iso_root/programs/kaleidoscope'
  if path.startswith('/tube/'):
   f=(SITE/path.removeprefix('/tube/')).resolve()
   if SITE in f.parents and f.is_file():return f
 def send(self,code,body,head=()):
  self.send_response(code)
  for k,v in head:self.send_header(k,v)
  self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
 def do_GET(self):
  f=self.file()
  if f is None:self.send(404,b'');return
  self.send(200,f.read_bytes())
 def do_POST(self):
  # Like googlevideo's videoplayback: the byte range is in the URL, the request carries a body.
  got=self.rfile.read(int(self.headers.get('Content-Length','0') or 0))
  f=self.file();m=re.search(r'[?&]range=(\d+)-(\d+)',self.path)
  if f is None or m is None:self.send(404,b'');return
  SEEN['posts']+=1;SEEN['bodies']+=bool(got)
  body=f.read_bytes();a,b=int(m[1]),min(int(m[2]),len(body)-1)
  kind='video/mp4' if f.suffix=='.mp4' else 'video/webm'
  self.send(200,body[a:b+1],[('Content-Type',kind),('X-Total-Length',str(len(body)))])
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
server=Quiet(('127.0.0.1',0),Handler);PORT=server.server_address[1]
threading.Thread(target=server.serve_forever,daemon=True).start()
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
os.environ.setdefault('SEXOS_GATE_BOUND','900')
from quil_autosave_gate import boot
from sound_gate import segments
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 wav=a.out/'web-youtube.wav';wav.unlink(missing_ok=True)
 os.environ['SEXOS_GATE_DEVICES']=f'-audiodev wav,id=snd0,path={wav},out.frequency=48000,out.channels=2 -device virtio-sound-pci,audiodev=snd0'
 base=f'http://10.0.2.2:{PORT}';url=f'{base}/tube/yt.html'
 g=boot(a.iso,disk,a.out,'webyt',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
   elif k=='+':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'equal'}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 console=lambda pat,s,timeout=300:g['wait'](r'\[web.console\] level=0 tube yt '+pat,s,timeout)
 try:
  g['wait'](r'\[bell.sound.ready\]',0,180)
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'fetch {base}/web /bin/kaleidoscope');q.key('ret')
  m=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,600);assert m[1].startswith('Saved'),m[1]
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'run +net kaleidoscope {url}');q.key('ret')
  types=console(r'types ([^\n]*)\n',s,1200);assert types[1]=='true true true function function',types[0]
  first=console(r'(first|failed) ([^\n]*)\n',s,600);assert first[1]=='first',first[0]
  assert [abs(float(x)-4.0)<=0.1 for x in first[2].split(' ')]==[True,True],first[0]
  buf=console(r'(buffered|failed) ([^\n]*)\n',s,600);assert buf[1]=='buffered',buf[0]
  n,start,end,posts,cut=buf[2].split(' ')
  assert n=='1' and float(start)<=0.05 and abs(float(end)-8.0)<=0.1 and cut=='true',buf[0]
  assert SEEN['posts']==int(posts)>=8 and SEEN['bodies']==SEEN['posts'],(SEEN,buf[0])
  ended=console(r'ended (\S+) (\S+)',s,60);assert ended.groups()==('ended','8.0'),ended[0]
  g['wait'](r'\[web.mse.decoder\] kind=video/mp4[^\n]* format=mov,mp4[^\n]* video=(av1|libdav1d)',s,300)
  g['wait'](r'\[web.mse.decoder\] kind=audio/webm[^\n]* format=matroska,webm[^\n]* audio=opus',s,60)
  console(r'playing',s,120)
  g['wait'](r'\[web.mse.switch\] at=4',s,120)
  done=g['wait'](r'\[web.media.ended\] node=\d+ frames=(\d+) sound_frames=(\d+) played=(\d+)',s,300)
  frames,heard,played=map(int,done.groups())
  assert 195<=frames<=202,f'{frames} pictures across the AV1 to VP9 switch'
  assert 370000<=heard<=400000 and played>=heard-2048,done[0]
  console(r'done (\S+)',s,60)
  text=t()[s:];assert 'web.mse.starved' not in text and 'web.mse.failed' not in text and 'web.mse.refused' not in text and 'play.sound.stalled' not in text
  time.sleep(1)
  found=segments(wav,split=True)
  for hz in(600,750):
   long=sum(f[1] for f in found if abs(f[2]-hz)<hz*0.02)
   assert 3.5<=long<=4.5,f'{hz} Hz played {long:.2f} s in all, not about 4 s: {found}'
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
  print({'ok':True,'buffered':buf[2],'requests':SEEN,'frames':frames,'sound_frames':heard,'tones':[(round(s,2),round(l,2),round(h)) for s,l,h in found]})
 finally:g['stop']()
if __name__=='__main__':main()
