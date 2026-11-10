#!/usr/bin/env python3
"""Web loads an iframe as a page of its own (YouTube stage Y8): the frame's page comes
from another origin, its request carries the embedding page as `Referer` (only the
origin, across origins), its scripts run, and the two pages talk by postMessage, a
message reaching only the origin it names. The frame is drawn over its box, and a
click there reaches the frame's page."""
import argparse,http.server,os,re,shutil,threading,time
from pathlib import Path
from browser_gate_viewport import live_viewport
from web_surface_geometry import canvas_box
from PIL import Image
ROOT=Path(__file__).resolve().parents[1]
SITE=ROOT/'scripts/web_site'
SEEN=[]
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  SEEN.append((self.server.server_address[1],self.path,self.headers.get('Referer')))
  if self.path=='/web':body=(ROOT/'iso_root/programs/web').read_bytes();self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
  f=SITE/self.path.removeprefix('/site/') if self.path.startswith('/site/') else None
  if f is None or not f.is_file():self.send_response(404);self.end_headers();return
  body=f.read_bytes();self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
# Two origins: the page's, and the frame's on the next port.
for _ in range(50):
 a_srv=Quiet(('127.0.0.1',0),Handler);PORT=a_srv.server_address[1]
 try:b_srv=Quiet(('127.0.0.1',PORT+1),Handler);break
 except OSError:a_srv.server_close()
for s in (a_srv,b_srv):threading.Thread(target=s.serve_forever,daemon=True).start()
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
from quil_autosave_gate import boot
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 rw,rh=map(int,a.resolution.split('x'))
 base=f'http://10.0.2.2:{PORT}';url=f'{base}/site/frame.html'
 g=boot(a.iso,disk,a.out,'webframes',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
   elif k=='+':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'equal'}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 def move(x,y):q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':round(x*32767/(rw-1))}},{'type':'abs','data':{'axis':'y','value':round(y*32767/(rh-1))}}]});time.sleep(.3)
 def button(down):q.cmd('input-send-event',{'events':[{'type':'btn','data':{'down':down,'button':'left'}}]});time.sleep(.2)
 def shot(name,tile,px,py):
  path=a.out/f'{name}.ppm';q.cmd('screendump',{'filename':str(path)});time.sleep(.5)
  img=Image.open(path).convert('RGB');x,y,_,_=canvas_box(ROOT,tile,pw,ph)
  img.crop((x,y,x+pw,y+ph)).save(a.out/f'{name}.png')
  return img.getpixel((x+px,y+py))
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'fetch {base}/web /bin/web');q.key('ret')
  m=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,240);assert m[1].startswith('Saved'),m[1]
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'run +net web {url}');q.key('ret')
  frame=g['wait'](r'\[web.frame.loaded\] node=(\d+) items=(\d+) scripted=(\d) referrer=(\S+)',s,300)
  assert frame[3]=='1' and frame[4]==f'{base}/',frame[0]
  verdict=g['wait'](r'\[web.console\] level=0 frames (ok|fail [^\n]*)',s,120);assert verdict[1]=='ok',verdict[0]
  g['wait'](r'\[web.console\] level=0 framed got hi frame',s,60)
  g['wait'](r'\[web.console\] level=0 frame load event',s,60)
  # The frame's own request: from the page's origin, naming only that origin.
  asked=[r for r in SEEN if r[0]==PORT+1 and r[1]=='/site/framed.html'];assert asked and asked[0][2]==f'{base}/',('referer',asked,SEEN)
  m=g['wait'](r'\[web.ready\] target=(\d+) task=(\d+)',s,120);target,task=m[1],m[2]
  surface=int(g['wait'](rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',s,60)[1])
  time.sleep(3)
  tile,pw,ph=live_viewport(g,ROOT,surface)
  bx,by,_,_=canvas_box(ROOT,tile,pw,ph)
  green=shot('frames-loaded',tile,60,90);assert green[1]>150 and green[0]<80 and green[2]<110,('frame not drawn',green)
  # A click on the frame reaches its page, which turns blue and tells the page.
  s=len(t());move(bx+120,by+100);button(True);button(False)
  g['wait'](r'\[web.console\] level=0 frame clicked 1',s,60);time.sleep(1.5)
  blue=shot('frames-clicked',tile,60,90);assert blue[2]>150 and blue[1]<110,('click not shown',blue)
  text=t();assert 'framed fail' not in text and 'frames fail' not in text
  s=len(t());g['focus'](surface);time.sleep(.3);q.key('f11');g['wait'](rf'\[web.closed\] target={target} task={task}',s,60)
  text=t();assert 'KERNEL PANIC' not in text and '[web.fatal]' not in text and 'panicked' not in text
  print({'ok':True,'frame':frame[0],'referer':asked[0][2],'green':green,'blue':blue})
 finally:g['stop']()
if __name__=='__main__':main()
