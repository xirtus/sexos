#!/usr/bin/env python3
"""Kaleidoscope private and container tabs (N4). Each box keeps its own cookies and
localStorage. A cookie set in the person's own tab can't be seen from a container tab, a
second container, or a private tab, and the reverse holds too. Tabs in the same box share
it. When the last private tab closes, the private box's cookies are forgotten, so a new
private tab starts empty. A container's cookies are saved to its own folder in Files.

The test site: /set/<v> answers with `Set-Cookie: who=<v>` and a script that stores
`who=<v>` in localStorage. /show/<tag> runs a script that fetches /ls/<tag>/<what
localStorage holds>. The server notes the Cookie header of every request."""
import argparse,base64,hashlib,http.server,os,shutil,threading,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
seen={}
ls={}
PRIVATE_SENTINEL='privateDiagCanary7519'
ws_started=threading.Event()
ws_release=threading.Event()
ws_answered=threading.Event()
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  cookie=self.headers.get('Cookie')
  parts=self.path.strip('/').split('/')
  head=[]
  if parts[0]=='ws' and len(parts)==2 and parts[1]==PRIVATE_SENTINEL:
   ws_started.set();assert ws_release.wait(30),'private WebSocket was never released'
   key=self.headers.get('Sec-WebSocket-Key','')
   answer=base64.b64encode(hashlib.sha1((key+'258EAFA5-E914-47DA-95CA-C5AB0DC85B11').encode()).digest()).decode()
   self.send_response(101);self.send_header('Upgrade','websocket');self.send_header('Connection','Upgrade')
   self.send_header('Sec-WebSocket-Accept',answer);self.send_header('Sec-WebSocket-Protocol',PRIVATE_SENTINEL);self.end_headers();ws_answered.set();return
  if self.path=='/web':body=(ROOT/'iso_root/programs/kaleidoscope').read_bytes()
  elif parts[0]=='private-canary':
   body=f'''<!doctype html><title>{PRIVATE_SENTINEL}</title><p>Private page.</p><script>
console.log('{PRIVATE_SENTINEL}');new WebSocket('ws://10.0.2.2:{PORT}/ws/{PRIVATE_SENTINEL}','{PRIVATE_SENTINEL}');
</script>'''.encode()
  elif parts[0]=='set' and len(parts)==2:
   v=parts[1];head=[('Set-Cookie',f'who={v}; Path=/; Max-Age=86400')]
   body=f'<!doctype html><title>set {v}</title><p>Set {v}.</p><script>localStorage.setItem("who","{v}")</script>'.encode()
  elif parts[0]=='show' and len(parts)==2:
   seen[parts[1]]=cookie
   body=f'<!doctype html><title>show {parts[1]}</title><p>Show.</p><script>fetch("/ls/{parts[1]}/"+(localStorage.getItem("who")||"none"))</script>'.encode()
  elif parts[0]=='ls' and len(parts)==3:ls[parts[1]]=parts[2];body=b'ok'
  else:self.send_response(404);self.send_header('Content-Length','0');self.end_headers();return
  self.send_response(200)
  for k,v in head:self.send_header(k,v)
  self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
