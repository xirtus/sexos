#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""CPython 3.13 (apps/python, vendor/cpython-3.13.18) runs on SexOS.

Each run is typed into Spindle as `run python ARGS`, exactly as a person would:

* `-c print(2**100)`: big integers.
* `-c "..."` (COMP): dict, list and set comprehensions, f-strings with format specs,
  `%` formatting, str methods, map/join.
* `-c "..."` (JSON): `json.dumps`/`json.loads` and `re.sub`, from the frozen standard
  library (json, re, enum, functools, collections...).
* `-c "..."` (FILES): open/write/read of a script file in the program's folder.
* `t.py x y`: that script file run with arguments (`sys.argv`), `os.listdir` of the
  folder, and `time.time()` against this host's clock.
* `-c "..."` (RAISE): an uncaught exception: the traceback reaches standard error and
  the exit status is 1. The window interprets no escape sequences, so the
  traceback must come without colour.

Output must reach the serial record as `[std.out]`/`[std.err]` lines, and each
program must exit with the right status and close."""
import argparse,re,shutil,time
from pathlib import Path
from install_program import install
import os
os.environ.setdefault('SEXOS_GATE_BOUND','1200')
from quil_autosave_gate import boot
from lua_gate import qcodes
ROOT=Path(__file__).resolve().parents[1]
SURFACE={int(a):int(b)for a,b in re.findall(r'app: App::Program, ordinal: \d+, target: (\d+), surface: (\d+)',(ROOT/'crates/sex-pdx/src/app_catalog.rs').read_text())}

# Spindle's line holds 256 bytes; quotes group words, with \" for a quote inside
# (crates/sex-std/src/args.rs). Python's own escapes (\n, \') pass through.
COMP=("\"d={k:k*k for k in range(6)};l=[v for v in d.values() if v%2==0];s={c.upper() for c in 'abca'};"
      "print('[py.c]',d,l,sorted(s),f'{sum(l):05d}|{3.14159:8.3f}|{255:#x}|%s-%d'%('x',7),'-'.join(map(str,range(3))))\"")
JSON=("\"import json,re;print('[py.j]',json.dumps({'b':[1,2.5,None,True],'a':{'c':'x'}},sort_keys=True),"
      "json.loads('[1,{\\\"k\\\":[2,3]}]')[1]['k'][1],re.sub(r'[0-9]+','#','a1b22c'))\"")
FILES=("\"f=open('t.py','w');f.write('import sys,os,time\\nprint(\\'[py.s]\\',sys.argv[1:],\\'t.py\\' in os.listdir(\\'.\\'),int(time.time()))\\n');"
       "f.close();print('[py.f]',len(open('t.py').read().splitlines()))\"")
RAISE="\"import json;json.loads('{bad')\""

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 elf=(ROOT/'iso_root/programs/python').read_bytes();assert elf[:4]==b'\x7fELF'
 if b'This build has no C compiler output (musl headers were missing).' in elf:
  raise SystemExit('python is the no-compiler fallback: install musl headers or set MUSL_INCLUDE, then rebuild before running this gate')
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 install(disk,[('python',elf)])
 g=boot(a.iso,disk,a.out,'python',a.resolution);t=g['text'];q=g['q']
 def type_line(line):
  assert len(line)<256,f'line too long for Spindle: {len(line)}'
  g['focus'](153);time.sleep(.5)
  # One key at a time, each confirmed by Spindle's echo (see lua_gate.py).
  base=None
  for n,ch in enumerate(line):
   s=len(t())
   q.cmd('send-key',{'keys':[{'type':'qcode','data':k}for k in qcodes(ch)],'hold-time':40})
   m=g['wait'](rf'\[spindle\.input\.recv\] key=printable ch={re.escape(ch)} len=(\d+)\n',s,30)
   if base is None:base=int(m[1])-n
   assert int(m[1])==base+n,f'Spindle dropped a key before {line[:n+1]!r}'
  s=len(t());q.key('ret')
  g['wait'](rf'\[spindle\.input\.recv\] key=enter len={base+len(line)-1}\n',s,30)
 def run(args,expect,status=0):
  s=len(t());type_line(f'run python {args}')
  found=[g['wait'](pattern,s,600) for pattern in expect]
  e=g['wait'](rf'\[std\.exit\] target=(\d+) task=(\d+) status={status}\n',s,300)
  target,task=int(e[1]),int(e[2])
  lines=re.findall(r'\[std\.(?:out|err)\][^\n]*',t()[s:])
  s2=len(t());g['focus'](SURFACE[target]);time.sleep(.5);q.key('f11')
  g['wait'](rf'\[app\.private\.retired\] target={target} task={task} ',s2,180)
  return found,lines
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  report=[]
  _,l=run('-c print(2**100)',[r'\[std\.out\] 1267650600228229401496703205376\n']);report+=l
  _,l=run('-c '+COMP,[re.escape("[std.out] [py.c] {0: 0, 1: 1, 2: 4, 3: 9, 4: 16, 5: 25} [0, 4, 16] ['A', 'B', 'C'] 00020|   3.142|0xff|x-7 0-1-2\n")]);report+=l
  _,l=run('-c '+JSON,[re.escape('[std.out] [py.j] {"a": {"c": "x"}, "b": [1, 2.5, null, true]} 3 a#b#c\n')]);report+=l
  _,l=run('-c '+FILES,[r'\[std\.out\] \[py\.f\] 2\n']);report+=l
  (m,),l=run('t.py x y',[r"\[std\.out\] \[py\.s\] \['x', 'y'\] True (\d+)\n"]);report+=l
  assert abs(int(m[1])-time.time())<600,f'time.time() is off: {m[1]} vs {int(time.time())}'
  _,l=run('-c '+RAISE,[r'\[std\.err\] Traceback \(most recent call last\):\n',r'\[std\.err\] json\.decoder\.JSONDecodeError: [^\n]*\n'],status=1);report+=l
  assert not any('\x1b' in x for x in l),'escape sequences reached the window (TERM=dumb)'
  text=t();assert 'KERNEL PANIC' not in text and '[std.abort]' not in text and 'KERNEL PAGE FAULT' not in text
  for line in report:print(line)
  print({'ok':True,'runs':6})
 finally:g['stop']()
if __name__=='__main__':main()
