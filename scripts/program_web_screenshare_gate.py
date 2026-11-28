#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Kaleidoscope shares the screen with a page (getDisplayMedia), only once the person
picks it in the Shell's own picker.

The browser opens screenshare.html (with --fake-media, so a plain http page may ask);
the gate clicks the page's button, which asks. The Shell opens its picker (the window
overview with a hint pill and an "Entire screen" button, drawn by Display); nothing is
shared before the gate clicks that button. The page then gets a stream whose video
track says Screen/monitor, shows the shared screen in a <video> (the gate checks the
video looks like the screen, shrunk), and Display rings the screen in red with a
"Stop sharing" pill. The page then opens the (fake) camera too, which the gate allows
on the browser's card: both videos get pictures at once. The gate clicks the pill:
the page's track ends, the camera stays on, and Display wipes the bank. A second ask
the gate answers with Escape rejects NotAllowedError."""
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
server=Quiet(('127.0.0.1',0),Handler);threading.Thread(target=server.serve_forever,daemon=True).start()
PORT=server.server_address[1]
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
from quil_autosave_gate import boot
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus','?':'slash','=':'equal'}
def near(p,q,d):return max(abs(a-b) for a,b in zip(p,q))<=d
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 rw,rh=map(int,a.resolution.split('x'))
 base=f'http://10.0.2.2:{PORT}';url=f'{base}/site/screenshare.html'
 g=boot(a.iso,disk,a.out,'webshare',a.resolution);t=g['text'];q=g['q']
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
  wait(r'\[boot\.screen\.share\] display=\d+ slot=1015 client_slot=264 ',0,120)
  wait(r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'fetch {base}/web /bin/kaleidoscope');q.key('ret')
  m=wait(r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,240);assert m[1].startswith('Saved'),m[1]
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'run +net kaleidoscope {url} --fake-media');q.key('ret')
  wait(r'\[web.capture.ready\] fake=1 ',s,300)
  wait(r'\[web.console\] level=0 share asked function',s,300)
  m=wait(r'\[web.ready\] target=(\d+) task=(\d+)',s,120);target,task=m[1],m[2]
  launch=wait(rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',s,60);surface=int(launch[1])
  time.sleep(2)
  tile,pw,ph=live_viewport(g,ROOT,surface)
  ox,oy,_,_=canvas_box(ROOT,tile,pw,ph)
  btn=re.findall(r'\[web.console\] level=0 share button (-?\d+) (-?\d+)',t()[s:]) or [wait(r'\[web.console\] level=0 share button (-?\d+) (-?\d+)',s,60)[1:3]]
  bx,by=map(int,btn[-1])
  # The page asks from a click; the Shell's picker opens and nothing is shared yet.
  s=len(t());click(ox+bx,oy+BAR+by)
  wait(rf'\[web.share.ask\] id=\d+ origin=http://10\.0\.2\.2:{PORT} framed=0 fake=0',s,60)
  wait(rf'\[shell.share.ask\] pd={target} task={task} focused=1 used=1 open=1',s,30)
  pick=wait(r'\[shell.share.picker\] windows=(\d+) screen=(\d+),(\d+)',s,30);sx,sy=int(pick[2]),int(pick[3])
  assert int(pick[1])>=1,pick[0]
  wait(r'\[display.share.picker\] pill=',s,30);time.sleep(1)
  img=shot('share-picker')
  assert '[display.share.started]' not in t()[s:] and 'share ok' not in t()[s:],'shared before the person picked'
  # The page cannot answer the picker: only the Shell's button does.
  s=len(t());click(sx,sy)
  wait(r'\[shell.share.picked\] kind=1 surface=0',s,30)
  wait(rf'\[shell.share.answer\] pd={target} task={task} status=1 kind=1 ',s,30)
  on=wait(r'\[shell.share.on\] stop=(\d+),(\d+) ',s,30);px,py=int(on[1]),int(on[2])
  wait(rf'\[display.share.started\] kind=1 surface=0 target={target} ',s,30)
  wait(r'\[display.share.frame\] n=1 ',s,30)
  wait(r'\[web.share.frame\] n=1 w=(\d+) h=(\d+)',s,30)
  ok=wait(r'\[web.console\] level=0 share ok (\S+) (\d+)x(\d+) (\d+)',s,60)
  assert ok[1]=='Screen|monitor' and ok[4]=='0',ok[0]
  assert int(ok[2])*rh==int(ok[3])*rw or abs(int(ok[2])/int(ok[3])-rw/rh)<.02,('keeps the screen shape',ok[0])
  wait(r'\[web.console\] level=0 share playing \d+ \d+',s,60)
  rect=wait(r'\[web.console\] level=0 share rect (-?\d+) (-?\d+) (\d+) (\d+)',s,60);vx,vy,vw,vh=map(int,rect[1:5])
  frames=int(wait(r'\[web.console\] level=0 share frames (\d+)',s,30)[1]);assert frames>=1,('shared frames',frames)
  time.sleep(1);img=shot('share-live')
  # Display rings the shared screen in red and shows its stop pill.
  red=(0xE5,0x39,0x35)
  assert near(img.getpixel((1,rh//2)),red,40) and near(img.getpixel((rw-2,rh//2)),red,40),('no red ring',img.getpixel((1,rh//2)))
  # The video shows the screen, shrunk: compare with the screenshot fitted to the video.
  vid=img.crop((ox+vx,oy+BAR+vy,ox+vx+vw,oy+BAR+vy+vh))
  fw,fh=(vw,round(vw*rh/rw)) if vw*rh<=vh*rw else (round(vh*rw/rh),vh)
  dx,dy=(vw-fw)//2,(vh-fh)//2
  shown=vid.crop((dx,dy,dx+fw,dy+fh));want=img.resize((fw,fh))
  pts=[(x,y) for y in range(4,fh-4,9) for x in range(4,fw-4,9)]
  diff=sum(max(abs(a-b) for a,b in zip(shown.getpixel(p),want.getpixel(p))) for p in pts)/len(pts)
  colours=len({shown.getpixel(p) for p in pts})
  assert colours>=6,('video shows no screen',colours)
  assert diff<70,('video does not look like the screen',diff)
  # The camera too, beside the shared screen: the browser's own card asks, and both
  # videos keep getting pictures at once.
  assert '[web.capture.card]' not in t()[s:],'the camera card came before the gate looked'
  wait(r'\[web.capture.card\] x=(\d+) y=(\d+) w=(\d+) h=(\d+) allow=(\d+),(\d+) block=(\d+),(\d+)',s,60);time.sleep(1)
  cards=re.findall(r'\[web.capture.card\] x=(\d+) y=(\d+) w=(\d+) h=(\d+) allow=(\d+),(\d+) block=(\d+),(\d+)',t()[s:])
  ax,ay=map(int,cards[-1][4:6])
  s=len(t());click(ox+ax,oy+ay)
  wait(r'\[web.capture.answer\] origin=\S+ button=0',s,30)
  wait(r'\[web.capture.camera\] on=1 source=fake',s,30)
  assert wait(r'\[web.console\] level=0 share camera (.+)',s,60)[1].strip()=='SexOS test camera'
  both=wait(r'\[web.console\] level=0 share both (\d+) (\d+) (\S+)',s,60)
  assert int(both[1])>=2 and int(both[2])>=5 and both[3]=='live',('screen and camera both live',both[0])
  assert '[display.share.stopped]' not in t()[s:],'the camera stopped the share'
  shot('share-and-camera')
  # The person stops sharing in the Shell: the page's track ends, the bank is wiped;
  # the camera stays on.
  s=len(t());click(px,py)
  wait(r'\[shell.share.stop\] by=person',s,30)
  wait(rf'\[shell.share.answer\] pd={target} task={task} status=4 ',s,30)
  wait(r'\[display.share.stopped\] frames=\d+ scrubbed=1',s,30)
  assert wait(r'\[web.console\] level=0 share ended (\S+) (\S+) (\S+)',s,30)[1:4]==('ended','false','live')
  wait(r'\[web.share.stop\] ',s,30)
  time.sleep(1);img=shot('share-stopped')
  assert not near(img.getpixel((1,rh//2)),red,40),'red ring stayed after stopping'
  # A second ask, answered no with Escape: NotAllowedError, and nothing is shared.
  s=len(t());click(ox+bx,oy+BAR+by)
  wait(r'\[shell.share.picker\] windows=',s,60);time.sleep(1)
  s2=len(t());q.key('esc')
  wait(rf'\[shell.share.answer\] pd={target} task={task} status=2 ',s2,30)
  assert wait(r'\[web.console\] level=0 share again (\S+)',s2,30)[1]=='NotAllowedError'
  assert '[display.share.started]' not in t()[s:],'shared after Escape'
  s=len(t());g['focus'](surface);time.sleep(.3);q.key('f11');wait(rf'\[web.closed\] target={target} task={task}',s,60)
  text=t();assert 'KERNEL PANIC' not in text and '[web.fatal]' not in text and 'panicked' not in text
  print({'ok':True,'frames':frames,'diff':round(diff,1),'colours':colours,'video':(vx,vy,vw,vh)})
 finally:g['stop']()
if __name__=='__main__':main()
