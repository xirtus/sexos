import sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from check_self_wait import inspect
BASE='''[boot.self_wait.sealed] tasks=31 capacity=64 max_ticks=64 runtime_registration=0
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
  for before,after in [('task=13','task=16'),('deadline=102 now=103','deadline=102 now=101'),('before=100 after=104','before=105 after=104'),('before=100 after=104','before=103 after=104'),('capacity=64','capacity=33'),('deadline=102 count=1','deadline=99 count=1')]:self.assertFalse(inspect(LIVE.replace(before,after))['ok'])
  lines=LIVE.splitlines();lines[2],lines[3]=lines[3],lines[2];self.assertFalse(inspect('\n'.join(lines))['ok'])
 def test_duplicate_ordinals_seal_and_failed_guest(self):
  for extra in [BASE,LIVE.splitlines()[2]+'\n','KERNEL PANIC','BOOT_SELF_WAIT_AUTHORITY']:
   self.assertFalse(inspect(LIVE+extra)['ok'])

 def test_independent_64_inventory_bound_accepts_32_33_and_64_tasks(self):
  for count in(1,31,32,33,64):
   for active,fixture in((True,LIVE),(False,BASE)):
    with self.subTest(tasks=count,active=active):
     checked=inspect(fixture.replace('tasks=31',f'tasks={count}'),active)
     self.assertTrue(checked['ok'],checked);self.assertEqual(checked['boot_tasks'],count)
  for count in(0,65):
   checked=inspect(LIVE.replace('tasks=31',f'tasks={count}'))
   self.assertFalse(checked['ok']);self.assertIsNone(checked['boot_tasks'])

 def test_logged_capacity_cannot_expand_or_shrink_the_independent_bound(self):
  for tasks,capacity in((31,32),(33,33),(31,65),(65,65),(64,128),(0,64)):
   with self.subTest(tasks=tasks,capacity=capacity):
    checked=inspect(LIVE.replace('tasks=31',f'tasks={tasks}').replace('capacity=64',f'capacity={capacity}'))
    self.assertFalse(checked['ok']);self.assertIsNone(checked['boot_tasks'])

 def test_malformed_extra_seal_cannot_hide_behind_one_valid_match(self):
  seal=BASE.splitlines(True)[0]
  for extra in(seal,seal.replace('capacity=64','capacity=32'),seal.replace('tasks=31','tasks=65'),
               '[boot.self_wait.sealed] malformed\n','[boot.self_wait.sealed] tasks=',
               '[boot.self_wait.sealed','[boot.self_wait.sealedX] tasks=31\n',
               seal.rstrip('\n')+' extra=1\n'):
   with self.subTest(extra=extra):
    checked=inspect(LIVE+extra);self.assertFalse(checked['ok']);self.assertIsNone(checked['boot_tasks'])

 def test_seal_requires_exact_complete_ascii_line(self):
  seal=BASE.splitlines(True)[0]
  for bad in(seal.replace('tasks=31','tasks=031'),seal.replace('tasks=31','tasks=+31'),
             seal.replace('tasks=31','tasks=٣١'),seal.replace('tasks=31','tasks='+'9'*5000),
             seal.replace('capacity=64','capacity=064'),
             seal.replace('max_ticks=64','max_ticks=65'),seal.replace('runtime_registration=0','runtime_registration=1'),
             seal.replace('tasks=31 capacity=64','capacity=64 tasks=31'),
             ' '+seal,seal.replace(' capacity=64','\tcapacity=64'),
             seal.replace('\n','\x00\n'),seal.rstrip('\n'),seal.replace('\n',' extra=1\n')):
   with self.subTest(seal=bad):
    checked=inspect(LIVE.replace(seal,bad,1));self.assertFalse(checked['ok']);self.assertIsNone(checked['boot_tasks'])
  self.assertTrue(inspect(LIVE.replace('\n','\r\n'))['ok'])

 def test_sampled_sexdrive_nvme_wait_is_sanctioned_but_other_intervals_are_not(self):
  # Round 105: sexdrive (task 2) waits one tick for NVMe completions.
  drive='[scheduler.self_wait.park] task=2 ticks=1 deadline=3145 count=1024\n'
  self.assertTrue(inspect(LIVE+drive)['ok'])
  expiry='[scheduler.self_wait.expired] task=2 deadline=11220 now=11220 count=4096\n'
  self.assertTrue(inspect(LIVE+expiry)['ok'])
  self.assertFalse(inspect(LIVE+expiry.replace('task=2','task=3'))['ok'])
  self.assertFalse(inspect(LIVE+expiry.replace('now=11220','now=11219'))['ok'])
  for bad in(drive.replace('ticks=1','ticks=2'),drive.replace('task=2','task=3'),drive.replace('deadline=3145','deadline=0')):
   self.assertFalse(inspect(LIVE+bad)['ok'],bad)

 def test_successor_browser_tasks_require_explicit_observed_ids(self):
  for task in(30,31):
   sample=f'[scheduler.self_wait.park] task={task} ticks=2 deadline=3145 count=1024\n[scheduler.self_wait.expired] task={task} deadline=3145 now=3145 count=4096\n'
   with self.subTest(task=task):
    self.assertFalse(inspect(LIVE+sample)['ok'])
    checked=inspect(LIVE+sample,browser_tasks=(15,30,31));self.assertTrue(checked['ok'],checked)
    self.assertEqual(checked['browser_tasks'],[15,30,31]);self.assertIn('caller-selected observed',checked['scope'])
  self.assertEqual(inspect(LIVE)['browser_tasks'],[15])

 def test_unknown_task_and_wrong_browser_interval_still_fail(self):
  for task,ticks in((32,2),(30,1),(31,3)):
   sample=f'[scheduler.self_wait.park] task={task} ticks={ticks} deadline=3145 count=1024\n'
   with self.subTest(task=task,ticks=ticks):self.assertFalse(inspect(LIVE+sample,browser_tasks=(15,30,31))['ok'])
  foreign='[scheduler.self_wait.expired] task=32 deadline=3145 now=3145 count=4096\n'
  self.assertFalse(inspect(LIVE+foreign,browser_tasks=(15,30,31))['ok'])

 def test_explicit_unequal_task_identity_is_not_a_pd_or_inventory_index(self):
  sample='[scheduler.self_wait.park] task=99 ticks=2 deadline=3145 count=1024\n[scheduler.self_wait.expired] task=99 deadline=3145 now=3146 count=4096\n'
  self.assertFalse(inspect(LIVE+sample)['ok'])
  checked=inspect(LIVE+sample,browser_tasks=(99,));self.assertTrue(checked['ok'],checked);self.assertEqual(checked['browser_tasks'],[99])
  # The bound applies to the number of supplied IDs, not their numeric value.
  self.assertTrue(inspect(LIVE,browser_tasks=tuple(range(100,164)))['ok'])
  self.assertTrue(inspect(LIVE,browser_tasks=(0xffff_ffff,))['ok'])

 def test_invalid_explicit_task_arguments_return_failure_without_exceptions(self):
  for tasks in((),[],(0,),(-1,),(0x1_0000_0000,),(15,15),(True,),(15.0,),([15],),
               '15',None,15,{15},iter([15]),tuple(range(100,165))):
   with self.subTest(tasks=repr(tasks)):
    checked=inspect(LIVE,browser_tasks=tasks);self.assertFalse(checked['ok']);self.assertEqual(checked['browser_tasks'],[])
    self.assertTrue(any('invalid explicit Browser task IDs'in error for error in checked['errors']))

 def test_inactive_retained_reply_rule_is_preserved_with_explicit_browser_tasks(self):
  self.assertTrue(inspect(BASE,False,browser_tasks=(30,31))['ok'])
  self.assertFalse(inspect(LIVE,False,browser_tasks=(30,31))['ok'])
  self.assertTrue(inspect(LIVE.replace('irq=0','irq=0 device=0 reply_pending=1'),False,browser_tasks=(30,31))['ok'])
if __name__=='__main__':unittest.main()
