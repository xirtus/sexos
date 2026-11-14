#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""bat 0.26.1 (apps/bat) runs on SexOS.

Everything is typed into Spindle, exactly as a person would:

* `run bat --version`: bat starts, and Files makes its folder /data/bat.
* Spindle's own `write` and `append` put a three-line Rust file, hello.rs, there.
* `run bat hello.rs`: bat's header names the file and every line comes numbered, in
  bat's grid; the program's window is a terminal that shows no colours, so the
  output holds no escape sequences.
* `run bat --list-languages` lists Rust with its extension rs.
* `run bat -p hello.rs` prints the file plain, as it is.
* `run bat missing.rs` prints bat's error on standard error and exits with status 1.

Output must reach the serial record as `[std.out]` lines; each run must exit with
the right status and close."""
import argparse,re,shutil,time
from pathlib import Path
from install_program import install
import os
os.environ.setdefault('SEXOS_GATE_BOUND','1800')
from quil_autosave_gate import boot
from lua_gate import qcodes,SURFACE
ROOT=Path(__file__).resolve().parents[1]

# fs.rs request kinds, as `[spindle.fs.result]` names them.
WRITE=8
SOURCE=['fn main() {','let answer = 42;','}']
FILE=[('write',f'/data/bat/hello.rs {SOURCE[0]}')]+[('append',f'/data/bat/hello.rs {line}')for line in SOURCE[1:]]

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 elf=(ROOT/'iso_root/programs/bat').read_bytes();assert elf[:4]==b'\x7fELF'
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 install(disk,[('bat',elf)])
 g=boot(a.iso,disk,a.out,'bat',a.resolution);t=g['text'];q=g['q']
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
 def shell(cmd,rest):
  s=len(t());type_line(f'{cmd} {rest}')
  g['wait'](rf'\[spindle\.fs\.result\] kind={WRITE} status=0 ',s,60)
 def run(args,status=0):
  s=len(t());type_line(f'run bat {args}')
  e=g['wait'](r'\[std\.exit\] target=(\d+) task=(\d+) status=(\d+)\n',s,600)
  target,task=int(e[1]),int(e[2])
  # The serial record holds whole lines: everything before the exit is in.
  done=t()[s:s+e.start()]
  out=re.findall(r'\[std\.out\] ([^\n]*)\n',done)
  err=re.findall(r'\[std\.err\] ([^\n]*)\n',done)
  # A run that printed nothing opened no window and ends by itself; otherwise its
  # window stays until the person closes it.
  if out or err:
   s2=len(t());g['focus'](SURFACE[target]);time.sleep(.5);q.key('f11')
  else:s2=s
  g['wait'](rf'\[app\.private\.retired\] target={target} task={task} ',s2,180)
  print(f'$ run bat {args}  (status {e[3]})',flush=True)
  shown=out if len(out)<=12 else out[:6]+[f'... ({len(out)-12} more lines)']+out[-6:]
  for line in shown:print('  '+line,flush=True)
  for line in err:print('  stderr: '+line,flush=True)
  assert int(e[3])==status,f'run bat {args}: status {e[3]}, want {status}'
  # The window shows no colours: bat must not send escape sequences to it.
  assert not any('\x1b' in line for line in out+err),f'run bat {args}: escape sequences in the output'
  return out,err
 def same(got,want,what):
  assert got==want,f'{what}: got {got!r}, want {want!r}'
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  out,err=run('--version');same(out,['bat 0.26.1'],'bat --version');same(err,[],'bat --version stderr')
  for cmd,rest in FILE:shell(cmd,rest)
  # Decorations: the header names the file, then the numbered lines, in bat's grid.
  out,err=run('hello.rs');same(err,[],'bat FILE stderr')
  assert any(re.fullmatch(r'\s*│ File: hello\.rs',line) for line in out),f'bat FILE: no header naming hello.rs in {out!r}'
  numbered=[(int(m[1]),m[2]) for m in (re.fullmatch(r'\s*(\d+)\s+│ (.*)',line) for line in out) if m]
  same(numbered,list(enumerate(SOURCE,1)),'bat FILE numbered lines')
  out,err=run('--list-languages');same(err,[],'bat --list-languages stderr')
  rust=[line for line in out if re.match(r'Rust\s',line)]
  assert rust and re.search(r'\brs\b',rust[0]),f'bat --list-languages: no Rust (rs) line among {len(out)} lines'
  out,err=run('-p hello.rs');same(out,SOURCE,'bat -p');same(err,[],'bat -p stderr')
  out,err=run('missing.rs',status=1);same(out,[],'bat missing file stdout')
  assert len(err)==1 and err[0].startswith('[bat error]: ') and 'missing.rs' in err[0],f'bat missing file: {err!r}'
  text=t();assert 'KERNEL PANIC' not in text and '[std.abort]' not in text and 'KERNEL PAGE FAULT' not in text
  print({'ok':True,'runs':5})
 finally:g['stop']()
if __name__=='__main__':main()
