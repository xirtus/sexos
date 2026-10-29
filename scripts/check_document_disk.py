"""Read-only independent QJ02 decoder for native disk evidence."""
import struct

def fnv(data):
 h=0xcbf29ce484222325
 for b in data:h=((h^b)*0x100000001b3)&0xffffffffffffffff
 return h

def object_bytes(disk,slot):
 if not 2<=slot<15:raise ValueError('not a workspace document')
 manifest=disk[2046*512:2047*512];entry=manifest[16+slot*32:48+slot*32]
 if not entry or entry[0]==0:raise ValueError('object not live')
 size=struct.unpack_from('<I',entry,24)[0];identity=((slot+1)<<16)|struct.unpack_from('<H',entry,30)[0]
 descriptor=disk[(1925-slot)*512:(1926-slot)*512]
 if descriptor[:4]!=b'SFEX':raise ValueError('bad descriptor')
 count=struct.unpack_from('<H',descriptor,6)[0]
 if count>8:raise ValueError('extent overflow')
 raw=b''.join(disk[start*512:(start+length)*512]for start,length in(struct.unpack_from('<HH',descriptor,12+i*4)for i in range(count)))[:size]
 if len(raw)!=size:raise ValueError('truncated object')
 return raw,identity

def recover(disk,slot):
 raw,identity=object_bytes(disk,slot);candidates=[]
 for bank in (0,1):
  h=raw[bank*32768:bank*32768+512]
  if len(h)!=512 or h[:4]!=b'QJ02' or h[4]!=slot or struct.unpack_from('<I',h,8)[0]!=identity:continue
  if fnv(h[:504])!=struct.unpack_from('<Q',h,504)[0]:continue
  generation=struct.unpack_from('<Q',h,16)[0];length=struct.unpack_from('<I',h,24)[0];hash_=struct.unpack_from('<Q',h,32)[0]
  payload=raw[bank*32768+512:bank*32768+512+length]
  if not generation or length>12288 or len(payload)!=length or fnv(payload)!=hash_:continue
  candidates.append(dict(bank=bank,generation=generation,identity=identity,bytes=length,hash=hex(hash_),payload=payload))
 if not candidates:raise ValueError('no complete QJ02 bank')
 candidates.sort(key=lambda c:c['generation'])
 if len(candidates)>1 and candidates[-1]['generation']==candidates[-2]['generation']:raise ValueError('ambiguous generation')
 return candidates[-1]
