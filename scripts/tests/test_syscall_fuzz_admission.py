"""Retry only the source-bound, pre-dispatch session-busy refusal."""
from pathlib import Path
import json
import tempfile
import unittest
import syscall_fuzz_gate as gate

LINE='run fuzz 20000 4 proto'
START='[fuzz.start] target=42 task=57 calls=20000 seed=4 proto=1 ops=112\n'
SAVE='[spindle.session.saved] bytes=0 status=0\n'

def busy(line=LINE):
    body=b'The disk is saving the session for a moment. Try again.\n'
    return ('[spindle.cmd.output] name=run bytes=84\n'
            f'[spindle.line.enter] len={len(line)} mode=insert text={json.dumps(line)}\n'
            f'[spindle.view.text] target=12 sid=154 sequence=10 paint=91 cols=80 rows=1 bytes={len(body)}\n'
            f'[spindle.view.bytes] target=12 sid=154 seq=10 paint=91 off=0 hex={body.hex()}\n')

class Admission(unittest.TestCase):
    def test_explicit_busy_and_later_save_retry_identical_command(self):
        trace=[''];sent=[];now=[0]
        def send(line):
            sent.append(line);trace[0]+=busy()+SAVE if len(sent)==1 else START
        def clock():now[0]+=.1;return now[0]
        m,offset,retries=gate.wait_fuzz_start({'text':lambda:trace[0]},LINE,20000,4,send,clock=clock,pause=lambda _:None)
        self.assertEqual(sent,[LINE,LINE]);self.assertEqual(retries,1)
        self.assertEqual(m.groups()[:2],('42','57'));self.assertEqual(trace[0][offset:],START)

    def test_no_busy_or_no_later_successful_save_never_resubmits(self):
        for receipt in ('',busy(),SAVE+busy(),busy()+SAVE.replace('status=0','status=1'),
                        busy().replace('target=12','target=13')+SAVE,
                        busy().replace('off=0','off=1')+SAVE,
                        busy().replace('bytes=84','bytes=83')+SAVE):
            sent=[];now=[0]
            def clock():now[0]+=.1;return now[0]
            with self.subTest(receipt=receipt),self.assertRaises((TimeoutError,ValueError)):
                gate.wait_fuzz_start({'text':lambda:receipt if sent else ''},LINE,20000,4,sent.append,timeout=.8,clock=clock,pause=lambda _:None)
            self.assertEqual(sent,[LINE])

    def test_launch_or_ticket_side_effect_forbids_busy_retry(self):
        for tag in ('shell.program.launch','shell.launch.asked','files.program.read',
                    'files.program.bound','app.program.loaded','app.activation.committed',
                    'spindle.session.started','fuzz.start'):
            with self.subTest(tag=tag),self.assertRaises(ValueError):
                gate.busy_refusal(busy()+f'[{tag}] target=42\n'+SAVE,LINE)

    def test_wrong_command_duplicates_and_partial_frame_cannot_qualify(self):
        for trace in (busy('run other'),busy()+busy(),busy().rsplit('[spindle.view.bytes]',1)[0],
                      busy().replace('paint=91 off','paint=92 off')):
            with self.subTest(trace=trace):
                try:self.assertIsNone(gate.busy_refusal(trace,LINE))
                except ValueError:pass

    def test_fixed_deadline_and_two_retry_limit(self):
        sent=[];trace=[''];now=[0]
        def send(line):sent.append(line);trace[0]+=busy()+SAVE
        def clock():now[0]+=.01;return now[0]
        with self.assertRaises(AssertionError):
            gate.wait_fuzz_start({'text':lambda:trace[0]},LINE,20000,4,send,clock=clock,pause=lambda _:None)
        self.assertEqual(sent,[LINE]*3)

    def test_source_changed_guard_is_closed(self):
        self.assertEqual(gate.busy_sources(gate.ROOT),gate.BUSY_SOURCE_HASHES)
        with tempfile.TemporaryDirectory() as name:
            root=Path(name)
            for file in gate.BUSY_SOURCE_HASHES:
                p=root/file;p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes((gate.ROOT/file).read_bytes())
            p.write_bytes(p.read_bytes()+b'\n// changed dispatch\n')
            with self.assertRaises(AssertionError):gate.busy_sources(root)

if __name__=='__main__':unittest.main()
