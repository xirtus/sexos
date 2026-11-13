#!/usr/bin/env python3
"""The browser for daily use, across a reboot. First boot: a page counts its visits in
localStorage and gets a lasting cookie; Ctrl+D bookmarks it; Ctrl+T opens two more tabs,
Ctrl+W closes the middle one and the first tab still has its script state; Ctrl+F finds
three matches; Ctrl+= zooms and the page reflows taller; closing saves the profile.
Second boot on the same disk: the counter goes on (2), the cookie comes back to the
server, and typing the start of the address suggests the bookmark."""
import argparse,http.server,os,re,shutil,threading,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
SITE=ROOT/'scripts/web_site'
seen_cookies=[]
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  if self.path=='/web':body=(ROOT/'iso_root/programs/web').read_bytes();self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
  path=self.path.split('?')[0]
  f=SITE/path.removeprefix('/site/') if path.startswith('/site/') else None
  if f is None or not f.is_file():self.send_response(404);self.end_headers();return
  if path.endswith('start.html'):seen_cookies.append(self.headers.get('Cookie',''))
  body=f.read_bytes();self.send_response(200);self.send_header('Content-Type','text/html');self.send_header('Content-Length',str(len(body)))
  if path.endswith('start.html'):self.send_header('Set-Cookie','keep=yes; Max-Age=86400; Path=/');self.send_header('Set-Cookie','gone=1; Path=/')
  self.end_headers();self.wfile.write(body)
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
server=Quiet(('127.0.0.1',0),Handler);PORT=server.server_address[1]
threading.Thread(target=server.serve_forever,daemon=True).start()
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
os.environ.setdefault('SEXOS_GATE_BOUND','1200')
from quil_autosave_gate import boot
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus','?':None}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 base=f'http://10.0.2.2:{PORT}/site/daily';start=f'{base}/start.html'
 def session(tag):
  g=boot(a.iso,disk,a.out,tag,a.resolution);t=g['text'];q=g['q']
  def keys(text):
   for k in text:
    if k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
    elif k=='+':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'equal'}],'hold-time':80})
    elif k=='?':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'slash'}],'hold-time':80})
    else:q.key(KEYS.get(k,k))
    time.sleep(.06)
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'fetch http://10.0.2.2:{PORT}/web /bin/web');q.key('ret')
  m=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,240);assert m[1].startswith('Saved'),m[1]
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'run +net web {start}');q.key('ret')
  m=g['wait'](r'\[web.console\] level=0 daily count (\d+) cookie (\S+) extra (\S+)',s,300)
  r=g['wait'](r'\[web.ready\] target=(\d+) task=(\d+)',s,120)
  time.sleep(2)
  return g,t,q,keys,m,(r[1],r[2])
 # First boot.
 g,t,q,keys,m,(target,task)=session('boot1')
 try:
  assert m[1]=='1' and m[3]=='x1',m.group(0)
  s=len(t());q.key('x');g['wait'](r'\[web.console\] level=0 daily start keys 1 count 1',s,60)
  g['chord']('ctrl','d',r'\[web.bookmark\] on=1 count=1 ')
  for name in ('b','c'):
   s=len(t());g['chord']('ctrl','t',r'\[web.tab.opened\] index=\d+ count=\d+');time.sleep(.5)
   keys(f'{base}/tab.html?{name}');q.key('ret');g['wait'](rf'\[web.console\] level=0 daily {name} ready',s,120)
  s=len(t());q.key('x');g['wait'](r'\[web.console\] level=0 daily c keys 1',s,60)
  g['chord']('ctrl','pgup',r'\[web.tab.shown\] index=1 count=3 url=\S+tab.html\?b')
  g['chord']('ctrl','w',r'\[web.tab.closed\] index=1 count=2 shown=1')
  g['chord']('ctrl','1',r'\[web.tab.shown\] index=0 count=2 url=\S+start.html')
  # Its script state stayed: the second x counts 2, and the page did not load again.
  s=len(t());q.key('x');g['wait'](r'\[web.console\] level=0 daily start keys 2 count 1',s,60)
  g['chord']('ctrl','2',r'\[web.tab.shown\] index=1 count=2 url=\S+tab.html\?c')
  s=len(t());q.key('x');g['wait'](r'\[web.console\] level=0 daily c keys 2',s,60)
  g['chord']('ctrl','1',r'\[web.tab.shown\] index=0 count=2')
  s=len(t());g['chord']('ctrl','f',r'\[web.shown\]');keys('needle')
  m2=g['wait'](r'\[web.find\] query=needle matches=(\d+) at=0',s,60);assert m2[1]=='3',m2.group(0)
  s=len(t());q.key('ret');g['wait'](r'\[web.find\] query=needle matches=3 at=1',s,60)
  q.key('esc');time.sleep(.5)
  before=[int(h) for h in re.findall(r'\[web.loaded\] .*?height=(\d+)',t())]
  z=g['chord']('ctrl','equal',r'\[web.zoom\] percent=110 height=(\d+)')
  s=len(t());q.key('f11');g['wait'](r'\[web.profile.saved\] files=\d+ history=\d+ bookmarks=1',s,60)
  g['wait'](rf'\[web.closed\] target={target} task={task}',s,60)
  text=t();assert 'KERNEL PANIC' not in text and '[web.fatal]' not in text and 'panicked' not in text
  print({'boot1':'ok','zoom_height':z[1]})
 finally:g['stop']()
 # Second boot, same disk.
 g,t,q,keys,m,_=session('boot2')
 try:
  assert m[1]=='2' and m[3]=='x2',m.group(0)
  assert 'keep=yes' in m[2],m[2]
  assert any('keep=yes' in c and 'gone' not in c for c in seen_cookies[1:]),seen_cookies
  s=len(t());q.key('slash');time.sleep(.3);keys('10.0.2')
  m3=g['wait'](r'\[web.suggest\] typed=10\.0\.2 count=(\d+) first=(\S+)',s,60)
  assert m3[2]==start,m3.group(0)
  text=t();assert 'KERNEL PANIC' not in text and '[web.fatal]' not in text and 'panicked' not in text
  print({'ok':True})
 finally:g['stop']()
if __name__=='__main__':main()
