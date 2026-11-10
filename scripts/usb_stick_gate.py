#!/usr/bin/env python3
"""The ISO, written byte for byte to a USB stick, starts SexOS: under UEFI (OVMF) and under
a legacy BIOS (SeaBIOS), from a USB mass-storage device on the xHCI controller, with no
CD drive. The stick is a copy of the ISO, as `dd` or any image writer leaves it."""
import argparse,os,shutil,time
from pathlib import Path
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True)
 from quil_autosave_gate import boot
 results={}
 for firmware in('uefi','bios'):
  stick=a.out/f'stick-{firmware}.img';shutil.copyfile(a.iso,stick)
  disk=a.out/f'nvme-{firmware}.img'
  with disk.open('wb') as f:f.truncate(256*1024*1024)
  os.environ['SEXOS_GATE_NO_CDROM']='1'
  if firmware=='uefi':os.environ['SEXOS_GATE_FIRMWARE']=os.environ.get('SEXOS_UEFI_CODE','/usr/share/OVMF/OVMF_CODE_4M.fd')
  else:os.environ.pop('SEXOS_GATE_FIRMWARE',None)
  # bootindex makes the stick the first thing the firmware tries.
  os.environ['SEXOS_GATE_DEVICES']=f'-drive if=none,id=stick,file={stick},format=raw -device usb-storage,bus=xhci.0,drive=stick,bootindex=0'
  g=boot(a.iso,disk,a.out,'stick-'+firmware,a.resolution)
  try:
   g['wait'](r'\[app.private.retired\] target=41 ',0,400)
   assert 'KERNEL PANIC' not in g['text']()
   g['q'].cmd('screendump',{'filename':str(a.out/f'desktop-{firmware}.ppm')});time.sleep(.5)
   results[firmware]='desktop'
  finally:g['stop']()
  stick.unlink();disk.unlink()
 print({'ok':True,**results})
if __name__=='__main__':main()
