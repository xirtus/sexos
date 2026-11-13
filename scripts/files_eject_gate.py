#!/usr/bin/env python3
"""Files: safe removal of USB sticks.

Stick PULLME: Files, given only that stick, copies a file on it; the stick is pulled out
half-way. Files says so instead of failing silently, the files already on the stick are
untouched, and the grant to /media/PULLME ends with the stick: plugged back in, it mounts
again but the old run can no longer reach it.

Stick EJECTME: a second Files run copies a file on it to the end; its SHA-256 read back
matches; Ctrl+E ejects it (written out, flushed, grant taken back) and it is pulled
out. On the host, `fsck.vfat -n` finds the image clean and the copy reads back equal."""
import argparse,hashlib,os,random,re,shutil,subprocess,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
BIG=random.Random(8).randbytes(8*1024*1024)
SHA=hashlib.sha256(BIG).hexdigest()
KEEP=b'kept safe on the stick\n'
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}
MT={**os.environ,'MTOOLS_SKIP_CHECK':'1'}
def make_stick(path,label,scratch):
 subprocess.run(['truncate','-s','64M',str(path)],check=True)
 subprocess.run(['mkfs.vfat','-F','32','-n',label,str(path)],check=True,capture_output=True)
 (scratch/'BIG.BIN').write_bytes(BIG);(scratch/'KEEP.TXT').write_bytes(KEEP)
 for n in ('BIG.BIN','KEEP.TXT'):subprocess.run(['mcopy','-i',str(path),str(scratch/n),'::'+n],check=True,env=MT)
