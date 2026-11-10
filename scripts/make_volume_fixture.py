#!/usr/bin/env python3
"""Packs a disk image into the sparse fixture format the sex-volumes tests read:
b"SXSPARSE", total sectors (u64 LE), then (LBA u64 LE, 512 bytes) per non-zero sector."""
import struct,sys
src,dst=sys.argv[1],sys.argv[2]
data=open(src,'rb').read()
assert len(data)%512==0,len(data)
out=[b'SXSPARSE',struct.pack('<Q',len(data)//512)]
zero=bytes(512)
for i in range(len(data)//512):
    s=data[i*512:(i+1)*512]
    if s!=zero: out.append(struct.pack('<Q',i)+s)
open(dst,'wb').write(b''.join(out))
