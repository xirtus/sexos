#!/usr/bin/env python3
"""Adversarial checker tests. The valid fixture is synthetic, NOT guest proof."""
import importlib.util
from pathlib import Path
import unittest

CHECKER = Path(__file__).resolve().parents[1] / "check_app_resource_log.py"
spec = importlib.util.spec_from_file_location("app_resource_checker", CHECKER)
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)

# Explicit parser fixture only: no build, hardware, scheduler, or guest claim.
SYNTHETIC_VALID = """[boot.app.resources] target=15 task=32783 surface=300 slot=40 private_pages=32 private_bytes=131072 required=31 pager=14 coordinator=3 metadata=retained
[shell.app_surface.accept] sid=300 caller=15 frame=12
[display.app.admitted] target=15 surface=300 owner=15
[display.app.presented] target=15 surface=300 scene=1
[app.resources.acquired] target=15 surface=300 handle=7 notification=1 private_heap=elf_owned resident=4 visible_ticks=128
[task.finish] pd=15 task=32783 reason=0 terminal=1
[shell.app.cleanup] target=15 task=32783 surface=300 tabs=1 focus=200 retired=1
[shell.app.cleanup.ack] target=15 task=32783 surface=300 ok=1
[app.service.ack] target=15 task=32783 holder=3 role=1 acked=1 required=31
[display.app.cleanup.prepared] target=15 task=32783 removed=1 focus=0 tombstone=1
[display.app.cleanup.committed] target=15 task=32783 scene=1
[display.app.cleanup.ack] target=15 task=32783 role=display
[app.service.ack] target=15 task=32783 holder=1 role=2 acked=3 required=31
[files.app.cleanup] target=15 task=32783 handles=1 removed=1
[files.app.cleanup.ack] target=15 task=32783 ok=1
[app.service.ack] target=15 task=32783 holder=11 role=4 acked=7 required=31
[net.app.cleanup.ack] target=15 task=32783 ok=1
[app.service.ack] target=15 task=32783 holder=13 role=8 acked=15 required=31
[bell.app.cleanup] target=15 task=32783 events=1 callbacks=0 policies=0 mute=0 spam=0
[bell.app.cleanup.ack] target=15 task=32783 ok=1
[app.service.ack] target=15 task=32783 holder=10 role=16 acked=31 required=31
[paging.target.cleanup] target=15 task=32783 frames=0x0f leaves=4 closed=1 detached=1 all_flushed=1 runtime_alloc=0
[app.private.retired] target=15 task=32783 pages=32 bytes=131072 acked=31 required=31 all_flushed=1 pool_reusable=1 metadata=retained runtime_alloc=0 messages=0 signals=0
[sext.app.pool.ready] target=15 task=32783 bytes=131072 zeroed=131072 workspace_records=2 reused=1 zero_owner=sext ok=1
[silkbar.clock.send] h=10 m=45 s=48
"""

def line(text, marker):
    return next(row for row in text.splitlines() if row.startswith("[" + marker + "]"))

def without(text, marker):
    return text.replace(line(text, marker) + "\n", "", 1)

def moved_before(text, marker, before):
    row = line(text, marker)
    text = without(text, marker)
    return text.replace(line(text, before), row + "\n" + line(text, before), 1)

