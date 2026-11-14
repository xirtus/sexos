#!/usr/bin/env python3
"""Jot keeps unsaved work through a crash, and keeps how a file is stored.

* A Windows file (CRLF line ends) is read as CRLF; a line typed and saved goes back to
  disk with CRLF, byte for byte (FNV-1a hash).
* Ctrl+M shows a Markdown file as a preview (headings, lists), Ctrl+R turns soft wrap on.
* Text typed and never saved is written to Jot's session a moment later. The machine is
  then killed mid-run, as in a crash or power cut. After a fresh boot, Jot opens the same
  file with the unsaved text back, and saving it puts those bytes on disk."""
import argparse,http.server,os,re,shutil,threading,time
from pathlib import Path
from install_program import install,fnv
ROOT=Path(__file__).resolve().parents[1]
CRLF=b'first line\r\nsecond line\r\n'
MD=b'# Shopping\n\nThings to **buy** this week:\n\n- milk\n- [ ] eggs\n\n> fresh ones\n'
FILES={'/crlf.txt':CRLF,'/list.md':MD}
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
KEYS={' ':'spc','.':'dot','/':'slash','-':'minus',':':'semicolon','\n':'ret'}
TYPED='bread too'
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 jot=(ROOT/'iso_root/programs/jot').read_bytes();assert jot[:4]==b'\x7fELF'
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 install(disk,[('jot',jot)])
 def session(tag):
  g=boot(a.iso,disk,a.out,tag,a.resolution);t=g['text'];q=g['q']
  def keys(text):
   for k in text:
    if k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
    else:q.key(KEYS.get(k,k))
    time.sleep(.05)
  def chord(*ks):q.cmd('send-key',{'keys':[{'type':'qcode','data':k} for k in ks],'hold-time':80});time.sleep(.15)
  def spindle(line):g['focus'](153);time.sleep(.5);keys(line);q.key('ret')
  def start(line):
   s=len(t());spindle(line)
   m=g['wait'](r'\[shell\.program\.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,240)
   g['wait'](r'\[jot\.ready\] target=\d+ task=\d+ tabs=\d+',s,120);time.sleep(1.5)
   g['focus'](int(m[3]));time.sleep(.5)
   return int(m[1]),int(m[2]),s
  def close(target,task):
   s=len(t());q.key('f11');g['wait'](rf'\[app\.private\.retired\] target={target} task={task} ',s,120)
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  return g,t,q,keys,chord,spindle,start,close
 g,t,q,keys,chord,spindle,start,close=session('jotrestore1')
 try:
  for name in FILES:
   s=len(t());spindle(f'fetch http://10.0.2.2:{PORT}{name} {name[1:]}')
   m=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,240);assert m[1].startswith('Saved'),m[1]
  # CRLF in, CRLF out.
  target,task,s=start('edit crlf.txt')
  g['wait'](r'\[jot\.format\.read\] encoding=UTF-8 eol=CRLF\n',s,60)
  chord('ctrl','end');keys('third line\n')
  s=len(t());chord('ctrl','s');want=CRLF+b'third line\r\n'
  m=g['wait'](r'\[jot\.saved\] bytes=(\d+) fnv=([0-9a-f]+)\n',s,60);assert (int(m[1]),int(m[2],16))==(len(want),fnv(want)),(m[0],len(want),hex(fnv(want)))
  close(target,task)
  # Markdown preview and soft wrap; then text typed and never saved.
  target,task,s=start('edit list.md')
  s=len(t());chord('ctrl','m');m=g['wait'](r'\[jot\.preview\] blocks=(\d+) rows=(\d+) headings=(\d+)\n',s,60);assert (m[1],m[3])==('5','1'),m[0]
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'jot-preview.ppm')});time.sleep(.3)
  q.key('esc');s=len(t());chord('ctrl','r');g['wait'](r'\[jot\.wrap\] on=1 rows=\d+\n',s,60)
  s=len(t());chord('ctrl','end');keys(TYPED)
  g['wait'](r'\[jot\.session\.saved\] tabs=1 bytes=\d+\n',s,60)
  # A change made by find and replace is kept too, not only typing.
  time.sleep(1.5);s=len(t());chord('ctrl','h');keys('too');q.key('ret');keys('also');chord('ctrl','a')
  g['wait'](r'\[jot\.replaced\] n=1\n',s,60);q.key('esc')
  g['wait'](r'\[jot\.session\.saved\] tabs=1 bytes=\d+\n',s,60)
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'jot-unsaved.ppm')});time.sleep(.3)
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t() and '[jot.fatal]' not in t()
 finally:g['stop']()  # Killed, not closed: Jot gets no chance to save anything more.
 restored=MD+TYPED.replace('too','also').encode()
 g,t,q,keys,chord,spindle,start,close=session('jotrestore2')
 try:
  target,task,s=start('edit list.md')
  g['wait'](rf'\[jot\.loaded\] bytes={len(MD)} fnv={fnv(MD):016x}\n',s,60)
  m=g['wait'](r'\[jot\.restored\] path=(\S*) bytes=(\d+) fnv=([0-9a-f]+)\n',s,60)
  assert m[1].endswith('list.md') and (int(m[2]),int(m[3],16))==(len(restored),fnv(restored)),m[0]
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'jot-restored.ppm')});time.sleep(.3)
  s=len(t());chord('ctrl','s');m=g['wait'](r'\[jot\.saved\] bytes=(\d+) fnv=([0-9a-f]+)\n',s,60)
  assert (int(m[1]),int(m[2],16))==(len(restored),fnv(restored)),m[0]
  g['wait'](r'\[jot\.session\.saved\] tabs=0 ',s,60)
  close(target,task)
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t() and '[jot.fatal]' not in t()
 finally:g['stop']()
 print({'ok':True,'restored_bytes':len(restored),'fnv':f'{fnv(restored):016x}'})
if __name__=='__main__':main()
