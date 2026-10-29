import sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from check_spindle_history_log import validate

def fixture():
    lines=[]
    for pd in [12,17]:
        if pd==17:lines.append('[spindle.history.receipt] target=17 task=17 kind=missing handle=0 bytes=0 count=0 hash=0x1 error=4')
        lines.extend([f'[files.history.open] target={pd} handle={pd} mode=save seq=2',f'[files.history.commit] target={pd} handle={pd} seq=10 bytes=40 hash=0xabc',f'[files.history.close] target={pd} handle={pd} seq=11 ok=1',f'[spindle.history.receipt] target={pd} task={pd} kind=saved handle={pd} bytes=40 count=1 hash=0xabc error=0'])
        if pd==17:lines.extend(['[files.history.open] target=17 handle=18 mode=load seq=12','[files.history.close] target=17 handle=18 seq=25 ok=1','[spindle.history.receipt] target=17 task=17 kind=loaded handle=18 bytes=40 count=1 hash=0xabc error=0'])
        lines.extend([f'[files.history.retire] target={pd} staged=0 pending=0 terminal=1',f'[app.service.ack] target={pd} role=4',f'[app.private.retired] target={pd}'])
    return '\n'.join(lines)
class Check(unittest.TestCase):
    def test_complete_fixture(self):self.assertEqual(validate(fixture()),[])
    def test_enqueue_and_fake_completion_do_not_prove_commit(self):self.assertTrue(validate(fixture().replace('[files.history.commit]','[request.admitted]')))
    def test_wrong_owner_and_fingerprint_rejected(self):
        for old,new in [('target=17 handle=17 mode=save','target=12 handle=17 mode=save'),('seq=10 bytes=40 hash=0xabc','seq=10 bytes=40 hash=0xdef')]:self.assertTrue(validate(fixture().replace(old,new)))
    def test_reused_or_missing_close_handle_rejected(self):
        self.assertTrue(validate(fixture().replace('handle=18','handle=17')));self.assertTrue(validate(fixture().replace('[files.history.close]','[close.enqueued]')))
    def test_initial_private_empty_and_cleanup_required(self):
        for old,new in [('kind=missing','kind=inherited'),('staged=0','staged=1'),('pending=0','pending=1')]:self.assertTrue(validate(fixture().replace(old,new)))
    def test_late_terminal_commit_and_sequence_reversal_rejected(self):
        self.assertTrue(validate(fixture()+'\n[files.history.commit] target=17 handle=18 seq=26 bytes=40 hash=0xabc'))
        self.assertTrue(validate(fixture().replace('seq=10','seq=1')))
if __name__=='__main__':unittest.main()
