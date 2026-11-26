# SPDX-License-Identifier: GPL-3.0-only
import pathlib,sys,unittest
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parents[1]))
from moan_recovery_receipts import admission,SERVICES
class Admission(unittest.TestCase):
 def trace(self):
  lines=['[moan.role.reserved] pd=42 task=420 generation=7']
  for phase in range(1,10):
   for service in SERVICES:lines.append(f'[moan.role.policy] service={service} phase={phase} pd=42 task=420 generation=7 complete=1')
  lines.append('[moan.role.copied] pd=42 task=421 generation=7')
  for phase in (10,11):
   for service in SERVICES:lines.append(f'[moan.role.policy] service={service} phase={phase} pd=42 task=421 generation=7 complete=1')
  lines+=['[moan.role.active] pd=42 task=421 generation=7','[moanplug.bootstrap] pd=42 task=421 scrubbed=1 hashes=1','[moan.role.ready] pd=42 task=421','[moanplug.native] pd=42 task=421 ready=1']
  return '\n'.join(lines)+'\n'
 def test_complete_actual_schema_changes_task_only_at_copy(self):
  trace=self.trace();proof=admission('old\n'+trace,42,421,4);self.assertEqual(proof['complete_policy_joins'],44);self.assertEqual(proof['previous_task'],420);self.assertEqual(proof['native_offset'],4+trace.index('[moanplug.native]'))
 def test_child_may_run_before_shell_post_commit_log_lines(self):
  trace=self.trace();active='[moan.role.active] pd=42 task=421 generation=7\n';boot='[moanplug.bootstrap] pd=42 task=421 scrubbed=1 hashes=1\n';ready='[moan.role.ready] pd=42 task=421\n';native='[moanplug.native] pd=42 task=421 ready=1\n'
  reordered=trace.replace(active+boot+ready+native,boot+active+native+ready)
  self.assertEqual(admission(reordered,42,421)['complete_policy_joins'],44)
 def test_missing_wrong_actor_and_old_generation_do_not_form_quorum(self):
  trace=self.trace();needle='[moan.role.policy] service=13 phase=11 pd=42 task=421 generation=7 complete=1\n'
  for replacement in ('',needle.replace('task=421','task=420'),needle.replace('generation=7','generation=6'),needle.replace('service=13','service=12'),needle.replace('complete=1','complete=0')):
   with self.subTest(replacement=replacement),self.assertRaises(ValueError):admission(trace.replace(needle,replacement),42,421)
 def test_early_activation_native_or_missing_ready_rejected(self):
  trace=self.trace();active='[moan.role.active] pd=42 task=421 generation=7\n';native='[moanplug.native] pd=42 task=421 ready=1\n'
  for broken in (active+trace.replace(active,''),native+trace.replace(native,''),trace.replace('[moan.role.ready] pd=42 task=421\n','')):
   with self.assertRaises(ValueError):admission(broken,42,421)
if __name__=='__main__':unittest.main()