class AppResourceLogTests(unittest.TestCase):
    def reject(self, text):
        self.assertTrue(checker.validate(text), "invalid synthetic evidence was accepted")

    def test_valid_synthetic_parser_fixture_is_not_guest_proof(self):
        self.assertEqual(checker.validate(SYNTHETIC_VALID), [])

    def test_each_missing_service_role_is_rejected(self):
        for role in (1, 2, 4, 8, 16):
            with self.subTest(role=role):
                rows = [r for r in SYNTHETIC_VALID.splitlines() if not (r.startswith("[app.service.ack]") and f" role={role} " in r)]
                self.reject("\n".join(rows))

    def test_duplicate_role_ack_is_rejected(self):
        row = line(SYNTHETIC_VALID, "app.service.ack")
        self.reject(SYNTHETIC_VALID.replace(row, row + "\n" + row))

    def test_wrong_task_on_every_identity_bearing_cleanup_is_rejected(self):
        for marker in ("shell.app.cleanup", "shell.app.cleanup.ack", "display.app.cleanup.prepared",
                       "display.app.cleanup.committed", "display.app.cleanup.ack", "files.app.cleanup",
                       "files.app.cleanup.ack", "net.app.cleanup.ack", "bell.app.cleanup",
                       "bell.app.cleanup.ack", "app.service.ack", "paging.target.cleanup",
                       "app.private.retired", "sext.app.pool.ready", "task.finish"):
            with self.subTest(marker=marker):
                row = line(SYNTHETIC_VALID, marker)
                self.reject(SYNTHETIC_VALID.replace(row, row.replace("task=32783", "task=15")))

    def test_partial_zeroing_and_unreused_pool_are_rejected(self):
        self.reject(SYNTHETIC_VALID.replace("zeroed=131072", "zeroed=4096"))
        self.reject(SYNTHETIC_VALID.replace("reused=1", "reused=0"))

    def test_presentation_after_exit_is_rejected(self):
        self.reject(moved_before(SYNTHETIC_VALID, "display.app.presented", "shell.app.cleanup"))

    def test_missing_private_or_demand_flush_is_rejected(self):
        for marker in ("app.private.retired", "paging.target.cleanup"):
            with self.subTest(marker=marker):
                row = line(SYNTHETIC_VALID, marker)
                self.reject(SYNTHETIC_VALID.replace(row, row.replace("all_flushed=1", "all_flushed=0")))
                self.reject(without(SYNTHETIC_VALID, marker))

    def test_demand_private_and_zero_order_is_enforced(self):
        self.reject(moved_before(SYNTHETIC_VALID, "app.private.retired", "paging.target.cleanup"))
        self.reject(moved_before(SYNTHETIC_VALID, "sext.app.pool.ready", "app.private.retired"))

    def test_desktop_clock_must_continue_after_pool_reuse(self):
        self.reject(without(SYNTHETIC_VALID, "silkbar.clock.send"))
        self.reject(moved_before(SYNTHETIC_VALID, "silkbar.clock.send", "sext.app.pool.ready"))

    def test_real_handle_surface_and_exact_page_accounting_are_required(self):
        for old, new in (("handle=7", "handle=0"), ("pages=32 bytes=", "pages=31 bytes="),
                         ("leaves=4", "leaves=3"), ("scene=1", "scene=0")):
            with self.subTest(old=old):
                self.reject(SYNTHETIC_VALID.replace(old, new))

    def test_finish_cannot_precede_acquisition_or_presentation(self):
        self.reject(moved_before(SYNTHETIC_VALID, "task.finish", "display.app.presented"))

    def test_cleanup_cannot_run_before_exact_task_finishes(self):
        self.reject(moved_before(SYNTHETIC_VALID, "shell.app.cleanup", "task.finish"))

    def test_display_ack_cannot_precede_scene_commit(self):
        self.reject(moved_before(SYNTHETIC_VALID, "display.app.cleanup.ack", "display.app.cleanup.committed"))

    def test_display_commit_cannot_precede_prepare(self):
        self.reject(moved_before(SYNTHETIC_VALID, "display.app.cleanup.committed", "display.app.cleanup.prepared"))

    def test_standard_unbracketed_fault_kill_and_panic_markers_are_fatal(self):
        for fatal in ("fault.kill [paging.fault.terminal] reason=ineligible", "[paging.client.fatal] panic", "[sext.fatal] invalid"):
            with self.subTest(fatal=fatal):
                self.reject(SYNTHETIC_VALID + fatal + "\n")

    def test_atomic_records_after_unrelated_partial_prefix_remain_visible(self):
        value=SYNTHETIC_VALID.replace("[files.app.cleanup]", "[noisy.partial] task=123 tail=[files.app.cleanup]")
        self.assertEqual(checker.validate(value),[])

    def test_following_record_cannot_supply_missing_task(self):
        self.reject(SYNTHETIC_VALID.replace("[files.app.cleanup] target=15 task=32783", "[files.app.cleanup] target=15[other] task=32783"))

    def test_preceding_record_cannot_supply_missing_target(self):
        self.reject(SYNTHETIC_VALID.replace("[files.app.cleanup] target=15 task=32783", "[other] target=15[files.app.cleanup] task=32783"))

if __name__ == "__main__":
    unittest.main()
