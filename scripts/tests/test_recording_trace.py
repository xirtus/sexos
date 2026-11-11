import sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from recording_trace import correlate,check_packets
class RecordingTrace(unittest.TestCase):
    def trace(self):
        return ('[display.recording.frame] session=7 task=42 generation=10 publication_ms=1000 width=640\n'
                '[record.frame.accepted] session=7 task=42 index=1 generation=10 publication_ms=1000 pts_ms=0\n'
                '[display.recording.frame] session=7 task=42 generation=12 publication_ms=1200 width=640\n'
                '[display.recording.frame] session=7 task=42 generation=15 publication_ms=1600 width=640\n'
                '[record.frame.accepted] session=7 task=42 index=2 generation=15 publication_ms=1600 pts_ms=600\n')
    def test_actual_publication_gaps_survive_a_dropped_frame(self):
        result=correlate(self.trace(),7,42,2);check_packets(result,[0,600])
        self.assertEqual(result['dropped_publications'],[dict(generation=12,publication_ms=1200)])
        with self.assertRaises(AssertionError):check_packets(result,[0,200])
    def test_wrong_stamp_generation_and_sequence_are_rejected(self):
        for old,new in [('index=2','index=3'),('index=2 generation=15','index=2 generation=14'),('publication_ms=1600 pts_ms','publication_ms=1601 pts_ms'),('pts_ms=600','pts_ms=200')]:
            with self.subTest(old=old),self.assertRaises(AssertionError):correlate(self.trace().replace(old,new),7,42,2)
    def test_other_session_or_task_cannot_complete_the_receipts(self):
        for suffix in ['session=8 task=42','session=7 task=43']:
            text=self.trace().replace('[record.frame.accepted] session=7 task=42 index=2','[record.frame.accepted] '+suffix+' index=2')
            with self.assertRaises(AssertionError):correlate(text,7,42,2)
    def test_receipt_must_follow_its_actual_publication(self):
        lines=self.trace().splitlines(keepends=True);lines[0],lines[1]=lines[1],lines[0]
        with self.assertRaises(AssertionError):correlate(''.join(lines),7,42,2)
    def test_duplicate_publication_and_missing_diagnostic_fail(self):
        with self.assertRaises(AssertionError):correlate(self.trace()+self.trace().splitlines(keepends=True)[0],7,42,2)
        with self.assertRaises(AssertionError):correlate(self.trace(),7,42,3)
if __name__=='__main__':unittest.main()
