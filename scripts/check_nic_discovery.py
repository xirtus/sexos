#!/usr/bin/env python3
"""Normal-profile PCI discovery/lease evidence; does not prove a network stack."""
import argparse,json,re
from pathlib import Path

def inspect(text,expected_leases=1):
 errors=[]
 found=re.findall(r'\[pci.net.device\] vendor=(0x[0-9A-Fa-f]+) device=(0x[0-9A-Fa-f]+) ',text)
 passive=re.findall(r'\[pci.net.passive\] vendor=(0x[0-9A-Fa-f]+) device=(0x[0-9A-Fa-f]+) config_writes=0 mmio=0 rings=0 dma=0 packets=0 owner=boot_assigned_sexnet ok=1',text)
 if not found or found!=passive:errors.append('each actual network device needs exact passive discovery')
 if re.search(r'\[e1000\.|\[pci.net.command.enable\]|\[pci.net.legacy_diagnostics\]|\[sexnet.proof.profile\]',text):errors.append('default image activated historical NIC diagnostics')
 leases=re.findall(r'\[kernel.cap.nic.grant\] pd=13 slot=19(?:\s|$)',text)
 if len(leases)!=expected_leases:errors.append('wrong BOOT NIC lease count for exact SexNet subject')
 if '[sexnet.passive.ready.fast] network=0 dns=0 tcp=0 http=0 tls=0 ok=1'not in text:errors.append('normal service not honestly passive')
 if re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at',text):errors.append('fatal guest fault')
 return dict(ok=not errors,errors=errors,devices=found,scope='Normal enumeration has no NIC config/MMIO/ring/DMA side effects; subsequent BOOT BAR sizing/lease is distinct. No packet, physical-hardware or hostile-containment proof.')

def main():
 p=argparse.ArgumentParser();p.add_argument('log',type=Path);a=p.parse_args();r=inspect(a.log.read_text(errors='replace'));print(json.dumps(r,indent=2));return not r['ok']
if __name__=='__main__':raise SystemExit(main())
