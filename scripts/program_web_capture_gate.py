#!/usr/bin/env python3
"""Kaleidoscope gives pages the camera and microphone (getUserMedia), only once the
person says so in the browser's own card.

Started with --fake-media (a moving test pattern camera and a test tone microphone),
the browser opens capture.html, which asks for both. The gate checks the card shows
and clicks Allow; the page then gets a stream whose camera shows in a <video> (the
gate checks the pixels move), reads its permissions and device names, stops its
tracks (the browser lets the devices go), and asks again without a card because the
site is remembered. It then goes to capture-block.html on another origin, where the
gate clicks Block and the page hears NotAllowedError."""
import argparse,http.server,os,re,shutil,threading,time
from pathlib import Path
from PIL import Image
from browser_gate_viewport import live_viewport
from web_surface_geometry import canvas_box
ROOT=Path(__file__).resolve().parents[1]
SITE=ROOT/'scripts/web_site'
BAR=30
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  if self.path=='/web':body=(ROOT/'iso_root/programs/kaleidoscope').read_bytes();self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
  path=self.path.split('?')[0]
  f=SITE/path.removeprefix('/site/') if path.startswith('/site/') else None
  if f is None or not f.is_file():self.send_response(404);self.end_headers();return
  body=f.read_bytes();self.send_response(200);self.send_header('Content-Type','text/html');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
