#!/usr/bin/env python3
"""Builds the boot partition `install` writes to a disk: FAT with Limine's UEFI loader, its
configuration, the kernel and every module, exactly as on the ISO."""
import argparse,subprocess,tempfile
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
# 12 MiB: room for today's kernel and modules. The disk keeps a 32 MiB slot for it.
SECTORS=24576
def main():
 p=argparse.ArgumentParser();p.add_argument('--root',type=Path,default=ROOT/'iso_root');p.add_argument('--out',type=Path,required=True);a=p.parse_args()
 img=a.out;img.parent.mkdir(parents=True,exist_ok=True)
 with img.open('wb') as f:f.truncate(SECTORS*512)
 run=lambda *c:subprocess.run(c,check=True,capture_output=True)
 run('mformat','-i',str(img),'-T',str(SECTORS),'-h','64','-s','32','-v','SEXOS','::')
 with tempfile.TemporaryDirectory() as tmp:
  efi=Path(tmp)/'BOOTX64.EFI'
  run('mcopy','-i',str(a.root/'boot/limine/limine-uefi-cd.bin'),'::/EFI/BOOT/BOOTX64.EFI',str(efi))
  for d in ('::/EFI','::/EFI/BOOT','::/servers','::/apps'):run('mmd','-i',str(img),d)
  run('mcopy','-i',str(img),str(efi),'::/EFI/BOOT/BOOTX64.EFI')
 # The installed system boots without the image it was installed from.
 cfg=Path(tmp_cfg:=str(img)+'.cfg');cfg.write_text(''.join(l for l in (a.root/'limine.cfg').read_text().splitlines(True) if 'boot:///boot.img' not in l))
 run('mcopy','-i',str(img),str(cfg),'::/limine.cfg');cfg.unlink()
 run('mcopy','-i',str(img),str(a.root/'sexos-kernel'),'::/sexos-kernel')
 for sub in ('servers','apps'):
  for f in sorted((a.root/sub).iterdir()):
   if f.is_file():run('mcopy','-i',str(img),str(f),f'::/{sub}/{f.name}')
 print(img,img.stat().st_size)
if __name__=='__main__':main()
