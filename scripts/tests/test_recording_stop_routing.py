"""Source integration checks for the actual trusted Shell Stop dispatch sites.

These check admission/order; native pointer and Atlas interaction remain required.
"""
from pathlib import Path
import unittest
ROOT=Path(__file__).resolve().parents[2]
def function(source,name):
    start=source.index('fn '+name+'(');end=source.index('{',start)+1;depth=1
    while depth:
        depth+=(source[end]=='{')-(source[end]=='}');end+=1
    return source[start:end]
class RecordingStopRouting(unittest.TestCase):
    def test_stop_precedes_atlas_and_captured_app_down(self):
        source=(ROOT/'servers/silk-shell/src/main.rs').read_text()
        hit=function(source,'click_hit_test_and_focus')
        self.assertLess(hit.index('recording_stop_hit('),hit.index('if ATLAS_MODE_ENABLED'))
        inline=function(source,'handle_hid_event')
        at=inline.index('if button==1&&pressed&&recording_stop_hit(')
        self.assertLess(at,inline.index('program_pointer_button(button, pressed)'))
        self.assertIn('{return;}',inline[at:at+100])
        main=source[source.index('                OP_HID_EVENT => {'):]
        at=main.index('if button==1&&pressed&&recording_stop_hit(')
        self.assertLess(at,main.index('program_pointer_button(button, pressed)'))
        self.assertIn('{continue;}',main[at:at+100])
    def test_raw_usb_admission_and_scale_barrier_precede_pointer_mutation(self):
        source=(ROOT/'servers/silk-shell/src/main.rs').read_text()
        usb=source[source.index('                OP_USB_MOUSE_REPORT => {'):source.index('                OP_HID_EVENT => {')]
        for check in ['msg.caller_pd!=sex_pdx::shell_ready::INPUT_PD_ID','msg.caller_task!=sex_pdx::shell_ready::INPUT_PD_ID','DISPLAY_SCALE.pending()']:
            self.assertLess(usb.index(check),usb.index('POINTER_BUTTONS ='))
        self.assertLess(usb.index('recording_stop_hit('),usb.index('let left_held'))
    def test_stop_does_not_drop_captured_button_up_or_apply_drag_geometry(self):
        source=(ROOT/'servers/silk-shell/src/main.rs').read_text()
        stop=function(source,'recording_stop_hit')
        for check in ['DISPLAY_SCALE.pending()','ABS_SEEN_VALID||POINTER_USB_STATE_INIT','reflow_chrome','StatusChip(0)']:
            self.assertLess(stop.index(check),stop.index('DESKTOP_RECORDING.stop_now()'))
        self.assertLess(stop.index('DESKTOP_RECORDING.stop_now()'),stop.index('atlas_pointer_drag_cancel('))
        for forbidden in ['POINTER_CAPTURE=','POINTER_BUTTONS=','try_snap_on_drag_release','try_transition(','program_pointer_button(']:
            self.assertNotIn(forbidden,stop)
if __name__=='__main__':unittest.main()
