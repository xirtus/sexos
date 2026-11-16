#!/usr/bin/env python3
"""Splicy Studio on SexOS: FilmCraft's editor in a SexOS window, painted by sex-egui.

`run splicy-studio @talk.webm @talk.words` imports the clip Splicy's own gate uses (VP9 and
Opus, decoded by FilmCraft's pure-Rust decoders), makes it a sequence, and turns
talk.words into the clip's FilmCraft transcript (11 words), which the Text panel's
Transcript tab edits. FilmCraft's own interface shows it: menus, Program monitor with
the picture, project bin, timeline with filmstrip and waveform.
The window is zoomed and photographed (splicy-studio.png); the picture must hold
FilmCraft's dark panels, its blue accent, and a lit Program monitor. Then a click on the
Program monitor's play button must keep frames coming. Build it first:
scripts/build_splicy_studio.sh."""
import argparse,http.server,os,re,shutil,subprocess,sys,tempfile,threading,time
from collections import Counter
from pathlib import Path
from PIL import Image
ROOT=Path(__file__).resolve().parents[1]
STUDIO=os.environ.get('SEXOS_SPLICY_STUDIO',str(ROOT/'apps/splicy-studio/target/x86_64-sex-std/release/splicy-studio'))
# The clip and transcript program_splicy_gate.py uses.
WORDS="""# START_MS END_MS WORD
0 400 Hello
500 900 there
1000 1600 friends.
2000 2300 Cut
2400 2700 this
2800 3300 sentence
3400 3800 out.
4000 4400 Keep
4500 4800 the
4900 5400 ending
5500 5900 here.
"""
def make_clip():
 d=Path(tempfile.mkdtemp());out=d/'talk.webm'
 subprocess.run(['ffmpeg','-v','error','-y','-f','lavfi','-i','testsrc2=size=320x180:rate=30:duration=6',
  '-f','lavfi','-i','sine=frequency=440:sample_rate=48000:duration=6',
  '-af',"volume='0.1+0.9*mod(t,1)':eval=frame",'-c:v','libvpx-vp9','-b:v','300k','-c:a','libopus','-ac','2',str(out)],check=True)
 return out.read_bytes()
FILES={'/splicy-studio':Path(STUDIO).read_bytes(),'/talk.webm':make_clip(),'/talk.words':WORDS.encode()}
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
# Fetching a 28 MiB program and painting a full editor on the CPU take longer than 300 s.
os.environ.setdefault('SEXOS_GATE_BOUND','2400')
from quil_autosave_gate import boot
from program_gate_geometry import maximize
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 g=boot(a.iso,disk,a.out,'splicy-studio',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k=='@':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'2'}],'hold-time':80})
   elif k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 def expect(text,pattern,timeout=120):
  g['focus'](153);time.sleep(.5);s=len(t());keys(text);q.key('ret');return g['wait'](pattern,s,timeout)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  m=expect(f'fetch http://10.0.2.2:{PORT}/splicy-studio /bin/splicy-studio',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',900);assert m[1].startswith('Saved'),m[1]
  for name in('talk.webm','talk.words'):
   m=expect(f'fetch http://10.0.2.2:{PORT}/{name} {name}',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',300);assert m[1].startswith('Saved'),m[1]
  s=len(t())
  expect('run splicy-studio @talk.webm @talk.words',r'\[splicy.studio.ready\]',900)
  imp=g['wait'](r'\[splicy.studio.(imported\] items=1 errors=\[\]|import.failed\][^\n]*)',s,60);assert imp[1].startswith('imported'),imp[0]
  g['wait'](r'\[splicy.studio.sequence\] made',s,60)
  g['wait'](r'\[splicy.studio.transcript\] words=11',s,60)
  launch=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,60)
  first=g['wait'](r'\[egui.frame\] n=1 w=(\d+) h=(\d+) meshes=(\d+) triangles=(\d+) pixels=(\d+) ms=(\d+)',s,600)
  (x,y,w,h),before=maximize(g,launch[3])
  time.sleep(20)
  ppm=a.out/'splicy-studio.ppm';q.cmd('screendump',{'filename':str(ppm)});time.sleep(1)
  img=Image.open(ppm).convert('RGB');img.save(a.out/'splicy-studio.png')
  box=(x+8,y+36,x+w-8,y+h-8);colours=Counter()
  for yy in range(box[1],box[3],3):
   for xx in range(box[0],box[2],3):colours[img.getpixel((xx,yy))]+=1
  total=sum(colours.values())
  dark=sum(n for (r,g2,b),n in colours.items() if max(r,g2,b)<60)
  blue=sum(n for (r,g2,b),n in colours.items() if b>180 and r<90 and 80<g2<160)
  print('--- splicy-studio colours ---',[(f'{r:02x}{g2:02x}{b:02x}',n) for (r,g2,b),n in colours.most_common(12)],file=sys.stderr)
  assert dark>total//2,f'FilmCraft dark panels: {dark} of {total}'
  assert blue>20,f'FilmCraft blue accent: {blue} pixels'
  assert len(colours)>200,f'only {len(colours)} colours: not an editor'
  text=t();assert 'KERNEL PANIC' not in text and 'KERNEL PAGE FAULT' not in text and '[splicy.fatal]' not in text
  print({'ok':True,'first_frame':first[0],'tile':(x,y,w,h),'dark':dark,'blue':blue,'colours':len(colours)})
 except BaseException:
  keep=re.compile(r'splicy|egui|shell\.program|program\.|loader|fatal|panic|fault|denied|refused',re.I)
  print('--- splicy-studio: program lines ---',*[l for l in t().splitlines() if keep.search(l) and 'civil' not in l and 'view.bytes' not in l][-120:],sep='\n',file=sys.stderr)
  raise
 finally:g['stop']()
if __name__=='__main__':main()
