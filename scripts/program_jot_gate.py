#!/usr/bin/env python3
"""A program built on sex-app: installed as a file, typed into, saving in its own folder, refused outside it, and finding its note again after a reboot."""
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
NOTE='hello from jot'
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 def session(tag):
  g=boot(a.iso,disk,a.out,tag,a.resolution);t=g['text'];q=g['q']
  def keys(text):
   for k in text:
    if k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
    else:q.key(KEYS.get(k,k))
    time.sleep(.06)
  def line(text):
   g['focus'](153);time.sleep(.5);keys(text);q.key('ret')
  def expect(text,pattern,timeout=60):
   s=len(t());line(text);return g['wait'](pattern,s,timeout)
  def chord(key):q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':key}],'hold-time':80})
  def jot():
   s=len(t());line('run jot');m=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,60)
   loaded=g['wait'](r'\[jot.loaded\] bytes=(\d+)',s,60);g['wait'](r'\[jot.ready\]',s,60)
   g['wait'](rf'\[files.program.bound\] target={m[1]} task={m[2]} root=/data/jot',s,60)
   g['wait'](rf'\[display.text.committed\] target={m[1]} ',s,60);g['wait'](rf'\[display.app.title\] target={m[1]} surface={m[3]}',s,60);time.sleep(2)
   return int(m[1]),int(m[2]),int(m[3]),int(loaded[1])
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  return g,t,q,keys,expect,chord,jot
 g,t,q,keys,expect,chord,jot=session('jot1')
 try:
  m=expect(f'fetch http://10.0.2.2:{PORT}/jot /bin/jot',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',240);assert m[1].startswith('Saved'),m[1]
  target,task,sid,loaded=jot();assert loaded==0,loaded
  g['focus'](sid);time.sleep(.5)
  s=len(t());keys(NOTE);time.sleep(1);chord('s');g['wait'](rf'\[jot.saved\] bytes={len(NOTE)}\b',s,60)
  s=len(t());chord('e');g['wait'](r'\[jot.escape\] refused=1 status=7\b',s,60)
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'jot.ppm')});time.sleep(.5)
  s=len(t());q.key('f11');g['wait'](rf'\[jot.closed\] target={target} task={task}',s,60);g['wait'](rf'\[app.private.retired\] target={target} task={task} ',s,60)
  # Spindle sees the note where Files put it.
  expect('cat /data/jot/notes.txt',r'\[spindle.fs.cat\] '+re.escape(NOTE))
  # Given one file by name, Jot edits that file and nothing else of the person's.
  expect('write todo.txt buy milk',r'\[spindle.fs.result\] kind=8 status=0')
  s=len(t());expect('edit todo.txt',r'\[jot.ready\]');m=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,60)
  g['wait'](r'\[jot.loaded\] bytes=9\b',s,60);g['wait'](rf'\[display.text.committed\] target={m[1]} ',s,60);time.sleep(1)
  g['focus'](int(m[3]));time.sleep(.5);s=len(t());q.key('backspace')
  for _ in range(4):q.key('left');time.sleep(.06)
  keys('fresh ');q.key('end');keys(' and eggs');time.sleep(.5);chord('s');g['wait'](r'\[jot.saved\] bytes=23\b',s,60)
  s=len(t());q.key('f11');g['wait'](rf'\[app.private.retired\] target={m[1]} task={m[2]} ',s,90)
  expect('cat todo.txt',r'\[spindle.fs.cat\] buy fresh milk and eggs')
  # A run Shell never bound, such as the boot image's own program, has no files.
  assert '[files.program.bound]' not in t().split('[app.private.retired] target=41 ')[0]
  assert 'KERNEL PANIC'not in t() and 'panicked' not in t()
 finally:g['stop']()
 g,t,q,keys,expect,chord,jot=session('jot2')
 try:
  target,task,sid,loaded=jot();assert loaded==len(NOTE),loaded
  assert 'KERNEL PANIC'not in t() and 'panicked' not in t()
 finally:g['stop']()
 print({'ok':True,'note':NOTE,'first_task':task})
if __name__=='__main__':main()
