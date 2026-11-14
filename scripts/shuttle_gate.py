#!/usr/bin/env python3
"""A program starts another program (sex_pdx::launch): Shuttle, given one picture, opens it in View, which gets that file alone; when View is closed Shuttle hears it ended with status 0; a grant Shuttle does not hold is refused by Files; and a run Shuttle started can be ended by Shuttle."""
import argparse,http.server,io,os,shutil,threading,time
from pathlib import Path
from PIL import Image
ROOT=Path(__file__).resolve().parents[1]
img=Image.new('RGB',(300,200),(40,120,200));b=io.BytesIO();img.save(b,'PNG')
FILES={'/shuttle':(ROOT/'iso_root/programs/shuttle').read_bytes(),'/view':(ROOT/'iso_root/programs/view').read_bytes(),'/pic.png':b.getvalue()}
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
 g=boot(a.iso,disk,a.out,'launch',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k=='@':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'2'}],'hold-time':80})
   elif k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 def expect(text,pattern,timeout=120):
  g['focus'](153);time.sleep(.5);s=len(t());keys(text);q.key('ret');return g['wait'](pattern,s,timeout)
 def fetched(url,name):
  m=expect(f'fetch http://10.0.2.2:{PORT}{url} {name}',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',300);assert m[1].startswith('Saved'),m[1]
 def close(target,task,surface):
  s=len(t());g['focus'](int(surface));time.sleep(.3);q.key('f11');g['wait'](rf'\[app.private.retired\] target={target} task={task} ',s,90)
 launch=r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)'
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  fetched('/shuttle','/bin/shuttle');fetched('/view','/bin/view');fetched('/pic.png','pic.png')
  # Shuttle, given pic.png alone, opens it in View: the type picks the program.
  s=len(t());expect('run shuttle @pic.png',r'\[shell.launch.asked\] parent=(\d+) task=(\d+) cookie=1 network=0',240)
  opener=g['wait'](launch,s,60)
  m=g['wait'](r'\[shuttle.started\] row=(\d+) task=(\d+) kill=0',s,120)
  g['wait'](r'\[view.shown\] w=\d+ h=\d+',s,120)
  child=g['wait'](rf'\[shell.program.launch\] target={m[1]} task={m[2]} surface=(\d+)',s,30)
  log=t()[s:];assert f'[files.program.bound] target={m[1]} task={m[2]}' in log,'the child was not bound'
  time.sleep(2);q.cmd('screendump',{'filename':str(a.out/'launch-shuttle.ppm')});time.sleep(.5)
  # View ends when closed and says so; Shuttle hears status 0.
  s2=len(t());close(m[1],m[2],child[1]);g['wait'](r'\[shuttle.done\] status=Some\(0\)',s2,60)
  time.sleep(3);q.cmd('screendump',{'filename':str(a.out/'launch-done.ppm')});time.sleep(.5)
  close(opener[1],opener[2],opener[3])
  # A grant Shuttle was not given is refused by Files before anything starts.
  s=len(t());expect('run shuttle -a view',r'\[shuttle.refused\] by=files',240)
  assert '[files.launch.denied]' in t()[s:],'Files did not refuse the grant'
  opener=g['wait'](launch,s,30);close(opener[1],opener[2],opener[3])
  # A run Shuttle started, Shuttle can end; it ended without a status.
  s=len(t());expect('run shuttle -k @pic.png',r'\[shuttle.started\] row=(\d+) task=(\d+) kill=1',240)
  opener=g['wait'](launch,s,30)
  g['wait'](r'\[shell.launch.kill\] parent=\d+ child=\d+ task=\d+ ok=1',s,60)
  g['wait'](r'\[shuttle.done\] status=None',s,90)
  close(opener[1],opener[2],opener[3])
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
  print({'ok':True})
 finally:g['stop']()
if __name__=='__main__':main()
