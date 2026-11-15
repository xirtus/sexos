"""Synthetic negative controls; no native launch or console proof is asserted."""
from pathlib import Path
import sys
import tempfile
import unittest
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import program_term_native_gate as gate

SETUP=('[files.program.bound] target=42 task=1073741836 root=/data/term\n'
       '[term.ready] target=42 task=1073741836 cols=56 rows=19\n')
ACTION=('[term.command] stdhello\n[term.native.start] name=stdhello\n'
        '[files.ticket.denied] requester=42\n[shell.program.denied] reason=store error=1\n'
        '[spindle.program.refused] code=1\n'
        '[sched.active] tick=78018 second_distinct=9\n'
        '[sched.active] tick=79018 second_distinct=9\n')

class DownstreamRelayDenial(unittest.TestCase):
 def prove(self,action=ACTION,setup=SETUP,prefix='old history\n'):
  trace=prefix+setup+action
  return gate.downstream_denial(trace,len(prefix),len(prefix+setup),len(trace),42,1073741836,{'source':'hash'})
 def test_exact_run_action_and_downstream_refusal_without_feedback_claim(self):
  proof=self.prove()
  self.assertTrue(proof['downstream_denied']);self.assertFalse(proof['child_started'])
  self.assertFalse(proof['relay_attached']);self.assertFalse(proof['feedback_qualified'])
  self.assertEqual(proof['observation_ticks'],[78018,79018])
 def test_wrong_requester_or_task_never_qualifies(self):
  with self.assertRaises(ValueError):self.prove(ACTION.replace('requester=42','requester=43'))
  with self.assertRaises(ValueError):self.prove(setup=SETUP.replace('1073741836','1073741835'))
 def test_stale_denial_cannot_replace_current_attempt(self):
  denial='[files.ticket.denied] requester=42\n'
  with self.assertRaises(ValueError):self.prove(ACTION.replace(denial,''),prefix=denial)
 def test_child_launch_bind_relay_or_activation_is_rejected(self):
  for extra in ('[shell.program.launch] target=43 task=9 surface=360\n',
                '[files.program.bound] target=43 task=9 root=/data/stdhello\n',
                '[files.console.attached] terminal_row=1 target=43 task=9\n',
                '[shell.launch.asked] parent=42 task=1073741836 cookie=1 network=0\n',
                '[shell.launch.event] parent=42 event=Started sent=1\n',
                '[app.activation.committed] logical=43 incarnation=1\n'):
   with self.subTest(extra=extra),self.assertRaises(ValueError):self.prove(ACTION+extra)
 def test_unrelated_store_rejection_or_wrong_order_is_rejected(self):
  for action in (ACTION.replace('[files.ticket.denied] requester=42\n',''),
                 ACTION.replace('reason=store error=1','reason=no_free_row'),
                 ACTION.replace('requester=42','requester=43')+'[files.ticket.denied] requester=42\n',
                 ACTION.replace('[term.command] stdhello\n','')+'[term.command] stdhello\n'):
   with self.subTest(action=action),self.assertRaises(ValueError):self.prove(action)
 def test_second_command_duplicate_refusal_and_no_progress_are_rejected(self):
  for action in (ACTION+'[term.command] help\n',ACTION+'[files.ticket.denied] requester=42\n',
                 ACTION.replace('tick=79018','tick=78018'),
                 ACTION.replace('[sched.active] tick=79018 second_distinct=9\n','')):
   with self.subTest(action=action),self.assertRaises(ValueError):self.prove(action)
 def test_source_contract_records_current_route(self):
  hashes=gate.denial_sources(gate.ROOT)
  self.assertEqual(set(hashes),{
   "servers/sexfiles/src/fs_service.rs","crates/sex-app/src/lib.rs",
   "servers/silk-shell/src/main.rs","servers/sexfiles/src/trampoline.rs"})
  self.assertTrue(all(len(digest)==64 for digest in hashes.values()))


