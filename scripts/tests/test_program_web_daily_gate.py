"""Web-profile reboot tests must finish the desktop's separate session save."""
import re
from pathlib import Path
import sys
import unittest

sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import program_web_daily_gate as gate

RETIRED='[app.private.retired] target=42 task=57 pages=8 reusable=1\n'
OBSERVED_RETIRED='[app.private.retired] target=42 task=42 pages=13260 reusable=1\n'
ENDED='[spindle.session.ended] target=42 task=57\n'
SAVED='[spindle.session.saved] bytes=0 status=0\n'


def guest(trace):
    calls=[]
    def wait(pattern,start,timeout):
        calls.append((pattern,start))
        found=re.search(pattern,trace[start:])
        if found is None:raise TimeoutError(pattern)
        return found
    return dict(wait=wait,text=lambda:trace),calls


class DailySessionFenceTests(unittest.TestCase):
    def test_exact_close_retirement_and_later_session_sync_are_required(self):
        for trace in (RETIRED+ENDED+SAVED,ENDED+RETIRED+SAVED):
            g,calls=guest('[web.profile.saved] files=1\n'+trace)
            gate.wait_closed_session(g,0,42,57)
            self.assertEqual(len(calls),3)
            self.assertGreaterEqual(calls[-1][1],g['text']().index(ENDED)+len(ENDED))

    def test_actual_observed_ordinary_program_retirement_schema(self):
        gate.wait_closed_session(guest(OBSERVED_RETIRED+ENDED.replace('task=57','task=42')+SAVED)[0],0,42,42)

    def test_wrong_run_missing_retirement_failed_sync_or_early_save_cannot_pass(self):
        for trace in (ENDED+SAVED,RETIRED+SAVED,
                      RETIRED.replace('task=57','task=58')+ENDED+SAVED,
                      RETIRED+ENDED.replace('target=42','target=43')+SAVED,
                      RETIRED+ENDED+SAVED.replace('status=0','status=1'),
                      RETIRED+ENDED+SAVED.replace('bytes=0','bytes=66'),
                      SAVED+RETIRED+ENDED,
                      RETIRED.replace('reusable=1','reusable=0')+ENDED+SAVED):
            with self.subTest(trace=trace),self.assertRaises(TimeoutError):
                gate.wait_closed_session(guest(trace)[0],0,42,57)

    def test_receipts_before_this_close_are_not_reused(self):
        trace=RETIRED+ENDED+SAVED+'[web.closed] target=42 task=57\n'
        with self.assertRaises(TimeoutError):gate.wait_closed_session(guest(trace)[0],trace.index('[web.closed]'),42,57)

    def test_second_boot_rejects_automatic_restoration_before_explicit_launch(self):
        gate.require_empty_restored_session(guest('[spindle.session.read] status=0 programs=0\n')[0])
        for trace in ('[spindle.session.read] status=0 programs=1\n',
                      '[spindle.session.read] status=2 programs=0\n',
                      '[spindle.session.read] status=0 programs=0\n[spindle.session.restore] scene=0 path=/bin/kaleidoscope\n'):
            with self.subTest(trace=trace),self.assertRaises(AssertionError):gate.require_empty_restored_session(guest(trace)[0])


if __name__=='__main__':unittest.main()
