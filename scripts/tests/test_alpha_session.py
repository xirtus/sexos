import copy
import pathlib
import sys
import unittest
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parents[1]))
from check_alpha_session import session_errors,boot_lifecycle_errors


def accepted():
    kinds=['quil_input_receipt','quil_input_pixels','warm_save_receipt','warm_save_pixels','spindle_stat','linen_refresh']
    return dict(ok=True,acceptance=True,keyboard=True,pointer=True,edit_during_save_dirty=True,
        documents=[dict(object=5),dict(object=6)],
        soak=dict(acceptance=True,elapsed_seconds=1800.1,cycles=12,
                  pending_quil_close=True,pending_spindle_close=True,boot_successors=[16,17],cleanup_errors=[],
                  samples=[dict(kind=k,seconds=.1)for _ in range(12)for k in kinds]))

class AlphaSessionTests(unittest.TestCase):
    def test_complete_sustained_metadata_passes(self):self.assertEqual(session_errors(accepted()),[])
    def test_short_rehearsal_or_missing_acceptance_cannot_pass(self):
        for field,value in [('elapsed_seconds',1799.9),('elapsed_seconds',float('nan')),('elapsed_seconds',True),('acceptance',False),('cycles',1),('pending_quil_close',False),('boot_successors',[9,12])]:
            r=accepted();r['soak'][field]=value;self.assertTrue(session_errors(r),(field,value))
        r=accepted();r['acceptance']=False;self.assertTrue(session_errors(r))
    def test_missing_or_invalid_samples_and_duplicate_object_reject(self):
        for sample in [None,-1,float('inf'),True]:
            r=accepted()
            if sample is None:r['soak']['samples'].pop()
            else:r['soak']['samples'][0]['seconds']=sample
            self.assertTrue(session_errors(r))
        r=accepted();r['documents'][1]['object']=5;self.assertTrue(session_errors(r))
    def test_third_reserve_needs_its_own_recovery_and_terminal_evidence(self):
        r=accepted();r['soak']['boot_successors']=[16,17,18]
        self.assertTrue(session_errors(r))
        r['soak']['third_quil']=dict(target=18,surface=307,logical=9,incarnation=3,
            edited_and_saved=True,pixel_errors=[],terminal=True,reserve_exhausted=True,documents=[5,6])
        self.assertEqual(session_errors(r),[])
        r['soak']['third_quil']['target']=16;self.assertTrue(session_errors(r))
    def test_reboot_lifecycle_support_does_not_relabel_rehearsal_as_sustained(self):
        r=accepted();r['acceptance']=False;r['soak']['acceptance']=False;r['soak']['elapsed_seconds']=1
        self.assertEqual(boot_lifecycle_errors(r),[]);self.assertTrue(session_errors(r))
        r['soak']['boot_successors']=[16,17,18];self.assertTrue(boot_lifecycle_errors(r))
if __name__=='__main__':unittest.main()
