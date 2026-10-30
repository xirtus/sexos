import sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from check_nic_command import inspect
from test_nic_discovery import TRACE as DISCOVERY


def trace():
 return DISCOVERY+'''[kernel.pci.nic.found] 00:02.0 vendor=8086 device=10d3
[boot.nic.command] pd=13 slot=19 bus=0 device=2 function=0 vendor=32902 product=4307 allowed=6 dma_region=2 runtime_mint=0
[pci.command.negative.selftest] denied=9 output_zero=1 runtime_mint=0 ok=1
[boot.memory.authority.sealed] runtime_mint=0
[pci.command.apply] pd=13 slot=19 before=3 after=3 set=0 clear=0 changed=0
[sexnet.nic.command.ready] before=3 after=3 requested_bits=0 changed=0 denied=5 ok=1
'''


class CommandTrace(unittest.TestCase):
 def test_exact_genesis_and_after_seal_query(self):self.assertTrue(inspect(trace())['ok'])
 def test_wrong_holder_device_mask_peer_mutation_or_missing_denials_fails(self):
  for before,after in [('allowed=6','allowed=7'),('device=2 function=0','device=3 function=0'),('pd=13 slot=19 before','pd=12 slot=19 before'),('after=3 set=0','after=7 set=4'),('output_zero=1','output_zero=0'),('dma_region=2','dma_region=3')]:self.assertFalse(inspect(trace().replace(before,after))['ok'])
 def test_duplicate_or_out_of_phase_query_fails(self):
  lines=trace().splitlines()
  for bad in [trace()+'\n'+lines[-2],trace().replace('[boot.memory.authority.sealed]','[absent]'),trace().replace('[boot.memory.authority.sealed] runtime_mint=0','')+'\n[boot.memory.authority.sealed] runtime_mint=0']:
   self.assertFalse(inspect(bad)['ok'])
 def test_unsupported_controller_has_no_command_grant_or_query(self):
  t=DISCOVERY.replace('[kernel.cap.nic.grant] pd=13 slot=19\n','')+'''[pci.command.negative.selftest] denied=9 output_zero=1 runtime_mint=0 ok=1
[boot.memory.authority.sealed] runtime_mint=0
[sexnet.nic.command.unavailable] status=18446744073709551615 requested_bits=0 denied=5 ok=1
'''
  self.assertTrue(inspect(t,0)['ok']);self.assertFalse(inspect(t)['ok'])


if __name__=='__main__':unittest.main()
