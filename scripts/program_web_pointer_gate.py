#!/usr/bin/env python3
"""The mouse reaches pages: Shell turns the USB tablet's moves, presses and wheel steps
over a program window into pointer events in that window's pixels, and Web gives the
page DOM mouse events at the element under the pointer (offsetX/Y on a canvas,
click, contextmenu, wheel), and follows a link a click lands on."""
import argparse,http.server,os,re,shutil,threading,time
from pathlib import Path
from browser_gate_viewport import full_viewport, content_rect
ROOT=Path(__file__).resolve().parents[1]
SITE=ROOT/'scripts/web_site'
BAR=30
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  if self.path=='/web':body=(ROOT/'iso_root/programs/web').read_bytes();self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
  f=SITE/self.path.removeprefix('/site/') if self.path.startswith('/site/') else None
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
 rw,rh=map(int,a.resolution.split('x'))
 base=f'http://10.0.2.2:{PORT}';url=f'{base}/site/mouse.html'
 g=boot(a.iso,disk,a.out,'webptr',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
   elif k=='+':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'equal'}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 def move(x,y):
  q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':round(x*32767/(rw-1))}},{'type':'abs','data':{'axis':'y','value':round(y*32767/(rh-1))}}]});time.sleep(.3)
 def button(name,down):q.cmd('input-send-event',{'events':[{'type':'btn','data':{'down':down,'button':name}}]});time.sleep(.2)
 def near(m,x,y):assert abs(int(m[1])-x)<=1 and abs(int(m[2])-y)<=1,(m[0],x,y)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'fetch {base}/web /bin/web');q.key('ret')
  m=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,240);assert m[1].startswith('Saved'),m[1]
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'run +net web {url}');q.key('ret')
  g['wait'](r'\[web.console\] level=0 mouse ready',s,300)
  m=g['wait'](r'\[web.ready\] target=(\d+) task=(\d+)',s,120);target,task=m[1],m[2]
  launch=g['wait'](rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',s,60);surface=int(launch[1])
  time.sleep(3)
  x,y,w,h=full_viewport(g,ROOT,surface)
  ox,oy,_,_=content_rect(t(),(x,y,w,h));oy+=BAR  # page origin after browser toolbar
  s=len(t());move(ox+50,oy+40)
  near(g['wait'](r'\[web.console\] level=0 mouse move (-?\d+),(-?\d+)',s,60),50,40)
  s=len(t());button('left',True);button('left',False)
  m=g['wait'](r'\[web.console\] level=0 mouse down (-?\d+),(-?\d+) button 0 buttons 1',s,60);near(m,50,40)
  near(g['wait'](r'\[web.console\] level=0 mouse click (-?\d+),(-?\d+)',s,60),50,40)
  g['wait'](rf'\[shell.program.pointer\] pd={target} task={task} ',s,10)
  s=len(t());move(ox+120,oy+70);button('right',True);button('right',False)
  g['wait'](r'\[web.console\] level=0 mouse menu 2',s,60)
  s=len(t());button('wheel-up',True);button('wheel-up',False)
  g['wait'](r'\[web.console\] level=0 mouse wheel -100',s,60)
  # The link sits in the paragraph under the canvas: 100 + 8 px down, a few px in.
  s=len(t());move(ox+20,oy+100+8+8);button('left',True);button('left',False)
  g['wait'](r'\[web.console\] level=0 mouse link',s,60)
  g['wait'](r'\[web.console\] level=0 mouse arrived',s,120)
  s=len(t());q.key('f11');g['wait'](rf'\[web.closed\] target={target} task={task}',s,60)
  text=t();assert 'KERNEL PANIC' not in text and '[web.fatal]' not in text and 'panicked' not in text
  print({'ok':True,'tile':(x,y,w,h)})
 finally:g['stop']()
if __name__=='__main__':main()
