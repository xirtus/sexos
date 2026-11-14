"""Exact raw-master Denim admission is auxiliary to the unchanged Net proof."""
from pathlib import Path
import hashlib
import json
import shutil
import sys
import tempfile
import unittest
ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'scripts'))
import check_self_wait as gate
from test_self_wait_source import SAMPLED,EXPIRY,copy_causal_sources
ROLE='[kernel.spawn.denim] id=51 key=14 organ=memory\n'
HANDOFF='[bootgraph.boot.handoff] target=denim id=51 entry=0x40001234\n'
PARK='[scheduler.self_wait.park] task=51 ticks=64 deadline=200 count=16\n'
DUE='[scheduler.self_wait.expired] task=51 deadline=200 now=200 count=32\n'
TRACE=ROLE+SAMPLED+HANDOFF+PARK+DUE

class AuxiliaryDenim(unittest.TestCase):
    def test_exact_raw_cohort_role_and_both_intervals(self):
        for ticks in (8,64):
            result=gate.inspect(TRACE.replace('task=51 ticks=64',f'task=51 ticks={ticks}'),source_root=ROOT)
            self.assertTrue(result['ok'],result)
            self.assertTrue(result['auxiliary_wait_profiles']['denim']['ok'])
            self.assertEqual([j['task']for j in result['inferred_timer_user_joins']],[13])

    def test_missing_or_duplicate_wrong_or_misordered_role(self):
        for trace in (TRACE.replace(ROLE,''),TRACE+ROLE,SAMPLED+ROLE+HANDOFF+PARK+DUE,
                      TRACE.replace('id=51 key','id=52 key'),TRACE.replace('key=14','key=4'),
                      TRACE.replace('organ=memory','organ=senses'),TRACE.replace(ROLE,' '+ROLE),
                      TRACE.replace(ROLE,ROLE.rstrip()+' extra=1\n')):
            with self.subTest(trace=trace):self.assertFalse(gate.inspect(trace,source_root=ROOT)['ok'])

    def test_missing_duplicate_wrong_or_preseal_handoff(self):
        for trace in (TRACE.replace(HANDOFF,''),TRACE+HANDOFF,ROLE+HANDOFF+SAMPLED+PARK+DUE,
                      TRACE.replace('denim id=51 entry','denim id=52 entry'),
                      TRACE.replace(HANDOFF,HANDOFF.rstrip()+' extra=1\n'),
                      ROLE+SAMPLED+PARK+DUE+HANDOFF):
            with self.subTest(trace=trace):self.assertFalse(gate.inspect(trace,source_root=ROOT)['ok'])

    def test_missing_source_smp_and_missing_net_join_reject(self):
        self.assertFalse(gate.inspect(TRACE)['ok'])
        for trace in (TRACE.replace('[smp.online] cores=1 present=1','[smp.online] cores=2 present=2'),
                      TRACE.replace(EXPIRY,''),TRACE.replace('task=51 ticks=64','task=51 ticks=63'),
                      TRACE.replace('task=51','task=52'),TRACE.replace('now=200','now=199')):
            with self.subTest(trace=trace):self.assertFalse(gate.inspect(trace,source_root=ROOT)['ok'])

    def test_mutated_or_missing_auxiliary_sources_reject_even_expiry_only(self):
        with tempfile.TemporaryDirectory()as td:
            root=Path(td);copy_causal_sources(root)
            for name in gate.DENIM_AUX_SOURCES:
                p=root/name;p.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(ROOT/name,p)
            self.assertTrue(gate.inspect(TRACE,source_root=root)['ok'])
            for name in gate.DENIM_AUX_SOURCES:
                p=root/name;old=p.read_bytes()
                for mutation in (None,old+b'\n// unreviewed\n'):
                    if mutation is None:p.unlink()
                    else:p.write_bytes(mutation)
                    for trace in(TRACE,TRACE.replace(PARK,'')):
                        self.assertFalse(gate.inspect(trace,source_root=root)['ok'],name)
                    p.write_bytes(old)
            extra=root/'kernel/src/unreviewed.rs';extra.write_text('fn unrelated(){}')
            self.assertFalse(gate.inspect(TRACE,source_root=root)['ok'])
            extra.write_text('fn writer(){task.wait_deadline.arm(0,1);}')
            self.assertFalse(gate.inspect(TRACE,source_root=root)['ok'])

    def test_only_complete_bounded_possible_timer_records(self):
        for record in(PARK,DUE):
            for bad in(record.rstrip()+' trailing=1\n',record.rstrip(),
                       record.replace('deadline=200','deadline=\n200'),
                       record.replace('deadline=200','deadline=0'),
                       record.replace('deadline=200','deadline='+'9'*21),
                       record.replace('count=32','count=0').replace('count=16','count=0'),
                       record.replace('deadline=200','deadline=18446744073709551616')):
                with self.subTest(bad=bad):self.assertFalse(gate.inspect(TRACE.replace(record,bad),source_root=ROOT)['ok'])
        self.assertFalse(gate.inspect(TRACE.replace('task=51 ticks=64 deadline=200','task=51 ticks=64 deadline=63'),source_root=ROOT)['ok'])
        result=gate.inspect(TRACE.replace(PARK,'foreign prefix '+PARK).replace(DUE,'foreign prefix '+DUE),source_root=ROOT)
        self.assertTrue(result['ok'],result)

    def test_zero_prefixed_task_cannot_bypass_record_checks(self):
        for task in('051','0000000000000000000051'):
            for record in(PARK,DUE):
                for suffix in('', ' extra=1', ' deadline=0'):
                    bad=record.replace('task=51',f'task={task}').rstrip()+suffix+'\n'
                    self.assertFalse(gate.inspect(TRACE.replace(record,bad),source_root=ROOT)['ok'])
        source=gate.source_contract(ROOT)
        import re
        seals=list(re.finditer(r'^\[boot.self_wait.sealed\].*\n',TRACE,re.M))
        unrelated=TRACE+PARK.replace('task=51','task=510')
        self.assertTrue(gate._denim_wait_profile(unrelated,ROOT,source,seals)['ok'])

    def test_old_individually_approved_sdk_cannot_mix_with_denim_kernel(self):
        with tempfile.TemporaryDirectory()as td:
            root=Path(td);copy_causal_sources(root)
            p=root/'crates/sex-pdx/src/lib.rs'
            raw=p.read_bytes()
            self.assertEqual(raw.count(b'pub mod denim;\n'),1)
            old=raw.replace(b'pub mod denim;\n',b'',1)
            self.assertEqual(hashlib.sha256(old).hexdigest(),'f9dd629e5cdda6ba58fb130e988d33f0d3a88948278d2f414bf612dca02fc166')
            p.write_bytes(old)
            result=gate.source_contract(root)
            self.assertFalse(result['ok'])
            self.assertIn('mixed Denim causal',result['errors'][0])

    def test_source_receipt_includes_complete_before_after_manifests(self):
        receipt=json.loads((ROOT/'docs/handoff/SELF_WAIT_DENIM_REVIEW.json').read_text())
        before=receipt['before_sha256'];after=receipt['after_sha256']
        kernels={n:h for n,h in after.items()if n.startswith('kernel/')}
        self.assertEqual(len(kernels),140)
        self.assertEqual(len([n for n in before if n.startswith('kernel/')]),139)
        digest=hashlib.sha256(json.dumps(kernels,sort_keys=True,separators=(',',':')).encode()).hexdigest()
        self.assertEqual(digest,gate.DENIM_KERNEL_TREE)
        self.assertEqual(set(receipt['changed_paths']),{n for n in before.keys()|after.keys()if before.get(n)!=after.get(n)})
        for name,h in gate.DENIM_AUX_SOURCES.items():self.assertEqual(after[name],h)
        for name in ('kernel/src/scheduler.rs','kernel/src/self_wait.rs','kernel/src/interrupts.rs','kernel/src/program_records.rs'):
            self.assertEqual(before[name],after[name])

if __name__=='__main__':unittest.main()
