#!/usr/bin/env python3
"""Drag D1 (syscall 110, sex_pdx::vm): `run dragtest` makes a VM from one of its own heap
pages and runs a 16-bit real-mode guest that writes "Drag" to port 0xE9 and halts.

QEMU's TCG does not emulate VMX, so what this gate proves depends on the accelerator:
- tcg (CI, and any host without a usable /dev/kvm): the kernel logs
  `[drag] vmx=unavailable reason=...` on the first SYS_VM call, the program gets a clean
  UNAVAILABLE refusal for INFO and CREATE, the kernel stays up, and the program closes
  and its row retires as usual.
- kvm (`--accel kvm`, or auto when /dev/kvm is usable and the host CPU has vmx and pku):
  QEMU runs with `-accel kvm -cpu host` (nested VMX), and the guest path must run: four
  I/O exits on port 0xE9 with D, r, a, g, `[drag.guest] said=Drag`, HLT, `[drag.done]`.
D2 (`run drag`, the drag program on crates/drag-vmm) runs next in the same boot. It loads
the in-tree test guest (crates/drag-vmm/guest, a bzImage) through the Linux boot protocol
into 8 MiB of its own heap and makes `drag-test.img` in its folder for virtio-blk:
- tcg: `[drag.vmm.loaded] guest=test protocol=0x20f entry=64 rip=0x100200`, then the
  same clean refusal (`[drag.vmm.refused] status=3`), and the program closes as usual.
- kvm: the guest's serial console lines come back as `[drag.serial] [guest] ...`: its
  banner, the zero page and command line it was given, the disk's first line read over
  virtio-blk, its write to sector 1 read back, `done`; then `[drag.vmm.stop]
  reason=halted` and `[drag.disk] sector1=...` shows the write reached the file.
Crossdress CW1 (`run crossdress`, crates/sex-crossdress) runs last: a 64-bit Windows
program (the built-in test .exe) loaded into a Drag cell, a SYS_VM guest with no OS:
- tcg: `[crossdress.loaded] base=0x140000000 ... imports=7` (the PE was parsed and the
  cell built inside SexOS), then `[crossdress.refused] status=3`.
- kvm: the program's console lines come back as `[crossdress.out] ...`, ending with
  `crossdress: all checks passed`, and `[crossdress.stop] reason=exited code=0x0 ...
  handled=5` (five hardware exceptions through its vectored handler).
Then `run crossdress plugin`: the built-in Windows CLAP plugin DLL (test/gain.c) in a cell:
- tcg: `[crossdress.clap.loaded] base=0x180000000 ... imports=3`, then the refusal.
- kvm: `[crossdress.clap.open] id=org.sexos.crossdress.test-gain ...` and
  `[crossdress.clap.done] frames=1280 exact=1 ... reported=2` (its audio bit for bit).
`--require-guest` fails unless the guest paths ran."""
import argparse,os,re,shutil,time
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
# drag-vmm's DEFAULT_CMDLINE, which the drag program gives every guest first.
CMDLINE=re.search(r'DEFAULT_CMDLINE: &\[u8\] = b"([^"]*)"',(ROOT/'crates/drag-vmm/src/lib.rs').read_text())[1]