class ExactTicketSourceContract(unittest.TestCase):
 def source_case(self,path,old,new):
  names=('servers/sexfiles/src/fs_service.rs','crates/sex-app/src/lib.rs',
         'servers/silk-shell/src/main.rs','servers/sexfiles/src/trampoline.rs')
  with tempfile.TemporaryDirectory()as folder:
   root=Path(folder)
   for name in names:
    text=(gate.ROOT/name).read_text()
    if name==path:
     self.assertIn(old,text,'mutation must actually change the current source')
     text=text.replace(old,new,1)
    target=root/name;target.parent.mkdir(parents=True,exist_ok=True);target.write_text(text)
   with self.assertRaisesRegex(ValueError,'legacy relay source contract changed'):
    gate.denial_sources(root)
 def test_owner_stamp_live_task_or_consumption_drift_refuses_contract(self):
  path='servers/sexfiles/src/fs_service.rs'
  cases=[
   ('struct TicketOwner { target: u32, task: u32 }','struct TicketOwner { target: u32 }'),
   ('sender_task != 0 && sender_task == bound_task','true'),
   ('bound_sender(caller, caller_task, b.task, r.kind == w::PREPARE)','program_live(caller, b.task)'),
   ('binding.task != 0 && program_live(target, binding.task)','binding.task != 0'),
   ('c.is_subject(target, row.surface, 3) && c.task_id == task','c.is_subject(target, row.surface, 3)'),
   ('TicketOwner { target, task: binding.task }','TicketOwner { target, task: 0 }'),
   ('PREPARED_OWNER } != owner || (by.is_some() && by != asker)','PREPARED_OWNER }.is_some() && (by.is_some() && by != asker)'),
   ('unsafe { PREPARED_OWNER = None; }','unsafe { PREPARED_OWNER = owner; }'),
   ('let result = run(fs, r, path, data, owner, relay);','let result = run(fs, r, path, data, None, relay);'),
   ('PREPARED_OWNER = owner; PREPARED_LAUNCHER = by;','PREPARED_OWNER = None; PREPARED_LAUNCHER = by;'),
  ]
  for old,new in cases:
   with self.subTest(old=old):self.source_case(path,old,new)
 def test_relay_grant_sanitizer_or_explicit_context_drift_refuses_contract(self):
  path='servers/sexfiles/src/fs_service.rs'
  for old,new in [
   ('is_some_and(|b| holds_launch(&b))','is_some_and(|_| true)'),
   ('if !crate::console_relay::may_prepare(k)','if false'),
   ('sex_pdx::console_relay::plain_args(','unreviewed_args('),
   ('let relay = if relay_terminal { program } else { None };','let relay = program;'),
   ('let by = relay;','let by = None;'),
   ('return prepare(fs, r, path, data, owner, relay);','return prepare(fs, r, path, data, owner, None);'),
  ]:
   with self.subTest(old=old):self.source_case(path,old,new)
 def test_transport_caller_stamp_and_legacy_requester_cannot_drift(self):
  self.source_case('servers/sexfiles/src/trampoline.rs',
   'fs_service::handle(caller,msg.caller_task,msg.arg0,msg.arg1,msg.arg2)',
   'fs_service::handle(caller,0,msg.arg0,msg.arg1,msg.arg2)')
  self.source_case('servers/silk-shell/src/main.rs',
   'if by_program&&!program_run(msg.caller_pd).is_some_and(|(_,task)|launch_caller_task(task,msg.caller_task)){return true}',
   'if by_program&&false{return true}')
  self.source_case('servers/silk-shell/src/main.rs',
   'if by_program{msg.caller_pd as u64}else{0}','0')
  self.source_case('servers/silk-shell/src/main.rs',
   'current_task!=0 && current_task==caller_task','current_task!=0')
  self.source_case('crates/sex-app/src/lib.rs',
   'sex_pdx::program_store::OP_RUN, sex_pdx::program_store::TICKET, ticket as u64',
   'sex_pdx::program_store::OP_RUN, 0, ticket as u64')
 def test_owner_and_sender_checks_must_precede_consumption_or_forwarding(self):
  path='servers/sexfiles/src/fs_service.rs'
  source=(gate.ROOT/path).read_text()
  guard=next(line for line in source.splitlines(True)if 'if unsafe { PREPARED_OWNER } != owner' in line)
  loaded='    unsafe { LOADED_LAUNCHER = by; }\n'
  consumed='    let len = core::mem::take(&mut prepared.1);\n'
  self.source_case(path,guard+loaded+consumed,consumed+guard+loaded)
  path='servers/silk-shell/src/main.rs';source=(gate.ROOT/path).read_text()
  start=source.index('    if msg.type_id==store::OP_RUN {')
  end=source.index('        PROGRAM_READING=status==0;',start)
  old=source[start:end]
  guard=next(line for line in old.splitlines(True)if 'if by_program&&!program_run' in line)
  self.source_case(path,old,old.replace(guard,'')+guard)
 def test_comment_only_old_guard_does_not_qualify(self):
  old='bound_task != 0 && program_live(target, bound_task) && (!preparing || (sender_task != 0 && sender_task == bound_task))'
  self.source_case('servers/sexfiles/src/fs_service.rs',old,'true /* '+old+' */')

if __name__=='__main__':unittest.main()
