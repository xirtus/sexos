import unittest,sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from check_nic_discovery import inspect

TRACE='''[pci.net.device] vendor=0x8086 device=0x10D3 class=0x02 subclass=0x00 prog_if=0x00 ok=1 reason=network_controller_detected
[pci.net.passive] vendor=0x8086 device=0x10D3 config_writes=0 mmio=0 rings=0 dma=0 packets=0 owner=boot_assigned_sexnet ok=1
[kernel.cap.nic.grant] pd=13 slot=19
[sexnet.passive.ready.fast] network=0 dns=0 tcp=0 http=0 tls=0 ok=1
'''
class Discovery(unittest.TestCase):
 def test_exact_normal_discovery_and_lease(self):self.assertTrue(inspect(TRACE)['ok'])
 def test_missing_device_passive_pair_lease_or_service_fails(self):
  for text in [TRACE.replace('[pci.net.device]','[missing]'),TRACE.replace('config_writes=0','config_writes=1'),TRACE.replace('pd=13','pd=15'),TRACE.replace('network=0','network=1'),TRACE.replace('device=0x10D3 config','device=0x100E config')]:self.assertFalse(inspect(text)['ok'])
 def test_legacy_side_effect_or_fault_fails_even_with_passive_marker(self):
  for marker in ['[e1000.ring.truth] allocated=1','[pci.net.command.enable] bm=1','[pci.net.legacy_diagnostics] enabled=1','[sexnet.proof.profile] enabled=1','KERNEL PAGE FAULT']:
   self.assertFalse(inspect(TRACE+marker)['ok'])
if __name__=='__main__':unittest.main()
