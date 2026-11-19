#!/usr/bin/env python3
# GPL-3.0-only
"""Unit tests for 12-Hour CI-able Long-Run Soak Gate (scripts/long_soak_gate.py).

Verifies:
- SoakAnalyzer baseline tracking and leak threshold enforcement.
- Ordinary-Least-Squares (OLS) regression slope and steady leak projection.
- Multi-segment report continuity (carrying forward baseline, round counter, and canaries).
- Key latency watchdog enforcement.
- Structured failure diagnostics (JSON report and Markdown step summary).
- Structural validity and configuration of .github/workflows/long-soak.yml.
"""

import json
import subprocess
import tempfile
import unittest
from pathlib import Path

from scripts.long_soak_gate import SoakAnalyzer, generate_summary_markdown

ROOT = Path(__file__).resolve().parents[2]


class TestSoakAnalyzer(unittest.TestCase):
    def test_warmup_and_baseline(self):
        analyzer = SoakAnalyzer(warmup=3, leak_pages=256, max_key_ms=3000.0)
        # Round 1: 100000 pages
        analyzer.record_round({'n': 1, 'free_pages': 100000, 'elapsed_s': 60, 'key_ms': 50.0})
        self.assertEqual(analyzer.baseline_pages, 100000)

        # Round 2: 100200 pages (new high baseline)
        analyzer.record_round({'n': 2, 'free_pages': 100200, 'elapsed_s': 120, 'key_ms': 45.0})
        self.assertEqual(analyzer.baseline_pages, 100200)

        # Round 3: 100100 pages (still in warmup)
        analyzer.record_round({'n': 3, 'free_pages': 100100, 'elapsed_s': 180, 'key_ms': 48.0})
        self.assertEqual(analyzer.baseline_pages, 100200)

        # Round 4: 100300 pages (post-warmup, baseline frozen at 100200)
        r4 = {'n': 4, 'free_pages': 100300, 'elapsed_s': 240, 'key_ms': 42.0}
        analyzer.record_round(r4)
        self.assertEqual(analyzer.baseline_pages, 100200)
        ok, reason = analyzer.check_round_health(r4)
        self.assertTrue(ok)
        self.assertIsNone(reason)

    def test_leak_threshold_breach(self):
        analyzer = SoakAnalyzer(warmup=2, leak_pages=100, max_key_ms=3000.0)
        analyzer.record_round({'n': 1, 'free_pages': 50000, 'elapsed_s': 60, 'key_ms': 50.0})
        analyzer.record_round({'n': 2, 'free_pages': 50000, 'elapsed_s': 120, 'key_ms': 50.0})
        self.assertEqual(analyzer.baseline_pages, 50000)

        # Round 3: drops by 50 pages (<= 100 -> PASS)
        r3 = {'n': 3, 'free_pages': 49950, 'elapsed_s': 180, 'key_ms': 50.0}
        analyzer.record_round(r3)
        ok, reason = analyzer.check_round_health(r3)
        self.assertTrue(ok)
        self.assertIsNone(reason)

        # Round 4: drops by 150 pages (> 100 -> FAIL)
        r4 = {'n': 4, 'free_pages': 49850, 'elapsed_s': 240, 'key_ms': 50.0}
        analyzer.record_round(r4)
        ok, reason = analyzer.check_round_health(r4)
        self.assertFalse(ok)
        self.assertIn("dropped 150 pages below baseline 50000", reason)

    def test_key_latency_threshold_breach(self):
        analyzer = SoakAnalyzer(warmup=2, leak_pages=256, max_key_ms=200.0)
        r1 = {'n': 1, 'free_pages': 50000, 'elapsed_s': 60, 'key_ms': 150.0}
        analyzer.record_round(r1)
        ok, reason = analyzer.check_round_health(r1)
        self.assertTrue(ok)

        # Latency exceeded
        r2 = {'n': 2, 'free_pages': 50000, 'elapsed_s': 120, 'key_ms': 250.0}
        analyzer.record_round(r2)
        ok, reason = analyzer.check_round_health(r2)
        self.assertFalse(ok)
        self.assertIn("key latency 250.0 ms exceeded max threshold 200.0 ms", reason)

    def test_ols_slope_mathematics(self):
        analyzer = SoakAnalyzer(warmup=1, leak_pages=500)
        # Synthetic steady drop: -10 pages per round, 360 seconds (0.1 hr) per round
        for n in range(1, 6):
            analyzer.record_round({
                'n': n,
                'free_pages': 1000 - 10 * n,
                'elapsed_s': n * 360.0,
                'key_ms': 50.0,
            })

        slope_info = analyzer.compute_ols_slope()
        self.assertAlmostEqual(slope_info['slope_per_round'], -10.0, places=3)
        self.assertAlmostEqual(slope_info['slope_per_hour'], -100.0, places=1)
        self.assertAlmostEqual(slope_info['r_squared'], 1.0, places=3)
        self.assertEqual(slope_info['min_free_pages'], 950)
        self.assertEqual(slope_info['max_drop'], 40)

    def test_steady_slow_leak_projection(self):
        analyzer = SoakAnalyzer(warmup=2, leak_pages=500)
        # 8 rounds with steady drop projecting > 500 pages over 12h
        for n in range(1, 9):
            round_data = {
                'n': n,
                'free_pages': 50000 - 15 * n,
                'elapsed_s': n * 600.0,  # 1 round every 10 min (6/hr)
                'key_ms': 40.0,
            }
            analyzer.record_round(round_data)
            if n == 8:
                ok, reason = analyzer.check_round_health(round_data)
                self.assertFalse(ok)
                self.assertIn("steady linear memory leak detected via OLS regression", reason)


