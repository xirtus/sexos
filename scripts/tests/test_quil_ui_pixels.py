import unittest,pathlib,sys
from unittest.mock import patch
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parents[1]))
import check_quil_ui_pixels as c
from check_desktop_pixels import glyph,font_from_source,draw_cell,cell_coverage
from native_scalar_cells import coverage as scalar_coverage
FONT=font_from_source((pathlib.Path(__file__).resolve().parents[2]/'servers/sexdisplay/src/main.rs').read_text())
TEXT='note * READY\nNEW  OPEN SAVE AS\nCTRL N O S / ESC CMDS\nhello#'
LOG=f'[shell.tile.apply] sid=201 x=640 y=50 w=640 h=335\n[shell.app.viewport] target=9 sid=156 x=648 y=86 w=624 h=291\n[quil.ui.view] target=9 surface=156 paint=1 w=624 h=291 top=12 cols=51 rows=15 cursor=5 view=0\n[quil.ui.text] paint=1 off=0 hex={TEXT.encode().hex()}\n[quil.ui.admitted] paint=1 bytes={len(TEXT)} presentation=unconfirmed\n[quil.ui.applied] paint=1 bytes={len(TEXT)} pixels=unconfirmed\n'
def fixture(text=TEXT,top=12,scale=2):
 p=bytearray(b'\x0c\x14\x20'*(1280*800))
 for row,line in enumerate(text.split('\n')):
  column=0
  for ch in line:
   span=2 if ch=='😀'else 1
   x0=656+column*6*scale;y0=86+top+row*9*scale
   if 32<=ord(ch)<=126:draw_cell(p,1280,x0,y0,ch,scale,b'\xe0\xf0\xff',FONT)
   elif ch!='\t':
    coverage=scalar_coverage(ord(ch),6*scale*span,9*scale)
    if coverage is None:raise AssertionError('fixture scalar unexpectedly missing')
    for i,alpha in enumerate(coverage):
     at=((y0+i//(6*scale*span))*1280+x0+i%(6*scale*span))*3
     p[at:at+3]=bytes((fg*alpha+bg*(255-alpha))//255 for fg,bg in zip(b'\xe0\xf0\xff',b'\x0c\x14\x20'))
   column+=span
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
  current=LOG.replace('target=9 surface=156','target=18 surface=307').replace('top=12','top=34').replace('sid=201','sid=307').replace('sid=307 x=640 y=50 w=640 h=335','sid=307 x=648 y=86 w=624 h=291')
  combined=LOG+'[shell.tile.apply] sid=307 x=648 y=86 w=624 h=291\n'+current
  v=c.view(combined,target=18)
  self.assertEqual(v['surface'],307);self.assertEqual(v['text'],TEXT)
class PublishedViewportTests(unittest.TestCase):
 def test_later_incarnation_geometry_does_not_depend_on_boot_tile_log_budget(self):
  current=LOG.replace('target=9 surface=156','target=18 surface=307').replace('top=12','top=34')
  current='\n'.join(x for x in current.splitlines()if not x.startswith('[shell.tile.apply]')and not x.startswith('[shell.app.viewport]'))+'\n'
  published='[shell.document.viewport] target=18 task=180 surface=307 sequence=4 x=648 y=86 w=624 h=291 enqueued=1\n'
  self.assertEqual(c.verify(published+current,fixture(top=34),FONT,'hello#',target=18,resolution=(1280,800)),[])
  self.assertTrue(c.verify(published.replace('w=624','w=640')+current,fixture(),FONT,target=18))
  self.assertTrue(c.verify(published.replace('target=18','target=16')+current,fixture(),FONT,target=18))

class FullCoverageQuilPixels(unittest.TestCase):
 def test_partial_zero_and_solid_coverage_corruption_all_fail(self):
  cell=cell_coverage(FONT,'n',2)
  for predicate in (lambda a:0<a<255,lambda a:a==0,lambda a:a==255):
   with self.subTest(predicate=predicate):
    i=next(i for i,a in enumerate(cell)if predicate(a))
    data=bytearray(fixture());at=data.index(b'255\n')+4+((98+i//12)*1280+656+i%12)*3
    data[at]^=1
    self.assertTrue(c.verify(LOG,bytes(data),FONT))
 def test_erasure_rejects_non_foreground_stale_antialias_pixel(self):
  data=bytearray(fixture());at=data.index(b'255\n')+4+(230*1280+660)*3;data[at:at+3]=b'\x0e\x15\x21'
  self.assertTrue(any('stray' in error for error in c.verify(LOG,bytes(data),FONT)))
 def test_body_top_and_compact_font_threshold_are_source_bound(self):
  self.assertTrue(c.verify(LOG.replace('top=12','top=14'),fixture(),FONT))
  self.assertEqual(c.source_contract()['metrics']['scale_threshold'],256)
 def test_compact_255_and_normal_256_use_actual_independent_atlases(self):
  text='note#'
  for width,scale in ((255,1),(256,2)):
   log=LOG.replace('w=624','w='+str(width)).replace('cols=51','cols=19').replace(TEXT.encode().hex(),text.encode().hex()).replace('bytes='+str(len(TEXT)),'bytes='+str(len(text)))
   self.assertEqual(c.verify(log,fixture(text,scale=scale),FONT),[])
   self.assertTrue(c.verify(log,fixture(text,scale=3-scale),FONT))
 def test_bidi_formatting_is_not_mistaken_for_logical_scalar_order(self):
  for control in ('\u202e','\u2067','\u200d'):
   text=TEXT+control
   log=LOG.replace(TEXT.encode().hex(),text.encode().hex()).replace('bytes='+str(len(TEXT)),'bytes='+str(len(text.encode())))
   self.assertTrue(any('visual-order' in error for error in c.verify(log,fixture(),FONT)))
 def test_missing_scalar_is_reported_for_explicit_bitmap_recovery(self):
  self.assertIsNone(scalar_coverage(0x10ffff,12,18))

class ScalarQuilPixels(unittest.TestCase):
 def test_utf8_admission_uses_byte_length_and_actual_fallback_wide_cells(self):
  text='note READY\né€😀\tZ#';raw=text.encode()
  log=LOG.replace(TEXT.encode().hex(),raw.hex()).replace('bytes='+str(len(TEXT)),'bytes='+str(len(raw)))
  image=fixture(text)
  self.assertEqual(c.view(log)['text'],text)
  self.assertEqual(c.verify(log,image,FONT,'é€😀\tZ#'),[])
  self.assertTrue(c.verify(log.replace(raw.hex(),text.replace('é','?').encode().hex()),image,FONT))
  bad=bytearray(raw);bad[raw.index('é'.encode())]=0xff
  self.assertTrue(c.verify(log.replace(raw.hex(),bad.hex()),image,FONT))

if __name__=='__main__':unittest.main()
