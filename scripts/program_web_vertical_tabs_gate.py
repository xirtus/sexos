#!/usr/bin/env python3
"""Kaleidoscope vertical tabs (N13b). Ctrl+Shift+, moves the tabs from the bar to a
strip down the left side, as in Edge and Brave. The page gets the rest of the width:
it hears `resize` with innerWidth 200 px less, and its mouse coordinates start right of
the strip. Clicking a tab in the strip shows it (a tab that waited relays out to the new
width). Ctrl+Shift+\\ shows the tab and the next one side by side (split view): each
half lays out to its own width, a press on the other half gives it the keyboard, and
pages get coordinates in their own half. Ctrl+Shift+, again puts the tabs back, and the
choice is kept for next time.

The test site: /p/<name> logs `vt <name> ready|resize <innerWidth>` and
`vt <name> down <clientX>,<clientY>` to the console."""
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
 g=boot(a.iso,disk,a.out,'vtabs',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k in SHIFTED:q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':SHIFTED[k]}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 def chord(*ks):
  s=len(t());q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':dn,'key':{'type':'qcode','data':x}}} for dn,order in((True,ks),(False,ks[::-1])) for x in order]});time.sleep(.8);return s
 def move(x,y):
  q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':round(x*32767/(rw-1))}},{'type':'abs','data':{'axis':'y','value':round(y*32767/(rh-1))}}]});time.sleep(.3)
 def click(x,y):
  move(x,y);q.cmd('input-send-event',{'events':[{'type':'btn','data':{'down':True,'button':'left'}}]});time.sleep(.2)
  q.cmd('input-send-event',{'events':[{'type':'btn','data':{'down':False,'button':'left'}}]});time.sleep(.2)
 def launch():
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'run +net kaleidoscope {base}/p/a');q.key('ret')
  w=int(g['wait'](r'\[web.console\] level=0 vt a ready (\d+)',s,1200)[1])
  m=g['wait'](r'\[web.ready\] target=(\d+) task=(\d+)',s,60);target,task=m[1],m[2]
  surface=int(g['wait'](rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',s,60)[1])
  time.sleep(3)
  # The window is tiled to its full size; the page's last width then counts.
  tile,pw,ph=live_viewport(g,ROOT,surface);time.sleep(2)
  w=int(re.findall(r'\[web.console\] level=0 vt a (?:ready|resize) (\d+)',t()[s:])[-1])
  return w,target,task,tile,pw,ph
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'fetch {base}/web /bin/kaleidoscope');q.key('ret')
  m=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,240);assert m[1].startswith('Saved'),m[1]
  wide,target,task,tile,pw,ph=launch()
  bx,by,_,_=canvas_box(ROOT,tile,pw,ph)
  # A second tab, B, shown.
  s=chord('ctrl','t');g['wait'](r'\[web.tab.opened\] index=1 count=2 ',s,60);time.sleep(.5)
  keys(f'{base}/p/b');q.key('ret');assert int(g['wait'](r'\[web.console\] level=0 vt b ready (\d+)',s,300)[1])==wide
  # Tabs down the side: B is told it is 200 px narrower.
  s=chord('ctrl','shift','comma')
  g['wait'](rf'\[web.tabs.vertical\] on=1 side={SIDE}\n',s,60)
  assert int(g['wait'](r'\[web.console\] level=0 vt b resize (\d+)',s,120)[1])==wide-SIDE
  # A click on the first tab in the strip shows A, laid out to the narrower width.
  s=len(t());click(bx+SIDE//2,by+BAR+6+SIDE_ROW//2)
  g['wait'](r'\[web.tab.shown\] index=0 count=2 ',s,60)
  assert int(g['wait'](r'\[web.console\] level=0 vt a resize (\d+)',s,120)[1])==wide-SIDE
  # The page's own coordinates start right of the strip.
  s=len(t());click(bx+SIDE+50,by+BAR+40)
  m=g['wait'](r'\[web.console\] level=0 vt a down (-?\d+),(-?\d+)',s,60)
  assert abs(int(m[1])-50)<=1 and abs(int(m[2])-40)<=1,m[0]
  # Split view (Ctrl+Shift+\\): A and B side by side beside the strip, halves 531 wide.
  half=(wide-SIDE-2)//2
  s=chord('ctrl','shift','backslash')
  g['wait'](rf'\[web.split\] on=1 left=0 right=1 shown=0 w={half}\n',s,60)
  assert int(g['wait'](r'\[web.console\] level=0 vt a resize (\d+)',s,120)[1])==half
  assert int(g['wait'](r'\[web.console\] level=0 vt b resize (\d+)',s,120)[1])==wide-SIDE-2-half
  # The left half has the keyboard; a press there reaches A in its own coordinates.
  s=len(t());click(bx+SIDE+50,by+BAR+40)
  m=g['wait'](r'\[web.console\] level=0 vt a down (-?\d+),(-?\d+)',s,60)
  assert abs(int(m[1])-50)<=1 and abs(int(m[2])-40)<=1,m[0]
  # A press on the right half gives it the keyboard; the next one reaches B.
  right=bx+SIDE+half+2
  s=len(t());click(right+50,by+BAR+40);g['wait'](r'\[web.split.focus\] shown=1\n',s,60)
  s=len(t());click(right+50,by+BAR+40)
  m=g['wait'](r'\[web.console\] level=0 vt b down (-?\d+),(-?\d+)',s,60)
  assert abs(int(m[1])-50)<=1 and abs(int(m[2])-40)<=1,m[0]
  shot=a.out/'split.ppm';q.cmd('screendump',{'filename':str(shot)});time.sleep(1)
  # One again: B takes the whole width beside the strip; A follows when shown.
  s=chord('ctrl','shift','backslash')
  g['wait'](r'\[web.split\] on=0 shown=1\n',s,60)
  assert int(g['wait'](r'\[web.console\] level=0 vt b resize (\d+)',s,120)[1])==wide-SIDE
  s=len(t());click(bx+SIDE//2,by+BAR+6+SIDE_ROW//2)
  g['wait'](r'\[web.tab.shown\] index=0 count=2 ',s,60)
  assert int(g['wait'](r'\[web.console\] level=0 vt a resize (\d+)',s,120)[1])==wide-SIDE
  # Back across the bar: full width again.
  s=chord('ctrl','shift','comma')
  g['wait'](r'\[web.tabs.vertical\] on=0 side=0\n',s,60)
  assert int(g['wait'](r'\[web.console\] level=0 vt a resize (\d+)',s,120)[1])==wide
  # On again, and kept: a new window starts with the strip.
  s=chord('ctrl','shift','comma');g['wait'](rf'\[web.tabs.vertical\] on=1 side={SIDE}\n',s,60)
  s=len(t());q.key('f11');g['wait'](rf'\[web.closed\] target={target} task={task}',s,60);time.sleep(2)
  again=launch()[0]
  assert again==wide-SIDE,(again,wide)
  text=t();assert 'KERNEL PANIC' not in text and '[web.fatal]' not in text and 'panicked' not in text
  print({'ok':True,'wide':wide,'narrow':again})
 finally:g['stop']()
if __name__=='__main__':main()
