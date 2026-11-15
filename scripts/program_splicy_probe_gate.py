#!/usr/bin/env python3
"""FilmCraft's editing engine on SexOS, headless (apps/splicy-probe).

`run splicy-probe 2` opens FilmCraft's demo project, selects everything, ripple-deletes,
undoes, then exports two seconds as WAV and as H.264 MP4 with FilmCraft's own encoders.
Every step must print `[splicy.probe.ok]`, both exports must write real files, and the
run must end with `[splicy.probe.done]`. This is the engine Splicy's FilmCraft UI will
drive. Build it first: scripts/build_splicy_probe.sh."""
import argparse,http.server,os,shutil,threading,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
PROBE=os.environ.get('SEXOS_SPLICY_PROBE',str(ROOT/'target/x86_64-sex-std/release/splicy-probe'))
FILES={'/splicy-probe':Path(PROBE).read_bytes()}
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
os.environ.setdefault('SEXOS_GATE_BOUND','2400')
from quil_autosave_gate import boot
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 g=boot(a.iso,disk,a.out,'splicy-probe',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 def expect(text,pattern,timeout=120):
  g['focus'](153);time.sleep(.5);s=len(t());keys(text);q.key('ret');return g['wait'](pattern,s,timeout)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  m=expect(f'fetch http://10.0.2.2:{PORT}/splicy-probe /bin/splicy-probe',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',900);assert m[1].startswith('Saved'),m[1]
  s=len(t())
  expect('run splicy-probe 2',r'\[splicy.probe.(ok\] file.openDemoProject|failed\][^\n]*)',300)
  done=g['wait'](r'\[splicy.probe.(done\]|failed\][^\n]*)',s,1800);assert done[1]=='done]',done[0]
  text=t()[s:]
  for step in('edit.selectAll','edit.rippleDelete','edit.undo','edit.deselectAll'):assert f'[splicy.probe.ok] {step} ' in text,step
  import re
  ex={m[1]:int(m[2]) for m in re.finditer(r'\[splicy.probe.export\] format=(\w+) file=\S+ bytes=(\d+)',text)}
  assert ex.get('wav',0)>=300_000,ex;assert ex.get('h264',0)>=10_000,ex
  time.sleep(1.5);q.cmd('screendump',{'filename':str(a.out/'splicy-probe.ppm')})
  assert 'KERNEL PANIC' not in text and 'KERNEL PAGE FAULT' not in text
  print({'ok':True,'exports':ex})
 finally:g['stop']()
if __name__=='__main__':main()
