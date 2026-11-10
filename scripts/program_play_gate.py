#!/usr/bin/env python3
"""A video plays: Play, a program linking FFmpeg through sex-libc, decodes an H.264 MP4 it was given inside its own confinement and Display shows every frame; a file that is not a video is refused in words.

Build Play first: scripts/build_play.sh."""
import argparse,http.server,os,re,shutil,threading,time
from pathlib import Path
from PIL import Image
ROOT=Path(__file__).resolve().parents[1]
CLIP=(ROOT/'apps/play/tests/clip.mp4').read_bytes()
# The clip: 320x240, 25 fps, 50 frames. Left half moves (testsrc2); right half is
# solid 0xC82828 for one second, then 0x28C828. Values as FFmpeg 9 on the host
# decodes them at (3w/4, h/2), 0x00RRGGBB.
FIRST,SECOND=0xC62726,0x26C626
PLAY=os.environ.get('SEXOS_PLAY',str(ROOT/'iso_root/programs/play'))
FILES={'/play':Path(PLAY).read_bytes(),'/clip.mp4':CLIP,'/note.txt':b'not a video\n'}
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  body=FILES.get(self.path)
  if body is None:self.send_response(404);self.end_headers();return
  self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
server=Quiet(('127.0.0.1',0),Handler);PORT=server.server_address[1]
threading.Thread(target=server.serve_forever,daemon=True).start()
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
from quil_autosave_gate import boot
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}
def near(a,b,tol=6):return all(abs(((a>>s)&255)-((b>>s)&255))<=tol for s in(0,8,16))
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 g=boot(a.iso,disk,a.out,'play',a.resolution);t=g['text'];q=g['q']
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
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  fetched('/play','/bin/play');fetched('/clip.mp4','clip.mp4');fetched('/note.txt','note.txt')
  s=len(t());expect('run play @clip.mp4',r'\[play.opened\] format=\S+ codec=h264 w=320 h=240 fps=25/1 ms=\d+ shown=320x240',300)
  m=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,60)
  done=g['wait'](r'\[play.done\] frames=(\d+) ms=(\d+) w=320 h=240 ',s,900)
  frames,ms=int(done[1]),int(done[2]);assert frames==50,f'{frames} frames'
  probes={int(n):int(v,16) for n,v in re.findall(r'\[play.frame\] n=(\d+) at_ms=-?\d+ hash=[0-9a-f]+ probe=([0-9a-f]+)',t()[s:])}
  for n in(0,10,20):assert near(probes[n],FIRST),f'frame {n}: {probes[n]:06x}'
  for n in(30,40):assert near(probes[n],SECOND),f'frame {n}: {probes[n]:06x}'
  time.sleep(2);shot=a.out/'play.ppm';q.cmd('screendump',{'filename':str(shot)});time.sleep(.5)
  tile=re.findall(rf'\[shell.tile.apply\] sid={m[3]} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',t());x,y,w,h=map(int,tile[-1])
  img=Image.open(shot).convert('RGB');bx,by=x+8,y+36
  checked=bad=0
  for py in range(4,min(236,h-44),9):
   for px in range(164,min(316,w-16),9):
    r,gr,b=img.getpixel((bx+px,by+py));checked+=1;bad+=not near((r<<16)|(gr<<8)|b,SECOND,10)
  assert checked>50 and bad==0,f'{bad} of {checked} pixels of the last frame differ'
  close(m)
  expect('run play @note.txt',r'\[play.failed\] Cannot play that file',120)
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
  print({'ok':True,'frames':frames,'play_ms':ms,'fps':round(frames*1000/max(ms,1),1),'pixels_checked':checked})
 finally:g['stop']()
if __name__=='__main__':main()
