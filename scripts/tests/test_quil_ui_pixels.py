import unittest,pathlib,sys
from unittest.mock import patch
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parents[1]))
import check_quil_ui_pixels as c
from check_desktop_pixels import glyph,font_from_source
FONT=font_from_source((pathlib.Path(__file__).resolve().parents[2]/'servers/sexdisplay/src/main.rs').read_text())
TEXT='note * READY\nNEW  OPEN SAVE AS\nCTRL N O S / ESC CMDS\nhello#'
LOG=f'[shell.tile.apply] sid=201 x=640 y=50 w=640 h=335\n[shell.app.viewport] target=9 sid=156 x=648 y=86 w=624 h=291\n[quil.ui.view] target=9 surface=156 paint=1 w=624 h=291 top=12 cols=51 rows=15 cursor=5 view=0\n[quil.ui.text] paint=1 off=0 hex={TEXT.encode().hex()}\n[quil.ui.admitted] paint=1 bytes={len(TEXT)} presentation=unconfirmed\n[quil.ui.applied] paint=1 bytes={len(TEXT)} pixels=unconfirmed\n'
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
class SettledViewportTests(unittest.TestCase):
 def test_new_glyphs_without_application_ack_cannot_pass(self):
  self.assertTrue(c.verify(LOG.split('[quil.ui.applied]')[0],fixture(),FONT))
 def test_old_rows_outside_new_text_must_be_erased(self):
  data=bytearray(fixture());at=data.index(b'255\n')+4+(230*1280+660)*3;data[at:at+3]=b'\xe0\xf0\xff'
  self.assertTrue(any('stray' in error for error in c.verify(LOG,bytes(data),FONT)))
class IncarnationScopeTests(unittest.TestCase):
 def test_reused_paint_sequence_cannot_mix_previous_incarnation_chunks(self):
  current=LOG.replace('target=9 surface=156','target=18 surface=307').replace('sid=201','sid=307').replace('sid=307 x=640 y=50 w=640 h=335','sid=307 x=648 y=86 w=624 h=291')
  combined=LOG+'[shell.tile.apply] sid=307 x=648 y=86 w=624 h=291\n'+current
  v=c.view(combined,target=18)
  self.assertEqual(v['surface'],307);self.assertEqual(v['text'],TEXT)
class PublishedViewportTests(unittest.TestCase):
 def test_later_incarnation_geometry_does_not_depend_on_boot_tile_log_budget(self):
  current=LOG.replace('target=9 surface=156','target=18 surface=307')
  current='\n'.join(x for x in current.splitlines()if not x.startswith('[shell.tile.apply]')and not x.startswith('[shell.app.viewport]'))+'\n'
  published='[shell.document.viewport] target=18 task=180 surface=307 sequence=4 x=648 y=86 w=624 h=291 enqueued=1\n'
  self.assertEqual(c.verify(published+current,fixture(),FONT,'hello#',target=18,resolution=(1280,800)),[])
  self.assertTrue(c.verify(published.replace('w=624','w=640')+current,fixture(),FONT,target=18))
  self.assertTrue(c.verify(published.replace('target=18','target=16')+current,fixture(),FONT,target=18))
if __name__=='__main__':unittest.main()
