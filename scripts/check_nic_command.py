#!/usr/bin/env python3
"""Exact BOOT command grant and read-only runtime query; no DMA/packet proof."""
import argparse,json,re
from pathlib import Path
from check_nic_discovery import inspect as discovery


def inspect(text, expected=1, preparation=False, ethernet=False):
    preparation=preparation or ethernet
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
    count=6 if ethernet else 4 if preparation else 1
    if len(applied)!=count*expected or len(ready)!=expected:errors.append('wrong actual runtime command invocation count')
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
            if len(applied)==count and len(phases)==4 and len(ready)==1:
                values=[tuple(map(int,m.groups()))for m in applied[:4]];first,query,mutation,verify=values
                for record,m in zip(values,applied[:4]):
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
    identities=list(re.finditer(r'\[pci.device.identity\] pd=(\d+) slot=(\d+) vendor=(\d+) product=(\d+) runtime_mint=0',text))
    if ethernet:
        if text.count('[pci.device.identity]')!=len(identities):errors.append('malformed typed identity receipt')
        if '[sexnet.nic.identity.denied] requests=4 output_zero=1 ok=1'not in text:errors.append('actual Ring3 identity denials missing')
        if len(identities)!=expected:errors.append('wrong typed BOOT identity receipt count')
        driver=list(re.finditer(r'\[sexnet.ether.phase\] from=(\w+) to=(\w+) failure=(\w+)',text))
        if text.count('[sexnet.ether.phase]')!=len(driver):errors.append('malformed or failed Ethernet transition')
        if expected:
            if [m.groups()for m in driver]!=[(a,b,'None')for a,b in [('Check','Zero'),('Zero','Descriptors'),('Descriptors','Registers'),('Registers','Enable'),('Enable','Link'),('Link','Running')]]:errors.append('missing/failed/repeated Ethernet initialization')
            if len(identities)==1 and len(grants)==1:
                holder,slot,vendor,product=map(int,identities[0].groups());g=list(map(int,grants[0].groups()))
                if (holder,slot,vendor,product)!=(g[0],g[1],g[5],g[6])or identities[0].start()<seal:errors.append('identity differs from held BOOT lease')
                if phases and identities[0].start()>phases[0].start():errors.append('device model selected after preparation began')
                model={0x100e:'I82540',0x10d3:'I82574'}.get(product,'INVALID')
                marker=rf'\[sexnet.ether.ready\] model={model} region=2 budget=262144 used=40960 descriptors=8 buffer=2048 polling=cooperative timing_verified=([01]) network=0 dns=0 tcp=0 http=0 tls=0'
                readiness=list(re.finditer(marker,text))
                if len(readiness)!=1:errors.append('missing exact model/budget/offline runtime readiness')
                elif model=='I82574'and readiness[0].group(1)!='1':errors.append('82574 timing register was not verified')
            if len(applied)==6 and len(driver)==6 and len(phases)==4:
                before=tuple(map(int,applied[3].groups()));query=tuple(map(int,applied[4].groups()));enable=tuple(map(int,applied[5].groups()))
                if query!=(13,19,before[3],before[3],0,0,0)or query[3]&6!=2:errors.append('DMA initialization did not start from memory-on/master-off')
                if enable!=(13,19,query[3],query[3]|4,4,0,1):errors.append('wrong actual bus-master transition')
                if not phases[-1].start()<applied[4].start()<driver[0].start()<driver[3].start()<applied[5].start()<driver[4].start()<driver[5].start():errors.append('bus-master enable/initialization order differs')
            if '[sexnet.ether.send.failed]'in text:errors.append('normal Ethernet submission failed')
        elif driver or '[sexnet.ether.ready]'in text or '[sexnet.ether.tx.submit]'in text:errors.append('absent lease reached DMA/packet path')
    return dict(ok=not errors,errors=errors,grants=[list(map(int,g.groups()))for g in grants],
                runtime_queries=sum(m.group(5)=='0'and m.group(6)=='0'for m in applied),command_invocations=len(applied),
                preparation=preparation,ethernet=ethernet,transitions=[m.group(0)for m in phases],
                scope=('Exact BOOT identity/command custody, reset, fixed DMA queue initialization and actual bus-master transition. Wire bytes require independent peer capture. No online protocol, firmware DMA drain, physical hardware or hostile/SMP containment proof.'if ethernet else 'Normal-profile exact BOOT lease, Ring3 memory-on/master-off command, bounded reset/quiescence and actual station address. No DMA, packets, firmware DMA-drain, physical hardware or hostile containment proof.'if preparation else 'Normal-profile BOOT-only command grant and actual Ring3 zero-mutation query/denials. Masked writes/status preservation are exercised with port mocks; live DMA, packets, hotplug, physical hardware and hostile containment are not established.'))


def main():
    p=argparse.ArgumentParser();p.add_argument('log',type=Path);p.add_argument('--expected',type=int,choices=[0,1],default=1);p.add_argument('--preparation',action='store_true');p.add_argument('--ethernet',action='store_true');a=p.parse_args()
    r=inspect(a.log.read_text(errors='replace'),a.expected,a.preparation,a.ethernet);print(json.dumps(r,indent=2));return not r['ok']


if __name__=='__main__':raise SystemExit(main())
