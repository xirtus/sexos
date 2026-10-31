import sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from check_self_wait import inspect
BASE='''[boot.self_wait.sealed] tasks=18 capacity=32 max_ticks=64 runtime_registration=0
[sexnet.wait.authority] invalid_intervals=3 spare_arguments=4 ok=1
'''
LIVE=BASE+'''[scheduler.self_wait.park] task=13 ticks=2 deadline=102 count=1
[scheduler.self_wait.expired] task=13 deadline=102 now=103 count=1
[sexnet.receive.wait] before=100 after=104 ticks=2 count=1 kind=timer_assisted_poll irq=0
'''
class SelfWait(unittest.TestCase):
 def test_real_chain_shape_and_unsupported(self):
  self.assertTrue(inspect(LIVE)['ok']);self.assertTrue(inspect(BASE,False)['ok']);self.assertFalse(inspect(BASE)['ok']);self.assertFalse(inspect(LIVE,False)['ok']);self.assertTrue(inspect(LIVE.replace('irq=0','irq=0 device=0 reply_pending=1'),False)['ok'])
 def test_missing_reordered_and_foreign_events(self):
  for line in LIVE.splitlines():self.assertFalse(inspect(LIVE.replace(line+'\n',''))['ok'])
  for before,after in [('task=13','task=16'),('deadline=102 now=103','deadline=102 now=101'),('before=100 after=104','before=105 after=104'),('before=100 after=104','before=103 after=104'),('capacity=32','capacity=33'),('deadline=102 count=1','deadline=99 count=1')]:self.assertFalse(inspect(LIVE.replace(before,after))['ok'])
  lines=LIVE.splitlines();lines[2],lines[3]=lines[3],lines[2];self.assertFalse(inspect('\n'.join(lines))['ok'])
 def test_duplicate_ordinals_seal_and_failed_guest(self):
  for extra in [BASE,LIVE.splitlines()[2]+'\n','KERNEL PANIC','BOOT_SELF_WAIT_AUTHORITY']:
   self.assertFalse(inspect(LIVE+extra)['ok'])

 def test_sampled_sexdrive_nvme_wait_is_sanctioned_but_other_intervals_are_not(self):
  # Round 105: sexdrive (task 2) waits one tick for NVMe completions.
  drive='[scheduler.self_wait.park] task=2 ticks=1 deadline=3145 count=1024\n'
  self.assertTrue(inspect(LIVE+drive)['ok'])
  for bad in(drive.replace('ticks=1','ticks=2'),drive.replace('task=2','task=3'),drive.replace('deadline=3145','deadline=0')):
   self.assertFalse(inspect(LIVE+bad)['ok'],bad)
if __name__=='__main__':unittest.main()
