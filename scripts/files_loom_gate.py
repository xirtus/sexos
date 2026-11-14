#!/usr/bin/env python3
"""Files (Loom): archives, batch rename, ratings and tags, duplicates, disk usage, compare.

A FAT32 USB stick holds a folder `docs` (two text files and a 300 KiB file in `docs/sub`),
a folder `ren` with three files, and `host.tar.gz` made on the host. Files, given that
stick:
1. packs `docs` into `docs.zip` (Ctrl+Shift+A), browses into it (Enter, then into its
   `docs` folder) and out (Backspace), then extracts it (Ctrl+Shift+X) into a new folder;
2. compares `docs` with the extracted copy (Ctrl+Shift+C): everything is the same;
3. browses into `host.tar.gz`, made by another program;
4. renames the files in `ren` by the mask `pic_[C:2]` (Ctrl+Shift+R), takes it back with
   Ctrl+Z, and renames again;
5. rates the first renamed file 3 stars (Ctrl+3) and tags it (Ctrl+Shift+G);
6. closes, starts again: the rating and tag are read back from `.loom-meta`; Ctrl+Shift+D
   finds the three duplicated files, Ctrl+U maps the disk usage.
On the host the stick then holds a zip that Python reads back byte for byte, the renamed
files, and the metadata file."""
import argparse,io,os,random,re,shutil,subprocess,tarfile,time,zipfile
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus','[':'bracket_left',']':'bracket_right',',':'comma'}
SHIFTED={':':'semicolon','_':'minus','(':'9',')':'0'}
MT={**os.environ,'MTOOLS_SKIP_CHECK':'1'}
CONTENT={'docs/alpha.txt':b'alpha: the first file\n'*40,'docs/beta.txt':b'beta, the second one\n'*7,'docs/sub/gamma.bin':random.Random(5).randbytes(300*1024)}
REN={'one.txt':b'one\n','two.txt':b'two two\n','three.txt':b'three three three\n'}
TAR={'readme.txt':b'made on the host\n','data/numbers.txt':b''.join(b'%d\n'%i for i in range(500))}
def make_stick(path,scratch):
 subprocess.run(['truncate','-s','64M',str(path)],check=True)
 subprocess.run(['mkfs.vfat','-F','32','-n','LOOM',str(path)],check=True,capture_output=True)
 for d in ('docs','docs/sub','ren'):subprocess.run(['mmd','-i',str(path),'::'+d],check=True,env=MT)
 for name,data in list(CONTENT.items())+[('ren/'+k,v) for k,v in REN.items()]:
  f=scratch/name.replace('/','_');f.write_bytes(data);subprocess.run(['mcopy','-i',str(path),str(f),'::'+name],check=True,env=MT)
 buf=io.BytesIO()
 with tarfile.open(fileobj=buf,mode='w:gz') as t:
  for name,data in TAR.items():
   info=tarfile.TarInfo(name);info.size=len(data);info.mtime=1700000000;t.addfile(info,io.BytesIO(data))
 f=scratch/'host.tar.gz';f.write_bytes(buf.getvalue());subprocess.run(['mcopy','-i',str(path),str(f),'::host.tar.gz'],check=True,env=MT)
