import unittest
import pku_parity_gate as gate

BASE=''.join(['[bootgraph.cap.grant] pd=%d slot=%d\n'%(i,i) for i in range(8)]+
             ['[boot.app.route] target=%d\n'%i for i in (9,12,15,16)]+
             ['[boot.app.successor] logical=%d\n'%i for i in (9,12,15)]+
             ['[boot.paging.genesis] client=%d\n'%i for i in (1,2)]+
             ['[collar.grant.auto] grant_id=12 subject=202 object=6 op=LinkObjectToBuffer\n','[sext.release.denied] tick=55 client=2\n'])
PKU='PKU: Protection Keys enabled in CR4.\n';NOPKU='PKU: unsupported on this CPU/QEMU path; PKRU ops gated off\n'

class PkuParity(unittest.TestCase):
    def test_identical_decisions_pass_and_volatile_fields_are_ignored(self):
        result=gate.compare(PKU+BASE,NOPKU+BASE.replace('tick=55','tick=91'))
        self.assertTrue(result['ok'],result['errors']);self.assertEqual(result['interleaved_prefix_pairs'],0)
    def test_missing_extra_or_changed_decision_fails(self):
        for changed in(BASE.replace('[sext.release.denied] tick=55 client=2\n',''),BASE+'[bell.subscribe.deny] pd=15\n',
                       BASE.replace('subject=202','subject=203')):
            with self.subTest(changed=changed[-60:]):self.assertFalse(gate.compare(PKU+BASE,NOPKU+changed)['ok'])
    def test_interleaved_truncation_pairs_only_with_whole_counterpart(self):
        cut=BASE.replace(' object=6 op=LinkObjectToBuffer\n','[silkbar.clock.real_tick] iter=0\n')
        result=gate.compare(PKU+cut,NOPKU+BASE);self.assertTrue(result['ok'],result['errors']);self.assertEqual(result['interleaved_prefix_pairs'],1)
        bare=BASE.replace('[collar.grant.auto] grant_id=12 subject=202 object=6 op=LinkObjectToBuffer\n','[collar.grant.auto][silkbar.clock.real_tick] iter=0\n')
        self.assertFalse(gate.compare(PKU+bare,NOPKU+BASE)['ok'],'a tag with no field never pairs')
    def test_mode_banners_faults_and_minimum_genesis_are_required(self):
        self.assertFalse(gate.compare(NOPKU+BASE,NOPKU+BASE)['ok'])
        self.assertFalse(gate.compare(PKU+BASE,PKU+BASE)['ok'])
        self.assertFalse(gate.compare(PKU+BASE+'KERNEL PAGE FAULT\n',NOPKU+BASE)['ok'])
        thin=BASE.replace('[bootgraph.cap.grant] pd=7 slot=7\n','')
        self.assertFalse(gate.compare(PKU+thin,NOPKU+thin)['ok'])
    def test_transport_argument_fields_are_normalized_and_data_records_ignored(self):
        self.assertEqual(gate.records('[sexfiles.route.recv] a1=57\n[net.rx] bytes=3\n'),
                         gate.records('[sexfiles.route.recv] a1=0x68\n'))

if __name__=='__main__':unittest.main()
