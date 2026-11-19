# SPDX-License-Identifier: GPL-3.0-only
"""Exercise actual gate Driver methods without starting a VM."""
import ast,re,tempfile,unittest,sys
from unittest.mock import patch
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import moan_gate_geometry as geometry
SCRIPT=Path(__file__).resolve().parents[1]/'moan_workspace_gate.py'
class Probe:pass
class Queue:
 def __init__(self):self.events=[]
 def cmd(self,*args):self.events.append(args)
class Clock:
 def __init__(self):self.delays=[]
 def sleep(self,value):self.delays.append(value)
class Inputs(unittest.TestCase):
 def setUp(self):
  self.tmp=tempfile.TemporaryDirectory();self.addCleanup(self.tmp.cleanup)
  wav=Path(self.tmp.name)/'test.wav';wav.write_bytes(bytes(444))
  self.clock=Clock();ns=dict(Probe=Probe,re=re,time=self.clock,wav=wav);self.ns=ns
  tree=ast.parse(SCRIPT.read_text());node=next(n for n in tree.body if isinstance(n,ast.ClassDef)and n.name=='Driver')
  exec(compile(ast.Module(body=[node],type_ignores=[]),str(SCRIPT),'exec'),ns)
  self.d=ns['Driver']();self.d.q=Queue();self.d.refresh=lambda surface:(0,(8,62),(1264,730));self.moves=[];self.buttons=[]
  self.d.move=lambda x,y:self.moves.append((x,y));self.d.button=self.buttons.append
 def test_make_release_pair_is_held_and_reversed(self):
  self.d.chord('ctrl','right');events=self.d.q.events
  self.assertEqual(self.clock.delays,[.04,.12]);self.assertEqual([e['data']['key']['data']for e in events[1][1]['events']],['right','ctrl'])
  self.assertTrue(all(e['data']['down']for e in events[0][1]['events']));self.assertTrue(all(not e['data']['down']for e in events[1][1]['events']))
 def test_latest_matching_value_is_no_op_despite_queued_old_value(self):
  self.d.text=lambda:'[moan.mixer.control] name=master_level x=20 y=30 w=118 h=26 value=9800\n[moan.mixer.control] name=master_level x=20 y=30 w=118 h=26 value=10000\n'
  self.assertEqual(self.d.mix(3,None,'master_level',100),100);self.assertEqual(self.moves,[]);self.assertEqual(self.d.q.events,[])
 def test_driver_refresh_converts_relative_wait_match_to_absolute_offset(self):
  prefix='old partial telemetry';trace=prefix+'\n[moan.viewport] w=624 h=410 frame=12\n[moan.timeline] x=2 y=3 w=600 h=200 start=0 span=192000\n[moan.viewport] w=624 h=410 frame=13\n';state={'text':prefix}
  self.d.text=lambda:state['text'];self.d.chord=lambda*keys:state.update(text=trace)
  self.d.g={'wait':lambda pattern,start,timeout:re.search(pattern,state['text'][start:])}
  expected=trace.index('[moan.timeline]')
  def viewport(g,surface,since):self.assertEqual(since,expected);return(0,28,640,452),(624,410)
  self.ns.update(viewport=viewport,origin=lambda tile,size:(8,62))
  self.assertEqual(self.ns['Driver'].refresh(self.d,360)[0],expected)
 def test_odd_pointer_value_uses_acknowledged_even_keyboard_origin(self):
  state={'value':49};acks=[]
  self.d.text=lambda:f"[moan.mixer.track] track=0 name=volume x=8 y=455 w=118 h=26 value={state['value']}\n"
  def wait(pattern,start,timeout):
   value=int(re.search(r'value=(-?\d+)',pattern)[1]);acks.append(value);state['value']=value
  self.d.g={'wait':wait};self.d.mix(3,0,'volume',50)
  self.assertEqual(acks,[0,20,40,42,44,46,48,50]);self.assertEqual(state['value'],50)
  downs=[e['data']['key']['data']for _,data in self.d.q.events for e in data['events']if e['data']['down']]
  self.assertEqual(downs,['home','pgup','pgup','right','right','right','right','right'])
 def test_pointer_already_reached_max_does_not_wait_for_noop_end(self):
  state={'value':100}
  self.d.text=lambda:f"[moan.mixer.track] track=0 name=volume x=8 y=455 w=118 h=26 value={state['value']}\n"
  self.d.move=lambda x,y:state.update(value=200)
  self.d.g={'wait':lambda*args:self.fail('target already achieved: no extra key receipt exists')}
  self.assertEqual(self.d.mix(3,0,'volume',200),100)
  self.assertEqual(self.d.q.events,[]);self.assertEqual(self.buttons,[True,False])
 def test_pointer_uses_completed_local_refresh_newest_record(self):
  self.d.text=lambda:'[moan.control] name=Master x=20 y=30\n[moan.control] name=Master x=50 y=60\n'
  self.d.click(3,'Master');self.assertEqual(self.moves,[(58,122)]);self.assertEqual(self.buttons,[True,False])
 def test_pointer_rejects_offscreen_control(self):
  self.d.text=lambda:'[moan.control] name=Master x=1264 y=60\n'
  with self.assertRaises(AssertionError):self.d.click(3,'Master')
  self.assertEqual(self.moves,[])
class LegacyFrameFence(unittest.TestCase):
 def test_held_native_chord_pairs_same_device_queue(self):
  q=Queue()
  with patch.object(geometry.time,'sleep')as sleep:
   geometry.chord({'q':q},'ctrl','shift','l')
  self.assertEqual([call.args[0]for call in sleep.call_args_list],[.04,.12])
  self.assertEqual([e['data']['key']['data']for e in q.events[1][1]['events']],['l','shift','ctrl'])
 def test_refresh_ignores_a_viewport_from_an_already_started_burst(self):
  prefix='partial old controls';trace=prefix+'\n[moan.viewport] w=624 h=410 frame=12\n[moan.timeline] x=2 y=3 w=600 h=200 start=0 span=192000\n[moan.viewport] w=624 h=410 frame=13\n'
  reads=[prefix,trace];calls=[]
  def text():return reads.pop(0)if len(reads)>1 else reads[0]
  def wait(pattern,since,timeout):
   calls.append((pattern,since));return re.compile(pattern).search(trace[since:])
  start=trace.index('[moan.timeline]')
  def viewport(g,surface,since):
   self.assertEqual(since,start);return(0,28,640,452),(624,410)
  with patch.object(geometry,'chord'),patch.object(geometry,'viewport',side_effect=viewport):
   result=geometry.refresh({'focus':lambda surface:None,'text':text,'wait':wait},360)
  self.assertEqual(result[0],start);self.assertIn('moan.timeline',calls[0][0]);self.assertEqual(calls[0][1],len(prefix))
if __name__=='__main__':unittest.main()
