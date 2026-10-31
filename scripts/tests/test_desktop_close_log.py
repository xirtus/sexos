import importlib.util,pathlib,unittest
path=pathlib.Path(__file__).resolve().parents[1]/'check_desktop_close_log.py'
spec=importlib.util.spec_from_file_location('checker',path);checker=importlib.util.module_from_spec(spec);spec.loader.exec_module(checker)
def fixture():
    lines=['[display.app.presented] target=15 surface=300 scene=1']
    for target,sid,native,slot in [(15,300,300,43),(9,201,156,41),(12,153,154,42)]:
        lines.extend([f'[shell.app.close.request] target={target} task={target} surface={sid}',f'[shell.app.close.send] target={target} task={target} slot={slot} tick=16 accepted=1',f'[app.close.accepted] target={target} task={target} coordinator=3',f'[task.finish] pd={target} task={target} reason=108 terminal=1',f'[display.app.cleanup.prepared] target={target} task={target} removed=2 focus=0 tombstone=1',f'[display.app.cleanup.committed] target={target} task={target} scene=1',f'[shell.app.cleanup] target={target} task={target} surface={native} tabs=1 focus=200 retired=1'])
        mask=0
        for role,holder in [(1,3),(2,1),(4,11),(8,13),(16,10)]:
            mask|=role;lines.append(f'[app.service.ack] target={target} task={target} holder={holder} role={role} acked={mask} required=31')
        lines.extend([f'[app.private.retired] target={target} task={target} pages=2 bytes=8192 acked=31 required=31 all_flushed=1 pool_reusable=1 messages=0 signals=0 metadata=retained runtime_alloc=0',f'[sext.app.pool.ready] target={target} task={target} bytes=8192 zeroed=8192 workspace_records=1 reused=1 zero_owner=sext ok=1'])
    lines.extend(['[linen.remote.entry] slot=0 id=1 kind=0 name_len=3','[linen.remote.entry] slot=1 id=2 kind=0 name_len=3','[linen.remote.snapshot.ok] count=2 unique=1 complete=1 selection_preserved=1','[shell.linen.rescan.sent] ok=1','[shell.action.select_next_linen] id=2','[linen.disk.rescan.skip] gen=3 reason=unchanged','[linen.remote.entry] slot=0 id=1 kind=0 name_len=3','[linen.remote.entry] slot=1 id=2 kind=0 name_len=3','[linen.remote.snapshot.ok] count=2 unique=1 complete=1 selection_preserved=1','[linen.object_select.current] id=2','[shell.action.select_next_linen] id=2'])
    return '\n'.join(lines)+'\n'
