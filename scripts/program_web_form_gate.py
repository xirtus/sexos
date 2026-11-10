#!/usr/bin/env python3
"""Web forms work: a click focuses a text field, typing (with Backspace) edits it, a
click toggles a checkbox, and Enter submits the form as a GET with every field; that
page then submits a POST form, whose answer is the next page."""
import argparse,http.server,json,os,re,shutil,threading,time
from pathlib import Path
from browser_gate_viewport import live_viewport
from web_surface_geometry import canvas_box
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
 def do_POST(self):
  n=int(self.headers.get('Content-Length','0'));sent=self.rfile.read(n).decode('latin-1')
  kind=self.headers.get('Content-Type','')
  body=f'<!doctype html><p>posted</p><script>console.log({json.dumps("form posted "+kind+" "+sent)})</script>'.encode()
  self.send_response(200);self.send_header('Content-Type','text/html');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
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
 base=f'http://10.0.2.2:{PORT}';url=f'{base}/site/form.html'
 g=boot(a.iso,disk,a.out,'webform',a.resolution);t=g['text'];q=g['q']
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
  g['wait'](r'\[web.console\] level=0 form ready',s,300)
  m=g['wait'](r'\[web.ready\] target=(\d+) task=(\d+)',s,120);target,task=m[1],m[2]
  launch=g['wait'](rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',s,60);surface=int(launch[1])
  time.sleep(3)
  tile,pw,ph=live_viewport(g,ROOT,surface)
  x,y,w,h=tile
  bx,by,_,_=canvas_box(ROOT,tile,pw,ph)
  ox,oy=bx,by+BAR  # the page's top-left on screen
  def click(px,py):move(ox+px,oy+py);button('left',True);button('left',False)
  s=len(t());click(20,10)
  g['wait'](r'\[web.console\] level=0 form focus',s,60)
  s=len(t());keys('sexosx');q.key('backspace');time.sleep(.3)
  g['wait'](r'\[web.console\] level=0 form typing',s,60)
  # The checkbox sits under the 26 px field.
  s=len(t());click(5,32)
  g['wait'](r'\[web.console\] level=0 form check true',s,60)
  s=len(t());click(20,10);g['wait'](r'\[web.console\] level=0 form focus',s,60)
  s=len(t());q.key('ret')
  m=g['wait'](r'\[web.console\] level=0 form got (\S+)',s,120)
  assert m[1]=='?q=sexos&c=on&t=two%0D%0Alines',m[1]
  m=g['wait'](r'\[web.console\] level=0 form posted ([^\n]*)',s,120)
  assert m[1].strip()=='application/x-www-form-urlencoded a=1+2&b=x%26y',m[1]
  s=len(t());q.key('f11');g['wait'](rf'\[web.closed\] target={target} task={task}',s,60)
  text=t();assert 'KERNEL PANIC' not in text and '[web.fatal]' not in text and 'panicked' not in text
  print({'ok':True,'tile':(x,y,w,h)})
 finally:g['stop']()
if __name__=='__main__':main()
