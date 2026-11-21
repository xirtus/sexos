#!/usr/bin/env python3
"""Kaleidoscope side panel (N13e). Ctrl+Shift+B opens a panel 360 px wide at the right
of the window, as in Chrome, Edge and Brave, on the bookmarks and recent pages; the
page beside it hears `resize` with innerWidth 362 px less. A press on the panel gives
it the keyboard, and a link followed there opens in the page beside it. New tabs open
beside the panel, and the strip leaves the panel's page out. An extension's
`side_panel` page opens there through chrome.sidePanel.open() (here from its options
page), laid out to the panel's width. Ctrl+Shift+B shows the bookmarks again, and once
more closes the panel: the page takes the width back.

The test site: /p/<name> logs `vt <name> ready|resize <innerWidth>` to the console;
/sample.crx is the sample extension (scripts/web_site/extensions/sample)."""
import argparse,http.server,io,os,re,shutil,struct,threading,time,zipfile
from pathlib import Path
from browser_gate_viewport import live_viewport
from web_surface_geometry import canvas_box
ROOT=Path(__file__).resolve().parents[1]
SAMPLE=ROOT/'scripts/web_site/extensions/sample'
BAR=30
PANEL=360
GAP=2
PAGE=('<!doctype html><title>Tab {n}</title><p>Tab {n}.</p><script>'
 'console.log("vt {n} ready "+innerWidth);'
 'addEventListener("resize",()=>console.log("vt {n} resize "+innerWidth));'
 '</script>')
def crx():
 b=io.BytesIO()
 with zipfile.ZipFile(b,'w',zipfile.ZIP_DEFLATED) as z:
  for f in sorted(SAMPLE.rglob('*')):
   if f.is_file():z.write(f,f.relative_to(SAMPLE).as_posix())
 hdr=b'\x0a\x00';return b'Cr24'+struct.pack('<II',3,len(hdr))+hdr+b.getvalue()
