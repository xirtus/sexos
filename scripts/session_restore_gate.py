#!/usr/bin/env python3
"""Session restore: two programs are open, one moved to workspace 3; the machine goes down without warning; after the next boot both come back, each on its workspace, with the file each was given."""
import argparse,http.server,io,os,shutil,threading,time
from pathlib import Path
from PIL import Image
ROOT=Path(__file__).resolve().parents[1]
img=Image.new('RGB',(300,200),(200,80,40));b=io.BytesIO();img.save(b,'PNG')
FILES={'/view':(ROOT/'iso_root/programs/view').read_bytes(),'/pic.png':b.getvalue()}
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
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 launch=r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)'
 g=boot(a.iso,disk,a.out,'session',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k=='@':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'2'}],'hold-time':80})
   elif k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 def expect(text,pattern,timeout=120):
  g['focus'](153);time.sleep(.5);s=len(t());keys(text);q.key('ret');return g['wait'](pattern,s,timeout)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  for url,name in [('/view','/bin/view'),('/pic.png','pic.png')]:
   m=expect(f'fetch http://10.0.2.2:{PORT}{url} {name}',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',300);assert m[1].startswith('Saved'),m[1]
  # The first window goes to workspace 3 (Super+Shift+3); the second stays.
  s=len(t());expect('run view @pic.png',r'\[spindle.session.started\] target=(\d+) task=(\d+) scene=0',240)
  first=g['wait'](launch,s,60);g['wait'](r'\[view.shown\]',s,120);time.sleep(1)
  g['focus'](int(first[3]));time.sleep(.5);s=len(t())
  q.cmd('send-key',{'keys':[{'type':'qcode','data':'meta_l'},{'type':'qcode','data':'shift'},{'type':'qcode','data':'3'}],'hold-time':120})
  g['wait'](rf'\[spindle.session.placed\] target={first[1]} task={first[2]} scene=2',s,60)
  s=len(t());expect('run view @pic.png',r'\[spindle.session.started\] target=(\d+) task=(\d+) scene=0',240)
  second=g['wait'](launch,s,60);g['wait'](r'\[view.shown\]',s,120)
  g['wait'](r'\[spindle.session.saved\] bytes=(\d+) status=0',s,60);time.sleep(1)
 finally:g['stop']()
 # Power gone, no goodbye. The next boot opens both again.
 g=boot(a.iso,disk,a.out,'session2',a.resolution);t=g['text'];q=g['q']
 try:
  g['wait'](r'\[spindle.session.read\] status=0 programs=2',0,240)
  restored=[g['wait'](r'\[spindle.session.restore\] scene=(\d) path=/bin/view',0,120)]
  g['wait'](r'\[spindle.session.started\] target=\d+ task=\d+ scene=\d',0,240)
  g['wait'](r'\[spindle.session.restored\]',0,300)
  log=t();scenes=sorted(int(x) for x in __import__('re').findall(r'\[spindle.session.restore\] scene=(\d) path=/bin/view',log))
  assert scenes==[0,2],scenes
  assert len(__import__('re').findall(r'\[spindle.session.started\]',log))==2,'both did not start'
  g['wait'](r'\[shell.launch.placed\] target=\d+ surface=\d+ scene=2',0,60)
  assert len(__import__('re').findall(r'\[files.program.bound\] target=\d+ task=\d+ root=/data/view',log))==2
  time.sleep(3);q.cmd('screendump',{'filename':str(a.out/'session-restored.ppm')});time.sleep(.5)
  assert 'KERNEL PANIC' not in log and 'panicked' not in log
  print({'ok':True,'scenes':scenes})
 finally:g['stop']()
if __name__=='__main__':main()
