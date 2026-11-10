#!/usr/bin/env python3
"""Programs are ordinary files: downloaded into the filesystem, run by name or by path, refused with a plain answer, and still there after a reboot."""
import argparse,http.server,os,re,shutil,threading,time
from pathlib import Path
# A 16 MiB file over the network takes a while under emulation.
os.environ.setdefault('SEXOS_GATE_BOUND','900')
ROOT=Path(__file__).resolve().parents[1]
# One page past the largest program the store takes (sex_pdx::program_store::IMAGE_BYTES).
IMAGE_BYTES=__import__('math').prod(int(n) for n in re.search(r'pub const IMAGE_BYTES: usize = ([0-9* ]+);',(ROOT/'crates/sex-pdx/src/program_store.rs').read_text())[1].split('*'))
FILES={'/fortune':(ROOT/'iso_root/programs/fortune').read_bytes(),'/hum':(ROOT/'iso_root/programs/hum').read_bytes(),'/big':bytes(IMAGE_BYTES+4096)}
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
 def session(tag):
  g=boot(a.iso,disk,a.out,tag,a.resolution);t=g['text'];q=g['q']
  def line(text):
   g['focus'](153);time.sleep(.5)
   for k in text:
    if k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
    else:q.key(KEYS.get(k,k))
    time.sleep(.05)
   q.key('ret')
  def expect(text,pattern,timeout=60):
   s=len(t());line(text);return g['wait'](pattern,s,timeout)
  def started(text):
   s=len(t());line(text);m=g['wait'](r'\[fortune.ready\] target=(\d+) task=(\d+) surface=(\d+) bytes=',s,60)
   g['wait'](rf'\[display.text.committed\] target={m[1]} ',s,60);time.sleep(2)
   s=len(t());q.key('f11');g['wait'](rf'\[fortune.closed\] target={m[1]} task={m[2]}',s,60);g['wait'](rf'\[app.private.retired\] target={m[1]} task={m[2]} ',s,60);time.sleep(1)
   return int(m[2])
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  return g,t,expect,started
 g,t,expect,started=session('files1')
 try:
  base=f'http://10.0.2.2:{PORT}'
  def fetch(url,name,timeout=240):
   m=expect(f'fetch {base}{url} {name}',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',timeout)
   assert m[1].startswith('Saved'),m[1]
  # A new disk starts with the program directory, and nothing in it yet.
  expect('run fortune',r'\[spindle.program.refused\] code=1\b')
  assert '[files.fs.formatted]' in t(),'no new filesystem'
  fetch('/fortune','/bin/fortune')
  first=started('run fortune');assert '[files.program.file] bytes='+str(len(FILES['/fortune'])) in t()
  # By path, absolute and relative.
  expect('mkdir tools',r'\[spindle.fs.line\] ok')
  fetch('/fortune','tools/lucky');absolute=started('run /tools/lucky')
  expect('cd tools',r'\[spindle.fs.result\] kind=1 status=0');relative=started('run ./lucky');expect('cd /',r'\[spindle.fs.result\] kind=1 status=0')
  assert len({first,absolute,relative})==3,'a run reused an old task identity'
  # Refusals name the reason, and nothing starts.
  expect('run /nope',r'\[spindle.fs.line\] not found')
  expect('run /bin',r'\[spindle.fs.line\] is a directory')
  expect('write /bin/notes this is not a program',r'\[spindle.fs.result\] kind=8 status=0')
  expect('run notes',r'\[spindle.program.refused\] code=5\b')
  expect('run /bin/notes',r'\[spindle.program.refused\] code=5\b')
  expect('run nosuch',r'\[spindle.program.refused\] code=1\b')
  fetch('/big','/big',900);expect('run /big',r'\[spindle.program.refused\] code=2\b')
  expect('touch /empty',r'\[spindle.fs.line\] ok');expect('run /empty',r'\[spindle.program.refused\] code=5\b')
  # Background programs come from files too.
  fetch('/hum','/bin/hum');expect('spawn hum 10',r'\[shell.records.spawned\] live=10\b',120)
  def stopped():
   expect('stopall force',r'\[spindle.fs.result\]|Asked|$',5);time.sleep(5);expect('records',r'\[records.census\] live=0 ',60)
  stopped()
  fetch('/hum','/tools/hum2');expect('spawn /tools/hum2 5',r'\[shell.records.spawned\] live=5\b',120);stopped()
  # More than memory allows stops cleanly and says so.
  expect('spawn hum 4000',r'\[spindle.program.refused\] code=7\b',120);stopped()
  assert 'KERNEL PANIC'not in t() and 'panicked' not in t()
 finally:g['stop']()
 # They are still programs after a reboot.
 g,t,expect,started=session('files2')
 try:
  assert '[files.fs.formatted]'not in t()
  again=started('run fortune');path=started('run /tools/lucky')
  assert 'KERNEL PANIC'not in t() and 'panicked' not in t()
 finally:g['stop']()
 print({'ok':True,'tasks':[first,absolute,relative,again,path]})
if __name__=='__main__':main()
