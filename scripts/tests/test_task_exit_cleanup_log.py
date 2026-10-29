"""Mutation regressions for the actual retained guest cleanup transcript."""
import importlib.util
import re
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
spec = importlib.util.spec_from_file_location('exit_cleanup', ROOT / 'check_task_exit_cleanup_log.py')
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)
BASE = (Path(__file__).parent / 'fixtures/task_exit_cleanup.log').read_text()


def synthetic_quota():
    """Mutated fixture for validator coverage; never claimed as a guest run."""
    text = BASE.replace('[paging.client.finish] pd=15 resident=4 reason=proof',
                        '[paging.client.quota.begin] pd=15 page=4 in_range=1')
    base = int(re.search(r'\[paging.client.begin\] pd=15 base=(0x[0-9a-f]+)', text)[1], 16)
    address = hex(base + 4 * 4096)
    text = re.sub(r'\[task.finish\][^\n]*',
        f'[paging.fault.trap] pd=15 addr={address} rip=0x4e123 error=0x4 eligible=1\n'
        f'[paging.fault.pending] target=15 task=15 token=10 page={address} rip=0x4e123\n'
        '[paging.fault.rejected] target=15 task=15 token=10 terminal=1\n'
        f'[sext.fault.reject] target=15 token=10 page={address} reason=Quota used=4 quota=4', text)
    before, after = text.split('[paging.exit.notice]', 1)
    after = re.sub(r'(new_token=|token=)(1[012])\b', lambda m: m[1] + str(int(m[2]) + 1), after)
    return before + '[paging.exit.notice]' + after


