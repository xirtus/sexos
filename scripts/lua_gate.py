#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""The official Lua 5.4 interpreter (apps/lua, vendor/lua-5.4.7) runs on SexOS.

Each run is typed into Spindle as `run lua ARGS`, exactly as a person would:

* `-e print(2^100)`: float arithmetic and `%.14g` printing.
* `-e "..."` (TABLES): tables, table.sort with a Lua comparator, table.concat,
  coroutines passing values both ways, string.format (%s %d %5.2f %q), io.write.
* `-e "..."` (FILES): io.open/write/close/read of a script file in the program's
  folder, os.time against this host's clock, os.date, string methods, and pcall
  catching error() (setjmp/longjmp in sex-libc).
* `t.lua x y`: that script file run with arguments (`...` and `arg`).
* `-e "error('bad')"`: an uncaught error reaches standard error and the exit status
  is 1.

Output must reach the serial record as `[std.out]`/`[std.err]` lines, and each
program must exit with the right status and close."""
import argparse,re,shutil,time
from pathlib import Path
from install_program import install
import os
os.environ.setdefault('SEXOS_GATE_BOUND','1200')
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
SURFACE={int(a):int(b)for a,b in re.findall(r'app: App::Program, ordinal: \d+, target: (\d+), surface: (\d+)',(ROOT/'crates/sex-pdx/src/app_catalog.rs').read_text())}

# Spindle's line holds 256 bytes; quotes group words (crates/sex-std/src/args.rs).
TABLES=("\"t={}for i=1,3 do t[i]=i*i end table.sort(t,function(a,b)return a>b end)"
        "c=coroutine.wrap(function(a)coroutine.yield(coroutine.yield(a+1)*2)end)"
        "io.write(('[lua.t] %s %d %d %5.2f %q\\n'):format(table.concat(t,','),c(1),c(20),math.pi,_VERSION))\"")
FILES=("\"local f=assert(io.open('t.lua','w')) f:write([[print('[lua.s]',#arg,os.time(),...)]]) f:close() "
       "local ok,e=pcall(error,'boom') print('[lua.f]',#io.open('t.lua'):read('a'),os.date('!%Y-%m-%d',86400),('abc'):upper():rep(2,'-'),ok,e)\"")

# US keyboard: characters and the qcodes that type them (with shift where marked).
PLAIN={' ':'spc','.':'dot',',':'comma','-':'minus','=':'equal','[':'bracket_left',']':'bracket_right',
       '\\':'backslash',';':'semicolon',"'":'apostrophe','`':'grave_accent','/':'slash'}
SHIFTED={'!':'1','@':'2','#':'3','$':'4','%':'5','^':'6','&':'7','*':'8','(':'9',')':'0','_':'minus',
         '+':'equal','{':'bracket_left','}':'bracket_right','|':'backslash',':':'semicolon','"':'apostrophe',
         '~':'grave_accent','<':'comma','>':'dot','?':'slash'}
def qcodes(ch):
 if ch in PLAIN:return [PLAIN[ch]]
 if ch in SHIFTED:return ['shift',SHIFTED[ch]]
 if ch.isupper():return ['shift',ch.lower()]
 assert ch.isalnum() and ch.isascii(),f'cannot type {ch!r}'
 return [ch]

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 elf=(ROOT/'iso_root/programs/lua').read_bytes();assert elf[:4]==b'\x7fELF'
 if b'This build has no C compiler output (musl headers were missing).' in elf:
  raise SystemExit('lua is the no-compiler fallback: install musl headers or set MUSL_INCLUDE, then rebuild before running this gate')
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 install(disk,[('lua',elf)])
 g=boot(a.iso,disk,a.out,'lua',a.resolution);t=g['text'];q=g['q']
 def type_line(line):
  assert len(line)<256,f'line too long for Spindle: {len(line)}'
  g['focus'](153);time.sleep(.5)
  # One key at a time, each confirmed by Spindle's echo, so a busy guest never sees a
  # burst it could drop (a lost shift release turns the rest of a line into capitals).
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
  s=len(t());type_line(f'run lua {args}')
  found=[g['wait'](pattern,s,600) for pattern in expect]
  e=g['wait'](rf'\[std\.exit\] target=(\d+) task=(\d+) status={status}\n',s,120)
  target,task=int(e[1]),int(e[2])
  lines=re.findall(r'\[std\.(?:out|err)\][^\n]*',t()[s:])
  s2=len(t());g['focus'](SURFACE[target]);time.sleep(.5);q.key('f11')
  g['wait'](rf'\[app\.private\.retired\] target={target} task={task} ',s2,180)
  return found,lines
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  report=[]
  _,l=run('-e print(2^100)',[r'\[std\.out\] 1\.2676506002282e\+30\n']);report+=l
  _,l=run('-e '+TABLES,[r'\[std\.out\] \[lua\.t\] 9,4,1 2 40  3\.14 "Lua 5\.4"\n']);report+=l
  _,l=run('-e '+FILES,[r'\[std\.out\] \[lua\.f\]\t35\t1970-01-02\tABC-ABC\tfalse\tboom\n']);report+=l
  (m,),l=run('t.lua x y',[r'\[std\.out\] \[lua\.s\]\t2\t(\d+)\tx\ty\n']);report+=l
  assert abs(int(m[1])-time.time())<600,f'os.time is off: {m[1]} vs {int(time.time())}'
  _,l=run("-e \"error('bad')\"",[r'\[std\.err\] [^\n]*bad'],status=1);report+=l
  text=t();assert 'KERNEL PANIC' not in text and '[std.abort]' not in text and 'KERNEL PAGE FAULT' not in text
  for line in report:print(line)
  print({'ok':True,'runs':5})
 finally:g['stop']()
if __name__=='__main__':main()
