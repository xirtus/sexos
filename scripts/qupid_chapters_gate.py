#!/usr/bin/env python3
"""Qupid's chapters and "Skip intro" card. A 12 s clip with three chapters made by host
ffmpeg ("Prologue" 0-2 s, "Opening" 2-7 s, "Story" 7-12 s): Qupid reads the chapters,
offers "Skip intro" over the picture during "Opening" (an accent-edged card in the lower
right), and Enter lands at the end of the opening. C lists the chapters; PgUp and PgDn
move between them. B doubles the sound (heard in the recording), and E's sound menu
changes the equalizer preset.

Needs ffmpeg on the host and Qupid built (scripts/build_play.sh), or SEXOS_PLAY."""
import argparse,http.server,os,re,shutil,struct,subprocess,tempfile,threading,time
from pathlib import Path
from PIL import Image
from install_program import install
ROOT=Path(__file__).resolve().parents[1]
META=""";FFMETADATA1
[CHAPTER]
TIMEBASE=1/1000
START=0
END=2000
title=Prologue
[CHAPTER]
TIMEBASE=1/1000
START=2000
END=7000
title=Opening
[CHAPTER]
TIMEBASE=1/1000
START=7000
END=12000
title=Story
"""

def clip():
 d=Path(tempfile.mkdtemp());meta=d/'meta.txt';meta.write_text(META);out=d/'ch.mp4'
 subprocess.run(['ffmpeg','-v','error','-y','-f','lavfi','-i','testsrc2=size=320x240:rate=25:duration=12',
  '-f','lavfi','-i','sine=frequency=440:sample_rate=48000:duration=12','-i',str(meta),'-map_metadata','2','-map_chapters','2',
  '-c:v','libx264','-pix_fmt','yuv420p','-g','25','-c:a','aac','-b:a','96k','-shortest',str(out)],check=True)
 return out.read_bytes()

class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 out=a.out.resolve();out.mkdir(parents=True,exist_ok=True);disk=out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 play=os.environ.get('SEXOS_PLAY',str(ROOT/'target/play/x86_64-sex-app/release/qupid'))
 install(disk,[('qupid',Path(play).read_bytes())])
 files={'/ch.mp4':clip()}
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
 g=boot(a.iso,disk,out,'chapters',a.resolution);t=g['text'];q=g['q']
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
  fetched('ch.mp4')
  s=len(t());say('run qupid @ch.mp4',r'\[qupid.chapters\] count=3 segments=1',300)
  seg=g['wait'](r'\[qupid.segment\] kind=Skip intro from_ms=(\d+) to_ms=(\d+)',s,30)
  assert (int(seg[1]),int(seg[2]))==(2000,7000),seg[0]
  launch=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,60)
  g['focus'](int(launch[3]));time.sleep(.3)
  # Hold the picture while the card is up, so the screenshot is steady.
  card=g['wait'](r'\[qupid.card\] at_ms=(\d+) text=Skip intro  \(Enter\)',s,120)
  ss=len(t());q.key('spc');g['wait'](r'\[play.paused\] 1',ss,30);time.sleep(.8)
  from qupid_shot import accent_shot,zoom
  zoom(q);accent,notes=accent_shot(q,out,'skip-card',61)
  assert accent>60,f'no card edge on screen: {accent} accent pixels {notes}'
  q.key('spc');g['wait'](r'\[play.paused\] 0',ss,30)
  ss=len(t());q.key('ret')
  skip=g['wait'](r'\[qupid.skip\] kind=intro to_ms=(\d+)',ss,30);assert skip[1]=='7000',skip[0]
  g['wait'](r'\[qupid.card\] at_ms=(\d+) text=\n',ss,30)
  ss=len(t());q.key('c');g['wait'](r'\[qupid.chapters.list\] shown=1 count=3',ss,30)
  lst=g['wait'](r'\[qupid.card\] at_ms=\d+ text=Chapters[^\n]*Story',ss,30)
  q.key('pgup');up=g['wait'](r'\[qupid.chapter\] n=(\d+) to_ms=(\d+) title=(\S+)',ss,30)
  assert (up[1],up[3])==('1','Opening'),up[0]
  ss=len(t());q.key('pgdn');dn=g['wait'](r'\[qupid.chapter\] n=(\d+) to_ms=(\d+) title=(\S+)',ss,30)
  assert (dn[1],dn[2],dn[3])==('2','7000','Story'),dn[0]
  # B: the 200% boost, heard twice as loud; E: the sound menu changes the preset.
  ss=len(t());q.key('b');g['wait'](r'\[qupid.sound\] boost=200 preset=flat ',ss,30)
  q.key('e');g['wait'](r'\[qupid.sound.menu\] open=1',ss,30)
  q.key('down');q.key('right');pre=g['wait'](r'\[qupid.sound\] boost=200 preset=(\w+) ',ss,30);assert pre[1]!='flat',pre[0]
  menu=g['wait'](r'\[qupid.card\] at_ms=\d+ text=Sound  [^\n]*Boost  200%',ss,30)
  q.key('e');g['wait'](r'\[qupid.sound.menu\] open=0',ss,30)
  g['wait'](r'\[play.done\] frames=\d+ ',s,180)
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
  q.cmd('quit');g['proc'].wait(30)
  data=wav.read_bytes();at=data.find(b'data');pcm=data[at+8:]
  left=struct.unpack(f'<{len(pcm)//2}h',pcm[:len(pcm)//2*2])[::2]
  peaks=sorted(max(abs(x) for x in left[i:i+4800]) for i in range(0,len(left)-4800,4800));peaks=[p for p in peaks if p>800]
  lo,hi=peaks[len(peaks)//5],peaks[len(peaks)*4//5]
  assert hi>1.6*lo,f'boost not heard: quiet {lo} loud {hi}'
  print({'ok':True,'boost_peaks':(lo,hi),'preset':pre[1],'card_at_ms':int(card[1]),'card_edge_px':accent,'skip_to_ms':int(skip[1]),'pgup':up[0],'pgdn':dn[0],'list':lst[0][:120]})
 finally:g['stop']()
if __name__=='__main__':main()
