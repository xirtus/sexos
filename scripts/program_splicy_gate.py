#!/usr/bin/env python3
"""Splicy on SexOS: the editor opens a real clip with its transcript and looks like an editor.

* The host's FFmpeg makes talk.webm: 6 s of testsrc2 at 320x180, 30 fps (VP9), with a
  tone whose loudness swells every second (Opus), and talk.words beside it.
* `run splicy @talk.webm` decodes it inside Splicy's confinement: the timeline scan finds
  pictures for the filmstrip and loudness for the waveform.
* Screenshot 1 (splicy-opened.png): the viewer shows the picture, the picture track
  carries the filmstrip under its label, the sound track a waveform.
* Right x3 puts the caret on "Cut", S selects its sentence, Delete cuts it: picture and
  sound lose 2.0 to 4.0 s and the edit is 4 s long with one join. Screenshot 2.
* Ctrl+E exports talk-edit.webm: 120 frames.

Build Splicy first: scripts/build_splicy.sh (with the export profile for Ctrl+E)."""
import argparse,http.server,os,re,shutil,subprocess,tempfile,threading,time
from pathlib import Path
from PIL import Image
ROOT=Path(__file__).resolve().parents[1]
SPLICY=os.environ.get('SEXOS_SPLICY',str(ROOT/'target/splicy/x86_64-sex-app/release/splicy'))
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
FILES={'/splicy':Path(SPLICY).read_bytes(),'/talk.webm':make_clip(),'/talk.words':WORDS.encode()}
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
# Fetching the 23 MB program, decoding the clip for the timeline and exporting take longer than the usual 300 s.
os.environ.setdefault('SEXOS_GATE_BOUND','1500')
from quil_autosave_gate import boot
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}
def near(a,b,tol=8):return all(abs(((a>>s)&255)-((b>>s)&255))<=tol for s in(0,8,16))
def mix(under,over,alpha):return sum((((over>>s)&255)*alpha+((under>>s)&255)*(255-alpha)+127)//255<<s for s in(0,8,16))
LABEL=mix(0x3949ab,0,70);WAVE=mix(0x00796b,0xffffff,110)
def count(img,box,pred,step=2):
 x0,y0,x1,y1=box;n=0
 for y in range(y0,y1,step):
  for x in range(x0,x1,step):
   r,g,b=img.getpixel((x,y));n+=pred((r<<16)|(g<<8)|b)
 return n
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');p.add_argument('--require-export',action='store_true');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 g=boot(a.iso,disk,a.out,'splicy',a.resolution);t=g['text'];q=g['q']
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
 def shot(name):
  time.sleep(1.5);ppm=a.out/f'{name}.ppm';q.cmd('screendump',{'filename':str(ppm)});time.sleep(.8)
  img=Image.open(ppm).convert('RGB');img.save(a.out/f'{name}.png');return img
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  fetched('/splicy','/bin/splicy');fetched('/talk.webm','talk.webm');fetched('/talk.words','talk.words')
  s=len(t());expect('run splicy @talk.webm @talk.words',r'\[splicy.opened\] file=@/?talk\.webm format=\S+ video=vp9 audio=opus w=320 h=180 fps=30/1 ms=\d+ words=11',300)
  m=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,60)
  sc=g['wait'](r'\[splicy.scan\] thumbs=(\d+) peaks=(\d+) ms=(\d+)',s,600)
  assert int(sc[1])>=6 and int(sc[2])>=550,sc[0]
  g['wait'](r'\[splicy.state\] words=11 playing=11 duration_ms=\d+ clips=2 joins=0',s,60)
  g['focus'](int(m[3]))
  tile=re.findall(rf'\[shell.tile.apply\] sid={m[3]} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',t());x,y,w,h=map(int,tile[-1])
  box=(x+8,y+36,x+w-8,y+h-8)
  img=shot('splicy-opened')
  labels=count(img,box,lambda c:near(c,LABEL));waves=count(img,box,lambda c:near(c,WAVE))
  top_right=(x+w*2//5+8,y+44,x+w-16,y+h//2)
  lit=count(img,top_right,lambda c:sum((c>>s)&255 for s in(0,8,16))>200)
  assert labels>50,f'picture clip label: {labels} pixels';assert waves>50,f'waveform: {waves} pixels';assert lit>200,f'viewer picture: {lit} pixels'
  for _ in range(3):q.key('right');time.sleep(.3)
  q.key('s');time.sleep(.5)
  s=len(t());q.key('delete')
  g['wait'](r'\[splicy.cut\] first=3 last=6 text=Cut this sentence out\.\n',s,120)
  st=g['wait'](r'\[splicy.state\] words=11 playing=7 duration_ms=(\d+) clips=4 joins=1',s,60)
  assert abs(int(st[1])-4000)<=60,st[0]
  shot('splicy-cut')
  exported=None
  s=len(t());q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':'e'}],'hold-time':80})
  e=g['wait'](r'\[splicy\.(exported\] file=talk-edit\.webm frames=(\d+) bytes=(\d+)|export\.failed\] [^\n]*)',s,900)
  if e[1].startswith('exported'):
   exported=(int(e[2]),int(e[3]));assert abs(exported[0]-120)<=2,e[0];assert exported[1]>2000,e[0]
  else:
   assert not a.require_export,e[0]
  shot('splicy-exported')
  text=t();assert 'KERNEL PANIC' not in text and '[splicy.fatal]' not in text and 'KERNEL PAGE FAULT' not in text
  print({'ok':True,'thumbs':int(sc[1]),'peaks':int(sc[2]),'scan_ms':int(sc[3]),'label_px':labels,'wave_px':waves,'exported':exported})
 except BaseException:
  # The usual tail is mostly clock noise: show what the shell, loader and program said.
  keep=re.compile(r'splicy|shell\.program|spindle|program\.|loader|app\.|fatal|panic|fault|denied|refused|oom|limit',re.I)
  print('--- splicy: program lines ---',*[l for l in t().splitlines() if keep.search(l) and 'civil' not in l][-120:],sep='\n',file=__import__('sys').stderr)
  raise
 finally:g['stop']()
if __name__=='__main__':main()
