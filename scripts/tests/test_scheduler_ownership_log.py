import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("owner_log", Path(__file__).resolve().parents[1] / "check_scheduler_ownership_log.py")
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)


def good():
    rows = [f"[scheduler.ownership] event={event} task=9 count=1" for event in
            ("park", "detached_wake", "post_stack_release", "user_resumed")]
    return "\n".join(rows + ["[scheduler.proof.last_runnable.begin] task=9 held=13 lane=bsp",
                            "[scheduler.proof.last_runnable.timer_wake] task=9",
                            "[scheduler.proof.last_runnable.pass] task=9 restored=13 ownership=preserved"])


class OwnershipLogTests(unittest.TestCase):
    def test_complete_normal_and_proof(self):
        self.assertFalse(checker.validate(good()))
        self.assertFalse(checker.validate(good(), True))

    def test_missing_transition_or_empty_log_fails(self):
        self.assertTrue(checker.validate(""))
        for event in ("park", "detached_wake", "post_stack_release", "user_resumed"):
            self.assertTrue(checker.validate(good().replace("event=" + event, "event=omitted")))

    def test_duplicate_or_nonmonotonic_counter_fails(self):
        for n in (0, 1):
            self.assertTrue(checker.validate(good() + f"\n[scheduler.ownership] event=park task=9 count={n}"))

    def test_missing_stage_is_not_proof(self):
        self.assertTrue(checker.validate(good().replace("last_runnable.timer_wake", "omitted"), True))

    def test_identity_and_token_count_must_match(self):
        for before, after in (("timer_wake] task=9", "timer_wake] task=8"), ("restored=13", "restored=12")):
            self.assertTrue(checker.validate(good().replace(before, after), True))

    def test_fault_signatures_fail(self):
        self.assertTrue(checker.validate(good() + "\nOUTGOING_STACK_NOT_RELEASED"))

    def test_duplicate_or_reordered_proof_fails(self):
        self.assertTrue(checker.validate(good() + "\n[scheduler.proof.last_runnable.timer_wake] task=9", True))
        rows = good().splitlines()
        rows[-1], rows[-3] = rows[-3], rows[-1]
        self.assertTrue(checker.validate("\n".join(rows), True))


if __name__ == "__main__":
    unittest.main()
