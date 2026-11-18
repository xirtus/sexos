#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Cross-domain isolation matrix, one boot per backend (kernel plan phase 7).

Program B (`run fuzz iso hold`) names a code address, a data page and a stack slot of
its own. Program A (`run fuzz iso OP ADDR`) then touches one of them. The kernel must
end A and nothing else; `[fuzz.iso.breach]` means the touch went through. Rows:

  own          A touches its own memory: the probe works (must pass everywhere)
  read_data    A reads B's data page
  write_data   A writes B's data page (B checks its byte at the end)
  read_stack   A reads B's stack
  read_code    A reads B's code
  exec_data    A jumps into B's data page (no-execute)
  exec_code    A calls B's code (a function that only returns)
  dead_read    A reads B's data page after B has ended (dead incarnation): faults, or
               A was given B's row and sees fresh memory, not B's byte
  wrpkru       a copy of the fuzz program with WRPKRU (0F 01 EF) patched into its
               code must be refused at load (`[app.program.rejected] reason=key_instruction`)
  revoked_grant, dead_badge
               pending: need authority v2 (stream 3) and IPC v2 badges (stream 4)

Backends: kvm-pku (`-cpu host -accel kvm`, only when /dev/kvm and the host has PKU),
tcg-pku (`max,+pku`) and tcg-nopku (`max,-pku`). Each cell has an expectation:
`fault` must fault, `gap` is a known hole recorded in the plan (it is reported either
way, and a gap that starts faulting says so, so the table can be tightened):
  * without PKU nothing separates programs in the one address space until protection
    views land (stream 6): every data row is a gap on tcg-nopku;
  * protection keys govern data access, not instruction fetch, so exec_code is a gap
    on every backend until views make B's code not present for A.
A `fault` cell that breaches fails the gate. Writes isolation.json and prints a table.

  isolation_matrix_gate.py --iso ISO --source-disk IMG --out DIR [--backends a,b]
