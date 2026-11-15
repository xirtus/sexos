#!/usr/bin/env python3
"""The system clipboard for ordinary programs (sex_pdx::program_clipboard), and the keys
held during a click (app_pointer mods).

1. Jot: text typed, Ctrl+A, Ctrl+C. Shell takes the copy from the focused program the
   person just used, and it becomes the clipboard.
2. Ctrl+N, Ctrl+V in Jot: the text comes back from Shell (not Jot's own copy).
3. Quil (a document view) pastes what the program copied.
4. A Shift-click and an Alt-click in Jot reach it with those keys in the pointer word.
5. A key going up reaches Jot as the press's word with down=0.
6. Numpad keys carry the keypad mark for a program that asked (Jot does)."""
import argparse,http.server,os,re,shutil,threading,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
FILES={'/jot':(ROOT/'iso_root/programs/jot').read_bytes()}
TEXT='copied from jot'
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  body=FILES.get(self.path)
  if body is None:self.send_response(404);self.end_headers();return
  self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
server=Quiet(('127.0.0.1',0),Handler);PORT=server.server_address[1]
threading.Thread(target=server.serve_forever,daemon=True).start()
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
from quil_autosave_gate import boot
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 shutil.rmtree(a.out,ignore_errors=True);a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 g=boot(a.iso,disk,a.out,'clip',a.resolution);t=g['text'];q=g['q']
 W,H=(int(v) for v in a.resolution.split('x'))
 def keys(text):
  for k in text:
   if k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 def chord(*names):q.cmd('send-key',{'keys':[{'type':'qcode','data':n} for n in names],'hold-time':120});time.sleep(.4)
 def expect(text,pattern,timeout=120):
  g['focus'](153);time.sleep(.5);s=len(t());keys(text);q.key('ret');return g['wait'](pattern,s,timeout)
 def move(x,y):
  q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':round(x*32767/(W-1))}},{'type':'abs','data':{'axis':'y','value':round(y*32767/(H-1))}}]});time.sleep(.3)
 def click_holding(key,x,y):
  move(x,y)
  q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':True,'key':{'type':'qcode','data':key}}}]});time.sleep(.2)
  for down in (True,False):q.cmd('input-send-event',{'events':[{'type':'btn','data':{'down':down,'button':'left'}}]});time.sleep(.2)
  q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':False,'key':{'type':'qcode','data':key}}}]});time.sleep(.4)
 def centre(sid):
  m=re.findall(rf'\[shell.tile.apply\] sid={sid} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',t());x,y,w,h=map(int,m[-1]);return (x+w//2,y+h//2)
 launch=r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)'
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  m=expect(f'fetch http://10.0.2.2:{PORT}/jot /bin/jot',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',300);assert m[1].startswith('Saved'),m[1]
  s=len(t());expect('run jot',r'\[jot.ready\] target=(\d+) task=(\d+)',120);jot=g['wait'](launch,s,30)
  g['wait'](rf'\[shell.app.registered.reply\] target={jot[1]} task={jot[2]} surface={jot[3]} committed=1',s,60)
  g['wait'](rf'\[shell.keys.wanted\] pd={jot[1]} task={jot[2]} flags=2 ok=1',s,30)
  g['focus'](int(jot[3]));time.sleep(1)
  # 1. Copy: Shell takes it from the focused program the person just typed in.
  keys(TEXT);time.sleep(.5)
  s=len(t());chord('ctrl','a');chord('ctrl','c')
  g['wait'](rf'\[shell.clip.copied\] pd={jot[1]} task={jot[2]} len={len(TEXT)} ok=1',s,30)
  g['wait'](rf'\[jot.clip.copy\] bytes={len(TEXT)} ok=1',s,30)
  # 2. Paste into a new tab: the bytes come from Shell.
  s=len(t());chord('ctrl','n');time.sleep(.5);chord('ctrl','v')
  g['wait'](rf'\[shell.clip.pasted\] pd={jot[1]} task={jot[2]} len={len(TEXT)} ok=1',s,30)
  g['wait'](rf'\[jot.clip.paste\] bytes={len(TEXT)} system=1 err=None',s,30)
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'clip-jot.ppm')});time.sleep(.5)
  # 3. Quil pastes what the program copied.
  g['focus'](201);time.sleep(1)
  s=len(t());chord('ctrl','n');g['wait'](r'\[quil.new.ok\] bytes=0',s,30)
  s=len(t());chord('ctrl','v')
  g['wait'](rf'\[quil.clipboard.paste\] ticket=\d+ bytes={len(TEXT)} generation=\d+ hash=0x[0-9a-f]+ applied=1',s,60)
  # 4. Keys held during a click ride in the pointer word (bits 52..54: Shift, Ctrl, Alt).
  g['focus'](int(jot[3]));time.sleep(1)
  x,y=centre(jot[3])
  words=[]
  for key,bit in (('shift',1),('alt',4)):
   s=len(t());click_holding(key,x,y)
   w=g['wait'](rf'\[shell.program.pointer\] pd={jot[1]} task={jot[2]} word=(0x[0-9a-f]+) status=0',s,30)
   mods=(int(w[1],16)>>52)&7;words.append((key,hex(int(w[1],16))));assert mods==bit,(key,w[0])
  # 5. A key going up reaches the focused program (Jot drops it, as older programs do).
  s=len(t());q.key('a')
  g['wait'](rf'\[shell.program.key.up\] pd={jot[1]} task={jot[2]} code=0x1e keypad=0 status=0',s,30)
  # 6. Jot asked for numpad marks: numpad 8 and the grey Up arrow share code 0x48.
  for qcode,mark in (('kp_8',1),('up',0)):
   s=len(t());q.key(qcode)
   g['wait'](rf'\[shell.program.key.up\] pd={jot[1]} task={jot[2]} code=0x48 keypad={mark} status=0',s,30)
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
  print({'ok':True,'copied':len(TEXT),'pointer':words})
 finally:g['stop']()
if __name__=='__main__':main()
