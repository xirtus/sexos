"""Boot log receipts only; synthetic fixtures do not establish clock pixels."""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from check_bootgraph_log import clock_gate
from test_silkbar_clock_gate import fixture, T

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


if __name__ == '__main__':
    unittest.main()
