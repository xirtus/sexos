import sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from check_spindle_history_log import validate
from history_boot_fixture import boot

def fixture():
    lines=[]
    for pd in [12,17,24]:
        if pd!=12:lines.append(f'[spindle.history.receipt] target={pd} task={pd} kind=missing handle=0 bytes=0 count=0 hash=0x1 error=4')
        lines.extend([f'[files.history.open] target={pd} handle={pd} mode=save seq=2',f'[files.history.commit] target={pd} handle={pd} seq=10 bytes=40 hash=0xabc',f'[files.history.close] target={pd} handle={pd} seq=11 ok=1',f'[spindle.history.receipt] target={pd} task={pd} kind=saved handle={pd} bytes=40 count=1 hash=0xabc error=0'])
        if pd!=12:lines.extend([f'[files.history.open] target={pd} handle={pd+1} mode=load seq=12',f'[files.history.close] target={pd} handle={pd+1} seq=25 ok=1',f'[spindle.history.receipt] target={pd} task={pd} kind=loaded handle={pd+1} bytes=40 count=1 hash=0xabc error=0'])
        lines.extend([f'[files.history.retire] target={pd} staged=0 pending=0 terminal=1',f'[app.service.ack] target={pd} role=4',f'[app.private.retired] target={pd}'])
    return '\n'.join(lines)
def durable_fixture():
    text=fixture()
    for generation,predecessor,target in [(1,12,17),(2,17,24)]:
        text=text.replace(f'[spindle.history.receipt] target={target} task={target} kind=missing handle=0 bytes=0 count=0 hash=0x1 error=4',f'[files.history.recovered] logical=12 source={predecessor} source_task={predecessor} target={target} generation={generation} bytes=40 source_hash=0xabc owned_hash=0xabc data_hash=0x123 boot_grant=53')
    for generation,pd in enumerate([12,17,24],1):
        text=text.replace(f'[files.history.commit] target={pd}',f'[files.history.durable] logical=12 target={pd} generation={generation} bytes=40 hash=0xabc barriers=3 readback=1\n[files.history.commit] target={pd}')
    return boot()+text
def prefix_fixture():
    # Keep the complete three-participant BOOT grant; exercise only 12 -> 17.
    return '\n'.join(line for line in durable_fixture().splitlines()
                     if not ('target=24 ' in line or line.endswith('target=24'))
                     or line.startswith('[boot.'))+'\n'
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
    def test_durable_barriers_and_exact_boot_relationship(self):self.assertEqual(validate(durable_fixture(),durable=True),[])
    def test_durable_evidence_requires_real_barriers_readback_and_grant(self):
        for old,new in [('barriers=3','barriers=0'),('readback=1','readback=0'),('slot=53','slot=54'),('source_task=12','source_task=99'),('source_hash=0xabc','source_hash=0xdef'),('generation=2','generation=1')]:self.assertTrue(validate(durable_fixture().replace(old,new),durable=True),(old,new))
    def test_volatile_fixture_cannot_prove_durability(self):self.assertTrue(validate(fixture(),durable=True))
    def test_explicit_prefix_under_complete_boot_roster(self):
        for targets in [(12,17),[12,17]]:
            self.assertEqual(validate(prefix_fixture(),durable=True,expected_targets=targets),[])
        self.assertTrue(validate(prefix_fixture(),durable=True))
        self.assertEqual(validate(durable_fixture(),durable=True,expected_targets=(12,17,24)),[])
    def test_prefix_rejects_unknown_reordered_skipped_duplicate_or_non_sequence(self):
        for targets in [(),[],(12,),(17,24),(12,24),(17,12),(12,17,17),
                        (12,17,25),(12,17,24,25),{12,17},'12,17',(12,'17'),(True,17)]:
            with self.subTest(targets=targets):
                self.assertTrue(validate(prefix_fixture(),durable=True,expected_targets=targets))
    def test_prefix_does_not_hide_later_subject_receipts(self):
        self.assertTrue(validate(durable_fixture(),durable=True,expected_targets=(12,17)))
        for kind in ['missing','saved','loaded']:
            extra=f'[spindle.history.receipt] target=24 task=24 kind={kind} handle=24 bytes=40 count=1 hash=0xabc error=0\n'
            self.assertTrue(validate(prefix_fixture()+extra,durable=True,expected_targets=(12,17)))
    def test_prefix_still_requires_all_boot_tasks_and_exact_receipt_tasks(self):
        for old,new in [
            ('index=2 target=24 task=24','index=2 target=24 task=999'),
            ('index=2 target=24 task=24','index=2 target=24'),
            ('target=17 task=17 kind=saved','target=17 task=999 kind=saved'),
            ('target=17 task=17 kind=loaded','target=17 kind=loaded'),
            ('source_task=12','source_task=999'),
        ]:
            with self.subTest(old=old,new=new):
                self.assertTrue(validate(prefix_fixture().replace(old,new),durable=True,expected_targets=(12,17)))
        text='\n'.join(line for line in prefix_fixture().splitlines()
                       if not line.startswith('[boot.history.participant] index=2'))+'\n'
        self.assertTrue(validate(text,durable=True,expected_targets=(12,17)))
    def test_prefix_accepts_distinct_held_task_namespace(self):
        text=prefix_fixture()
        for pd in [12,17,24]:
            text=text.replace(f'target={pd} task={pd} ',f'target={pd} task={pd*10} ')
        text=text.replace('source_task=12 ','source_task=120 ')
        self.assertEqual(validate(text,durable=True,expected_targets=(12,17)),[])
if __name__=='__main__':unittest.main()
