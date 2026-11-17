#!/usr/bin/env python3
"""Qupid's "Up next" card. Three 60 s episodes (Show.S01E01..03, made by host ffmpeg) in
a playlist. In episode 1, 9 jumps to 90%: the card shows "Up next S01E02", counts down
10 s of playing and episode 2 starts by itself. In episode 2 the card shows again and
Esc dismisses it (the credits play to the end, then episode 3 starts). Episode 3 is the
last: no card.

Needs ffmpeg on the host and Qupid built (scripts/build_play.sh), or SEXOS_PLAY."""
import argparse,http.server,os,re,shutil,subprocess,tempfile,threading,time
from pathlib import Path
from PIL import Image
from install_program import install
ROOT=Path(__file__).resolve().parents[1]

def clip(n):
 d=Path(tempfile.mkdtemp());out=d/f'Show.S01E0{n}.mp4'
 subprocess.run(['ffmpeg','-v','error','-y','-f','lavfi','-i','testsrc2=size=160x120:rate=10:duration=60',
  '-f','lavfi','-i',f'sine=frequency={330+110*n}:sample_rate=48000:duration=60',
  '-c:v','libx264','-pix_fmt','yuv420p','-g','10','-c:a','aac','-b:a','64k','-shortest',str(out)],check=True)
 return out.read_bytes()

class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 out=a.out.resolve();out.mkdir(parents=True,exist_ok=True);disk=out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 play=os.environ.get('SEXOS_PLAY',str(ROOT/'target/play/x86_64-sex-app/release/qupid'))
 install(disk,[('qupid',Path(play).read_bytes())])
 names=[f'Show.S01E0{n}.mp4' for n in (1,2,3)]
 files={'/'+name:clip(n) for n,name in zip((1,2,3),names)}
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
 g=boot(a.iso,disk,out,'upnext',a.resolution);t=g['text'];q=g['q']
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
  g['wait'](r'\[bell.sound.ready\] card=\S+ stream=\d+ rate=48000 channels=2',0,120)
  g['wait'](r'\[app.private.retired\] target=41 ',0,300);time.sleep(2)
  for name in names:fetched(name)
  s=len(t());say('run qupid '+' '.join('@'+n for n in names),r'\[play.item\] n=0 of=3 ',300)
  armed=g['wait'](r'\[qupid.upnext.armed\] from_ms=(\d+) label=S01E02',s,120);assert abs(int(armed[1])-30000)<500,armed[0]
  launch=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,60)
  sid=int(launch[3])
  def jump():
   g['focus'](sid);time.sleep(.3);ss=len(t());q.key('9');g['wait'](r'\[play.seek\] to_ms=5\d{4}\n',ss,30);return ss
  # Episode 1: the card counts down and episode 2 starts by itself.
  g['wait'](r'\[play.frame\] n=\d+ ',s,120);ss=jump()
  shown=g['wait'](r'\[qupid.upnext\] shown left_ms=(\d+)',ss,60)
  card=g['wait'](r'\[qupid.card\] at_ms=\d+ text=Up next  S01E02  in \d+ s',ss,60)
  time.sleep(.5);shot=out/'up-next.ppm';q.cmd('screendump',{'filename':str(shot)});time.sleep(.5)
  img=Image.open(shot).convert('RGB');img.save(out/'up-next.png');px=img.load()
  accent=sum(1 for y in range(img.height) for x in range(img.width) if abs(px[x,y][0]-0xDB)<8 and abs(px[x,y][1]-0x4A)<8 and abs(px[x,y][2]-0x7E)<8)
  assert accent>40,f'no card edge on screen: {accent} accent pixels'
  g['wait'](r'\[qupid.upnext\] go label=S01E02',ss,120)
  s2=len(t());g['wait'](r'\[play.item\] n=1 of=3 file=\S*S01E02',ss,60)
  # Episode 2: Esc watches the credits; episode 3 follows at the end anyway.
  g['wait'](r'\[play.frame\] n=\d+ ',s2,120);ss=jump()
  g['wait'](r'\[qupid.upnext\] shown left_ms=\d+',ss,60);q.key('esc')
  g['wait'](r'\[qupid.upnext\] dismissed',ss,30)
  done=g['wait'](r'\[play.done\] frames=\d+ ',ss,180)
  assert '[qupid.upnext] go' not in t()[ss:done.end()],'the card went on after Esc'
  s3=len(t());g['wait'](r'\[play.item\] n=2 of=3 file=\S*S01E03',ss,60)
  # Episode 3, the last: nothing is up next.
  g['wait'](r'\[play.frame\] n=\d+ ',s3,120);ss=jump()
  g['wait'](r'\[play.playlist.done\] items=3',ss,180)
  assert '[qupid.upnext' not in t()[s3:],'a card in the last episode'
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
  print({'ok':True,'armed_from_ms':int(armed[1]),'first_left_ms':int(shown[1]),'card':card[0].strip(),'card_edge_px':accent})
 finally:g['stop']()
if __name__=='__main__':main()
