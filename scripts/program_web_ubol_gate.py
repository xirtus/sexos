#!/usr/bin/env python3
"""uBlock Origin Lite, unmodified, in Kaleidoscope (workstream N7b). Not part of CI: it
downloads the extension as published in uBlockOrigin/uBOL-home (its chromium/ folder,
from raw.githubusercontent.com: the background's module graph, the rulesets turned on
by default, their details and scripting files, and _locales/en) and zips it, or uses the
zip given with --zip.

On about:extensions it installs; its service worker (an ES module) starts in its own
realm, loads its rulesets and registers its content scripts through
chrome.scripting. On the Shields test page with Shields turned down for the site, its
declarativeNetRequest rules block the trackers before they leave."""
import argparse,http.server,io,os,re,shutil,struct,threading,time,zipfile
from pathlib import Path
from PIL import Image
ROOT=Path(__file__).resolve().parents[1]
SITE=ROOT/'scripts/web_site'
def crx():
 b=io.BytesIO()
 with zipfile.ZipFile(b,'w',zipfile.ZIP_DEFLATED) as z:
  for f in sorted((SITE/'extensions/sample').rglob('*')):
   if f.is_file():z.write(f,f.relative_to(SITE/'extensions/sample').as_posix())
 hdr=b'\x0a\x00';return b'Cr24'+struct.pack('<II',3,len(hdr))+hdr+b.getvalue()
RAW='https://raw.githubusercontent.com/uBlockOrigin/uBOL-home/main/chromium/'
def fetch_ubol():
 import json,urllib.request,posixpath
 files={}
 def get(p):
  if p in files:return files[p]
  try:files[p]=urllib.request.urlopen(RAW+p,timeout=60).read()
  except Exception:files[p]=None
  return files[p]
 m=json.loads(get('manifest.json'));todo=[m['background']['service_worker'].lstrip('/')]
 while todo:
  p=todo.pop();b=get(p) if p not in files else None
  if b is None:continue
  for x in re.finditer(rb'''(?:import|export)\s*(?:[^'";]*?from\s*)?['"](\.{1,2}/[^'"]+|/[^'"]+)['"]''',b):
   spec=x.group(1).decode();todo.append(spec[1:] if spec.startswith('/') else posixpath.normpath(posixpath.join(posixpath.dirname(p),spec)))
 on=[r for r in m['declarative_net_request']['rule_resources'] if r.get('enabled')]
 for p in ['_locales/en/messages.json','managed_storage.json']+[f'rulesets/{n}-details.json' for n in ('ruleset','generic','scriptlet')]+[r['path'].lstrip('/') for r in on]:get(p)
 for n in ('css-api','css-generic','css-procedural-api','css-specific','css-user-terminate','css-user','isolated-api','prevent-popup','prevent-popup-target','toolbar-icon'):get(f'js/scripting/{n}.js')
 for r in on:
  for d in ('regex','scripting/generic','scripting/specific'):get(f"rulesets/{d}/{r['id']}.json")
  for w in ('isolated','main'):get(f"rulesets/scripting/scriptlet/{w}/{r['id']}.js")
 for w in ('isolated','main'):
  for e in ('begin','end'):get(f'rulesets/scripting/scriptlet/{w}/scriptlet-{e}.js')
 b=io.BytesIO()
 with zipfile.ZipFile(b,'w',zipfile.ZIP_DEFLATED) as z:
  for p,data in sorted(files.items()):
   if data is not None:z.writestr(p,data)
 return b.getvalue()
CRX=None
hits=[]
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  hits.append(self.path)
  if self.path=='/web':body=(ROOT/'iso_root/programs/kaleidoscope').read_bytes()
  elif self.path=='/sample.crx':body=CRX
  else:
   f=SITE/self.path.removeprefix('/site/') if self.path.startswith('/site/') else None
   if f is None or not f.is_file():self.send_response(404);self.send_header('Content-Length','0');self.end_headers();return
   body=f.read_bytes()
  self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
server=Quiet(('127.0.0.1',0),Handler);PORT=server.server_address[1]
threading.Thread(target=server.serve_forever,daemon=True).start()
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
from quil_autosave_gate import boot
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus','=':'equal'}
SHIFTED={':':'semicolon','?':'slash','+':'equal','_':'minus'}
BLOCKED=['/site/extensions/blocked-by-extension.png','/site/extensions/blocked-by-extension.js','/site/extensions/blocked-dynamic.png']
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');p.add_argument('--zip',type=Path);a=p.parse_args()
 global CRX;CRX=a.zip.read_bytes() if a.zip else fetch_ubol()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 base=f'http://10.0.2.2:{PORT}';page=f'{base}/site/extensions/page.html'
 g=boot(a.iso,disk,a.out,'extensions',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k in SHIFTED:q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':SHIFTED[k]}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 def go(url):s=len(t());q.key('slash');time.sleep(.3);keys(url);q.key('ret');return s
 def pink(name):
  shot=a.out/f'{name}.ppm';q.cmd('screendump',{'filename':str(shot)});time.sleep(.5)
  img=Image.open(shot).convert('RGB');img.save(a.out/f'{name}.png')
  return sum(1 for px in img.getdata() if px==(255,224,240))
 loaded=r'\[web.loaded\] bytes_ok=1 '
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'fetch {base}/web /bin/kaleidoscope');q.key('ret')
  m=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,240);assert m[1].startswith('Saved'),m[1]
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'run +net kaleidoscope {page}');q.key('ret')
  g['wait'](loaded,s,1200);m=g['wait'](r'\[web.ready\] target=(\d+) task=(\d+)',s,60);target,task=m[1],m[2]
  assert '[web.extension.' not in t()[s:],'an extension before any was installed'
  hits.clear()
  s=go(f'about:extensions?install={base}/sample.crx')
  m=g['wait'](r'\[web.extension.(installed|install_failed)\] ([^\n]*)',s,600);assert m[1]=='installed',m[0]
  rules=int(g['wait'](r'\[web.extension.loaded\] id=ublock-origin-lite mv=3 enabled=1 rules=(\d+) ',s,120)[1]);assert rules>50000,rules
  g['wait'](r'\[web.extension.background\] id=ublock-origin-lite scripts=1 module=1 ',s,300)
  registered=int(g['wait'](r'\[web.extension.registered\] id=ublock-origin-lite scripts=(\d+)',s,300)[1]);assert registered>0
  time.sleep(5)
  s=go(f'{base}/site/shields/index.html');g['wait'](loaded,s,600);time.sleep(10)
  s=len(t());q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':'shift'},{'type':'qcode','data':'s'}],'hold-time':80})
  g['wait'](r'\[web.shields.toggle\] up=0',s,60);g['wait'](loaded,s,600);time.sleep(20)
  blocked=re.findall(r'\[web.extension.blocked\] id=ublock-origin-lite kind=\S+ filter=\S+ url=(\S+)',t()[s:])
  assert len(blocked)>=5,blocked
  assert '[web.shields.blocked]' not in t()[s:]
  text=t();assert 'KERNEL PANIC' not in text and '[web.fatal]' not in text and 'panicked' not in text and 'trapped=' not in text
  print({'ok':True,'rules':rules,'registered_scripts':registered,'blocked':blocked})
 finally:g['stop']()
if __name__=='__main__':main()
