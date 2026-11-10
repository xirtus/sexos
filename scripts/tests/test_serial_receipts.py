"""Live UART writes must finish a record before its fields become evidence."""
import pathlib,re,sys,unittest
from unittest.mock import patch
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parents[1]))
from quil_autosave_gate import wait_serial,check_serial_health

RESULT=r'\[web.console\] level=0 jit-result ([^\r\n]+)'
VALUE='124929280|2147483648|-2147483649|4294967296|true|a1|2'
LINE='[web.console] level=0 jit-result '+VALUE

class SerialReceiptTests(unittest.TestCase):
 def wait(self,chunks,pattern=RESULT,**kwargs):
  """Advance the growing file by one observed UART write per poll."""
  state={'text':'','reads':0};chunks=iter(chunks)
  def text():
   state['text']+=next(chunks,'');state['reads']+=1;return state['text']
  def health():check_serial_health(state['text'],'test')
  with patch('quil_autosave_gate.time.sleep'):
   match=wait_serial(text,health,pattern,timeout=.01,tag='test',**kwargs)
  return match,state
 def test_actual_ci_partial_result_waits_for_rest_and_newline(self):
  partial='[web.console] level=0 jit-result 124929280|2147483648|-2147'
  match,state=self.wait([partial,LINE[len(partial):],'\n'])
  self.assertEqual(match[1],VALUE);self.assertEqual(state['reads'],3)
 def test_every_byte_prefix_waits_for_terminator(self):
  for ending in ('\n','\r\n'):
   match,state=self.wait(list(LINE+ending))
   self.assertEqual(match[1],VALUE);self.assertEqual(state['reads'],len(LINE+ending))
 def test_complete_numeric_counters_and_timing_not_prefixes(self):
  for line,pattern,expected in (
   ('[web.jit] script_ticks=12345',r'script_ticks=(\d+)','12345'),
   ('[web.ready] target=41 task=1234',r'task=(\d+)','1234'),
   ('[web.console] level=0 moved after 2345',r'moved after (\d+)','2345')):
   match,_=self.wait(list(line+'\n'),pattern);self.assertEqual(match[1],expected)
 def test_no_newline_is_no_receipt(self):
  with self.assertRaises(TimeoutError):self.wait([LINE])
 def test_complete_malformed_record_is_not_discarded(self):
  match,_=self.wait([LINE+'|unexpected\n']);self.assertNotEqual(match[1],VALUE)
  match,_=self.wait(['[web.console] level=0 jit-result malformed\n'+LINE+'\n'])
  self.assertEqual(match[1],'malformed')
 def test_start_and_match_offsets_preserved(self):
  prefix='old receipt\n';match,_=self.wait([prefix+LINE+'\n'],start=len(prefix))
  self.assertEqual(match.start(),0);self.assertEqual(match.end(),len(LINE))
 def test_only_explicit_raw_mode_can_match_unterminated_menu(self):
  pattern='SexOS compact 1024x768'
  match,_=self.wait([pattern],pattern,raw=True);self.assertEqual(match[0],pattern)
  with self.assertRaises(TimeoutError):self.wait([pattern],pattern)
 def test_unterminated_fault_fails_even_when_no_receipt_completes(self):
  for fault in ('KERNEL PANIC','KERNEL PAGE FAULT','DOUBLE FAULT','GP FAULT','panicked at','serial.record.overflow'):
   for raw in (False,True):
    with self.assertRaisesRegex(RuntimeError,'guest failure'):
     self.wait([fault[:3],fault[3:]],raw=raw)
 def test_health_runs_before_accepting_a_complete_receipt(self):
  def health():check_serial_health('KERNEL PANIC','test')
  with self.assertRaisesRegex(RuntimeError,'guest failure'):
   wait_serial(lambda:LINE+'\n',health,RESULT)

if __name__=='__main__':unittest.main()
