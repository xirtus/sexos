#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Finds bytes the program loader refuses in code: WRPKRU (0F 01 EF), XRSTOR (0F AE /5,
memory form) and XRSTORS (0F C7 /3, memory form), at any offset, in the executable
segments of a program file (ELF) or the executable sections of an object file.

The same rule as sex_pdx::program_code::refused. A compiler can make them by chance,
for example `rol $0xf,%ebx` followed by `add %ebp,%edi` (c1 c3 0f 01 ef), or a
RIP-relative displacement that happens to hold them.

  key_bytes.py FILE...    prints each hit; exits 1 if any file has one
"""
import subprocess, sys


def refused(b1, b2):
    reg, memory = (b2 >> 3) & 7, (b2 >> 6) != 3
    return (b1 == 0x01 and b2 == 0xEF) or (b1 == 0xAE and reg == 5 and memory) or (b1 == 0xC7 and reg == 3 and memory)


def hits(data):
    return [i for i in range(len(data) - 2) if data[i] == 0x0F and refused(data[i + 1], data[i + 2])]


def ranges(path):
    """(name, file offset, size, address) of each executable piece."""
    data = open(path, 'rb').read(18)
    kind = int.from_bytes(data[16:18], 'little')
    if kind == 1:  # relocatable object: executable sections
        out = subprocess.run(['llvm-readelf', '-SW', path], capture_output=True, text=True, check=True).stdout
        for line in out.splitlines():
            if ']' not in line:
                continue
            parts = line.split(']', 1)[1].split()
            if len(parts) >= 7 and parts[1] == 'PROGBITS' and 'X' in parts[6]:
                yield parts[0], int(parts[3], 16), int(parts[4], 16), 0
    else:  # program: executable LOAD segments
        out = subprocess.run(['llvm-readelf', '-lW', path], capture_output=True, text=True, check=True).stdout
        for line in out.splitlines():
            parts = line.split()
            if parts and parts[0] == 'LOAD' and 'E' in parts[-2]:
                yield 'LOAD', int(parts[1], 16), int(parts[4], 16), int(parts[2], 16)


def scan(path):
    data = open(path, 'rb').read()
    found = []
    for name, off, size, va in ranges(path):
        found += [(name, va + i if va else i) for i in hits(data[off:off + size])]
    return found


def main():
    bad = False
    for path in sys.argv[1:]:
        for name, at in scan(path):
            print(f'{path}: {name} {at:#x}')
            bad = True
    sys.exit(1 if bad else 0)


if __name__ == '__main__':
    main()
