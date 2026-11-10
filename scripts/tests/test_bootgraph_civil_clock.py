"""Boot log receipts only; synthetic fixtures do not establish clock pixels."""
import sys
import hashlib
import subprocess
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from check_bootgraph_log import clock_gate
from test_silkbar_clock_gate import fixture, T
from test_runtime_proof import good_log

PRESENTATION = '[sexdisplay.clock.presentation] label=UTC state=Unknown snapshot_only=1 x=1090 y=18 w=80 h=22 width=1280\n'
LIVE = '[sexdisplay.render.live.ok] fb_w=1280 fb_h=800\n'
LEGACY = '[silkbar.clock.send]\n[sexdisplay.clock.recv]\n[sexdisplay.clock.redraw]\n'+LIVE


def civil(**kwargs):
    return PRESENTATION+fixture(**kwargs)+LIVE


class BootCivilClock(unittest.TestCase):
    def check(self, trace):
        return clock_gate(trace.splitlines(), True)[0]

    def test_snapshot_admission_completed_draw_and_live_render(self):
        result = self.check(civil())
        self.assertTrue(result.passed, result.reason)
        self.assertIn('log receipts only', result.reason)

    def test_late_post_send_receipt_is_not_causality_anchor(self):
        self.assertTrue(self.check(civil(late_enqueue=True)).passed)

    def test_interrupted_legacy_prefix_preserves_atomic_source(self):
        self.assertTrue(self.check(civil().replace('[clock.source.snapshot]', '[sexdrive.nvme.submit.detail] lba=[clock.source.snapshot]')).passed)

    def test_missing_or_malformed_civil_evidence_never_uses_legacy(self):
        text = civil()
        for prefix in ('[boot.silkbar.clock.source]', '[clock.source.snapshot]',
                       '[silkbar.civil.sample]', '[silkbar.civil.publish.try]',
                       '[sexdisplay.clock.civil.applied]', '[display.civil.draw]',
                       '[sexdisplay.clock.presentation]'):
            bad = '\n'.join(line for line in text.splitlines() if not line.startswith(prefix))
            with self.subTest(prefix=prefix):
                self.assertFalse(self.check(bad+'\n'+LEGACY).passed)
        self.assertFalse(self.check(PRESENTATION+LEGACY).passed)

    def test_live_render_is_required(self):
        self.assertFalse(self.check(civil().replace(LIVE, '')).passed)

    def test_unknown_unavailable_and_admission_without_draw_are_insufficient(self):
        for state in ('Unknown', 'Unavailable'):
            self.assertFalse(self.check(civil(state=state)).passed)
        self.assertFalse(self.check(civil().replace('pixels=written', 'pixels=unconfirmed')).passed)

    def test_draw_requires_exact_prior_admission_and_clean_record_end(self):
        text = civil()
        draw = next(line for line in text.splitlines(True) if line.startswith('[display.civil.draw]'))
        for bad in (text.replace(draw, draw.replace('sequence=1', 'sequence=2')),
                    text.replace(draw, draw.replace(f'seconds={T}', f'seconds={T+1}')),
                    PRESENTATION+draw+text.replace(PRESENTATION, '').replace(draw, ''),
                    text.replace(draw, draw.rstrip()+' extra=1\n'),
                    text.replace('pd=6 task=6 sequence=', 'pd=13 task=13 sequence=')):
            with self.subTest(bad=bad):
                self.assertFalse(self.check(bad).passed)

    def test_explicit_demo_and_historical_receipts_keep_legacy_lane(self):
        self.assertTrue(self.check(LEGACY).passed)
        self.assertTrue(self.check('[sexdisplay.clock.presentation] label=DEMO wall_time=0\n'+LEGACY).passed)
        self.assertFalse(self.check(LEGACY.replace('[sexdisplay.clock.recv]', '')).passed)


class BoundedClockCapture(unittest.TestCase):
    def run_capture(self, raw):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "serial.log"
            path.write_bytes(raw)
            result = subprocess.run([sys.executable, str(Path(__file__).resolve().parents[1] / "check_bootgraph_log.py"),
                                     str(path), "--strict-clock"], capture_output=True, text=True)
            self.assertEqual(path.read_bytes(), raw, "the raw artifact must stay unchanged")
        self.assertIn(f"raw_sha256={hashlib.sha256(raw).hexdigest()}", result.stdout)
        return result

    def full_trace(self):
        return ("\n".join(good_log()) + "\n" + civil()).encode()

    def test_actual_unfinished_sample_tail_is_not_a_clock_receipt(self):
        tail = b"[silkbar.civil.sample] seque"
        result = self.run_capture(self.full_trace() + tail)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn(f"ignored_clock_tail_bytes={len(tail)} ", result.stdout)
        self.assertIn("CLOCK_GATE: PASS", result.stdout)

    def test_complete_malformed_tail_or_interior_still_fails(self):
        bad = b"[silkbar.civil.sample] seque\n"
        for raw in [self.full_trace() + bad, bad + self.full_trace()]:
            with self.subTest(raw=raw):
                result = self.run_capture(raw)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("CLOCK_GATE: FAIL CIVIL: malformed civil sample", result.stdout)
                self.assertIn("ignored_clock_tail_bytes=0 ", result.stdout)

    def test_only_unfinished_clock_evidence_cannot_pass(self):
        raw = ("\n".join(good_log()) + "\n").encode() + b"[silkbar.civil.sample] seque"
        result = self.run_capture(raw)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("CLOCK_GATE: FAIL", result.stdout)

    def test_unfinished_fault_still_reaches_fault_gate(self):
        result = self.run_capture(self.full_trace() + b"KERNEL PAGE FAULT HALT:")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("FAULT_GATE: FAIL", result.stdout)


if __name__ == '__main__':
    unittest.main()