"""
import argparse,json,os,re,shutil,time
from pathlib import Path
os.environ.setdefault('SEXOS_GATE_BOUND','2400')
from install_program import install
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
MARKER=bytes.fromhex('0f1f804b455948')  # 7-byte NOP whose operand spells KEYH
WRPKRU=bytes.fromhex('0f01ef0f1f4000')  # WRPKRU + 4-byte NOP, same length
SPINDLE=153

BACKENDS={
 'kvm-pku':dict(cpu='host',devices='-accel kvm',pku=True),
 'tcg-pku':dict(cpu='max,+pku,+rdseed',devices='',pku=True),
 'tcg-nopku':dict(cpu='max,-pku,+rdseed',devices='',pku=False),
}
ROWS=['own','read_data','write_data','read_stack','read_code','exec_data','exec_code','dead_read','wrpkru','revoked_grant','dead_badge']
PENDING={'revoked_grant':'authority v2 (stream 3)','dead_badge':'IPC v2 badges (stream 4)'}

def expect(row,backend):
 if row in PENDING:return 'pending'
 if row=='own':return 'ok'
 if row=='exec_code':return 'gap'
 if row in('wrpkru','exec_data','dead_read'):return 'fault'
 return 'fault' if BACKENDS[backend]['pku'] else 'gap'

def kvm_ready():
 if not os.access('/dev/kvm',os.R_OK|os.W_OK):return 'no usable /dev/kvm'
 flags=Path('/proc/cpuinfo').read_text()
 return None if re.search(r'^flags\s*:.*\bpku\b',flags,re.M) else 'host CPU has no PKU'

def patched(elf):
 assert elf.count(MARKER)==1,'fuzz program lost its KEYH marker (apps/fuzz iso_key_marker)'
 return elf.replace(MARKER,WRPKRU)

def run_backend(name,a,elf,out):
 cfg=BACKENDS[name];out.mkdir(parents=True,exist_ok=True)
 os.environ['SEXOS_GATE_CPU']=cfg['cpu'];os.environ['SEXOS_GATE_DEVICES']=cfg['devices']
 disk=out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 install(disk,[('fuzz',elf),('fuzzkey',patched(elf))])
 g=boot(a.iso,disk,out,'iso-'+name,a.resolution);t=g['text'];q=g['q']
 def type_run(line):
  g['focus'](SPINDLE);time.sleep(.5)
  for k in line:q.key('spc' if k==' ' else k);time.sleep(.05)
  q.key('ret')
 def close(target,task):
  surface=int(re.findall(rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',t())[-1])
  s=len(t());g['focus'](surface);time.sleep(.3);q.key('f11');g['wait'](rf'\[app.private.retired\] target={target} task={task} ',s,90)
 def probe(op,addr):
  s=len(t());type_run(f'run fuzz iso {op} {addr:x}')
  m=g['wait'](rf'\[fuzz.iso.try\] target=(\d+) task=(\d+) op={op} addr={addr:#x}',s,120);target,task=m[1],m[2]
  r=g['wait'](rf'(\[fuzz.iso.breach\] target={target} task={task} [^\n]*|fault.kill \[[^\]]*\] pd={target} [^\n]*)',s,120)
  if r[0].startswith('[fuzz.iso.breach]'):
   close(target,task);return 'breach',r[0]
  g['wait'](rf'\[app.private.retired\] target={target} ',s,90);return 'fault',r[0]
 cells={}
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  # own: the probe can touch its own memory.
  s=len(t());type_run('run fuzz iso own')
  m=g['wait'](r'\[fuzz.iso.own\] target=(\d+) task=(\d+) ok=(\d)',s,120)
  cells['own']=('ok' if m[3]=='1' else 'broken',m[0]);close(m[1],m[2])
  # B holds; A probes each of B's addresses.
  s=len(t());type_run('run fuzz iso hold')
  h=g['wait'](r'\[fuzz.iso.hold\] target=(\d+) task=(\d+) code=(0x[0-9a-f]+) data=(0x[0-9a-f]+) stack=(0x[0-9a-f]+)',s,120)
  b_target,b_task=h[1],h[2];code,data,stack=(int(h[i],16) for i in(3,4,5));time.sleep(1)
  for row,op,addr in[('read_data','read',data),('write_data','write',data),('read_stack','read',stack),
                     ('read_code','read',code),('exec_data','exec',data),('exec_code','exec',code)]:
   cells[row]=probe(op,addr)
   assert re.search(rf'fault.kill \[[^\]]*\] pd={b_target} ',t()) is None,f'{row}: B (pd {b_target}) was ended too'
  # B ends normally; its byte shows whether A's write landed.
  s=len(t());close(b_target,b_task)
  kept=re.search(rf'\[fuzz.iso.closed\] target={b_target} task={b_task} data_byte=(0x[0-9a-f]+)',t()[s-4096:])
  cells['b_data_byte']=('ok' if kept and kept[1]=='0x5a' else 'changed',kept[0] if kept else 'no closed record')
  # After B ends its row may be reused, even by A itself: then A reading B's old
  # address reads A's own fresh memory, and the rule is that B's byte (0x5a) is
  # gone. From any other row the read must fault.
  time.sleep(2);kind,ev=probe('read',data)
  if kind=='breach':
   m=re.search(r'target=(\d+) .* byte=(0x[0-9a-f]+)',ev)
   if m and m[1]==b_target and int(m[2],16)!=0x5a:kind='fresh'
  cells['dead_read']=(kind,ev)
  # A program whose code holds WRPKRU is refused at load.
  s=len(t());type_run('run fuzzkey')
  r=g['wait'](r'(\[app.program.rejected\] [^\n]*reason=key_instruction[^\n]*|\[shell.program.launch\] [^\n]*)',s,120)
  cells['wrpkru']=('fault' if 'key_instruction' in r[0] else 'breach',r[0])
  text=t()
  services=[p for p in re.findall(r'fault.kill \[[^\]]+\] pd=(\d+)',text) if int(p)<=15]
  assert not services,f'services faulted: {services}'
  assert 'KERNEL PANIC' not in text,'kernel stopped'
 finally:g['stop']()
 return cells

def verdict(row,backend,cells):
 want=expect(row,backend)
 if row=='dead_read' and want=='fault' and cells.get(row,('',''))[0]=='fresh':return want,'fresh',cells[row][1],True
 if want=='pending':return want,'pending',PENDING[row],True
 got,evidence=cells.get(row,('missing',''))
 if want=='ok':return want,got,evidence,got=='ok'
 if want=='fault':return want,got,evidence,got=='fault'
 return want,got,evidence,True  # gap: recorded either way

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path)
 p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800')
 p.add_argument('--backends',default='kvm-pku,tcg-pku,tcg-nopku');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True)
 elf=(ROOT/'iso_root/programs/fuzz').read_bytes();assert elf[:4]==b'\x7fELF'
 report={'rows':ROWS,'backends':{}};failed=[]
 for name in [b for b in a.backends.split(',') if b]:
  why=kvm_ready() if name=='kvm-pku' else None
  if why:report['backends'][name]={'skipped':why};print(f'[isolation.backend] {name} skipped: {why}');continue
  cells=run_backend(name,a,elf,a.out/name)
  table={}
  for row in ROWS:
   want,got,evidence,ok=verdict(row,name,cells)
   table[row]=dict(expect=want,got=got,ok=ok,evidence=evidence[:200])
   if not ok:failed.append(f'{name}/{row}: expected {want}, got {got}')
   if want=='gap' and got=='fault':print(f'[isolation.gap.closed] {name}/{row}: now faults; tighten the expectation')
  b=cells.get('b_data_byte',('missing',''))
  if b[0]!='ok' and BACKENDS[name]['pku']:failed.append(f'{name}/write_data: B saw its byte change ({b[1]})')
  table['b_data_byte']=dict(expect='ok',got=b[0],ok=b[0]=='ok' or not BACKENDS[name]['pku'],evidence=b[1][:200])
  report['backends'][name]=table
 (a.out/'isolation.json').write_text(json.dumps(report,indent=1))
 names=list(report['backends'])
 print('| row | '+' | '.join(names)+' |');print('|---|'+'---|'*len(names))
 for row in ROWS+['b_data_byte']:
  cols=[]
  for n in names:
   c=report['backends'][n]
   if 'skipped' in c:cols.append('skipped');continue
   x=c[row];mark='' if x['ok'] else ' **FAIL**'
   cols.append(f"{x['got']} (want {x['expect']}){mark}")
  print(f'| {row} | '+' | '.join(cols)+' |')
 assert not failed,'isolation matrix failed: '+'; '.join(failed)
 print({'ok':True})
if __name__=='__main__':main()