server=Quiet(('127.0.0.1',0),Handler);PORT=server.server_address[1]
threading.Thread(target=server.serve_forever,daemon=True).start()
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
from quil_autosave_gate import boot
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus','=':'equal'}
SHIFTED={':':'semicolon','?':'slash','+':'equal','_':'minus'}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 base=f'http://10.0.2.2:{PORT}'
 g=boot(a.iso,disk,a.out,'private',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k in SHIFTED:q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':SHIFTED[k]}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 def chord(*ks):
  s=len(t());q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':dn,'key':{'type':'qcode','data':x}}} for dn,order in((True,ks),(False,ks[::-1])) for x in order]});time.sleep(.8);return s
 loaded=r'\[web.loaded\] bytes_ok=1 '
 def go(path,typing=False):
  s=len(t())
  if not typing:q.key('slash');time.sleep(.3)
  keys(base+path);q.key('ret');g['wait'](loaded,s,300);time.sleep(1.5);return s
 def show(tag,typing=False):
  go(f'/show/{tag}',typing)
  for _ in range(60):
   if tag in ls:break
   time.sleep(.5)
  return seen.get(tag),ls.get(tag)
 def tab(*ks,box):
  s=chord('ctrl',*ks);g['wait'](rf'\[web.tab.opened\] index=\d+ count=\d+ box={box}\b',s,60);time.sleep(.5)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'fetch {base}/web /bin/kaleidoscope');q.key('ret')
  m=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,240);assert m[1].startswith('Saved'),m[1]
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'run +net kaleidoscope {base}/set/own');q.key('ret')
  g['wait'](loaded,s,1200);g['wait'](r'\[web.ready\] target=(\d+) task=(\d+)',s,60);time.sleep(2)
  results={}
  results['own']=show('own1');assert results['own']==('who=own','own'),results
  # Container 1 (Personal): nothing of the person's own; then its own cookie.
  tab('shift','1',box='1')
  results['a1']=show('a1',True);assert results['a1']==(None,'none'),results
  go('/set/a');results['a2']=show('a2');assert results['a2']==('who=a','a'),results
  # Container 2 (Work) sees neither.
  tab('shift','2',box='2')
  results['b1']=show('b1',True);assert results['b1']==(None,'none'),results
  # A private tab sees none; its own cookie is shared with the next private tab.
  tab('shift','p',box='private')
  results['p1']=show('p1',True);assert results['p1']==(None,'none'),results
  go('/set/p');results['p2']=show('p2');assert results['p2']==('who=p','p'),results
  tab('t',box='private')
  results['p3']=show('p3',True);assert results['p3']==('who=p','p'),results
  # A private page's URL, title, console and a delayed WebSocket protocol must
  # stay out of the serial stream, even after another tab becomes active.
  s=len(t());go(f'/private-canary/{PRIVATE_SENTINEL}?key={PRIVATE_SENTINEL}')
  assert ws_started.wait(30),'private WebSocket did not begin'
  time.sleep(.3);q.cmd('screendump',{'filename':str(a.out/'web-private-diagnostics.ppm')})
  assert (a.out/'web-private-diagnostics.ppm').stat().st_size>100000
  chord('ctrl','1');ws_release.set()
  assert ws_answered.wait(30),'private WebSocket did not finish after tab switch'
  time.sleep(2)
  assert PRIVATE_SENTINEL not in t()[s:],'private content reached the serial log after tab switch'
  chord('ctrl','5');time.sleep(.5)
  # Closing both private tabs forgets the private box.
  s=chord('ctrl','w');time.sleep(1);chord('ctrl','w')
  m=g['wait'](r'\[web.private.forgotten\] cookies=(\d+) origins=(\d+)',s,60);assert m[1]=='1',m[0]
  tab('shift','p',box='private')
  results['p4']=show('p4',True);assert results['p4']==(None,'none'),results
  # The person's own tab still has its own; a second Personal tab shares Personal's.
  s=chord('ctrl','1');g['wait'](r'\[web.tab.shown\] index=0 ',s,30);time.sleep(1)
  results['own2']=show('own2');assert results['own2']==('who=own','own'),results
  tab('shift','1',box='1')
  results['a3']=show('a3',True);assert results['a3']==('who=a','a'),results
  # The container's cookies reach its own folder; the private box's never do.
  m=g['wait'](r'\[web.jarbox.saved\] box=1 cookies=(\d+) origins=(\d+)',0,60);assert m[1]=='1' and m[2]=='1',m[0]
  text=t();assert '[web.jarbox.saved] box=private' not in text
  assert 'KERNEL PANIC' not in text and '[web.fatal]' not in text and 'panicked' not in text
  print({'ok':True,**{k:v for k,v in results.items()}})
 finally:g['stop']()
if __name__=='__main__':main()
