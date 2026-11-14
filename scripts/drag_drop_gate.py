#!/usr/bin/env python3
"""Drag and drop between programs (sex_pdx::dnd): a file dragged out of Files and let go
over Jot opens in Jot, which may read that file alone because the drop gave it; the
source hears who took it. A drag let go over a window that is not a program reaches no
one, and Jot is given nothing more."""
import argparse,http.server,os,re,shutil,threading,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
NOTE=b'Dropped from Files into Jot.\n'
FILES={'/files':(ROOT/'iso_root/programs/files').read_bytes(),'/jot':(ROOT/'iso_root/programs/jot').read_bytes(),'/note.txt':NOTE}
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
 g=boot(a.iso,disk,a.out,'dnd',a.resolution);t=g['text'];q=g['q']
 W,H=(int(v) for v in a.resolution.split('x'))
 def keys(text):
  for k in text:
   if k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 def expect(text,pattern,timeout=120):
  g['focus'](153);time.sleep(.5);s=len(t());keys(text);q.key('ret');return g['wait'](pattern,s,timeout)
 def move(x,y):
  q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':round(x*32767/(W-1))}},{'type':'abs','data':{'axis':'y','value':round(y*32767/(H-1))}}]});time.sleep(.3)
 def button(down):q.cmd('input-send-event',{'events':[{'type':'btn','data':{'down':down,'button':'left'}}]});time.sleep(.3)
 def drag(frm,to):
  move(*frm);button(True)
  for i in range(1,6):move(frm[0]+(to[0]-frm[0])*i//5,frm[1]+(to[1]-frm[1])*i//5)
  button(False)
 def centre(sid):
  m=re.findall(rf'\[shell.tile.apply\] sid={sid} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',t());x,y,w,h=map(int,m[-1]);return (x+w//2,y+h//2)
 launch=r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)'
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  expect('mkdir 0gate',r'\[spindle.fs.result\] kind=3 status=0')
  for name,dest in [('files','/bin/files'),('jot','/bin/jot'),('note.txt','0gate/note.txt')]:
   m=expect(f'fetch http://10.0.2.2:{PORT}/{name} {dest}',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',300);assert m[1].startswith('Saved'),m[1]
  s=len(t());expect('files /0gate',r'\[files.app.ready\] target=\d+ task=\d+ root=/0gate portal=0',120)
  fa=g['wait'](launch,s,30);g['wait'](r'\[files.app.list\] dir=/0gate count=1\b',s,60)
  s=len(t());expect('run jot',r'\[jot.ready\] target=(\d+) task=(\d+)',120);jot=g['wait'](launch,s,30)
  time.sleep(3)
  # Jot cannot read the file before it is dropped.
  log=t();assert '[files.dnd.given]' not in log
  # Files -> Jot: Files offers its selected file, Shell gives it to Jot, Jot opens it.
  s=len(t());drag(centre(fa[3]),centre(jot[3]))
  g['wait'](r'\[files.dnd.offered\] row=\d+ ticket=\d+ path=/0gate/note.txt',s,30)
  g['wait'](rf'\[shell.dnd.drop\] source={fa[1]} target={jot[1]} kind=1 len=15',s,30)
  g['wait'](rf'\[files.dnd.given\] from={fa[1]} to={jot[1]} path=/0gate/note.txt',s,30)
  g['wait'](rf'\[jot.dropped\] kind=file bytes={len(NOTE)} path=/0gate/note.txt',s,60)
  g['wait'](rf'\[files.app.dropped\] target={jot[1]}',s,30)
  time.sleep(2);q.cmd('screendump',{'filename':str(a.out/'dnd-jot.ppm')});time.sleep(.5)
  # Let go over the bar, which is no program's window: nobody takes it.
  s=len(t());drag(centre(fa[3]),(W//2,3))
  g['wait'](rf'\[shell.dnd.drop\] source={fa[1]} target=0 ',s,30)
  g['wait'](r'\[files.app.dropped\] target=0',s,30)
  assert '[files.dnd.given]' not in t()[s:],'a drop on nothing gave a grant'
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
  print({'ok':True})
 finally:g['stop']()
if __name__=='__main__':main()
