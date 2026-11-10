#!/usr/bin/env python3
"""USB sticks: a FAT12 stick on the USB controller mounts itself under /media, is listed,
read and written from Spindle, unmounts safely, is pulled out and plugged back in (and
mounts again with the new file on it), and the file is on the stick image afterwards as
mtools reads it."""
import argparse,os,re,shutil,struct,subprocess,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
def unsparse(name):
 raw=(ROOT/'tests/volumes'/f'{name}.sparse').read_bytes();assert raw[:8]==b'SXSPARSE'
 out=bytearray(struct.unpack('<Q',raw[8:16])[0]*512);at=16
 while at<len(raw):
  lba=struct.unpack('<Q',raw[at:at+8])[0];out[lba*512:lba*512+512]=raw[at+8:at+520];at+=520
 return bytes(out)
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 stick=a.out/'stick.img';stick.write_bytes(unsparse('fat12'))
 # The stick is plugged in after boot: present at power-on, the firmware takes it over
 # (and boots from it) and the PS/2 keyboard stops interrupting. A blockdev node, unlike a
 # -drive, outlives the unplug, so the same stick can go back in.
 os.environ.pop('SEXOS_GATE_DEVICES',None)
 os.environ.setdefault('SEXOS_GATE_BOUND','600')
 (a.out/'usbstick-serial.log').unlink(missing_ok=True)
 from quil_autosave_gate import boot
 g=boot(a.iso,disk,a.out,'usbstick',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   shift=k.isupper() or k in ':_'
   code={':':'semicolon','_':'minus'}.get(k,KEYS.get(k,k.lower()))
   if shift:q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':code}],'hold-time':80})
   else:q.key(code)
   time.sleep(.06)
 def line(text):g['focus'](153);time.sleep(.5);keys(text);q.key('ret')
 def expect(text,pattern,timeout=60):s=len(t());line(text);return g['wait'](pattern,s,timeout)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  q.cmd('blockdev-add',{'driver':'raw','node-name':'stickdrive','file':{'driver':'file','filename':str(stick)}})
  s=len(t());q.cmd('device_add',{'driver':'usb-storage','bus':'xhci.0','drive':'stickdrive','id':'stick'})
  g['wait'](r'\[sexusb\.storage\] slot=\d+ disk=0 name="QEMU QEMU HARDDISK" block=512 sectors=\d+ ',s,60)
  g['wait'](r'\[files\.media\.mounted\] name=FLOPPY format=FAT12 [^\n]*source=usb disk=0',s,60)
  # Spindle can drop the first keys while it opens; the first command is retried.
  for attempt in range(3):
   s=len(t());line('ls /media/FLOPPY')
   try:g['wait'](r'\[spindle.fs.result\] kind=2 status=0',s,30);break
   except TimeoutError:
    if attempt==2:raise
  time.sleep(1)
  listed=re.findall(r'\[spindle.fs.line\] ([^\n]*)',t()[s:]);assert 'HELLO.TXT  17' in listed,listed
  expect('cat /media/FLOPPY/hello.txt',r'\[spindle.fs.cat\] hello from fat12')
  expect('write /media/FLOPPY/from-usb.txt written over usb',r'\[spindle.fs.result\] kind=8 status=0')
  expect('cat /media/FLOPPY/from-usb.txt',r'\[spindle.fs.cat\] written over usb')
  expect('unmount FLOPPY',r'\[spindle.fs.line\] Safe to remove\.')
  # Pull it out, then plug it back in: it mounts again, with the file.
  s=len(t());q.cmd('device_del',{'id':'stick'});g['wait'](r'\[sexusb\.storage\.gone\] disk=0 ',s,60)
  s=len(t())
  for _ in range(40):  # device_del finishes when the guest lets go; the drive is free then
   try:q.cmd('device_add',{'driver':'usb-storage','bus':'xhci.0','drive':'stickdrive','id':'stick2'});break
   except RuntimeError:time.sleep(.5)
  else:raise RuntimeError('the stick could not be plugged back in')
  g['wait'](r'\[files\.media\.mounted\] name=FLOPPY format=FAT12 [^\n]*source=usb',s,120)
  expect('cat /media/FLOPPY/from-usb.txt',r'\[spindle.fs.cat\] written over usb')
  # Pulled out while mounted: Files forgets it.
  s=len(t());q.cmd('device_del',{'id':'stick2'});g['wait'](r'\[files\.media\.usb\.gone\] name=FLOPPY disk=0',s,60)
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
 finally:g['stop']()
 if shutil.which('mtype'):
  out=subprocess.run(['mtype','-i',str(stick),'::FROM-USB.TXT'],capture_output=True,text=True,env={**os.environ,'MTOOLS_SKIP_CHECK':'1'})
  assert out.stdout.strip()=='written over usb',(out.stdout,out.stderr)
  print('[usb_storage_gate] mtools reads the file back')
 print({'ok':True})
if __name__=='__main__':main()
