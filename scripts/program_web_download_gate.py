#!/usr/bin/env python3
"""Downloads: opening a 20 MB attachment saves it to Downloads/ in Files as it arrives
(never whole in memory), with progress; the file read back from disk hashes the same as
what the server sent; about:downloads lists it; a second download of the same name gets
"name (2)"."""
import argparse,http.server,os,random,shutil,threading,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
DATA=random.Random(6455).randbytes(20*1024*1024)
def fnv(b):
 h=0xcbf29ce484222325
 for x in b:h=((h^x)*0x100000001b3)&0xFFFFFFFFFFFFFFFF
 return h
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  if self.path=='/web':body=(ROOT/'iso_root/programs/kaleidoscope').read_bytes();self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
  if self.path.startswith('/files/big.bin'):
   self.send_response(200);self.send_header('Content-Type','application/octet-stream');self.send_header('Content-Disposition','attachment; filename="big.bin"');self.send_header('Content-Length',str(len(DATA)));self.end_headers()
   for i in range(0,len(DATA),1<<16):self.wfile.write(DATA[i:i+(1<<16)])
   return
  if self.path=='/files/small.zip':
   body=b'PK\x03\x04small';self.send_response(200);self.send_header('Content-Type','application/zip');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
  self.send_response(404);self.end_headers()
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
server=Quiet(('127.0.0.1',0),Handler);PORT=server.server_address[1]
threading.Thread(target=server.serve_forever,daemon=True).start()
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
os.environ.setdefault('SEXOS_GATE_BOUND','1500')
from quil_autosave_gate import boot
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 g=boot(a.iso,disk,a.out,'download',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
   elif k=='?':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'slash'}],'hold-time':80})
   elif k=='+':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'equal'}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 base=f'http://10.0.2.2:{PORT}/files'
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'fetch http://10.0.2.2:{PORT}/web /bin/kaleidoscope');q.key('ret')
  m=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,240);assert m[1].startswith('Saved'),m[1]
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'run +net kaleidoscope {base}/big.bin');q.key('ret')
  g['wait'](r'\[web.download.start\] path=Downloads/big.bin ',s,300)
  g['wait'](r'\[web.download.progress\] bytes=\d+ total=20971520',s,300)
  m=g['wait'](r'\[web.download.done\] path=Downloads/big.bin bytes=(\d+) ticks=\d+ fnv=([0-9a-f]+)',s,900)
  assert int(m[1])==len(DATA) and int(m[2],16)==fnv(DATA),(m.group(0),hex(fnv(DATA)))
  g['wait'](r'\[web.loaded\] ',s,60)
  # The same name again lands beside it; a zip type is a download too.
  s=len(t());q.key('slash');time.sleep(.3);keys(f'{base}/big.bin?again');q.key('ret')
  g['wait'](r'\[web.download.done\] path=Downloads/big \(2\)\.bin bytes=20971520 ',s,900)
  s=len(t());q.key('slash');time.sleep(.3);keys(f'{base}/small.zip');q.key('ret')
  g['wait'](r'\[web.download.done\] path=Downloads/small.zip bytes=9 ',s,300)
  m=g['wait'](r'\[web.loaded\] bytes_ok=1 items=(\d+) links=(\d+)',s,60);assert int(m[2])>=3,m.group(0)
  text=t();assert 'KERNEL PANIC' not in text and '[web.fatal]' not in text and 'panicked' not in text
  print({'ok':True,'fnv':m.group(0)})
 finally:g['stop']()
if __name__=='__main__':main()
