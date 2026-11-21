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
# The NVIDIA GSP pack (38 MB) stays out: Limine loads this image into RAM on every entry
# (boot:///boot.img), so the pack inside it cost every machine 32 MiB. It is a module of
# the ISO's "SexOS with NVIDIA GPU" entry only, and that entry is left out of installs.
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
NVIDIA_ENTRY=':SexOS with NVIDIA GPU'
def installed_cfg(text,wifi):
 """The ISO's limine.cfg as an installed system boots it: no install image, no Wi-Fi line
 without the pack, and no NVIDIA entry (its firmware lives only on the ISO)."""
 out=[];skip=False
 for l in text.splitlines(True):
  if l.startswith(':'):
   skip=l.strip()==NVIDIA_ENTRY
   # The comment block introducing the NVIDIA entry goes with it.
   while skip and out and out[-1].startswith('#'):out.pop()
  if skip or 'boot:///boot.img' in l or (not wifi and 'boot:///firmware/wifi.pack' in l):continue
  out.append(l)
 return ''.join(out)

def recorder_module(root, config):
 seen_in_entry=False
 configured=False
 for line in config.splitlines():
  line=line.strip()
  if line.startswith(':'):
   seen_in_entry=False
  elif line.startswith('MODULE_PATH='):
   path=line.split('=',1)[1].strip()
   if not path.startswith('boot:///programs/'):continue
   if path!='boot:///programs/record' or seen_in_entry:
    raise ValueError('unsupported or duplicate installed program module in entry')
   seen_in_entry=True;configured=True
 if not configured:return None
 record=root/'programs'/'record'
 if not record.is_file():raise FileNotFoundError('configured recorder module is missing')
 return record
def main():
 p=argparse.ArgumentParser();p.add_argument('--root',type=Path,default=ROOT/'iso_root');p.add_argument('--out',type=Path,required=True);a=p.parse_args()
 img=a.out;img.parent.mkdir(parents=True,exist_ok=True)
 pack=a.root/'firmware'/'wifi.pack'
 installed=installed_cfg((a.root/'limine.cfg').read_text(),pack.is_file())
 record=recorder_module(a.root,installed)
 # Everything copied in, plus room for FAT tables and the loader.
 files=[a.root/'sexos-kernel']+[f for sub in ('servers','apps') for f in (a.root/sub).iterdir() if f.is_file()]+([pack] if pack.is_file() else [])+([record] if record is not None else [])
 firmware_bytes=pack.stat().st_size if pack.is_file() else 0
 sectors=sectors_for([f.stat().st_size for f in files],firmware_bytes)
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
 cfg=Path(tmp_cfg:=str(img)+'.cfg');cfg.write_text(installed)
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
