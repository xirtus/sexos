#!/usr/bin/env python3
"""Laptop power through ACPI: battery, AC, lid, brightness, profiles, power button, thermal trip.

QEMU gets laptop firmware (tests/acpi/laptop_gate_ssdt.asl): an unplugged AC adapter, a
half-full battery drawing 21.5 W, a lid that closes for a moment every 40 reads, a thermal
zone, and a panel with ACPI brightness. The battery's _BST sends the panel the brightness-up
notification once, as a laptop's EC event method does for the Fn key.

Boot 1 checks: the ACPI service finds every device and reports 50%, 2:00 left, 96% health,
on battery; Notify reaches the service (it used to stop the interpreter) and raises the
brightness; the panel shows "50%  2:00 left"; the Shell sees the lid close and open; the
launcher's Power Saver and Brightness Down reach the kernel and the firmware; the power
button opens the power dialog instead of cutting power, and Enter on it shuts down cleanly.
Boot 2 (-DTHERMAL_TRIP): the zone passes its critical trip point and the machine shuts
itself down cleanly, Files flushing first.
"""
import argparse,os,re,shutil,subprocess,sys,time
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
from desktop_close_gate import QMP
from quil_workflow_gate import palette_commands,palette_select
ROOT=Path(__file__).resolve().parents[1]
FAIL=r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at|\[sexacpi\.aml\.panic\]|\[acpi\.power\.off\] failed=1'

def ssdt(out,thermal):
 aml=out/('laptop_trip.aml' if thermal else 'laptop.aml')
 subprocess.run(['iasl',*(['-DTHERMAL_TRIP']if thermal else[]),'-p',str(aml.with_suffix('')),str(ROOT/'tests/acpi/laptop_gate_ssdt.asl')],check=True,stdout=subprocess.DEVNULL)
 return aml

def launch(iso,disk,out,tag,thermal):
 serial=out/f'{tag}-serial.log';debug=out/f'{tag}-debugcon.log';sock=Path('/tmp')/f'sex-laptop-{os.getpid()}-{tag}.sock'
 for f in(serial,debug):f.unlink(missing_ok=True)
 cmd=['qemu-system-x86_64','-M','q35','-cpu','max,+pku,+rdseed','-smp',os.environ.get('SEXOS_GATE_CORES','1'),'-m','512M','-vga','none','-device','VGA,xres=1280,yres=800',
      '-acpitable',f'file={ssdt(out,thermal)}','-chardev',f'file,id=sxdbg,path={debug}','-device','isa-debugcon,iobase=0x402,chardev=sxdbg',
      '-device','nec-usb-xhci,id=xhci','-device','usb-tablet,bus=xhci.0','-cdrom',str(iso),'-drive',f'if=none,id=nvm,file={disk},format=raw','-device','nvme,serial=sexos01,drive=nvm',
      '-serial',f'file:{serial}','-qmp',f'unix:{sock},server=on,wait=off','-display','none','-nic','none','-no-reboot']
 proc=subprocess.Popen(cmd,stdout=(out/f'{tag}-qemu.log').open('w'),stderr=subprocess.STDOUT)
 def text():return serial.read_text(errors='replace').replace('\\_SB_.','\\_SB.')if serial.exists()else''
 def wait(pattern,start=0,timeout=240,exited_ok=False):
  d=time.monotonic()+timeout
  while time.monotonic()<d:
   t=text();bad=re.search(FAIL,t)
   if bad:raise RuntimeError(f'{tag}: guest failure {bad[0]}')
   m=re.search(pattern,t[start:])
   if m:return m
   if proc.poll()is not None and not exited_ok:raise RuntimeError(f'{tag}: guest exited early')
   time.sleep(.05)
  raise TimeoutError(f'{tag}: {pattern}')
 d=time.monotonic()+15
 while not sock.exists():
  if time.monotonic()>d:raise TimeoutError('QMP unavailable')
  time.sleep(.05)
 q=QMP(sock)
 def ended(timeout=30):
  try:return proc.wait(timeout)
  except subprocess.TimeoutExpired:raise TimeoutError(f'{tag}: QEMU still running after S5')
 def stop():
  if proc.poll()is None:proc.kill();proc.wait()
  sock.unlink(missing_ok=True)
 def firmware():return debug.read_bytes().decode(errors='replace')if debug.exists()else''
 return dict(q=q,text=text,wait=wait,ended=ended,stop=stop,firmware=firmware)