def mread(stick,name):return subprocess.run(['mtype','-i',str(stick),'::'+name],check=True,env=MT,capture_output=True).stdout
def mlist(stick,d):
 out=subprocess.run(['mdir','-b','-i',str(stick),'::'+d],check=True,env=MT,capture_output=True,text=True).stdout
 return sorted(l.rstrip('/').split('/')[-1] for l in out.split('\n') if l.strip())
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 shutil.rmtree(a.out,ignore_errors=True);a.out.mkdir(parents=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 scratch=a.out/'host';scratch.mkdir();stick=a.out/'loom.img';make_stick(stick,scratch)
 import http.server,threading
 FILES={'/files':(ROOT/'iso_root/programs/files').read_bytes()}
 class Handler(http.server.BaseHTTPRequestHandler):
  def do_GET(self):
   body=FILES.get(self.path)
   if body is None:self.send_response(404);self.end_headers();return
   self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
  def log_message(self,*x):pass
 class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
 server=Quiet(('127.0.0.1',0),Handler);PORT=server.server_address[1];threading.Thread(target=server.serve_forever,daemon=True).start()
 os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
 os.environ.pop('SEXOS_GATE_DEVICES',None);os.environ.setdefault('SEXOS_GATE_BOUND','900')
 from quil_autosave_gate import boot
 g=boot(a.iso,disk,a.out,'loom',a.resolution);t=g['text'];q=g['q']
 def chord(k,*mods):q.cmd('send-key',{'keys':[{'type':'qcode','data':m} for m in mods]+[{'type':'qcode','data':k}],'hold-time':80})
 def keys(text):
  for k in text:
   if k.isupper() or k in SHIFTED:chord(SHIFTED.get(k,k.lower()),'shift')
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 def line(text):g['focus'](153);time.sleep(.5);keys(text);q.key('ret')
 def expect(text,pattern,timeout=60):s=len(t());line(text);return g['wait'](pattern,s,timeout)
 def step(action,pattern,timeout=60):s=len(t());action();return g['wait'](pattern,s,timeout)
 ctrl=lambda k:chord(k,'ctrl')
 cs=lambda k:chord(k,'ctrl','shift')
 E=re.escape
 def open_files():
  s=len(t());line('files /media/LOOM');m=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,60)
  g['wait'](r'\[files.app.list\] dir=/media/LOOM count=\d+',s,60);g['wait'](rf'\[display.text.committed\] target={m[1]} ',s,60);time.sleep(1)
  g['focus'](int(m[3]));time.sleep(.5);return s
 result={}
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  m=expect(f'fetch http://10.0.2.2:{PORT}/files /bin/files',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',240);assert m[1].startswith('Saved'),m[1]
  q.cmd('blockdev-add',{'driver':'raw','node-name':'loomdrive','file':{'driver':'file','filename':str(stick)}})
  s=len(t())
  for _ in range(40):
   try:q.cmd('device_add',{'driver':'usb-storage','bus':'xhci.0','drive':'loomdrive','id':'loom1'});break
   except RuntimeError:time.sleep(.5)
  g['wait'](r'\[files\.media\.mounted\] name=LOOM format=FAT32 [^\n]*source=usb',s,120)
  s=open_files()
  g['wait'](r'\[files.app.meta.loaded\] root=/media/LOOM records=0',s,10)
  # 1. Pack docs (the first row), browse into the zip and out, extract it.
  m=step(lambda:cs('a'),r'\[files.app.archive.created\] from=/media/LOOM/docs to=/media/LOOM/docs.zip files=(\d+) bytes=(\d+)',180)
  assert int(m[1])==3,m[0];result['zip_bytes']=int(m[2])
  g['wait'](r'\[files.app.list\] dir=/media/LOOM count=4\b',0,30)
  m=step(lambda:q.key('ret'),r'\[files.app.archive.opened\] path=/media/LOOM/docs.zip format=Zip entries=(\d+)')
  result['zip_entries']=int(m[1])
  g['wait'](r'\[files.app.archive.list\] archive=/media/LOOM/docs.zip dir=/ count=1\b',0,10)
  step(lambda:q.key('ret'),r'\[files.app.archive.list\] archive=/media/LOOM/docs.zip dir=docs count=3\b')
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'inside-zip.ppm')})
  step(lambda:q.key('backspace'),r'\[files.app.archive.list\] archive=/media/LOOM/docs.zip dir=/ count=1\b')
  step(lambda:q.key('backspace'),r'\[files.app.archive.left\] path=/media/LOOM/docs.zip')
  m=step(lambda:cs('x'),r'\[files.app.archive.extracted\] archive=/media/LOOM/docs.zip member=\* to=(/media/LOOM/[^\n]*?) files=(\d+) bytes=(\d+)',180)
  assert m[1]=='/media/LOOM/docs (extracted)' and int(m[2])==3 and int(m[3])==sum(map(len,CONTENT.values())),m[0]
  # 2. Compare docs with what came out of the zip.
  step(lambda:q.key('home'),r'\[display.text.committed\]',10)
  step(lambda:q.key('ret'),r'\[files.app.list\] dir=/media/LOOM/docs count=3\b')
  cs('c');time.sleep(.5);keys('/media/LOOM/docs (extracted)/docs')
  m=step(lambda:q.key('ret'),r'\[files.app.compare\] here=/media/LOOM/docs there=/media/LOOM/docs \(extracted\)/docs rows=(\d+) same=(\d+) differs=(\d+) here_only=(\d+) there_only=(\d+)',120)
  assert m.groups()==('4','4','0','0','0'),m[0]
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'compare.ppm')})
  # 3. A tar.gz made by another program browses too (the last row of the top).
  step(lambda:q.key('backspace'),r'\[files.app.list\] dir=/media/LOOM count=5\b')
  step(lambda:q.key('end'),r'\[display.text.committed\]',10)
  m=step(lambda:q.key('ret'),r'\[files.app.archive.opened\] path=/media/LOOM/host.tar.gz format=TarGz entries=(\d+)')
  g['wait'](r'\[files.app.archive.list\] archive=/media/LOOM/host.tar.gz dir=/ count=2\b',0,10)
  step(lambda:q.key('esc'),r'\[files.app.archive.left\] path=/media/LOOM/host.tar.gz')
  # 4. Batch rename in ren (third folder), undo, and again.
  step(lambda:q.key('home'),r'\[display.text.committed\]',10)
  q.key('down');time.sleep(.3);q.key('down');time.sleep(.3)
  step(lambda:q.key('ret'),r'\[files.app.list\] dir=/media/LOOM/ren count=3\b')
  for n in (1,2):
   step(lambda:cs('r'),r'\[files.app.rename.start\] dir=/media/LOOM/ren items=3')
   for _ in range(3):q.key('backspace');time.sleep(.05)
   keys('pic_[C:2]');time.sleep(1)
   if n==1:q.cmd('screendump',{'filename':str(a.out/'rename-preview.ppm')})
   step(lambda:q.key('ret'),r'\[files.app.renamed\] dir=/media/LOOM/ren mask=pic_\[C:2\] files=3 moves=3')
   g['wait'](r'\[files.app.renamed.file\] from=one.txt to=pic_01.txt',0,5)
   if n==1:step(lambda:ctrl('z'),r'\[files.app.undone\]');g['wait'](r'\[files.app.list\] dir=/media/LOOM/ren count=3\b',0,10)
  # 5. Rate and tag the first file.
  step(lambda:q.key('home'),r'\[display.text.committed\]',10)
  step(lambda:ctrl('3'),r'\[files.app.meta.rated\] path=ren/pic_01.txt stars=3')
  cs('g');time.sleep(.5);keys('holiday')
  step(lambda:q.key('ret'),r'\[files.app.meta.tagged\] path=ren/pic_01.txt tags=holiday')
  g['wait'](r'\[files.app.meta.saved\] records=1',0,10)
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'rated.ppm')})
  step(lambda:q.key('f11'),r'\[files.app.closed\]');time.sleep(1)
  # 6. Again: the rating is still there; duplicates and the usage map.
  s=open_files()
  g['wait'](r'\[files.app.meta.loaded\] root=/media/LOOM records=1',s,10)
  g['wait'](r'\[files.app.meta.record\] path=ren/pic_01.txt rating=3 tags=holiday',s,10)
  m=step(lambda:cs('d'),r'\[files.app.dupes\] base=/media/LOOM files=(\d+) groups=(\d+) wasted=(\d+)',180)
  assert m[2]=='3' and int(m[3])==sum(map(len,CONTENT.values())),m[0]
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'dupes.ppm')})
  step(lambda:q.key('esc'),r'\[files.app.list\] dir=/media/LOOM count=5\b')
  m=step(lambda:ctrl('u'),r'\[files.app.usage\] base=/media/LOOM files=(\d+) nodes=(\d+) bytes=(\d+)',180)
  result['usage']=m[0]
  time.sleep(2);q.cmd('screendump',{'filename':str(a.out/'usage.ppm')})
  # Enter goes into the biggest item when it is a folder.
  m=g['wait'](r'\[files.app.usage.view\] path=/ total=\d+ items=5 biggest=([^:\n]+):',0,10)
  if not m[1].endswith('.zip'):step(lambda:q.key('ret'),rf'\[files.app.usage.view\] path=/{E(m[1])} ',10)
  step(lambda:q.key('esc'),r'\[files.app.list\] dir=/media/LOOM count=5\b')
  step(lambda:q.key('f11'),r'\[files.app.closed\]');time.sleep(2)
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t() and '[files.app.fatal]' not in t()
 finally:g['stop']()
 # On the host: the zip reads back, the names changed, the metadata is on the stick.
 z=zipfile.ZipFile(io.BytesIO(mread(stick,'docs.zip')))
 assert z.testzip() is None
 for name,data in CONTENT.items():assert z.read(name)==data,name
 for name,data in CONTENT.items():assert mread(stick,'docs (extracted)/'+name)==data,name
 assert mlist(stick,'ren')==['pic_01.txt','pic_02.txt','pic_03.txt'],mlist(stick,'ren')
 assert mread(stick,'ren/pic_01.txt')==REN['one.txt']
 meta=mread(stick,'.loom-meta');assert b'ren/pic_01.txt\t3\t' in meta and b'holiday' in meta,meta
 result['ok']=True;print(result)
if __name__=='__main__':main()
