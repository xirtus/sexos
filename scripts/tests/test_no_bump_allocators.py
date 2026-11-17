"""No program or service may ship a heap that never frees (a "bump" allocator): every
freed block would stay lost until the run ends, so a long session leaks without bound.
An allocator that hands out nothing (alloc returns null, for code with no heap) is fine."""
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
# Retired by their own workstreams; remove an entry when that lands.
ALLOWED = {
    'apps/qupid/src/main.rs',  # WS7 retires the stub; the player is apps/play.
    'crates/polyamour-sdk/src/transport.rs',  # wasm guests: one short call each.
}
EMPTY_DEALLOC = re.compile(r'fn\s+dealloc\s*\([^)]*\)\s*\{\s*\}')
ALLOC_HEAD = re.compile(r'fn\s+alloc\s*\([^)]*\)\s*->\s*\*mut\s+u8\s*\{')


def alloc_bodies(text):
    for m in ALLOC_HEAD.finditer(text):
        depth, i = 1, m.end()
        while depth and i < len(text):
            depth += {'{': 1, '}': -1}.get(text[i], 0)
            i += 1
        yield text[m.end():i - 1]


def offenders():
    found = []
    for top in ('servers', 'apps', 'crates'):
        for path in (ROOT / top).rglob('*.rs'):
            if 'target' in path.parts or 'vendor' in path.parts:
                continue
            text = path.read_text(errors='replace')
            if 'GlobalAlloc for' not in text or not EMPTY_DEALLOC.search(text):
                continue
            rel = path.relative_to(ROOT).as_posix()
            bodies = list(alloc_bodies(text))
            # The no-heap pattern: alloc only ever returns null.
            if bodies and all(re.fullmatch(r'\s*(core::)?ptr::null_mut\(\)\s*', b) for b in bodies):
                continue
            if rel not in ALLOWED:
                found.append(rel)
    return found


class NoBumpAllocators(unittest.TestCase):
    def test_every_heap_frees(self):
        self.assertEqual(offenders(), [], 'use talc (see crates/sex-app/src/heap.rs) instead of a never-freeing heap')

    def test_allowlist_is_live(self):
        for rel in ALLOWED:
            self.assertTrue((ROOT / rel).exists(), f'{rel} is gone: drop it from ALLOWED')


if __name__ == '__main__':
    unittest.main()
