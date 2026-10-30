#!/usr/bin/env python3
"""Exact BOOT command grant and read-only runtime query; no DMA/packet proof."""
import argparse,json,re
from pathlib import Path
from check_nic_discovery import inspect as discovery


def inspect(text, expected=1, preparation=False):
    errors=list(discovery(text,expected_leases=expected)['errors'])
    pattern=r'\[boot.nic.command\] pd=(\d+) slot=(\d+) bus=(\d+) device=(\d+) function=(\d+) vendor=(\d+) product=(\d+) allowed=(\d+) dma_region=(\d+) runtime_mint=0'
    grants=list(re.finditer(pattern,text));seal=text.find('[boot.memory.authority.sealed]')
    if len(grants)!=expected:errors.append('wrong exact BOOT command grant count')
    found=[(m.start(),tuple(int(v,16)for v in m.groups()))for m in re.finditer(r'\[kernel.pci.nic.found\] ([0-9a-f]+):([0-9a-f]+)\.([0-9a-f]+) vendor=([0-9a-f]+) device=([0-9a-f]+)',text,re.I)]
    for grant in grants:
        pd,slot,bus,dev,func,vendor,product,bits,dma=map(int,grant.groups())
        selected=next((g for position,g in reversed(found)if position<grant.start()),None)
        if (pd,slot,bits,dma)!=(13,19,6,2)or(vendor,product)not in[(0x8086,0x100e),(0x8086,0x10d3)]:errors.append('wrong holder/slot/mask/DMA/supported-device binding')
        if (bus,dev,func,vendor,product)!=selected:errors.append('command grant differs from actual admitted controller')
        if seal<0 or grant.start()>seal:errors.append('command genesis after seal')
    if '[pci.command.negative.selftest] denied=9 output_zero=1 runtime_mint=0 ok=1'not in text:errors.append('actual denied/zero-output selftest missing')
    applied=list(re.finditer(r'\[pci.command.apply\] pd=(\d+) slot=(\d+) before=(\d+) after=(\d+) set=(\d+) clear=(\d+) changed=(\d+)',text))
    ready=list(re.finditer(r'\[sexnet.nic.command.ready\] before=(\d+) after=(\d+) requested_bits=0 changed=0 denied=5 ok=1',text))
    if len(applied)!=(4*expected if preparation else expected) or len(ready)!=expected:errors.append('wrong actual runtime command invocation count')
    for query in applied[:expected] if preparation else applied:
        pd,slot,before,after,set_bits,clear,changed=map(int,query.groups())
        if (pd,slot,set_bits,clear,changed)!=(13,19,0,0,0)or before!=after or before>65535:errors.append('normal profile mutated NIC command or queried wrong subject')
        if seal<0 or query.start()<seal:errors.append('query did not traverse runtime after seal')
    if expected==1 and applied and ready:
        values=tuple(map(int,applied[0].groups()[2:4]));receipt=tuple(map(int,ready[0].groups()))
        if values!=receipt or ready[0].start()<applied[0].start():errors.append('service receipt differs from actual query')
    if expected==0 and '[sexnet.nic.command.unavailable] status=18446744073709551615 requested_bits=0 denied=5 ok=1'not in text:errors.append('missing unsupported/absent-capability denial')
    phases=list(re.finditer(r'\[sexnet.nic.prepare\] from=(\w+) to=(\w+) failure=(\w+) status=(\d+) mac=([0-9a-f]{12}) dma=0 packets=0',text))
    if preparation:
        if text.count('[sexnet.nic.prepare]')!=len(phases):errors.append('malformed or failed preparation record')
        if text.count('[pci.command.apply]')!=len(applied):errors.append('malformed command receipt')
        if expected==0:
            if phases or '[sexnet.nic.prepare.unavailable] status=18446744073709551615 dma=0 packets=0'not in text:errors.append('absent lease did not fail closed before MMIO')
        else:
            if [m.groups()[:3]for m in phases]!=[(a,b,'None')for a,b in [('Cold','Drain'),('Drain','Reset'),('Reset','Verify'),('Verify','Quiescent')]]:errors.append('missing, repeated or failed bounded preparation transition')
            if len(applied)==4 and len(phases)==4 and len(ready)==1:
                values=[tuple(map(int,m.groups()))for m in applied];first,query,mutation,verify=values
                for record,m in zip(values,applied):
                    if record[:2]!=(13,19)or min(record[2:4])<0 or max(record[2:4])>65535 or m.start()<seal:errors.append('preparation command holder/width/phase mismatch')
                if query!=(13,19,first[3],first[3],0,0,0)or query[3]&4:errors.append('device was already mastering or initial query changed')
                if mutation!=(13,19,query[3],query[3]|2,2,4,int(query[3]!=(query[3]|2))):errors.append('preparation mutation differs from exact memory-on/master-off request')
                if verify!=(13,19,mutation[3],mutation[3],0,0,0):errors.append('post-reset command differs or enables bus mastering')
                if not applied[0].start()<ready[0].start()<applied[1].start()<applied[2].start()<phases[0].start()<phases[1].start()<phases[2].start()<applied[3].start()<phases[3].start():errors.append('preparation command/transition order differs')
                for m in phases[:-1]:
                    if m.group(5)!='000000000000':errors.append('station address published before final verification')
                mac=bytes.fromhex(phases[-1].group(5))
                if not any(mac)or mac[0]&1 or int(phases[-1].group(4))==4294967295:errors.append('invalid actual station address/status')
            if '[sexnet.nic.prepare.unavailable]'in text:errors.append('admitted NIC unexpectedly unavailable')
        if re.search(r'\[sexnet.nic.prepare[^\r\n]*dma=[1-9]|\[sexnet.nic.prepare[^\r\n]*packets=[1-9]',text):errors.append('unqualified DMA or packet activity')
    return dict(ok=not errors,errors=errors,grants=[list(map(int,g.groups()))for g in grants],
                runtime_queries=sum(m.group(5)=='0'and m.group(6)=='0'for m in applied),command_invocations=len(applied),
                preparation=preparation,transitions=[m.group(0)for m in phases],
                scope=('Normal-profile exact BOOT lease, Ring3 memory-on/master-off command, bounded reset/quiescence and actual station address. No DMA, packets, firmware DMA-drain, physical hardware or hostile containment proof.'if preparation else 'Normal-profile BOOT-only command grant and actual Ring3 zero-mutation query/denials. Masked writes/status preservation are exercised with port mocks; live DMA, packets, hotplug, physical hardware and hostile containment are not established.'))


def main():
    p=argparse.ArgumentParser();p.add_argument('log',type=Path);p.add_argument('--expected',type=int,choices=[0,1],default=1);p.add_argument('--preparation',action='store_true');a=p.parse_args()
    r=inspect(a.log.read_text(errors='replace'),a.expected,a.preparation);print(json.dumps(r,indent=2));return not r['ok']


if __name__=='__main__':raise SystemExit(main())
