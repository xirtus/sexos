#!/usr/bin/env python3
"""Builds the boot partition `install` writes to a disk: FAT with Limine's UEFI loader, its
configuration, the kernel and every module, exactly as on the ISO."""
import argparse,subprocess,tempfile
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
# 12 MiB: room for today's kernel and modules. With Wi-Fi firmware in the pack it takes
# 32 MiB, and more in whole MiB once kernel, modules and pack outgrow that. Installs write
# it into a 256 MiB GPT system slot (sex_disk::install::SLOT_SECTORS); the old 32 MiB
# legacy slot (sex_pdx::fs::BOOT_SECTORS) is only ever replaced by a GPT install now.
SECTORS=24576
SECTORS_WITH_FIRMWARE=65536
SLOT_SECTORS=256*2048
CLUSTER=4096

def sectors_for(sizes,pack_bytes):
 # Each file rounded up to a cluster, plus FAT tables, directories and the loader.
 need=sum(-(-n//CLUSTER)*CLUSTER for n in sizes)+(2<<20)
 if need<=SECTORS*512 and pack_bytes<=1<<20:return SECTORS
 sectors=max(SECTORS_WITH_FIRMWARE,-(-need//(1<<20))*2048)
 if sectors>SLOT_SECTORS:raise SystemExit(f'boot image needs {need} bytes, more than the {SLOT_SECTORS*512}-byte system slot')
 return sectors
def main():
 p=argparse.ArgumentParser();p.add_argument('--root',type=Path,default=ROOT/'iso_root');p.add_argument('--out',type=Path,required=True);a=p.parse_args()
 img=a.out;img.parent.mkdir(parents=True,exist_ok=True)
 pack=a.root/'firmware'/'wifi.pack'
 # Everything copied in, plus room for FAT tables and the loader.
 files=[a.root/'sexos-kernel']+[f for sub in ('servers','apps') for f in (a.root/sub).iterdir() if f.is_file()]+([pack] if pack.is_file() else [])
 sectors=sectors_for([f.stat().st_size for f in files],pack.stat().st_size if pack.is_file() else 0)
 with img.open('wb') as f:f.truncate(sectors*512)
 def run(*c):
  r=subprocess.run(c,capture_output=True,text=True)
  if r.returncode:raise SystemExit(f'{" ".join(c)} failed ({r.returncode}): {r.stderr.strip() or r.stdout.strip()}')
 run('mformat','-i',str(img),'-T',str(sectors),'-h','64','-s','32','-v','SEXOS','::')
 with tempfile.TemporaryDirectory() as tmp:
  efi=Path(tmp)/'BOOTX64.EFI'
  run('mcopy','-i',str(a.root/'boot/limine/limine-uefi-cd.bin'),'::/EFI/BOOT/BOOTX64.EFI',str(efi))
  for d in ('::/EFI','::/EFI/BOOT','::/servers','::/apps','::/firmware'):run('mmd','-i',str(img),d)
  run('mcopy','-i',str(img),str(efi),'::/EFI/BOOT/BOOTX64.EFI')
 # The installed system boots without the image it was installed from.
 cfg=Path(tmp_cfg:=str(img)+'.cfg');cfg.write_text(''.join(l for l in (a.root/'limine.cfg').read_text().splitlines(True)
   if 'boot:///boot.img' not in l and (pack.is_file() or 'boot:///firmware/wifi.pack' not in l)))
 run('mcopy','-i',str(img),str(cfg),'::/limine.cfg');cfg.unlink()
 run('mcopy','-i',str(img),str(a.root/'sexos-kernel'),'::/sexos-kernel')
 for sub in ('servers','apps'):
  for f in sorted((a.root/sub).iterdir()):
   if f.is_file():run('mcopy','-i',str(img),str(f),f'::/{sub}/{f.name}')
 if pack.is_file():run('mcopy','-i',str(img),str(pack),'::/firmware/wifi.pack')
 print(img,img.stat().st_size)
if __name__=='__main__':main()
