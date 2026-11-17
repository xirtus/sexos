"""Source-bound UP inference is separate from the original observed receipt proof."""
from pathlib import Path
import hashlib
import json
import shutil
import sys
import tempfile
import unittest

ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'scripts'))
from check_self_wait import inspect,source_contract,APPROVED_TIMED_WAIT_SOURCES,DEADLINE_REFERENCE_OWNERS,APPROVED_KERNEL_RUST_TREES

BASE='''[kernel.spawn.sexnet] id=13 path=/servers/sexnet
[smp.online] cores=1 present=1
[smp.core.first_task] core=0 pd=1 tsc=100
[boot.self_wait.sealed] tasks=31 capacity=64 max_ticks=64 runtime_registration=0
[sexnet.wait.authority] invalid_intervals=3 spare_arguments=4 ok=1
[scheduler.self_wait.park] task=2 ticks=1 deadline=50 count=1
'''
EXPIRY='[scheduler.self_wait.expired] task=13 deadline=102 now=103 count=16\n'
RETURN='[sexnet.receive.wait] before=100 after=104 ticks=2 count=2 kind=timer_assisted_poll irq=0 device=1 reply_pending=0\n'
SAMPLED=BASE+EXPIRY+RETURN
OBSERVED=BASE+'[scheduler.self_wait.park] task=13 ticks=2 deadline=102 count=2\n'+EXPIRY+RETURN


def copy_causal_sources(root):
    names=set(APPROVED_TIMED_WAIT_SOURCES)
    names.update(path.relative_to(ROOT).as_posix()for path in (ROOT/'kernel').rglob('*.rs'))
    for name in names:
        target=root/name;target.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(ROOT/name,target)


