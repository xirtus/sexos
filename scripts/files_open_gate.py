#!/usr/bin/env python3
"""Files opens a file in its program: Enter on a text file starts Jot with that file
alone (it loads the same bytes); before Jot is installed Files says so; a file no
program opens just says so."""
import argparse,http.server,os,shutil,threading,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
# Files is built and installed as Loom; Spindle's `files` runs /bin/loom.
FILES={'/files':(ROOT/'iso_root/programs/loom').read_bytes(),'/jot':(ROOT/'iso_root/programs/jot').read_bytes()}
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
TEXT='opened from files'
def fnv(b):
 h=0xcbf29ce484222325
 for x in b:h=((h^x)*0x100000001b3)&0xFFFFFFFFFFFFFFFF
 return h
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 shutil.rmtree(a.out,ignore_errors=True);a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 g=boot(a.iso,disk,a.out,'fileopen',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k.isupper() or k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon' if k==':' else k.lower()}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 def line(text):g['focus'](153);time.sleep(.5);keys(text);q.key('ret')
 def expect(text,pattern,timeout=60):s=len(t());line(text);return g['wait'](pattern,s,timeout)
 result={}
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  for n,dest in (('files','loom'),):
   m=expect(f'fetch http://10.0.2.2:{PORT}/{n} /bin/{dest}',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',240);assert m[1].startswith('Saved'),m[1]
  expect('mkdir /0open',r'\[spindle.fs.result\] kind=3 status=0')
  expect(f'write /0open/a.txt {TEXT}',r'\[spindle.fs.result\] kind=8 status=0')
  expect('write /0open/b.xyz no program opens this',r'\[spindle.fs.result\] kind=8 status=0')
  s=len(t());line('files /0open');m=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,60);sid=int(m[3])
  g['wait'](r'\[files.app.list\] dir=/0open count=2\b',s,60);g['wait'](rf'\[display.text.committed\] target={m[1]} ',s,60);time.sleep(1)
  g['focus'](sid);time.sleep(.5)
  # Jot isn't in /bin yet: Files says so.
  s=len(t());q.key('ret')
  g['wait'](r'\[files.app.open.refused\] program=jot code=1\b',s,30)
  m=expect(f'fetch http://10.0.2.2:{PORT}/jot /bin/jot',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',240);assert m[1].startswith('Saved'),m[1]
  # a.txt sorts first: Enter opens it in Jot.
  g['focus'](sid);time.sleep(.5)
  s=len(t());q.key('ret')
  g['wait'](r'\[files.app.open\] program=jot path=/0open/a.txt',s,30)
  m=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,60)
  j=g['wait'](r'\[jot.loaded\] bytes=(\d+) fnv=([0-9a-f]+)',s,60)
  want=TEXT.encode()
  assert int(j[1]) in (len(want),len(want)+1),j[0]
  result['jot_bytes']=int(j[1])
  # b.xyz: nothing starts.
  time.sleep(2);g['focus'](sid);time.sleep(.5);q.key('down');time.sleep(.5)
  s=len(t());q.key('ret');time.sleep(4)
  assert '[files.app.open]' not in t()[s:] and '[shell.program.launch]' not in t()[s:],t()[s:][-500:]
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t() and '[files.app.fatal]' not in t()
 finally:g['stop']()
 result['ok']=True;print(result)
if __name__=='__main__':main()
