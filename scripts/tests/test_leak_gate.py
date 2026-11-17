"""Host checks for scripts/leak_gate.py's arithmetic: which way is a leak, and the
tolerances. The QEMU part runs in CI."""
import pathlib
import sys
import unittest
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
from leak_gate import growth, leaks, SAMPLE, FIELDS

BASE = dict(heap_used=1000, heap_mapped=4 << 20, heap_blocks=50, heap_peak=2000, free_frames=10000, free_pages=9000, rows=0, row_pages=0)
TOL = {'heap_bytes': 65536, 'heap_blocks': 24, 'frames': 48, 'row_pages': 0}


class LeakGate(unittest.TestCase):
    def test_same_memory_is_no_leak(self):
        self.assertEqual(leaks(growth(BASE, dict(BASE)), TOL), [])

    def test_memory_given_back_is_no_leak(self):
        after = dict(BASE, heap_used=500, heap_blocks=10, free_frames=10500)
        self.assertEqual(leaks(growth(BASE, after), TOL), [])

    def test_lost_frames_and_blocks_are_leaks(self):
        after = dict(BASE, heap_blocks=80, free_frames=9900, row_pages=1)
        self.assertEqual(sorted(leaks(growth(BASE, after), TOL)), ['frames=100', 'heap_blocks=30', 'row_pages=1'])

    def test_one_time_growth_is_not_a_leak(self):
        grew = {'heap_bytes': 66496, 'heap_blocks': 2, 'frames': 0, 'row_pages': 0}
        steps = [dict(grew), *[{k: 0 for k in grew} for _ in range(3)]]
        self.assertEqual(leaks(grew, TOL, steps), [])

    def test_growth_every_cycle_is_a_leak(self):
        grew = {'heap_bytes': 80000, 'heap_blocks': 4, 'frames': 0, 'row_pages': 0}
        steps = [{'heap_bytes': 20000, 'heap_blocks': 1, 'frames': 0, 'row_pages': 0}] * 4
        self.assertEqual(leaks(grew, TOL, steps), ['heap_bytes=80000'])

    def test_sample_line_parses(self):
        line = '[swatch.sample] heap_used=1 heap_mapped=2 heap_blocks=3 heap_peak=4 free_frames=5 free_pages=6 rows=7 row_pages=8'
        self.assertEqual(dict(zip(FIELDS, map(int, SAMPLE.search(line).groups()))), dict(zip(FIELDS, range(1, 9))))

    def test_sample_matches_swatch_source(self):
        src = (pathlib.Path(__file__).resolve().parents[2] / 'apps/hum/src/bin/swatch.rs').read_text()
        for field in FIELDS:
            self.assertIn(f'{field}={{}}', src)


if __name__ == '__main__':
    unittest.main()
