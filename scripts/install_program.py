#!/usr/bin/env python3
"""Writes programs into the disk program store: directory sectors from LBA 2048, then the files."""
import argparse,struct
from pathlib import Path
DIRECTORY_LBA=2048;SECTOR=512;MAGIC=b'SEXPROG1';ENTRY=32;PER_SECTOR=(SECTOR-16)//ENTRY;MAX_SECTORS=64;MAX=PER_SECTOR*MAX_SECTORS
def fnv(data):
 h=0xcbf29ce484222325
 for b in data:h=((h^b)*0x100000001b3)&(2**64-1)
 return h
def install(disk,programs):
 """programs: (name, bytes[, stored hash]). The disk grows to fit; DiskFS sectors stay untouched.
 The directory takes as many sectors as the programs need (15 per sector, up to 64)."""
 assert len(programs)<=MAX,'directory full'
 sectors=max(1,(len(programs)+PER_SECTOR-1)//PER_SECTOR)
 directory=bytearray(SECTOR*sectors)
 for k in range(sectors):directory[k*SECTOR:k*SECTOR+8]=MAGIC
 directory[8:12]=struct.pack('<I',sectors);lba=DIRECTORY_LBA+sectors
 with Path(disk).open('r+b') as f:
  for i,program in enumerate(programs):
   name,data=program[0].encode(),program[1];stored=program[2] if len(program)>2 else fnv(data)
   assert 0<len(name)<=16,'name'
   assert data and len(data)<=16*1024*1024,'size'
   at=(i//PER_SECTOR)*SECTOR+16+(i%PER_SECTOR)*ENTRY
   directory[at:at+ENTRY]=name.ljust(16,b'\0')+struct.pack('<IIQ',lba,len(data),stored)
   f.seek(lba*SECTOR);f.write(data.ljust((len(data)+SECTOR-1)//SECTOR*SECTOR,b'\0'))
   lba+=(len(data)+SECTOR-1)//SECTOR
  f.seek(DIRECTORY_LBA*SECTOR);f.write(directory)
  f.seek(0,2)
  if f.tell()<lba*SECTOR:f.truncate(lba*SECTOR)
def main():
 p=argparse.ArgumentParser();p.add_argument('disk',type=Path);p.add_argument('programs',nargs='+',metavar='NAME=FILE')
 a=p.parse_args()
 install(a.disk,[(s.split('=',1)[0],Path(s.split('=',1)[1]).read_bytes()) for s in a.programs])
if __name__=='__main__':main()