def desktop(g):
 g['wait'](r'\[shell.app.registered.reply\] target=15 .*committed=1')
 g['wait'](r'\[linen.disk.publish.done\]')

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',type=Path);p.add_argument('--out',required=True,type=Path);a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img'
 if a.source_disk:shutil.copyfile(a.source_disk,disk)
 else:
  with disk.open('wb')as f:f.truncate(256*1024*1024)
 commands=palette_commands(ROOT);result={}
 g=launch(a.iso,disk,a.out,'laptop',thermal=False)
 try:
  ready=g['wait'](r'\[sexacpi\.ready\] aml=1 ([^\n]*)\n')[1];result['ready']=ready
  for want in('batteries=1 ','ac=1 ','lids=1 ','backlight=1'):
   if want not in ready:raise RuntimeError(f'inventory: {ready}')
  g['wait'](r'\[sexacpi\.backlight\] path=\\_SB\.PCI0\.GFX0\.DD1F levels=10 current=50')
  # The touchpad: described from _CRS/_DSM/_ADR/FMCN and handed to Input, which holds no
  # Serial IO controller under QEMU.
  if 'i2c_hid=1' not in ready:raise RuntimeError(f'touchpad inventory: {ready}')
  result['touchpad']=g['wait'](r'\[sexacpi\.touchpad\] path=\\_SB\.PCI0\.I2C1\.TPD0 pci=00:15\.1 address=0x2c ten_bit=0 khz=400 hid_register=0x20 gpio=348 counts=82/156/30')[0]
  g['wait'](r'\[sexacpi\.touchpad\.sent\] pci=00:15\.1 status=0x0')
  g['wait'](r'\[sexinput\.touchpad\.found\] pci=00:15\.1 address=0x2c khz=400 hid_register=0x20')
  g['wait'](r'\[sexinput\.touchpad\.absent\] reason=controller_not_leased')
  sensors=g['wait'](r'\[sexacpi\.sensors\] battery=50 charging=0 minutes=120 health=96 ac=0 lid=1 temp_dk=3182 crit_dk=3732 brightness=\d+ critical=0')
  # The brightness-up key, as firmware Notify: it used to stop the interpreter.
  g['wait'](r'\[sexacpi\.notify\] path=\\_SB\.PCI0\.GFX0\.DD1F value=0x86 kind=backlight')
  g['wait'](r'\[sexacpi\.backlight\.set\] level=60 by=key bcm=1 bqc=60')
  g['wait'](r'\[display\.panel\.battery\] text=50%  2:00 left width=\d+')
  desktop(g)
  # The lid closes and opens again (the firmware's _LID, read by the service).
  closed=g['wait'](r'\[shell\.lid\] open=0 ',0,180).end()
  # Closing the lid suspends to idle: firmware's LPS0 screen-off and entry, panel dark.
  g['wait'](r'\[shell\.sleep\] enter=1 by=lid ok=1',closed,30)
  g['wait'](r'\[acpi\.sleep\] enter=1 firmware_told=1',closed,30)
  g['wait'](r'\[sexacpi\.sleep\] enter=1 lps0=\\_SB\.PEPD ran=intel:3 intel:5 panel=10',closed,30)
  opened=g['wait'](r'\[shell\.lid\] open=1 ',closed,120).end()
  g['wait'](r'\[shell\.sleep\] enter=0 by=lid ok=1',opened,30)
  g['wait'](r'\[sexacpi\.sleep\] enter=0 lps0=\\_SB\.PEPD ran=intel:6 intel:4 panel=60',opened,30)
  if 'dSWD' not in g['firmware']().replace('B','').replace('P',''):raise RuntimeError(f"LPS0 order: {g['firmware']()!r}")
  def key(code,pattern):s=len(g['text']());g['q'].key(code);return g['wait'](pattern,s,30)
  # Launcher: Power Saver, then Brightness Down.
  s=len(g['text']());palette_select(key,'PowerSaver',commands);g['q'].key('ret')
  g['wait'](r'\[shell\.laptop\.command\] what=profile_saver ok=1',s,30)
  result['profile']=g['wait'](r'\[acpi\.profile\] profile=saver epp=0xc0 ([^\n]*)',s,30)[1]
  s=len(g['text']());palette_select(key,'BrightnessDown',commands);g['q'].key('ret')
  g['wait'](r'\[shell\.laptop\.command\] what=brightness_down ok=1',s,30)
  g['wait'](r'\[sexacpi\.backlight\.set\] level=50 by=desktop bcm=1 bqc=50',s,30)
  g['wait'](r'\[sexacpi\.sensors\] [^\n]*brightness=50 ',s,30)
  fw=g['firmware']()
  if fw.count('B')<2:raise RuntimeError(f'_BCM did not run twice: {fw!r}')
  if fw.count('P')<2:raise RuntimeError(f'touchpad and controller _PS0 did not run: {fw!r}')
  # The power button opens the power dialog; Enter shuts down cleanly.
  s=len(g['text']());g['q'].cmd('system_powerdown')
  g['wait'](r'\[acpi\.power_button\] pressed=1 to_desktop=1',s,30)
  g['wait'](r'\[shell\.power\.dialog\] open=1 selected=Shut Down',s,30)
  if re.search(r'\[acpi\.power\.begin\]',g['text']()[s:]):raise RuntimeError('power button cut power before the dialog')
  g['q'].key('ret')
  g['wait'](r'\[shell\.power\.request\] op=off ok=1',s,30,exited_ok=True)
  g['wait'](r'\[acpi\.power\.off\] s5=0x[0-9a-f]+ entering=1',s,30,exited_ok=True)
  code=g['ended']();result['dialog_off']=code
  if code!=0:raise RuntimeError(f'QEMU exit {code} after S5')
 except Exception:
  print('\n'.join(l for l in g['text']().splitlines() if re.search(r'\[(sexacpi|sexinput\.touchpad|boot\.touchpad|acpi|shell\.(lid|power|laptop|sleep)|display\.panel\.battery)',l)),file=sys.stderr);raise
 finally:g['stop']()
 # Boot 2: a thermal zone past its critical trip point shuts the machine down.
 g=launch(a.iso,disk,a.out,'thermal',thermal=True)
 try:
  crit=g['wait'](r'\[acpi\.power\.critical\] reason=thermal_critical [^\n]*trip_dk=3732',0,300)
  g['wait'](r'\[acpi\.power\.begin\] kind=off by=thermal_critical ',crit.start(),30,exited_ok=True)
  g['wait'](r'\[acpi\.power\.off\] s5=0x[0-9a-f]+ entering=1',crit.start(),60,exited_ok=True)
  code=g['ended']();result['thermal_off']=code
  if code!=0:raise RuntimeError(f'QEMU exit {code} after S5')
 except Exception:
  print('\n'.join(l for l in g['text']().splitlines() if re.search(r'\[(sexacpi|acpi)',l)),file=sys.stderr);raise
 finally:g['stop']()
 print({'ok':True,**result})
if __name__=='__main__':main()
