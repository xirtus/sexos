#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Fix Limine 7.13.3's BIOS boot-menu countdown hang (one instruction in stage 2).

Limine's BIOS `_pit_sleep_and_quit_on_keypress` (common/lib/sleep.asm_bios_ia32)
counts IRQ0 ticks and leaves its loop on

    cmp dword [int_08_ticks_counter], edx
    je .done

With SERIAL=yes the menu sleeps one tick at a time, so the loop waits for the
counter to be exactly 1. When two PIT ticks land during one `int 0x16` keyboard
poll (a busy QEMU TCG host delivers late ticks back to back) the counter goes
0 -> 2, never equals 1, and the menu sits at "Booting automatically in 1" until
a key is pressed. That is the pre-kernel hang seen in CI (std programs, RAID,
Splicy Studio gates). Upstream Limine later made this `jae` (plus a TSC
deadline); this rewrites the one `je` (0x74) as `jae` (0x73).

The routine lives in stage 2 (`.realmode`), which BIOS boots take from a gzip
stream inside limine-bios-cd.bin (CD) or the `limine` tool's embedded
limine-bios-hdd.h (disks, hybrid ISO), not from limine-bios.sys. All three are
patched: the stream is inflated, patched, deflated again and zero-padded to its
old size (Limine's decompressor checks neither CRC nor length and stops at the
last deflate block).

Usage: patch_limine_bios.py [LIMINE_DIR_OR_FILES...]   (default: limine/)
Idempotent; refuses files it does not recognise.
"""
import hashlib
import re
import sys
import zlib
from pathlib import Path

# Limine ca3a5b20 (v7.13.3 binary release): the original files.
KNOWN = {
    "limine-bios.sys": "66f02e013fe044f3a954cb72474c29796f6541d990019c23269068100044f556",
    "limine-bios-cd.bin": "7ece36a098644baa2d4e8858cfe552b9f1e4825a85abb859249e2062a9693f34",
    "limine-bios-hdd.h": "af8c911369ddca5e39154b708fafbf4e82ee329cdb5d3d6f3e2b701b3abda418",
}
# Its uncompressed stage 2, before and after the fix.
STAGE2 = "fd61797d73ef6978a6ec1293efa8f2b3b7e43d4edf847e6ce80807b66225e3eb"
STAGE2_FIXED = "ae6637d17e6df239b442527dfa563b4f0d2715eb2df9572a83a63e9b230b6f76"
# cmp dword [disp16], edx ; je/jae rel8 ; push ecx ; push edx ; mov ah, 1
LOOP = re.compile(rb"\x66\x39\x16..([\x73\x74]).\x66\x51\x66\x52\xb4\x01", re.S)
GZIP = b"\x1f\x8b\x08"


def fix_loop(code: bytes) -> bytes:
    hits = list(LOOP.finditer(code))
    if len(hits) != 1:
        raise ValueError(f"expected one countdown loop, found {len(hits)}")
    at = hits[0].start(1)
    return code[:at] + b"\x73" + code[at + 1:]


def fix_stream(image: bytes) -> bytes:
    """Patch the gzip-compressed stage 2 inside a boot image, keeping its size."""
    if image.count(GZIP) != 1:
        raise ValueError("expected one gzip stage 2 stream")
    start = image.index(GZIP)
    inflate = zlib.decompressobj(31)
    stage2 = inflate.decompress(image[start:])
    size = len(image) - start - len(inflate.unused_data)
    digest = hashlib.sha256(stage2).hexdigest()
    if digest == STAGE2_FIXED:
        return image
    if digest != STAGE2:
        raise ValueError(f"unknown stage 2 (sha256 {digest})")
    fixed = fix_loop(stage2)
    best = None
    for level in range(9, 0, -1):
        for mem in range(9, 0, -1):
            deflate = zlib.compressobj(level, zlib.DEFLATED, -15, mem)
            body = deflate.compress(fixed) + deflate.flush()
            if best is None or len(body) < len(best):
                best = body
    trailer = zlib.crc32(fixed).to_bytes(4, "little") + len(fixed).to_bytes(4, "little")
    stream = image[start:start + 10] + best + trailer
    if len(stream) > size:
        raise ValueError(f"patched stage 2 does not fit ({len(stream)} > {size} bytes)")
    stream += bytes(size - len(stream))
    out = image[:start] + stream + image[start + size:]
    check = zlib.decompressobj(31).decompress(out[start:])
    assert hashlib.sha256(check).hexdigest() == STAGE2_FIXED
    return out


def c_array(text: str) -> bytes:
    return bytes(int(x, 16) for x in re.findall(r"0x([0-9a-fA-F]{2})", text))


def to_c_array(text: str, data: bytes) -> str:
    head = text[:text.index("{") + 1]
    rows = [", ".join(f"0x{b:02x}" for b in data[i:i + 16]) + "," for i in range(0, len(data), 16)]
    return head + "\n" + "\n".join("    " + r for r in rows) + "\n};\n"


def patch_file(path: Path) -> str:
    raw = path.read_bytes()
    name = path.name
    if name not in KNOWN:
        raise ValueError(f"{path}: not a Limine BIOS file this knows")
    if name == "limine-bios.sys":
        code = raw
        hits = list(LOOP.finditer(code))
        if len(hits) == 1 and hits[0].group(1) == b"\x73":
            return "already fixed"
        if hashlib.sha256(raw).hexdigest() != KNOWN[name]:
            raise ValueError(f"{path}: unknown version; check the Limine pin")
        new = fix_loop(code)
    elif name == "limine-bios-cd.bin":
        new = fix_stream(raw)
    else:
        text = raw.decode()
        data = c_array(text)
        fixed = fix_stream(data)
        new = raw if fixed == data else to_c_array(text, fixed).encode()
    if new == raw:
        return "already fixed"
    path.write_bytes(new)
    return "fixed countdown je->jae"


def main(args):
    paths = []
    for a in map(Path, args or ["limine"]):
        paths += [a / n for n in KNOWN if (a / n).exists()] if a.is_dir() else [a]
    if not paths:
        raise SystemExit("no Limine BIOS files found")
    for p in paths:
        print(f"[limine.patch] {p}: {patch_file(p)}")


if __name__ == "__main__":
    main(sys.argv[1:])
