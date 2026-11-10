#!/usr/bin/env python3
"""A discrete GPU nothing drives is powered off at boot (D3cold through ACPI).

QEMU gets a laptop-shaped discrete GPU: a PCIe root port at 00:1c.0 with a display
controller behind it, and an SSDT (tests/acpi/dgpu_gate_ssdt.asl) that gives the root port
a power resource named by _PR0/_PR3, like Dell's PEG1/PG01. Its _ON/_OFF and the GPU's
_PS0/_PS3 write to the debug console, so the gate sees the firmware methods ran.

Checks: the boot GPU on the root bus stays on; the discrete one goes to D3hot, runs _PS3,
the root port's power resource turns off; the test marker (SXGT) makes BOOT power it back
on with its configuration restored, then off again; the desktop still comes up.
"""
import argparse,os,re,shutil,subprocess,sys,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
FAIL=r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at|\[acpi\.aml\.quarantined\]'

def ssdt(out):
 aml=out/'dgpu_gate_ssdt.aml'
 subprocess.run(['iasl','-p',str(aml.with_suffix('')),str(ROOT/'tests/acpi/dgpu_gate_ssdt.asl')],check=True,stdout=subprocess.DEVNULL)
 return aml

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',type=Path);p.add_argument('--out',required=True,type=Path);a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img'
 if a.source_disk:shutil.copyfile(a.source_disk,disk)
 else:
  with disk.open('wb')as f:f.truncate(256*1024*1024)
 serial=a.out/'serial.log';debug=a.out/'debugcon.log'
 for f in(serial,debug):f.unlink(missing_ok=True)
 cmd=['qemu-system-x86_64','-M','q35','-cpu','max,+pku,+rdseed','-smp',os.environ.get('SEXOS_GATE_CORES','1'),'-m','512M','-vga','none','-device','VGA,xres=1280,yres=800',
      '-device','pcie-root-port,id=rp5,bus=pcie.0,addr=1c.0,chassis=5','-device','bochs-display,bus=rp5,addr=0.0',
      '-acpitable',f'file={ssdt(a.out)}','-debugcon',f'file:{debug}',
      '-device','nec-usb-xhci,id=xhci','-device','usb-tablet,bus=xhci.0','-cdrom',str(a.iso),'-drive',f'if=none,id=nvm,file={disk},format=raw','-device','nvme,serial=sexos01,drive=nvm',
      '-serial',f'file:{serial}','-display','none','-nic','none','-no-reboot']
 proc=subprocess.Popen(cmd,stdout=(a.out/'qemu.log').open('w'),stderr=subprocess.STDOUT)
 # AML names print with their 4-byte padding (\_SB_); compare without it.
 def text():return serial.read_text(errors='replace').replace('\\_SB_.','\\_SB.')if serial.exists()else''
 def wait(pattern,timeout=240):
  d=time.monotonic()+timeout
  while time.monotonic()<d:
   t=text();bad=re.search(FAIL,t)
   if bad:raise RuntimeError(f'guest failure {bad[0]}')
   m=re.search(pattern,t)
   if m:return m
   if proc.poll()is not None:raise RuntimeError('guest exited early')
   time.sleep(.05)
  raise TimeoutError(pattern)
 try:
  keep=wait(r'\[gpu\.power\] at=00:01\.0 action=keep reason=integrated')
  found=wait(r'\[gpu\.found\] at=(\w\w:00\.0) vendor=1234 device=1111 class=0380')
  gpu=found[1]
  if gpu.startswith('00:'):raise RuntimeError('discrete GPU on the root bus')
  first=wait(rf'\[gpu\.power\] at={gpu} name="[^"]*" action=off state=(d3cold|d3cold_requested) gpu_node=\\_SB\.PCI0\.RP05\.PEGP port=00:1c\.0 port_node=\\_SB\.PCI0\.RP05 resources_off=1 config_gone=\d steps:([^\n]*)')
  if '\\_SB.PCI0.RP05.PEGP._PS3' not in first[2] or '\\_SB.PCI0.RP05.PG05._OFF sta=0' not in first[2]:raise RuntimeError(f'steps: {first[2]}')
  wait(rf'\[gpu\.power\] at={gpu} action=on ok=1 cold=1')
  wait(rf'\[gpu\.power\.cycle\] at={gpu} on=1 restored=1')
  again=wait(rf'(?s)\[gpu\.power\.cycle\][^\n]*\n.*\[gpu\.power\] at={gpu} name="[^"]*" action=off state=(d3cold|d3cold_requested) ')
  wait(r'\[shell.app.registered.reply\] target=15 .*committed=1')
  # Firmware methods, in order: GPU _PS3, port _OFF; back on: port _ON, GPU _PS0; off again.
  dc=debug.read_bytes().decode(errors='replace') if debug.exists() else ''
  if '3FN03F' not in dc:raise RuntimeError(f'debug console {dc!r}')
  print({'ok':True,'gpu':gpu,'state':first[1],'again':again[1],'firmware':dc[dc.index('3FN03F'):][:6]})
 except Exception:
  print('\n'.join(l for l in text().splitlines() if re.search(r'\[(gpu|acpi)',l)),file=sys.stderr);raise
 finally:
  if proc.poll()is None:proc.kill();proc.wait()
if __name__=='__main__':main()
