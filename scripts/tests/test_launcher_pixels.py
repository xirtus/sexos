"""Synthetic adversarial pixel fixtures; native QMP remains the guest proof."""
from pathlib import Path
import sys,unittest
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import check_launcher_pixels as c
from check_desktop_pixels import font_from_source,glyph
FONT=font_from_source((Path(__file__).resolve().parents[2]/'servers/sexdisplay/src/main.rs').read_text())
LOG='''[launcher.text.view] x=400 y=200 w=480 h=280 scale=2 selected=1 first=0 rows=2
[launcher.text.row] index=0 row=0 name=Focus Spindle status=CLOSED / REBOOT TO REOPEN
[launcher.text.row] index=1 row=1 name=Focus Quil status=CLOSED / REBOOT TO REOPEN
'''
def image(log=LOG):
    v,lines=c.layout(log);data=bytearray(1280*800*3)
    for row,line in lines:
        for col,char in enumerate(line[:37]):
            for gy,bits in enumerate(glyph(FONT,char)):
                for gx in range(5):
                    if bits&(1<<(4-gx)):
                        for dy in range(2):
                            for dx in range(2):
                                x=408+col*12+gx*2+dx;y=236+row*18+gy*2+dy
                                data[(y*1280+x)*3:(y*1280+x)*3+3]=b'\xf0\xf8\xff'
    return b'P6\n1280 800\n255\n'+data
class LauncherPixels(unittest.TestCase):
    def test_actual_labels_and_statuses(self):self.assertEqual(c.verify(LOG,image(),FONT),[])
    def test_empty_pixels_do_not_pass(self):self.assertTrue(c.verify(LOG,b'P6\n1280 800\n255\n'+bytes(1280*800*3),FONT))
    def test_stale_running_status_fails(self):self.assertTrue(c.verify(LOG,image(LOG.replace('CLOSED / REBOOT TO REOPEN','RUNNING / FOCUS')),FONT))
    def test_wrong_selection_fails(self):self.assertTrue(c.verify(LOG.replace('selected=1','selected=0'),image(),FONT))
    def test_incomplete_or_invalid_intents_fail(self):
        for text in ['',LOG.split('[launcher.text.row] index=1')[0],LOG.replace('x=400','x=1400')]:self.assertTrue(c.verify(text,image(),FONT))
if __name__=='__main__':unittest.main()
