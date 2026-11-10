#!/usr/bin/env python3
"""Power off and restart through ACPI.

Boot 1: the power button (QMP system_powerdown raises the SCI) starts a shut down; Files
flushes and answers; the kernel enters S5 and QEMU ends by itself (no -no-shutdown).
Boot 2: Restart from the desktop launcher; Files flushes, the kernel resets through the
FADT reset register and the machine boots again; then Shut Down from the launcher ends it.
"""
import argparse,os,re,shutil,subprocess,sys,time
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
from desktop_close_gate import QMP
from quil_workflow_gate import palette_commands,palette_select
ROOT=Path(__file__).resolve().parents[1]
FAIL=r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at|\[acpi\.power\.off\] failed=1'

def launch(iso,disk,out,tag,reboot):
 serial=out/f'{tag}-serial.log';sock=Path('/tmp')/f'sex-power-{os.getpid()}-{tag}.sock'
 cmd=['qemu-system-x86_64','-M','q35','-cpu','max,+pku,+rdseed','-smp',os.environ.get('SEXOS_GATE_CORES','1'),'-m','512M','-vga','none','-device','VGA,xres=1280,yres=800',
      '-device','nec-usb-xhci,id=xhci','-device','usb-tablet,bus=xhci.0','-cdrom',str(iso),'-drive',f'if=none,id=nvm,file={disk},format=raw','-device','nvme,serial=sexos01,drive=nvm',
      '-serial',f'file:{serial}','-qmp',f'unix:{sock},server=on,wait=off','-display','none','-nic','none',*([]if reboot else['-no-reboot'])]
 proc=subprocess.Popen(cmd,stdout=(out/f'{tag}-qemu.log').open('w'),stderr=subprocess.STDOUT);began=time.monotonic()
 def text():return serial.read_text(errors='replace')if serial.exists()else''
 def wait(pattern,start=0,timeout=180,exited_ok=False):
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
 return dict(q=q,proc=proc,text=text,wait=wait,ended=ended,stop=stop,began=began)

def desktop(g,start=0):
 g['wait'](r'\[acpi\.power\] s5=1 ',start)
 g['wait'](r'\[shell.app.registered.reply\] target=15 .*committed=1',start,240)
 g['wait'](r'\[linen.disk.publish.done\]',start,240)

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',type=Path);p.add_argument('--out',required=True,type=Path);a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img'
 if a.source_disk:shutil.copyfile(a.source_disk,disk)
 else:
  with disk.open('wb')as f:f.truncate(256*1024*1024)
 result={}
 # Boot 1: the power button.
 g=launch(a.iso,disk,a.out,'button',reboot=False)
 try:
  desktop(g);result['sexacpi']=g['wait'](r'\[sexacpi\.ready\] aml=1 ([^\n]*)\n')[1];g['wait'](r'\[acpi\.firmware\.inventory\] aml=1 ');
  if re.search(r'\[acpi\.aml',g['text']()):raise RuntimeError('AML ran in the kernel');info=g['wait'](r'\[acpi\.power\] ([^\n]*)\n')[1];result['acpi']=info
  s=len(g['text']());t0=time.monotonic();g['q'].cmd('system_powerdown')
  g['wait'](r'\[acpi\.power_button\] pressed=1',s,30)
  g['wait'](r'\[acpi\.power\.begin\] kind=off by=power_button files=\d+ told=1 firmware_told=1',s,30)
  g['wait'](r'\[files\.power\.flush\] op=2 flushed=\d document_saves_done=1',s,30)
  g['wait'](r'\[acpi\.power\.flush\] files=(flushed|failed) late=0',s,30,exited_ok=True)
  g['wait'](r'\[acpi\.power\.off\] s5=0x[0-9a-f]+ entering=1',s,30,exited_ok=True)
  code=g['ended']();result['button']=dict(exit=code,seconds=round(time.monotonic()-t0,2))
  if code!=0:raise RuntimeError(f'QEMU exit {code} after S5')
 finally:g['stop']()
 # Boot 2: Restart, then Shut Down, from the launcher.
 commands=palette_commands(ROOT)
 g=launch(a.iso,disk,a.out,'launcher',reboot=True)
 try:
  desktop(g)
  def key(code,pattern):s=len(g['text']());g['q'].key(code);return g['wait'](pattern,s,30)
  boots=len(re.findall(r'\[acpi\.power\] s5=1 ',g['text']()))
  s=len(g['text']());palette_select(key,'Restart',commands);g['q'].key('ret')
  g['wait'](r'\[shell\.power\.request\] op=restart ok=1',s,30)
  g['wait'](r'\[files\.power\.flush\] op=3 flushed=\d document_saves_done=1',s,30)
  g['wait'](r'\[acpi\.power\.restart\] method=fadt_reset_register',s,30)
  # The same QEMU process boots again.
  again=g['wait'](r'\[acpi\.power\] s5=1 ',s+1,180).start()+s+1
  if len(re.findall(r'\[acpi\.power\] s5=1 ',g['text']()))!=boots+1:raise RuntimeError('restart did not boot again')
  desktop(g,again)
  s=len(g['text']());palette_select(key,'ShutDown',commands);t0=time.monotonic();g['q'].key('ret')
  g['wait'](r'\[shell\.power\.request\] op=off ok=1',s,30,exited_ok=True)
  g['wait'](r'\[acpi\.power\.off\] s5=0x[0-9a-f]+ entering=1',s,30,exited_ok=True)
  code=g['ended']();result['launcher']=dict(exit=code,seconds=round(time.monotonic()-t0,2),restarted=1)
  if code!=0:raise RuntimeError(f'QEMU exit {code} after S5')
 finally:g['stop']()
 print({'ok':True,**result})
if __name__=='__main__':main()
