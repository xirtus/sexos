import unittest,sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from app_switch_gate import focus_observation,verify_indicator,indicator_colors

TRACE='''[display.repaint] ordinal=17 kind=strip start=3 end=7 ticks=4 seen=4 drained=1 upserts=0 fitted_noops=0 focus=153
[display.focus.drawn] focus=153 ordinal=17 kind=strip tick=7 pixels=unconfirmed
'''
class SwitchFeedback(unittest.TestCase):
 def test_exact_same_redraw_focus_correlation(self):self.assertEqual(focus_observation(TRACE,153),dict(ordinal=17,kind='strip',seen=4))
 def test_expected_colors_come_from_actual_production_native_overrides(self):
  colors=indicator_colors(Path(__file__).resolve().parents[2]);self.assertEqual(colors[300],0x5ed9c3);self.assertEqual(colors[153],0x89b4fa);self.assertEqual(colors[302],colors[153]);self.assertEqual(colors['unknown'],0x444444)
 def test_last_reserved_quil_keeps_its_family_focus_color(self):
  colors=indicator_colors(Path(__file__).resolve().parents[2])
  for surface in [308,309,310,311,312]:self.assertEqual(colors[surface],0xcba6f7)
  raw=self.ppm(b'\xcb\xa6\xf7');verify_indicator(raw,312,(320,100),colors)
  with self.assertRaises(RuntimeError):verify_indicator(self.ppm(b'\x44\x44\x44'),312,(320,100),colors)
 def test_unrelated_missing_or_clock_only_repaint_is_rejected(self):
  for text in [TRACE.replace('seen=4','seen=0'),TRACE.replace('focus=153\n','focus=201\n'),TRACE.replace('ordinal=17 kind=strip start','ordinal=18 kind=strip start'),TRACE.replace('kind=strip start','kind=full start')]:
   with self.assertRaises(RuntimeError):focus_observation(text,153)
  with self.assertRaises(RuntimeError):focus_observation(TRACE,201)
 def ppm(self,color):
  w,h=320,100;pixels=bytearray(w*h*3)
  for y in range(22,26):
   for x in range(155,159):pixels[(y*w+x)*3:(y*w+x)*3+3]=color
  return b'P6\n320 100\n255\n'+pixels
 def test_indicator_checks_every_actual_pixel_and_dimensions(self):
  raw=self.ppm(b'\x89\xb4\xfa');verify_indicator(raw,153,(320,100))
  with self.assertRaises(RuntimeError):verify_indicator(raw,201,(320,100))
  with self.assertRaises(RuntimeError):verify_indicator(raw,153,(1024,768))
  bad=bytearray(raw);bad[len(bad)-320*3*(100-22)+155*3]^=1
  with self.assertRaises(RuntimeError):verify_indicator(bad,153,(320,100))
if __name__=='__main__':unittest.main()
