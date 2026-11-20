#!/usr/bin/env python3
"""Settings' Privacy page takes a grant away, and the program's next connect is refused
(workstream R1).

chello (sex-libc's C program) is packaged asking for the network, installed from the
host's HTTP server, and allowed the network in Spindle (`pkg allow chello network`):
`run chello net HOST PORT` reaches the echo server (23 checks). Settings (installed on
the disk) opens, PgUp goes to its last page, Privacy and grants, which lists chello's
network grant from Files; Left on that row takes it away (Files records the person's
answer for Settings, and only for Settings). Then `run chello one two` is not asked
anything and runs without the network: its own check that connect() fails with EACCES
passes (21 checks)."""
import re,shutil,time
from pathlib import Path
import package_gate as pg
from install_program import install
from settings_persist_gate import first_start
ROOT=Path(__file__).resolve().parents[1]

def main():
 import argparse
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True)
 elf=ROOT/'iso_root/programs/chello';data=elf.read_bytes();assert data[:4]==b'\x7fELF'
 if b'This build has no C compiler output' in data:raise SystemExit('chello is the no-compiler fallback: install musl headers and rebuild')
 serve=a.out/'serve';shutil.rmtree(serve,ignore_errors=True);serve.mkdir()
 (serve/'README').write_text('chello from a package\n')
 (serve/'m').write_text('name=chello\nversion=1.0\nsummary=C library checks\nlicense=GPL-3.0\ngrant=network\n')
 pg.sexpkg('build','test',serve/'m',serve/'chello.sexpkg',f'bin/chello={elf}',f'data/README={serve/"README"}')
 key=pg.sexpkg('public','test');assert re.fullmatch(r'[0-9a-f]{64}',key)
 import functools,http.server,threading
 web=http.server.ThreadingHTTPServer(('127.0.0.1',0),functools.partial(pg.Quiet,directory=str(serve)));threading.Thread(target=web.serve_forever,daemon=True).start()
 base=f'http://10.0.2.2:{web.server_address[1]}/'
 disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 install(disk,[('tatami',(ROOT/'iso_root/programs/tatami').read_bytes())])
 g=pg.boot(a.iso,disk,a.out,'privacy',a.resolution);t=g['text'];q=g['q']
 def type_line(line):
  g['focus'](153);time.sleep(.5)
  for k in line:
   if k in pg.SHIFTED:q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':pg.SHIFTED[k]}],'hold-time':80})
   else:q.key(pg.KEYS.get(k,k))
   time.sleep(.04)
  q.key('ret')
 def do(line,pattern,timeout=600):
  s=len(t());type_line(line);return g['wait'](pattern,s,timeout),s
 def checks(least,s):
  m=g['wait'](r'\[chello\.(ok|fail)\] passed=(\d+)/(\d+)\n',s,600)
  if m[1]!='ok':time.sleep(1);raise AssertionError('chello checks failed: '+''.join(re.findall(r'\[std\.out\] FAIL[^\n]*\n',t()[s:])))
  assert int(m[3])>=least and m[2]==m[3],f'not every check ran: {m[0]}'
  e=g['wait'](r'\[std\.exit\] target=(\d+) task=(\d+) status=0\n',s,60)
  s2=len(t());g['focus'](pg.SURFACE[int(e[1])]);time.sleep(.5);q.key('f11')
  g['wait'](rf'\[app\.private\.retired\] target={e[1]} task={e[2]} ',s2,180)
  return int(m[2])
 out={}
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  do(f'pkg trust {key}',r'\[files\.pkg\.trusted\] key='+key[:16])
  m,_=do(f'pkg install {base}chello.sexpkg',r'\[files\.pkg\.(refused|installed)\] ([^\n]*)\n')
  assert m[1]=='installed',m[0]
  do('pkg allow chello network',r'\[files\.pkg\.consent\] program=chello answered=1 allowed=1\n')
  m,s=do(f'run chello net 10.0.2.2 {pg.ECHO_PORT}',r'\[spindle\.pkg\.note\] program=chello asked=1 answered=1 allowed=1\n')
  out['allowed_checks']=checks(23,s)
  # Settings: first-start setup, then its last page (PgUp from the first).
  s=len(t());type_line('run tatami');first_start(g,q,t,s);g['wait'](r'\[settings\.open\] generation=\d+',s,120);time.sleep(1)
  s=len(t());q.key('pgup')
  g['wait'](r'\[files\.pkg\.settings\] op=10 arg=\n',s,30)
  g['wait'](r'\[settings\.grants\] status=0 rows=1\n',s,30)
  # Two first-run settings, then chello's network row; Left takes it away.
  time.sleep(.3);q.key('down');time.sleep(.3);q.key('down');time.sleep(.3)
  s=len(t());q.key('left')
  g['wait'](r'\[files\.pkg\.settings\] op=4 arg=chello\n',s,30)
  g['wait'](r'\[files\.pkg\.consent\] program=chello answered=1 allowed=0\n',s,30)
  g['wait'](r'\[settings\.grant\] program=chello grant=network allow=0 status=0\n',s,30)
  g['wait'](r'\[settings\.grants\] status=0 rows=1\n',s,30)
  # Its next run: no question, no network; connect() is refused (EACCES).
  m,s=do('run chello one two',r'\[spindle\.pkg\.note\] program=chello asked=1 answered=1 allowed=0\n')
  out['revoked_checks']=checks(21,s)
  assert '[spindle.pkg.question]' not in t()[s:],'asked again after Settings answered'
  text=t();assert 'KERNEL PANIC' not in text and 'KERNEL PAGE FAULT' not in text
  print({'ok':True,**out})
 finally:
  g['stop']();web.shutdown()
if __name__=='__main__':main()
