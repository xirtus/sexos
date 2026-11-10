#!/usr/bin/env python3
"""Driver DMA through the IOMMU: SexOS on a QEMU machine with Intel VT-d
(default) or AMD-Vi (--vendor amd).

1. Files on disk (filesystem_gate) with VT-d and QEMU's `edu` DMA test device:
   the kernel gives each driver service its own DMA domain, the BOOT self-test
   proves an unleased device is blocked, translated DMA lands, a read-only
   page holds and an unmapped address faults; NVMe reads/writes and the
   power-cut reboot all run through sexdrive's domain.
2. Sound (sound_gate) with VT-d: virtio sound DMA runs through Bell's domain
   and the notes come out of QEMU at the right pitch.
Neither run may show a DMA fault from a real driver.
"""
import argparse,os,re,subprocess,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
IOMMU={'intel':'-device intel-iommu,aw-bits=48','amd':'-device amd-iommu'}

def run(script,out,extra,a):
 env=dict(os.environ,SEXOS_GATE_IOMMU=f'{IOMMU[a.vendor]} {extra}'.strip())
 cmd=[sys.executable,str(ROOT/'scripts'/script),'--iso',str(a.iso.resolve()),'--source-disk',str(a.source_disk.resolve()),'--out',str(out.resolve())]
 r=subprocess.run(cmd,env=env,cwd=ROOT/'scripts',capture_output=True,text=True)
 print(r.stdout[-2000:]);print(r.stderr[-4000:],file=sys.stderr)
 assert r.returncode==0,f'{script} failed behind the {a.vendor} IOMMU'
 logs=sorted(out.glob('*serial.log'))
 assert logs,f'{script} left no serial log'
 return [p.read_text(errors='replace') for p in logs]

NVME=('NVMe',r'\[kernel.pci.nvme.found\] (\S+) ')
XHCI=('xHCI',r'Discovered USB XHCI \((\S+)\)')
SOUND=('virtio sound',r'\[kernel.pci.virtio.found\] (\S+) product=0x1059')

def check(text,services,vendor):
 enabled=re.search(rf'\[iommu.enabled\] vendor={vendor} units=(\d+) enabled=(\d+) domains=(\d+) devices=(\d+) levels=(\d)[^\n]*\n',text)
 assert enabled and int(enabled[2])>=1,f'the {vendor} IOMMU was not enabled'
 for name,found in services:
  m=re.search(found,text);assert m,f'{name} was not found on the bus'
  bdf=re.escape(m[1])
  assert re.search(rf'\[iommu.device\] {bdf} pd=\d+ did=\d+ unit=\d+ translation=domain\n',text),f'{name} ({bdf}) has no DMA domain'
 faults=[l for l in re.findall(r'\[iommu.fault\] ([^\n]*)\n',text) if 'owner_pd=selftest' not in l]
 assert not faults,f'driver DMA faulted: {faults[:4]}'
 return enabled.groups()

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--vendor',choices=sorted(IOMMU),default='intel');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True)
 files=run('filesystem_gate.py',a.out/'files','-device edu',a)
 first=files[0]
 m=re.search(r'\[iommu.selftest\] device=\S+ blocked_unleased=1 unleased_faults=\d+ translated=1 read_only_held=1 unmapped_blocked=1 ok=1\n',first)
 assert m,'IOMMU self-test did not pass'
 summary={'files':[check(t,[NVME,XHCI],a.vendor) for t in files]}
 for t in files:
  assert re.search(r'\[sexdrive.dma.loan\] slot=17 active=1 domain=\d+ device_address=0x[0-9a-f]+ pages=32 reads=zero_copy\n',t),'sexdrive did not borrow the block buffer'
  assert re.search(r'\[sexdrive.read.zero_copy\] first=1 ',t),'no read went straight into the loan'
 sound=run('sound_gate.py',a.out/'sound','',a)
 summary['sound']=[check(t,[SOUND],a.vendor) for t in sound]
 print({'ok':True,'vendor':a.vendor,**summary})

if __name__=='__main__':main()
