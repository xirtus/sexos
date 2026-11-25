#!/usr/bin/env python3
"""Valentine's native local mail view: import an RFC 5322 message through Files,
parse a multipart body, search it, reload it and preserve a screenshot in CI."""
import argparse,http.server,os,shutil,threading,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
MAIL=(b'From: =?UTF-8?Q?Ren=C3=A9e?= <renee@example.org>\r\n'
      b'Subject: Dinner on Friday\r\nDate: Tue, 24 Nov 2026 12:00:00 +0000\r\n'
      b'Content-Type: multipart/alternative; boundary=x\r\n\r\n'
      b'--x\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n'
      b'Meet at seven.\r\n--x\r\nContent-Type: text/html\r\n\r\n'
      b'<img src="https://tracker.example/p.gif">\r\n--x--\r\n')
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  if self.path!='/inbox.eml':self.send_response(404);self.end_headers();return
  self.send_response(200);self.send_header('Content-Length',str(len(MAIL)));self.end_headers();self.wfile.write(MAIL)
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
server=Quiet(('127.0.0.1',0),Handler);PORT=server.server_address[1]
threading.Thread(target=server.serve_forever,daemon=True).start()
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
from install_program import install
from quil_autosave_gate import boot
from valentine_key_input import send_text

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 elf=(ROOT/'iso_root/programs/valentine').read_bytes();assert elf[:4]==b'\x7fELF'
 install(disk,[('valentine',elf)])
 g=boot(a.iso,disk,a.out,'valentine',a.resolution);t=g['text'];q=g['q']
 def keys(value):
  send_text(q,value)
 def ctrl(key):q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':key}],'hold-time':80})
 def shell(line,pattern,timeout=120):
  g['focus'](153);time.sleep(.5);s=len(t());keys(line);q.key('ret');return g['wait'](pattern,s,timeout)
 def launch():
  s=len(t());shell('run valentine',r'\[shell.program.launch\]')
  ready=g['wait'](r'\[valentine.ready\] target=(\d+) task=(\d+)',s,90)
  display=g['wait'](rf'\[shell.program.launch\] target={ready[1]} task={ready[2]} surface=(\d+)',s,60)
  g['focus'](int(display[1]));time.sleep(.7)
  return ready[1],ready[2],s
 def close(target,task):
  s=len(t());q.key('f11');g['wait'](rf'\[valentine.closed\] target={target} task={task}',s,60)
  g['wait'](rf'\[app.private.retired\] target={target} task={task} ',s,90)
 def shot(name):time.sleep(.8);q.cmd('screendump',{'filename':str(a.out/f'{name}.ppm')});time.sleep(.4)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(1)
  target,task,s=launch();g['wait'](r'\[valentine.loaded\] messages=0',s,60);close(target,task)
  m=shell(f'fetch http://10.0.2.2:{PORT}/inbox.eml /data/valentine/inbox.eml',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',240)
  assert m[1].startswith('Saved'),m[1]
  target,task,s=launch()
  g['wait'](r'\[valentine.opened\] file=inbox.eml bytes=\d+ subject=Dinner on Friday',s,60)
  g['wait'](r'\[valentine.loaded\] messages=1',s,60)
  shot('valentine-inbox')
  pixels=(a.out/'valentine-inbox.ppm').read_bytes();assert pixels.startswith(b'P6\n') and len(pixels)>100000
  s=len(t());keys('dinner');g['wait'](r'\[valentine.search\] query=dinner matches=1',s,60);shot('valentine-search')
  s=len(t());q.key('esc');g['wait'](r'\[valentine.search\] query= matches=1',s,60)
  s=len(t());q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':'r'}],'hold-time':80})
  g['wait'](r'\[valentine.loaded\] messages=1',s,60)
  s=len(t());ctrl('n');keys('bob@example.org');q.key('ret');keys('plans');q.key('ret');keys('see you at seven.')
  ctrl('s');g['wait'](r'\[valentine.draft.saved\] path=draft-[^ ]+\.eml bytes=\d+',s,60)
  shot('valentine-draft')
  close(target,task)
  target,task,s=launch();g['wait'](r'\[valentine.loaded\] messages=2',s,60)
  keys('plans');g['wait'](r'\[valentine.search\] query=plans matches=1',s,60)
  s=len(t());q.key('ret');g['wait'](r'\[valentine.opened\] file=draft-[^ ]+\.eml bytes=\d+ subject=plans',s,60)
  s=len(t());ctrl('e');time.sleep(.4);shot('valentine-draft-reopened');q.key('esc')
  close(target,task)
  assert '[valentine.fatal]' not in t() and 'KERNEL PANIC' not in t()
 finally:g['stop']();server.shutdown()
 print({'ok':True,'messages':1})
if __name__=='__main__':main()