CRX=crx()
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  parts=self.path.strip('/').split('/')
  if self.path=='/web':body=(ROOT/'iso_root/programs/kaleidoscope').read_bytes()
  elif self.path=='/sample.crx':body=CRX
  elif parts[0]=='p' and len(parts)==2 and parts[1].isalnum():body=PAGE.format(n=parts[1]).encode()
  else:self.send_response(404);self.send_header('Content-Length','0');self.end_headers();return
  self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
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
 rw,rh=map(int,a.resolution.split('x'))
 base=f'http://10.0.2.2:{PORT}'
 g=boot(a.iso,disk,a.out,'sidepanel',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k in SHIFTED:q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':SHIFTED[k]}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 def go(url):s=len(t());q.key('slash');time.sleep(.3);keys(url);q.key('ret');return s
 def chord(*ks):
  s=len(t());q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':dn,'key':{'type':'qcode','data':x}}} for dn,order in((True,ks),(False,ks[::-1])) for x in order]});time.sleep(.8);return s
 def move(x,y):
  q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':round(x*32767/(rw-1))}},{'type':'abs','data':{'axis':'y','value':round(y*32767/(rh-1))}}]});time.sleep(.3)
 def click(x,y):
  move(x,y);q.cmd('input-send-event',{'events':[{'type':'btn','data':{'down':True,'button':'left'}}]});time.sleep(.2)
  q.cmd('input-send-event',{'events':[{'type':'btn','data':{'down':False,'button':'left'}}]});time.sleep(.2)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'fetch {base}/web /bin/kaleidoscope');q.key('ret')
  m=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,240);assert m[1].startswith('Saved'),m[1]
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'run +net kaleidoscope {base}/p/a');q.key('ret')
  g['wait'](r'\[web.console\] level=0 vt a ready (\d+)',s,1200)
  m=g['wait'](r'\[web.ready\] target=(\d+) task=(\d+)',s,60);target,task=m[1],m[2]
  surface=int(g['wait'](rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',s,60)[1])
  time.sleep(3)
  tile,pw,ph=live_viewport(g,ROOT,surface);time.sleep(2)
  wide=int(re.findall(r'\[web.console\] level=0 vt a (?:ready|resize) (\d+)',t()[s:])[-1])
  bx,by,_,_=canvas_box(ROOT,tile,pw,ph)
  narrow=wide-PANEL-GAP
  # A bookmarked, then B in a second tab.
  s=chord('ctrl','d');g['wait'](r'\[web.bookmark\] on=1 count=1 ',s,60)
  s=chord('ctrl','t');g['wait'](r'\[web.tab.opened\] index=1 count=2 ',s,60);time.sleep(.5)
  keys(f'{base}/p/b');q.key('ret');assert int(g['wait'](r'\[web.console\] level=0 vt b ready (\d+)',s,300)[1])==wide
  # The bookmarks panel: B is told it is narrower.
  s=chord('ctrl','shift','b')
  g['wait'](rf'\[web.panel\] on=1 w={PANEL} url=about:bookmarks\n',s,60)
  assert int(g['wait'](r'\[web.console\] level=0 vt b resize (\d+)',s,120)[1])==narrow
  # A press on the panel gives it the keyboard; its first link (A) opens beside it.
  px=bx+wide-PANEL
  s=len(t());click(px+40,by+BAR+200);g['wait'](r'\[web.split.focus\] shown=2\n',s,60)
  s=len(t())
  for y in range(36,90,6):
   click(px+30,by+BAR+y);time.sleep(1)
   if '[web.panel.follow]' in t()[s:]:break
  m=g['wait'](r'\[web.panel.follow\] shown=1 url=([^\n]*)',s,30);assert m[1]==f'{base}/p/a',m[0]
  assert int(g['wait'](r'\[web.console\] level=0 vt a ready (\d+)',s,300)[1])==narrow
  shot=a.out/'bookmarks-panel.ppm';q.cmd('screendump',{'filename':str(shot)});time.sleep(1)
  # A new tab, asked for from the panel, opens beside it, before its slot.
  s=len(t());click(px+40,by+BAR+200);g['wait'](r'\[web.split.focus\] shown=2\n',s,60)
  s=chord('ctrl','t');g['wait'](r'\[web.tab.opened\] index=2 count=4 ',s,60);time.sleep(.5)
  keys(f'{base}/p/c');q.key('ret');assert int(g['wait'](r'\[web.console\] level=0 vt c ready (\d+)',s,300)[1])==narrow
  # Ctrl+Shift+B again closes it: C takes the width back.
  s=chord('ctrl','shift','b');g['wait'](r'\[web.panel\] on=0 shown=2\n',s,60)
  assert int(g['wait'](r'\[web.console\] level=0 vt c resize (\d+)',s,120)[1])==wide
  # An extension's side panel, opened by chrome.sidePanel.open() on its options page.
  s=go(f'about:extensions?install={base}/sample.crx')
  g['wait'](r'\[web.extension.installed\] id=recolour-sample files=12 ',s,300)
  # Its background settles first (it adds a rule), so no typing is lost meanwhile.
  g['wait'](r'\[web.extension.console\] id=recolour-sample level=0 dynamic rule added',s,120);g['wait'](r'\[web.loaded\] ',s,120);time.sleep(2)
  panel='kaleidoscope-extension://recolour-sample/panel.html'
  s=go('kaleidoscope-extension://recolour-sample/options.html')
  g['wait'](rf'\[web.console\] level=0 panel ready {PANEL} function {re.escape(panel)}\n',s,120)
  g['wait'](rf'\[web.panel\] on=1 w={PANEL} url={re.escape(panel)}\n',s,60)
  time.sleep(2);shot=a.out/'extension-panel.ppm';q.cmd('screendump',{'filename':str(shot)});time.sleep(1)
  # Ctrl+Shift+B shows the bookmarks there instead; once more closes it.
  s=chord('ctrl','shift','b');g['wait'](rf'\[web.panel\] on=1 w={PANEL} url=about:bookmarks\n',s,60)
  s=chord('ctrl','shift','b');g['wait'](r'\[web.panel\] on=0 shown=2\n',s,60)
  text=t();assert 'KERNEL PANIC' not in text and '[web.fatal]' not in text and 'panicked' not in text
  print({'ok':True,'wide':wide,'narrow':narrow})
 finally:g['stop']()
if __name__=='__main__':main()
