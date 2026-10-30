"""Strict independent Zoom receipts; no QEMU or native workflow is executed."""
import sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from spindle_durable_reboot_gate import zoom_receipt

ACTIVE='[silk.lifecycle.zoom.active] frame=9 sid=153 zx=0 zy=50 zw=1280 zh=750\n'
ZOOM='[shell.frame.zoom] frame=9 surface=153\n'
CORRUPT='[shell.window.action] action=ZoomToggle frame=91 reason=close_allowed\n sid=153 ok=1 reason=ok\n'

class ZoomReceipts(unittest.TestCase):
    def test_exact_ordered_frame_subject_and_geometry_are_recorded(self):
        prefix='prior records\n';trace=prefix+ACTIVE+ZOOM;receipt=zoom_receipt(trace,len(prefix))
        self.assertEqual(receipt,dict(kind='complete_active_geometry_and_frame_zoom',frame=9,surface=153,
            geometry=dict(x=0,y=50,w=1280,h=750),action_start=len(prefix),
            active_log_offset=len(prefix),frame_zoom_log_offset=len(prefix)+len(ACTIVE)))
    def test_damaged_legacy_line_is_never_reconstructed_or_sufficient(self):
        for trace in [CORRUPT,ACTIVE+CORRUPT,CORRUPT+ZOOM]:self.assertIsNone(zoom_receipt(trace))
        self.assertIsNotNone(zoom_receipt(ACTIVE+CORRUPT+ZOOM))
        self.assertIsNotNone(zoom_receipt(ACTIVE+ZOOM+CORRUPT))
    def test_frame_mismatch_zero_and_reverse_order_cannot_join(self):
        for trace in [ACTIVE+ZOOM.replace('frame=9','frame=8'),ACTIVE.replace('frame=9','frame=8')+ZOOM,ZOOM+ACTIVE,(ACTIVE+ZOOM).replace('frame=9','frame=0')]:
            self.assertIsNone(zoom_receipt(trace))
    def test_surface_and_every_geometry_component_are_exact(self):
        for before,after in [('sid=153','sid=154'),('zx=0','zx=1'),('zx=0','zx=-1'),('zy=50','zy=51'),('zw=1280','zw=1024'),('zh=750','zh=749')]:
            self.assertIsNone(zoom_receipt(ACTIVE.replace(before,after)+ZOOM))
        self.assertIsNone(zoom_receipt(ACTIVE+ZOOM.replace('surface=153','surface=154')))
    def test_missing_truncated_embedded_and_appended_records_are_not_complete(self):
        for trace in ['',ACTIVE,ZOOM,ACTIVE+ZOOM.rstrip('\n'),ACTIVE.rstrip('\n')+ZOOM,'interleaved'+ACTIVE+ZOOM,ACTIVE.replace(' zh=750\n',' zh=750 partial\n')+ZOOM,ACTIVE+ZOOM.replace('surface=153\n','surface=153 partial\n')]:
            self.assertIsNone(zoom_receipt(trace))
    def test_action_start_excludes_all_stale_or_partly_old_receipts(self):
        trace=ACTIVE+ZOOM
        for start in [1,len(ACTIVE),len(trace)]:self.assertIsNone(zoom_receipt(trace,start))
        stale=ACTIVE+ZOOM+'next action\n';self.assertIsNone(zoom_receipt(stale+ZOOM,len(stale)))
        self.assertIsNotNone(zoom_receipt(stale+ACTIVE+ZOOM,len(stale)))
    def test_invalid_action_offsets_fail_instead_of_reinterpreting_input(self):
        for start in [-1,len(ACTIVE+ZOOM)+1,True,1.0]:
            with self.assertRaises(ValueError):zoom_receipt(ACTIVE+ZOOM,start)
    def test_latest_complete_active_record_for_frame_cannot_be_replaced_by_old_geometry(self):
        wrong=ACTIVE.replace('zw=1280','zw=640')
        self.assertIsNone(zoom_receipt(ACTIVE+wrong+ZOOM))
        self.assertIsNotNone(zoom_receipt(wrong+ACTIVE+ZOOM))
    def test_unrelated_frames_do_not_stand_in_for_matching_subject(self):
        other=ACTIVE.replace('frame=9 sid=153','frame=8 sid=300')
        self.assertIsNotNone(zoom_receipt(ACTIVE+other+ZOOM))
        self.assertIsNone(zoom_receipt(other+ZOOM))

if __name__=='__main__':unittest.main()
