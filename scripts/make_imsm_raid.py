#!/usr/bin/env python3
"""Make disk images that look like members of an Intel RST (IMSM) RAID set.

The layout facts (structure sizes and offsets, checksum, where the metadata
sits) are the ones Linux mdadm and dmraid read; this script writes them from
scratch. Check what it makes with:

    losetup -f --show d0.img            # one loop device per member
    IMSM_NO_PLATFORM=1 mdadm --examine /dev/loopN
    dmraid -r ; dmraid -s

Used by the SexOS RAID gate (two NVMe disks in RAID 0) and to refresh the
fixtures in crates/sex-imsm/tests/fixtures.
"""
import argparse
import os
import struct
import sys

SIG = b"Intel Raid ISM Cfg Sig. "
SECTOR = 512
# Space at the end of every member kept for the metadata (mdadm keeps the same).
RESERVED_SECTORS = 8192 + 2210


def disk_entry(serial: bytes, total: int, status: int) -> bytes:
    # Stored as mdadm and RST store it: no spaces or control bytes, ':' -> ';', last 16.
    serial = bytes(b";"[0] if c == b":"[0] else c for c in serial if c > 0x20)[-16:].ljust(16, b"\0")
    return serial + struct.pack("<IIIIIIII", total & 0xFFFFFFFF, 0, status, 0, total >> 32, 0, 0, 0)


def dev_entry(name: bytes, size: int, level: int, members: list, pba: int, bpm: int, strip: int, migrating=False) -> bytes:
    name = name[:16].ljust(16, b"\0")
    head = name + struct.pack("<IIII", size & 0xFFFFFFFF, size >> 32, 0, 0)
    head += b"\0" * (80 - len(head))
    vol = struct.pack("<IIBBBBHHI", 0, 0, 1 if migrating else 0, 0, 0, 0xFF, 0, 0, 0) + b"\0" * 12
    stripes = bpm // strip
    m = struct.pack("<IIIHBBBBBB", pba & 0xFFFFFFFF, bpm & 0xFFFFFFFF, stripes & 0xFFFFFFFF, strip,
                    0, level, len(members), 2 if level == 1 else 1, 0xFF, 0)
    m += struct.pack("<III", pba >> 32, bpm >> 32, stripes >> 32) + b"\0" * 16
    m += b"".join(struct.pack("<I", i) for i in members)
    out = head + vol + m
    if migrating:
        out += m  # the second map of a migrating volume
    return out


def mpb(serials, total, volumes, family=0x5E05_1234, version=b"1.0.00") -> bytes:
    disks = b"".join(disk_entry(s, total, 0x02) for s in serials)  # CONFIGURED_DISK
    devs = b"".join(volumes)
    size = 0xD8 + len(disks) + len(devs)
    head = (SIG + version).ljust(32, b"\0")
    attrs = 0x80000000 | 0x1 | 0x2
    rest = struct.pack("<IIIIBBBBIIIIHHQ", family, 1, 0, attrs, len(serials), len(volumes), 0, 0,
                       0, family, 0, 0, len(volumes), 0, 0)
    body = head + struct.pack("<II", 0, size) + rest
    body += b"\0" * (0xD8 - len(body)) + disks + devs
    assert len(body) == size
    body += b"\0" * (-len(body) % 4)
    total_sum = sum(struct.unpack("<%dI" % (size // 4), body[: size // 4 * 4])) & 0xFFFFFFFF
    body = body[:32] + struct.pack("<I", total_sum) + body[36:]
    return body


def place(path, total_sectors, blob):
    """Anchor (first sector of the MPB) at the second-to-last sector; any further
    MPB sectors go just before it, in order, as mdadm reads them."""
    sectors = (len(blob) + SECTOR - 1) // SECTOR
    blob = blob.ljust(sectors * SECTOR, b"\0")
    with open(path, "r+b") as f:
        f.seek((total_sectors - 2) * SECTOR)
        f.write(blob[:SECTOR])
        if sectors > 1:
            f.seek((total_sectors - 1 - sectors) * SECTOR)
            f.write(blob[SECTOR:])


def main():
    p = argparse.ArgumentParser()
    p.add_argument("out", nargs="+", help="member image paths, in member order")
    p.add_argument("--mib", type=int, default=64, help="size of each member")
    p.add_argument("--level", type=int, choices=(0, 1), default=0)
    p.add_argument("--strip-kib", type=int, default=128)
    p.add_argument("--serial", default="SEXRAID", help="serial prefix; member i gets PREFIX-i")
    p.add_argument("--name", default="Volume0")
    p.add_argument("--volume", help="file whose bytes become the start of the RAID volume")
    p.add_argument("--matrix", action="store_true",
                   help="Intel Matrix RAID: the first half of each member is the volume, the rest a second RAID 1 volume")
    p.add_argument("--pattern", action="store_true", help="fill the first 4096 volume sectors with LBA ^ 0x5345585241494400, eight bytes at a time")
    a = p.parse_args()

    total = a.mib * 1024 * 1024 // SECTOR
    strip = a.strip_kib * 1024 // SECTOR
    n = len(a.out)
    usable = (total - RESERVED_SECTORS) // strip * strip
    bpm = usable // 2 // strip * strip if a.matrix else usable
    size = bpm * n if a.level == 0 else bpm
    serials = [("%s-%d" % (a.serial, i)).encode() for i in range(n)]
    vols = [dev_entry(a.name.encode(), size, a.level, list(range(n)), 0, bpm, strip)]
    if a.matrix:
        vols.append(dev_entry(b"Volume1", usable - bpm, 1, list(range(n)), bpm, usable - bpm, strip))
    blob = mpb(serials, total, vols, version=b"1.0.00" if a.level == 0 and not a.matrix else b"1.1.00")

    for path in a.out:
        with open(path, "wb") as f:
            f.truncate(total * SECTOR)

    data = b""
    if a.volume:
        with open(a.volume, "rb") as f:
            data = f.read()
    if a.pattern and not data:
        data = b"".join(struct.pack("<Q", lba ^ 0x5345_5852_4149_4400) * 64 for lba in range(min(size, 4096)))
    data = data[: size * SECTOR]
    files = [open(path, "r+b") for path in a.out]
    for lba in range(0, (len(data) + SECTOR - 1) // SECTOR, strip):
        chunk = data[lba * SECTOR: (lba + strip) * SECTOR]
        if a.level == 0:
            s = lba // strip
            member, row = s % n, s // n
            files[member].seek(row * strip * SECTOR)
            files[member].write(chunk)
        else:
            for f in files:
                f.seek(lba * SECTOR)
                f.write(chunk)
    for f in files:
        f.close()
    for path in a.out:
        place(path, total, blob)
    print("imsm level=%d members=%d strip_sectors=%d member_sectors=%d volume_sectors=%d mpb_bytes=%d"
          % (a.level, n, strip, total, size, len(blob)))


if __name__ == "__main__":
    sys.exit(main())
