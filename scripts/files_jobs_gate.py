#!/usr/bin/env python3
"""Files: background copy with progress and cancel, the trash, and deleting for good.

A 32 MiB file is copied in the Files app in the background; Esc half-way must leave no
file (not even a hidden part) in the destination. Copied again to the end, its SHA-256
read back through Files (Ctrl+I) must equal the source's. Moved to the trash, it shows
up there and Ctrl+R restores it with the same hash; Shift+Del twice deletes it for good,
and Ctrl+E twice empties the trash."""
import argparse,hashlib,http.server,os,random,re,shutil,threading,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
SIZE=int(os.environ.get('FILES_JOBS_BYTES',str(32*1024*1024)))
BIG=random.Random(167).randbytes(SIZE)
BIG_SHA=hashlib.sha256(BIG).hexdigest()
FILES={'/files':(ROOT/'iso_root/programs/files').read_bytes(),'/big.bin':BIG}
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
 shutil.rmtree(a.out,ignore_errors=True);a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 g=boot(a.iso,disk,a.out,'filejobs',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   shift=k.isupper() or k in ':_'
   code={':':'semicolon','_':'minus'}.get(k,KEYS.get(k,k.lower()))
   if shift:q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':code}],'hold-time':80})
   else:q.key(code)
   time.sleep(.06)
 def line(text):g['focus'](153);time.sleep(.5);keys(text);q.key('ret')
 def expect(text,pattern,timeout=60):s=len(t());line(text);return g['wait'](pattern,s,timeout)
 def chord(key,*mods):q.cmd('send-key',{'keys':[{'type':'qcode','data':m} for m in mods]+[{'type':'qcode','data':key}],'hold-time':80})
 ctrl=lambda k:chord(k,'ctrl')
 def step(action,pattern,timeout=30):s=len(t());action();return g['wait'](pattern,s,timeout)
 result={}
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  expect('mkdir 0gate',r'\[spindle.fs.result\] kind=3 status=0')
  for name,dest in [('files','/bin/files'),('big.bin','0gate/big.bin')]:
   began=time.monotonic()
   m=expect(f'fetch http://10.0.2.2:{PORT}/{name} {dest}',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',600);assert m[1].startswith('Saved'),m[1]
   result[f'fetch_{name}_s']=round(time.monotonic()-began,1)
  s=len(t());line('files');m=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,60);fa_target,fa_sid=m[1],int(m[3])
  g['wait'](r'\[files.app.ready\] target=\d+ task=\d+ root=/ portal=1',s,60);g['wait'](rf'\[display.text.committed\] target={fa_target} ',s,60);time.sleep(1)
  g['focus'](fa_sid);time.sleep(.5)
  # 0gate sorts first in /.
  step(lambda:ctrl('h'),r'\[files.app.list\] dir=/ count=')
  step(lambda:q.key('ret'),r'\[files.app.list\] dir=/0gate count=1\b')
  ctrl('c');time.sleep(.5)
  step(lambda:ctrl('n'),r'\[display.text.committed\]');keys('dst')
  step(lambda:q.key('ret'),r'\[files.app.list\] dir=/0gate count=2\b')
  step(lambda:q.key('ret'),r'\[files.app.list\] dir=/0gate/dst count=0\b')
  # 1. Copy and stop it part way: nothing may be left in dst.
  s=len(t());ctrl('v')
  g['wait'](rf'\[files.app.job.start\] kind=Copy what=big.bin total={SIZE}\b',s,60)
  g['wait'](r'\[files.app.job.progress\] kind=Copy pct=([2-9]\d) ',s,300)
  q.cmd('screendump',{'filename':str(a.out/'files-copying.ppm')})
  q.key('esc')
  m=g['wait'](r'\[files.app.job.cancelled\] kind=Copy what=big.bin pct=(\d+)',s,60);result['cancelled_at_pct']=int(m[1])
  assert 0<int(m[1])<100,m[1]
  g['wait'](r'\[files.app.list\] dir=/0gate/dst count=0\b',s,30)
  assert '[files.app.job.done]' not in t()[s:],'the copy finished instead of stopping'
  # 2. Copy to the end; the copy reads back with the source's checksum.
  s=len(t());began=time.monotonic();ctrl('v')
  g['wait'](rf'\[files.app.job.done\] kind=Copy what=big.bin from=/0gate/big.bin to=/0gate/dst/big.bin bytes={SIZE}\b',s,600)
  result['copy_s']=round(time.monotonic()-began,1)
  g['wait'](r'\[files.app.list\] dir=/0gate/dst count=1\b',s,30)
  m=step(lambda:ctrl('i'),r'\[files.app.sum\] path=(\S+) bytes=(\d+) sha256=([0-9a-f]{64})',300)
  assert m[1]=='/0gate/dst/big.bin' and int(m[2])==SIZE and m[3]==BIG_SHA,(m[1],m[2],m[3],BIG_SHA)
  # 3. Trash, see it there with where it came from, restore, same checksum.
  step(lambda:q.key('delete'),r'\[files.app.trashed\] from=/0gate/dst/big.bin to=/.Trash/big.bin')
  step(lambda:ctrl('t'),r'\[files.app.list\] dir=/.Trash count=1\b')
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'files-trash.ppm')})
  step(lambda:ctrl('r'),r'\[files.app.restored\] from=/.Trash/big.bin to=/0gate/dst/big.bin')
  step(lambda:q.key('backspace'),r'\[files.app.list\] dir=/ count=')
  step(lambda:q.key('ret'),r'\[files.app.list\] dir=/0gate count=2\b')
  step(lambda:q.key('ret'),r'\[files.app.list\] dir=/0gate/dst count=1\b')
  m=step(lambda:ctrl('i'),r'\[files.app.sum\] path=(\S+) bytes=(\d+) sha256=([0-9a-f]{64})',300)
  assert m[1]=='/0gate/dst/big.bin' and m[3]==BIG_SHA,(m[1],m[3])
  # 4. Shift+Del asks first; another key keeps it; twice deletes for good.
  s=len(t());chord('delete','shift');time.sleep(1);q.key('down');time.sleep(1)
  assert '[files.app.job.start] kind=Delete' not in t()[s:],'deleted without asking twice'
  chord('delete','shift');time.sleep(.5)
  step(lambda:chord('delete','shift'),r'\[files.app.job.done\] kind=Delete what=big.bin ',120)
  g['wait'](r'\[files.app.list\] dir=/0gate/dst count=0\b',s,30)
  # 5. Empty the trash: the source goes there, then Ctrl+E twice.
  step(lambda:q.key('backspace'),r'\[files.app.list\] dir=/0gate count=2\b')
  q.key('down');time.sleep(.3)
  step(lambda:q.key('delete'),r'\[files.app.trashed\] from=/0gate/big.bin to=/.Trash/big.bin')
  step(lambda:ctrl('t'),r'\[files.app.list\] dir=/.Trash count=1\b')
  ctrl('e');time.sleep(.5)
  s=len(t());ctrl('e')
  g['wait'](r'\[files.app.job.done\] kind=Delete what=the trash ',s,120)
  g['wait'](r'\[files.app.list\] dir=/.Trash count=0\b',s,30)
  # 6. Search by a word inside a text file; the second time from the index, in under 1 s.
  expect('mkdir 0gate/notes',r'\[spindle.fs.result\] kind=3 status=0')
  expect('write 0gate/notes/plan.txt meeting about the Zanzibar budget',r'\[spindle.fs.result\] kind=8 status=0')
  expect('write 0gate/notes/other.txt nothing to see',r'\[spindle.fs.result\] kind=8 status=0')
  g['focus'](fa_sid);time.sleep(.5)
  step(lambda:ctrl('h'),r'\[files.app.list\] dir=/ count=')
  step(lambda:q.key('ret'),r'\[files.app.list\] dir=/0gate count=2\b')
  for n in (1,2):
   ctrl('f');time.sleep(.5);keys('zanzib')
   m=step(lambda:q.key('ret'),r'\[files.app.search\] base=/0gate query=zanzib results=(\d+) looked=(\d+) read=(\d+) ms=(\d+) finished=1',60)
   result[f'search{n}']={'results':int(m[1]),'looked':int(m[2]),'read':int(m[3]),'ms':int(m[4])}
   assert int(m[1])==1,m[0]
   g['wait'](r'\[files.app.found\] path=notes/plan.txt by=words',0,10)
  assert result['search2']['read']==0 and result['search2']['ms']<1000,result
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'files-search.ppm')})
  step(lambda:q.key('ret'),r'\[files.app.found.opened\] path=/0gate/notes/plan.txt')
  g['wait'](r'\[files.app.list\] dir=/0gate/notes count=2\b',0,10)
  # Spindle sees the same: nothing left of either copy, no hidden parts.
  s=len(t());line('ls 0gate/dst');g['wait'](r'\[spindle.fs.result\] kind=2 status=0',s,60);time.sleep(1)
  listed=re.findall(r'\[spindle.fs.line\] ([^\n]*)',t()[s:]);assert not any('big' in x or 'part' in x for x in listed),listed
  s=len(t());line('ls 0gate');g['wait'](r'\[spindle.fs.result\] kind=2 status=0',s,60);time.sleep(1)
  listed=re.findall(r'\[spindle.fs.line\] ([^\n]*)',t()[s:]);assert not any('big' in x for x in listed),listed
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t() and '[files.app.fatal]' not in t()
 finally:g['stop']()
 result['ok']=True;print(result)
if __name__=='__main__':main()
