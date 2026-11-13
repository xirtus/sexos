#!/usr/bin/env python3
"""Play's subtitles, sound tracks and speed. A video with two sound tracks (440 Hz and
880 Hz) and a SubRip file, both given to Play (`run play @film.mp4 @film.srt`): Play draws
each line at its time (white text appears in the bottom of the picture), the A key
switches to the second sound track (880 Hz is heard after it), and ] plays faster.

Needs ffmpeg on the host (to make the clip) and Play built (scripts/build_play.sh)."""
import argparse,http.server,os,re,shutil,subprocess,tempfile,threading,time
from pathlib import Path
from PIL import Image
from install_program import install
from sound_gate import segments
ROOT=Path(__file__).resolve().parents[1]
SRT=b"1\n00:00:00,500 --> 00:00:03,000\nHello from SexOS\n\n2\n00:00:03,500 --> 00:00:07,000\n<i>Second</i> line\nand a third\n"

def clip():
 d=Path(tempfile.mkdtemp());out=d/'two.mp4'
 subprocess.run(['ffmpeg','-v','error','-y','-f','lavfi','-i','testsrc2=size=320x240:rate=25:duration=8',
  '-f','lavfi','-i','sine=frequency=440:sample_rate=48000:duration=8','-f','lavfi','-i','sine=frequency=880:sample_rate=48000:duration=8',
  '-map','0:v','-map','1:a','-map','2:a','-c:v','libx264','-pix_fmt','yuv420p','-g','25','-c:a','aac','-b:a','96k','-shortest',str(out)],check=True)
 return out.read_bytes()

class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 out=a.out.resolve();out.mkdir(parents=True,exist_ok=True);disk=out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 play=os.environ.get('SEXOS_PLAY',str(ROOT/'target/play/x86_64-sex-app/release/play'))
 install(disk,[('play',Path(play).read_bytes())])
 files={'/film.mp4':clip(),'/film.srt':SRT}
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
 g=boot(a.iso,disk,out,'subs',a.resolution);t=g['text'];q=g['q']
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
  fetched('film.mp4');fetched('film.srt')
  s=len(t());say('run play @film.mp4 @film.srt',r'\[play.subtitles.loaded\] file=\S*film\.srt cues=2',300)
  launch=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,60)
  g['wait'](r'\[play.caption\] at_ms=\d+ text=Hello from SexOS',s,60)
  # The picture with the caption, made big (Super+F): white text with a dark outline
  # in the bottom of the picture, gone once subtitles are turned off with S.
  g['focus'](int(launch[3]));time.sleep(.4)
  q.cmd('send-key',{'keys':[{'type':'qcode','data':'meta_l'},{'type':'qcode','data':'f'}],'hold-time':80});time.sleep(.8)
  def shot(name):
   path=out/f'{name}.ppm';q.cmd('screendump',{'filename':str(path)});time.sleep(.5)
   img=Image.open(path).convert('RGB');img.save(out/f'{name}.png')
   zoom=re.findall(rf'\[silk.lifecycle.zoom.active\] frame=\d+ sid={launch[3]} zx=(-?\d+) zy=(-?\d+)',t());x0,y0=int(zoom[-1][0])+8,int(zoom[-1][1])+34
   px=img.load()
   # White glyph pixels with dark outline pixels close beside them.
   white=dark=0
   # The bottom fifth of the 320x240 picture, where the caption goes.
   for y in range(y0+190,y0+238):
    for x in range(x0,x0+312):
     r,gg,b=px[x,y]
     if min(r,gg,b)>235:
      white+=1
      if any(max(px[x+d,y])<45 for d in (2,4,6)):dark+=1
   return white,dark
  on=shot('caption')
  ss=len(t());q.key('s');g['wait'](r'\[play.subtitles\] on=0',ss,30);time.sleep(.6)
  off=shot('no-caption')
  q.key('s');g['wait'](r'\[play.subtitles\] on=1',ss,30)
  assert on[1]>50 and on[1]>3*max(off[1],1),f'caption edge pixels: shown={on} hidden={off}'
  ss=len(t());q.key('a');tr=g['wait'](r'\[play.audio.track\] track=(\d+) of=(\d+) ok=(\d)',ss,30)
  assert tr.groups()==('1','2','1'),tr[0]
  q.key('bracket_right');sp=g['wait'](r'\[play.speed\] percent=(\d+)',ss,30);assert sp[1]=='125',sp[0]
  g['wait'](r'\[play.caption\] at_ms=\d+ text=Second line / and a third',s,60)
  g['wait'](r'\[play.done\] frames=\d+ ',s,120)
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
  q.cmd('quit');g['proc'].wait(30)
  heard=[f for f in segments(wav,split=True) if f[1]>=0.3]
  hz=[round(f[2]) for f in heard]
  assert any(abs(f[2]-440)<20 for f in heard),f'first track not heard: {heard}'
  # 880 Hz played at 125% sounds at 1100 Hz.
  assert any(abs(f[2]-880)<30 or abs(f[2]-1100)<40 for f in heard),f'second track not heard: {heard}'
  print({'ok':True,'caption':on,'no_caption':off,'tones':hz})
 finally:g['stop']()
if __name__=='__main__':main()
