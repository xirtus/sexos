import unittest,pathlib,sys
from unittest.mock import patch
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parents[1]))
import check_quil_ui_pixels as c
from check_desktop_pixels import glyph,font_from_source
FONT=font_from_source((pathlib.Path(__file__).resolve().parents[2]/'servers/sexdisplay/src/main.rs').read_text())
TEXT='note * READY\nNEW  OPEN SAVE AS\nCTRL N O S / ESC CMDS\nhello#'
LOG=f'[shell.tile.apply] sid=201 x=640 y=50 w=640 h=335\n[shell.app.viewport] target=9 sid=156 x=648 y=86 w=624 h=291\n[quil.ui.view] target=9 surface=156 paint=1 w=624 h=291 top=12 cols=51 rows=15 cursor=5 view=0\n[quil.ui.text] paint=1 off=0 hex={TEXT.encode().hex()}\n[quil.ui.admitted] paint=1 bytes={len(TEXT)} presentation=unconfirmed\n'
def fixture():
 p=bytearray(1280*800*3)
 for row,line in enumerate(TEXT.split('\n')):
  for col,ch in enumerate(line):
   for gy,bits in enumerate(glyph(FONT,ch)):
    for gx in range(5):
     if bits&(1<<(4-gx)):
      for dy in range(2):
       for dx in range(2):
        x=656+(col*6+gx)*2+dx;y=98+(row*9+gy)*2+dy;p[(y*1280+x)*3:(y*1280+x)*3+3]=b'\xe0\xf0\xff'
 return b'P6\n1280 800\n255\n'+p
class QuilPixels(unittest.TestCase):
 def test_exact_complete_glyphs_and_caret(self):self.assertEqual(c.verify(LOG,fixture(),FONT,'hello#',resolution=(1280,800)),[])
 def test_empty_framebuffer_and_missing_admission_rejected(self):
  self.assertTrue(c.verify(LOG,b'P6\n1280 800\n255\n'+bytes(1280*800*3),FONT))
  self.assertTrue(c.verify(LOG.split('[quil.ui.admitted]')[0],fixture(),FONT))
 def test_stale_geometry_wrong_text_and_resolution_rejected(self):
  for log,expected,res in[(LOG.replace('paint=1 w=624','paint=1 w=640'),'hello#',(1280,800)),(LOG,'other',(1280,800)),(LOG,'hello#',(1024,768))]:self.assertTrue(c.verify(log,fixture(),FONT,expected,resolution=res))
 def test_waits_for_latest_exact_admission_without_accepting_older_paint(self):
  missing=LOG.split('[quil.ui.admitted]')[0]
  values=iter([missing,missing,LOG])
  with patch.object(c.time,'sleep')as sleep:
   self.assertEqual(c.wait_view(lambda:next(values))['paint'],1)
   self.assertEqual(sleep.call_count,2)
 def test_permanent_missing_or_malformed_view_fails_at_deadline(self):
  with patch.object(c.time,'monotonic',side_effect=[0,0,1]),patch.object(c.time,'sleep'):
   with self.assertRaisesRegex(RuntimeError,'not fully admitted'):c.wait_view(lambda:LOG.split('[quil.ui.admitted]')[0],timeout=.5)
if __name__=='__main__':unittest.main()
