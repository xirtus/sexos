"""Independent whole reader pixels/navigation: no native/QEMU execution."""
import copy,sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from http_content_gate import READER_BODY,READER_TAIL,load_pixel_contract,overview_preview,reader_frame,reader_move,verify_reader_pixels
from check_desktop_pixels import glyph
ROOT=Path(__file__).resolve().parents[2]

class ReaderPixels(unittest.TestCase):
 @classmethod
 def setUpClass(cls):cls.contract=load_pixel_contract(ROOT)
 def frame(self,body=READER_BODY,w=624,h=706,anchor=0):return reader_frame(body,w,h,self.contract,anchor)
 def trace(self,frame,body=READER_BODY,w=624,h=706,x=600,y=70,sequence=1):
  return(f'[shell.tile.apply] sid=300 x={x} y={y} w={w} h={h}\n'
   f'[shell.document.viewport] target=15 task=15 surface=300 sequence={sequence} x={x} y={y} w={w} h={h} enqueued=1\n'
   f'[browser.viewport] target=15 task=15 sequence={sequence} w={w} h={h} origin=held_shell\n'
   f'[browser.reader.layout] first={frame["first"]} end={frame["end"]} body_rows={frame["body_rows"]} rows={frame["rows"]} cols={frame["cols"]} total_lines={frame["total_lines"]} source_bytes={len(body)} viewport={sequence} origin=validated_body tls=0 pixels=unconfirmed\n'
   f'[browser.view.applied] bytes={frame["bytes"]} http=258 body_bytes={len(body)} pixels=unconfirmed\n')
 def screen(self,frame,x=600,y=70,shift=0):
  pixels=bytearray(1280*800*3);factor=frame['scale'];contract=self.contract;color=contract['color'].to_bytes(3,'big')
  for row,line in enumerate(frame['lines']):
   for column,byte in enumerate(line):
    for gy,bits in enumerate(glyph(contract['font'],chr(byte))):
     for gx in range(5):
      if not bits&(1<<(4-gx)):continue
      for dy in range(factor):
       for dx in range(factor):
        px=x+contract['inset']+(column*contract['advance']+gx)*factor+dx;py=y+contract['top']+(row*contract['line']+gy)*factor+dy+shift
        pixels[(py*1280+px)*3:(py*1280+px)*3+3]=color
  return b'P6\n1280 800\n255\n'+pixels
 def result(self,frame,body=READER_BODY,w=624,h=706,anchor=0,screen=None,trace=None):
  return verify_reader_pixels(trace or self.trace(frame,body,w,h),screen or self.screen(frame),body,self.contract,anchor)
 def test_exact_2048_fixture_complete_home_chrome_and_body(self):
  self.assertEqual(len(READER_BODY),2048);self.assertTrue(READER_BODY.endswith(READER_TAIL));self.assertTrue(all(b==10 or 32<=b<=126 for b in READER_BODY))
  frame=self.frame();self.assertEqual((frame['cols'],frame['rows'],frame['body_rows'],frame['bytes']),(49,10,7,499));self.assertTrue(self.result(frame)['ok'])
  self.assertEqual(frame['lines'][0].rstrip(),b'Plain HTTP / no TLS');self.assertEqual(frame['lines'][1].rstrip(),b'probe.sexos.test /welcome')
 def test_end_reaches_final_source_byte_well_beyond_preview(self):
  start=self.frame();anchor=reader_move(start,'end');end=self.frame(anchor=anchor)
  self.assertGreater(anchor,64);self.assertEqual(end['end'],end['total_lines']);self.assertIn(READER_TAIL,b''.join(end['lines'][2:-1]));self.assertTrue(self.result(end,anchor=anchor)['ok'])
  corrupt=copy.deepcopy(end);last=next(i for i in range(len(corrupt['lines'])-2,1,-1)if b'Z'in corrupt['lines'][i]);corrupt['lines'][last]=corrupt['lines'][last].replace(b'Z',b'X')
  self.assertFalse(self.result(end,anchor=anchor,screen=self.screen(corrupt))['ok'])
 def test_line_page_home_end_navigation_and_no_wrap_at_bounds(self):
  body=b'0123456789'*60;frame=self.frame(body,w=82,h=160);self.assertEqual((frame['cols'],frame['body_rows'],frame['total_lines']),(10,11,60))
  for key,first in [('up',0),('down',1),('pgdn',12),('pgup',1),('home',0),('end',49),('down',49),('pgdn',49),('up',48)]:
   frame=self.frame(body,w=82,h=160,anchor=reader_move(frame,key));self.assertEqual(frame['first'],first)
 def test_resize_uses_source_anchor_and_latest_geometry(self):
  frame=self.frame();anchor=reader_move(frame,'pgdn');frame=self.frame(anchor=anchor);anchor=frame['anchor']
  expanded=self.frame(w=1280,h=706,anchor=anchor);self.assertEqual(expanded['cols'],79)
  # Tile is deliberately moved to fit this actual synthetic framebuffer.
  trace=self.trace(expanded,w=1280,h=706,x=0);screen=self.screen(expanded,x=0)
  self.assertTrue(verify_reader_pixels(trace,screen,READER_BODY,self.contract,anchor)['ok'])
  restored=self.frame(anchor=expanded['anchor']);self.assertTrue(self.result(restored,anchor=expanded['anchor'])['ok'])
 def test_zoom_geometry_uses_exact_shell_notice_instead_of_old_tile(self):
  frame=self.frame(w=1280,h=706);trace=self.trace(frame,w=1280,h=706,x=0)
  trace=trace.replace('[shell.tile.apply] sid=300 x=0 y=70 w=1280 h=706','[shell.tile.apply] sid=300 x=600 y=70 w=624 h=706')
  screen=self.screen(frame,x=0);self.assertTrue(verify_reader_pixels(trace,screen,READER_BODY,self.contract)['ok'])
  for wrong in [trace.replace('surface=300 sequence=1','surface=300 sequence=2'),trace.replace('surface=300 sequence=1','surface=301 sequence=1'),trace.replace('x=0 y=70 w=1280 h=706 enqueued=1','x=0 y=70 w=624 h=706 enqueued=1')]:
   self.assertFalse(verify_reader_pixels(wrong,screen,READER_BODY,self.contract)['ok'])
 def test_wrong_complete_body_byte_or_row_is_rejected(self):
  frame=self.frame();wrong=copy.deepcopy(frame);wrong['lines'][2]=b'X'+wrong['lines'][2][1:]
  self.assertFalse(self.result(frame,screen=self.screen(wrong))['ok']);self.assertFalse(self.result(frame,screen=self.screen(frame,shift=18))['ok'])
 def test_header_host_and_footer_are_required(self):
  frame=self.frame()
  for row in [0,1,frame['rows']-1]:
   wrong=copy.deepcopy(frame);wrong['lines'][row]=b' '*frame['cols'];self.assertFalse(self.result(frame,screen=self.screen(wrong))['ok'])
 def test_padding_spare_column_and_old_rows_below_snapshot_must_erase(self):
  frame=self.frame();raw=self.screen(frame);header=len(b'P6\n1280 800\n255\n')
  for row,column in [(0,frame['cols']-1),(4,frame['cols']),(frame['rows']+2,0)]:
   wrong=bytearray(raw);x=600+self.contract['inset']+column*self.contract['advance']*frame['scale'];y=70+self.contract['top']+row*self.contract['line']*frame['scale'];offset=header+(y*1280+x)*3;wrong[offset:offset+3]=self.contract['color'].to_bytes(3,'big');self.assertFalse(self.result(frame,screen=bytes(wrong))['ok'])
 def test_short_body_keeps_exact_blank_rows(self):
  body=b'first\n\nlast';frame=self.frame(body);self.assertEqual(frame['total_lines'],3);self.assertEqual(frame['lines'][3],b' '*frame['cols']);self.assertTrue(self.result(frame,body)['ok'])
  wrong=copy.deepcopy(frame);wrong['lines'][6]=b'STALE'.ljust(frame['cols'],b' ');self.assertFalse(self.result(frame,body,screen=self.screen(wrong))['ok'])
 def test_missing_or_forged_viewport_layout_and_applied_receipts_fail(self):
  frame=self.frame();trace=self.trace(frame)
  for wrong in ['',trace.replace('first=0','first=1'),trace.replace('http=258','http=2'),trace.replace('sequence=1 w=624','sequence=1 w=620'),trace[:trace.index('[browser.view.applied]')]]:
   self.assertFalse(verify_reader_pixels(wrong,self.screen(frame),READER_BODY,self.contract)['ok'])
 def test_actual_framebuffer_must_match_requested_boot_profile(self):
  frame=self.frame();trace=self.trace(frame);screen=self.screen(frame)
  self.assertTrue(verify_reader_pixels(trace,screen,READER_BODY,self.contract,resolution=(1280,800))['ok'])
  self.assertFalse(verify_reader_pixels(trace,screen,READER_BODY,self.contract,resolution=(1024,768))['ok'])
  compact=self.frame(w=512,h=359);raw=self.screen(compact,x=0);pixels=raw.split(b'\n',3)[3]
  cropped=b'P6\n1024 768\n255\n'+b''.join(pixels[y*1280*3:(y*1280+1024)*3]for y in range(768))
  self.assertTrue(verify_reader_pixels(self.trace(compact,w=512,h=359,x=0),cropped,READER_BODY,self.contract,resolution=(1024,768))['ok'])
 def test_clipped_missing_source_and_oversize_fail_closed(self):
  frame=self.frame()
  for wrong in [self.trace(frame).replace('h=706','h=150'),self.trace(frame).replace('y=70','y=200')]:self.assertFalse(self.result(frame,trace=wrong)['ok'])
  with self.assertRaises(ValueError):self.frame(b'x'*2049)
  with self.assertRaises(ValueError):self.frame(w=20,h=100)
 def test_non_ascii_controls_model_and_overview_are_distinct(self):
  body=b'a\t\rb\x00\x7f\xff\nnext';frame=self.frame(body);self.assertTrue(frame['lines'][2].startswith(b'a  b  ?'));self.assertTrue(frame['lines'][3].startswith(b'next'))
  self.assertEqual(overview_preview(b' \nA\t\tB\0\xff  ')[:5],b'A B ?');self.assertEqual(overview_preview(READER_BODY)[61:],b'...')

if __name__=='__main__':unittest.main()
