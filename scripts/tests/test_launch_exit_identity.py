"""Compile the actual Shell exit predicate; stale reused-PD receipts cannot finish a run."""
import re
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
class ExitIdentity(unittest.TestCase):
 def test_production_exact_run_predicate(self):
  source=(ROOT/'servers/silk-shell/src/main.rs').read_text()
  function=re.search(r'fn launch_exit_subject\([^\n]+\)->bool\{[^}]+\}',source)
  self.assertIsNotNone(function)
  self.assertIn('if launch_exit_subject(run.child,run.child_task,run.status,msg.caller_pd,msg.caller_task)',source)
  rust=function[0]+'''\n#[test]fn exact_run(){
   assert!(launch_exit_subject(42,101,None,42,101));
   assert!(!launch_exit_subject(42,101,None,42,100));
   assert!(!launch_exit_subject(42,101,None,43,101));
   assert!(!launch_exit_subject(42,101,Some(0),42,101));
  }'''
  with tempfile.TemporaryDirectory()as temp:
   path=Path(temp);(path/'predicate.rs').write_text(rust)
   subprocess.run([shutil.which('rustc'),'--test',str(path/'predicate.rs'),'-o',str(path/'test')],check=True,capture_output=True)
   subprocess.run([str(path/'test')],check=True,capture_output=True)
if __name__=='__main__':unittest.main()
