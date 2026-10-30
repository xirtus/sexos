#!/usr/bin/env python3
"""Join actual BOOT document view grants with native leaf/alias/scrub selftest."""
import argparse
import json
import re
from pathlib import Path


def inspect(text):
    errors = []
    grants = []
    positions = []
    for match in re.finditer(r'\[boot.memory.region\] pd=(\d+) slot=(\d+) region=(\d+) bytes=(\d+) rights=(\d+) peer=(\d+)', text):
        row = tuple(map(int, match.groups()))
        if row[1] in (64, 65, 66):
            grants.append(row)
            positions.append(match.start())
    expected = []
    for index, client in enumerate([9, 16, 18]):
        expected.extend([(11, 64 + index, 6 + index, 16384, 7, client),
                         (client, 64, 6 + index, 16384, 3, 11)])
    if grants != expected:
        errors.append('missing/duplicate/wrong exact document capability grant')
    markers = list(re.finditer(r'\[boot.document.views\] pairs=3 bytes_each=16384 total_bytes=49152 readonly=1 distinct=1 scrubbed=1 runtime_mint=0', text))
    seals = list(re.finditer(r'\[boot.memory.authority.sealed\]', text))
    if len(markers) != 1 or len(seals) != 1 or not positions or not (max(positions) < markers[0].start() < seals[0].start()):
        errors.append('all grants and actual selftest must precede the unique seal')
    if re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at', text):
        errors.append('fatal guest fault')
    return dict(ok=not errors, errors=errors, grants=grants,
                scope='Normal profile: three distinct permanent BOOT backings; actual query/leaf/alias/scrub assertions before admission. Transport, hostile containment and reclamation require separate checks.')


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('log', type=Path)
    args = parser.parse_args()
    result = inspect(args.log.read_text(errors='replace'))
    print(json.dumps(result, indent=2))
    return not result['ok']


if __name__ == '__main__':
    raise SystemExit(main())