servers=[Quiet(('127.0.0.1',0),Handler) for _ in range(2)]
for sv in servers:threading.Thread(target=sv.serve_forever,daemon=True).start()
PORT,OTHER=(sv.server_address[1] for sv in servers)
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
from quil_autosave_gate import boot
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus','?':'slash','=':'equal'}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 rw,rh=map(int,a.resolution.split('x'))
 base=f'http://10.0.2.2:{PORT}';other=f'http://10.0.2.2:{OTHER}'
 url=f'{base}/site/capture.html?next={other}/site/capture-block.html'
 g=boot(a.iso,disk,a.out,'webcapture',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k in ':+?':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':{':':'semicolon','+':'equal','?':'slash'}[k]}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 def move(x,y):
  q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':round(x*32767/(rw-1))}},{'type':'abs','data':{'axis':'y','value':round(y*32767/(rh-1))}}]});time.sleep(.3)
 def click(x,y):
  move(x,y)
  for down in (True,False):q.cmd('input-send-event',{'events':[{'type':'btn','data':{'down':down,'button':'left'}}]});time.sleep(.2)
 def shot(name):
  path=a.out/f'{name}.ppm';q.cmd('screendump',{'filename':str(path)});time.sleep(.5)
  img=Image.open(path).convert('RGB');img.save(a.out/f'{name}.png');return img
 def wait(pattern,s,timeout=60):return g['wait'](pattern,s,timeout)
 try:
  wait(r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'fetch {base}/web /bin/kaleidoscope');q.key('ret')
  m=wait(r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,240);assert m[1].startswith('Saved'),m[1]
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'run +net kaleidoscope {url} --fake-media');q.key('ret')
  wait(r'\[web.capture.ready\] fake=1 ',s,300)
  wait(r'\[web.console\] level=0 capture asked function',s,300)
  wait(rf'\[web.capture.prompt\] origin=http://10\.0\.2\.2:{PORT} audio=1 video=1',s,60)
  card=wait(r'\[web.capture.card\] x=(\d+) y=(\d+) w=(\d+) h=(\d+) allow=(\d+),(\d+) block=(\d+),(\d+)',s,60)
  m=wait(r'\[web.ready\] target=(\d+) task=(\d+)',s,120);target,task=m[1],m[2]
  launch=wait(rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',s,60);surface=int(launch[1])
  time.sleep(2)
  # Nothing is shared before the person answers.
  assert '[web.capture.camera] on=1' not in t()[s:] and 'capture ok' not in t()[s:],'devices started before an answer'
  tile,pw,ph=live_viewport(g,ROOT,surface)
  ox,oy,_,_=canvas_box(ROOT,tile,pw,ph)
  # The card may have been laid out for an earlier width: look again after the resize.
  cards=re.findall(r'\[web.capture.card\] x=(\d+) y=(\d+) w=(\d+) h=(\d+) allow=(\d+),(\d+) block=(\d+),(\d+)',t()[s:])
  cx,cy,cw,ch,ax,ay,bx,by=map(int,cards[-1])
  img=shot('capture-prompt')
  # The card's own fill, inside the card and away from its text and buttons.
  px=img.getpixel((ox+cx+6,oy+cy+ch-8))
  assert max(abs(px[0]-0x26),abs(px[1]-0x1F),abs(px[2]-0x40))<=24,('card not shown',px)
  s=len(t());click(ox+ax,oy+ay)
  wait(r'\[web.capture.answer\] origin=\S+ button=0',s,30)
  ok=wait(r'\[web.console\] level=0 capture ok (.+)\|(.+) (\d+)x(\d+)',s,60)
  assert ok[1]=='SexOS test microphone' and ok[2]=='SexOS test camera' and ok[3:5]==('640','480'),ok[0]
  wait(r'\[web.capture.camera\] on=1 source=fake',s,30);wait(r'\[web.capture.mic\] on=1 source=fake',s,30)
  wait(r'\[web.console\] level=0 capture playing 640 480',s,60)
  assert wait(r'\[web.console\] level=0 capture perms (\S+ \S+)',s,30)[1]=='granted granted'
  assert wait(r'\[web.console\] level=0 capture devices (.+)',s,30)[1].strip()=='SexOS test microphone|SexOS test camera'
  rect=wait(r'\[web.console\] level=0 capture rect (-?\d+) (-?\d+) (\d+) (\d+)',s,60);vx,vy,vw,vh=map(int,rect[1:5])
  frames=int(wait(r'\[web.console\] level=0 capture frames (\d+)',s,30)[1]);assert frames>=10,('camera frames',frames)
  heard=int(wait(r'\[web.capture.mic.heard\] frames=(\d+)',s,30)[1]);assert heard>0
  # The camera's picture shows in the video, and moves.
  def video_pixels(img):return [img.getpixel((ox+vx+x,oy+BAR+vy+y)) for y in range(8,vh-8,12) for x in range(8,vw-8,12)]
  one=video_pixels(shot('capture-live-1'));time.sleep(1);two=video_pixels(shot('capture-live-2'))
  assert len(set(one))>=4,('video shows no camera picture',len(set(one)))
  assert sum(p!=q for p,q in zip(one,two))>=8,'camera picture does not move'
  assert all(max(p)>16 for p in one[:4]) or len(set(one))>8,'video is black'
  # The page stops its tracks: the devices go, and a second ask needs no card.
  s=len(t());click(ox+vx+40,oy+BAR+vy-20)
  assert wait(r'\[web.console\] level=0 capture stopped (\w+)',s,30)[1]=='false'
  wait(r'\[web.capture.camera\] on=0',s,30);wait(r'\[web.capture.mic\] on=0',s,30)
  assert wait(r'\[web.console\] level=0 capture again (\w+)',s,30)[1]=='live'
  assert '[web.capture.prompt]' not in t()[s:t().find('capture again')],'asked again for a remembered site'
  # Another site: blocked.
  wait(rf'\[web.capture.prompt\] origin=http://10\.0\.2\.2:{OTHER} audio=1 video=0',s,120)
  time.sleep(2)
  cards=re.findall(r'\[web.capture.card\] x=(\d+) y=(\d+) w=(\d+) h=(\d+) allow=(\d+),(\d+) block=(\d+),(\d+)',t()[s:])
  cx,cy,cw,ch,ax,ay,bx,by=map(int,cards[-1])
  s2=len(t());click(ox+bx,oy+by)
  m=wait(r'\[web.console\] level=0 capture block (\S+) (\S+)',s2,60);assert m[1:3]==('NotAllowedError','denied'),m[0]
  assert '[web.capture.mic] on=1' not in t()[s2:],'blocked site got the microphone'
  s=len(t());g['focus'](surface);time.sleep(.3);q.key('f11');wait(rf'\[web.closed\] target={target} task={task}',s,60)
  text=t();assert 'KERNEL PANIC' not in text and '[web.fatal]' not in text and 'panicked' not in text
  print({'ok':True,'frames':frames,'heard':heard,'video':(vx,vy,vw,vh)})
 finally:g['stop']()
if __name__=='__main__':main()
