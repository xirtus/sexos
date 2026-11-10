#!/usr/bin/env python3
"""CSS animations run on their own: a page with a `@keyframes` animation and no script
is drawn again and again while it runs (passing through in-between colours and
positions), ends on its last frame (fill-mode forwards), and then stops drawing.
Hovering a box with a `transition` fades its background through in-between colours."""
import argparse,http.server,os,re,shutil,threading,time
from PIL import Image
from pathlib import Path
from browser_gate_viewport import full_viewport
ROOT=Path(__file__).resolve().parents[1]
SITE=ROOT/'scripts/web_site'
BAR=30
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  if self.path=='/web':body=(ROOT/'iso_root/programs/web').read_bytes();self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
  path=self.path.split('?')[0]  # a submitted form's query names the same file
  f=SITE/path.removeprefix('/site/') if path.startswith('/site/') else None
  if f is None or not f.is_file():self.send_response(404);self.end_headers();return
  body=f.read_bytes();self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
server=Quiet(('127.0.0.1',0),Handler);PORT=server.server_address[1]
threading.Thread(target=server.serve_forever,daemon=True).start()
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
from quil_autosave_gate import boot
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 base=f'http://10.0.2.2:{PORT}';url=f'{base}/site/animate.html'
 g=boot(a.iso,disk,a.out,'webanimate',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
   elif k=='+':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'equal'}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 def frames(s,target):return len(re.findall(r'\[web.shown\] w=410 h=330 ',t()[s:]))
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'fetch {base}/web /bin/web');q.key('ret')
  m=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,240);assert m[1].startswith('Saved'),m[1]
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'run +net web {url}');q.key('ret')
  m=g['wait'](r'\[web.ready\] target=(\d+) task=(\d+)',s,300);target,task=m[1],m[2]
  launch=g['wait'](rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',s,60);surface=int(launch[1])
  g['wait'](rf'\[display.app.pixels\] target={target} surface={surface} ',s,60);time.sleep(.5)
  x,y,w,h=map(int,re.findall(rf'\[shell.tile.apply\] sid={surface} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',t())[-1])
  ox,oy=x+8,y+36+BAR  # the page's top-left on screen
  # Four seconds of animation: on the way the box is part red, part blue, and moving.
  mixed=0;deadline=time.time()+7
  while time.time()<deadline:
   shot=a.out/'during.ppm';q.cmd('screendump',{'filename':str(shot)});time.sleep(.3)
   r,_,b=Image.open(shot).convert('RGB').getpixel((ox+150,oy+40))
   mixed+=r>40 and b>40
  assert mixed>=1,'never saw the box between its first and last frame'
  time.sleep(2)
  ran=frames(s,target);assert ran>=30,f'only {ran} frames drawn while animating'
  s2=len(t());time.sleep(3);after=frames(s2,target);assert after==0,f'{after} frames drawn after the animation ended'
  # Keep the initial animation timing intact; reveal the full page before the hover check.
  x,y,w,h=full_viewport(g,ROOT,surface)
  ox,oy=x+8,y+36+BAR
  shot=a.out/'animate.ppm';q.cmd('screendump',{'filename':str(shot)});time.sleep(.5)
  img=Image.open(shot).convert('RGB')
  img.crop((x+8,y+36,x+8+410,y+36+330)).save(a.out/'animate.png')
  moved,left=img.getpixel((ox+260,oy+40)),img.getpixel((ox+60,oy+40))
  assert moved[2]>200 and moved[0]<60,('the box ends blue at 200 px',moved)
  assert min(left)>200,('nothing left behind',left)
  # Hover the second box: green fades to magenta over 2 s.
  rw,rh=map(int,a.resolution.split('x'))
  q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':round((ox+60)*32767/(rw-1))}},{'type':'abs','data':{'axis':'y','value':round((oy+110)*32767/(rh-1))}}]})
  faded=0;deadline=time.time()+4
  while time.time()<deadline:
   shot=a.out/'hover.ppm';q.cmd('screendump',{'filename':str(shot)});time.sleep(.3)
   faded+=all(40<c<215 for c in Image.open(shot).convert('RGB').getpixel((ox+20,oy+110)))
  time.sleep(1)
  shot=a.out/'hovered.ppm';q.cmd('screendump',{'filename':str(shot)});time.sleep(.5)
  end=Image.open(shot).convert('RGB').getpixel((ox+20,oy+110))
  assert faded>=1,'the hover never showed an in-between colour'
  assert end[0]>200 and end[1]<60 and end[2]>200,('the hover ends magenta',end)
  s3=len(t());q.key('f11');g['wait'](rf'\[web.closed\] target={target} task={task}',s3,60)
  text=t();assert 'KERNEL PANIC' not in text and '[web.fatal]' not in text and 'panicked' not in text
  print({'ok':True,'frames':ran,'between':mixed,'hover_between':faded})
 finally:g['stop']()
if __name__=='__main__':main()
