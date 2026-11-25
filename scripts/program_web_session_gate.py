#!/usr/bin/env python3
"""Kaleidoscope session restore (N13f), as Chrome's "Continue where you left off".
Kaleidoscope keeps its window's tabs, their groups and the shown tab as they change.
Started again without an address, it opens them again: only the shown tab loads at
once, and the others load when first shown. Private tabs are not kept.

The test site: /p/<name> logs `vt <name> ready|resize <innerWidth>` to the console."""
import argparse,http.server,os,re,shutil,threading,time
from pathlib import Path
from browser_gate_viewport import live_viewport
from web_surface_geometry import canvas_box
ROOT=Path(__file__).resolve().parents[1]
BAR=30
SIDE=200
SIDE_ROW=30
PAGE=('<!doctype html><title>Tab {n}</title><p>Tab {n}.</p><script>'
 'console.log("vt {n} ready "+innerWidth);'
 'addEventListener("resize",()=>console.log("vt {n} resize "+innerWidth));'
 'addEventListener("mousedown",e=>console.log("vt {n} down "+e.clientX+","+e.clientY));'
 '</script>')
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  parts=self.path.strip('/').split('/')
  if self.path=='/web':body=(ROOT/'iso_root/programs/kaleidoscope').read_bytes()
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
SHIFTED={':':'semicolon','+':'equal'}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 rw,rh=map(int,a.resolution.split('x'))
 base=f'http://10.0.2.2:{PORT}'
 g=boot(a.iso,disk,a.out,'session',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k in SHIFTED or k.isupper():q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':SHIFTED.get(k,k.lower())}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 def chord(*ks):
  s=len(t());q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':dn,'key':{'type':'qcode','data':x}}} for dn,order in((True,ks),(False,ks[::-1])) for x in order]});time.sleep(.8);return s
 def move(x,y):
  q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':round(x*32767/(rw-1))}},{'type':'abs','data':{'axis':'y','value':round(y*32767/(rh-1))}}]});time.sleep(.3)
 def click(x,y):
  move(x,y);q.cmd('input-send-event',{'events':[{'type':'btn','data':{'down':True,'button':'left'}}]});time.sleep(.2)
  q.cmd('input-send-event',{'events':[{'type':'btn','data':{'down':False,'button':'left'}}]});time.sleep(.2)
 def launch(first):
  g['focus'](153);time.sleep(.5);s=len(t());keys('run +net kaleidoscope'+(f' {base}/p/{first}' if first else ''));q.key('ret')
  w=int(g['wait'](rf'\[web.console\] level=0 vt {first or "b"} ready (\d+)',s,1200)[1])
  m=g['wait'](r'\[web.ready\] target=(\d+) task=(\d+)',s,60);target,task=m[1],m[2]
  surface=int(g['wait'](rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',s,60)[1])
  time.sleep(3)
  # The window is tiled to its full size; the page's last width then counts.
  tile,pw,ph=live_viewport(g,ROOT,surface);time.sleep(2)
  return s,target,task
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'fetch {base}/web /bin/kaleidoscope');q.key('ret')
  m=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,240);assert m[1].startswith('Saved'),m[1]
  _,target,task=launch('a')
  # A; B in a group; C opened from B, so in its group too; D private.
  for i,n in((1,'b'),(2,'c')):
   s=chord('ctrl','t');g['wait'](rf'\[web.tab.opened\] index={i} count={i+1} ',s,60);time.sleep(.5)
   keys(f'{base}/p/{n}');q.key('ret');g['wait'](rf'\[web.console\] level=0 vt {n} ready',s,300)
   if n=='b':s=chord('ctrl','shift','g');g['wait'](r'\[web.group\] group=0 tab=1 ',s,60)
  s=chord('ctrl','shift','p');g['wait'](r'\[web.tab.opened\] index=3 count=4 box=private',s,60);time.sleep(.5)
  s=len(t());keys(f'{base}/p/d');q.key('ret');g['wait'](r'\[web.loaded\] bytes_ok=1 ',s,300)
  assert 'vt d ready' not in t()[s:],'private page console reached serial diagnostics'
  # B shown last: the private tab is not kept.
  s=chord('ctrl','2');g['wait'](r'\[web.tab.shown\] index=1 count=4 ',s,60)
  g['wait'](r'\[web.session.saved\] tabs=3 ',s,60)
  s=len(t());q.key('f11');g['wait'](rf'\[web.closed\] target={target} task={task}',s,60);time.sleep(2)
  # Reproduce the durable-save interruption after the live names moved to their
  # previous copies. Partial staged files must not be promoted on next launch.
  def spindle(command, answer=None):
   g['focus'](153);time.sleep(.4);s=len(t());keys(command);q.key('ret')
   if answer:g['wait'](answer,s,60)
   else:time.sleep(2)
  spindle('mv Kaleidoscope/session.tsv Kaleidoscope/session.tsv.previous',r'\[spindle.fs.line\] ok')
  spindle('mv Kaleidoscope/history.tsv Kaleidoscope/history.tsv.previous',r'\[spindle.fs.line\] ok')
  spindle('write Kaleidoscope/session.tsv.next interrupted',r'\[spindle.fs.result\] kind=8 status=0 ')
  spindle('write Kaleidoscope/history.tsv.next interrupted',r'\[spindle.fs.result\] kind=8 status=0 ')
  # Started again without an address: the three tabs, B shown and loaded, A and C waiting.
  s,target,task=launch(None)
  g['wait'](r'\[web.session.restored\] tabs=3 groups=1 shown=1\n',s,60)
  history=g['wait'](r'\[web.profile.loaded\] history=(\d+) ',s,60)
  assert int(history[1])>=3,'history did not recover from previous copy'
  time.sleep(3);later=t()[s:]
  assert 'vt a ready' not in later and 'vt c ready' not in later and 'vt d ready' not in later,'a waiting tab loaded'
  shot=a.out/'restored.ppm';q.cmd('screendump',{'filename':str(shot)});time.sleep(1)
  # A loads when shown.
  s=chord('ctrl','1');g['wait'](r'\[web.tab.shown\] index=0 count=3 ',s,60);g['wait'](r'\[web.console\] level=0 vt a ready',s,300)
  # B is still in its group with C: out of it, it moves after C.
  s=chord('ctrl','2');g['wait'](r'\[web.tab.shown\] index=1 count=3 ',s,60)
  s=chord('ctrl','shift','g');g['wait'](r'\[web.group.left\] group=0 from=1 to=2\n',s,60)
  text=t();assert 'KERNEL PANIC' not in text and '[web.fatal]' not in text and 'panicked' not in text
  print({'ok':True})
 finally:g['stop']()
if __name__=='__main__':main()
