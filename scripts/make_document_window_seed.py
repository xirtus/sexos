#!/usr/bin/env python3
"""Create benchmark input in a fresh copy of an existing native QJ02 object.

This is host-created fixture data, never evidence of guest creation or saving.
Object identity, extent metadata, and every other object's bytes stay unchanged.
"""
import argparse
import hashlib
import json
import pathlib
import struct
from check_document_disk import fnv, object_bytes, recover


def seed(disk, slot, payload):
    if len(payload) > 12288:
        raise ValueError('payload exceeds document bound')
    raw, identity = object_bytes(disk, slot)
    previous = recover(disk, slot)
    if len(raw) != 65536 or previous['generation'] == 0xffffffffffffffff:
        raise ValueError('requires existing full-capacity native journal')
    desc = disk[(1925-slot)*512:(1926-slot)*512]
    extents = [struct.unpack_from('<HH', desc, 12+i*4) for i in range(struct.unpack_from('<H', desc, 6)[0])]
    occupied = set()
    for start, length in extents:
        sectors = set(range(start, start+length))
        if not length or start < 0 or (start+length)*512 > len(disk) or sectors & occupied:
            raise ValueError('invalid or overlapping extents')
        occupied.update(sectors)
    if sum(length for _, length in extents)*512 < len(raw):
        raise ValueError('short extent mapping')
    bank = 1-previous['bank']; base = bank*32768
    header = bytearray(512); header[:4] = b'QJ02'; header[4] = slot
    struct.pack_into('<I', header, 8, identity)
    struct.pack_into('<Q', header, 16, previous['generation']+1)
    struct.pack_into('<I', header, 24, len(payload))
    struct.pack_into('<Q', header, 32, fnv(payload))
    struct.pack_into('<Q', header, 504, fnv(header[:504]))
    replacement = bytearray(raw)
    replacement[base:base+512] = header
    replacement[base+512:base+512+12288] = payload+bytes(12288-len(payload))
    result = bytearray(disk); offset = 0
    for start, length in extents:
        count = min(length*512, len(replacement)-offset)
        if count <= 0: break
        result[start*512:start*512+count] = replacement[offset:offset+count]; offset += count
    result = bytes(result)
    actual = recover(result, slot)
    if actual['payload'] != payload or actual['identity'] != identity:
        raise ValueError('fixture readback differs')
    return result, {k:v for k,v in actual.items() if k != 'payload'}


def main():
    p=argparse.ArgumentParser();p.add_argument('--source-disk',type=pathlib.Path,required=True);p.add_argument('--out',type=pathlib.Path,required=True);p.add_argument('--object',type=int,default=5);p.add_argument('--bytes',type=int,default=12287);a=p.parse_args()
    if not 0<=a.bytes<=12288:p.error('bytes outside document bound')
    disk=a.source_disk.read_bytes();result,info=seed(disk,a.object,b'x'*a.bytes)
    with a.out.open('xb')as f:f.write(result)
    report=dict(scope='host-created input fixture, not guest save evidence',source_disk_sha256=hashlib.sha256(disk).hexdigest(),fixture_sha256=hashlib.sha256(result).hexdigest(),object=a.object,document=info)
    a.out.with_suffix('.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report))
if __name__=='__main__':main()
