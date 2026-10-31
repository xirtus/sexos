"""Counterexamples for bounded native-profile control and exact journal mapping."""
import struct,sys,unittest
from pathlib import Path
from unittest.mock import Mock,patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import pending_io_close as gate

def prepared_disk(payload=gate.BASELINE):
 slot=5;identity=(slot+1)<<16|3;start=2048;disk=bytearray((start+128)*512)
 manifest=2046*512+16+slot*32;disk[manifest]=1;struct.pack_into('<I',disk,manifest+24,65536);struct.pack_into('<H',disk,manifest+30,3)
 descriptor=(1925-slot)*512;disk[descriptor:descriptor+4]=b'SFEX';struct.pack_into('<H',disk,descriptor+6,1);struct.pack_into('<HH',disk,descriptor+12,start,128)
 header=bytearray(512);header[:4]=b'QJ02';header[4]=slot;struct.pack_into('<I',header,8,identity);struct.pack_into('<Q',header,16,1);struct.pack_into('<I',header,24,len(payload));struct.pack_into('<Q',header,32,gate.fnv(payload));struct.pack_into('<Q',header,504,gate.fnv(header[:504]));disk[start*512:start*512+512]=header;disk[start*512+512:start*512+512+len(payload)]=payload
 return disk

class PendingProfile(unittest.TestCase):
 def test_only_actual_complete_baseline_maps_the_journal(self):
  ranges,identity=gate.journal_ranges(prepared_disk(),5);self.assertEqual((ranges,identity),([(2048,2176)],6<<16|3))
  for disk in [prepared_disk(gate.SNAPSHOT),prepared_disk(gate.BASELINE+b'z')]:
   with self.assertRaises(ValueError):gate.journal_ranges(disk,5)
  disk=prepared_disk();disk[2048*512+512]^=1
  with self.assertRaises(ValueError):gate.journal_ranges(disk,5)
 def test_logical_text_limit_is_not_a_physical_journal_size(self):
  disk=prepared_disk();struct.pack_into('<I',disk,2046*512+16+5*32+24,12288)
  with self.assertRaisesRegex(ValueError,'fully grown'):gate.journal_ranges(disk,5)
 def controller(self,fail_arm=False,wrong_backend=False):
  q=Mock();calls=[]
  def command(op,args=None):
   calls.append((op,args))
   if op=='query-block':return[dict(device='other'if wrong_backend else'nvm')]
   if fail_arm and op=='block_set_io_throttle'and args['iops_wr']==2:raise RuntimeError('arm failed')
  q.cmd.side_effect=command
  profile=gate.CriticalProfile('/owned/control.sock',Mock());profile.done.set()
  return profile,q,calls
 def test_independent_connection_restores_limits_and_disables_every_event(self):
  p,q,calls=self.controller()
  with patch.object(gate,'QMP',return_value=q):p.start();proof=p.finish()
  self.assertTrue(proof['restored']);self.assertLess(proof['armed_seconds'],8)
  self.assertEqual([a['iops_wr']for op,a in calls if op=='block_set_io_throttle'],[2,0])
  self.assertEqual([a['name']for op,a in calls if op=='trace-event-set-state'and not a['enable']],list(gate.EVENTS));q.file.close.assert_called_once();q.sock.close.assert_called_once();p.stop_guest.assert_not_called()
 def test_failed_arm_is_not_success_and_still_restores_the_backend(self):
  p,q,calls=self.controller(fail_arm=True)
  with patch.object(gate,'QMP',return_value=q):
   with self.assertRaisesRegex(RuntimeError,'could not arm'):p.start()
   with self.assertRaisesRegex(RuntimeError,'restore failed'):p.finish()
  self.assertEqual([a['iops_wr']for op,a in calls if op=='block_set_io_throttle'],[2,0]);self.assertEqual(sum(op=='trace-event-set-state'and not a['enable']for op,a in calls),4)
 def test_unknown_backend_cannot_arm_a_different_disk(self):
  p,q,calls=self.controller(wrong_backend=True)
  with patch.object(gate,'QMP',return_value=q):
   with self.assertRaises(RuntimeError):p.start()
   with self.assertRaises(RuntimeError):p.finish()
  self.assertFalse(any(op=='block_set_io_throttle'and a['iops_wr']==2 for op,a in calls))
 def test_absolute_deadline_uses_independent_owned_guest_kill(self):
  kill=Mock();p=gate.CriticalProfile('/owned/control.sock',Mock(),kill_guest=kill)
  # No QMP command or main-thread finish is needed to enforce the fallback.
  p.expire();kill.assert_called_once_with();p.stop_guest.assert_not_called();self.assertTrue(p.expired);self.assertTrue(p.done.is_set())

if __name__=='__main__':unittest.main()
