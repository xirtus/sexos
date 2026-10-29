import pathlib,re,sys,unittest
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parents[1]))
from quil_third_probe import open_selected

class OpenCompletionTests(unittest.TestCase):
 def context(self,intent=True,loaded=True):
  state=dict(log='',events=[],pending=False)
  class Q:
   def key(self,key):
    assert key=='ret';state['events'].append('ret');state['pending']=True
    state['log']+='[linen.open.queued] id=7 accepted=1\n'
  def wait(pattern,start=0):
   state['events'].append(pattern)
   if 'open.complete' in pattern:
    # Completion can arrive after any former one-second polling cutoff.
    if intent:state['log']+='[linen.quil.disk_doc.intent] object_id=7 path_id=6 ok=1\n'
    state['log']+='[linen.open.complete] status=0\n';state['pending']=False
   if 'persist.load.ok' in pattern:
    assert not state['pending']
    if loaded:state['log']+='[quil.persist.load.ok] bytes=123 hash=0x1234 doc=6\n'
   match=re.search(pattern,state['log'][start:])
   if match is None:raise RuntimeError('required receipt absent')
   return match
  return dict(text=lambda:state['log'],q=Q(),wait=wait),state
 def test_delayed_intent_waits_completion_and_exact_document_load(self):
  c,s=self.context();self.assertEqual(open_selected(c),dict(object=6,bytes=123,hash='0x1234'))
  self.assertFalse(s['pending']);self.assertIn('open.complete',s['events'][2]);self.assertIn('doc=6',s['events'][3])
 def test_completed_non_document_has_no_fabricated_load(self):
  c,s=self.context(intent=False);self.assertIsNone(open_selected(c));self.assertFalse(s['pending']);self.assertEqual(len(s['events']),3)
 def test_missing_load_never_becomes_success_from_completion(self):
  c,s=self.context(loaded=False)
  with self.assertRaisesRegex(RuntimeError,'required receipt absent'):open_selected(c)
 def test_previous_intent_and_load_do_not_satisfy_new_row_open(self):
  c,s=self.context(intent=False);s['log']='[linen.quil.disk_doc.intent] object_id=7 path_id=6 ok=1\n[quil.persist.load.ok] bytes=123 hash=0x1234 doc=6\n'
  self.assertIsNone(open_selected(c));self.assertEqual(len(s['events']),3)
if __name__=='__main__':unittest.main()
