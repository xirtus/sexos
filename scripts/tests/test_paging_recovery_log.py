import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("paging_log", Path(__file__).resolve().parents[1] / "check_paging_recovery_log.py")
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)


def good(quota=False):
    rows = [
        "[boot.paging.genesis] pager=14 target=15 pool_pages=8 arena_pages=8 quota=8 target_pkey=15 leaves_absent=8 tables_preallocated=1",
        "[boot.memory.authority.sealed] runtime_alloc=0 runtime_map=capability_leaf runtime_mint=0",
        "[sext.ready] target=15 frames=8 quota=4 pool_bytes=32768 arena_bytes=32768 zero_owner=sext",
        "[paging.client.negative] invalid_slot=1 high_alias=1 wrong_query=1 wrong_commit=1 wrong_reject=1 ok=1",
        "[paging.client.begin] pd=15 base=0x400000000000 bytes=32768",
    ]
    for page in range(4):
        token = page + 1
        address = hex(0x400000000000 + 4096 * page)
        rows += [
            f"[paging.fault.trap] pd=15 addr={address} rip=0x4e001230 error={hex(6 if page & 1 else 4)} eligible=1",
            f"[paging.fault.pending] target=15 task=27 token={token} page={address} rip=0x4e001230",
            f"[paging.leaf.commit] token={token} target=15 page={address} frame={page} rights=6 pkey=15 zero=1 runtime_alloc=0",
            f"[paging.fault.completed] target=15 task=27 token={token} resume=1 saved_rip=0x4e001230",
            f"[sext.fault.commit] target=15 token={token} page={address} frame={page} used={token} zeroed=4096 ok=1",
            f"[sext.commit.negative] token={token} exact=1 conflicting_frame=1 zero_token=1 unknown_token=1 ok=1",
            f"[paging.client.page] pd=15 page={page} first={'write' if page & 1 else 'read'} zero=1 value=1 tail=1 ok=1",
        ]
    rows += ["[paging.client.recovered] pd=15 pages=4 retries=4 retained=4 ok=1"]
    if quota:
        rows += [
            "[paging.client.quota.begin] pd=15 page=4 in_range=1",
            "[paging.fault.trap] pd=15 addr=0x400000004000 rip=0x4e001230 error=0x4 eligible=1",
            "[paging.fault.pending] target=15 task=27 token=5 page=0x400000004000 rip=0x4e001230",
            "[paging.fault.rejected] target=15 task=27 token=5 terminal=1",
            "[sext.fault.reject] target=15 token=5 page=0x400000004000 reason=Quota used=4 quota=4",
            "[silkbar.clock.send] hh=1 mm=2 ss=3 status=0x0",
        ]
    return "\n".join(rows)


class PagingRecoveryLogTests(unittest.TestCase):
    def test_complete_normal_and_quota(self):
        self.assertFalse(checker.validate(good()))
        self.assertFalse(checker.validate(good(True), True))

    def test_missing_stage_and_empty_log(self):
        self.assertTrue(checker.validate(""))
        for marker in ("paging.fault.pending", "paging.leaf.commit", "paging.fault.completed", "sext.commit.negative", "paging.client.recovered"):
            self.assertTrue(checker.validate(good().replace(marker, "omitted")))

    def test_foreign_token_task_and_rip(self):
        for before, after in (("task=27 token=1 resume=1", "task=28 token=1 resume=1"),
                              ("token=1 target=15", "token=9 target=15"),
                              ("saved_rip=0x4e001230", "saved_rip=0x4e001231")):
            self.assertTrue(checker.validate(good().replace(before, after, 1)))

    def test_frame_reuse_and_rights(self):
        for before, after in (("frame=1", "frame=0"), ("rights=6", "rights=7"), ("pkey=15 zero", "pkey=14 zero"), ("runtime_alloc=0", "runtime_alloc=1")):
            self.assertTrue(checker.validate(good().replace(before, after, 1)))

    def test_zeroing_and_write_retry_required(self):
        for before, after in (("zeroed=4096", "zeroed=2048"), ("first=write", "first=read"), ("error=0x6", "error=0x4")):
            self.assertTrue(checker.validate(good().replace(before, after, 1)))

    def test_completion_before_leaf_rejected(self):
        rows = good().splitlines()
        rows[7], rows[8] = rows[8], rows[7]
        self.assertTrue(checker.validate("\n".join(rows)))

    def test_extra_duplicate_leaf_not_idempotent(self):
        leaf = next(line for line in good().splitlines() if "[paging.leaf.commit]" in line)
        self.assertTrue(checker.validate(good() + "\n" + leaf))

    def test_quota_is_not_optional_or_foreign(self):
        self.assertTrue(checker.validate(good(), True))
        self.assertTrue(checker.validate(good(True)))
        self.assertTrue(checker.validate(good(True).replace("task=27 token=5 terminal", "task=28 token=5 terminal"), True))

    def test_other_domain_must_progress_after_rejection(self):
        self.assertTrue(checker.validate(good(True).replace("silkbar.clock.send", "omitted"), True))
        self.assertTrue(checker.validate(good(True).replace("status=0x0", "status=0xdead"), True))
        self.assertTrue(checker.validate(good(True).replace("status=0x0", "status=0x01"), True))

    def test_quota_cannot_switch_tasks_or_resurrect(self):
        self.assertTrue(checker.validate(good(True).replace("task=27 token=5", "task=999 token=5"), True))
        self.assertTrue(checker.validate(good(True) + "\ntask.running id=27 pd_id=15 rip=0x123 rsp=0x456", True))
        self.assertTrue(checker.validate(good().replace("task=27", "task=0")))

    def test_boot_and_negative_probe_stage_order(self):
        rows = good().splitlines()
        rows.insert(0, rows.pop(2))
        self.assertTrue(checker.validate("\n".join(rows)))
        rows = good().splitlines()
        rows.append(rows.pop(3))
        self.assertTrue(checker.validate("\n".join(rows)))

    def test_malformed_extra_record_fails(self):
        self.assertTrue(checker.validate(good() + "\n[paging.leaf.commit] truncated"))

    def test_fatal_or_spurious_fault_fails(self):
        self.assertTrue(checker.validate(good() + "\nfault.kill unexpected"))
        self.assertTrue(checker.validate(good().replace("eligible=1", "eligible=0", 1)))


if __name__ == "__main__":
    unittest.main()
