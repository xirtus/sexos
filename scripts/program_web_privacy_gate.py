#!/usr/bin/env python3
"""Kaleidoscope's Brave-style privacy defaults (N13a). While Shields are up, cookies are
first party only: a page on site A that loads a picture, a frame and a fetch from site B
sends none of B's cookies, B's Set-Cookie there is ignored, and B's frame gets no
document.cookie. Visited directly, B still has its own cookie. With Shields down for A,
B's cookie goes along again. Every request carries `Sec-GPC: 1`.

Site A is the host at 10.0.2.2. Site B is 10.0.2.100, which QEMU forwards (guestfwd) to
the same test server through a small relay."""
import argparse,http.server,os,shutil,sys,threading,time
from pathlib import Path
from urllib.parse import unquote
from PIL import Image
ROOT=Path(__file__).resolve().parents[1]
reqs=[]
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  host=(self.headers.get('Host') or '').split(':')[0]
  reqs.append((host,self.path,self.headers.get('Cookie'),self.headers.get('Sec-GPC')))
  head=[];ctype='text/html'
  if self.path=='/web':body=(ROOT/'iso_root/programs/kaleidoscope').read_bytes();ctype='application/octet-stream'
  elif self.path=='/b/set':head=[('Set-Cookie','b=own; Path=/; Max-Age=86400')];body=b'<!doctype html><title>B set</title><p>B set its cookie.</p>'
  elif self.path.startswith('/b/show'):body=b'<!doctype html><title>B show</title><p>B.</p>'
  elif self.path=='/b/pixel.png':
   head=[('Set-Cookie','b2=tracked; Path=/; Max-Age=86400')];ctype='image/png'
   body=bytes.fromhex('89504e470d0a1a0a0000000d4948445200000001000000010806000000'+'1f15c4890000000d49444154789c6360000002000154a24f5d0000000049454e44ae426082')
  elif self.path=='/b/frame':
   head=[('Set-Cookie','b3=framed; Path=/; Max-Age=86400')]
   body=b'<!doctype html><title>B frame</title><p>frame</p><script>fetch("/b/seen/"+encodeURIComponent(document.cookie||"none"))</script>'
  elif self.path.startswith('/b/seen/') or self.path=='/b/api':body=b'ok';ctype='text/plain'
  elif self.path.startswith('/a/page'):
   body=b'<!doctype html><title>A</title><p>Site A with things from B.</p><img src="http://10.0.2.100/b/pixel.png" width=20 height=20><iframe src="http://10.0.2.100/b/frame" width=200 height=60></iframe><script>fetch("http://10.0.2.100/b/api")</script>'
   if self.path=='/a/page2':
    head=[('Set-Cookie','a=own; Path=/; Max-Age=86400')]
    body+=b'<script>localStorage.setItem("view","saved"); console.log("site data ready")</script>'
  else:self.send_response(404);self.send_header('Content-Length','0');self.end_headers();return
  self.send_response(200);self.send_header('Content-Type',ctype)
  for k,v in head:self.send_header(k,v)
  self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
