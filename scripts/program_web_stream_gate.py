#!/usr/bin/env python3
"""Web plays a video while it downloads. The test site sends a 10 s MP4 (index at the
front) at about 300 KB/s: Web must open it and show pictures before the last byte
arrives, then play it to the end, all 250 pictures and its 440 Hz sound. A second MP4, as
slow and with its index at the end, cannot open early: it opens once all is here.

Needs the host's ffmpeg to make the clips, FFmpeg built for programs, then the ISO."""
import argparse,http.server,os,re,shutil,subprocess,sys,threading,time
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
ROOT=Path(__file__).resolve().parents[1]
SITE=ROOT/'scripts/web_site'
CLIPS={}
RATE=300*1024
class Handler(http.server.BaseHTTPRequestHandler):
 rate=RATE
 def do_GET(self):
  name=self.path.removeprefix('/site/')
  if self.path=='/web':body=(ROOT/'iso_root/programs/kaleidoscope').read_bytes()
  elif name in CLIPS:body=CLIPS[name]
  elif self.path.startswith('/site/') and (SITE/name).is_file():body=(SITE/name).read_bytes()
  else:self.send_response(404);self.end_headers();return
  # Clips come in pieces on asking (Range: bytes=N-), as video sites send them.
  r=re.fullmatch(r'bytes=(\d+)-',self.headers.get('Range','')) if name in CLIPS else None
  first=int(r[1]) if r else 0
  if r and first<len(body):
   self.send_response(206);self.send_header('Content-Range',f'bytes {first}-{len(body)-1}/{len(body)}');body=body[first:]
  else:self.send_response(200)
  if name in CLIPS:self.send_header('Accept-Ranges','bytes')
  self.send_header('Content-Length',str(len(body)));self.end_headers()
  if name not in CLIPS:self.wfile.write(body);return
  # A slow site: 1/10 s of the rate at a time.
  step=Handler.rate//10
  try:
   for i in range(0,len(body),step):self.wfile.write(body[i:i+step]);self.wfile.flush();time.sleep(.1)
  except OSError:pass # Web let go of this piece for another
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
from quil_autosave_gate import boot
from sound_gate import segments
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}

def clips(out):
 """The two clips, made by the host's ffmpeg."""
 ff=shutil.which('ffmpeg')
 if not ff:raise SystemExit('program_web_stream_gate needs ffmpeg on the host to make its clips')
 run=lambda *a:subprocess.run([ff,'-loglevel','error','-y',*a],check=True)
 run('-f','lavfi','-i','testsrc2=size=320x240:rate=25,noise=alls=25:allf=t','-f','lavfi','-i','sine=frequency=440:sample_rate=48000','-t','10',
  '-c:v','libx264','-preset','veryfast','-b:v','1000k','-pix_fmt','yuv420p','-c:a','aac','-b:a','96k','-ac','2','-movflags','+faststart',str(out/'stream.mp4'))
 run('-f','lavfi','-i','testsrc2=size=160x120:rate=25,noise=alls=25:allf=t','-f','lavfi','-i','sine=frequency=660:sample_rate=48000','-t','6',
  '-c:v','libx264','-preset','veryfast','-b:v','1000k','-pix_fmt','yuv420p','-c:a','aac','-ac','2',str(out/'tail.mp4'))
 for n in('stream.mp4','tail.mp4'):CLIPS[n]=(out/n).read_bytes()
 assert CLIPS['stream.mp4'].find(b'moov')<CLIPS['stream.mp4'].find(b'mdat'),'stream.mp4 must have its index first'
 assert CLIPS['tail.mp4'].find(b'moov')>CLIPS['tail.mp4'].find(b'mdat'),'tail.mp4 must have its index last'

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);clips(a.out)
 server=Quiet(('127.0.0.1',0),Handler);port=server.server_address[1]
 threading.Thread(target=server.serve_forever,daemon=True).start()
 disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 wav=a.out/'web-stream.wav';wav.unlink(missing_ok=True)
 os.environ['SEXOS_GATE_DEVICES']=f'-audiodev wav,id=snd0,path={wav},out.frequency=48000,out.channels=2 -device virtio-sound-pci,audiodev=snd0'
 base=f'http://10.0.2.2:{port}';url=f'{base}/site/stream.html'
 g=boot(a.iso,disk,a.out,'webstream',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
   elif k=='+':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'equal'}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 console=lambda pat,s,timeout=300:g['wait'](r'\[web.console\] level=0 '+pat,s,timeout)
 total=len(CLIPS['stream.mp4'])
 try:
  g['wait'](r'\[bell.sound.ready\]',0,180)
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'fetch {base}/web /bin/kaleidoscope');q.key('ret')
  m=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,600);assert m[1].startswith('Saved'),m[1]
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'run +net kaleidoscope {url}');q.key('ret')
  console('loadstart',s,600)
  st=g['wait'](rf'\[web.media.stream\] node=(\d+) total={total} url=\S+stream.mp4',s,600);node=st[1]
  early=g['wait'](rf'\[web.media.early\] node={node} have=(\d+) done=(\d)',s,300)
  assert early[2]=='0' and int(early[1])<total,f'opened only at the end: {early[0]}'
  console('playing',s,120)
  # Pictures are shown while bytes are still coming.
  loaded=g['wait'](rf'\[web.media.load\] node={node} status=200 bytes=(\d+) ticks=\d+ streamed=1 ok=1 shown=(\d+) ',s,300)
  assert int(loaded[1])==total,loaded[0]
  shown=int(loaded[2]);assert shown>0,f'no picture shown before the download ended: {loaded[0]}'
  before=t()[s:t().index(loaded[0],s)]
  opened=re.search(rf'\[web.media.opened\] node={node} format=\S+ video=h264 audio=aac w=320 h=240',before)
  assert opened,'the video did not open before its download ended'
  ended=g['wait'](rf'\[web.media.ended\] node={node} frames=(\d+) sound_frames=(\d+) played=(\d+)',s,300)
  frames,heard,played=map(int,ended.groups())
  assert frames==250,f'{frames} pictures, not 250'
  assert 450000<=heard<=500000,ended[0]
  console(r'ended at (\S+)',s,60)
  tail=console(r'tail meta (\S+)',s,300);assert tail[1]=='6.0',tail[0]
  assert re.search(r'\[web.media.failed\] node=\d+ err=-?\d+ why=[^\n]* last=0',t()[s:]),'the end-indexed MP4 should first fail to open early'
  assert 'play.sound.stalled' not in t()[s:] and 'last=1' not in t()[s:]
  time.sleep(1)
  found=segments(wav,split=True)
  long440=sum(f[1] for f in found if abs(f[2]-440)<9)
  assert 8.5<=long440<=11,f'440 Hz played {long440:.2f} s in all, not about 10 s: {found}'
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
  print({'ok':True,'total':total,'opened_at':int(early[1]),'frames_before_end':shown,'frames':frames,'sound_frames':heard,'440_s':round(long440,2)})
 finally:g['stop']()
if __name__=='__main__':main()
