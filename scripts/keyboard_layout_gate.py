#!/usr/bin/env python3
"""Keyboard layouts end to end: Super+Ctrl+Space switches US -> DE (the panel names it),
then in Jot the German keys type ü, a dead ´ then e types é, AltGr+Q types @, and
Compose (Menu) o " types ö. The saved note holds exactly those characters as UTF-8.
Same harness as program_jot_gate.py (Jot fetched over HTTP, then run)."""
import argparse,http.server,os,re,shutil,threading,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
FILES={'/jot':(ROOT/'iso_root/programs/jot').read_bytes()}
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
def qk(*names):return [{'type':'qcode','data':n} for n in names]
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 g=boot(a.iso,disk,a.out,'keyboard',a.resolution);t=g['text'];q=g['q']
 def chord(*names):q.cmd('send-key',{'keys':qk(*names),'hold-time':80});time.sleep(.15)
 def keys(text):
  for k in text:
   if k==':':chord('shift','semicolon')
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 def line(text):g['focus'](153);time.sleep(.5);keys(text);q.key('ret')
 def expect(text,pattern,timeout=60):s=len(t());line(text);return g['wait'](pattern,s,timeout)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  m=expect(f'fetch http://10.0.2.2:{PORT}/jot /bin/jot',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',240);assert m[1].startswith('Saved'),m[1]
  s=len(t());line('run jot');m=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,60)
  g['wait'](r'\[jot.ready\]',s,60);g['wait'](rf'\[display.text.committed\] target={m[1]} ',s,60);time.sleep(2)
  g['focus'](int(m[3]));time.sleep(.5)
  s=len(t());chord('meta_l','ctrl','spc');g['wait'](r'\[shell.keyboard.layout\] label=DE title=German',s,30)
  g['focus'](int(m[3]));time.sleep(.5)
  q.key('bracket_left')            # ü
  q.key('equal');q.key('e')        # dead ´ then e: é
  chord('alt_r','q')               # AltGr+Q: @
  q.key('y')                       # the German z
  chord('menu');q.key('o');chord('shift','2')   # Compose o " : ö
  time.sleep(1);s=len(t());chord('ctrl','s');want='üé@zö'
  g['wait'](rf'\[jot.saved\] bytes={len(want.encode())}\b',s,60)
  q.cmd('screendump',{'filename':str(a.out/'keyboard-de.ppm')});time.sleep(.5)
  s=len(t());q.key('f11');g['wait'](rf'\[jot.closed\] target={m[1]} task={m[2]}',s,60)
  expect('cat /data/jot/notes.txt',r'\[spindle.fs.cat\] '+re.escape(want))
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
 finally:g['stop']()
 print({'ok':True,'typed':want})
if __name__=='__main__':main()