class Checks(unittest.TestCase):
    def test_valid_three_client_close(self):self.assertEqual(checker.check(fixture()),[])
    def test_kaleido_direct_finish_needs_no_extra_diagnostic(self):
        self.assertEqual(checker.check(fixture().replace('[app.close.accepted] target=15 task=15 coordinator=3\n','')),[])
    def test_quil_receiver_diagnostic_still_required(self):
        self.assertTrue(checker.check(fixture().replace('[app.close.accepted] target=9 task=9 coordinator=3\n','')))
    def test_kaleido_missing_successful_delivery_rejected(self):
        self.assertTrue(checker.check(fixture().replace('slot=43 tick=16 accepted=1','slot=43 tick=16 accepted=0')))
    def test_kaleido_send_diagnostic_can_follow_finish(self):
        lines=fixture().splitlines();send=lines.pop(2);finish=next(i for i,l in enumerate(lines) if l.startswith('[task.finish]'));lines.insert(finish+1,send)
        self.assertEqual(checker.check('\n'.join(lines)),[])
    def test_missing_ack(self):self.assertTrue(checker.check('\n'.join(l for l in fixture().splitlines() if not ('target=9 'in l and 'role=4'in l))))
    def test_duplicate_ack(self):line=next(l for l in fixture().splitlines()if l.startswith('[app.service.ack]'));self.assertTrue(checker.check(fixture().replace(line,line+'\n'+line)))
    def test_foreign_task(self):self.assertTrue(checker.check(fixture().replace('[sext.app.pool.ready] target=12 task=12','[sext.app.pool.ready] target=12 task=15')))
    def test_actual_fatal_prefix(self):self.assertTrue(checker.check(fixture()+'fault.kill [paging.fault.terminal] pd=3\n'))
    def test_ack_before_finish(self):lines=fixture().splitlines();index=next(i for i,l in enumerate(lines)if l.startswith('[app.service.ack]'));line=lines.pop(index);lines.insert(2,line);self.assertTrue(checker.check('\n'.join(lines)))
    def test_send_diagnostic_can_follow_receiver_accept(self):lines=fixture().splitlines();lines[2],lines[3]=lines[3],lines[2];self.assertEqual(checker.check('\n'.join(lines)),[])
    def test_survivor_input_before_cleanup_insufficient(self):text=fixture();line='[shell.action.select_next_linen] id=2\n';self.assertTrue(checker.check(line+text.replace(line,'')))
    def test_nonzero_undeclared_pool_denied(self):self.assertTrue(checker.check(fixture().replace('zeroed=8192','zeroed=4096')))
    def test_dead_native_focus_rejected(self):self.assertTrue(checker.check(fixture().replace('surface=156 tabs=1 focus=200','surface=156 tabs=1 focus=156')))
    def test_ui_hide_can_retire_surface_before_attested_cleanup(self):self.assertEqual(checker.check(fixture().replace('removed=2','removed=0')),[])
    def test_display_resource_count_cannot_exceed_table(self):self.assertTrue(checker.check(fixture().replace('removed=2','removed=17')))
    def test_late_registration_rejected(self):self.assertTrue(checker.check(fixture()+'[shell.app_surface.accept] sid=300 title_id=300 frame=13 caps=0 app_id=15 caller=15\n'))
    def test_late_dead_alias_focus_rejected(self):self.assertTrue(checker.check(fixture()+'[shell.focus.set] id=201\n'))
    def test_atomic_records_can_follow_foreign_partial_line(self):
        text=''.join('[foreign.partial] target=99 task=88 '+line for line in fixture().splitlines(keepends=True));self.assertEqual(checker.check(text),[])
    def test_identity_cannot_be_borrowed_from_foreign_suffix(self):
        text=fixture().replace('[app.close.accepted] target=9 task=9 coordinator=3','[app.close.accepted] task=9 coordinator=3[foreign.partial] target=9');self.assertTrue(checker.check(text))
    def test_identity_cannot_be_borrowed_from_foreign_prefix(self):
        text=fixture().replace('[app.close.accepted] target=9 task=9 coordinator=3','[foreign.partial] target=9[app.close.accepted] task=9 coordinator=3');self.assertTrue(checker.check(text))
    def test_input_only_after_completion_is_not_responsiveness(self):
        text=fixture().replace('[shell.linen.rescan.sent] ok=1\n[shell.action.select_next_linen] id=2','[shell.linen.rescan.sent] ok=1');self.assertTrue(checker.check(text))
    def test_selection_must_remain_exact_old_view_object(self):
        self.assertTrue(checker.check(fixture().replace('[linen.object_select.current] id=2','[linen.object_select.current] id=1')))
    def test_real_backend_rescan_required(self):
        self.assertTrue(checker.check(fixture().replace('[linen.disk.rescan.skip] gen=3 reason=unchanged','')))
    def test_malformed_fields_fail_without_crash(self):
        for before,after in [('task=15','task=x'),('bytes=8192','bytes=x'),('role=4','role=x'),('removed=2','removed=x')]:self.assertTrue(checker.check(fixture().replace(before,after)))
    def test_declined_unsaved_close_then_confirmed_close(self):
        f=fixture();line=next(l for l in f.splitlines() if l.startswith('[shell.app.close.request] target=9 '))
        declined=line+'\n[shell.app.close.declined] target=9 task=9 reason=unsaved next_close=confirmed\n'+line
        self.assertEqual(checker.check(f.replace(line,declined,1)),[])
        self.assertTrue(checker.check(f.replace(line,line+'\n'+line,1)),'two requests without a decline')
        self.assertTrue(checker.check(f.replace(line,line+'\n[shell.app.close.declined] target=9 task=9 reason=unsaved next_close=confirmed',1)),'decline with no later request')

if __name__=='__main__':unittest.main()
