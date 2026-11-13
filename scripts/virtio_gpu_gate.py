#!/usr/bin/env python3
"""Display drives a virtio-gpu itself (workstream L, slice L2).

QEMU gets a `virtio-vga` with EDID. BOOT leases it to Display (slot 25) and adds a
DMA-addressable scanout buffer (slot 26). Display reads the heads and the monitor's
EDID through the device, creates a host resource backed by that buffer, copies the
boot picture into it and switches the scanout, then presents every publication as
one transfer + fenced flush (a tear-free flip).

Checks: lease, scanout grant, head and EDID receipts, takeover, flip sequence
receipts; a screendump of the host resource shows the desktop (bar, launcher
square, window lights), and moving the pointer changes the picture afterwards, so
frames keep reaching the screen through the GPU and not the firmware framebuffer.
"""
import argparse,os,re,shutil,socket,subprocess,sys,time
from pathlib import Path
FAIL=r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at|takeover=0'

def ppm(path):
 data=path.read_bytes();parts=data.split(b'\n',3)
 if parts[0]!=b'P6':raise RuntimeError('screendump format')
 w,h=map(int,parts[1].split());return w,h,parts[3]
def px(img,x,y):
 w,h,raw=img;i=(y*w+x)*3;return tuple(raw[i:i+3])
def near(a,b,tol=24):return all(abs(p-q)<=tol for p,q in zip(a,b))

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',type=Path);p.add_argument('--out',required=True,type=Path);a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img'
 if a.source_disk:shutil.copyfile(a.source_disk,disk)
 else:
  with disk.open('wb')as f:f.truncate(256*1024*1024)
 serial=a.out/'serial.log';mon=a.out/'monitor.sock'
 for f in(serial,mon):f.unlink(missing_ok=True)
 cmd=['qemu-system-x86_64','-M','q35','-cpu','max,+pku,+rdseed','-smp',os.environ.get('SEXOS_GATE_CORES','1'),'-m','512M',
      '-vga','none','-device','virtio-vga,edid=on,xres=1280,yres=800',
      '-device','nec-usb-xhci,id=xhci','-device','usb-tablet,bus=xhci.0','-cdrom',str(a.iso),
      '-drive',f'if=none,id=nvm,file={disk},format=raw','-device','nvme,serial=sexos01,drive=nvm',
      '-serial',f'file:{serial}','-monitor',f'unix:{mon},server,nowait','-display','none','-nic','none','-no-reboot']
 proc=subprocess.Popen(cmd,stdout=(a.out/'qemu.log').open('w'),stderr=subprocess.STDOUT)
 def text():return serial.read_text(errors='replace')if serial.exists()else''
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
 def hmp(line):
  s=socket.socket(socket.AF_UNIX);s.connect(str(mon));s.settimeout(5)
  try:
   s.recv(4096);s.sendall(line.encode()+b'\n');time.sleep(.5)
  finally:s.close()
 def shot(name):
  path=a.out/name;path.unlink(missing_ok=True);hmp(f'screendump {path}')
  d=time.monotonic()+10
  while time.monotonic()<d and not(path.exists() and path.stat().st_size>1000):time.sleep(.1)
  time.sleep(.3);return ppm(path)
 try:
  wait(r'\[devmgr\.lease\] \S+ 1af4:1050 service=sexdisplay pd=\d+ slot=25 kind=Virtio')
  wait(r'\[boot\.display\.scanout\] pd=\d+ slot=26 region=0x4210 bytes=\d+ device=virtio-gpu')
  head=wait(r'\[display\.kms\.head\] scanout=0 enabled=1 width=(\d+) height=(\d+)')
  edid=wait(r'\[display\.kms\.edid\] vendor=(\w{3}) preferred=(\d+)x(\d+)@(\d+) size_mm=(\d+)x(\d+)')
  if (edid[2],edid[3])!=('1280','800'):raise RuntimeError(f'EDID preferred {edid[0]}')
  take=wait(r'\[display\.kms\.virtio\] takeover=1 scanouts=(\d+) width=(\d+) height=(\d+) resource=1 sequence=1')
  w,h=int(take[2]),int(take[3])
  wait(r'\[shell.app.registered.reply\] target=15 .*committed=1')
  wait(r'\[display\.kms\.flip\] sequence=4 rows=\d+\.\.\d+ fenced=1')
  time.sleep(3)
  first=shot('desktop.ppm')
  if first[:2]!=(w,h):raise RuntimeError(f'host resource {first[:2]} != {(w,h)}')
  # The bar, the blue launcher square and the red close light of the first window.
  checks={'bar':(px(first,300,14),(37,42,63)),'launcher':(px(first,19,14),(87,158,205)),'close':(px(first,20,49),(230,68,68))}
  bad={k:v for k,v in checks.items() if not near(*v)}
  if bad:raise RuntimeError(f'desktop not on the GPU resource: {bad}')
  for x,y in((200,300),(260,340),(320,380)):hmp(f'mouse_move {x} {y}');time.sleep(.4)
  time.sleep(2)
  second=shot('moved.ppm')
  changed=sum(1 for i in range(0,len(first[2]),3) if first[2][i:i+3]!=second[2][i:i+3])
  if changed<20:raise RuntimeError(f'pointer move did not reach the screen (changed={changed})')
  print({'ok':True,'head':(head[1],head[2]),'edid':edid[0],'desktop':(w,h),'changed_pixels':changed})
 except Exception:
  print('\n'.join(l for l in text().splitlines() if re.search(r'\[(display\.kms|boot\.display\.scanout|devmgr\.lease)|PANIC|panicked',l)),file=sys.stderr);raise
 finally:
  if proc.poll()is None:proc.kill();proc.wait()
if __name__=='__main__':main()
