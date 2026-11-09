#!/usr/bin/env python3
"""A program reaches the network only in a run the person started with `run +net`, and gets its arguments; without `+net` it is refused."""
import argparse,http.server,os,re,shutil,threading,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
PAGE=b'<html><body><h1>Hello peek</h1><p>Fetched by a program.</p></body></html>'
FILES={'/peek':(ROOT/'iso_root/programs/peek').read_bytes(),'/page.html':PAGE}
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  body=FILES.get(self.path)
  if body is None:self.send_response(404);self.end_headers();return
  self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
server=Quiet(('127.0.0.1',0),Handler);PORT=server.server_address[1]
threading.Thread(target=server.serve_forever,daemon=True).start()
# --nic picks QEMU's controller model; read before quil_autosave_gate builds the command.
import sys
NIC=next((sys.argv[i+1]for i,x in enumerate(sys.argv[:-1])if x=='--nic'),'e1000e')
os.environ['SEXOS_GATE_NIC']=f'-netdev user,id=n0 -device {NIC},netdev=n0'
from quil_autosave_gate import boot
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');p.add_argument('--public',default='');p.add_argument('--nic',choices=['e1000','e1000e','igb'],default='e1000e');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 g=boot(a.iso,disk,a.out,'net',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
   elif k=='+':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'equal'}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 def expect(text,pattern,timeout=120):
  g['focus'](153);time.sleep(.5);s=len(t());keys(text);q.key('ret');return g['wait'](pattern,s,timeout)
 def peek(line,pattern):
  s=len(t());expect(line,pattern,240);m=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,60)
  g['wait'](r'\[peek.ready\]',s,120);time.sleep(2);return m,s
 def close(m):
  g['focus'](int(m[3]));time.sleep(.3);s=len(t());q.key('f11');g['wait'](rf'\[app.private.retired\] target={m[1]} task={m[2]} ',s,90)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  base=f'http://10.0.2.2:{PORT}'
  m=expect(f'fetch {base}/peek /bin/peek',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',240);assert m[1].startswith('Saved'),m[1]
  m,s=peek(f'run +net peek {base}/page.html',r'\[peek.got\] status=200 bytes=(\d+)')
  assert f'[net.program.bound] target={m[1]} task={m[2]}' in t()[s:],'the run was not bound to the network'
  q.cmd('screendump',{'filename':str(a.out/'peek.ppm')});time.sleep(.5);close(m)
  # The same program, not given the network, is refused.
  m,s=peek(f'run peek {base}/page.html',r'\[peek.refused\] error=NotAllowed')
  close(m)
  public={}
  for url in [u for u in a.public.split(',') if u]:
   m,s=peek(f'run +net peek {url}',r'\[peek.(got\] status=\d+ bytes=\d+|refused\] error=\w+)');public[url]=m[0] if False else t()[s:].split('[peek.')[1][:60]
   q.cmd('screendump',{'filename':str(a.out/(re.sub(r'\W','_',url)+'.ppm'))});time.sleep(.5);close(m)
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
  print({'ok':True,'public':public})
 finally:g['stop']()
if __name__=='__main__':main()
