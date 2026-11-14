#!/usr/bin/env python3
"""Kaleidoscope extensions (N7a): an MV3 extension packed as a .crx (from
scripts/web_site/extensions/sample) is installed from a link on about:extensions. On a
page served over the network its content script runs, its style sheet turns the page
pink, and its declarativeNetRequest rule blocks a picture and a script before they
leave. Turned off on about:extensions, it stays off in the next run and the two
requests reach the server."""
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
CRX=crx()
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
BLOCKED=['/site/extensions/blocked-by-extension.png','/site/extensions/blocked-by-extension.js']
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
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
  g['wait'](r'\[web.extension.installed\] id=recolour-sample files=5 name=Recolour Sample',s,300)
  g['wait'](r'\[web.extension.loaded\] id=recolour-sample mv=3 enabled=1 rules=1 skipped=0 content_scripts=1 background=0',s,60)
  assert '/sample.crx' in hits,hits
  s=go(page);g['wait'](loaded,s,600)
  g['wait'](r'\[web.console\] level=0 extension content script ran',s,60)
  g['wait'](r'\[web.extension.content\] css=1 early=0 late=1 url=\S+page.html',s,5)
  blocked=re.findall(r'\[web.extension.blocked\] id=recolour-sample kind=(\S+) ',t()[s:])
  assert sorted(blocked)==['image','script'],blocked
  assert not [h for h in hits if h in BLOCKED],hits
  time.sleep(2);on=pink('extension-on');assert on>3000,f'page not recoloured ({on} pink pixels)'
  s=go('about:extensions?toggle=recolour-sample');g['wait'](r'\[web.extension.toggle\] id=recolour-sample enabled=0',s,120)
  s=len(t());q.key('f11');g['wait'](rf'\[web.closed\] target={target} task={task}',s,60)
  # The next run: still installed, still off; nothing blocked or recoloured.
  hits.clear();g['focus'](153);time.sleep(.5);s=len(t());keys(f'run +net kaleidoscope {page}');q.key('ret')
  g['wait'](r'\[web.extension.loaded\] id=recolour-sample mv=3 enabled=0 ',s,600);g['wait'](loaded,s,1200)
  g['wait'](r'\[web.console\] level=0 blocked script ran',s,120)
  assert '[web.extension.blocked]' not in t()[s:] and '[web.extension.content]' not in t()[s:]
  assert sorted(h for h in hits if h in BLOCKED)==sorted(BLOCKED),hits
  time.sleep(2);off=pink('extension-off');assert off<on/10,(on,off)
  text=t();assert 'KERNEL PANIC' not in text and '[web.fatal]' not in text and 'panicked' not in text
  print({'ok':True,'blocked':blocked,'pink_on':on,'pink_off':off})
 finally:g['stop']()
if __name__=='__main__':main()
