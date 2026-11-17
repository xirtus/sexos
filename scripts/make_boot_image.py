#!/usr/bin/env python3
"""Builds the boot partition `install` writes to a disk: FAT with Limine's UEFI loader, its
configuration, the kernel and every module, exactly as on the ISO."""
import argparse,subprocess,sys,tempfile
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
# 12 MiB: room for today's kernel and modules. With Wi-Fi firmware in the pack it takes
# the whole 32 MiB slot the disk keeps for it (sex_pdx::fs::BOOT_SECTORS).
SECTORS=24576
SECTORS_WITH_FIRMWARE=65536
def main():
 p=argparse.ArgumentParser();p.add_argument('--root',type=Path,default=ROOT/'iso_root');p.add_argument('--out',type=Path,required=True);a=p.parse_args()
 img=a.out;img.parent.mkdir(parents=True,exist_ok=True)
 pack=a.root/'firmware'/'wifi.pack'
 # Everything copied in, plus room for FAT tables and the loader: the larger slot when the
 # small one cannot hold it (the firmware pack, or modules outgrowing 12 MiB when the
 # firmware could not be fetched).
 files=[a.root/'sexos-kernel']+[f for sub in ('servers','apps') for f in (a.root/sub).iterdir() if f.is_file()]+([pack] if pack.is_file() else [])
 need=sum(f.stat().st_size for f in files)+(1<<20)
 sectors=SECTORS if need<=SECTORS*512 and not (pack.is_file() and pack.stat().st_size>1<<20) else SECTORS_WITH_FIRMWARE
 if need>sectors*512:
  # Say what fills the slot instead of letting mcopy fail with "disk full".
  for f in sorted(files,key=lambda f:-f.stat().st_size)[:12]:print(f'[boot_image] {f.stat().st_size:>10} {f.relative_to(a.root)}',file=sys.stderr)
  raise SystemExit(f'[boot_image] FAIL need={need} bytes > slot={sectors*512} bytes (sex_pdx::fs::BOOT_SECTORS)')
 print(f'[boot_image] need={need} slot={sectors*512} free={sectors*512-need}')
 with img.open('wb') as f:f.truncate(sectors*512)
 def run(*c):
  r=subprocess.run(c,capture_output=True,text=True)
  if r.returncode:raise SystemExit(f'[boot_image] FAIL {" ".join(c)}: {r.stderr.strip() or r.stdout.strip()} (need={need} slot={sectors*512})')
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
