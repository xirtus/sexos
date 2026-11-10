import sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from check_relaunch_pixels import verify,current_dirty_status
from check_desktop_pixels import font_from_source,glyph,draw_cell
ROOT=Path(__file__).resolve().parents[2]
FONT=font_from_source((ROOT/'servers/sexdisplay/src/main.rs').read_text())
TEXT='[shell.tile.apply] sid=301 x=40 y=60 w=600 h=400\n[quil.text.view] target=16 surface=301 status=N-V1 243B*'
STATUS='N-V1 243B*'
def pixels(status,ui=1,top=36,y=60):
    width,height=1280,800;data=bytearray(width*height*3)
    for col,char in enumerate(status):
        draw_cell(data,width,40+8*ui+col*12*ui,y+top,char,2*ui,b'\xe0\xf0\xff',FONT)
    return b'P6\n1280 800\n255\n'+data
class Check(unittest.TestCase):
    def test_auto_draft_status_keeps_exact_dirty_document_and_complete_intent(self):
        old='my * document * READY'
        def intent(value):return '[quil.text.view] target=16 surface=301 status='+value+'\n'
        self.assertEqual(current_dirty_status(intent('my * document * DRAFT KEPT'),old),'my * document * DRAFT KEPT')
        for value in ('other * document * DRAFT KEPT','my * document READY'):
            self.assertEqual(current_dirty_status(intent(value),old),old)
        self.assertEqual(current_dirty_status(intent('my * document * DRAFT KEPT').rstrip(),old),old)
        self.assertEqual(current_dirty_status('[web.console] '+intent('my * document * DRAFT KEPT'),old),old)

    def test_current_captured_scale_and_body_origin(self):
        for ui in (1,2,3):
            y=max(60,28*ui);metric=f'[display.chrome.metrics] scale={ui} bar={28*ui} title={26*ui} inset={8*ui} body_top={34*ui} target={24*ui} width=1280 height=800 publication=1\n'
            text=metric+TEXT.replace('y=60',f'y={y}')
            self.assertEqual(verify(text,pixels(STATUS,ui,34*ui,y),FONT,STATUS),[])
            self.assertTrue(verify(text,pixels(STATUS,ui,36*ui,y),FONT,STATUS))

    def test_synthetic_status_pixels(self):self.assertEqual(verify(TEXT,pixels(STATUS),FONT,STATUS),[])
    def test_blank_or_old_instance_status_cannot_pass(self):
        self.assertTrue(verify(TEXT,pixels(''),FONT,STATUS));self.assertTrue(verify(TEXT,pixels('N-V1 242B'),FONT,STATUS))
    def test_wrong_geometry_or_missing_intent(self):
        for text in [TEXT.replace('x=40','x=42'),TEXT.replace('target=16','target=9'),TEXT.replace('sid=301','sid=156')]:self.assertTrue(verify(text,pixels(STATUS),FONT,STATUS))
if __name__=='__main__':unittest.main()
