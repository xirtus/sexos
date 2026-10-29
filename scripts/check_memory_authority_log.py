#!/usr/bin/env python3
"""Check observed boot grants and migrated consumers, not hostile-code isolation."""
import argparse
from pathlib import Path
import re
import sys


def validate(text):
    errors = []
    grants = list(re.finditer(r"\[boot\.memory\.region\] pd=(\d+) slot=(\d+) region=(\d+) bytes=(\d+) rights=(\d+) peer=(\d+)", text))
    seals = list(re.finditer(r"\[boot\.memory\.authority\.sealed\] runtime_alloc=0 runtime_map=capability_leaf runtime_mint=0", text))
    running = re.search(r"task\.running id=", text)
    if len(seals) != 1 or not running or not grants:
        errors.append("missing/duplicate seal, grants or actual task execution")
    elif not (max(g.start() for g in grants) < seals[0].start() < running.start()):
        errors.append("region genesis must precede seal and task execution")
    rows = [tuple(map(int, g.groups())) for g in grants]
    if len(rows) != 7 or len({(r[0], r[1]) for r in rows}) != len(rows):
        errors.append("missing/duplicate fixed boot region slots")
    for region, size in ((1, 262144), (2, 262144), (3, 524288)):
        matches = [r for r in rows if r[2] == region]
        if len(matches) != 1 or matches[0][1:] != (32, region, size, 15, 0):
            errors.append(f"wrong DMA grant budget or rights for region {region}")
    for region, slot, size, rights in ((4, 17, 4096, [7, 7]), (5, 33, 3145728, [3, 7])):
        pair = [r for r in rows if r[2] == region]
        if len(pair) != 2 or sorted(r[4] for r in pair) != rights or any(r[1] != slot or r[3] != size for r in pair):
            errors.append(f"wrong paired region {region} extent or rights")
        elif pair[0][0] == pair[1][0] or pair[0][5] != pair[1][0] or pair[1][5] != pair[0][0]:
            errors.append(f"paired region {region} peer identity mismatch")
    drive = [r[0] for r in rows if r[2] == 3]
    if len(drive) == 1:
        producer = drive[0]
        frame = [r for r in rows if r[2] == 5 and r[0] == producer]
        block = [r for r in rows if r[2] == 4 and r[0] == producer]
        if len(frame) != 1 or frame[0][4] != 7 or len(block) != 1:
            errors.append("drive must own writable frame and paired block views")
        consumers = list(re.finditer(r"\[sexfiles\.bridge\.diskfs\.buf\.ready\] buf_va=(0x[0-9a-fA-F]+) region_id=(\d+) length=(\d+) peer_pd=(\d+)", text))
        if not consumers or any(int(c[1], 16) == 0 or tuple(map(int, c.groups()[1:])) != (4, 4096, producer) for c in consumers):
            errors.append("sexfiles resolved view must match actual drive peer and block grant")
    negative = list(re.finditer(r"\[memory\.authority\.negative\.selftest\] denied=15 idempotent=1024 output_zero=1 ok=1", text))
    if len(negative) != 1 or (seals and negative[0].start() > seals[0].start()):
        errors.append("missing/duplicate boot region-query denials and idempotent view proof")
    for marker in (
        r"\[sexusb\.memory\.dma\.ready\] region_id=1 length=262144",
        r"\[sexdrive\.dma\.region\] slot=32 bytes=524288 reusable_buffers=2",
        r"\[sexfiles\.bridge\.diskfs\.buf\.ready\].*region_id=4 length=4096 peer_pd=\d+",
        r"\[sexfiles\.memory\.authority\.negative\] empty=1 wrong_kind=1 dma_right=1 ok=1",
        r"\[runtime\.heap\.ready\] backing=elf_owned bytes=65536 runtime_mapping=0",
    ):
        if not re.search(marker, text):
            errors.append("missing migrated consumer proof: " + marker)
    if re.search(r"panic|PAGE FAULT|GP FAULT|DOUBLE FAULT|memory\.authority.*FAIL|mem\.overlap\.detected|dma\.quarantine", text, re.I):
        errors.append("fault, allocator overlap or unresolved DMA quarantine")
    return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("log", type=Path)
    args = parser.parse_args()
    errors = validate(args.log.read_text(errors="replace"))
    for error in errors:
        print("[memory.authority.runtime] FAIL " + error)
    if not errors:
        print("[memory.authority.runtime] PASS scope=observed_boot_grants_dispatch_and_consumers")
    return int(bool(errors))


if __name__ == "__main__":
    sys.exit(main())