def kvm_usable():
 try:flags=Path('/proc/cpuinfo').read_text()
 except OSError:return False
 return os.access('/dev/kvm',os.R_OK|os.W_OK) and re.search(r'^flags.*\bvmx\b',flags,re.M) is not None and re.search(r'^flags.*\bpku\b',flags,re.M) is not None

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800')
 p.add_argument('--accel',choices=('auto','tcg','kvm'),default='auto');p.add_argument('--require-guest',action='store_true');a=p.parse_args()
 accel=a.accel if a.accel!='auto' else ('kvm' if kvm_usable() else 'tcg')
 if accel=='kvm':
  os.environ['SEXOS_GATE_CPU']='host'
  os.environ['SEXOS_GATE_DEVICES']=(os.environ.get('SEXOS_GATE_DEVICES','')+' -accel kvm').strip()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 elf=ROOT/'iso_root/programs/dragtest';assert elf.read_bytes()[:4]==b'\x7fELF'
 drag=ROOT/'iso_root/programs/drag';assert drag.read_bytes()[:4]==b'\x7fELF'
 crossdress=ROOT/'iso_root/programs/crossdress';assert crossdress.read_bytes()[:4]==b'\x7fELF'
 install(disk,[('dragtest',elf.read_bytes()),('drag',drag.read_bytes()),('crossdress',crossdress.read_bytes())])
 g=boot(a.iso,disk,a.out,'drag',a.resolution);t=g['text'];q=g['q']
 def run(line):
  g['focus'](153);time.sleep(.5)
  for k in line:q.key('spc' if k==' ' else k);time.sleep(.05)
  q.key('ret')
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,180)
  assert '[drag] vmx=' not in t(),'the kernel touched VMX before any program asked'
  s=len(t());run('run dragtest')
  start=g['wait'](r'\[drag\.start\] target=(\d+) task=(\d+)',s,180);target,task=int(start[1]),int(start[2])
  end=g['wait'](r'\[drag\.(done|refused\.create|fail)\][^\n]*\n',s,180)
  text=t()[s:]
  assert end[1]!='fail',f'Drag test failed: {end[0].strip()}'
  if end[1]=='done':
   values=[int(v,16) for v in re.findall(r'\[drag\.exit\] reason=io port=0xe9 value=(0x[0-9a-f]+)',text)]
   assert bytes(values)==b'Drag',f'guest wrote {bytes(values)!r} to port 0xE9'
   assert '[drag.guest] said=Drag' in text and '[drag.exit] reason=hlt' in text,'guest did not finish with HLT'
   assert re.search(r'\[drag\] vmx=available ',text),'kernel did not report VMX'
   assert re.search(r'\[drag\.vm\] pd=\d+ vm=\d+ op=destroy pages=1 ',text),'VM was not destroyed with its one page'
   path='guest'
  else:
   assert accel=='tcg' or not a.require_guest,'VMX was expected to work under KVM'
   assert re.search(r'\[drag\] vmx=unavailable reason=\w+',text),'kernel did not report why VMX is unavailable'
   assert '[drag.refused] status=3 ' in text and '[drag.refused.create] status=3' in text,'refusal was not UNAVAILABLE'
   path='refused'
  if a.require_guest:assert path=='guest','--require-guest: the guest did not run'
  # The program closes and its row retires as usual: the kernel is still well.
  surface=int(re.findall(rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',t())[-1])
  s=len(t());g['focus'](surface);time.sleep(.3);q.key('f11')
  g['wait'](rf'\[app.private.retired\] target={target} task={task} ',s,120)
  # D2: the drag program and its test guest.
  s=len(t());run('run drag')
  start=g['wait'](r'\[drag\.vmm\.start\] target=(\d+) task=(\d+)',s,180);target,task=int(start[1]),int(start[2])
  g['wait'](r'\[drag\.vmm\.loaded\] guest=test protocol=0x20f entry=64 rip=0x100200 [^\n]*disk=128\n',s,180)
  end=g['wait'](r'\[drag\.vmm\.(done|refused|fail)\][^\n]*\n',s,240)
  text=t()[s:]
  assert end[1]!='fail',f'drag failed: {end[0].strip()}'
  if end[1]=='done':
   serial=re.findall(r'\[drag\.serial\] ([^\n]*)',text)
   want=['[guest] Drag D2 test guest: 64-bit entry through the Linux boot protocol','[guest] boot_params ok, e820 entries 0x4',
    '[guest] cmdline: '+CMDLINE+' virtio_mmio.device=4K@0xd0000000:5','[guest] virtio-blk at 0xd0000000 sectors 0x80',
    '[guest] disk: Drag D2 disk: hello from a file the drag program was granted.','[guest] disk write and read back ok','[guest] done']
   assert serial==want,f'guest console: {serial!r}'
   assert re.search(r'\[drag\.vmm\.stop\] reason=halted ',text),'guest did not halt'
   assert '[drag.disk] sector1=drag-d2 guest wrote sector one.' in text,'the guest write did not reach the file'
   d2='guest'
  else:
   assert '[drag.vmm.refused] status=3 ' in text,'drag refusal was not UNAVAILABLE'
   assert path=='refused','dragtest ran a guest but drag was refused'
   d2='refused'
  if a.require_guest:assert d2=='guest','--require-guest: the D2 guest did not run'
  surface=int(re.findall(rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',t())[-1])
  s=len(t());g['focus'](surface);time.sleep(.3);q.key('f11')
  g['wait'](rf'\[app.private.retired\] target={target} task={task} ',s,120)
  # Crossdress CW1: a Windows program in a Drag cell.
  s=len(t());run('run crossdress')
  start=g['wait'](r'\[crossdress\.start\] target=(\d+) task=(\d+)',s,180);target,task=int(start[1]),int(start[2])
  g['wait'](r'\[crossdress\.loaded\] base=0x140000000 entry=0x14000[0-9a-f]+ imports=7 ',s,180)
  end=g['wait'](r'\[crossdress\.(stop|refused|fail)\][^\n]*\n',s,240)
  text=t()[s:]
  assert end[1]!='fail',f'crossdress failed: {end[0].strip()}'
  if end[1]=='stop':
   out=re.findall(r'\[crossdress\.out\] ([^\n]*)',text)
   assert 'crossdress: hello from a Windows program' in out,f'program output: {out!r}'
   assert not [l for l in out if l.startswith('FAIL')],f'program checks failed: {out!r}'
   assert any(l.startswith('crossdress: all checks passed') for l in out),f'program output: {out!r}'
   assert re.search(r'\[crossdress\.stop\] reason=exited code=0x0 [^\n]*handled=5',text),end[0]
   cw1='guest'
  else:
   assert '[crossdress.refused] status=3 ' in text,'crossdress refusal was not UNAVAILABLE'
   assert path=='refused','dragtest ran a guest but crossdress was refused'
   cw1='refused'
  if a.require_guest:assert cw1=='guest','--require-guest: the Windows program did not run'
  surface=int(re.findall(rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',t())[-1])
  s=len(t());g['focus'](surface);time.sleep(.3);q.key('f11')
  g['wait'](rf'\[app.private.retired\] target={target} task={task} ',s,120)
  # Crossdress plugin bridge: a Windows CLAP plugin DLL in a cell, driven by the host.
  s=len(t());run('run crossdress plugin')
  start=g['wait'](r'\[crossdress\.start\] target=(\d+) task=(\d+)',s,180);target,task=int(start[1]),int(start[2])
  g['wait'](r'\[crossdress\.clap\.loaded\] base=0x180000000 entry=0x18000[0-9a-f]+ imports=3 ',s,180)
  end=g['wait'](r'\[crossdress\.(clap\.done|refused|fail)\][^\n]*\n',s,240)
  assert end[1]!='fail',f'crossdress plugin failed: {end[0].strip()}'
  if end[1]=='clap.done':
   text=t()[s:]
   assert '[crossdress.clap.open] id=org.sexos.crossdress.test-gain name=Crossdress Test Gain ' in text,text[-2000:]
   assert re.search(r'\[crossdress\.clap\.done\] frames=1280 exact=1 [^\n]*reported=2 ',text),end[0]
   assert '[crossdress.fail]' not in text,end[0]
   clap='guest'
  else:
   assert path=='refused','dragtest ran a guest but the crossdress plugin was refused'
   clap='refused'
  if a.require_guest:assert clap=='guest','--require-guest: the Windows plugin did not run'
  surface=int(re.findall(rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',t())[-1])
  s=len(t());g['focus'](surface);time.sleep(.3);q.key('f11')
  g['wait'](rf'\[app.private.retired\] target={target} task={task} ',s,120)
  text=t();assert 'KERNEL PANIC' not in text and '[drag.fatal]' not in text and '[crossdress.fatal]' not in text and 'panicked' not in text
  reason=re.search(r'\[drag\] vmx=(\w+)(?: reason=(\w+))?',text)
  print({'ok':True,'accel':accel,'path':path,'d2':d2,'crossdress':cw1,'clap':clap,'vmx':reason[1],'reason':reason[2]})
 finally:g['stop']()
if __name__=='__main__':main()
