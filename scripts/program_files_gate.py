#!/usr/bin/env python3
"""USB sticks and other systems' disks in Files: a FAT stick image and a Linux ext4 image
mounted under /media from Spindle, read and written there; the Files app copying from the
stick to the SexOS disk, moving to the trash and undoing; and Jot asking for a file through
the open picker, which Files answers with that one file."""
import argparse,http.server,os,re,shutil,struct,threading,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
def unsparse(name):
 raw=(ROOT/'tests/volumes'/f'{name}.sparse').read_bytes();assert raw[:8]==b'SXSPARSE'
 out=bytearray(struct.unpack('<Q',raw[8:16])[0]*512);at=16
 while at<len(raw):
  lba=struct.unpack('<Q',raw[at:at+8])[0];out[lba*512:lba*512+512]=raw[at+8:at+520];at+=520
 return bytes(out)
FILES={'/jot':(ROOT/'iso_root/programs/jot').read_bytes(),'/files':(ROOT/'iso_root/programs/files').read_bytes(),
       '/stick.img':unsparse('fat12'),'/linux.img':unsparse('ext2')}
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
 g=boot(a.iso,disk,a.out,'files1',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   shift=k.isupper() or k in ':_'
   code={':':'semicolon','_':'minus'}.get(k,KEYS.get(k,k.lower()))
   if shift:q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':code}],'hold-time':80})
   else:q.key(code)
   time.sleep(.06)
 def line(text):g['focus'](153);time.sleep(.5);keys(text);q.key('ret')
 def expect(text,pattern,timeout=60):s=len(t());line(text);return g['wait'](pattern,s,timeout)
 def chord(key,*mods):q.cmd('send-key',{'keys':[{'type':'qcode','data':m} for m in ('ctrl',)+mods]+[{'type':'qcode','data':key}],'hold-time':80})
 def launched(s):return g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,60)
 def step(action,pattern,timeout=30):s=len(t());action();return g['wait'](pattern,s,timeout)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  for name,dest in [('jot','/bin/jot'),('files','/bin/files'),('stick.img','stick.img'),('linux.img','linux.img')]:
   m=expect(f'fetch http://10.0.2.2:{PORT}/{name} {dest}',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',240);assert m[1].startswith('Saved'),m[1]
  # The stick: FAT12, read and written from Spindle.
  expect('mount stick.img',r'\[files.media.mounted\] name=FLOPPY format=FAT12 ')
  g['wait'](r'\[spindle.fs.line\] /media/FLOPPY \(FAT12',0,30)
  s=len(t());line('ls /media/FLOPPY');g['wait'](r'\[spindle.fs.result\] kind=2 ',s,60);time.sleep(1)
  listed=re.findall(r'\[spindle.fs.line\] ([^\n]*)',t()[s:]);assert 'HELLO.TXT  17' in listed and 'Docs/' in listed,listed
  expect('cat /media/FLOPPY/hello.txt',r'\[spindle.fs.cat\] hello from fat12')
  expect('write /media/FLOPPY/from-sexos.txt written by sexos',r'\[spindle.fs.result\] kind=8 status=0')
  expect('cat /media/FLOPPY/from-sexos.txt',r'\[spindle.fs.cat\] written by sexos')
  expect('df /media/FLOPPY',r'\[spindle.fs.line\] 0 MiB free of 0 MiB')
  # A Linux disk: ext2, read only.
  expect('mount linux.img',r'\[files.media.mounted\] name=(\S+) format=ext4 [^\n]*read_only=1')
  linux=re.search(r'\[files.media.mounted\] name=(\S+) format=ext4',t())[1]
  s=len(t());line(f'ls /media/{linux}');g['wait'](r'\[spindle.fs.result\] kind=2 status=0',s,60)
  expect(f'write /media/{linux}/nope.txt x',r'\[spindle.fs.line\] read-only disk')
  expect('mv /media/FLOPPY/from-sexos.txt from-stick.txt',r'\[spindle.fs.line\] on another disk')
  # The Files app, given everything.
  s=len(t());line('files');m=launched(s);fa_target,fa_task,fa_sid=m[1],m[2],int(m[3])
  g['wait'](r'\[files.app.ready\] target=\d+ task=\d+ root=/ portal=1',s,60);g['wait'](rf'\[display.text.committed\] target={fa_target} ',s,60);time.sleep(1)
  g['focus'](fa_sid);time.sleep(.5)
  step(lambda:chord('d'),r'\[files.app.list\] dir=/media count=2')
  step(lambda:q.key('ret'),r'\[files.app.list\] dir=/media/FLOPPY count=5')
  # Docs/, Long Folder Name/, from-sexos.txt, HELLO.TXT, lower.txt: the fourth.
  for _ in range(3):q.key('down');time.sleep(.1)
  chord('c');time.sleep(.5)
  step(lambda:chord('h'),r'\[files.app.list\] dir=/ count=')
  step(lambda:chord('v'),r'\[files.app.pasted\] from=/media/FLOPPY/HELLO.TXT to=/HELLO.TXT cut=0',60)
  step(lambda:q.key('delete'),r'\[files.app.trashed\] from=/HELLO.TXT to=/.Trash/HELLO.TXT')
  step(lambda:chord('z'),r'\[files.app.undone\] left=1')
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'files.ppm')});time.sleep(.5)
  expect('cat /HELLO.TXT',r'\[spindle.fs.cat\] hello from fat12')
  expect('ls /.Trash',r'\[spindle.fs.result\] kind=2 status=0')
  # Jot asks for a file; the person picks one on the stick in Files.
  s=len(t());line('run jot');m=launched(s);jt,jtask,jsid=m[1],m[2],int(m[3]);g['wait'](r'\[jot.ready\]',s,60);time.sleep(2)
  g['focus'](jsid);time.sleep(.5)
  s=len(t());chord('o');g['wait'](r'\[files.pick.asked\] row=\d+ mode=0',s,30);g['wait'](r'\[files.app.question\] program=jot mode=Open',s,30)
  g['focus'](fa_sid);time.sleep(.5)
  step(lambda:chord('d'),r'\[files.app.list\] dir=/media count=2')
  step(lambda:q.key('ret'),r'\[files.app.list\] dir=/media/FLOPPY ')
  for _ in range(3):q.key('down');time.sleep(.1)
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'files-picker.ppm')});time.sleep(.5)
  s=len(t());q.key('ret');g['wait'](r'\[files.pick.granted\] row=\d+ path=/media/FLOPPY/HELLO.TXT',s,30)
  g['wait'](r'\[jot.picked\] path=@/media/FLOPPY/HELLO.TXT',s,30)
  g['focus'](jsid);time.sleep(.5)
  s=len(t());keys('again');chord('s');g['wait'](r'\[jot.saved\] bytes=22\b',s,60)
  # The pick gave Jot that file only.
  s=len(t());chord('e');g['wait'](r'\[jot.escape\] refused=1 ',s,60)
  s=len(t());q.key('f11');g['wait'](rf'\[app.private.retired\] target={jt} task={jtask} ',s,90)
  expect('cat /media/FLOPPY/HELLO.TXT',r'\[spindle.fs.cat\] hello from fat12')
  g['wait'](r'\[spindle.fs.cat\] again',0,30)
  expect('unmount FLOPPY',r'\[spindle.fs.line\] Safe to remove\.')
  s=len(t());line('ls /media');g['wait'](r'\[spindle.fs.result\] kind=2 status=0',s,60);time.sleep(1)
  assert 'FLOPPY/' not in re.findall(r'\[spindle.fs.line\] ([^\n]*)',t()[s:])
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t() and '[files.app.fatal]' not in t()
 finally:g['stop']()
 print({'ok':True})
if __name__=='__main__':main()