server=Quiet(('127.0.0.1',0),Handler);PORT=server.server_address[1]
threading.Thread(target=server.serve_forever,daemon=True).start()
RELAY='''import socket,sys,threading,os
s=socket.create_connection(("127.0.0.1",%d))
def up():
 while True:
  d=os.read(0,65536)
  if not d:break
  s.sendall(d)
 try:s.shutdown(socket.SHUT_WR)
 except OSError:pass
threading.Thread(target=up,daemon=True).start()
while True:
 d=s.recv(65536)
 if not d:break
 os.write(1,d)
'''
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus','=':'equal'}
SHIFTED={':':'semicolon','?':'slash','+':'equal','_':'minus'}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 relay=a.out.resolve()/'relay.py';relay.write_text(RELAY%PORT)
 sh=a.out.resolve()/'relay.sh';sh.write_text(f'#!/bin/sh\nexec {sys.executable} {relay}\n');sh.chmod(0o755)
 os.environ['SEXOS_GATE_NIC']=f'-netdev user,id=n0,guestfwd=tcp:10.0.2.100:80-cmd:{sh} -device e1000e,netdev=n0'
 from quil_autosave_gate import boot
 A=f'http://10.0.2.2:{PORT}';B='http://10.0.2.100'
 g=boot(a.iso,disk,a.out,'privacy',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k in SHIFTED:q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':SHIFTED[k]}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 loaded=r'\[web.loaded\] bytes_ok=1 '
 def go(url):
  s=len(t());q.key('slash');time.sleep(.3);keys(url);q.key('ret');g['wait'](loaded,s,300);time.sleep(3);return s
 def seen(host,path):return [r for r in reqs if r[0]==host and r[1]==path]
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'fetch {A}/web /bin/kaleidoscope');q.key('ret')
  m=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,240);assert m[1].startswith('Saved'),m[1]
  # B visited directly keeps its own cookie (first party there).
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'run +net kaleidoscope {B}/b/set');q.key('ret')
  g['wait'](loaded,s,1200);g['wait'](r'\[web.ready\] target=(\d+) task=(\d+)',s,60);time.sleep(2)
  assert seen('10.0.2.100','/b/set'),reqs
  go(f'{B}/b/show1');assert seen('10.0.2.100','/b/show1')[-1][2]=='b=own',reqs[-3:]
  # On A, B's picture, frame and fetch carry none of B's cookies and set none.
  reqs.clear();go(f'{A}/a/page1')
  for _ in range(40):
   if any(r[1].startswith('/b/seen/') for r in reqs):break
   time.sleep(.5)
  third=[r for r in reqs if r[0]=='10.0.2.100']
  paths={r[1] for r in third}
  assert {'/b/pixel.png','/b/frame','/b/api'}<=paths,paths
  assert all(r[2] is None for r in third),third
  frame_cookie=[unquote(r[1][len('/b/seen/'):]) for r in third if r[1].startswith('/b/seen/')]
  assert frame_cookie==['none'],frame_cookie
  assert all(r[3]=='1' for r in reqs),[r for r in reqs if r[3]!='1']
  go(f'{B}/b/show2');assert seen('10.0.2.100','/b/show2')[-1][2]=='b=own',seen('10.0.2.100','/b/show2')
  # Shields down for A: B's cookie goes along again, as in Brave.
  go(f'{A}/a/page2');reqs.clear()
  s=len(t());q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':dn,'key':{'type':'qcode','data':x}}} for dn,ks in((True,('ctrl','shift','s')),(False,('s','shift','ctrl'))) for x in ks]})
  g['wait'](r'\[web.shields.toggle\] up=0',s,60);g['wait'](loaded,s,300);time.sleep(4)
  g['wait'](r'\[web.console\] level=0 site data ready',s,60)
  px=seen('10.0.2.100','/b/pixel.png');assert px and px[-1][2] and 'b=own' in px[-1][2],px
  # Site settings are opened by a browser key, and their controls act on A even
  # though the visible address is now about:site.
  def chord(*keys):
   q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':dn,'key':{'type':'qcode','data':x}}} for dn,ks in((True,keys),(False,keys[::-1])) for x in ks]})
  s=len(t());chord('ctrl','shift','e')
  g['wait'](r'\[web.site.settings\] origin=http://10\.0\.2\.2:'+str(PORT)+r' shields=Off box=Main cookies=1 storage=1 bytes=9',s,120)
  time.sleep(1)
  shot=a.out/'site-settings.ppm';q.cmd('screendump',{'filename':str(shot)})
  Image.open(shot).convert('RGB').save(a.out/'site-settings.png')
  s=len(t());chord('ctrl','shift','s')
  g['wait'](r'\[web.shields.toggle\] up=1 host=10\.0\.2\.2',s,60)
  g['wait'](r'\[web.site.settings\] origin=http://10\.0\.2\.2:'+str(PORT)+r' shields=On box=Main cookies=1 storage=1 bytes=9',s,120)
  # Selectively clear one kind of site data in the active box. The first
  # action must leave local storage alone, and the second must leave Shields on.
  s=len(t());chord('ctrl','alt','c')
  g['wait'](r'\[web.site.part-cleared\] kind=cookies count=1 kept=1 url=http://10\.0\.2\.2:'+str(PORT)+r'/a/page2',s,60)
  g['wait'](r'\[web.site.settings\] origin=http://10\.0\.2\.2:'+str(PORT)+r' shields=On box=Main cookies=0 storage=1 bytes=9',s,120)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'site-cookies-cleared.ppm')})
  assert (a.out/'site-cookies-cleared.ppm').stat().st_size>100000
  s=len(t());chord('ctrl','alt','s')
  g['wait'](r'\[web.site.part-cleared\] kind=storage count=1 kept=1 url=http://10\.0\.2\.2:'+str(PORT)+r'/a/page2',s,60)
  g['wait'](r'\[web.site.settings\] origin=http://10\.0\.2\.2:'+str(PORT)+r' shields=On box=Main cookies=0 storage=0 bytes=0',s,120)
  s=len(t());chord('ctrl','shift','backspace')
  g['wait'](r'\[web.site.cleared\] url=http://10\.0\.2\.2:'+str(PORT)+r'/a/page2',s,60)
  g['wait'](r'\[web.site.settings\] origin=http://10\.0\.2\.2:'+str(PORT)+r' shields=On box=Main cookies=0 storage=0 bytes=0',s,120)
  text=t();assert 'KERNEL PANIC' not in text and '[web.fatal]' not in text and 'panicked' not in text
  print({'ok':True,'third_party_requests':len(third),'frame_cookie':frame_cookie[0],'shields_down_cookie':px[-1][2]})
 finally:g['stop']()
if __name__=='__main__':main()
