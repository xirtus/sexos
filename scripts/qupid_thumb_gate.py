#!/usr/bin/env python3
"""`qupid --thumb @IN @OUT SIZE`, for Files and Loom. A 20 s clip, blue for its first
second and red after: the thumbnail comes from about 10% in (red), fitted inside SIZE and
not larger than the video, written as u16 LE width, u16 LE height and RGB rows, with no
window. A song without cover art has no picture: the run fails and writes no file.

Needs ffmpeg on the host and Qupid built (scripts/build_play.sh), or SEXOS_PLAY."""
import argparse,http.server,os,re,shutil,subprocess,tempfile,threading,time
from pathlib import Path
from install_program import install
ROOT=Path(__file__).resolve().parents[1]

def media():
 d=Path(tempfile.mkdtemp())
 subprocess.run(['ffmpeg','-v','error','-y','-f','lavfi','-i','color=c=blue:size=320x180:rate=25:duration=1','-f','lavfi','-i','color=c=red:size=320x180:rate=25:duration=19',
  '-filter_complex','[0:v][1:v]concat=n=2:v=1[v]','-map','[v]','-c:v','libx264','-pix_fmt','yuv420p','-g','25',str(d/'clip.mp4')],check=True)
 subprocess.run(['ffmpeg','-v','error','-y','-f','lavfi','-i','sine=frequency=440:sample_rate=44100:duration=3','-c:a','libmp3lame','-b:a','64k',str(d/'song.mp3')],check=True)
 return (d/'clip.mp4').read_bytes(),(d/'song.mp3').read_bytes()

class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 out=a.out.resolve();out.mkdir(parents=True,exist_ok=True);disk=out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 play=os.environ.get('SEXOS_PLAY',str(ROOT/'target/play/x86_64-sex-app/release/qupid'))
 install(disk,[('qupid',Path(play).read_bytes())])
 clip,song=media();files={'/clip.mp4':clip,'/song.mp3':song}
 class Handler(http.server.BaseHTTPRequestHandler):
  def do_GET(self):
   body=files.get(self.path)
   if body is None:self.send_response(404);self.end_headers();return
   self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
  def log_message(self,*a):pass
 server=Quiet(('127.0.0.1',0),Handler);port=server.server_address[1]
 threading.Thread(target=server.serve_forever,daemon=True).start()
 os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
 from quil_autosave_gate import boot
 g=boot(a.iso,disk,out,'thumb',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k=='@':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'2'}],'hold-time':80})
   elif k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 def say(text,pattern,timeout=180):
  g['focus'](153);time.sleep(.5);s=len(t());keys(text);q.key('ret');return g['wait'](pattern,s,timeout)
 def fetched(name):
  m=say(f'fetch http://10.0.2.2:{port}/{name} {name}',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',600);assert m[1].startswith('Saved'),m[1]
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,300);time.sleep(2)
  fetched('clip.mp4');fetched('song.mp3')
  s=len(t());say('run qupid --thumb @clip.mp4 @clip.thumb 96',r'\[qupid.thumb(?:.failed)?\] [^\n]*\n',300)
  m=g['wait'](r'\[qupid.thumb\] w=(\d+) h=(\d+) at_ms=(\d+) ms=(\d+)',s,5)
  w,h,at,ms=map(int,m.groups())
  assert (w,h)==(96,54),m[0]
  assert at>=1000,f'not from 10% in: {m[0]}'
  wr=g['wait'](r'\[qupid.thumb.written\] bytes=(\d+) mid=([0-9a-f]{6})',s,5)
  assert int(wr[1])==4+96*54*3,wr[0]
  r,gg,b=(int(wr[2][k:k+2],16) for k in (0,2,4));assert r>180 and gg<70 and b<70,f'not the red part: {wr[0]}'
  # Bigger than the video: the video's own size.
  s=len(t());say('run qupid --thumb @clip.mp4 @big.thumb 1000',r'\[qupid.thumb(?:.failed)?\] [^\n]*\n',300)
  big=g['wait'](r'\[qupid.thumb\] w=(\d+) h=(\d+)',s,5);assert big.groups()==('320','180'),big[0]
  # A song with no cover art: no picture, no file.
  s=len(t());say('run qupid --thumb @song.mp3 @song.thumb 96',r'\[qupid.thumb(?:.failed)?\] [^\n]*\n',300)
  fail=g['wait'](r'\[qupid.thumb.failed\] ([^\n]*)',s,5)
  assert 'qupid.thumb.written' not in t()[s:],'a file for a song without art'
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
  print({'ok':True,'thumb':(w,h),'from_ms':at,'ms':ms,'mid':wr[2],'big':big.groups(),'song':fail[1]})
 finally:g['stop']()
if __name__=='__main__':main()
