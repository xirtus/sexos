"""Synthetic negative controls; no native launch or console proof is asserted."""
from pathlib import Path
import sys
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
  self.assertEqual(len(hashes),3)
  self.assertTrue(all(len(digest)==64 for digest in hashes.values()))

if __name__=='__main__':unittest.main()
