import unittest
from test_nic_command import trace
from check_nic_command import inspect


def prepared():
    return trace()+'''[pci.command.apply] pd=13 slot=19 before=3 after=3 set=0 clear=0 changed=0
[pci.command.apply] pd=13 slot=19 before=3 after=3 set=2 clear=4 changed=0
[sexnet.nic.prepare] from=Cold to=Drain failure=None status=2148009859 mac=000000000000 dma=0 packets=0
[sexnet.nic.prepare] from=Drain to=Reset failure=None status=2148009859 mac=000000000000 dma=0 packets=0
[sexnet.nic.prepare] from=Reset to=Verify failure=None status=2148009859 mac=000000000000 dma=0 packets=0
[pci.command.apply] pd=13 slot=19 before=3 after=3 set=0 clear=0 changed=0
[sexnet.nic.prepare] from=Verify to=Quiescent failure=None status=2148009859 mac=525400123456 dma=0 packets=0
'''


class PrepareTrace(unittest.TestCase):
    def test_exact_current_lease_command_sequence_and_observed_station_address(self):
        self.assertTrue(inspect(prepared(),preparation=True)['ok'])
        self.assertFalse(inspect(prepared())['ok'])

    def test_wrong_request_master_enable_address_and_reordered_transitions_fail(self):
        for old,new in [('set=2 clear=4','set=6 clear=0'),('set=2 clear=4 changed=0','set=2 clear=4 changed=1'),
                        ('from=Drain to=Reset','from=Reset to=Drain'),('failure=None','failure=Some(Command)'),
                        ('mac=525400123456','mac=535400123456'),('mac=525400123456','mac=000000000000'),
                        ('dma=0 packets=0','dma=1 packets=1')]:
            self.assertFalse(inspect(prepared().replace(old,new),preparation=True)['ok'],new)

    def test_missing_duplicate_and_failed_records_fail_without_checker_exception(self):
        text=prepared();lines=text.splitlines()
        for i in range(len(lines)):
            self.assertFalse(inspect('\n'.join(lines[:i]+lines[i+1:]),preparation=True)['ok'],lines[i])
        for line in lines:
            if line.startswith(('[pci.command.apply]','[sexnet.nic.prepare]')):
                self.assertFalse(inspect(text+line,preparation=True)['ok'])
        self.assertFalse(inspect(text+'[sexnet.nic.prepare] from=Verify to=Quarantined failure=Some(Command)',preparation=True)['ok'])

    def test_absent_controller_has_no_prepare_or_mmio_receipt(self):
        from test_nic_discovery import TRACE
        t=TRACE.replace('[kernel.cap.nic.grant] pd=13 slot=19\n','')+'''[pci.command.negative.selftest] denied=9 output_zero=1 runtime_mint=0 ok=1
[boot.memory.authority.sealed] runtime_mint=0
[sexnet.nic.command.unavailable] status=18446744073709551615 requested_bits=0 denied=5 ok=1
[sexnet.nic.prepare.unavailable] status=18446744073709551615 dma=0 packets=0
'''
        self.assertTrue(inspect(t,0,True)['ok'])
        self.assertFalse(inspect(t+'[sexnet.nic.prepare] from=Cold to=Drain failure=None status=1 mac=000000000000 dma=0 packets=0',0,True)['ok'])


if __name__=='__main__':unittest.main()