def mread(path,name):return subprocess.run(['mtype','-i',str(path),'::'+name],capture_output=True,check=True,env=MT).stdout
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 shutil.rmtree(a.out,ignore_errors=True);a.out.mkdir(parents=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 scratch=a.out/'host';scratch.mkdir()
 pull=a.out/'pullme.img';eject=a.out/'ejectme.img'
 make_stick(pull,'PULLME',scratch);make_stick(eject,'EJECTME',scratch)
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
 g=boot(a.iso,disk,a.out,'eject',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   shift=k.isupper() or k in ':_'
   code={':':'semicolon','_':'minus'}.get(k,KEYS.get(k,k.lower()))
   if shift:q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':code}],'hold-time':80})
   else:q.key(code)
   time.sleep(.06)
 def line(text):g['focus'](153);time.sleep(.5);keys(text);q.key('ret')
 def expect(text,pattern,timeout=60):s=len(t());line(text);return g['wait'](pattern,s,timeout)
 def ctrl(k):q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':k}],'hold-time':80})
 def step(action,pattern,timeout=30):s=len(t());action();return g['wait'](pattern,s,timeout)
 def plug(node,dev,name):
  s=len(t())
  for _ in range(40):
   try:q.cmd('device_add',{'driver':'usb-storage','bus':'xhci.0','drive':node,'id':dev});break
   except RuntimeError:time.sleep(.5)
  else:raise RuntimeError(f'{dev} could not be plugged in')
  g['wait'](rf'\[files\.media\.mounted\] name={name} format=FAT32 [^\n]*source=usb',s,120)
 def open_files(folder,count):
  s=len(t());line(f'files {folder}');m=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,60)
  g['wait'](rf'\[files.app.list\] dir={folder} count={count}\b',s,60);g['wait'](rf'\[display.text.committed\] target={m[1]} ',s,60);time.sleep(1)
  g['focus'](int(m[3]));time.sleep(.5);return int(m[3])
 result={}
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  m=expect(f'fetch http://10.0.2.2:{PORT}/files /bin/files',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',240);assert m[1].startswith('Saved'),m[1]
  for node,img in (('pulldrive',pull),('ejectdrive',eject)):
   q.cmd('blockdev-add',{'driver':'raw','node-name':node,'file':{'driver':'file','filename':str(img)}})
  # 1. Pulled out mid-copy.
  plug('pulldrive','pull1','PULLME')
  r1=open_files('/media/PULLME',2)
  ctrl('c');time.sleep(.5)
  s=len(t());ctrl('v')
  g['wait'](r'\[files.app.job.start\] kind=Copy what=BIG \(copy\)\.BIN ',s,60)
  g['wait'](r'\[files.app.job.progress\] kind=Copy pct=([1-9]\d) ',s,300)
  q.cmd('device_del',{'id':'pull1'})
  g['wait'](r'\[files\.media\.usb\.gone\] name=PULLME ',s,60)
  g['wait'](r'\[files\.media\.revoked\] prefix=/media/PULLME grants=1\b',s,60)
  g['wait'](r'\[files.app.job.failed\] kind=Copy ',s,120)
  g['wait'](r'\[files.app.pulled\] name=PULLME',s,60)
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'files-pulled.ppm')})
  # Back in: it mounts again, but the old run's grant ended with it.
  plug('pulldrive','pull2','PULLME')
  g['focus'](r1);time.sleep(.5)
  step(lambda:ctrl('h'),r'\[files.app.list.failed\] dir=/media/PULLME ')
  expect('cat /media/PULLME/KEEP.TXT',r'\[spindle.fs.cat\] kept safe on the stick')
  expect('unmount PULLME',r'\[spindle.fs.line\] Safe to remove\.')
  s=len(t());q.cmd('device_del',{'id':'pull2'});g['wait'](r'\[sexusb\.storage\.gone\] disk=\d+ ',s,60)
  # 2. Copied to the end, ejected, then pulled out.
  plug('ejectdrive','eject1','EJECTME')
  open_files('/media/EJECTME',2)
  ctrl('c');time.sleep(.5)
  s=len(t());ctrl('v')
  g['wait'](rf'\[files.app.job.done\] kind=Copy what=BIG \(copy\)\.BIN from=/media/EJECTME/BIG.BIN to=/media/EJECTME/BIG \(copy\)\.BIN bytes={len(BIG)}\b',s,600)
  g['wait'](r'\[files.app.list\] dir=/media/EJECTME count=3\b',s,30)
  m=step(lambda:ctrl('i'),r'\[files.app.sum\] path=([^\n]+) bytes=(\d+) sha256=([0-9a-f]{64})',300)
  assert m[1]=='/media/EJECTME/BIG (copy).BIN' and m[3]==SHA,(m[1],m[3])
  s=len(t());ctrl('e');time.sleep(3);q.cmd('screendump',{'filename':str(a.out/'files-ejecting.ppm')})
  g['wait'](r'\[files\.media\.unmounted\] name=EJECTME',s,60)
  g['wait'](r'\[files\.media\.revoked\] prefix=/media/EJECTME grants=1\b',s,60)
  g['wait'](r'\[files.app.ejected\] name=EJECTME',s,60)
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'files-ejected.ppm')})
  s=len(t());q.cmd('device_del',{'id':'eject1'});g['wait'](r'\[sexusb\.storage\.gone\] disk=\d+ ',s,60)
  assert '[files.app.pulled] name=EJECTME' not in t()
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t() and '[files.app.fatal]' not in t()
 finally:g['stop']()
 # On the host: the ejected stick is clean and holds the copy; the pulled one kept its files.
 fsck=subprocess.run(['fsck.vfat','-n',str(eject)],capture_output=True,text=True)
 assert fsck.returncode==0,fsck.stdout+fsck.stderr
 assert hashlib.sha256(mread(eject,'BIG (copy).BIN')).hexdigest()==SHA
 assert mread(pull,'KEEP.TXT')==KEEP and hashlib.sha256(mread(pull,'BIG.BIN')).hexdigest()==SHA
 result['pulled_fsck']=subprocess.run(['fsck.vfat','-n',str(pull)],capture_output=True,text=True).returncode
 result['ok']=True;print(result)
if __name__=='__main__':main()
