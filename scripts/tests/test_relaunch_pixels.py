import sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from check_relaunch_pixels import verify
from check_desktop_pixels import font_from_source,glyph,draw_cell
ROOT=Path(__file__).resolve().parents[2]
FONT=font_from_source((ROOT/'servers/sexdisplay/src/main.rs').read_text())
TEXT='[shell.tile.apply] sid=301 x=40 y=60 w=600 h=400\n[quil.text.view] target=16 surface=301 status=N-V1 243B*'
STATUS='N-V1 243B*'
def pixels(status):
    width,height=1280,800;data=bytearray(width*height*3)
    for col,char in enumerate(status):
        draw_cell(data,width,40+8+col*12,60+36,char,2,b'\xe0\xf0\xff',FONT)
    return b'P6\n1280 800\n255\n'+data
class Check(unittest.TestCase):
    def test_synthetic_status_pixels(self):self.assertEqual(verify(TEXT,pixels(STATUS),FONT,STATUS),[])
    def test_blank_or_old_instance_status_cannot_pass(self):
        self.assertTrue(verify(TEXT,pixels(''),FONT,STATUS));self.assertTrue(verify(TEXT,pixels('N-V1 242B'),FONT,STATUS))
    def test_wrong_geometry_or_missing_intent(self):
        for text in [TEXT.replace('x=40','x=42'),TEXT.replace('target=16','target=9'),TEXT.replace('sid=301','sid=156')]:self.assertTrue(verify(text,pixels(STATUS),FONT,STATUS))
if __name__=='__main__':unittest.main()
