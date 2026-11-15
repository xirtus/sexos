# SPDX-License-Identifier: GPL-3.0-only
"""Exercise actual gate Driver methods without starting a VM."""
import ast,re,tempfile,unittest
from pathlib import Path
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
  self.clock=Clock();ns=dict(Probe=Probe,re=re,time=self.clock,wav=wav)
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
 def test_odd_pointer_value_uses_acknowledged_even_keyboard_origin(self):
  state={'value':49};acks=[]
  self.d.text=lambda:f"[moan.mixer.track] track=0 name=volume x=8 y=455 w=118 h=26 value={state['value']}\n"
  def wait(pattern,start,timeout):
   value=int(re.search(r'value=(-?\d+)',pattern)[1]);acks.append(value);state['value']=value
  self.d.g={'wait':wait};self.d.mix(3,0,'volume',50)
  self.assertEqual(acks,[0,20,40,42,44,46,48,50]);self.assertEqual(state['value'],50)
  downs=[e['data']['key']['data']for _,data in self.d.q.events for e in data['events']if e['data']['down']]
  self.assertEqual(downs,['home','pgup','pgup','right','right','right','right','right'])
 def test_pointer_uses_completed_local_refresh_newest_record(self):
  self.d.text=lambda:'[moan.control] name=Master x=20 y=30\n[moan.control] name=Master x=50 y=60\n'
  self.d.click(3,'Master');self.assertEqual(self.moves,[(58,122)]);self.assertEqual(self.buttons,[True,False])
 def test_pointer_rejects_offscreen_control(self):
  self.d.text=lambda:'[moan.control] name=Master x=1264 y=60\n'
  with self.assertRaises(AssertionError):self.d.click(3,'Master')
  self.assertEqual(self.moves,[])
if __name__=='__main__':unittest.main()
