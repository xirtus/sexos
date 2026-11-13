#!/usr/bin/env python3
"""Writes scripts/web_site/fonts/geist-mono.woff: Geist Mono (SIL OFL 1.1, as in
assets/fonts/geist) packed as WOFF 1.0 with zlib tables, for the web font tests."""
import struct,zlib
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
ttf=(ROOT/'assets/fonts/geist/GeistMono-Regular.ttf').read_bytes()
flavor,n=struct.unpack('>IH',ttf[:6])
tables=[struct.unpack('>4sIII',ttf[12+16*i:28+16*i]) for i in range(n)]
head=44+20*n;data=b'';entries=[]
for tag,sum_,off,length in tables:
 raw=ttf[off:off+length];comp=zlib.compress(raw,9)
 if len(comp)>=len(raw):comp=raw
 entries.append((tag,head+len(data),len(comp),length,sum_));data+=comp+b'\0'*(-len(comp)%4)
total=12+16*n+sum((t[3]+3)//4*4 for t in tables)
out=struct.pack('>4sIIHHIHHIIIII',b'wOFF',flavor,head+len(data),n,0,total,1,0,0,0,0,0,0)
out+=b''.join(struct.pack('>4sIIII',*e) for e in entries)+data
(ROOT/'scripts/web_site/fonts/geist-mono.woff').write_bytes(out);print(len(out))
