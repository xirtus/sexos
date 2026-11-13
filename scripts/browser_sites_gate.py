#!/usr/bin/env python3
"""The site suite: the browser reaches each site's expected visible text. Five pages in
the shape of everyday sites (an encyclopedia article, a GitHub repository page as GitHub
sent it, a link-aggregator front page, a news front page, and a webmail sign-in that
posts, gets a session cookie, is redirected and shows the inbox) load one after the
other in one browser, typed into the address bar. Each counts as reached when Ctrl+F
finds its text in what was laid out. The outcomes go to OUT/sites.json.

--live loads the real sites instead (needs the guest to reach the internet); the
webmail step stays local."""
import argparse,gzip,http.server,json,os,re,shutil,threading,time
from pathlib import Path
from browser_gate_viewport import live_viewport
from web_surface_geometry import canvas_box
ROOT=Path(__file__).resolve().parents[1]
SITES=ROOT/'scripts/web_site/sites'
BAR=30
SESSION='s3ss10n'
logins=[]
posts=[]
class Handler(http.server.BaseHTTPRequestHandler):
 def send(self,code,body=b'',kind='text/html; charset=utf-8',extra=()):
  self.send_response(code);self.send_header('Content-Type',kind);self.send_header('Content-Length',str(len(body)))
  for k,v in extra:self.send_header(k,v)
  self.end_headers();self.wfile.write(body)
 def do_GET(self):
  path=self.path.split('?')[0]
  if path=='/web':return self.send(200,(ROOT/'iso_root/programs/web').read_bytes(),'application/octet-stream')
  if path=='/sites/github.html':
   # Sent compressed, as GitHub does.
   return self.send(200,(SITES/'github.html.gz').read_bytes(),extra=[('Content-Encoding','gzip'),('Vary','Accept-Encoding')])
  if path=='/mail/':return self.send(200,(SITES/'mail-login.html').read_bytes().replace(b'ERROR',b''))
  if path=='/mail/inbox':
   if f'session={SESSION}' not in self.headers.get('Cookie',''):return self.send(302,extra=[('Location','/mail/')])
   rows=''.join(f'<tr><td><b>{f}</b></td><td>{s}</td><td>{d}</td></tr>' for f,s,d in [('Finance','Quarterly report is ready','09:12'),('Calendar','Team lunch on Friday','Yesterday'),('Builds','Nightly image passed','Nov 11')])
   return self.send(200,f'<!doctype html><title>Inbox (3) - Webmail</title><style>body{{font:14px sans-serif;margin:0}}header{{background:#2b5797;color:#fff;padding:8px}}table{{border-collapse:collapse;width:100%}}td{{border-bottom:1px solid #ddd;padding:6px}}</style><header>Webmail · signed in as {logins[-1] if logins else "?"}</header><h2 style="margin:8px">Inbox</h2><table>{rows}</table>'.encode())
  f=SITES/path.removeprefix('/sites/') if path.startswith('/sites/') else None
  if f is None or not f.is_file() or f.suffix!='.html':return self.send(404)
  self.send(200,f.read_bytes())
 def do_POST(self):
  n=int(self.headers.get('Content-Length','0'));sent=self.rfile.read(n).decode('latin-1')
  if self.path!='/mail/login':return self.send(404)
  from urllib.parse import parse_qs
  form={k:v[0] for k,v in parse_qs(sent).items()};posts.append(sent)
  if form.get('_user')=='reader' and form.get('_pass')=='hunter2' and form.get('_token')=='f00dfeed':
   logins.append(form['_user'])
   return self.send(303,extra=[('Location','/mail/inbox'),('Set-Cookie',f'session={SESSION}; Path=/mail; HttpOnly')])
  self.send(200,(SITES/'mail-login.html').read_bytes().replace(b'ERROR',b'<p class=err>Wrong username or password.</p>'))
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
server=Quiet(('127.0.0.1',0),Handler);PORT=server.server_address[1]
threading.Thread(target=server.serve_forever,daemon=True).start()
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
os.environ.setdefault('SEXOS_GATE_BOUND','1500')
from quil_autosave_gate import boot
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus','_':None}
BASE=f'http://10.0.2.2:{PORT}'
# name, local address, live address, text to find (lowercase), fewest matches.
SUITE=[
 ('encyclopedia',f'{BASE}/sites/wikipedia.html','https://en.wikipedia.org/wiki/Microkernel','microkernel',3),
 ('news',f'{BASE}/sites/news.html','https://text.npr.org/','comeback',1),
 ('links',f'{BASE}/sites/hackernews.html','https://news.ycombinator.com/','comments',4),
 ('github',f'{BASE}/sites/github.html','https://github.com/xirtus/sexos','microkernel',1),
]
LIVE_TEXT={'news':'npr','links':'comments','encyclopedia':'microkernel','github':'sexos'}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');p.add_argument('--live',action='store_true');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 rw,rh=map(int,a.resolution.split('x'))
 g=boot(a.iso,disk,a.out,'websites',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   shifted={':':'semicolon','+':'equal','?':'slash','_':'minus'}.get(k)
   if shifted:q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':shifted}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 def move(x,y):
  q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':round(x*32767/(rw-1))}},{'type':'abs','data':{'axis':'y','value':round(y*32767/(rh-1))}}]});time.sleep(.3)
 def button(name,down):q.cmd('input-send-event',{'events':[{'type':'btn','data':{'down':down,'button':name}}]});time.sleep(.2)
 outcomes=[]
 def loaded(s,timeout=240):
  m=g['wait'](r'\[web.loaded\] bytes_ok=1 items=(\d+) links=(\d+) height=(\d+) .*?fetch_ticks=(\d+) layout_ticks=(\d+) script_ticks=(\d+)',s,timeout)
  return {k:int(v) for k,v in zip(('items','links','height','fetch_ms','layout_ms','script_ms'),m.groups())}
 def find(text,least):
  s=len(t());g['chord']('ctrl','f',r'\[web.shown\]');keys(text)
  m=g['wait'](rf'\[web.find\] query={re.escape(text)} matches=(\d+) at=0',s,90)
  q.key('esc');time.sleep(.4)
  assert int(m[1])>=least,(text,m.group(0));return int(m[1])
 def go(url):
  s=len(t());q.key('slash');time.sleep(.3);keys(url);q.key('ret');return s
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'fetch {BASE}/web /bin/web');q.key('ret')
  m=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,240);assert m[1].startswith('Saved'),m[1]
  for i,(name,local,live,text,least) in enumerate(SUITE):
   url=live if a.live else local
   if a.live:text,least=LIVE_TEXT[name],1
   began=time.time()
   if i==0:
    g['focus'](153);time.sleep(.5);s=len(t());keys(f'run +net web {url}');q.key('ret')
    r=g['wait'](r'\[web.ready\] target=(\d+) task=(\d+)',s,300);target,task=r[1],r[2]
    stats=loaded(s,300)
    launch=g['wait'](rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',s,60);surface=int(launch[1])
   else:
    s=go(url);stats=loaded(s)
   time.sleep(1.5)
   found=find(text,least)
   outcomes.append({'site':name,'url':url,'text':text,'matches':found,'seconds':round(time.time()-began,1),**stats})
   print(outcomes[-1],flush=True)
  # Webmail: sign in, follow the redirect with the new cookie, read the inbox.
  began=time.time();s=go(f'{BASE}/mail/');loaded(s);time.sleep(1.5)
  tile,pw,ph=live_viewport(g,ROOT,surface);bx,by,_,_=canvas_box(ROOT,tile,pw,ph);ox,oy=bx,by+BAR
  def click(px,py):move(ox+px,oy+py);button('left',True);button('left',False);time.sleep(.3)
  click(20,10);keys('reader');time.sleep(1)  # keys and the pointer come by different paths
  click(20,36);keys('hunter2');time.sleep(1)
  s=len(t());q.key('ret');stats=loaded(s)
  assert logins==['reader'],posts
  found=find('quarterly',1)
  outcomes.append({'site':'webmail','url':f'{BASE}/mail/','text':'quarterly','matches':found,'seconds':round(time.time()-began,1),**stats})
  print(outcomes[-1],flush=True)
  s=len(t());q.key('f11');g['wait'](rf'\[web.closed\] target={target} task={task}',s,60)
  text=t();assert 'KERNEL PANIC' not in text and '[web.fatal]' not in text and 'panicked' not in text
  (a.out/'sites.json').write_text(json.dumps(outcomes,indent=1))
  print({'ok':True,'sites':len(outcomes)})
 finally:g['stop']()
if __name__=='__main__':main()
