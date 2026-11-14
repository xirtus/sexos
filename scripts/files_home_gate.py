#!/usr/bin/env python3
"""The encrypted home: unlocked with its password, unreadable without it.

Boot 1: Files (holding the whole filesystem) encrypts /home with Ctrl+K and a password
typed twice. A file arrives in /home over the network (so its text is never typed and
never lands in Spindle's history), reads back, then Ctrl+K locks home and the same read
answers LOCKED. On the host the disk image holds no byte of that file's text, and
`sexfs-fsck --deep` finds the system disk clean.

Boot 2: Files starts and asks for the password. A wrong one is refused and the file
stays unreadable; the right one unlocks home and the file reads back. Ctrl+Shift+K
changes the password (old one, new one twice), home is locked, the old password is
refused and the new one opens it."""
import argparse,os,random,shutil,subprocess,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus',':':'semicolon','_':'minus'}
MARKER=b'SEXOS-HOME-SECRET-'+random.Random(9).randbytes(8).hex().encode()
SECRET=(MARKER+b'\n')*2000
PASSWORD='correct horse 42'
NEWPASS='battery staple 7'
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 shutil.rmtree(a.out,ignore_errors=True);a.out.mkdir(parents=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 subprocess.run(['cargo','build','-q','--release','--manifest-path',str(ROOT/'tools/sexfs-fsck/Cargo.toml')],check=True)
 fsck=ROOT/'tools/sexfs-fsck/target/release/sexfs-fsck'
 import http.server,threading
 class Handler(http.server.BaseHTTPRequestHandler):
  def do_GET(self):
   body={'/secret':SECRET,'/files':(ROOT/'iso_root/programs/loom').read_bytes()}.get(self.path)
   if body is None:self.send_response(404);self.end_headers();return
   self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
  def log_message(self,*x):pass
 class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
 server=Quiet(('127.0.0.1',0),Handler);PORT=server.server_address[1];threading.Thread(target=server.serve_forever,daemon=True).start()
 os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
 os.environ.pop('SEXOS_GATE_DEVICES',None);os.environ.setdefault('SEXOS_GATE_BOUND','900')
 from quil_autosave_gate import boot
 result={}
 def session(tag,body):
  g=boot(a.iso,disk,a.out,tag,a.resolution);t=g['text'];q=g['q']
  def keys(text):
   for k in text:
    shift=k.isupper() or k in ':_'
    code=KEYS.get(k,k.lower())
    if shift:q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':code}],'hold-time':80})
    else:q.key(code)
    time.sleep(.06)
  def line(text):g['focus'](153);time.sleep(.5);keys(text);q.key('ret')
  def expect(text,pattern,timeout=60):s=len(t());line(text);return g['wait'](pattern,s,timeout)
  def ctrl(k,shift=False):q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'}]+([{'type':'qcode','data':'shift'}] if shift else [])+[{'type':'qcode','data':k}],'hold-time':80})
  def files():
   s=len(t());line('files');m=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,60)
   g['wait'](r'\[files.app.ready\] ',s,60);g['wait'](rf'\[display.text.committed\] target={m[1]} ',s,60);time.sleep(1)
   return int(m[3]),s
  def typed(surface,text):g['focus'](surface);time.sleep(.5);keys(text);q.key('ret')
  try:
   g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
   body(g,t,q,expect,ctrl,files,typed)
   assert 'KERNEL PANIC' not in t() and 'panicked' not in t() and '[files.app.fatal]' not in t()
  finally:g['stop']()
 def first(g,t,q,expect,ctrl,files,typed):
  m=expect(f'fetch http://10.0.2.2:{PORT}/files /bin/loom',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',240);assert m[1].startswith('Saved'),m[1]
  surface,_=files()
  s=len(t());g['focus'](surface);time.sleep(.5);ctrl('k')
  g['wait'](r'\[display.text.committed\] ',s,30);time.sleep(.5)
  typed(surface,PASSWORD);time.sleep(.5);typed(surface,PASSWORD)
  m=g['wait'](r'\[files.home.setup\] mib=(\d+) kdf_ms=(\d+) ',s,300);result['setup_kdf_ms']=int(m[2])
  g['wait'](r'\[files.app.home.setup\] ',s,60)
  m=expect(f'fetch http://10.0.2.2:{PORT}/secret /home/secret.txt',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',240);assert m[1].startswith('Saved'),m[1]
  expect('cat /home/secret.txt',r'\[spindle.fs.cat\] '+MARKER.decode())
  s=len(t());g['focus'](surface);time.sleep(.5);ctrl('k');g['wait'](r'\[files.home.locked\]',s,60)
  expect('cat /home/secret.txt',r'\[spindle.fs.result\] kind=7 status=17 ')
  time.sleep(4)
 session('home1',first)
 raw=disk.read_bytes()
 assert MARKER not in raw and MARKER[18:] not in raw,'the home file\'s text is on the disk in the clear'
 assert raw.count(b'SEXCRYPT')>=1,'no encrypted home header on the disk'
 r=subprocess.run([str(fsck),'--deep',str(disk)],capture_output=True,text=True);print(r.stdout.strip())
 assert r.returncode==0,r.stdout+r.stderr
 def second(g,t,q,expect,ctrl,files,typed):
  surface,s=files()
  g['wait'](r'\[files.app.home\] state=locked',s,30)
  typed(surface,'wrong horse 42')
  m=g['wait'](r'\[files.home.unlock.refused\] why=WrongPassword ms=(\d+)',s,300);result['wrong_ms']=int(m[1])
  expect('cat /home/secret.txt',r'\[spindle.fs.result\] kind=7 status=17 ')
  s=len(t());typed(surface,PASSWORD)
  m=g['wait'](r'\[files.home.unlocked\] ms=(\d+) ',s,300);result['unlock_ms']=int(m[1])
  g['wait'](r'\[files.app.home.unlocked\]',s,30)
  expect('cat /home/secret.txt',r'\[spindle.fs.cat\] '+MARKER.decode())
  s=len(t());g['focus'](surface);time.sleep(.5);ctrl('k',True);g['wait'](r'\[display.text.committed\] ',s,30);time.sleep(.5)
  typed(surface,PASSWORD);time.sleep(.5);typed(surface,NEWPASS);time.sleep(.5);typed(surface,NEWPASS)
  m=g['wait'](r'\[files.home.changed\] ms=(\d+)',s,300);result['change_ms']=int(m[1])
  g['wait'](r'\[files.app.home.changed\]',s,30)
  s=len(t());g['focus'](surface);time.sleep(.5);ctrl('k');g['wait'](r'\[files.home.locked\]',s,60)
  s=len(t());g['focus'](surface);time.sleep(.5);ctrl('k');g['wait'](r'\[display.text.committed\] ',s,30);time.sleep(.5)
  typed(surface,PASSWORD);g['wait'](r'\[files.home.unlock.refused\] why=WrongPassword',s,300)
  s=len(t());typed(surface,NEWPASS);g['wait'](r'\[files.home.unlocked\] ',s,300)
  expect('cat /home/secret.txt',r'\[spindle.fs.cat\] '+MARKER.decode())
 session('home2',second)
 result['ok']=True;print(result)
if __name__=='__main__':main()
