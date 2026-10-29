"""Adversarial checker regressions; synthetic rows are never guest evidence."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('multi_paging', Path(__file__).resolve().parents[1] / 'check_multi_client_paging_log.py')
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)


def good(quota=False):
    bases = {10: 0x400000000000, 15: 0x400000010000}
    rows = []
    for target in (15, 10):
        rows.append(f'[boot.paging.genesis] pager=14 target={target} pool_pages=8 arena_pages=8 quota=4 target_pkey={target} leaves_absent=8 tables_preallocated=1')
    rows.append('[boot.memory.authority.sealed] runtime_alloc=0 runtime_map=capability_leaf runtime_mint=0')
    for target in (15, 10):
        rows.append(f'[sext.client.ready] target={target} frames=8 quota=4 pool_bytes=32768 arena_bytes=32768 zero_owner=sext')
    rows.append(f'[paging.client.begin] pd=10 base={hex(bases[10])} bytes=32768')

    def fault(target, page, frame, token, used, old=None, user=True):
        address = hex(bases[target] + page * 4096)
        error = 6 if target == 15 and page in (1, 3) else 4
        rows.extend([
            f'[paging.fault.trap] pd={target} addr={address} rip=0x4e001230 error={hex(error)} eligible=1',
            f'[paging.fault.pending] target={target} task={target+20} token={token} page={address} rip=0x4e001230',
            f'[paging.leaf.commit] token={token} target={target} page={address} frame={frame} rights=6 pkey={target} zero=1 runtime_alloc=0',
            f'[paging.fault.completed] target={target} task={target+20} token={token} resume=1 saved_rip=0x4e001230',
            f'[sext.fault.commit] target={target} token={token} page={address} frame={frame} used={used} zeroed=4096 ok=1',
            f'[sext.commit.negative] token={token} exact=1 conflicting_frame=1 zero_token=1 unknown_token=1 ok=1',
        ])
        if old:
            rows.append(f'[sext.reuse.negative] frame={frame} old_token={old[1]} token={token} old_target={old[0]} target={target} stale_commit=1 stale_reclaim=1 new_commit=1 ok=1')
        if user:
            rows.append(f"[paging.client.page] pd={target} page={page} first={'write' if error==6 else 'read'} zero=1 value=1 tail=1 ok=1")

    def release(target, page, frame, token, request, used):
        address = hex(bases[target] + page * 4096)
        rows.extend([
            f'[paging.leaf.reclaim] token={token} target={target} page={address} frame={frame} cleared=1 tlb=1 runtime_alloc=0',
            f'[sext.release] target={target} request={request} page={address} token={token} frame={frame} status=0 replay=0 used={used}',
            f'[sext.reclaim.negative] target={target} token={token} frame={frame} stale_commit=1 duplicate_reclaim=1 ok=1',
        ])
    for page in range(4):
        fault(10, page, page, page+1, page+1)
    for page in range(3):
        release(10, page, page, page+1, page+1, 3-page)
    rows.extend([
        '[paging.client.released] pd=10 pages=3 retained=1 ok=1',
        '[paging.client.negative] invalid_slot=1 high_alias=1 wrong_query=1 wrong_commit=1 wrong_reject=1 wrong_reclaim=1 ok=1',
        f'[paging.client.begin] pd=15 base={hex(bases[15])} bytes=32768',
    ])
    for page, frame in enumerate((0, 1, 2, 4)):
        fault(15, page, frame, page+5, page+1, (10, page+1) if page<3 else None)
    rows.append('[paging.client.recovered] pd=15 pages=4 retries=4 retained=4 ok=1')
    release(15, 0, 0, 5, 1, 3)
    fault(15, 0, 0, 9, 4, (15, 5), False)
    rows.extend([
        f'[sext.release] target=15 request=1 page={hex(bases[15])} token=5 frame=0 status=0 replay=1 used=4',
        f'[sext.release.denied] target=15 request=1 page={hex(bases[15]+4096)} status=3',
        f'[sext.release.denied] target=15 request=0 page={hex(bases[15])} status=3',
        '[paging.client.reuse] pd=15 page=0 refault=1 zero=1 replay_safe=1 neighbors=3 ok=1',
        '[paging.client.retained] pd=10 pages=1 cross_client_reuse=1 ok=1',
    ])
    if quota:
        address = hex(bases[15] + 4*4096)
        rows.extend([
            '[paging.client.quota.begin] pd=15 page=4 in_range=1',
            f'[paging.fault.trap] pd=15 addr={address} rip=0x4e001230 error=0x4 eligible=1',
            f'[paging.fault.pending] target=15 task=35 token=10 page={address} rip=0x4e001230',
            '[paging.fault.rejected] target=15 task=35 token=10 terminal=1',
            f'[sext.fault.reject] target=15 token=10 page={address} reason=Quota used=4 quota=4',
            '[silkbar.clock.send] hh=1 mm=2 ss=3 status=0x0',
        ])
    return '\n'.join(rows)


class MultiPagingTests(unittest.TestCase):
    def test_positive_and_quota(self):
        self.assertEqual(checker.validate(good()), [])
        self.assertEqual(checker.validate(good(True), True), [])

    def test_each_proof_stage_required(self):
        for marker in ('boot.paging.genesis', 'paging.client.begin', 'paging.fault.trap', 'paging.fault.pending', 'paging.leaf.commit', 'paging.fault.completed', 'sext.fault.commit', 'sext.commit.negative', 'paging.client.page', 'paging.leaf.reclaim', 'sext.release', 'sext.reclaim.negative', 'sext.reuse.negative', 'sext.release.denied', 'paging.client.released', 'paging.client.reuse', 'paging.client.retained'):
            self.assertTrue(checker.validate(good().replace('['+marker+']', '[omitted]', 1)), marker)

    def test_foreign_identity_task_token_rip(self):
        for before, after in (('target=10 task=30', 'target=10 task=99'), ('target=10 page=0x400000000000 frame=0', 'target=15 page=0x400000000000 frame=0'), ('saved_rip=0x4e001230','saved_rip=0x4e001231'), ('task=30 token=1','task=30 token=0')):
            self.assertTrue(checker.validate(good().replace(before, after, 1)))

    def test_clear_and_tlb_before_free_required(self):
        for before, after in (('cleared=1','cleared=0'), ('tlb=1','tlb=0'), ('runtime_alloc=0','runtime_alloc=1')):
            self.assertTrue(checker.validate(good().replace(before, after, 1)))
        rows=good().splitlines(); i=next(i for i,r in enumerate(rows) if '[paging.leaf.reclaim]' in r); rows[i],rows[i+1]=rows[i+1],rows[i]
        self.assertTrue(checker.validate('\n'.join(rows)))

    def test_live_frame_alias_denied(self):
        self.assertTrue(checker.validate(good().replace('frame=4','frame=3')))

    def test_pkey_permissions_and_zero(self):
        for before, after in (('pkey=10 zero','pkey=15 zero'), ('rights=6','rights=7'), ('zeroed=4096','zeroed=2048'), ('first=write','first=read'), ('error=0x6','error=0x4')):
            self.assertTrue(checker.validate(good().replace(before, after, 1)))

    def test_stale_reclaim_generation_must_match(self):
        self.assertTrue(checker.validate(good().replace('old_token=1','old_token=4',1)))
        self.assertTrue(checker.validate(good().replace('stale_reclaim=1','stale_reclaim=0',1)))

    def test_replay_cannot_decrement_quota(self):
        self.assertTrue(checker.validate(good().replace('replay=1 used=4','replay=1 used=3')))

    def test_neighbor_check_after_reuse(self):
        rows=good().splitlines(); rows[-1],rows[-2]=rows[-2],rows[-1]
        self.assertTrue(checker.validate('\n'.join(rows)))

    def test_partial_duplicate_record_fails(self):
        self.assertTrue(checker.validate(good()+'\n[paging.leaf.reclaim] partial'))
        self.assertTrue(checker.validate(good().replace('cleared=1','cleared=1 cleared=1',1)))

    def test_quota_cannot_be_optional_or_resurrect(self):
        self.assertTrue(checker.validate(good(),True))
        self.assertTrue(checker.validate(good(True)))
        self.assertTrue(checker.validate(good(True)+'\ntask.running id=35 pd_id=15',True))
        self.assertTrue(checker.validate(good(True).replace('status=0x0','status=0x01'),True))

    def test_client_arenas_cannot_alias(self):
        self.assertTrue(checker.validate(good().replace('base=0x400000010000','base=0x400000000000')))

    def test_duplicate_witness_cannot_cover_distinct_generations(self):
        for marker in ('sext.commit.negative', 'sext.reclaim.negative', 'sext.reuse.negative', 'sext.client.ready'):
            rows = good().splitlines()
            first = next(r for r in rows if '[' + marker + ']' in r)
            rows = [first if '[' + marker + ']' in r else r for r in rows]
            self.assertTrue(checker.validate('\n'.join(rows)), marker)

    def test_release_replay_requires_exact_original_binding(self):
        rows = good().splitlines()
        rows = [r.replace('token=5', 'token=999').replace('frame=0', 'frame=7')
                if '[sext.release]' in r and 'replay=1' in r else r for r in rows]
        self.assertTrue(checker.validate('\n'.join(rows)))

    def test_fresh_release_witnesses_are_one_per_reclaim(self):
        rows = good().splitlines()
        first = next(r for r in rows if '[sext.release]' in r and 'replay=0' in r)
        rows = [first if '[sext.release]' in r and 'replay=0' in r else r for r in rows]
        self.assertTrue(checker.validate('\n'.join(rows)))

    def test_summary_cannot_precede_its_instructions(self):
        for marker in ('paging.client.released', 'paging.client.recovered', 'paging.client.reuse'):
            rows = good().splitlines()
            index = next(i for i, r in enumerate(rows) if '[' + marker + ']' in r)
            row = rows.pop(index)
            rows.insert(6, row)
            self.assertTrue(checker.validate('\n'.join(rows)), marker)

    def test_quota_rejection_has_exact_page_and_stage_order(self):
        rows = good(True).splitlines()
        index = next(i for i, r in enumerate(rows) if '[paging.fault.rejected]' in r)
        reject = rows.pop(index)
        before = next(i for i, r in enumerate(rows) if '[paging.client.quota.begin]' in r)
        rows.insert(before, reject)
        self.assertTrue(checker.validate('\n'.join(rows), True))
        rows = good(True).splitlines()
        rows = [r.replace('page=0x400000014000', 'page=0x400000015000')
                if '[sext.fault.reject]' in r else r for r in rows]
        self.assertTrue(checker.validate('\n'.join(rows), True))

    def test_instruction_result_precedes_release_of_that_binding(self):
        rows = good().splitlines()
        index = next(i for i, r in enumerate(rows) if '[paging.client.page] pd=10 page=0 ' in r)
        page = rows.pop(index)
        after = next(i for i, r in enumerate(rows) if '[paging.leaf.reclaim] token=1 ' in r)
        rows.insert(after + 1, page)
        self.assertTrue(checker.validate('\n'.join(rows)))

    def test_sext_free_confirmation_precedes_reuse(self):
        rows = good().splitlines()
        index = next(i for i, r in enumerate(rows) if '[sext.release] target=10 request=1 ' in r)
        release = rows.pop(index)
        after = next(i for i, r in enumerate(rows) if '[paging.leaf.commit] token=5 ' in r)
        rows.insert(after + 1, release)
        self.assertTrue(checker.validate('\n'.join(rows)))

    def test_same_frame_wrong_retired_generation_rejected(self):
        self.assertTrue(checker.validate(good().replace('frame=0 old_token=5 token=9 old_target=15',
                                                       'frame=0 old_token=1 token=9 old_target=10')))

    def test_valid_user_return_before_pager_diagnostics(self):
        rows = good().splitlines()
        index = next(i for i, r in enumerate(rows) if '[paging.client.page] pd=15 page=0 ' in r)
        page = rows.pop(index)
        after = next(i for i, r in enumerate(rows) if '[paging.fault.completed]' in r and 'token=5 ' in r)
        rows.insert(after + 1, page)
        self.assertEqual(checker.validate('\n'.join(rows)), [])

    def test_malformed_leaf_identity_reports_failure_without_exception(self):
        for value in ('', 'target=not-a-number ', 'target=99 '):
            rows = good().splitlines()
            index = next(i for i, r in enumerate(rows) if '[paging.leaf.commit]' in r)
            rows[index] = rows[index].replace('target=10 ', value)
            self.assertTrue(checker.validate('\n'.join(rows)))

    def test_fatal_fault_fails(self):
        self.assertTrue(checker.validate(good()+'\n[paging.client.fatal] error'))


if __name__ == '__main__':
    unittest.main()