class ExitCleanupLogTests(unittest.TestCase):
    def test_recorded_positive(self):
        self.assertEqual(checker.validate(BASE), [])

    def test_each_boundary_required(self):
        for marker in ('paging.client.finish', 'task.finish', 'paging.exit.notice', 'paging.leaf.cleanup', 'paging.target.cleanup', 'sext.cleanup', 'sext.cleanup.wait', 'sext.cleanup.replay', 'paging.client.exit_reuse.page', 'paging.client.exit_reuse'):
            self.assertTrue(checker.validate(BASE.replace('['+marker+']','[omitted]',1)), marker)

    def test_live_owner_or_incomplete_invalidation_fails(self):
        for before, after in (('detached=1 accepted=1','detached=0 accepted=1'), ('all_flushed=1','all_flushed=0'), ('finalized=1 cleared=1','finalized=1 cleared=0'), ('finalized=1 cleared=1 tlb=1','finalized=1 cleared=1 tlb=0')):
            self.assertTrue(checker.validate(BASE.replace(before,after,1)))

    def test_cleanup_bitmap_identity(self):
        self.assertTrue(checker.validate(BASE.replace('frames=0x17 leaves=4','frames=0x7 leaves=4',1)))
        self.assertTrue(checker.validate(BASE.replace('bitmap=0x17 released=4','bitmap=0x7 released=4',1)))
        self.assertTrue(checker.validate(BASE.replace('target=15 task=15 token=9','target=10 task=15 token=9',1)))

    def test_exact_task_and_generation(self):
        self.assertTrue(checker.validate(BASE.replace('target=15 task=15 pager=14','target=15 task=99 pager=14',1)))
        self.assertTrue(checker.validate(BASE.replace('target=15 task=15 token=9','target=15 task=15 token=5',1)))
        self.assertTrue(checker.validate(BASE.replace('new_token=10','new_token=9',1)))

    def test_end_to_reuse_stage_order(self):
        rows=BASE.splitlines(); a=next(i for i,r in enumerate(rows) if '[paging.target.cleanup]' in r); b=next(i for i,r in enumerate(rows) if '[sext.cleanup]' in r); rows[a],rows[b]=rows[b],rows[a]
        self.assertTrue(checker.validate('\n'.join(rows)))

    def test_ended_task_never_runs_again(self):
        self.assertTrue(checker.validate(BASE+'\ntask.running id=15 pd_id=15'))

    def test_survivor_zero_and_retained_neighbor(self):
        for before, after in (('dead_target=15 ok=1','dead_target=14 ok=1'), ('tail=1 value=1 neighbor=1','tail=0 value=1 neighbor=1'), ('value=1 neighbor=1','value=1 neighbor=0')):
            self.assertTrue(checker.validate(BASE.replace(before,after,1)))

    def test_cached_cleanup_must_preserve_survivor(self):
        self.assertTrue(checker.validate(BASE.replace('preserved=1','preserved=0',1)))
        self.assertTrue(checker.validate(BASE.replace('bitmap=0x17 frame=0','bitmap=0x7 frame=0',1)))

    def test_survivor_trap_is_required(self):
        rows=BASE.splitlines(); i=max(i for i,r in enumerate(rows) if '[paging.fault.trap]' in r); rows[i]=rows[i].replace('paging.fault.trap','omitted')
        self.assertTrue(checker.validate('\n'.join(rows)))

    def test_missing_fields_never_crash(self):
        for line in BASE.splitlines():
            if not any(m in line for m in ('[paging.leaf.cleanup]','[paging.target.cleanup]','[sext.cleanup.replay]','[paging.exit.notice]')):
                continue
            for field in line.split()[1:]:
                if '=' in field:
                    self.assertTrue(checker.validate(BASE.replace(line,line.replace(field,'',1),1)),field)

    def test_quota_profile_validates_without_shadowed_policy_crash(self):
        self.assertEqual(checker.validate(synthetic_quota(), True), [])

    def test_dead_and_survivor_tasks_cannot_change_after_initial_probe(self):
        head, tail = BASE.split('[paging.client.finish]', 1)
        for before, after in (('task=15', 'task=99'), ('task=10', 'task=99')):
            self.assertTrue(checker.validate(head + '[paging.client.finish]' + tail.replace(before, after)))

    def test_survivor_policy_address_and_accounting_join(self):
        head, tail = BASE.split('[paging.client.finish]', 1)
        for key, value in (('page', '0x1234'), ('used', '1')):
            rows = tail.splitlines()
            index = next(i for i, row in enumerate(rows) if '[sext.fault.commit]' in row)
            rows[index] = re.sub(r'\b' + key + r'=[^ ]+', key + '=' + value, rows[index])
            self.assertTrue(checker.validate(head + '[paging.client.finish]' + '\n'.join(rows)))

    def test_invalid_survivor_frame_fails_without_exception(self):
        head, tail = BASE.split('[paging.client.finish]', 1)
        rows = tail.splitlines()
        index = next(i for i, row in enumerate(rows) if '[paging.leaf.commit]' in row)
        rows[index] = rows[index].replace('frame=0', 'frame=999')
        self.assertTrue(checker.validate(head + '[paging.client.finish]' + '\n'.join(rows)))

    def test_quota_trap_and_policy_are_exact_and_ordered(self):
        text = synthetic_quota()
        before, after = text.split('[paging.client.quota.begin]', 1)
        self.assertTrue(checker.validate(before + '[paging.client.quota.begin]' + after.replace('error=0x4', 'error=0x6', 1), True))
        self.assertTrue(checker.validate(text + '\n[task.finish] pd=15 task=15 reason=0 terminal=1', True))

    def test_kernel_notice_can_preempt_userland_rejection_diagnostic(self):
        rows = synthetic_quota().splitlines()
        index = next(i for i, row in enumerate(rows) if '[sext.fault.reject]' in row)
        policy = rows.pop(index)
        after = next(i for i, row in enumerate(rows) if '[paging.exit.notice]' in row)
        rows.insert(after + 1, policy)
        self.assertEqual(checker.validate('\n'.join(rows), True), [])

    def test_wrong_profile_and_fatal_fault(self):
        self.assertTrue(checker.validate(BASE,True))
        self.assertTrue(checker.validate(BASE+'\n[paging.client.fatal] panic'))


if __name__ == '__main__':
    unittest.main()
