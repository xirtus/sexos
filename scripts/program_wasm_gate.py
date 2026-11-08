#!/usr/bin/env python3
"""Wasm guests run inside the `wasm` program: the built-in guest on the native imports,
and an unmodified Rust std program (wasm32-wasip1) downloaded as a file, which prints,
reads typed lines, and exits. Close ends a guest waiting for input. Nothing panics."""
import argparse,http.server,os,shutil,threading,time
from pathlib import Path
from install_program import install
ROOT=Path(__file__).resolve().parents[1]
GREET=(ROOT/'crates/sex-wasm/tests/greet.wasm').read_bytes()
RUNNER=(ROOT/'iso_root/programs/wasm').read_bytes()
FILES={'/greet.wasm':GREET,'/wasm':RUNNER}
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
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus','@':None,':':None}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 elf=(ROOT/'iso_root/programs/wasm').read_bytes();assert elf[:4]==b'\x7fELF' and len(elf)<=16*1024*1024,len(elf)  # program_store::IMAGE_BYTES
 install(disk,[('wasm',elf)])
 g=boot(a.iso,disk,a.out,'wasm',a.resolution);t=g['text'];q=g['q']
 def shifted(k):q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':k}],'hold-time':80})
 def keys(text):
  for k in text:
   if k=='@':shifted('2')
   elif k==':':shifted('semicolon')
   elif k=='\n':q.key('ret')
   else:q.key(KEYS.get(k,k))
   time.sleep(.08)
 def line(text):g['focus'](153);time.sleep(.5);keys(text+'\n')
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  # The built-in guest: native imports, keys echoed, Enter ends it with the count typed.
  s=len(t());line('run wasm')
  m=g['wait'](r'\[wasm.start\] name=hello bytes=(\d+) args=0',s,120)
  g['wait'](r'\[wasm.guest\] hello.ready',s,60);time.sleep(1)
  keys('hi there');time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'wasm-hello.ppm')})
  keys('\n');g['wait'](r'\[wasm.exited\] code=8 shown=',s,60);time.sleep(1)
  s2=len(t());q.key('f11');g['wait'](r'\[app.private.retired\] target=\d+ task=\d+ ',s2,60);time.sleep(1)
  assert '[wasm.fatal]'not in t()[s:]
  # An ordinary Rust program, downloaded like any file and handed to the runner.
  # Runs with arguments find programs as files in /bin.
  for url,name in (('/wasm','/bin/wasm'),('/greet.wasm','/greet.wasm')):
   s=len(t());line(f'fetch http://10.0.2.2:{PORT}{url} {name}')
   m=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,240);assert m[1].startswith('Saved'),m[1];time.sleep(2)
  s=len(t());line('run wasm @/greet.wasm andreas')
  g['wait'](rf'\[wasm.start\] name=greet bytes={len(GREET)} args=1',s,120);time.sleep(3)
  keys('b a b\n');time.sleep(1);keys('\n')
  expect='Hello, andreas, from Rust on Wasm.\nType words; an empty line ends.\nb a b\n\na: 1\nb: 2\n'
  g['wait'](rf'\[wasm.exited\] code=5 shown={len(expect)}\b',s,120);time.sleep(1)
  q.cmd('screendump',{'filename':str(a.out/'wasm-greet.ppm')})
  s2=len(t());q.key('f11');g['wait'](r'\[app.private.retired\] target=\d+ task=\d+ ',s2,60);time.sleep(1)
  # Close while the Rust program waits for a line: input ends, it finishes, the run ends.
  s=len(t());line('run wasm @/greet.wasm')
  g['wait'](r'\[wasm.start\] name=greet ',s,120);time.sleep(3)
  q.key('f11');g['wait'](r'\[wasm.closed\] shown=',s,60);g['wait'](r'\[app.private.retired\] target=\d+ task=\d+ ',s,60)
  text=t();assert 'KERNEL PANIC'not in text and '[wasm.fatal]'not in text and '[wasm.trapped]'not in text and '[wasm.rejected]'not in text
  print({'ok':True,'greet_bytes':len(GREET),'runner_bytes':len(elf)})
 finally:g['stop']()
if __name__=='__main__':main()
