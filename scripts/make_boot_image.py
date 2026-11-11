#!/usr/bin/env python3
"""Builds the boot partition `install` writes to a disk: FAT with Limine's UEFI loader, its
configuration, the kernel and every module, exactly as on the ISO."""
import argparse,subprocess,tempfile
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
# 12 MiB: room for today's kernel and modules. With Wi-Fi firmware in the pack it takes
# the whole 32 MiB slot the disk keeps for it (sex_pdx::fs::BOOT_SECTORS).
SECTORS=24576
SECTORS_WITH_FIRMWARE=65536
def recorder_module(root, config):
 modules=[line.split('=',1)[1].strip() for line in config.splitlines()
          if line.strip().startswith('MODULE_PATH=')]
 programs=[path for path in modules if path.startswith('boot:///programs/')]
 if not programs:return None
 if programs!=['boot:///programs/record']:
  raise ValueError('unsupported or duplicate installed program module')
 record=root/'programs'/'record'
 if not record.is_file():raise FileNotFoundError('configured recorder module is missing')
 return record
def main():
 p=argparse.ArgumentParser();p.add_argument('--root',type=Path,default=ROOT/'iso_root');p.add_argument('--out',type=Path,required=True);a=p.parse_args()
 img=a.out;img.parent.mkdir(parents=True,exist_ok=True)
 pack=a.root/'firmware'/'wifi.pack'
 config=(a.root/'limine.cfg').read_text()
 record=recorder_module(a.root,config)
 # Recorder images use the existing full boot slot; mcopy still fails closed
 # if modules and firmware exceed it. Never grow the install partition here.
 sectors=SECTORS_WITH_FIRMWARE if record is not None or pack.is_file() and pack.stat().st_size>1<<20 else SECTORS
 with img.open('wb') as f:f.truncate(sectors*512)
 run=lambda *c:subprocess.run(c,check=True,capture_output=True)
 run('mformat','-i',str(img),'-T',str(sectors),'-h','64','-s','32','-v','SEXOS','::')
 with tempfile.TemporaryDirectory() as tmp:
  efi=Path(tmp)/'BOOTX64.EFI'
  run('mcopy','-i',str(a.root/'boot/limine/limine-uefi-cd.bin'),'::/EFI/BOOT/BOOTX64.EFI',str(efi))
  for d in ('::/EFI','::/EFI/BOOT','::/servers','::/apps','::/firmware'):run('mmd','-i',str(img),d)
  run('mcopy','-i',str(img),str(efi),'::/EFI/BOOT/BOOTX64.EFI')
 # The installed system boots without the image it was installed from.
 cfg=Path(tmp_cfg:=str(img)+'.cfg');cfg.write_text(''.join(l for l in config.splitlines(True)
   if 'boot:///boot.img' not in l and (pack.is_file() or 'boot:///firmware/wifi.pack' not in l)))
 run('mcopy','-i',str(img),str(cfg),'::/limine.cfg');cfg.unlink()
 run('mcopy','-i',str(img),str(a.root/'sexos-kernel'),'::/sexos-kernel')
 for sub in ('servers','apps'):
  for f in sorted((a.root/sub).iterdir()):
   if f.is_file():run('mcopy','-i',str(img),str(f),f'::/{sub}/{f.name}')
 if record is not None:
  run('mmd','-i',str(img),'::/programs')
  run('mcopy','-i',str(img),str(record),'::/programs/record')
 if pack.is_file():run('mcopy','-i',str(img),str(pack),'::/firmware/wifi.pack')
 print(img,img.stat().st_size)
if __name__=='__main__':main()
