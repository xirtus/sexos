#!/usr/bin/env python3
"""List the kernel's direct user-memory accesses that SMAP audit mode found.

The kernel logs each new site once per boot as
`[smap.site] rip=.. from_audit=.. addr=.. write=..` (kernel/src/smap.rs). This
collects them from every log under the given directories, joins them across
logs by their offset from `smap::audit`, and names each one's function and line
with addr2line on the kernel ELF. Informational: it always exits 0.
"""
import argparse, os, re, subprocess
from pathlib import Path

LINE = re.compile(rb'\[smap\.site\] rip=(0x[0-9a-f]+) from_audit=(-?\d+) addr=(0x[0-9a-f]+) write=(\d)(?: callers=(-?\d+),(-?\d+),(-?\d+))?')

def logs(roots, limit=64 << 20):
    for root in roots:
        for d, _, files in os.walk(root):
            for f in files:
                p = Path(d, f)
                try:
                    if p.suffix not in ('.img', '.iso', '.qcow2', '.zst') and p.stat().st_size <= limit:
                        yield p
                except OSError:
                    pass

def audit_address(kernel):
    out = subprocess.run(['nm', kernel], capture_output=True, text=True).stdout
    for line in out.splitlines():
        parts = line.split()
        if len(parts) == 3 and re.search(r'4smap5audit', parts[2]):
            return int(parts[0], 16)
    return None

def main():
    a = argparse.ArgumentParser()
    a.add_argument('--kernel', type=Path)
    a.add_argument('roots', nargs='+')
    a = a.parse_args()
    sites = {}
    for p in logs(a.roots):
        try:
            data = p.read_bytes()
        except OSError:
            continue
        for m in LINE.finditer(data):
            rel = int(m[2])
            s = sites.setdefault(rel, {'hits': 0, 'write': False, 'addr': m[3].decode(), 'logs': set(),
                                       'callers': [int(c) for c in (m[5], m[6], m[7]) if c and int(c) != 0]})
            s['hits'] += 1
            s['write'] |= m[4] == b'1'
            s['logs'].add(p.parent.name)
    print(f'[smap.sites] distinct={len(sites)}')
    if not sites:
        return 0
    base = audit_address(a.kernel) if a.kernel and a.kernel.exists() else None
    names = {}
    if base is not None:
        rels = sorted({r for rel, s in sites.items() for r in (rel, *s['callers'])})
        # Without -i, addr2line prints one function line and one file:line per address.
        out = subprocess.run(['addr2line', '-f', '-C', '-e', str(a.kernel), *(hex(base + r) for r in rels)],
                             capture_output=True, text=True).stdout.splitlines()
        for rel, i in zip(rels, range(0, len(out), 2)):
            names[rel] = out[i]
    for rel, s in sorted(sites.items(), key=lambda kv: -kv[1]['hits']):
        print(f"[smap.sites] from_audit={rel} hits={s['hits']} write={int(s['write'])} "
              f"addr={s['addr']} where={names.get(rel, '?')} "
              f"callers={'|'.join(names.get(c, '?') for c in s['callers']) or '-'} seen_in={','.join(sorted(s['logs']))[:120]}")
    return 0

if __name__ == '__main__':
    raise SystemExit(main())
