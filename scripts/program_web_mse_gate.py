#!/usr/bin/env python3
"""Media Source Extensions in Web, the way YouTube and other DASH players stream: the
page makes a MediaSource, adds a video SourceBuffer (fragmented MP4, H.264) and an
audio one (fragmented MP4, AAC), fetches both files in 24 KiB Range pieces and
appends them, reads `buffered` (it must follow the segment times: 0 to 4 s), calls
endOfStream and plays. The two streams play on one clock: all 100 pictures show, and
QEMU's recording of the sound card holds 440 Hz then 880 Hz, about 2 s each.
Then a 12 s stream plays while its segments still arrive, a piece every half second:
it starts before endOfStream and plays 300 pictures and 12 s of 660 Hz without a gap.

Build FFmpeg first (scripts/build_ffmpeg.sh), then the ISO."""
import argparse,http.server,os,re,shutil,sys,threading,time
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
ROOT=Path(__file__).resolve().parents[1]
SITE=ROOT/'scripts/web_site/tube'
SEEN={'ranges':0,'whole':0}
class Handler(http.server.BaseHTTPRequestHandler):
 protocol_version='HTTP/1.1'
 def do_GET(self):
  if self.path=='/web':f=ROOT/'iso_root/programs/web'
  elif self.path.startswith('/tube/'):f=SITE/self.path.removeprefix('/tube/')
  else:f=None
  if f is None or not f.is_file():self.send_response(404);self.send_header('Content-Length','0');self.end_headers();return
  body=f.read_bytes();m=re.fullmatch(r'bytes=(\d+)-(\d+)',self.headers.get('Range',''))
  if m and f.suffix=='.mp4':
   a,b=int(m[1]),min(int(m[2]),len(body)-1);SEEN['ranges']+=1
   self.send_response(206);self.send_header('Content-Range',f'bytes {a}-{b}/{len(body)}');body=body[a:b+1]
  else:
   if f.suffix=='.mp4':SEEN['whole']+=1
   self.send_response(200)
  self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
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
 wav=a.out/'web-mse.wav';wav.unlink(missing_ok=True)
 os.environ['SEXOS_GATE_DEVICES']=f'-audiodev wav,id=snd0,path={wav},out.frequency=48000,out.channels=2 -device virtio-sound-pci,audiodev=snd0'
 base=f'http://10.0.2.2:{PORT}';url=f'{base}/tube/mse.html'
 g=boot(a.iso,disk,a.out,'webmse',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
   elif k=='+':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'equal'}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 console=lambda pat,s,timeout=300:g['wait'](r'\[web.console\] level=0 tube mse '+pat,s,timeout)
 try:
  g['wait'](r'\[bell.sound.ready\]',0,180)
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'fetch {base}/web /bin/web');q.key('ret')
  m=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,600);assert m[1].startswith('Saved'),m[1]
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'run +net web {url}');q.key('ret')
  types=console(r'types ([^\n]*)\n',s,1200);assert types[1]=='true true false',types[0]
  console(r'open open',s,300)
  buf=console(r'(buffered|failed) ([^\n]*)\n',s,600);assert buf[1]=='buffered',buf[0]
  pieces,n,start,end,vend=buf[2].split(' ')
  assert n=='1' and float(start)<=0.05 and abs(float(end)-4.0)<=0.1 and abs(float(vend)-4.0)<=0.1,buf[0]
  assert SEEN['ranges']>=4 and SEEN['whole']==0,SEEN
  ended=console(r'ended (\S+) (\S+)',s,60);assert ended.groups()==('ended','4.0'),ended[0]
  opened=g['wait'](r'\[web.media.opened\] node=\d+ format=\S+ video=h264 audio=aac w=320 h=240 ms=\d+ sound=1',s,300)
  console(r'playing',s,120)
  done=g['wait'](r'\[web.media.ended\] node=\d+ frames=(\d+) sound_frames=(\d+) played=(\d+)',s,300)
  frames,heard,played=map(int,done.groups())
  assert frames==100,f'{frames} pictures'
  assert 180000<=heard<=200000 and played>=heard-2048,done[0]
  console(r'done (\S+)',s,60)
  early=console(r'live playing (\d+) (\w+)',s,300);assert early[2]=='false',f'started only after endOfStream: {early[0]}'
  console(r'live ended (\d+) 12.0',s,300)
  first=re.search(r'\[web.media.opened\] node=(\d+)',opened[0])[1]
  live=g['wait'](rf'\[web.media.ended\] node=(?!{first} )\d+ frames=(\d+) sound_frames=(\d+) played=(\d+)',s,300)
  assert int(live[1])==300,f'{live[1]} pictures in the live stream'
  console(r'live done',s,60)
  text=t()[s:];assert 'web.mse.starved' not in text and 'web.mse.failed' not in text and 'web.mse.refused' not in text and 'play.sound.stalled' not in text
  time.sleep(1)
  found=segments(wav,split=True)
  for hz in(440,880):
   assert [f for f in found if abs(f[2]-hz)<hz*0.02 and f[1]>=0.4],f'no {hz} Hz tone in {found}'
  for hz in(440,880):
   long=sum(f[1] for f in found if abs(f[2]-hz)<hz*0.02)
   assert 1.6<=long<=2.4,f'{hz} Hz played {long:.2f} s in all, not about 2 s'
  tone=[f for f in found if abs(f[2]-660)<660*0.02]
  assert len(tone)==1 and 11.5<=tone[0][1]<=12.5,f'660 Hz should play once, unbroken, for 12 s: {found}'
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
  print({'ok':True,'buffered':buf[2],'requests':SEEN,'frames':frames,'live':live[0],'early':early[0],'sound_frames':heard,'tones':[(round(s,2),round(l,2),round(h)) for s,l,h in found]})
 finally:g['stop']()
if __name__=='__main__':main()
