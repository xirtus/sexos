"""Auxiliary ACPI admission never broadens the Net causal proof."""
from pathlib import Path
import shutil
import sys
import tempfile
import unittest
ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'scripts'))
import check_self_wait as gate
from test_self_wait_source import SAMPLED,EXPIRY,copy_causal_sources
from https_asset_gate import browser_wait_tasks

ROLE='[kernel.spawn.sexacpi] id=50 key=4 organ=senses\n'
PARK='[scheduler.self_wait.park] task=50 ticks=64 deadline=200 count=16\n'
DUE='[scheduler.self_wait.expired] task=50 deadline=200 now=200 count=32\n'
TRACE=ROLE+SAMPLED+PARK+DUE

class AuxiliaryAcpi(unittest.TestCase):
    def test_exact_role_and_source_admit_only_auxiliary_interval(self):
        result=gate.inspect(TRACE,source_root=ROOT)
        self.assertTrue(result['ok'],result)
        self.assertTrue(result['auxiliary_wait_profiles']['acpi']['ok'])
        self.assertEqual(len(result['inferred_timer_user_joins']),1)
        self.assertEqual(result['inferred_timer_user_joins'][0]['task'],13)

    def test_no_source_or_missing_duplicate_late_wrong_role_never_admits(self):
        self.assertFalse(gate.inspect(TRACE)['ok'])
        for trace in (TRACE.replace(ROLE,''),TRACE+ROLE,SAMPLED+ROLE+PARK+DUE,
                      TRACE.replace('key=4','key=5'),TRACE.replace('id=50 key','id=51 key'),
                      TRACE.replace('organ=senses','organ=other'),TRACE.replace(ROLE,ROLE.rstrip()+' extra=1\n')):
            with self.subTest(trace=trace):self.assertFalse(gate.inspect(trace,source_root=ROOT)['ok'])

    def test_other_intervals_tasks_early_expiry_or_missing_net_join_reject(self):
        for trace in (TRACE.replace('task=50 ticks=64','task=50 ticks=63'),
                      TRACE.replace('task=50','task=51'),TRACE.replace('now=200','now=199'),
                      TRACE.replace(EXPIRY,'')):
            with self.subTest(trace=trace):self.assertFalse(gate.inspect(trace,source_root=ROOT)['ok'])
        self.assertFalse(gate.inspect(TRACE.replace('[smp.online] cores=1 present=1','[smp.online] cores=2 present=2'),source_root=ROOT)['ok'])

    def test_auxiliary_changed_or_missing_source_cannot_admit_park_or_expiry(self):
        with tempfile.TemporaryDirectory() as name:
            root=Path(name);copy_causal_sources(root)
            self.assertFalse(gate.inspect(TRACE,source_root=root)['ok'])
            for file in gate.ACPI_AUX_SOURCES:
                p=root/file;p.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(ROOT/file,p)
            self.assertTrue(gate.inspect(TRACE,source_root=root)['ok'])
            for file in gate.ACPI_AUX_SOURCES:
                p=root/file;old=p.read_bytes();p.write_bytes(old+b'\n// unreviewed\n')
                self.assertFalse(gate.inspect(TRACE,source_root=root)['ok'])
                self.assertFalse(gate.inspect(TRACE.replace(PARK,''),source_root=root)['ok'])
                p.write_bytes(old)

    def test_asset_caller_selects_only_its_exact_observed_boot_browser(self):
        spawn='[kernel.spawn.kaleidoscope] id=15 domain_id=15\n'
        handoff='[bootgraph.boot.handoff] target=kaleidoscope id=15 entry=0x40001234\n'
        seal='[boot.self_wait.sealed] tasks=50 capacity=64 max_ticks=64 runtime_registration=0\n'
        self.assertEqual(browser_wait_tasks(spawn+seal+handoff),(15,))
        for trace in (handoff+seal,spawn+seal,spawn+spawn+seal+handoff,spawn+seal+handoff+handoff,
                      seal+spawn+handoff,spawn+handoff+seal,spawn+seal+handoff+handoff.replace('id=15','id=16'),
                      (spawn+seal+handoff).replace('id=15','id=16')):
            with self.subTest(trace=trace),self.assertRaises(ValueError):browser_wait_tasks(trace)

if __name__=='__main__':unittest.main()
