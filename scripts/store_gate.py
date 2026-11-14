#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""The Store program (apps/store) and the `+pkg` run grant that lets it install packages.

The host builds chello into signed packages (tools/sexpkg, the gates' test key) and serves
a signed store catalog over HTTP, as scripts/package_gate.py does. Store goes into the
disk program store. In Spindle:

* `pkg trust KEY` and `pkg refresh URL` once, so Files keeps a catalog and its source;
* `run +net store get chello`, without `+pkg`: Files refuses the package request
  (DENIED, logged as `[files.pkg.denied]`) and nothing is installed;
* `run +net +pkg sexshop refresh` downloads the catalog into Store's own folder and Files
  checks it; `run +pkg +net sexshop search C library` finds chello;
  `run +net +pkg sexshop get chello` installs 1.0;
* `run chello one two` still asks the person the package's grant question in Spindle;
  the answer `no` runs it without the network (its own 21 checks pass);
* the catalog then offers 1.1: `store refresh` and `store upgrade` upgrade it;
  `store remove chello` removes it, and `pkg list` shows none;
* plain `store` opens the window, which draws the catalog list."""
import os
os.environ.setdefault('SEXOS_GATE_BOUND','2400')
import argparse,functools,http.server,re,shutil,threading,time
from pathlib import Path
# The echo server, the guest network card and the key typing come from the package gate.
from package_gate import ROOT,SURFACE,KEYS,SHIFTED,Quiet,sexpkg
from install_program import install
from quil_autosave_gate import boot

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True)
 elf=ROOT/'iso_root/programs/chello';data=elf.read_bytes();assert data[:4]==b'\x7fELF'
 if b'This build has no C compiler output' in data:raise SystemExit('chello is the no-compiler fallback: install musl headers and rebuild')
 store_elf=(ROOT/'iso_root/programs/sexshop').read_bytes();assert store_elf[:4]==b'\x7fELF'
 serve=a.out/'serve';shutil.rmtree(serve,ignore_errors=True);serve.mkdir()
 (serve/'README').write_text('chello from the Store\n')
 for v in ['1.0','1.1']:
  (serve/f'm{v}').write_text(f'name=chello\nversion={v}\nsummary=C library checks\nlicense=GPL-3.0\ngrant=network\n')
  sexpkg('build','test',serve/f'm{v}',serve/f'chello-{v}.sexpkg',f'bin/chello={elf}',f'data/README={serve/"README"}')
 store=serve/'store';store.mkdir()
 shutil.copyfile(serve/'chello-1.0.sexpkg',store/'chello-1.0.sexpkg');sexpkg('index','test',store)
 key=sexpkg('public','test');assert re.fullmatch(r'[0-9a-f]{64}',key)
 web=http.server.ThreadingHTTPServer(('127.0.0.1',0),functools.partial(Quiet,directory=str(serve)));threading.Thread(target=web.serve_forever,daemon=True).start()
 base=f'http://10.0.2.2:{web.server_address[1]}/'
 disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 install(disk,[('sexshop',store_elf)])
 g=boot(a.iso,disk,a.out,'store',a.resolution);t=g['text'];q=g['q']
 def type_line(line):
  g['focus'](153);time.sleep(.5)
  for k in line:
   if k in SHIFTED:q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':SHIFTED[k]}],'hold-time':80})
   elif k.isupper():q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':k.lower()}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.04)
  q.key('ret')
 def do(line,pattern,timeout=600):
  s=len(t());type_line(line);return g['wait'](pattern,s,timeout),s
 def store_run(line,op):
  """One Store command: its outcome, and the text it logged. Waits for its row to retire."""
  m,s=do(line,rf'\[store\.done\] op={op} status=(\w+) lines=\d+\n')
  ready=re.search(r'\[store\.ready\] target=(\d+) task=(\d+) packages=(\d)',t()[s:]);assert ready,'Store never said it was ready'
  g['wait'](rf'\[app\.private\.retired\] target={ready[1]} task={ready[2]} ',s,180)
  return m[1],t()[s:],ready[3]
 def checks(score,least,s):
  m=g['wait'](score,s,600)
  if m[1]!='ok':time.sleep(1);raise AssertionError('chello checks failed: '+''.join(re.findall(r'\[std\.out\] FAIL[^\n]*\n',t()[s:])))
  assert int(m[3])>=least and m[2]==m[3],f'not every check ran: {m[0]}'
  e=g['wait'](r'\[std\.exit\] target=(\d+) task=(\d+) status=0\n',s,60)
  s2=len(t());g['focus'](SURFACE[int(e[1])]);time.sleep(.5);q.key('f11')
  g['wait'](rf'\[app\.private\.retired\] target={e[1]} task={e[2]} ',s2,180)
  return int(m[2])
 out={}
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  do(f'pkg trust {key}',r'\[files\.pkg\.trusted\] key='+key[:16])
  do(f'pkg refresh {base}store',r'\[files\.pkg\.catalog\] packages=1\n')
  # Without +pkg: refused, nothing installed.
  status,text,granted=store_run('run +net store get chello','get')
  assert granted=='0' and status=='denied',f'a run without +pkg was not refused: status={status}'
  assert re.search(r'\[files\.pkg\.denied\] target=\d+ op=8 why=no_grant',text),'Files did not log the refusal'
  assert '[files.pkg.installed]' not in text and '[files.pkg.locate]' not in text,'Files acted for a run without +pkg'
  # A typed `+packages` is not the grant: Spindle drops it.
  status,text,granted=store_run('run +net store +packages get chello','get')
  assert granted=='0' and status=='denied',f'a typed +packages was taken as the grant: status={status}'
  out['refused_without_pkg']=True
  status,text,granted=store_run('run +net +pkg sexshop refresh','refresh')
  assert granted=='1' and status=='ok' and '[files.pkg.catalog] packages=1' in text,f'refresh: {status}'
  status,text,_=store_run('run +pkg +net sexshop search C library','search')
  assert status=='ok' and re.search(r'\[store\.line\] chello 1\.0  C library checks',text),f'search: {status}'
  status,text,_=store_run('run +net +pkg sexshop get chello','get')
  assert status=='ok' and re.search(r'\[files\.pkg\.installed\] name=chello version=1\.0 program=chello upgrade=0',text),f'get: {status}'
  out['installed']='chello 1.0'
  # The program it installed: its first run still asks the person, in Spindle.
  m,s=do('run chello one two',r'\[spindle\.pkg\.question\] program=chello grants=1\n')
  do('no',r'\[spindle\.pkg\.run\] program=chello network=0\n')
  out['first_run_checks']=checks(r'\[chello\.(ok|fail)\] passed=(\d+)/(\d+)\n',21,s)
  # A newer version in the store: refresh, then upgrade everything.
  (store/'chello-1.0.sexpkg').unlink();shutil.copyfile(serve/'chello-1.1.sexpkg',store/'chello-1.1.sexpkg');sexpkg('index','test',store)
  status,text,_=store_run('store refresh','refresh')
  assert status=='ok','refresh from the saved source: '+status
  status,text,_=store_run('store search chello','search')
  assert 'newer here' in text,'the catalog does not show 1.1 as newer'
  status,text,_=store_run('store upgrade','upgrade')
  assert status=='ok' and re.search(r'\[files\.pkg\.installed\] name=chello version=1\.1 program=chello upgrade=1',text),f'upgrade: {status}'
  status,text,_=store_run('store upgrade','upgrade')
  assert status=='ok' and '[files.pkg.locate] name= found=0' in text,'a second upgrade found something'
  out['upgraded']='chello 1.1'
  status,text,_=store_run('store remove chello','remove')
  assert status=='ok' and '[files.pkg.removed] name=chello program=chello' in text,f'remove: {status}'
  do('pkg list',r'\[files\.pkg\.list\] count=0\n')
  out['removed']=True
  # The window: it draws the catalog.
  m,s=do('store',r'\[store\.list\] items=(\d+) installed=(\d+) newer=(\d+) selected=0 render=1\n')
  assert m[1]=='1' and m[2]=='0',f'the window listed {m[0]}'
  shot=a.out/'store.ppm';q.cmd('screendump',{'filename':str(shot)});time.sleep(.5)
  try:
   from PIL import Image;Image.open(shot).save(a.out/'store.png')
  except Exception:pass
  out['window']='list drawn'
  text=t();assert 'KERNEL PANIC' not in text and 'KERNEL PAGE FAULT' not in text and '[store.fatal]' not in text
  print({'ok':True,**out})
 finally:
  g['stop']();web.shutdown()
if __name__=='__main__':main()
