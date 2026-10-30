import unittest,sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from nic_admission_gate import validate
from test_nic_discovery import TRACE

TAIL='''[boot.memory.mmio] pd=13 slot=19 bytes=65536 actual_bytes=131072
[boot.memory.authority.sealed] runtime_mint=0
[app.surface.presented] target=15 surface=300 request=1 ok=1
'''
class Admission(unittest.TestCase):
 def test_single_exact_lease_precedes_seal_and_uses_bounded_prefix(self):self.assertTrue(validate(TRACE+TAIL,1,1)['ok'])
 def test_duplicate_lease_postseal_lease_and_short_prefix_fail(self):
  for text in [TRACE+TRACE+TAIL,(TRACE+TAIL).replace('[kernel.cap.nic.grant] pd=13 slot=19\n','')+'[kernel.cap.nic.grant] pd=13 slot=19\n',(TRACE+TAIL).replace('bytes=65536','bytes=4096')]:self.assertFalse(validate(text,1,1)['ok'])
 def test_unsupported_device_has_no_alias_and_explicit_skip(self):
  text=TRACE.replace('[kernel.cap.nic.grant] pd=13 slot=19\n','')+TAIL.split('\n',1)[1]+'[kernel.cap.nic.skip] reason=unsupported_or_invalid_bar vendor=1af4 device=1000\n'
  self.assertTrue(validate(text,1,0,'unsupported_or_invalid_bar')['ok']);self.assertFalse(validate(text+TAIL,1,0,'unsupported_or_invalid_bar')['ok'])
if __name__=='__main__':unittest.main()
