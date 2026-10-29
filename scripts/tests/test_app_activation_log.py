import sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from check_app_activation_log import validate

def fixture():
    rows=['[boot.app.successor] logical=9 incarnation=2 predecessor=9 successor=16 task=16 surface=301 key=9 dormant=1 leaves=60 runtime_mint=0',
          '[app.activation.negative] dormant_route=denied early_activation=retry ok=1']
    for role in [1,2,4,8,16]:rows.append(f'[app.service.ack] target=9 role={role}')
    rows+=['[app.private.retired] target=9 task=9 acked=31 required=31 all_flushed=1 pool_reusable=1',
           '[app.activation.committed] logical=9 incarnation=2 predecessor=9 successor=16 task=16 surface=301 pages=60 runtime_alloc=0 runtime_mint=0 bsp=1',
           '[app.activation.negative] stale_predecessor_route=denied duplicate_activation=receipt ok=1',
           '[quil.incarnation.start] target=16 task=16 surface=301 fresh=1',
           '[shell.app_surface.accept] sid=301 app_id=16 caller=16',
           '[app.surface.presented] target=16 surface=301 sequence=1 ok=1']
    for role in [1,2,4,8,16]:rows.append(f'[app.service.ack] target=16 role={role}')
    rows+=['[app.private.retired] target=16 task=16 acked=31 required=31 all_flushed=1 pool_reusable=1 pages=60 bytes=245760',
           '[sext.app.pool.ready] target=16 task=16 reused=1 zero_owner=sext ok=1 bytes=245760 zeroed=245760',
           '[launcher.execute.denied] idx=1 reason=boot_reserve_exhausted']
    return '\n'.join(rows)
class Check(unittest.TestCase):
    def test_synthetic_complete_scoped_records(self):self.assertEqual(validate(fixture()),[])
    def test_missing_or_repeated_commit(self):
        text=fixture();line=next(l for l in text.splitlines() if '[app.activation.committed]' in l)
        self.assertTrue(validate(text.replace(line,'')));self.assertTrue(validate(text+'\n'+line))
    def test_wrong_task_or_key(self):
        for old,new in [('key=9','key=16'),('successor=16 task=16','successor=16 task=9')]:self.assertTrue(validate(fixture().replace(old,new)))
    def test_early_activation_and_missing_role(self):
        rows=fixture().splitlines();commit=rows.pop(next(i for i,l in enumerate(rows) if '[app.activation.committed]' in l));rows.insert(1,commit)
        self.assertTrue(validate('\n'.join(rows)));self.assertTrue(validate(fixture().replace('[app.service.ack] target=9 role=16','')))
    def test_partial_scrub_or_page_count(self):
        for old,new in [('zeroed=245760','zeroed=4096'),('pages=60 runtime_alloc','pages=59 runtime_alloc')]:self.assertTrue(validate(fixture().replace(old,new)))
if __name__=='__main__':unittest.main()
