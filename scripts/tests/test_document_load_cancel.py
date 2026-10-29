import unittest,sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from document_window_gate import validate_cancel_load_trace

TRACE='''[files.document.load.begin] caller=9 object=5 identity=393217 cooperative=1
[shell.app.close.request] target=9 reason=keyboard
[files.document.retire] target=9 staged=0 pending=0 terminal=1
'''+''.join(f'[app.service.ack] target=9 role={r} ok=1\n'for r in [1,2,4,8,16])+'''[sext.app.pool.ready] target=9 ok=1
'''
class CancelLoadTrace(unittest.TestCase):
 def test_exact_pending_close_cleanup(self):self.assertEqual(validate_cancel_load_trace(TRACE,5),[1,2,4,8,16])
 def test_missing_admission_wrong_object_or_incomplete_cleanup_fails(self):
  for trace in [TRACE.replace('caller=9','caller=16'),TRACE.replace('object=5','object=6'),TRACE.replace('role=8','role=2'),TRACE.replace('staged=0','staged=1'),TRACE.replace('ok=1\n','ok=0\n')]:
   with self.assertRaises(RuntimeError):validate_cancel_load_trace(trace,5)
 def test_already_loaded_and_late_loaded_both_fail(self):
  for trace in ['[quil.persist.load.ok] bytes=12287\n'+TRACE,TRACE+'[quil.persist.load.ok] bytes=12287\n']:
   with self.assertRaises(RuntimeError):validate_cancel_load_trace(trace,5)
if __name__=='__main__':unittest.main()
