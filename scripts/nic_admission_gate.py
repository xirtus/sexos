#!/usr/bin/env python3
"""Boot one, unsupported and duplicate NICs using one sealed normal image."""
import argparse,hashlib,json,pathlib,re,shutil,subprocess,time
from check_nic_discovery import inspect

def validate(text,count,leases,skip=None):
 result=inspect(text,expected_leases=leases);errors=result['errors']
 if len(result['devices'])!=count:errors.append('actual enumerated device count differs')
 if skip and '[kernel.cap.nic.skip] reason='+skip not in text:errors.append('missing explicit skip reason')
 maps=re.findall(r'\[boot.memory.mmio\] pd=13 slot=19 bytes=(\d+) actual_bytes=(\d+)',text)
 if len(maps)!=leases or any(not 0x5408<=int(n)<=65536 or int(n)>int(actual)for n,actual in maps):errors.append('invalid exact NIC BAR prefix admission')
 seal=text.find('[boot.memory.authority.sealed]')
 if seal<0 or any(m.start()>seal for m in re.finditer(r'\[kernel.cap.nic.grant\]',text)):errors.append('NIC authority not fixed before BOOT seal')
 if not re.search(r'\[app.surface.presented\] target=15 surface=300 .*ok=1',text):errors.append('cached/offline application did not present')
 result.update(ok=not errors,expected_devices=count,expected_leases=leases,bar_views=maps)
 return result

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',type=pathlib.Path,required=True);p.add_argument('--source-disk',type=pathlib.Path,required=True);p.add_argument('--out',type=pathlib.Path,required=True);p.add_argument('--preparation',action='store_true');a=p.parse_args()
 iso=a.iso.resolve(strict=True);src=a.source_disk.resolve(strict=True);out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise RuntimeError('fresh output required')
 variants=[('single',['-nic','none','-netdev','user,id=n1','-device','e1000e,netdev=n1'],1,1,None),
           ('unsupported',['-nic','none','-netdev','user,id=n1','-device','virtio-net-pci,netdev=n1'],1,0,'unsupported_or_invalid_bar'),
           ('duplicate',['-nic','none','-netdev','user,id=n1','-device','e1000e,netdev=n1','-netdev','user,id=n2','-device','e1000e,netdev=n2'],2,1,'slot_occupied')]
 if a.preparation:variants.append(('legacy',['-nic','none','-netdev','user,id=n1','-device','e1000,netdev=n1'],1,1,None))
 results=[]
 for name,network,count,leases,skip in variants:
  folder=out/name;folder.mkdir();disk=folder/'nvme.img';shutil.copyfile(src,disk);log=folder/'serial.log'
  cmd=['qemu-system-x86_64','-M','q35','-cpu','max,+pku','-m','512M','-cdrom',str(iso),'-device','nec-usb-xhci,id=xhci','-device','usb-tablet,bus=xhci.0','-drive',f'if=none,id=nvm,file={disk},format=raw','-device','nvme,serial=sexos01,drive=nvm','-serial',f'file:{log}','-display','none','-no-reboot','-no-shutdown',*network]
  manifest=dict(iso_sha256=hashlib.sha256(iso.read_bytes()).hexdigest(),input_disk_sha256=hashlib.sha256(src.read_bytes()).hexdigest(),command=cmd)
  (folder/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n');proc=subprocess.Popen(cmd,stdout=(folder/'qemu.log').open('w'),stderr=subprocess.STDOUT)
  try:
   deadline=time.monotonic()+45;ready=None
   while time.monotonic()<deadline:
    text=log.read_text(errors='replace')if log.exists()else''
    if proc.poll()is not None:raise RuntimeError('guest exited')
    if re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at',text):raise RuntimeError('guest fault')
    prepared=not a.preparation or ('to=Quiescent'in text if leases else '[sexnet.nic.prepare.unavailable]'in text)
    if '[app.surface.presented] target=15 surface=300'in text and '[sexnet.passive.ready.fast]'in text and prepared:
     if ready is None:ready=time.monotonic()
     if time.monotonic()-ready>=2:break
    time.sleep(.1)
   result=validate(text,count,leases,skip)
   if a.preparation:
    from check_nic_command import inspect as command_inspect
    command=command_inspect(text,leases,True);result['preparation']=command
    result['errors'].extend(command['errors']);result['ok']=not result['errors']
   (folder/'result.json').write_text(json.dumps(result,indent=2)+'\n');results.append(dict(name=name,**result))
   if not result['ok']:raise RuntimeError(name+': '+str(result['errors']))
  finally:
   if proc.poll()is None:
    proc.terminate()
    try:proc.wait(timeout=10)
    except subprocess.TimeoutExpired:proc.kill();proc.wait()
 (out/'result.json').write_text(json.dumps(dict(ok=True,variants=results,scope='Same sealed image, real PCI enumeration and BOOT capability admission; no live packet or physical-hardware claim.'),indent=2)+'\n')
if __name__=='__main__':main()