class TestMultiSegmentContinuity(unittest.TestCase):
    def test_resume_preserves_baseline_and_telemetry(self):
        # Simulate Segment 1 output
        seg1_analyzer = SoakAnalyzer(warmup=3, leak_pages=256)
        for n in range(1, 6):
            seg1_analyzer.record_round({
                'n': n,
                'free_pages': 80000 - (n % 2) * 20,
                'elapsed_s': n * 120.0,
                'key_ms': 35.0,
            })

        seg1_report = {
            'ok': True,
            'why': '',
            'completed': False,
            'hours': 0.17,
            'elapsed_seconds': 600.0,
            'total_rounds': 5,
            'baseline_pages': seg1_analyzer.baseline_pages,
            'canary_files': {'canary_h1.txt': 'abc12345'},
            'rounds': seg1_analyzer.rounds,
        }

        with tempfile.NamedTemporaryFile('w', suffix='.json') as f:
            f.write(json.dumps(seg1_report))
            f.flush()

            # Resume in Segment 2
            loaded_data = json.loads(Path(f.name).read_text())
            seg2_analyzer = SoakAnalyzer(
                warmup=3,
                leak_pages=256,
                baseline_pages=loaded_data['baseline_pages'],
                cumulative_rounds=loaded_data['rounds'],
            )

            # Baseline must be preserved exactly
            self.assertEqual(seg2_analyzer.baseline_pages, 80000)
            self.assertEqual(len(seg2_analyzer.rounds), 5)

            # Round 6 executes in Segment 2
            r6 = {
                'n': 6,
                'free_pages': 79900,  # 100 pages below baseline 80000 (<= 256 -> PASS)
                'elapsed_s': 720.0,
                'key_ms': 40.0,
                'resumed': True,
            }
            seg2_analyzer.record_round(r6)
            ok, reason = seg2_analyzer.check_round_health(r6)
            self.assertTrue(ok)
            self.assertIsNone(reason)
            self.assertEqual(len(seg2_analyzer.rounds), 6)

            # Round 7 leaks beyond baseline (300 pages below 80000 -> FAIL)
            r7 = {
                'n': 7,
                'free_pages': 79700,
                'elapsed_s': 840.0,
                'key_ms': 40.0,
                'resumed': True,
            }
            seg2_analyzer.record_round(r7)
            ok, reason = seg2_analyzer.check_round_health(r7)
            self.assertFalse(ok)
            self.assertIn("dropped 300 pages below baseline 80000", reason)


