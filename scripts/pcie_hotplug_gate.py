#!/usr/bin/env python3
"""PCIe native hot-plug: an NVMe drive is plugged into an empty PCIe root port after
boot (as a Thunderbolt dock or eGPU enclosure appears on its port), the kernel powers
the slot, numbers it, gives its BAR an address inside the port's window and names it.
It is then removed with the attention button (QEMU's device_del): after the 5 s
cancel window the slot is quieted and powered off and QEMU lets it go. A second drive
plugged into the same port arrives again."""
import argparse,os,re,shutil,time
from pathlib import Path
PORT='-device pcie-root-port,id=hp1,bus=pcie.0,chassis=7,slot=7,bus-reserve=3,mem-reserve=8M,pref64-reserve=32M'
# Native PCIe hot-plug on q35 root ports (QEMU 6.1+ defaults to ACPI hot-plug).
NATIVE='-global ICH9-LPC.acpi-pci-hotplug-with-bridge-support=off'
def qdev_ids(buses):
 ids=set()
 def walk(devices):
  for d in devices:
   ids.add(d.get('qdev_id'));walk(d.get('pci_bridge',{}).get('devices',[]))
 for b in buses:walk(b.get('devices',[]))
 return ids
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 plug=a.out/'plugged.img'
 with plug.open('wb') as f:f.truncate(64*1024**2)
 os.environ['SEXOS_GATE_DEVICES']=f'{PORT} {NATIVE}'
 os.environ.setdefault('SEXOS_GATE_BOUND','600')
 from quil_autosave_gate import boot
 g=boot(a.iso,disk,a.out,'pcie-hotplug',a.resolution);t=g['text'];q=g['q']
 try:
  port=g['wait'](r'\[hotplug\.port\] (\S+) \S+ kind=root slot=7 [^\n]*present=0 [^\n]*mem=(0x[0-9a-f]+)\+(0x[0-9a-f]+)',0,10)
  bdf,base,size=port[1],int(port[2],16),int(port[3],16)
  assert size>=8<<20,('port window',hex(size))
  q.cmd('blockdev-add',{'driver':'file','node-name':'hpfile','filename':str(plug)})
  q.cmd('blockdev-add',{'driver':'raw','node-name':'hpdisk','file':'hpfile'})
  def arrive(tag):
   s=len(t());q.cmd('device_add',{'driver':'nvme','drive':'hpdisk','serial':tag,'bus':'hp1','id':tag})
   m=g['wait'](rf'\[hotplug\.arrived\] port={re.escape(bdf)} functions=1 buses=([0-9a-f]+)-[0-9a-f]+ planned=1',s,30)
   bar=g['wait'](r'\[hotplug\.bar\] (\S+) bar=0 address=(0x[0-9a-f]+) bytes=(0x[0-9a-f]+)',s,5)
   assert bar[1].startswith(m[1]+':'),(bar[1],m[1])
   address,length=int(bar[2],16),int(bar[3],16)
   assert base<=address and address+length<=base+size,('BAR outside the port window',hex(address),hex(base),hex(size))
   g['wait'](r'\[hw\.device\] '+re.escape(bar[1])+r' 1b36:0010 class=010802',s,5)
   return bar[1]
  first=arrive('hpnvme1')
  # The attention button: 5 s to change one's mind, then quiet and power off.
  s=len(t());began=time.monotonic();q.cmd('device_del',{'id':'hpnvme1'})
  g['wait'](rf'\[hotplug\.button\] port={re.escape(bdf)} state=Button',s,10)
  g['wait'](rf'\[hotplug\.left\] port={re.escape(bdf)} functions_quieted=1',s,20)
  assert time.monotonic()-began>=4.5,'let go before the 5 s cancel window'
  g['wait'](rf'\[hotplug\.power\] port={re.escape(bdf)} on=0',s,5)
  for _ in range(100):
   if 'hpnvme1' not in qdev_ids(q.cmd('query-pci')):break
   time.sleep(.1)
  else:raise RuntimeError('QEMU kept the drive after the slot was powered off')
  # Plugged again: the slot is powered back on and the drive is numbered again.
  s=len(t());second=arrive('hpnvme2')
  g['wait'](rf'\[hotplug\.power\] port={re.escape(bdf)} on=1',s,5)
  assert second==first,(first,second)
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
 finally:g['stop']()
 print({'ok':True,'port':bdf,'device':first})
if __name__=='__main__':main()
