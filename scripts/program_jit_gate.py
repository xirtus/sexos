#!/usr/bin/env python3
"""Programs can make machine code while they run, the base a JIT engine needs: `run jit`
compiles an expression into its heap, seals it as code and runs it, matching an
interpreter. Code hiding WRPKRU is refused and re-encoded; code is opened, patched and
sealed again. Writing to sealed code and running open data each end only that run,
through a page fault, and the row runs again afterwards. A program file whose own code
holds WRPKRU is refused at load."""
import argparse,re,shutil,struct,time
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
SURFACE={int(a):int(b)for a,b in re.findall(r'app: App::Program, ordinal: \d+, target: (\d+), surface: (\d+)',(ROOT/'crates/sex-pdx/src/app_catalog.rs').read_text())}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 elf=(ROOT/'iso_root/programs/jit').read_bytes();assert elf[:4]==b'\x7fELF'
 # The same program with WRPKRU written into its code: the loader must refuse it.
 bad=bytearray(elf);phoff,=struct.unpack_from('<Q',bad,32);n,=struct.unpack_from('<H',bad,56)
 text=next((off,size)for kind,flags,off,_,_,size,*_ in(struct.unpack_from('<IIQQQQQQ',bad,phoff+i*56)for i in range(n))if kind==1 and flags&1)
 at=text[0]+text[1]//2;bad[at:at+3]=b'\x0f\x01\xef'
 install(disk,[('jit',elf),('jitbad',bytes(bad))])
 g=boot(a.iso,disk,a.out,'jit',a.resolution);t=g['text'];q=g['q']
 def run(line):
  g['focus'](153);time.sleep(.5)
  for k in line:q.key('spc' if k==' ' else k);time.sleep(.05)
  q.key('ret')
 def started():
  s=len(t());run('run jit')
  m=g['wait'](r'\[jit\.(ok|fail)\] target=(\d+) task=(\d+)([^\n]*)\n',s,300)
  assert m[1]=='ok',f'JIT test failed: {m[0]}'
  target,task=int(m[2]),int(m[3]);g['wait'](rf'\[jit\.ready\] target={target} task={task}\n',s,60)
  seals=re.findall(rf'\[app\.program\.code\] target={target} task={task} op=(seal|open) va=(0x[0-9a-f]+) pages=(\d+)\n',t()[s:])
  assert ('seal' in {o for o,*_ in seals}) and ('open' in {o for o,*_ in seals}),f'no seal/open evidence: {seals}'
  blocked=re.search(r'\[jit\.blocked\] offset=(\d+)\n',t()[s:]);assert blocked,'WRPKRU was not refused'
  speed=dict(kv.split('=')for kv in re.search(r'\[jit\.speed\] ([^\n]*)\n',t()[s:])[1].split())
  return dict(target=target,task=task,speedup=int(speed['speedup']),seals=len(seals),blocked=int(blocked[1]))
 def fault(r,key,mark):
  s=len(t());g['focus'](SURFACE[r['target']]);time.sleep(.5);q.key(key)
  g['wait'](rf'\[jit\.{mark}\] target={r["target"]} task={r["task"]}\n',s,60)
  g['wait'](rf'fault\.kill \[paging\.fault\.terminal\] pd={r["target"]} ',s,120)
  g['wait'](rf'\[app\.private\.retired\] target={r["target"]} task={r["task"]} ',s,120)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  first=started();g['wait'](rf'\[display.text.committed\] target={first["target"]} ',0,60);time.sleep(1)
  q.cmd('screendump',{'filename':str(a.out/'jit.ppm')})
  fault(first,'w','write_code')
  second=started();fault(second,'e','run_data')
  s=len(t());run('run jitbad')
  g['wait'](r'\[app\.program\.rejected\] target=\d+ bytes=\d+ reason=key_instruction page=\d+ offset=0x[0-9a-f]+\n',s,120)
  third=started()
  s=len(t());g['focus'](SURFACE[third['target']]);time.sleep(.5);q.key('f11')
  g['wait'](rf'\[jit\.closed\] target={third["target"]} task={third["task"]}\n',s,60)
  text=t();assert 'KERNEL PANIC' not in text and '[jit.fatal]' not in text and '[jit.fail]' not in text and 'KERNEL PAGE FAULT' not in text
  print({'ok':True,'runs':[first,second,third]})
 finally:g['stop']()
if __name__=='__main__':main()
