#!/usr/bin/env python3
"""Play's playlist, resume position and controls. `run play @one.mp4 @two.mp4` plays
both in turn, showing "1 of 2" in its controls. A long video paused half way shows
the paused controls; closed there, it opens again where it was left (within 1 s).

Build Play first: scripts/build_play.sh."""
import argparse,http.server,os,re,shutil,threading,time
from pathlib import Path
from install_program import install
ROOT=Path(__file__).resolve().parents[1]
T=ROOT/'apps/play/tests'
FILES={'/one.mp4':(T/'clip.mp4').read_bytes(),'/two.mp4':(T/'clip-av.mp4').read_bytes(),'/long.mp4':(T/'clip-long.mp4').read_bytes()}
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  body=FILES.get(self.path)
  if body is None:self.send_response(404);self.end_headers();return
  self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 out=a.out.resolve();out.mkdir(parents=True,exist_ok=True);disk=out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 play=os.environ.get('SEXOS_PLAY',str(ROOT/'target/play/x86_64-sex-app/release/play'))
 install(disk,[('play',Path(play).read_bytes())])
 server=Quiet(('127.0.0.1',0),Handler);port=server.server_address[1]
 threading.Thread(target=server.serve_forever,daemon=True).start()
 wav=out/'sound.wav';wav.unlink(missing_ok=True)
 os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
 os.environ['SEXOS_GATE_DEVICES']=f'-audiodev wav,id=snd0,path={wav},out.frequency=48000,out.channels=2 -device virtio-sound-pci,audiodev=snd0'
 from quil_autosave_gate import boot
 g=boot(a.iso,disk,out,'playlist',a.resolution);t=g['text'];q=g['q']
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
 def launch(cmd,pattern):
  s=len(t());say(cmd,pattern,300);return s,g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,60)
 def close(m):
  s=len(t());g['focus'](int(m[3]));time.sleep(.3);q.key('f11');g['wait'](rf'\[app.private.retired\] target={m[1]} task={m[2]} ',s,90)
 try:
  g['wait'](r'\[bell.sound.ready\] card=\S+ stream=\d+ rate=48000 channels=2',0,120)
  g['wait'](r'\[app.private.retired\] target=41 ',0,300);time.sleep(2)
  for n in ('one.mp4','two.mp4','long.mp4'):fetched(n)
  # A playlist of two: both play, in order, then it is done.
  s,m=launch('run play @one.mp4 @two.mp4',r'\[play.item\] n=0 of=2 file=\S*one\.mp4')
  g['wait'](r'\[play.controls\] text=Playing  0:0\d / 0:02   1 of 2',s,60)
  g['wait'](r'\[play.item\] n=1 of=2 file=\S*two\.mp4',s,120)
  g['wait'](r'\[play.controls\] text=Playing  0:0\d / 0:02   2 of 2',s,60)
  g['wait'](r'\[play.playlist.done\] items=2',s,120)
  assert len(re.findall(r'\[play.done\] frames=',t()[s:]))==2
  close(m)
  # Pause half way: the paused controls show; close there.
  s,m=launch('run play @long.mp4',r'\[play.opened\] ')
  g['wait'](r'\[play.frame\] n=1[05]0 at_ms=(\d+)',s,120)
  g['focus'](int(m[3]));time.sleep(.3)
  ss=len(t());q.key('spc');g['wait'](r'\[play.paused\] 1',ss,30)
  c=g['wait'](r'\[play.controls\] text=Paused  0:0(\d) / 0:08',ss,30)
  time.sleep(.5);q.cmd('screendump',{'filename':str(out/'paused.ppm')})
  ss=len(t());q.key('f11')
  saved=g['wait'](r'\[play.resume.saved\] ms=(\d+) kept=1 ok=1',ss,60);left=int(saved[1])
  g['wait'](rf'\[app.private.retired\] target={m[1]} task={m[2]} ',ss,90)
  assert 3000<=left<=6500,f'left at {left} ms'
  # Opened again: it goes on from there.
  s,m=launch('run play @long.mp4',rf'\[play.resume\] ms={left}')
  f=g['wait'](r'\[play.frame\] n=0 at_ms=(\d+)',s,60)
  assert abs(int(f[1])-left)<=1000,f'first picture at {f[1]} ms, left at {left} ms'
  g['wait'](r'\[play.done\] ',s,120)
  # Played to its end: next time it starts from the beginning.
  g['wait'](r'\[play.resume.saved\] ms=0 kept=0 ok=1',s,60)
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
  print({'ok':True,'left_ms':left,'resumed_first_frame_ms':int(f[1])})
 finally:g['stop']()
if __name__=='__main__':main()