class TestDiagnosticReporting(unittest.TestCase):
    def test_summary_markdown_generation(self):
        analyzer = SoakAnalyzer(warmup=2, leak_pages=256)
        analyzer.record_round({'n': 1, 'free_pages': 90000, 'elapsed_s': 60, 'key_ms': 45.0, 'app': 'jot', 'audio': 'beep 440Hz'})
        analyzer.record_round({'n': 2, 'free_pages': 90000, 'elapsed_s': 120, 'key_ms': 42.0, 'app': 'fortune', 'audio': 'beep 550Hz'})

        canaries = {'canary_h1.txt': 'hash_test_1234'}
        md = generate_summary_markdown(analyzer, ok=True, why="", total_hours=12.0, elapsed_seconds=120.0, canary_files=canaries)

        self.assertIn("# SexOS 12-Hour Long-Run Soak Gate Report", md)
        self.assertIn("PASS — Zero leaks, hangs, or faults detected.", md)
        self.assertIn("Baseline Free Pages", md)
        self.assertIn("90000", md)
        self.assertIn("`jot`", md)
        self.assertIn("`fortune`", md)
        self.assertIn("canary_h1.txt", md)

    def test_failure_summary_markdown_generation(self):
        analyzer = SoakAnalyzer(warmup=1, leak_pages=100)
        analyzer.record_round({'n': 1, 'free_pages': 50000, 'elapsed_s': 60, 'key_ms': 45.0})
        tail = ["line 1", "line 2", "KERNEL PANIC: test fault"]
        md = generate_summary_markdown(
            analyzer,
            ok=False,
            why="test panic failure",
            total_hours=12.0,
            elapsed_seconds=60.0,
            canary_files={},
            serial_tail=tail,
        )

        self.assertIn("FAILED", md)
        self.assertIn("test panic failure", md)
        self.assertIn("KERNEL PANIC: test fault", md)


class TestWorkflowConfiguration(unittest.TestCase):
    def test_long_soak_workflow_yaml_structure(self):
        workflow_path = ROOT / '.github/workflows/long-soak.yml'
        self.assertTrue(workflow_path.exists(), "long-soak.yml must exist")

        # Parse YAML using Ruby YAML
        cmd = ["ruby", "-ryaml", "-rjson", "-e", "puts JSON.dump(YAML.load_file(ARGV[0]))", str(workflow_path)]
        res = subprocess.run(cmd, capture_output=True, text=True, check=True)
        wf = json.loads(res.stdout)

        # 1. Trigger verification
        self.assertIn('workflow_dispatch', wf.get('on', {}))
        inputs = wf['on']['workflow_dispatch'].get('inputs', {})
        self.assertIn('hours', inputs)
        self.assertIn('leak_pages', inputs)
        self.assertIn('cores', inputs)
        self.assertEqual(str(inputs['hours']['default']), '12')
        self.assertEqual(str(inputs['leak_pages']['default']), '256')
        self.assertIn('schedule', wf.get('on', {}))

        # 2. Jobs verification (3 x 4.25h segments safely cover full 12h under GitHub's 360m limit)
        jobs = wf.get('jobs', {})
        self.assertIn('build-iso', jobs)
        self.assertIn('soak-segment-1', jobs)
        self.assertIn('soak-segment-2', jobs)
        self.assertIn('soak-segment-3', jobs)

        # 3. Timeout bounds (must be <= 300 min per 4.25h segment, well inside GitHub Actions 360m limit)
        for seg in ('soak-segment-1', 'soak-segment-2', 'soak-segment-3'):
            self.assertLessEqual(jobs[seg].get('timeout-minutes', 999), 300)
            for step in jobs[seg].get('steps', []):
                run_block = step.get('run', '')
                self.assertNotIn('inputs.', run_block, f"{seg} must pass inputs via env:, not raw shell interpolation")

        # 4. Dependencies
        self.assertEqual(jobs['soak-segment-1'].get('needs'), ['build-iso'])
        self.assertEqual(sorted(jobs['soak-segment-2'].get('needs', [])), ['build-iso', 'soak-segment-1'])
        self.assertEqual(sorted(jobs['soak-segment-3'].get('needs', [])), ['build-iso', 'soak-segment-2'])


if __name__ == '__main__':
    unittest.main()