class SourceBoundSelfWaitTests(unittest.TestCase):
    def check(self,text,root=ROOT):
        return inspect(text,source_root=root)

    def test_unsampled_park_requires_reviewed_source_and_actual_up_topology(self):
        self.assertFalse(inspect(SAMPLED)['ok'])
        result=self.check(SAMPLED)
        self.assertTrue(result['ok'],result)
        self.assertEqual(result['observed_timer_user_joins'],[])
        self.assertEqual(len(result['inferred_timer_user_joins']),1)
        self.assertEqual(result['inferred_timer_user_joins'][0]['evidence_type'],
                         'source_bound_up_expiry_return_inferred_park')
        self.assertEqual(set(result['source_contract']['source_sha256']),set(APPROVED_TIMED_WAIT_SOURCES))
        self.assertTrue(result['up_topology']['ok'])

    def test_per_task_sample_joins_when_global_ordinal_is_unsampled(self):
        # Other tasks' waits took the global samples; Net's own early records still log.
        expiry='[scheduler.self_wait.expired] task=13 deadline=102 now=103 count=37 task_count=5\n'
        late_return=RETURN.replace('count=2','count=20')
        result=self.check(BASE+expiry+late_return)
        self.assertTrue(result['ok'],result)
        self.assertEqual(result['inferred_timer_user_joins'][0]['expiry_count'],37)
        # An unsampled per-task ordinal, or a Net return past the first 32, is no proof.
        for bad in (BASE+expiry.replace('task_count=5','task_count=33')+late_return,
                    BASE+expiry+RETURN.replace('count=2','count=33')):
            self.assertFalse(self.check(bad)['ok'])
        # A task's own ordinals must increase; tasks past the BOOT slots carry 0.
        repeated=BASE+'[scheduler.self_wait.park] task=13 ticks=2 deadline=100 count=5 task_count=5\n'+'[scheduler.self_wait.park] task=13 ticks=2 deadline=101 count=6 task_count=5\n'+expiry+late_return
        self.assertIn('repeated/backward/unsampled per-task diagnostic ordinal',self.check(repeated)['errors'])
        foreign=BASE+'[scheduler.self_wait.park] task=13 ticks=2 deadline=100 count=5 task_count=0\n'+expiry+late_return
        self.assertFalse(self.check(foreign)['ok'])

    def test_original_observed_chain_does_not_need_source_or_up_inference(self):
        for topology in ('','[smp.online] cores=2 present=2\n'):
            result=inspect(OBSERVED.replace('[smp.online] cores=1 present=1\n',topology))
            self.assertTrue(result['ok'],result)
            self.assertEqual(len(result['observed_timer_user_joins']),1)
            self.assertEqual(result['inferred_timer_user_joins'],[])
            self.assertEqual(result['timer_user_joins'][0]['evidence_type'],'observed_park_expiry_return')

    def test_smp_can_expire_before_park_and_never_enables_inference(self):
        for topology in ('','[smp.online] cores=2 present=2\n','[smp.online] cores=1 present=2\n',
                         '[smp.online] cores=1 present=1\n'*2,'[smp.online] malformed\n',
                         '[smp.online] cores=01 present=1\n'):
            result=self.check(SAMPLED.replace('[smp.online] cores=1 present=1\n',topology))
            self.assertFalse(result['ok'],result)
            self.assertEqual(result['inferred_timer_user_joins'],[])
        for extra in ('[smp.core.first_task] core=1 pd=13 tsc=200\n','[smp.core.first_task] malformed\n'):
            self.assertFalse(self.check(SAMPLED+extra)['ok'])

    def test_inference_requires_unique_actual_net_boot_task_identity(self):
        record='[kernel.spawn.sexnet] id=13 path=/servers/sexnet\n'
        for text in (SAMPLED.replace(record,''),SAMPLED.replace('id=13 path=','id=14 path='),
                     SAMPLED+record,SAMPLED.replace('/servers/sexnet','/servers/other'),
                     SAMPLED.replace(record,'')+record):
            result=self.check(text);self.assertFalse(result['ok'],result)
            self.assertEqual(result['inferred_timer_user_joins'],[])

    def test_missing_early_foreign_and_unmatched_expiry_cannot_qualify(self):
        for text in (SAMPLED.replace(EXPIRY,''),SAMPLED.replace('now=103','now=101'),
                     SAMPLED.replace('task=13 deadline','task=16 deadline'),
                     SAMPLED.replace('deadline=102 now=103','deadline=100 now=101'),
                     SAMPLED.replace('count=16','count=5')):
            result=self.check(text);self.assertFalse(result['ok'],result)

    def test_return_must_be_forward_complete_ordered_and_uniquely_correlated(self):
        for text in (SAMPLED.replace(RETURN,''),SAMPLED.replace('after=104','after=99'),
                     SAMPLED.replace('before=100','before=101'),SAMPLED.replace('after=104','after=102'),
                     SAMPLED.replace('ticks=2 count=2','ticks=1 count=2'),
                     SAMPLED.replace(' device=1 reply_pending=0',''),
                     BASE+RETURN+EXPIRY):
            result=self.check(text);self.assertFalse(result['ok'],result)
        ambiguous=SAMPLED+RETURN.replace('count=2','count=4')
        self.assertFalse(self.check(ambiguous)['ok'])
        two_expiries=BASE+EXPIRY+EXPIRY.replace('deadline=102 now=103 count=16','deadline=103 now=104 count=32')+RETURN
        self.assertFalse(self.check(two_expiries)['ok'])

    def test_idle_input_and_denim_need_their_reviewed_waits_and_role(self):
        idle='[scheduler.self_wait.park] task=4 ticks=32 deadline=200 count=2\n'
        self.assertFalse(inspect(SAMPLED+idle)['ok'])
        self.assertTrue(self.check(SAMPLED+idle)['ok'])
        self.assertFalse(self.check(SAMPLED+idle.replace('ticks=32','ticks=33'))['ok'])
        denim='[scheduler.self_wait.park] task=51 ticks=64 deadline=200 count=64\n[scheduler.self_wait.expired] task=51 deadline=200 now=200 count=128\n'
        role='[kernel.spawn.denim] id=51 key=14 organ=memory\n'
        self.assertFalse(self.check(SAMPLED+denim)['ok'])
        self.assertTrue(self.check(role+SAMPLED+denim)['ok'])
        for bad in (role.replace('key=14','key=15'),role+role,''):
            self.assertFalse(self.check(bad+SAMPLED+denim)['ok'])
        self.assertFalse(self.check(role+SAMPLED+denim.replace('ticks=64','ticks=63'))['ok'])

    def test_quil_autosave_wait_needs_its_reviewed_wait_and_role(self):
        quil='[scheduler.self_wait.park] task=9 ticks=64 deadline=200 count=40 task_count=1\n[scheduler.self_wait.expired] task=9 deadline=200 now=200 count=41 task_count=1\n'
        role='[kernel.spawn.quil] id=9 path=/servers/quil\n'
        self.assertFalse(self.check(SAMPLED+quil)['ok'])
        self.assertTrue(self.check(role+SAMPLED+quil)['ok'])
        for bad in (role.replace('id=9','id=8'),role+role):
            self.assertFalse(self.check(bad+SAMPLED+quil)['ok'])
        self.assertFalse(self.check(role+SAMPLED+quil.replace('ticks=64','ticks=63'))['ok'])
        self.assertFalse(self.check(SAMPLED+role+quil)['ok'])

    def test_sex_pdx_module_list_is_outside_the_wait_fingerprint(self):
        from check_self_wait import wait_fingerprint
        name='crates/sex-pdx/src/lib.rs';data=(ROOT/name).read_bytes()
        self.assertIn(wait_fingerprint(name,data),APPROVED_TIMED_WAIT_SOURCES[name])
        self.assertIn(wait_fingerprint(name,data+b'\npub mod another_service;\n'),APPROVED_TIMED_WAIT_SOURCES[name])
        self.assertNotIn(wait_fingerprint(name,data+b'\npub fn f()->u64{0}\n'),APPROVED_TIMED_WAIT_SOURCES[name])

    def test_quiet_usb_32_is_only_allowed_with_exact_reviewed_predicate(self):
        usb='[scheduler.self_wait.park] task=5 ticks=32 deadline=200 count=2\n'
        self.assertFalse(inspect(SAMPLED+usb)['ok'])
        self.assertTrue(self.check(SAMPLED+usb)['ok'])
        for bad in (usb.replace('ticks=32','ticks=31'),usb.replace('task=5','task=7')):
            self.assertFalse(self.check(SAMPLED+bad)['ok'])

    def test_changed_cancellation_interrupt_clock_quiet_and_deadline_sources_fail_closed(self):
        mutations=[
            ('kernel/src/init.rs','sexnet_id = id;','sexnet_id = 14;'),
            ('kernel/src/pd/create.rs','pd_id, entry.as_u64(), stack_top, pd_ref, true','pd_id+1, entry.as_u64(), stack_top, pd_ref, true'),
            ('kernel/src/scheduler.rs','ParkResult::WakeConsumed=>{(*task).wait_deadline.cancel();true}',
             'ParkResult::WakeConsumed=>{true}'),
            ('kernel/src/scheduler.rs','ParkResult::Exited=>{(*task).wait_deadline.cancel();yield_and_switch(regs_ptr)}',
             'ParkResult::Exited=>{yield_and_switch(regs_ptr)}'),
            ('kernel/src/scheduler.rs','pub unsafe fn wait_for_message_ticks(regs_ptr:*const crate::interrupts::SyscallRegs,ticks:u64)->bool{\n    x86_64::instructions::interrupts::disable();',
             'pub unsafe fn wait_for_message_ticks(regs_ptr:*const crate::interrupts::SyscallRegs,ticks:u64)->bool{'),
            ('kernel/src/scheduler.rs','unsafe { (*task_ptr).wait_deadline.cancel(); }','unsafe { }'),
            ('kernel/src/scheduler.rs','(*task).wait_deadline.cancel();\n    (*task).state.exit();','(*task).state.exit();'),
            ('kernel/src/scheduler.rs','ParkResult::NotOwner=>{(*task).wait_deadline.cancel();panic!("TIMED_WAIT_WITHOUT_ATTACHED_OWNER")}',
             'ParkResult::NotOwner=>{panic!("TIMED_WAIT_WITHOUT_ATTACHED_OWNER")}'),
            ('kernel/src/self_wait.rs','if at==0||now<at{return None;}','if at==0{return None;}'),
            ('kernel/src/interrupts.rs','crate::scheduler::poll_self_wait_deadlines(TICKS.load(Ordering::Relaxed));',''),
            ('crates/sex-pdx/src/lib.rs','inlateout("rax") SYSCALL_GET_TICKS => ret','inlateout("rax") 99u64 => ret'),
            ('servers/sexnet/src/main.rs','let before=sex_pdx::get_ticks();','let before=0;'),
            ('servers/sexnet/src/main.rs','sex_pdx::sys_wait_self_ticks(2).expect','sex_pdx::sys_wait_self_ticks(1).expect'),
            ('servers/sexusb/src/main.rs','let quiet = irq && !queued && !usb.changed && !slept && !usb.ether_active() && !usb.bt_hid_active();',
             'let quiet = true;'),
        ]
        with tempfile.TemporaryDirectory()as directory:
            root=Path(directory)
            copy_causal_sources(root)
            self.assertTrue(source_contract(root)['ok'])
            for name,before,after in mutations:
                with self.subTest(file=name,mutation=before):
                    path=root/name;original=path.read_text();self.assertIn(before,original)
                    path.write_text(original.replace(before,after,1))
                    result=self.check(SAMPLED,root)
                    self.assertFalse(result['ok'],result)
                    self.assertFalse(result['source_contract']['ok'])
                    self.assertEqual(result['inferred_timer_user_joins'],[])
                    path.write_text(original)

    def test_new_kernel_writer_or_poller_outside_reviewed_closure_disables_inference(self):
        references=('(*task).wait_deadline.arm(0,2);','crate::scheduler::poll_self_wait_deadlines(99);',
                    'deadline.take_due(99);','crate::scheduler::wait_for_message_ticks(frame,2);',
                    'crate::scheduler::wake_record_if_due(task,99);')
        with tempfile.TemporaryDirectory()as directory:
            root=Path(directory)
            copy_causal_sources(root)
            self.assertTrue(source_contract(root)['ok'])
            for reference in references:
                path=root/'kernel/src/new_unreviewed_writer.rs'
                path.write_text('unsafe fn extra(){'+reference+'}')
                result=self.check(SAMPLED,root)
                self.assertFalse(result['ok'],result)
                self.assertFalse(result['source_contract']['ok'])
                self.assertIn('unreviewed kernel deadline reference owner',result['source_contract']['errors'][0])
                path.unlink()
            # Even a new module without recognized tokens requires review;
            # the complete Rust tree requires review even for unrecognized syntax.
            path.write_text('fn unrelated_new_module(){}')
            self.assertFalse(self.check(SAMPLED,root)['ok']);path.unlink()
            name=next(p.relative_to(ROOT).as_posix()for p in (ROOT/'kernel').rglob('*.rs')
                      if p.relative_to(ROOT).as_posix()not in APPROVED_TIMED_WAIT_SOURCES)
            path=root/name;original=path.read_text();path.write_text(original+'\n// changed outside causal owners\n')
            self.assertFalse(self.check(SAMPLED,root)['ok']);path.write_text(original)
            result=source_contract(ROOT)
            self.assertEqual(set(result['deadline_reference_closure']['owner_sha256']),DEADLINE_REFERENCE_OWNERS)
            self.assertGreater(result['deadline_reference_closure']['files'],len(DEADLINE_REFERENCE_OWNERS))

    def test_exact_c209_review_receipt_preserves_all_deadline_owners(self):
        receipt=json.loads((ROOT/'docs/handoff/SELF_WAIT_C209_REVIEW.json').read_text())
        closure=receipt['deadline_reference_closure']
        hashes=closure['kernel_rust_sha256']
        self.assertEqual(len(hashes),139)
        digest=hashlib.sha256(json.dumps(hashes,sort_keys=True,separators=(',',':')).encode()).hexdigest()
        self.assertEqual(digest,closure['tree_sha256'])
        self.assertIn(digest,APPROVED_KERNEL_RUST_TREES)
        self.assertEqual(set(closure['owner_sha256']),DEADLINE_REFERENCE_OWNERS)
        for name,digest in closure['owner_sha256'].items():
            self.assertEqual(digest,hashes[name])
            self.assertIn(digest,APPROVED_TIMED_WAIT_SOURCES[name])
        for name,digest in receipt['source_sha256'].items():
            self.assertIn(digest,APPROVED_TIMED_WAIT_SOURCES[name])
        # No arbitrary extension of the audited tree can inherit its identity.
        hashes['kernel/src/unreviewed.rs']='0'*64
        mutated=hashlib.sha256(json.dumps(hashes,sort_keys=True,separators=(',',':')).encode()).hexdigest()
        self.assertNotIn(mutated,APPROVED_KERNEL_RUST_TREES)

    def test_missing_source_does_not_enable_inference(self):
        with tempfile.TemporaryDirectory()as directory:
            result=self.check(SAMPLED,Path(directory))
            self.assertFalse(result['ok']);self.assertFalse(result['source_contract']['ok'])
            self.assertEqual(result['inferred_timer_user_joins'],[])


if __name__=='__main__':unittest.main()
