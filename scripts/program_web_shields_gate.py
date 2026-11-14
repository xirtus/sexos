#!/usr/bin/env python3
"""Kaleidoscope's Shields (N1): a page served over the network asks for twelve ads and
trackers (scripts, pictures, a style sheet, a frame and a fetch, on its own server and
on tracker hosts). None may leave: the server sees none of its four tracker paths, the
log names twelve blocked requests, the page's own style sheet, picture and script still
load, and the hiding rules reach the page. Then, on a page of four tracker paths,
Ctrl+Shift+S turns Shields down for the site: the reload reaches all four, and the
setting outlasts the page."""
import argparse,http.server,os,re,shutil,threading,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
SITE=ROOT/'scripts/web_site'
TRACKERS=['/pagead/js/adsbygoogle.js','/ads.js','/prebid.js','/track.gif']
hits=[]
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  hits.append(self.path)
  if self.path=='/web':body=(ROOT/'iso_root/programs/kaleidoscope').read_bytes();self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
  if self.path in TRACKERS:body=b'/* tracker */';self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
  f=SITE/self.path.removeprefix('/site/') if self.path.startswith('/site/') else None
  if f is None or not f.is_file():self.send_response(404);self.send_header('Content-Length','0');self.end_headers();return
  body=f.read_bytes();self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
server=Quiet(('127.0.0.1',0),Handler);PORT=server.server_address[1]
threading.Thread(target=server.serve_forever,daemon=True).start()
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
from quil_autosave_gate import boot
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 base=f'http://10.0.2.2:{PORT}';url=f'{base}/site/shields/index.html';local=f'{base}/site/shields/local.html'
 g=boot(a.iso,disk,a.out,'shields',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
   elif k=='+':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'equal'}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 def shot(name):q.cmd('screendump',{'filename':str(a.out/f'{name}.ppm')});time.sleep(.5)
 loaded=r'\[web.loaded\] bytes_ok=1 items=(\d+) links=(\d+) height=(\d+) sheets=(\d+) images=(\d+) '
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'fetch {base}/web /bin/kaleidoscope');q.key('ret')
  m=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,240);assert m[1].startswith('Saved'),m[1]
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'run +net kaleidoscope {url}');q.key('ret')
  sl=g['wait'](r'\[web.shields.loaded\] lists=(\d+) network=(\d+) cosmetic=(\d+) skipped=(\d+) off=(\d+) ticks=(\d+)',s,600)
  assert int(sl[2])>50 and sl[4]=='0' and sl[5]=='0',sl.group(0)
  first=g['wait'](loaded,s,1200)
  g['wait'](r'\[web.console\] level=0 shields mixpanel (reached|stopped)',s,300)
  g['wait'](r'\[web.console\] level=0 shields ok script ran',s,60)
  time.sleep(2);shot('shields-up')
  text=t()[s:]
  blocked=re.findall(r'\[web.shields.blocked\] kind=(\S+) filter=(\S+) url=(\S+)',text)
  kinds=sorted(k for k,_,_ in blocked)
  assert len(blocked)==12,blocked
  assert set(kinds)=={'script','image','stylesheet','subdocument','xmlhttprequest'},kinds
  assert 'shields mixpanel stopped' in text,'fetch to a tracker went out'
  assert not [h for h in hits if h in TRACKERS],hits
  assert '/site/style.css' in hits and '/site/logo.png' in hits and '/site/shields/ok.js' in hits,hits
  hide=re.search(r'\[web.shields.hide\] rules=(\d+) classes=(\d+) ids=(\d+) url=\S+index.html',text);assert hide and int(hide[1])>=2,hide and hide.group(0)
  assert int(first[4])>=1,first.group(0)
  # The address bar: a page with four tracker paths on this same server.
  m=g['wait'](r'\[web.ready\] target=(\d+) task=(\d+)',s,60);target,task=m[1],m[2]
  s=len(t());q.key('slash');time.sleep(.3);keys(local);q.key('ret')
  g['wait'](loaded,s,600);g['wait'](r'\[web.console\] level=0 shields ok script ran',s,60)
  assert len(re.findall(r'\[web.shields.blocked\]',t()[s:]))==4 and not [h for h in hits if h in TRACKERS],hits
  # Ctrl+Shift+S: Shields down for 10.0.2.2, and the page reloads.
  s=len(t());q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':dn,'key':{'type':'qcode','data':x}}} for dn,ks in((True,('ctrl','shift','s')),(False,('s','shift','ctrl'))) for x in ks]})
  g['wait'](r'\[web.shields.toggle\] up=0 host=10\.0\.2\.2',s,60)
  g['wait'](loaded,s,600);g['wait'](r'\[web.console\] level=0 shields ok script ran',s,60);time.sleep(1);shot('shields-down')
  assert '[web.shields.blocked]' not in t()[s:],'blocked with Shields down'
  assert sorted(h for h in hits if h in TRACKERS)==sorted(TRACKERS),hits
  s=len(t());q.key('f11');g['wait'](rf'\[web.closed\] target={target} task={task}',s,60)
  # The next run remembers the site is off.
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'run +net kaleidoscope {url}');q.key('ret')
  sl=g['wait'](r'\[web.shields.loaded\] lists=\d+ network=\d+ cosmetic=\d+ skipped=\d+ off=(\d+) ',s,600);assert sl[1]=='1',sl.group(0)
  text=t();assert 'KERNEL PANIC' not in text and '[web.fatal]' not in text and 'panicked' not in text
  print({'ok':True,'blocked':[(k,u) for k,_,u in blocked],'hide_rules':int(hide[1])})
 finally:g['stop']()
if __name__=='__main__':main()
